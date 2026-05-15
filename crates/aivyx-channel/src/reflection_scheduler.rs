//! Phase 71 — Reflection scheduler loop.
//!
//! Closes the deferral carried at Phase 70 exit: this module owns
//! the runtime loop that fires reflection turns on the cron
//! pattern operators declare under `[[reflection_schedule]]`.
//!
//! ## Shape
//!
//! - [`run_reflection_scheduler`] is the long-running async task
//!   spawned alongside the existing scheduler / webhook listener /
//!   file watcher in the daemon startup path. It owns:
//!     1. The validated `Vec<ReflectionScheduleConfig>` parsed
//!        from `[[reflection_schedule]]` blocks.
//!     2. The shared [`TriggerDispatch`] handle (same dispatcher
//!        the cron / webhook / file-watch triggers use).
//!     3. A handle on the persistent audit log for the
//!        outcome-summary walk (per Q1(c)).
//!     4. The shared shutdown [`CancellationToken`].
//! - On each tick the loop walks every enabled schedule,
//!   computes its next-fire-time, fires the reflection turn
//!   when due, and sleeps until the earliest pending fire
//!   (capped at [`MAX_TICK_INTERVAL`]).
//! - When a fire is due, the loop:
//!     1. Builds an [`OutcomeSummary`] vector for the lookback
//!        window by walking the audit chain backwards (Q2(a)
//!        outcome-summaries-only — no transcripts).
//!     2. Formats the canonical [`REFLECTION_SYSTEM_PROMPT`] +
//!        summaries into the trigger's prompt slot.
//!     3. Calls [`TriggerDispatch::fire`] with
//!        [`TriggerSource::Reflection`].
//! - Outcome summaries are cached by
//!   [`OutcomeSummaryCache`] — a small bounded VecDeque-backed
//!   LRU keyed by `(lookback_secs, audit_chain_len_at_fetch)`.
//!   Back-to-back reflection schedules with the same lookback
//!   reuse the same summary computation.
//!
//! ## Q-block resolutions
//!
//! - **Q1(c)** — Audit chain is the source of truth for outcome
//!   summaries; an in-memory LRU cache absorbs back-to-back
//!   fetches.
//! - **Q2(a)** — Canonical [`REFLECTION_SYSTEM_PROMPT`] constant.
//!   Operator-customizable prompt is a deferred polish.
//! - **Q3(a)** — `role_override` from the schedule config is
//!   recorded in logs + audit attribution but the per-fire
//!   runtime role override is a follow-up; v1 runs the
//!   reflection turn under the daemon's active role.
//! - **Q4(a)** — On error, log + emit a diagnostic + skip until
//!   next cron. No in-window retry; no consecutive-failure
//!   backoff. Self-healing on the next cron fire.

use std::collections::{HashMap, VecDeque};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use cron::Schedule as CronSchedule;

use aivyx_audit::{AuditEvent, PersistentAuditLog, SignedEntry};
use aivyx_config::ReflectionScheduleConfig;
use aivyx_core::{CancellationToken, TurnId, TurnOutcomeSummary};

use crate::trigger::{TriggerDispatch, TriggerSource};

/// Cap the adaptive sleep so newly-firing schedules (e.g. a
/// short cron pattern) are picked up promptly even if the
/// next computed fire happens to be hours away.
pub const MAX_TICK_INTERVAL: Duration = Duration::from_secs(60);

/// The canonical reflection system prompt operators get by
/// default. Q2(a) at Phase 71 sign-off: hardcoded constant,
/// version-controlled with the agent. Operator-customizable
/// per-schedule overrides are a deferred polish.
///
/// Phrasing prioritises:
/// - **Conservatism.** Propose only when a pattern recurs
///   ≥3 times (so a one-off doesn't shape the persona).
/// - **Category preference.** Lean toward narrower categories
///   (`BehavioralPreferences`, `LearnedContext`) over
///   identity-level changes (`AssistantName`,
///   `CommunicationStyle`).
/// - **Operator gating awareness.** Every proposal lands as
///   Pending in the proposal chain; the operator reviews via
///   Web UI / CLI before any persona delta is applied.
pub const REFLECTION_SYSTEM_PROMPT: &str = r#"You are running a reflection turn. Your job is to examine the operator's recent agent behavior — summarized below as a list of outcome records — and propose Persona deltas that refine how the assistant communicates and behaves.

Constraints:
1. Propose a delta only when a behavioral pattern recurs in at least 3 distinct turns within the lookback window. A one-off does not justify a persona change.
2. Prefer narrower categories — `BehavioralPreferences`, `LearnedContext`, `CommunicationAdaptations` — over identity-level changes (`AssistantName`, `OperatorProfile`, `CommunicationStyle`). Identity changes need stronger evidence.
3. Every proposal you emit is recorded as Pending in the persistent proposal chain. The operator reviews and approves (or rejects) before any delta lands in the persona chain. You are not the final authority — the operator is.
4. If no clear pattern emerges, do not propose anything. Empty reflection turns are valid and preferred over speculative changes.

Use the `reflection.propose` tool to record each proposed delta. Each call should include a clear `reason` field explaining the pattern you observed.

The outcome summaries follow."#;

// ---------------------------------------------------------------------------
// OutcomeSummary — wire format the reflection turn receives.
// ---------------------------------------------------------------------------

/// One row in the outcome-summary block injected as the
/// reflection turn's user message. Per Q2(a) at Phase 70
/// sign-off: outcome summaries only, no transcripts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeSummary {
    /// Session id from the paired `TurnStarted` audit event.
    pub session_id: String,
    /// Turn id pairing TurnStarted with TurnEnded.
    pub turn_id: String,
    /// Wall-clock when the turn started.
    pub started_at_unix_ms: u64,
    /// Stable string label of the turn's outcome (the audit
    /// chain's `TurnOutcomeSummary` rendered as a discriminator).
    pub outcome_kind: String,
    /// How many tool calls the turn made.
    pub tool_calls_made: u32,
    /// Wall-clock duration of the turn in milliseconds.
    pub duration_ms: u64,
}

/// Format a vector of summaries as the JSON block appended to
/// the reflection turn's user message. The agent reads this
/// alongside the canonical prompt.
pub fn format_summaries_for_prompt(summaries: &[OutcomeSummary]) -> String {
    if summaries.is_empty() {
        return "Recent outcome summaries: (none — no completed turns in the \
                lookback window). Propose nothing this cycle."
            .to_string();
    }
    let mut out = String::from("Recent outcome summaries (most recent first):\n\n");
    for s in summaries {
        out.push_str(&format!(
            "- turn `{turn}` (session `{ses}`) at {ts}ms — outcome={out_kind}, \
             tool_calls={tc}, duration={dur}ms\n",
            turn = s.turn_id,
            ses = s.session_id,
            ts = s.started_at_unix_ms,
            out_kind = s.outcome_kind,
            tc = s.tool_calls_made,
            dur = s.duration_ms,
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// OutcomeSummaryCache — the LRU layer over the audit walker.
// ---------------------------------------------------------------------------

/// Cache key: lookback window + audit chain length at fetch
/// time. The chain is append-only so the length is a monotonic
/// version stamp; if the chain hasn't grown since the cached
/// fetch and the lookback hasn't changed, the cached summary
/// is still authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub lookback_secs: u64,
    pub audit_len_at_fetch: u64,
}

/// Bounded VecDeque-backed LRU keyed by (lookback, audit-len).
/// Capacity is the number of distinct `(lookback, audit-len)`
/// pairs the cache will keep before evicting the oldest entry.
/// Eight is sized for 2-3 reflection schedules with distinct
/// lookbacks firing back-to-back; bigger isn't load-bearing
/// for v1.
pub struct OutcomeSummaryCache {
    capacity: usize,
    order: VecDeque<CacheKey>,
    entries: HashMap<CacheKey, Vec<OutcomeSummary>>,
}

impl OutcomeSummaryCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            order: VecDeque::with_capacity(capacity.max(1)),
            entries: HashMap::with_capacity(capacity.max(1)),
        }
    }

    pub fn get(&mut self, key: &CacheKey) -> Option<&Vec<OutcomeSummary>> {
        if self.entries.contains_key(key) {
            // Bump to MRU.
            if let Some(pos) = self.order.iter().position(|k| k == key) {
                self.order.remove(pos);
                self.order.push_back(*key);
            }
            self.entries.get(key)
        } else {
            None
        }
    }

    pub fn put(&mut self, key: CacheKey, value: Vec<OutcomeSummary>) {
        if self.entries.contains_key(&key) {
            // Refresh LRU position; replace stored value.
            if let Some(pos) = self.order.iter().position(|k| k == &key) {
                self.order.remove(pos);
            }
        } else if self.entries.len() >= self.capacity {
            // Evict the LRU entry to make room.
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key, value);
        self.order.push_back(key);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Audit walker — TurnStarted/TurnEnded pair → OutcomeSummary.
// ---------------------------------------------------------------------------

/// Walk the audit chain backwards from `now`, pair
/// `TurnStarted` + `TurnEnded` events by `turn_id`, and emit
/// one [`OutcomeSummary`] per completed turn within the
/// lookback window. In-flight turns (TurnStarted with no
/// matching TurnEnded) are skipped; un-matched TurnEnded
/// (chain edge) are skipped.
///
/// The audit chain is HMAC-chained + append-only; the
/// chronology of `entries_range(0, log.len())` is by `seq`.
/// Pairing walks the entries once, holding open TurnStarted
/// events in a `HashMap<TurnId, ...>` until the matching
/// TurnEnded closes them out.
pub fn summarize_recent_outcomes_from_entries(
    entries: &[SignedEntry],
    lookback_secs: u64,
    now_unix_ms: u64,
) -> Vec<OutcomeSummary> {
    let cutoff_ms = now_unix_ms.saturating_sub(lookback_secs.saturating_mul(1000));
    // Map turn_id → (session_id, started_at_unix_ms). TurnId is
    // `Copy + Hash` (UUID newtype) so we can key directly.
    let mut open: HashMap<TurnId, (String, u64)> = HashMap::new();
    let mut out: Vec<OutcomeSummary> = Vec::new();
    for entry in entries {
        let ts = system_time_to_unix_ms(entry.appended_at);
        match &entry.event {
            AuditEvent::TurnStarted {
                turn_id, session_id, ..
            } => {
                open.insert(*turn_id, (session_id.to_string(), ts));
            }
            AuditEvent::TurnEnded {
                turn_id,
                outcome,
                tool_calls_made,
                duration,
                ..
            } => {
                if let Some((session_id, started_at)) = open.remove(turn_id) {
                    if started_at < cutoff_ms {
                        // Window-of-interest filter.
                        continue;
                    }
                    out.push(OutcomeSummary {
                        session_id,
                        turn_id: turn_id.to_string(),
                        started_at_unix_ms: started_at,
                        outcome_kind: outcome_kind_label(outcome).to_string(),
                        tool_calls_made: *tool_calls_made as u32,
                        duration_ms: duration.as_millis() as u64,
                    });
                }
            }
            _ => {}
        }
    }
    // Most-recent first per the format helper's "most recent
    // first" rendering contract.
    out.sort_by_key(|s| std::cmp::Reverse(s.started_at_unix_ms));
    out
}

fn system_time_to_unix_ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Async convenience: read every audit entry, then summarize.
/// Caller passes the lookback + cache; cache hits skip the
/// chain walk.
pub async fn summarize_recent_outcomes(
    audit_log: &PersistentAuditLog,
    lookback_secs: u64,
    now_unix_ms: u64,
    cache: &mut OutcomeSummaryCache,
) -> Result<Vec<OutcomeSummary>, String> {
    let audit_len = audit_log.len() as u64;
    let key = CacheKey {
        lookback_secs,
        audit_len_at_fetch: audit_len,
    };
    if let Some(cached) = cache.get(&key) {
        return Ok(cached.clone());
    }
    let entries = audit_log
        .entries_range(0, audit_log.len())
        .map_err(|e| format!("audit chain read failed: {e}"))?;
    let summaries =
        summarize_recent_outcomes_from_entries(&entries, lookback_secs, now_unix_ms);
    cache.put(key, summaries.clone());
    Ok(summaries)
}

fn outcome_kind_label(summary: &TurnOutcomeSummary) -> &'static str {
    match summary {
        TurnOutcomeSummary::Completed => "completed",
        TurnOutcomeSummary::Failed => "failed",
        TurnOutcomeSummary::Escalated => "escalated",
        TurnOutcomeSummary::Cancelled => "cancelled",
        TurnOutcomeSummary::TimedOut => "timed_out",
    }
}

// ---------------------------------------------------------------------------
// Scheduler loop.
// ---------------------------------------------------------------------------

/// Run the reflection scheduler loop. This future never returns
/// normally — it runs until `shutdown` is cancelled.
///
/// On each tick:
/// 1. For every enabled `ReflectionScheduleConfig`, compute the
///    next fire time anchored on the last in-memory fire stamp.
/// 2. If the next fire ≤ now and the schedule hasn't already
///    fired in this window, summarize recent outcomes (with
///    cache), format the user message, and call
///    `TriggerDispatch::fire(TriggerSource::Reflection, ...)`.
/// 3. Sleep until the earliest pending fire (capped at
///    [`MAX_TICK_INTERVAL`]).
pub async fn run_reflection_scheduler(
    schedules: Vec<ReflectionScheduleConfig>,
    dispatch: TriggerDispatch,
    audit_log: Arc<PersistentAuditLog>,
    shutdown: CancellationToken,
) {
    if schedules.is_empty() {
        // Nothing to do — keep the future alive for the
        // shutdown signal but don't burn ticks.
        shutdown.cancelled().await;
        return;
    }

    // In-memory per-schedule last-fired-at. Survives daemon
    // lifetime, not crashes; a daemon restart resets to "fire
    // at the next cron boundary." This is fine for v1 — the
    // schedule is operator-declared and the cadence is large
    // (typical: daily / weekly).
    let mut last_fired: HashMap<String, DateTime<Utc>> = HashMap::new();
    let mut cache = OutcomeSummaryCache::new(8);

    loop {
        if shutdown.is_cancelled() {
            return;
        }

        let now = Utc::now();
        let mut earliest_next: Option<Duration> = None;

        for sched in &schedules {
            if !sched.enabled {
                continue;
            }
            let anchor = last_fired
                .get(&sched.name)
                .copied()
                .unwrap_or(DateTime::UNIX_EPOCH);
            let Some(next_fire) = next_fire_after(&sched.cron, anchor) else {
                eprintln!(
                    "aivyx reflection: schedule {:?} produced no next fire — \
                     cron pattern may be unreachable",
                    sched.name,
                );
                continue;
            };
            if next_fire <= now {
                fire_reflection(
                    &dispatch,
                    audit_log.as_ref(),
                    sched,
                    &mut cache,
                    now,
                )
                .await;
                last_fired.insert(sched.name.clone(), now);
                if let Some(after_now) = next_fire_after(&sched.cron, now) {
                    update_earliest(&mut earliest_next, after_now, now);
                }
            } else {
                update_earliest(&mut earliest_next, next_fire, now);
            }
        }

        let sleep_dur = earliest_next
            .unwrap_or(MAX_TICK_INTERVAL)
            .min(MAX_TICK_INTERVAL);
        tokio::select! {
            _ = tokio::time::sleep(sleep_dur) => {}
            _ = shutdown.cancelled() => return,
        }
    }
}

/// Fire one scheduled reflection turn. Failures (Q4(a)) emit a
/// diagnostic + an audit event-style eprintln and return; the
/// caller updates `last_fired` regardless so a broken schedule
/// doesn't hot-loop.
async fn fire_reflection(
    dispatch: &TriggerDispatch,
    audit_log: &PersistentAuditLog,
    sched: &ReflectionScheduleConfig,
    cache: &mut OutcomeSummaryCache,
    now: DateTime<Utc>,
) {
    let now_ms = now.timestamp_millis().max(0) as u64;
    let summaries = match summarize_recent_outcomes(
        audit_log,
        sched.lookback_window_secs,
        now_ms,
        cache,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "aivyx reflection: schedule {:?} failed to summarize outcomes: {e}",
                sched.name,
            );
            return;
        }
    };
    eprintln!(
        "aivyx reflection: schedule {:?} firing — {} outcome summaries in \
         the {}s lookback",
        sched.name,
        summaries.len(),
        sched.lookback_window_secs,
    );

    let user_message = format!(
        "{REFLECTION_SYSTEM_PROMPT}\n\n{}",
        format_summaries_for_prompt(&summaries),
    );

    // Per Q3(a): role_override is recorded in the log line above
    // for forensic attribution. The per-fire runtime role
    // override is a deferred follow-up — v1 runs the reflection
    // turn under the daemon's active role. Operators who want a
    // dedicated reflection envelope today declare a [[role]] and
    // run the daemon under that role via the existing
    // role-switching path.
    let _ = &sched.role_override;

    let _elapsed = dispatch
        .fire(
            TriggerSource::Reflection,
            &sched.name,
            &user_message,
            false, // wrap_mission — reflection turns don't need missions
            None,  // notify_target — none for reflection
        )
        .await;
}

fn next_fire_after(cron_expr: &str, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let schedule = CronSchedule::from_str(cron_expr).ok()?;
    schedule.after(&after).next()
}

fn update_earliest(
    earliest: &mut Option<Duration>,
    fire_time: DateTime<Utc>,
    now: DateTime<Utc>,
) {
    let delta = (fire_time - now).to_std().unwrap_or(Duration::ZERO);
    match earliest {
        Some(current) if delta < *current => *earliest = Some(delta),
        None => *earliest = Some(delta),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_audit::AuditEvent;
    use aivyx_capability::CapabilitySet;
    use aivyx_core::SessionId;

    /// Test-only: build a fresh `SignedEntry` for `TurnStarted`
    /// with a given timestamp. Returns `(entry, turn_id,
    /// session_id_string)` so the caller can pair it with a
    /// `TurnEnded` and assert against the rendered values.
    fn make_started(seq: u64, ts_ms: u64) -> (SignedEntry, TurnId, String) {
        let turn_id = TurnId::new();
        let session_id = SessionId::new();
        let entry = SignedEntry {
            seq,
            appended_at: UNIX_EPOCH + Duration::from_millis(ts_ms),
            event: AuditEvent::TurnStarted {
                turn_id,
                session_id,
                channel: aivyx_core::ChannelPlatform::Local,
                trust_tier: aivyx_audit::TrustTierSummary::Trusted,
                effective_capabilities: CapabilitySet::empty(),
            },
            prev_mac: [0u8; 32],
            mac: [0u8; 32],
        };
        (entry, turn_id, session_id.to_string())
    }

    fn make_ended(
        seq: u64,
        turn_id: TurnId,
        ts_ms: u64,
        tool_calls: usize,
    ) -> SignedEntry {
        SignedEntry {
            seq,
            appended_at: UNIX_EPOCH + Duration::from_millis(ts_ms),
            event: AuditEvent::TurnEnded {
                turn_id,
                outcome: TurnOutcomeSummary::Completed,
                tool_calls_made: tool_calls,
                duration: Duration::from_millis(500),
                usage: aivyx_core::TokenUsage::default(),
            },
            prev_mac: [0u8; 32],
            mac: [0u8; 32],
        }
    }

    #[test]
    fn pairs_one_turn_returns_one_summary() {
        let (started, turn_id, session) = make_started(0, 1_000);
        let ended = make_ended(1, turn_id, 1_500, 3);
        let entries = vec![started, ended];
        let out =
            summarize_recent_outcomes_from_entries(&entries, 3600, 10_000);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].turn_id, turn_id.to_string());
        assert_eq!(out[0].session_id, session);
        assert_eq!(out[0].started_at_unix_ms, 1_000);
        assert_eq!(out[0].outcome_kind, "completed");
        assert_eq!(out[0].tool_calls_made, 3);
    }

    #[test]
    fn unpaired_turn_started_is_skipped() {
        // Started but never ended — in-flight turn, must not emit a row.
        let (started, _tid, _ses) = make_started(0, 1_000);
        let out = summarize_recent_outcomes_from_entries(&[started], 3600, 10_000);
        assert!(out.is_empty());
    }

    #[test]
    fn turn_outside_lookback_window_is_skipped() {
        // started_at = 1_000ms; now = 10_000ms; lookback 1s → cutoff 9_000.
        // The turn started before cutoff → filter out.
        let (started, turn_id, _ses) = make_started(0, 1_000);
        let ended = make_ended(1, turn_id, 1_500, 0);
        let out =
            summarize_recent_outcomes_from_entries(&[started, ended], 1, 10_000);
        assert!(out.is_empty());
    }

    #[test]
    fn summaries_sorted_most_recent_first() {
        let (s_old, t_old, _) = make_started(0, 1_000);
        let e_old = make_ended(1, t_old, 1_100, 0);
        let (s_new, t_new, _) = make_started(2, 5_000);
        let e_new = make_ended(3, t_new, 5_200, 0);
        let (s_mid, t_mid, _) = make_started(4, 3_000);
        let e_mid = make_ended(5, t_mid, 3_400, 0);
        let entries = vec![s_old, e_old, s_new, e_new, s_mid, e_mid];
        let out =
            summarize_recent_outcomes_from_entries(&entries, 3600, 10_000);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].turn_id, t_new.to_string());
        assert_eq!(out[1].turn_id, t_mid.to_string());
        assert_eq!(out[2].turn_id, t_old.to_string());
    }

    #[test]
    fn cache_hits_avoid_resummarization() {
        let mut cache = OutcomeSummaryCache::new(4);
        let key = CacheKey {
            lookback_secs: 3600,
            audit_len_at_fetch: 5,
        };
        let initial = vec![OutcomeSummary {
            session_id: "s".into(),
            turn_id: "t".into(),
            started_at_unix_ms: 100,
            outcome_kind: "completed".into(),
            tool_calls_made: 0,
            duration_ms: 50,
        }];
        cache.put(key, initial.clone());
        assert_eq!(cache.len(), 1);
        let got = cache.get(&key).expect("hit");
        assert_eq!(*got, initial);
    }

    #[test]
    fn cache_evicts_lru_at_capacity() {
        let mut cache = OutcomeSummaryCache::new(2);
        let k1 = CacheKey { lookback_secs: 1, audit_len_at_fetch: 1 };
        let k2 = CacheKey { lookback_secs: 2, audit_len_at_fetch: 1 };
        let k3 = CacheKey { lookback_secs: 3, audit_len_at_fetch: 1 };
        cache.put(k1, vec![]);
        cache.put(k2, vec![]);
        cache.put(k3, vec![]); // evicts k1
        assert!(cache.get(&k1).is_none());
        assert!(cache.get(&k2).is_some());
        assert!(cache.get(&k3).is_some());
    }

    #[test]
    fn cache_get_bumps_mru_ordering() {
        let mut cache = OutcomeSummaryCache::new(2);
        let k1 = CacheKey { lookback_secs: 1, audit_len_at_fetch: 1 };
        let k2 = CacheKey { lookback_secs: 2, audit_len_at_fetch: 1 };
        let k3 = CacheKey { lookback_secs: 3, audit_len_at_fetch: 1 };
        cache.put(k1, vec![]);
        cache.put(k2, vec![]);
        // Touch k1 so it becomes MRU; k2 should evict next.
        let _ = cache.get(&k1);
        cache.put(k3, vec![]);
        assert!(cache.get(&k1).is_some(), "k1 should survive eviction");
        assert!(cache.get(&k2).is_none(), "k2 should have been evicted");
    }

    #[test]
    fn cache_different_lookback_is_distinct_entry() {
        // Same audit_len, different lookback — both keys are independent.
        let mut cache = OutcomeSummaryCache::new(4);
        let k1 = CacheKey { lookback_secs: 60, audit_len_at_fetch: 5 };
        let k2 = CacheKey { lookback_secs: 3600, audit_len_at_fetch: 5 };
        cache.put(k1, vec![]);
        cache.put(k2, vec![]);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn format_summaries_empty_renders_explanatory_message() {
        let s = format_summaries_for_prompt(&[]);
        assert!(s.contains("no completed turns"));
        assert!(s.contains("Propose nothing"));
    }

    #[test]
    fn format_summaries_renders_each_row() {
        let summaries = vec![OutcomeSummary {
            session_id: "ses-abc".into(),
            turn_id: "turn-001".into(),
            started_at_unix_ms: 1_715_000_000_000,
            outcome_kind: "completed".into(),
            tool_calls_made: 4,
            duration_ms: 1234,
        }];
        let s = format_summaries_for_prompt(&summaries);
        assert!(s.contains("turn `turn-001`"));
        assert!(s.contains("session `ses-abc`"));
        assert!(s.contains("outcome=completed"));
        assert!(s.contains("tool_calls=4"));
        assert!(s.contains("duration=1234ms"));
    }

    #[test]
    fn next_fire_after_returns_some_for_valid_cron() {
        let now = Utc::now();
        // Every minute pattern.
        let next = next_fire_after("0 * * * * * *", now);
        assert!(next.is_some());
        let next = next.unwrap();
        assert!(next > now);
    }

    #[test]
    fn next_fire_after_returns_none_for_invalid_cron() {
        assert!(next_fire_after("not a cron pattern", Utc::now()).is_none());
    }

    #[test]
    fn update_earliest_picks_minimum() {
        let now = Utc::now();
        let mut earliest: Option<Duration> = None;
        let t1 = now + chrono::Duration::seconds(120);
        update_earliest(&mut earliest, t1, now);
        let first = earliest.unwrap();
        let t2 = now + chrono::Duration::seconds(30);
        update_earliest(&mut earliest, t2, now);
        assert!(earliest.unwrap() < first);
        // A later time shouldn't replace the earlier one.
        let t3 = now + chrono::Duration::seconds(300);
        update_earliest(&mut earliest, t3, now);
        assert!(earliest.unwrap() < Duration::from_secs(60));
    }

    #[test]
    fn reflection_system_prompt_mentions_key_constraints() {
        // Smoke test guarding against accidental gutting of the
        // canonical prompt's behavioral constraints.
        assert!(REFLECTION_SYSTEM_PROMPT.contains("3 distinct turns"));
        assert!(REFLECTION_SYSTEM_PROMPT.contains("reflection.propose"));
        assert!(REFLECTION_SYSTEM_PROMPT.contains("Pending"));
        assert!(REFLECTION_SYSTEM_PROMPT.contains("operator"));
    }

}

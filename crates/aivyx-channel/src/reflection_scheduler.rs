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

/// Phase 77 — handles the recall→reflection feedback pass needs.
/// Bundled so `run_reflection_scheduler`'s signature doesn't grow
/// per-handle. `None` (no `[embedding]` / no recall substrate) →
/// the feedback pass is skipped entirely (pre-Phase-77 behavior).
pub struct RecallFeedbackDeps {
    pub recall_log: std::sync::Arc<crate::recall_log::PersistentRecallLog>,
    pub memory: std::sync::Arc<dyn aivyx_memory::Memory>,
    pub proposal_log:
        std::sync::Arc<crate::persona_proposal::PersistentPersonaProposalLog>,
    /// Retention window for the recall-log GC clamp (seconds).
    pub gc_retain_secs: u64,
    /// Phase 82 — the durable helpfulness ledger. The pass
    /// folds each window's per-topic net into it (after
    /// `correlate`, so Actuators A/B are untouched) and prunes
    /// on the same cadence. `None` → no fold (the ledger is a
    /// passive add-on; recall-feedback is unaffected).
    pub helpfulness_ledger: Option<
        std::sync::Arc<
            crate::helpfulness_ledger::PersistentHelpfulnessLedger,
        >,
    >,
    /// Phase 83 — the durable cross-session co-occurrence
    /// ledger. The pass folds each window's per-pair net into
    /// it (after the Phase 82 fold, so recall-feedback + the
    /// helpfulness ledger are byte-identical) and prunes on the
    /// same cadence. `None` → no fold (a passive add-on).
    pub cooccurrence_ledger: Option<
        std::sync::Arc<
            crate::cooccurrence_ledger::PersistentCooccurrenceLedger,
        >,
    >,
}

/// Phase 80 — handles the proactive-surfacing pass needs.
/// Bundled like [`RecallFeedbackDeps`]. `None` (no
/// `[proactive]`) → the pass is skipped entirely (pre-Phase-80
/// behavior). Even when `Some`, the pass no-ops unless
/// `config.enabled`.
pub struct ProactiveDeps {
    pub config: aivyx_config::ProactiveConfig,
    pub memory: std::sync::Arc<dyn aivyx_memory::Memory>,
    pub proactive_log:
        std::sync::Arc<crate::proactive_log::PersistentProactiveLog>,
    pub notify:
        std::sync::Arc<crate::notify_dispatcher::NotifyDispatcher>,
    /// Optional recall-feedback log: only the `RecallCluster`
    /// signal needs the Phase 77 tally; absent → that signal
    /// simply never fires, the other two still do.
    pub recall_log: Option<
        std::sync::Arc<crate::recall_log::PersistentRecallLog>,
    >,
    /// Global memory TTL (drives the `TtlExpiry` signal).
    pub memory_ttl_secs: Option<u64>,
    /// Retention window for the proactive-log GC clamp.
    pub gc_retain_secs: u64,
    /// Phase 80 (Q4a) — optional last-cycle stat sink for the
    /// Phase 78 surface. `None` → breadcrumb-only.
    pub stat: Option<crate::proactive_detect::SharedProactiveStat>,
}

/// Phase 81 — handles the Persona-lifecycle pass needs.
/// Bundled like [`ProactiveDeps`]. `None` (no
/// `[persona_lifecycle]` / no persona substrate) → the pass is
/// skipped entirely (pre-Phase-81 behavior — the Soul only
/// ever grows). Even when `Some`, the pass no-ops unless
/// `config.enabled`.
pub struct PersonaLifecycleDeps {
    pub config: aivyx_config::PersonaLifecycleConfig,
    pub persona_log:
        std::sync::Arc<crate::persona::PersistentPersonaLog>,
    pub proposal_log: std::sync::Arc<
        crate::persona_proposal::PersistentPersonaProposalLog,
    >,
    pub embedding: std::sync::Arc<
        dyn aivyx_llm::embedding::EmbeddingProvider,
    >,
    /// Phase 85 — the durable helpfulness ledger. When present,
    /// a facet whose `recall-fb:{topic}` provenance resolves is
    /// gated by that topic's decayed helpfulness (symmetric:
    /// sustained-negative triggers decay early, sustained-
    /// positive protects an age-old facet). `None` → pure
    /// age-only decay (byte-identical to Phase 81).
    pub helpfulness_ledger: Option<
        std::sync::Arc<
            crate::helpfulness_ledger::PersistentHelpfulnessLedger,
        >,
    >,
    /// Phase 81 (Q4a) — optional last-cycle stat sink for the
    /// Phase 78 surface. `None` → breadcrumb-only.
    pub stat: Option<
        crate::persona_lifecycle::SharedPersonaLifecycleStat,
    >,
}

/// Phase 87 — handles the consolidation pass needs. Bundled
/// like [`PersonaLifecycleDeps`]. `None` (no
/// `[persona_consolidation]` / no co-occurrence + helpfulness
/// substrate) → the pass is skipped entirely (pre-Phase-87
/// behavior — no pattern-driven proposals). Even when `Some`,
/// the pass no-ops unless `config.enabled`.
pub struct PersonaConsolidationDeps {
    pub config: aivyx_config::PersonaConsolidationConfig,
    pub cooccurrence_ledger: std::sync::Arc<
        crate::cooccurrence_ledger::PersistentCooccurrenceLedger,
    >,
    pub helpfulness_ledger: std::sync::Arc<
        crate::helpfulness_ledger::PersistentHelpfulnessLedger,
    >,
    pub proposal_log: std::sync::Arc<
        crate::persona_proposal::PersistentPersonaProposalLog,
    >,
    pub phraser: std::sync::Arc<
        dyn crate::persona_consolidation::PairPhraser,
    >,
    /// Phase 87 (Q4a) — optional last-cycle stat sink for the
    /// Phase 78 surface. `None` → breadcrumb-only.
    pub stat: Option<
        crate::persona_consolidation::SharedPersonaConsolidationStat,
    >,
}

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
#[allow(clippy::too_many_arguments)]
pub async fn run_reflection_scheduler(
    schedules: Vec<ReflectionScheduleConfig>,
    dispatch: TriggerDispatch,
    audit_log: Arc<PersistentAuditLog>,
    recall_feedback: Option<RecallFeedbackDeps>,
    proactive: Option<ProactiveDeps>,
    persona_lifecycle: Option<PersonaLifecycleDeps>,
    persona_consolidation: Option<PersonaConsolidationDeps>,
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
                    recall_feedback.as_ref(),
                    proactive.as_ref(),
                    persona_lifecycle.as_ref(),
                    persona_consolidation.as_ref(),
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
#[allow(clippy::too_many_arguments)]
async fn fire_reflection(
    dispatch: &TriggerDispatch,
    audit_log: &PersistentAuditLog,
    sched: &ReflectionScheduleConfig,
    cache: &mut OutcomeSummaryCache,
    now: DateTime<Utc>,
    recall_feedback: Option<&RecallFeedbackDeps>,
    proactive: Option<&ProactiveDeps>,
    persona_lifecycle: Option<&PersonaLifecycleDeps>,
    persona_consolidation: Option<&PersonaConsolidationDeps>,
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

    // Phase 77 — recall→reflection feedback, on the very same
    // cadence (Q4a). Independent of the LLM reflection turn
    // below; runs over the same lookback window the summaries
    // were built from. Entirely no-op when the deps are absent.
    if let Some(deps) = recall_feedback {
        run_recall_feedback_pass(deps, sched, &summaries, now_ms).await;
    }

    // Phase 80 — proactive surfacing on the same cadence (Q1a).
    // Independent of recall-feedback; no-op when absent or
    // disabled.
    if let Some(deps) = proactive {
        run_proactive_pass(deps, sched, &summaries, now_ms).await;
    }

    // Phase 81 — Persona lifecycle on the same cadence (Q1a).
    // Independent of the above; no-op when absent or disabled.
    // It only files Pending proposals — never resolves them.
    if let Some(deps) = persona_lifecycle {
        run_persona_lifecycle_pass(deps, sched, now_ms).await;
    }

    // Phase 87 — pattern-driven Persona consolidation on the
    // same cadence (Q3a). Independent of the above; no-op when
    // absent or disabled. Files Pending proposals only (same
    // Phase 70 propose-only + edit-then-approve flow).
    if let Some(deps) = persona_consolidation {
        run_persona_consolidation_pass(deps, sched, now_ms).await;
    }

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
            false,                             // wrap_mission — reflection turns don't need missions
            &[],                               // notify_targets — none for reflection
            aivyx_config::NotifyWhen::Always,  // unused (no targets) but the signature requires it
        )
        .await;
}

/// The recall→reflection feedback pass (Tasks 5–7), driven on
/// the reflection cadence. Reads the same lookback window as the
/// outcome summaries, correlates, applies the retention bias,
/// files any operator-gated proposals, then GC-clamps the
/// recall log. Every step is best-effort: a read/append error
/// is logged and the cycle continues — learning degrades, the
/// reflection turn and recall itself are untouched.
async fn run_recall_feedback_pass(
    deps: &RecallFeedbackDeps,
    sched: &ReflectionScheduleConfig,
    summaries: &[OutcomeSummary],
    now_ms: u64,
) {
    let now_secs = now_ms / 1000;
    let since = now_secs.saturating_sub(sched.lookback_window_secs);

    match deps.recall_log.events_since(since).await {
        Ok(recalls) if !recalls.is_empty() => {
            let tally = crate::recall_feedback::correlate(
                &recalls, summaries,
            );
            if !tally.is_empty() {
                let promoted =
                    crate::recall_feedback::apply_retention_feedback(
                        &deps.memory,
                        &tally,
                    )
                    .await;
                let filed =
                    crate::recall_feedback::emit_persona_proposals(
                        &deps.proposal_log,
                        &tally,
                        now_ms,
                        &format!("reflection:{}", sched.name),
                    )
                    .await;
                eprintln!(
                    "aivyx recall-feedback: schedule {:?} — {} entr{} \
                     scored, {promoted} promoted, {filed} proposal(s) \
                     filed",
                    sched.name,
                    tally.len(),
                    if tally.len() == 1 { "y" } else { "ies" },
                );

                // Phase 82 — fold this window's per-topic net
                // into the durable ledger (after the actuators,
                // so recall-feedback is byte-identical). The
                // ledger is a passive longitudinal signal:
                // absent → skipped, present → it never changes
                // recall-feedback behaviour.
                if let Some(ledger) = &deps.helpfulness_ledger {
                    let mut net: std::collections::HashMap<
                        String,
                        f32,
                    > = std::collections::HashMap::new();
                    for (topic, _seq, score) in tally.ranked() {
                        *net.entry(topic).or_insert(0.0) +=
                            score;
                    }
                    let net_by_topic: Vec<(String, f32)> =
                        net.into_iter().collect();
                    let folded = net_by_topic.len();
                    if let Err(e) = ledger
                        .record_window(&net_by_topic, now_secs)
                        .await
                    {
                        eprintln!(
                            "aivyx helpfulness-ledger: schedule \
                             {:?} fold error: {e}",
                            sched.name,
                        );
                    } else {
                        let pruned = ledger
                            .prune(now_secs)
                            .await
                            .unwrap_or(0);
                        eprintln!(
                            "aivyx helpfulness-ledger: folded \
                             {folded} topic(s), pruned {pruned}",
                        );
                    }
                }

                // Phase 83 — fold this window's co-occurring
                // topic pairs into the durable cross-session
                // ledger. After the Phase 82 fold, so
                // recall-feedback AND the helpfulness ledger
                // are byte-identical; a passive add-on,
                // absent → skipped.
                if let Some(cooc) = &deps.cooccurrence_ledger {
                    let detail =
                        crate::recall_feedback::correlate_detailed(
                            &recalls, summaries,
                        )
                        .1;
                    let mut pair_net: std::collections::HashMap<
                        (String, String),
                        f32,
                    > = std::collections::HashMap::new();
                    for (event, contrib) in
                        recalls.iter().zip(detail.iter())
                    {
                        let Some(sig) = contrib.signal else {
                            continue;
                        };
                        // Top-K highest-scoring DISTINCT
                        // topics for this event (the Q4a
                        // deterministic O(n²) bound). Phase 84
                        // (Q3a) self-policing: cluster-injected
                        // hits are EXCLUDED so the co-occurrence
                        // ledger only ever learns from organic
                        // keyword/semantic co-recall — never
                        // from its own expansion (no runaway
                        // self-reinforcement). They still count
                        // in the Phase 77/82 helpfulness signal
                        // (a bad expansion self-penalises).
                        let mut hits: Vec<
                            &crate::recall_log::RecallHit,
                        > = event
                            .hits
                            .iter()
                            .filter(|h| !h.cluster)
                            .collect();
                        hits.sort_by(|a, b| {
                            b.score
                                .partial_cmp(&a.score)
                                .unwrap_or(
                                    std::cmp::Ordering::Equal,
                                )
                        });
                        let mut topics: Vec<String> =
                            Vec::new();
                        for h in hits {
                            if !topics
                                .iter()
                                .any(|t| t == &h.topic)
                            {
                                topics.push(h.topic.clone());
                                if topics.len() >= crate::cooccurrence_ledger::COOCCURRENCE_TOP_K_HITS
                                {
                                    break;
                                }
                            }
                        }
                        // Distinct unordered pairs,
                        // canonicalised so {A,B} and {B,A}
                        // accumulate into one in-window bucket.
                        for i in 0..topics.len() {
                            for j in (i + 1)..topics.len() {
                                let (lo, hi) = if topics[i]
                                    <= topics[j]
                                {
                                    (
                                        topics[i].clone(),
                                        topics[j].clone(),
                                    )
                                } else {
                                    (
                                        topics[j].clone(),
                                        topics[i].clone(),
                                    )
                                };
                                *pair_net
                                    .entry((lo, hi))
                                    .or_insert(0.0) += sig;
                            }
                        }
                    }
                    let pairs: Vec<((String, String), f32)> =
                        pair_net.into_iter().collect();
                    let folded = pairs.len();
                    if folded > 0 {
                        if let Err(e) = cooc
                            .record_window(&pairs, now_secs)
                            .await
                        {
                            eprintln!(
                                "aivyx cooccurrence: schedule \
                                 {:?} fold error: {e}",
                                sched.name,
                            );
                        } else {
                            let pruned = cooc
                                .prune(now_secs)
                                .await
                                .unwrap_or(0);
                            eprintln!(
                                "aivyx cooccurrence: folded \
                                 {folded} pair(s), pruned \
                                 {pruned}",
                            );
                        }
                    }
                }
            }
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!(
                "aivyx recall-feedback: schedule {:?} recall-log read \
                 error: {e}",
                sched.name,
            );
        }
    }

    // Bounded-growth clamp on the same cadence.
    let cutoff = now_secs.saturating_sub(deps.gc_retain_secs);
    match deps.recall_log.gc_older_than(cutoff).await {
        Ok(n) if n > 0 => {
            eprintln!(
                "aivyx recall-feedback: gc clamped {n} old recall \
                 event(s)"
            );
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!(
                "aivyx recall-feedback: recall-log gc error: {e}"
            );
        }
    }
}

/// Phase 80 — the proactive-surfacing pass, on the reflection
/// cadence (Q1a). Detect structurally, drop already-surfaced
/// (cross-cycle dedup) and over-cap items, dispatch survivors
/// through the existing notify dispatcher, record + GC-clamp.
/// Every step best-effort: a failure is logged and the cycle
/// continues — the reflection turn is untouched, and a failed
/// dispatch is *not* marked surfaced so it retries next cycle.
async fn run_proactive_pass(
    deps: &ProactiveDeps,
    sched: &ReflectionScheduleConfig,
    summaries: &[OutcomeSummary],
    now_ms: u64,
) {
    if !deps.config.enabled {
        return;
    }
    let now_secs = now_ms / 1000;

    // Enumerate memory without perturbing LRU (scan_prefix does
    // not stamp last_read, unlike get_recent).
    let entries: Vec<aivyx_memory::MemoryEntry> = match deps
        .memory
        .scan_prefix("", usize::MAX)
        .await
    {
        Ok(groups) => {
            groups.into_iter().flat_map(|(_t, es)| es).collect()
        }
        Err(e) => {
            eprintln!(
                "aivyx proactive: schedule {:?} memory scan \
                 error: {e}",
                sched.name,
            );
            return;
        }
    };

    // The RecallCluster signal needs the Phase 77 tally; the
    // other two don't. No recall log → empty tally → that
    // signal simply never fires.
    let tally = match &deps.recall_log {
        Some(rl) => {
            let since = now_secs
                .saturating_sub(sched.lookback_window_secs);
            match rl.events_since(since).await {
                Ok(recalls) => {
                    crate::recall_feedback::correlate(
                        &recalls, summaries,
                    )
                }
                Err(_) => {
                    crate::recall_feedback::HelpfulnessTally::default()
                }
            }
        }
        None => crate::recall_feedback::HelpfulnessTally::default(),
    };

    let items = crate::proactive_detect::detect(
        &entries,
        &tally,
        &deps.config.signals,
        deps.memory_ttl_secs,
        now_secs,
    );

    // Hard per-window cap (Q4a) — the deterministic volume
    // guard on top of any per-target rate-limit.
    let window_start =
        now_secs.saturating_sub(deps.config.window_secs);
    let used = deps
        .proactive_log
        .count_since(window_start)
        .await
        .unwrap_or(0);
    let mut remaining =
        deps.config.max_per_window.saturating_sub(used);

    let (mut surfaced, mut deduped, mut capped) = (0u32, 0u32, 0u32);
    let mut surfaced_items: Vec<
        crate::proactive_detect::ProactiveSurfaced,
    > = Vec::new();
    for item in items {
        match deps.proactive_log.was_surfaced(&item.id).await {
            Ok(true) => {
                deduped += 1;
                continue;
            }
            Ok(false) => {}
            Err(_) => continue, // log read error → skip safely
        }
        if remaining == 0 {
            capped += 1;
            continue;
        }
        let subject = format!("Aivyx — proactive ({:?})", item.kind);
        let body = format!("{}\n\n(why: {})", item.summary, item.reason);
        match deps
            .notify
            .dispatch(&deps.config.target, &body, Some(&subject))
            .await
        {
            Ok(()) => {
                // Only mark on success → a failed send retries.
                let _ = deps
                    .proactive_log
                    .mark_surfaced(&item.id, now_secs)
                    .await;
                surfaced += 1;
                remaining -= 1;
                surfaced_items.push(
                    crate::proactive_detect::ProactiveSurfaced {
                        kind: item.kind,
                        topic: item.topic.clone(),
                        reason: item.reason.clone(),
                    },
                );
            }
            Err(e) => {
                eprintln!(
                    "aivyx proactive: dispatch to {:?} failed: {e}",
                    deps.config.target,
                );
            }
        }
    }

    if surfaced > 0 || deduped > 0 || capped > 0 {
        eprintln!(
            "aivyx proactive: schedule {:?} — surfaced {surfaced} \
             (deduped {deduped}, capped {capped})",
            sched.name,
        );
        // Q4a — record this cycle for the Phase 78 surface.
        if let Some(stat) = &deps.stat {
            if let Ok(mut w) = stat.write() {
                *w = Some(crate::proactive_detect::ProactiveStat {
                    ts_secs: now_secs,
                    surfaced: surfaced_items,
                    deduped,
                    capped,
                });
            }
        }
    }

    let cutoff = now_secs.saturating_sub(deps.gc_retain_secs);
    if let Ok(n) = deps.proactive_log.gc_older_than(cutoff).await {
        if n > 0 {
            eprintln!(
                "aivyx proactive: gc clamped {n} old dedup row(s)"
            );
        }
    }
}

/// The Phase 81 Persona-lifecycle pass, driven on the
/// reflection cadence (Q1a). Folds the persona chain, builds
/// per-soft-facet provenance (origin ts + whether a later
/// delta in the same category reinforced it), runs the
/// structural detector, and **files each surviving action as a
/// Pending `PersonaProposal`** — it never resolves anything
/// (Q2a: the operator approves/rejects, every action is
/// `Revert`-able). Cross-cycle dedup is by the deterministic
/// proposal id: an id already present in the proposal chain
/// (any status, including Rejected) is never re-filed — the
/// assistant must not nag about its own identity. Every step
/// is best-effort: an error is logged and the cycle continues;
/// the reflection turn and the persona chain are untouched.
async fn run_persona_lifecycle_pass(
    deps: &PersonaLifecycleDeps,
    sched: &ReflectionScheduleConfig,
    now_ms: u64,
) {
    if !deps.config.enabled {
        return;
    }
    let now_secs = now_ms / 1000;

    // Snapshot the persona chain (sync, in-memory) and fold it.
    let entries = deps.persona_log.entries();
    if entries.is_empty() {
        return;
    }
    let persona =
        crate::persona::compute_effective_persona(&entries);

    // Build per-facet provenance. `soft_facets_of` is the
    // core-protection choke point — only the six soft lists,
    // never the scalars or behavioral_constraints.
    let mut facets: Vec<crate::persona_lifecycle::LifecycleFacet> =
        Vec::new();
    for (cat, value) in
        crate::persona_lifecycle::soft_facets_of(&persona)
    {
        let dcat = cat.to_delta_category();
        // The latest AppendList that put this exact value into
        // effect for this category is the facet's origin.
        let Some(origin) = entries.iter().rev().find(|e| {
            e.delta.category == dcat
                && matches!(
                    &e.delta.op,
                    crate::persona::PersonaDeltaOp::AppendList {
                        value: v,
                    } if *v == value
                )
        }) else {
            continue;
        };
        // Reinforced = any *later* delta touched the same
        // category (active curation → do not decay it).
        let reinforced = entries
            .iter()
            .any(|e| e.seq > origin.seq && e.delta.category == dcat);
        // Phase 85 (Q1a) — recover the recall topic structurally
        // from `recall-fb:{topic}` provenance. Only
        // recall-feedback-derived facets carry it; reflection-
        // authored facets → `None` → age-only (unchanged).
        let recall_topic = origin
            .delta
            .proposal_id
            .strip_prefix("recall-fb:")
            .map(|t| t.to_string());
        // Resolve the topic's durable decayed helpfulness once,
        // here, so the detector stays pure. `None` when no
        // ledger, no provenance, or the topic is unseen → the
        // detector falls back to exact Phase 81 age-only.
        let helpfulness = match (
            &deps.helpfulness_ledger,
            recall_topic.as_deref(),
        ) {
            (Some(ledger), Some(topic)) => match ledger
                .topic_score(topic, now_secs)
                .await
            {
                Ok(Some(e)) => Some(
                    crate::persona_lifecycle::HelpfulnessHint {
                        score: e.ewma_score,
                        samples: e.samples,
                    },
                ),
                _ => None,
            },
            _ => None,
        };
        facets.push(crate::persona_lifecycle::LifecycleFacet {
            category: cat,
            value,
            origin_ts_secs: origin.delta.approved_at_unix_ms
                / 1000,
            reinforced,
            recall_topic,
            helpfulness,
        });
    }

    let detector =
        crate::persona_lifecycle::PersonaLifecycleDetector::new(
            deps.config.clone(),
            deps.embedding.clone(),
        );
    let actions = detector.detect(&facets, now_secs).await;

    let (mut proposed, mut deduped) = (0u32, 0u32);
    let mut proposed_items: Vec<
        crate::persona_lifecycle::PersonaLifecycleProposed,
    > = Vec::new();
    for action in &actions {
        for (pid, op) in action.to_proposals() {
            // Cross-cycle dedup: an id already in the chain in
            // ANY status (Pending/Approved/Rejected/Superseded)
            // is never re-filed — never nag about identity.
            if deps.proposal_log.get(&pid).is_some() {
                deduped += 1;
                continue;
            }
            let removed = match &op.op {
                crate::persona::PersonaDeltaOp::RemoveList {
                    value,
                } => value.clone(),
                _ => String::new(),
            };
            match deps
                .proposal_log
                .append_pending(
                    pid,
                    now_ms,
                    format!(
                        "persona-lifecycle:{}",
                        sched.name
                    ),
                    op,
                )
                .await
            {
                Ok(_) => {
                    proposed += 1;
                    proposed_items.push(
                        crate::persona_lifecycle::PersonaLifecycleProposed {
                            kind: action
                                .kind_label()
                                .to_string(),
                            category: action.category,
                            value: removed,
                            reason: action.reason.clone(),
                        },
                    );
                }
                Err(e) => {
                    eprintln!(
                        "aivyx persona-lifecycle: schedule \
                         {:?} append_pending failed: {e}",
                        sched.name,
                    );
                }
            }
        }
    }

    if proposed > 0 || deduped > 0 {
        eprintln!(
            "aivyx persona-lifecycle: schedule {:?} — \
             proposed {proposed} (deduped {deduped})",
            sched.name,
        );
        // Q4a — record this cycle for the Phase 78 surface.
        if let Some(stat) = &deps.stat {
            if let Ok(mut w) = stat.write() {
                *w = Some(
                    crate::persona_lifecycle::PersonaLifecycleStat {
                        ts_secs: now_secs,
                        proposed: proposed_items,
                        deduped,
                    },
                );
            }
        }
    }
}

/// Phase 87 — drive the pattern-driven Persona consolidation
/// pass on the reflection cadence (Q3a). For each surviving
/// `(A, B)` from the conservative double-gate selector, ask
/// the LLM phraser (Q2b) for a `learned_context` facet and
/// file it as a Pending proposal — same Phase 70 propose-only
/// + edit-then-approve flow.
///
/// Best-effort throughout: a per-candidate phrasing failure
/// skips that candidate; an append failure is logged and
/// skipped. A cycle-wide LLM outage (every survivor's
/// phrasing returns `None`) is recorded on the Phase 78 stat
/// so a quiet "0 filed" cycle stays distinguishable from "LLM
/// unavailable."
async fn run_persona_consolidation_pass(
    deps: &PersonaConsolidationDeps,
    sched: &ReflectionScheduleConfig,
    now_ms: u64,
) {
    if !deps.config.enabled {
        return;
    }
    let now_secs = now_ms / 1000;

    let candidates =
        crate::persona_consolidation::select_candidates(
            deps.cooccurrence_ledger.as_ref(),
            deps.helpfulness_ledger.as_ref(),
            deps.proposal_log.as_ref(),
            &deps.config,
            now_secs,
        )
        .await;
    if candidates.is_empty() {
        // Nothing to surface — the quiet case is a valid
        // outcome (Phase 70 / 80 / 85 same shape). Skip the
        // breadcrumb so chatty reflection cadences don't
        // spam stderr.
        return;
    }

    let source_label =
        format!("persona-consolidation:{}", sched.name);
    let stat = crate::persona_consolidation::consolidate(
        candidates,
        deps.phraser.as_ref(),
        deps.proposal_log.as_ref(),
        &source_label,
        now_ms,
    )
    .await;

    let llm_note =
        if stat.llm_unavailable { " (LLM unavailable)" } else { "" };
    eprintln!(
        "aivyx persona-consolidation: schedule {:?} — \
         filed {}{}",
        sched.name, stat.filed, llm_note,
    );

    // Q4a — record this cycle for the Phase 78 surface (the
    // actually-filed pairs + the LLM-availability flag, post
    // selector dedup). Written every armed cycle so "0 filed"
    // is itself legible.
    if let Some(sink) = &deps.stat {
        if let Ok(mut w) = sink.write() {
            *w = Some(stat);
        }
    }
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

    // ---- Phase 77 — recall-feedback pass orchestration ---------

    #[tokio::test]
    async fn recall_feedback_pass_promotes_and_files_and_clamps() {
        use crate::persona_proposal::{
            PersistentPersonaProposalLog, ProposalStatusFilter,
        };
        use crate::recall_log::{
            PersistentRecallLog, RecallEvent, RecallHit,
        };
        use aivyx_crypto::MasterKey;
        use aivyx_memory::{InMemoryMemory, Memory};
        use aivyx_storage::{
            KeyDomain, RedbStorage, Storage, StorageConfig,
        };
        use std::sync::Arc;

        let base =
            std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base).join(format!(
            "aivyx-recall-pass-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([79u8; 32]),
        )
        .await
        .unwrap();

        let recall_log = Arc::new(PersistentRecallLog::new(
            store.domain(KeyDomain::RecallEvents),
        ));
        let proposal_log = Arc::new(
            PersistentPersonaProposalLog::open(
                store.domain(KeyDomain::PersonaProposals),
                b"recall-pass-key".to_vec(),
            )
            .await
            .unwrap(),
        );
        let memory: Arc<dyn Memory> =
            Arc::new(InMemoryMemory::new());

        // Three entries under one topic, each recalled in its
        // own clean, well-separated turn → topic nets +3 →
        // promotion (all three) AND a proposal (>= 3*WEIGHT).
        let s = SessionId::new();
        let sid = s.to_string();
        let mut seqs = Vec::new();
        for i in 0..3u64 {
            let seq =
                memory.put("proj", &format!("note {i}")).await.unwrap();
            seqs.push(seq);
            recall_log
                .append(&RecallEvent {
                    ts_secs: 1000 + i * 100,
                    session_id: s,
                    hits: vec![RecallHit {
                        topic: "proj".into(),
                        seq,
                        score: 0.9,
                        cluster: false,
                    }],
                })
                .await
                .unwrap();
        }
        let summaries: Vec<OutcomeSummary> = (0..3u64)
            .map(|i| OutcomeSummary {
                session_id: sid.clone(),
                turn_id: format!("t{i}"),
                started_at_unix_ms: (1000 + i * 100) * 1000,
                outcome_kind: "completed".into(),
                tool_calls_made: 0,
                duration_ms: 500,
            })
            .collect();

        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 1_000_000,
            role_override: None,
            enabled: true,
        };
        let deps = RecallFeedbackDeps {
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            proposal_log: Arc::clone(&proposal_log),
            // cutoff = now_secs - this = 0 → nothing GC'd.
            gc_retain_secs: 2_000,
            helpfulness_ledger: None,
            cooccurrence_ledger: None,
        };

        // now well after the last event; whole window covered.
        run_recall_feedback_pass(&deps, &sched, &summaries, 2_000_000)
            .await;

        // Actuator A: every helpful entry LRU-promoted.
        let groups =
            memory.scan_prefix("", usize::MAX).await.unwrap();
        let proj = groups
            .iter()
            .find(|(t, _)| t == "proj")
            .map(|(_, e)| e.clone())
            .unwrap();
        for e in &proj {
            assert!(
                e.last_read_at_secs > 0,
                "seq {} should have been promoted",
                e.seq
            );
        }

        // Actuator B: one operator-gated Pending proposal.
        let pending = proposal_log.list(ProposalStatusFilter::Pending);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "recall-fb:proj");

        // Re-running the same window must not double-file or
        // re-promote-count (idempotent dedup).
        run_recall_feedback_pass(&deps, &sched, &summaries, 2_000_001)
            .await;
        assert_eq!(
            proposal_log.list(ProposalStatusFilter::Pending).len(),
            1,
            "second pass must not re-file the proposal"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Phase 82 — helpfulness-ledger fold-in -----------------

    #[tokio::test]
    async fn helpfulness_ledger_folds_across_two_cycles() {
        use crate::helpfulness_ledger::{
            PersistentHelpfulnessLedger, HELPFULNESS_HALF_LIFE_SECS,
        };
        use crate::persona_proposal::PersistentPersonaProposalLog;
        use crate::recall_log::{
            PersistentRecallLog, RecallEvent, RecallHit,
        };
        use aivyx_crypto::MasterKey;
        use aivyx_memory::{InMemoryMemory, Memory};
        use aivyx_storage::{
            KeyDomain, RedbStorage, Storage, StorageConfig,
        };
        use std::sync::Arc;

        let base =
            std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base).join(format!(
            "aivyx-ledger-fold-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([82u8; 32]),
        )
        .await
        .unwrap();

        let recall_log = Arc::new(PersistentRecallLog::new(
            store.domain(KeyDomain::RecallEvents),
        ));
        let ledger = Arc::new(PersistentHelpfulnessLedger::new(
            store.domain(KeyDomain::HelpfulnessLedger),
        ));
        let proposal_log = Arc::new(
            PersistentPersonaProposalLog::open(
                store.domain(KeyDomain::PersonaProposals),
                b"ledger-fold-key".to_vec(),
            )
            .await
            .unwrap(),
        );
        let memory: Arc<dyn Memory> =
            Arc::new(InMemoryMemory::new());

        // Topic "proj": three entries each recalled in its own
        // clean turn → per-topic net +3 each cycle.
        let s = SessionId::new();
        let sid = s.to_string();
        for i in 0..3u64 {
            let seq = memory
                .put("proj", &format!("note {i}"))
                .await
                .unwrap();
            recall_log
                .append(&RecallEvent {
                    ts_secs: 1000 + i * 100,
                    session_id: s,
                    hits: vec![RecallHit {
                        topic: "proj".into(),
                        seq,
                        score: 0.9,
                        cluster: false,
                    }],
                })
                .await
                .unwrap();
        }
        let summaries: Vec<OutcomeSummary> = (0..3u64)
            .map(|i| OutcomeSummary {
                session_id: sid.clone(),
                turn_id: format!("t{i}"),
                started_at_unix_ms: (1000 + i * 100) * 1000,
                outcome_kind: "completed".into(),
                tool_calls_made: 0,
                duration_ms: 500,
            })
            .collect();

        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            // Huge window + GC retain so the recall events stay
            // in-window and un-GC'd across both cycles.
            lookback_window_secs: 10_000_000_000,
            role_override: None,
            enabled: true,
        };
        let deps = RecallFeedbackDeps {
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            proposal_log: Arc::clone(&proposal_log),
            gc_retain_secs: 100_000_000_000,
            helpfulness_ledger: Some(Arc::clone(&ledger)),
            cooccurrence_ledger: None,
        };

        // Cycle 1 at now_secs = 1_000_000 → seed ewma = +3.
        run_recall_feedback_pass(
            &deps,
            &sched,
            &summaries,
            1_000_000_000,
        )
        .await;
        let e1 = ledger
            .topic_score("proj", 1_000_000)
            .await
            .unwrap()
            .expect("seeded after cycle 1");
        assert!(
            (e1.ewma_score - 3.0).abs() < 1e-3,
            "cycle 1 ewma got {}",
            e1.ewma_score
        );
        assert_eq!(e1.samples, 1);

        // Cycle 2 exactly one half-life later: stored +3 decays
        // to +1.5, then +3 folded in → +4.5; samples → 2.
        let now2_secs = 1_000_000 + HELPFULNESS_HALF_LIFE_SECS;
        run_recall_feedback_pass(
            &deps,
            &sched,
            &summaries,
            now2_secs * 1000,
        )
        .await;
        let e2 = ledger
            .topic_score("proj", now2_secs)
            .await
            .unwrap()
            .expect("present after cycle 2");
        assert!(
            (e2.ewma_score - 4.5).abs() < 2e-2,
            "cycle 2 decay(3)=1.5 + 3 = 4.5, got {}",
            e2.ewma_score
        );
        assert_eq!(e2.samples, 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Phase 83 — co-occurrence fold-in ----------------------

    #[tokio::test]
    async fn cooccurrence_folds_pairs_across_sessions_and_cycles()
    {
        use crate::cooccurrence_ledger::{
            PersistentCooccurrenceLedger,
            COOCCURRENCE_HALF_LIFE_SECS,
        };
        use crate::persona_proposal::PersistentPersonaProposalLog;
        use crate::recall_log::{
            PersistentRecallLog, RecallEvent, RecallHit,
        };
        use aivyx_crypto::MasterKey;
        use aivyx_memory::{InMemoryMemory, Memory};
        use aivyx_storage::{
            KeyDomain, RedbStorage, Storage, StorageConfig,
        };
        use std::sync::Arc;

        let base =
            std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base).join(format!(
            "aivyx-cooc-fold-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([83u8; 32]),
        )
        .await
        .unwrap();

        let recall_log = Arc::new(PersistentRecallLog::new(
            store.domain(KeyDomain::RecallEvents),
        ));
        let cooc = Arc::new(PersistentCooccurrenceLedger::new(
            store.domain(KeyDomain::CooccurrenceLedger),
        ));
        let proposal_log = Arc::new(
            PersistentPersonaProposalLog::open(
                store.domain(KeyDomain::PersonaProposals),
                b"cooc-fold-key".to_vec(),
            )
            .await
            .unwrap(),
        );
        let memory: Arc<dyn Memory> =
            Arc::new(InMemoryMemory::new());

        // Two DISTINCT sessions, each one clean turn that
        // co-recalled {deploy, rollback}. Cross-session
        // aggregation → the canonical pair accrues +1 per
        // helpful event = +2 this window.
        let mut summaries: Vec<OutcomeSummary> = Vec::new();
        for (i, ts) in [(0u64, 1000u64), (1, 1100)] {
            let s = SessionId::new();
            let d = memory
                .put("deploy", &format!("dep {i}"))
                .await
                .unwrap();
            let r = memory
                .put("rollback", &format!("rb {i}"))
                .await
                .unwrap();
            recall_log
                .append(&RecallEvent {
                    ts_secs: ts,
                    session_id: s,
                    hits: vec![
                        RecallHit {
                            topic: "deploy".into(),
                            seq: d,
                            score: 0.9,
                            cluster: false,
                        },
                        RecallHit {
                            topic: "rollback".into(),
                            seq: r,
                            score: 0.8,
                            cluster: false,
                        },
                    ],
                })
                .await
                .unwrap();
            summaries.push(OutcomeSummary {
                session_id: s.to_string(),
                turn_id: format!("t{i}"),
                started_at_unix_ms: ts * 1000,
                outcome_kind: "completed".into(),
                tool_calls_made: 0,
                duration_ms: 500,
            });
        }

        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 10_000_000_000,
            role_override: None,
            enabled: true,
        };
        let deps = RecallFeedbackDeps {
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            proposal_log: Arc::clone(&proposal_log),
            gc_retain_secs: 100_000_000_000,
            helpfulness_ledger: None,
            cooccurrence_ledger: Some(Arc::clone(&cooc)),
        };

        // Cycle 1 → pair {deploy,rollback} = +2 (two helpful
        // cross-session co-recall events), samples 1.
        run_recall_feedback_pass(
            &deps,
            &sched,
            &summaries,
            1_000_000_000,
        )
        .await;
        // Order-invariant lookup.
        let e1 = cooc
            .pair_score("rollback", "deploy", 1_000_000)
            .await
            .unwrap()
            .expect("pair seeded across sessions");
        assert!(
            (e1.ewma_score - 2.0).abs() < 1e-3,
            "cycle 1 pair ewma got {}",
            e1.ewma_score
        );
        assert_eq!(e1.samples, 1);

        // Cycle 2 one half-life later: decay(2)=1 + 2 = 3.
        let now2 = 1_000_000 + COOCCURRENCE_HALF_LIFE_SECS;
        run_recall_feedback_pass(
            &deps,
            &sched,
            &summaries,
            now2 * 1000,
        )
        .await;
        let e2 = cooc
            .pair_score("deploy", "rollback", now2)
            .await
            .unwrap()
            .unwrap();
        assert!(
            (e2.ewma_score - 3.0).abs() < 2e-2,
            "cycle 2 decay(2)=1 + 2 = 3, got {}",
            e2.ewma_score
        );
        assert_eq!(e2.samples, 2);

        // Disabled (absent ledger) → complete no-op.
        let off = RecallFeedbackDeps {
            cooccurrence_ledger: None,
            ..deps
        };
        run_recall_feedback_pass(
            &off,
            &sched,
            &summaries,
            now2 * 1000 + 1,
        )
        .await;
        let e3 = cooc
            .pair_score("deploy", "rollback", now2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            e3.samples, 2,
            "absent ledger must not fold"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phase 84 (Q3a) self-policing: a cluster-injected hit is
    /// EXCLUDED from the co-occurrence fold (the ledger never
    /// learns from its own expansion) but is STILL measured by
    /// the Phase 82 helpfulness fold (a bad expansion
    /// self-penalises).
    #[tokio::test]
    async fn cluster_hits_excluded_from_cooccurrence_kept_in_helpfulness(
    ) {
        use crate::cooccurrence_ledger::PersistentCooccurrenceLedger;
        use crate::helpfulness_ledger::PersistentHelpfulnessLedger;
        use crate::persona_proposal::PersistentPersonaProposalLog;
        use crate::recall_log::{
            PersistentRecallLog, RecallEvent, RecallHit,
        };
        use aivyx_crypto::MasterKey;
        use aivyx_memory::{InMemoryMemory, Memory};
        use aivyx_storage::{
            KeyDomain, RedbStorage, Storage, StorageConfig,
        };
        use std::sync::Arc;

        let base =
            std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base).join(format!(
            "aivyx-cluster-selfpolice-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([85u8; 32]),
        )
        .await
        .unwrap();
        let recall_log = Arc::new(PersistentRecallLog::new(
            store.domain(KeyDomain::RecallEvents),
        ));
        let cooc = Arc::new(PersistentCooccurrenceLedger::new(
            store.domain(KeyDomain::CooccurrenceLedger),
        ));
        let help = Arc::new(PersistentHelpfulnessLedger::new(
            store.domain(KeyDomain::HelpfulnessLedger),
        ));
        let proposal_log = Arc::new(
            PersistentPersonaProposalLog::open(
                store.domain(KeyDomain::PersonaProposals),
                b"selfpolice-key".to_vec(),
            )
            .await
            .unwrap(),
        );
        let memory: Arc<dyn Memory> =
            Arc::new(InMemoryMemory::new());

        let mut summaries: Vec<OutcomeSummary> = Vec::new();

        // Turn 1: primary "alpha" + CLUSTER-injected "beta".
        let s1 = SessionId::new();
        let a = memory.put("alpha", "a").await.unwrap();
        let b = memory.put("beta", "b").await.unwrap();
        recall_log
            .append(&RecallEvent {
                ts_secs: 1000,
                session_id: s1,
                hits: vec![
                    RecallHit {
                        topic: "alpha".into(),
                        seq: a,
                        score: 0.9,
                        cluster: false,
                    },
                    RecallHit {
                        topic: "beta".into(),
                        seq: b,
                        score: 0.8,
                        cluster: true, // injected sibling
                    },
                ],
            })
            .await
            .unwrap();
        summaries.push(OutcomeSummary {
            session_id: s1.to_string(),
            turn_id: "t1".into(),
            started_at_unix_ms: 1000 * 1000,
            outcome_kind: "completed".into(),
            tool_calls_made: 0,
            duration_ms: 500,
        });

        // Turn 2 (control): two PRIMARY topics co-recalled.
        let s2 = SessionId::new();
        let c = memory.put("cee", "c").await.unwrap();
        let d = memory.put("dee", "d").await.unwrap();
        recall_log
            .append(&RecallEvent {
                ts_secs: 1100,
                session_id: s2,
                hits: vec![
                    RecallHit {
                        topic: "cee".into(),
                        seq: c,
                        score: 0.9,
                        cluster: false,
                    },
                    RecallHit {
                        topic: "dee".into(),
                        seq: d,
                        score: 0.8,
                        cluster: false,
                    },
                ],
            })
            .await
            .unwrap();
        summaries.push(OutcomeSummary {
            session_id: s2.to_string(),
            turn_id: "t2".into(),
            started_at_unix_ms: 1100 * 1000,
            outcome_kind: "completed".into(),
            tool_calls_made: 0,
            duration_ms: 500,
        });

        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 10_000_000_000,
            role_override: None,
            enabled: true,
        };
        let deps = RecallFeedbackDeps {
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            proposal_log: Arc::clone(&proposal_log),
            gc_retain_secs: 100_000_000_000,
            helpfulness_ledger: Some(Arc::clone(&help)),
            cooccurrence_ledger: Some(Arc::clone(&cooc)),
        };
        run_recall_feedback_pass(
            &deps,
            &sched,
            &summaries,
            1_000_000_000,
        )
        .await;
        let t = 1_000_000u64;

        // Self-policing: {alpha,beta} is NOT in the
        // co-occurrence ledger — beta was cluster-injected, so
        // only alpha survived the fold filter and a lone topic
        // forms no pair.
        assert!(
            cooc.pair_score("alpha", "beta", t)
                .await
                .unwrap()
                .is_none(),
            "cluster-injected hit must not feed the \
             co-occurrence ledger"
        );
        // Control: the all-primary {cee,dee} pair IS folded.
        assert!(
            cooc.pair_score("cee", "dee", t)
                .await
                .unwrap()
                .is_some(),
            "an all-primary co-recall must still fold"
        );
        // But beta IS still measured by the Phase 82
        // helpfulness ledger (a bad expansion self-penalises).
        let bscore = help
            .topic_score("beta", t)
            .await
            .unwrap()
            .expect("cluster hit still scored by helpfulness");
        assert!(
            bscore.ewma_score > 0.0,
            "helpful turn → cluster hit scored positive, got {}",
            bscore.ewma_score
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Phase 80 — proactive pass orchestration ---------------

    #[tokio::test]
    async fn proactive_pass_surfaces_then_dedups_and_caps() {
        use crate::notify_dispatcher::{
            NotifyBackend, NotifyDispatcher, NotifyError,
        };
        use crate::proactive_log::PersistentProactiveLog;
        use aivyx_crypto::MasterKey;
        use aivyx_memory::{InMemoryMemory, Memory};
        use aivyx_storage::{
            KeyDomain, RedbStorage, Storage, StorageConfig,
        };
        use std::sync::{Arc, Mutex};

        struct RecBackend {
            calls: Mutex<Vec<(String, Option<String>)>>,
        }
        #[async_trait::async_trait]
        impl NotifyBackend for RecBackend {
            async fn send(
                &self,
                message: &str,
                subject: Option<&str>,
            ) -> Result<(), NotifyError> {
                self.calls.lock().unwrap().push((
                    message.to_string(),
                    subject.map(|s| s.to_string()),
                ));
                Ok(())
            }
            fn kind(&self) -> &'static str {
                "rec"
            }
        }

        let base =
            std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base).join(format!(
            "aivyx-proactive-pass-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([80u8; 32]),
        )
        .await
        .unwrap();

        let memory: Arc<dyn Memory> =
            Arc::new(InMemoryMemory::new());
        // A reminder that is past due → DueReminder fires.
        memory
            .put("rem", "@due:1000 water the plants")
            .await
            .unwrap();
        // A plain note → nothing.
        memory.put("notes", "just a note").await.unwrap();

        let plog = Arc::new(PersistentProactiveLog::new(
            store.domain(KeyDomain::ProactiveLog),
        ));
        let backend = Arc::new(RecBackend {
            calls: Mutex::new(Vec::new()),
        });
        let mut nd = NotifyDispatcher::new();
        nd.register("ops", backend.clone());
        let notify = Arc::new(nd);

        let deps = ProactiveDeps {
            config: aivyx_config::ProactiveConfig {
                enabled: true,
                target: "ops".into(),
                max_per_window: 5,
                window_secs: 3600,
                signals: aivyx_config::ProactiveSignals::default(),
            },
            memory: Arc::clone(&memory),
            proactive_log: Arc::clone(&plog),
            notify: Arc::clone(&notify),
            recall_log: None,
            memory_ttl_secs: None,
            gc_retain_secs: 1_000_000,
            stat: None,
        };
        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 86_400,
            role_override: None,
            enabled: true,
        };

        // now well past the @due:1000.
        run_proactive_pass(&deps, &sched, &[], 5_000_000).await;
        {
            let calls = backend.calls.lock().unwrap();
            assert_eq!(calls.len(), 1, "the due reminder surfaces");
            assert!(calls[0].0.contains("water the plants"));
            assert!(calls[0]
                .1
                .as_deref()
                .unwrap()
                .contains("DueReminder"));
        }
        assert!(plog.was_surfaced("due:rem:0").await.unwrap());

        // Second cycle, same state → deduped, no new send.
        run_proactive_pass(&deps, &sched, &[], 5_000_001).await;
        assert_eq!(
            backend.calls.lock().unwrap().len(),
            1,
            "already-surfaced item must not re-send"
        );

        // Disabled config → complete no-op even with fresh state.
        let off = ProactiveDeps {
            config: aivyx_config::ProactiveConfig {
                enabled: false,
                ..deps.config.clone()
            },
            memory,
            proactive_log: plog,
            notify,
            recall_log: None,
            memory_ttl_secs: None,
            gc_retain_secs: 1_000_000,
            stat: None,
        };
        run_proactive_pass(&off, &sched, &[], 6_000_000).await;
        assert_eq!(backend.calls.lock().unwrap().len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Phase 81 — persona-lifecycle pass orchestration -------

    #[tokio::test]
    async fn persona_lifecycle_pass_files_then_dedups() {
        use crate::persona::{
            PersonaDelta, PersonaDeltaCategory, PersonaDeltaOp,
            PersistentPersonaLog,
        };
        use crate::persona_proposal::{
            PersistentPersonaProposalLog, ProposalStatusFilter,
        };
        use aivyx_crypto::MasterKey;
        use aivyx_llm::embedding::{
            EmbeddingError, EmbeddingProvider,
        };
        use aivyx_storage::{
            KeyDomain, RedbStorage, Storage, StorageConfig,
        };
        use std::sync::Arc;

        struct FakeEmb;
        #[async_trait::async_trait]
        impl EmbeddingProvider for FakeEmb {
            async fn embed(
                &self,
                texts: &[String],
            ) -> Result<Vec<Vec<f32>>, EmbeddingError> {
                Ok(texts
                    .iter()
                    .map(|t| {
                        let l = t.to_lowercase();
                        if l.contains("dup") {
                            vec![1.0, 0.0, 0.0]
                        } else if l.contains("other") {
                            vec![0.0, 1.0, 0.0]
                        } else {
                            vec![0.0, 0.0, 1.0]
                        }
                    })
                    .collect())
            }
            fn model(&self) -> &str {
                "fake"
            }
            fn dimensions(&self) -> usize {
                3
            }
        }

        fn seed(
            seq_label: &str,
            cat: PersonaDeltaCategory,
            value: &str,
            approved_at_unix_ms: u64,
        ) -> PersonaDelta {
            PersonaDelta {
                delta_id: format!("d-{seq_label}"),
                proposed_at_unix_ms: approved_at_unix_ms,
                approved_at_unix_ms,
                proposal_id: "seed".into(),
                category: cat,
                op: PersonaDeltaOp::AppendList {
                    value: value.into(),
                },
            }
        }

        let base = std::env::var("TMPDIR")
            .unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base).join(format!(
            "aivyx-pl-pass-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([81u8; 32]),
        )
        .await
        .unwrap();

        let persona_log = Arc::new(
            PersistentPersonaLog::open(
                store.domain(KeyDomain::Persona),
                vec![1u8; 32],
            )
            .await
            .unwrap(),
        );
        // LearnedContext: two near-duplicates (consolidate) +
        // one distinct; all fresh so they never decay.
        persona_log
            .append(seed(
                "0",
                PersonaDeltaCategory::LearnedContext,
                "dup a",
                9_500_000,
            ))
            .await
            .unwrap();
        persona_log
            .append(seed(
                "1",
                PersonaDeltaCategory::LearnedContext,
                "dup a longer",
                9_500_000,
            ))
            .await
            .unwrap();
        persona_log
            .append(seed(
                "2",
                PersonaDeltaCategory::LearnedContext,
                "lc distinct other",
                9_500_000,
            ))
            .await
            .unwrap();
        // CharacterTraits: a kept (reinforced, fresh) facet +
        // one old, last-in-category, unreinforced → decays.
        persona_log
            .append(seed(
                "3",
                PersonaDeltaCategory::CharacterTraits,
                "ct keep other",
                9_600_000,
            ))
            .await
            .unwrap();
        persona_log
            .append(seed(
                "4",
                PersonaDeltaCategory::CharacterTraits,
                "ct stale",
                1_000,
            ))
            .await
            .unwrap();

        let proposal_log = Arc::new(
            PersistentPersonaProposalLog::open(
                store.domain(KeyDomain::PersonaProposals),
                vec![2u8; 32],
            )
            .await
            .unwrap(),
        );

        let stat =
            crate::persona_lifecycle::shared_persona_lifecycle_stat();
        let deps = PersonaLifecycleDeps {
            config: aivyx_config::PersonaLifecycleConfig {
                enabled: true,
                consolidation_similarity: 0.92,
                decay_max_age_secs: 1_000,
                min_soft_facets: 2,
                decay_unhelpful_threshold: -2.0,
                decay_min_samples: 3,
                signals:
                    aivyx_config::PersonaLifecycleSignals {
                        consolidate: true,
                        decay: true,
                    },
            },
            persona_log: Arc::clone(&persona_log),
            proposal_log: Arc::clone(&proposal_log),
            embedding: Arc::new(FakeEmb),
            helpfulness_ledger: None,
            stat: Some(Arc::clone(&stat)),
        };
        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 86_400,
            role_override: None,
            enabled: true,
        };

        // now_ms = 10_000_000 → now_secs 10_000.
        run_persona_lifecycle_pass(&deps, &sched, 10_000_000)
            .await;
        let pending =
            proposal_log.list(ProposalStatusFilter::Pending);
        assert_eq!(
            pending.len(),
            2,
            "one consolidate-removal + one decay-removal"
        );
        let ids: Vec<&str> =
            pending.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.iter().any(|i| i.starts_with(
            "pl:consolidate:learned_context:keep=dup a longer:"
        )));
        assert!(ids
            .contains(&"pl:decay:character_traits:ct stale"));
        {
            let s = stat.read().unwrap();
            let s = s.as_ref().expect("stat written");
            assert_eq!(s.proposed.len(), 2);
            assert_eq!(s.deduped, 0);
        }

        // Second cycle, same state → every id already in the
        // chain → all deduped, no new proposals.
        run_persona_lifecycle_pass(&deps, &sched, 10_000_001)
            .await;
        assert_eq!(
            proposal_log
                .list(ProposalStatusFilter::Pending)
                .len(),
            2,
            "already-filed proposals must not be re-filed"
        );
        {
            let s = stat.read().unwrap();
            let s = s.as_ref().unwrap();
            assert!(s.proposed.is_empty());
            assert_eq!(s.deduped, 2);
        }

        // Disabled config → complete no-op.
        let off = PersonaLifecycleDeps {
            config: aivyx_config::PersonaLifecycleConfig {
                enabled: false,
                ..deps.config.clone()
            },
            persona_log,
            proposal_log: Arc::clone(&proposal_log),
            embedding: Arc::new(FakeEmb),
            helpfulness_ledger: None,
            stat: None,
        };
        run_persona_lifecycle_pass(&off, &sched, 10_000_002)
            .await;
        assert_eq!(
            proposal_log
                .list(ProposalStatusFilter::Pending)
                .len(),
            2,
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Phase 85 — helpfulness-driven decay (provenance) ------

    #[tokio::test]
    async fn helpfulness_decay_uses_recall_fb_provenance() {
        use crate::helpfulness_ledger::PersistentHelpfulnessLedger;
        use crate::persona::{
            PersonaDelta, PersonaDeltaCategory, PersonaDeltaOp,
            PersistentPersonaLog,
        };
        use crate::persona_proposal::{
            PersistentPersonaProposalLog, ProposalStatusFilter,
        };
        use aivyx_crypto::MasterKey;
        use aivyx_llm::embedding::{
            EmbeddingError, EmbeddingProvider,
        };
        use aivyx_storage::{
            KeyDomain, RedbStorage, Storage, StorageConfig,
        };
        use std::sync::Arc;

        struct NoEmb;
        #[async_trait::async_trait]
        impl EmbeddingProvider for NoEmb {
            async fn embed(
                &self,
                _t: &[String],
            ) -> Result<Vec<Vec<f32>>, EmbeddingError> {
                Ok(vec![])
            }
            fn model(&self) -> &str {
                "noemb"
            }
            fn dimensions(&self) -> usize {
                1
            }
        }

        fn delta(
            id: &str,
            value: &str,
            proposal_id: &str,
            approved_ms: u64,
        ) -> PersonaDelta {
            PersonaDelta {
                delta_id: id.into(),
                proposed_at_unix_ms: approved_ms,
                approved_at_unix_ms: approved_ms,
                proposal_id: proposal_id.into(),
                category: PersonaDeltaCategory::LearnedContext,
                op: PersonaDeltaOp::AppendList {
                    value: value.into(),
                },
            }
        }

        let base = std::env::var("TMPDIR")
            .unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base).join(format!(
            "aivyx-pl-help-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([85u8; 32]),
        )
        .await
        .unwrap();

        let persona_log = Arc::new(
            PersistentPersonaLog::open(
                store.domain(KeyDomain::Persona),
                vec![1u8; 32],
            )
            .await
            .unwrap(),
        );
        // now_ms = 1e9 → now_secs 1_000_000. Both facets YOUNG
        // (origin == now → age 0), so age-only never decays
        // them — any decay here is helpfulness-driven.
        let now_ms = 1_000_000_000u64;
        // A: recall-feedback-derived (recall-fb:deploy).
        persona_log
            .append(delta(
                "a",
                "deploy runbook fact",
                "recall-fb:deploy",
                now_ms,
            ))
            .await
            .unwrap();
        // B: reflection-authored (no recall-fb provenance).
        persona_log
            .append(delta(
                "b",
                "a reflection note",
                "seed",
                now_ms,
            ))
            .await
            .unwrap();

        let proposal_log = Arc::new(
            PersistentPersonaProposalLog::open(
                store.domain(KeyDomain::PersonaProposals),
                vec![2u8; 32],
            )
            .await
            .unwrap(),
        );
        let ledger = Arc::new(PersistentHelpfulnessLedger::new(
            store.domain(KeyDomain::HelpfulnessLedger),
        ));
        // Topic "deploy" sustained-negative: 3 windows, net
        // -3 each at the same instant → ewma -9, samples 3.
        for _ in 0..3 {
            ledger
                .record_window(
                    &[("deploy".into(), -3.0)],
                    1_000_000,
                )
                .await
                .unwrap();
        }

        let cfg = aivyx_config::PersonaLifecycleConfig {
            enabled: true,
            consolidation_similarity: 0.92,
            decay_max_age_secs: 1_000_000, // age 0 → never
            min_soft_facets: 2,
            decay_unhelpful_threshold: -2.0,
            decay_min_samples: 3,
            signals: aivyx_config::PersonaLifecycleSignals {
                consolidate: false,
                decay: true,
            },
        };
        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 86_400,
            role_override: None,
            enabled: true,
        };

        // 1) No ledger → graceful pure age-only. Both facets
        //    are young → nothing proposed.
        let no_ledger = PersonaLifecycleDeps {
            config: cfg.clone(),
            persona_log: Arc::clone(&persona_log),
            proposal_log: Arc::clone(&proposal_log),
            embedding: Arc::new(NoEmb),
            helpfulness_ledger: None,
            stat: None,
        };
        run_persona_lifecycle_pass(&no_ledger, &sched, now_ms)
            .await;
        assert!(
            proposal_log
                .list(ProposalStatusFilter::Pending)
                .is_empty(),
            "no ledger + young facets → pure age-only → nothing"
        );

        // 2) Ledger present, "deploy" sustained-negative →
        //    facet A decays early via recall-fb provenance;
        //    the reflection-authored B (no provenance) does
        //    not.
        let with_ledger = PersonaLifecycleDeps {
            config: cfg,
            persona_log: Arc::clone(&persona_log),
            proposal_log: Arc::clone(&proposal_log),
            embedding: Arc::new(NoEmb),
            helpfulness_ledger: Some(Arc::clone(&ledger)),
            stat: None,
        };
        run_persona_lifecycle_pass(&with_ledger, &sched, now_ms)
            .await;
        let pending =
            proposal_log.list(ProposalStatusFilter::Pending);
        assert_eq!(pending.len(), 1, "only A decays");
        assert_eq!(
            pending[0].id,
            "pl:decay:learned_context:deploy runbook fact"
        );
        let reason = match &pending[0].proposed_op.op {
            PersonaDeltaOp::RemoveList { value } => {
                assert_eq!(value, "deploy runbook fact");
                pending[0]
                    .proposed_op
                    .reason
                    .clone()
                    .unwrap_or_default()
            }
            o => panic!("expected RemoveList, got {o:?}"),
        };
        assert!(
            reason.contains("\"deploy\"")
                && reason
                    .contains("sustained low helpfulness"),
            "reason must cite the helpfulness evidence: {reason}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

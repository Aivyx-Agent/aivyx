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

// moved to the wasm-clean aivyx-ipc crate (Chapter M.2d-2); re-exported here.
pub use aivyx_ipc::insights::{RecentReflectionStat};
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use cron::Schedule as CronSchedule;

use aivyx_audit::{AuditEvent, PersistentAuditLog, SignedEntry};
use aivyx_config::ReflectionScheduleConfig;
use aivyx_core::{CancellationToken, TurnId, TurnOutcomeSummary};

use crate::trigger::{TriggerDispatch, TriggerSource};

/// Phase 95 — per-schedule "should we fire this cycle"
/// decision. Pure arithmetic over the operator-configured
/// knobs + the observed audit-chain growth since the last
/// fired cycle for the same schedule.
///
/// Returns `false` only when:
/// - `skip_when_idle = true`, AND
/// - `min_to_fire >= 1` (defended — `0` always fires; the
///   loader validates against this combination, but the
///   helper defends defensively), AND
/// - `audit_growth < min_to_fire as u64`.
///
/// In all other cases the cycle fires. The operator's
/// `cron` interval is the upper bound on firing rate — this
/// helper can only suppress a fire, never schedule one.
pub fn should_fire_cycle(
    audit_growth: u64,
    min_to_fire: u32,
    skip_when_idle: bool,
) -> bool {
    if !skip_when_idle {
        return true;
    }
    if min_to_fire == 0 {
        // Defended: zero would always-skip; the loader
        // rejects this combination, but if it somehow
        // reaches the helper, fire instead of locking the
        // operator out of every cycle.
        return true;
    }
    audit_growth >= u64::from(min_to_fire)
}


/// Shared per-schedule cadence stats handle, keyed by
/// schedule name. The scheduler writes to it on every cycle
/// decision; the `GetLearningInsights` IPC reads to surface
/// the per-schedule cadence picture.
pub type SharedRecentReflectionStats =
    Arc<RwLock<HashMap<String, RecentReflectionStat>>>;

/// Construct an empty shared cadence-stat handle.
pub fn shared_recent_reflection_stats() -> SharedRecentReflectionStats {
    Arc::new(RwLock::new(HashMap::new()))
}

/// Phase 95 — per-cycle cadence decision. Returns whether
/// the cycle should fire AND increments the `skipped`
/// counter on the cadence stat if it shouldn't. Called by
/// `run_reflection_scheduler` before per-pass dispatch.
///
/// The first cycle (no entry in `last_fired_audit_len`)
/// always fires — no prior baseline to compare against.
/// Subsequent cycles compare audit-chain growth (current
/// minus last-fired) against the schedule's
/// `min_audit_entries_to_fire` via `should_fire_cycle`.
fn decide_cadence_action(
    schedule_name: &str,
    skip_when_idle: bool,
    min_to_fire: u32,
    current_audit_len: u64,
    last_fired_audit_len: &HashMap<String, u64>,
    cadence_stats: &SharedRecentReflectionStats,
) -> bool {
    let should_fire = match last_fired_audit_len.get(schedule_name) {
        None => true,
        Some(prev) => {
            let growth = current_audit_len.saturating_sub(*prev);
            should_fire_cycle(growth, min_to_fire, skip_when_idle)
        }
    };
    if !should_fire {
        if let Ok(mut stats) = cadence_stats.write() {
            let entry = stats
                .entry(schedule_name.to_string())
                .or_default();
            entry.skipped = entry.skipped.saturating_add(1);
        }
    }
    should_fire
}

/// Phase 95 — record that the cycle for `schedule_name`
/// fired. Stores the current audit-log length as the
/// baseline for the next cycle's growth comparison, and
/// increments the `fired` counter on the cadence stat.
fn mark_cycle_fired(
    schedule_name: &str,
    current_audit_len: u64,
    last_fired_audit_len: &mut HashMap<String, u64>,
    cadence_stats: &SharedRecentReflectionStats,
) {
    last_fired_audit_len.insert(
        schedule_name.to_string(),
        current_audit_len,
    );
    if let Ok(mut stats) = cadence_stats.write() {
        let entry = stats
            .entry(schedule_name.to_string())
            .or_default();
        entry.fired = entry.fired.saturating_add(1);
    }
}

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
    /// Phase 172 — the durable correction ledger. The pass folds
    /// each window's per-topic correction count (the
    /// `completed`-then-rapid-followup proxy from
    /// `crate::correction_detect`) into it, after the Phase
    /// 82/83 folds so recall-feedback + both existing ledgers
    /// stay byte-identical, and prunes on the same cadence.
    /// `None` → no fold (a passive add-on; recall-feedback is
    /// unaffected).
    pub correction_ledger: Option<
        std::sync::Arc<
            crate::correction_ledger::PersistentCorrectionLedger,
        >,
    >,
    /// Phase 178 — the LLM correction judge. When `Some` (armed
    /// `[correction_judgment]`), the correction fold classifies
    /// each detected correction's follow-up and folds only
    /// `Rework` events; `None` → the Phase 172 structural fold.
    pub correction_judge: Option<
        std::sync::Arc<dyn crate::correction_judgment::CorrectionJudge>,
    >,
    /// Phase 178 — per-cycle judge cap (from
    /// `[correction_judgment].max_corrections_per_cycle`).
    pub correction_judgment_max: u32,
    /// Phase 178 — last-cycle judgment stat sink for the Phase
    /// 78 surface. `None` → breadcrumb-only.
    pub correction_judgment_stat: Option<
        crate::correction_judgment::SharedCorrectionJudgmentStat,
    >,
    /// Phase 179 — when `true` (from
    /// `[correction_signal].attribute_tools`), the correction
    /// fold also attributes corrections to the corrected turn's
    /// tools (outcome-driven, keyed `tool:<base>`), additively
    /// over the topic counts. `false` → Phase 172 topic-only.
    pub attribute_tool_corrections: bool,
    /// Phase 93 — flip `correlate_detailed` from the
    /// pre-Phase-93 structural-only behaviour (the default,
    /// `false`) to per-hit judgment override with structural
    /// fallback for un-judged hits (`true`). Threaded from
    /// `[recall_feedback].use_judgment_signal`; absent
    /// section → `false`.
    pub use_judgment_signal: bool,
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
    /// Phase 88 — the durable co-occurrence ledger. When
    /// present, a facet whose `consolidate-pair:{lo}+{hi}`
    /// provenance resolves is gated by that pair's decayed
    /// affinity (symmetric with the helpfulness arm: a
    /// pair-below-floor triggers early decay, a still-strong
    /// pair protects an age-old facet). `None` → no
    /// pair-affinity signal, exact Phase 85/81 fallback for
    /// `consolidate-pair:` facets.
    pub cooccurrence_ledger: Option<
        std::sync::Arc<
            crate::cooccurrence_ledger::PersistentCooccurrenceLedger,
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
    /// Phase 92 — Persona chain handle for supersession
    /// detection. Read-only; the pass walks applied
    /// `consolidate-pair:` facets and feeds them to
    /// `detect_supersession`. `None` (or
    /// `config.enable_supersession = false`) → the
    /// supersession-detection branch is skipped, Phase
    /// 87/88 flow is byte-identical to pre-Phase-92.
    pub persona_log: Option<
        std::sync::Arc<crate::persona::PersistentPersonaLog>,
    >,
    /// Phase 92 — the Phase 88 `[persona_lifecycle].
    /// decay_pair_below_affinity` floor, threaded from
    /// `DaemonConfig`. Determines when a pair counts as
    /// "decayed" for supersession purposes. Default `1.0`
    /// matches the Phase 88 default; the binary fills it
    /// from the operator's actual `[persona_lifecycle]`
    /// config when present.
    pub pair_below_affinity: f32,
}

/// Phase 172 — handles the correction-consolidation pass needs.
/// Bundled like [`PersonaConsolidationDeps`]. `None` (no
/// `[correction_consolidation]` / no correction ledger
/// substrate) → the pass is skipped entirely (the correction
/// ledger still accumulates passively; no proposals are filed).
/// Even when `Some`, the pass no-ops unless `config.enabled`.
pub struct CorrectionConsolidationDeps {
    pub config: aivyx_config::CorrectionConsolidationConfig,
    pub correction_ledger: std::sync::Arc<
        crate::correction_ledger::PersistentCorrectionLedger,
    >,
    pub proposal_log: std::sync::Arc<
        crate::persona_proposal::PersistentPersonaProposalLog,
    >,
    pub phraser: std::sync::Arc<
        dyn crate::correction_consolidation::TopicPhraser,
    >,
    /// Optional last-cycle stat sink for the Phase 78 surface.
    /// `None` → breadcrumb-only.
    pub stat: Option<
        crate::correction_consolidation::SharedCorrectionConsolidationStat,
    >,
}

/// Phase 91 — handles the LLM-judged recall pass needs.
/// Bundled like [`PersonaConsolidationDeps`]. `None` (no
/// `[recall_judgment]` / no recall-log + judge substrate) →
/// the pass is skipped entirely (pre-Phase-91 behavior — the
/// structural recall-feedback signal is the only signal).
/// Even when `Some`, the pass no-ops unless `config.enabled`.
pub struct RecallJudgmentDeps {
    pub config: aivyx_config::RecallJudgmentConfig,
    pub recall_log: std::sync::Arc<
        crate::recall_log::PersistentRecallLog,
    >,
    pub memory: std::sync::Arc<dyn aivyx_memory::Memory>,
    pub judge: std::sync::Arc<
        dyn crate::recall_judgment::RecallJudge,
    >,
    /// Phase 91 (Q4a) — optional last-cycle stat sink for the
    /// Phase 78 surface. `None` → breadcrumb-only.
    pub stat: Option<
        crate::recall_judgment::SharedRecallJudgmentStat,
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
    /// Phase 179 — the distinct scope bases of the turn's
    /// `ToolCall` audit events, in first-seen order (e.g.
    /// `["fs.read", "git.read"]`). The scope base is stable
    /// across daemon restarts (unlike the per-process
    /// `tool_id`), so it is the restart-safe per-tool-surface
    /// key. Empty for a turn that made no tool calls.
    pub tools: Vec<String>,
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
        // Phase 179 — render the turn's tools so the reflection
        // LLM sees what the turn actually did, not just a count.
        let tools = if s.tools.is_empty() {
            String::new()
        } else {
            format!(", tools=[{}]", s.tools.join(", "))
        };
        out.push_str(&format!(
            "- turn `{turn}` (session `{ses}`) at {ts}ms — outcome={out_kind}, \
             tool_calls={tc}{tools}, duration={dur}ms\n",
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
    // Phase 179 — distinct scope bases per open turn, accumulated
    // from the `ToolCall` events that land between the turn's
    // `TurnStarted` and `TurnEnded`.
    let mut tools_by_turn: HashMap<TurnId, Vec<String>> = HashMap::new();
    let mut out: Vec<OutcomeSummary> = Vec::new();
    for entry in entries {
        let ts = system_time_to_unix_ms(entry.appended_at);
        match &entry.event {
            AuditEvent::TurnStarted {
                turn_id, session_id, ..
            } => {
                open.insert(*turn_id, (session_id.to_string(), ts));
            }
            AuditEvent::ToolCall {
                turn_id,
                scope_used,
                ..
            } => {
                // The scope base is the restart-safe per-tool
                // surface key (Phase 102 joins on it). Distinct,
                // first-seen order.
                let base = scope_used.base().to_string();
                let list = tools_by_turn.entry(*turn_id).or_default();
                if !list.contains(&base) {
                    list.push(base);
                }
            }
            AuditEvent::TurnEnded {
                turn_id,
                outcome,
                tool_calls_made,
                duration,
                ..
            } => {
                let tools =
                    tools_by_turn.remove(turn_id).unwrap_or_default();
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
                        tools,
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
        TurnOutcomeSummary::MaxStepsExceeded => "max_steps_exceeded",
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
    correction_consolidation: Option<CorrectionConsolidationDeps>,
    recall_judgment: Option<RecallJudgmentDeps>,
    cadence_stats: SharedRecentReflectionStats,
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
    // Phase 95 — per-schedule audit-log length at last fired
    // cycle. `None` means "this schedule hasn't fired yet
    // since daemon boot" — the first cycle is unconditional.
    let mut last_fired_audit_len: HashMap<String, u64> = HashMap::new();
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
                // Phase 95 — skip-when-idle gate. The first
                // cycle (no prior `last_fired_audit_len`)
                // fires unconditionally; subsequent cycles
                // consult audit-chain growth. The operator's
                // cron remains the upper bound on firing
                // rate — this gate only suppresses fires,
                // never schedules them.
                let current_len = audit_log.len() as u64;
                let should_fire = decide_cadence_action(
                    &sched.name,
                    sched.skip_when_idle,
                    sched.min_audit_entries_to_fire,
                    current_len,
                    &last_fired_audit_len,
                    &cadence_stats,
                );
                if !should_fire {
                    if let Some(prev) =
                        last_fired_audit_len.get(&sched.name)
                    {
                        eprintln!(
                            "aivyx reflection: schedule {:?} — \
                             skipped (audit-growth {} \
                             below threshold {})",
                            sched.name,
                            current_len.saturating_sub(*prev),
                            sched.min_audit_entries_to_fire,
                        );
                    }
                    last_fired.insert(sched.name.clone(), now);
                    if let Some(after_now) =
                        next_fire_after(&sched.cron, now)
                    {
                        update_earliest(
                            &mut earliest_next, after_now, now,
                        );
                    }
                    continue;
                }
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
                    correction_consolidation.as_ref(),
                    recall_judgment.as_ref(),
                )
                .await;
                last_fired.insert(sched.name.clone(), now);
                mark_cycle_fired(
                    &sched.name,
                    audit_log.len() as u64,
                    &mut last_fired_audit_len,
                    &cadence_stats,
                );
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
    correction_consolidation: Option<&CorrectionConsolidationDeps>,
    recall_judgment: Option<&RecallJudgmentDeps>,
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

    // Phase 172 — correction-driven Persona consolidation on the
    // same cadence. Independent of the above; no-op when absent
    // or disabled. Files Pending proposals only — the self-
    // improvement closure (the agent notices what it keeps
    // getting reworked on and asks the operator about it).
    if let Some(deps) = correction_consolidation {
        run_correction_consolidation_pass(deps, sched, now_ms).await;
    }

    // Phase 91 — LLM-judged per-recall classification on the
    // same cadence (Q1a). Independent of the above; no-op when
    // absent or disabled. Writes the new `judgment` field on
    // unjudged recall hits (Q3a augment) — every existing
    // accumulator stays byte-identical (no consumer reads the
    // new field in v1).
    if let Some(deps) = recall_judgment {
        run_recall_judgment_pass(deps, sched, now_ms).await;
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
                &recalls,
                summaries,
                deps.use_judgment_signal,
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
                            &recalls,
                            summaries,
                            deps.use_judgment_signal,
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

                // Phase 172 — fold this window's per-topic
                // correction counts (the completed-then-rapid-
                // followup proxy) into the durable correction
                // ledger. After the Phase 82/83 folds, so
                // recall-feedback AND both existing ledgers are
                // byte-identical; a passive add-on, absent →
                // skipped. Distinct actuator from helpfulness:
                // it counts reworks, not net helpfulness.
                if let Some(ledger) = &deps.correction_ledger {
                    // Phase 178 — when the correction judge is
                    // armed, classify each correction's follow-up
                    // and fold only `Rework` (plus structural
                    // fallback for un-judgeable / failed events);
                    // otherwise the Phase 172 structural fold.
                    let count_by_topic: Vec<(String, f32)> =
                        if let Some(judge) = &deps.correction_judge {
                            let events = crate::correction_detect::detect_corrections_detailed(
                                &recalls, summaries,
                            );
                            let (counts, stat) =
                                crate::correction_judgment::judged_correction_counts(
                                    &events,
                                    judge.as_ref(),
                                    deps.correction_judgment_max as usize,
                                    now_secs,
                                )
                                .await;
                            eprintln!(
                                "aivyx correction-judgment: schedule \
                                 {:?} — judged {} (rework {}, praise \
                                 {}, unrelated {}, structural {})",
                                sched.name,
                                stat.judged,
                                stat.rework,
                                stat.praise,
                                stat.unrelated,
                                stat.structural_fallback,
                            );
                            if let Some(sink) =
                                &deps.correction_judgment_stat
                            {
                                if let Ok(mut w) = sink.write() {
                                    *w = Some(stat);
                                }
                            }
                            counts
                        } else {
                            crate::correction_detect::detect_corrections(
                                &recalls, summaries,
                            )
                            .ranked()
                            .into_iter()
                            .map(|(topic, count)| (topic, count as f32))
                            .collect()
                        };
                    let folded = count_by_topic.len();
                    if folded > 0 {
                        if let Err(e) = ledger
                            .record_window(&count_by_topic, now_secs)
                            .await
                        {
                            eprintln!(
                                "aivyx correction-ledger: schedule \
                                 {:?} fold error: {e}",
                                sched.name,
                            );
                        } else {
                            let pruned = ledger
                                .prune(now_secs)
                                .await
                                .unwrap_or(0);
                            eprintln!(
                                "aivyx correction-ledger: folded \
                                 {folded} topic(s), pruned {pruned}",
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

    // Phase 179 — opt-in tool correction attribution. Runs
    // **outside** the recalls-non-empty gate above precisely so it
    // catches **no-recall** turns (the whole point — the topic
    // fold is recall-driven and never sees them). Outcome-driven,
    // `tool:`-namespaced, and a separate `record_window` call:
    // `record_window` is per-key, so disjoint topic + tool keys at
    // the same `now_secs` are equivalent to one merged call. Off →
    // nothing runs (byte-identical to Phase 172/178).
    if deps.attribute_tool_corrections {
        if let Some(ledger) = &deps.correction_ledger {
            let tools =
                crate::correction_detect::detect_tool_corrections(
                    summaries,
                );
            if !tools.is_empty() {
                let counts: Vec<(String, f32)> = tools
                    .ranked()
                    .into_iter()
                    .map(|(k, c)| (k, c as f32))
                    .collect();
                let n = counts.len();
                if let Err(e) =
                    ledger.record_window(&counts, now_secs).await
                {
                    eprintln!(
                        "aivyx correction-signal: schedule {:?} tool \
                         fold error: {e}",
                        sched.name,
                    );
                } else {
                    let _ = ledger.prune(now_secs).await;
                    eprintln!(
                        "aivyx correction-signal: schedule {:?} — \
                         attributed {n} tool key(s)",
                        sched.name,
                    );
                }
            }
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
                    // Phase 93 — proactive's RecallCluster
                    // signal stays on the structural-only
                    // path even when `[recall_feedback]
                    // .use_judgment_signal = true`. The
                    // augment is scoped to the recall-feedback
                    // actuator (memory promotion + Persona
                    // proposals); extending it into proactive
                    // surfacing is a future-phase decision.
                    crate::recall_feedback::correlate(
                        &recalls, summaries, false,
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
        // Phase 88 — recover the topic pair structurally from
        // `consolidate-pair:{lo}+{hi}` provenance. Only
        // consolidation-actuator-derived facets carry it; every
        // other provenance arm → `None` (the pair arm sits out,
        // the detector follows whatever other signal it has).
        let pair = origin
            .delta
            .proposal_id
            .strip_prefix("consolidate-pair:")
            .and_then(|rest| rest.split_once('+'))
            .map(|(a, b)| (a.to_string(), b.to_string()));
        // Resolve the pair's durable decayed affinity once,
        // here, so the detector stays pure. `None` when no
        // ledger, no provenance, or the pair is unseen → the
        // detector's pair arm sits out (age-only fallback for
        // `consolidate-pair:` facets, byte-identical to
        // pre-Phase-88).
        let pair_affinity =
            match (&deps.cooccurrence_ledger, pair.as_ref()) {
                (Some(ledger), Some((a, b))) => match ledger
                    .pair_score(a, b, now_secs)
                    .await
                {
                    Ok(Some(e)) => Some(
                        crate::persona_lifecycle::PairAffinityHint {
                            affinity: e.ewma_score,
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
            pair,
            pair_affinity,
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
    let source_label =
        format!("persona-consolidation:{}", sched.name);

    // Phase 92 — supersession detection. Runs BEFORE the
    // standard Phase 87 `select_candidates` so the new pairs
    // it claims are skipped from the standard selector
    // (avoiding duplicate filing). When `enable_supersession`
    // is false (the default) OR the Persona log isn't on the
    // deps, this branch is a no-op.
    let mut superseded_new_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut superseded_filed = 0u32;
    if deps.config.enable_supersession {
        if let Some(persona_log) = &deps.persona_log {
            let applied = collect_applied_pair_facets(
                persona_log.as_ref(),
            );
            let candidates =
                crate::persona_consolidation::detect_supersession(
                    &applied,
                    deps.cooccurrence_ledger.as_ref(),
                    deps.helpfulness_ledger.as_ref(),
                    deps.proposal_log.as_ref(),
                    &deps.config,
                    deps.pair_below_affinity,
                    now_secs,
                )
                .await;
            for cand in candidates {
                if file_supersession(
                    &cand,
                    deps.phraser.as_ref(),
                    deps.proposal_log.as_ref(),
                    &source_label,
                    now_ms,
                )
                .await
                {
                    superseded_filed += 1;
                    superseded_new_ids
                        .insert(cand.new_proposal_id);
                }
            }
        }
    }

    let mut candidates =
        crate::persona_consolidation::select_candidates(
            deps.cooccurrence_ledger.as_ref(),
            deps.helpfulness_ledger.as_ref(),
            deps.proposal_log.as_ref(),
            &deps.config,
            now_secs,
        )
        .await;
    // Phase 92 — exclude any pair the supersession path has
    // already filed under its canonical id. The standard
    // selector wouldn't dedup against fresh-this-cycle
    // proposals (the chain reads happen at the START of the
    // selector); we filter here.
    candidates.retain(|c| {
        !superseded_new_ids.contains(
            &crate::persona_consolidation::pair_proposal_id(
                &c.a, &c.b,
            ),
        )
    });

    if candidates.is_empty() && superseded_filed == 0 {
        // Nothing to surface — the quiet case is a valid
        // outcome (Phase 70 / 80 / 85 same shape). Skip the
        // breadcrumb so chatty reflection cadences don't
        // spam stderr.
        return;
    }

    let mut stat = crate::persona_consolidation::consolidate(
        candidates,
        deps.phraser.as_ref(),
        deps.proposal_log.as_ref(),
        &source_label,
        now_ms,
    )
    .await;
    // Phase 92 — supersession events landed two chain entries
    // each. Add to the standard `filed` count (twice each)
    // and stamp the per-event count.
    stat.filed += superseded_filed * 2;
    stat.superseded = superseded_filed;

    let super_note = if superseded_filed > 0 {
        format!(" (superseded={superseded_filed})")
    } else {
        String::new()
    };
    let llm_note =
        if stat.llm_unavailable { " (LLM unavailable)" } else { "" };
    eprintln!(
        "aivyx persona-consolidation: schedule {:?} — \
         filed {}{super_note}{llm_note}",
        sched.name, stat.filed,
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

/// Phase 172 — the correction-consolidation pass. Selects
/// topics the operator has repeatedly reworked (the correction
/// ledger past the `min_corrections` + `min_samples` double-
/// gate), phrases each, and files a Pending `correction:{topic}`
/// proposal through the Phase 70 chain. No-op when disabled.
/// Files Pending proposals only — never resolves them.
async fn run_correction_consolidation_pass(
    deps: &CorrectionConsolidationDeps,
    sched: &ReflectionScheduleConfig,
    now_ms: u64,
) {
    if !deps.config.enabled {
        return;
    }
    let now_secs = now_ms / 1000;
    let candidates =
        crate::correction_consolidation::select_corrections(
            deps.correction_ledger.as_ref(),
            deps.proposal_log.as_ref(),
            &deps.config,
            now_secs,
        )
        .await;
    if candidates.is_empty() {
        // The quiet case is a valid outcome; skip the breadcrumb
        // so chatty reflection cadences don't spam stderr.
        return;
    }

    let source_label = format!("reflection:{}", sched.name);
    let stat =
        crate::correction_consolidation::consolidate_corrections(
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
        "aivyx correction-consolidation: schedule {:?} — \
         filed {}{llm_note}",
        sched.name, stat.filed,
    );

    // Record this cycle for the Phase 78 surface (filed topics +
    // LLM-availability flag). Written every armed cycle so "0
    // filed" is itself legible.
    if let Some(sink) = &deps.stat {
        if let Ok(mut w) = sink.write() {
            *w = Some(stat);
        }
    }
}

/// Phase 92 — walk the Persona chain and return every
/// CURRENTLY-APPLIED `consolidate-pair:` facet as
/// `(proposal_id, facet_value, (lo, hi))`. The supersession
/// detector iterates this list.
///
/// An applied facet is one whose LATEST chain entry for the
/// same `(category, value)` is an `AppendList` (i.e. not
/// subsequently removed). The pair is recovered from the
/// proposal_id via `parse_pair_proposal_id`; any entry whose
/// id doesn't parse is skipped silently.
fn collect_applied_pair_facets(
    persona_log: &crate::persona::PersistentPersonaLog,
) -> Vec<(String, String, (String, String))> {
    let entries = persona_log.entries();
    let persona =
        crate::persona::compute_effective_persona(&entries);
    let mut out: Vec<(String, String, (String, String))> =
        Vec::new();
    for value in &persona.learned_context {
        // Find the LATEST AppendList in LearnedContext that
        // put this value into effect.
        let Some(origin) = entries.iter().rev().find(|e| {
            e.delta.category
                == crate::persona::PersonaDeltaCategory::LearnedContext
                && matches!(
                    &e.delta.op,
                    crate::persona::PersonaDeltaOp::AppendList {
                        value: v,
                    } if *v == *value
                )
        }) else {
            continue;
        };
        let Some(pair) =
            crate::persona_consolidation::parse_pair_proposal_id(
                &origin.delta.proposal_id,
            )
        else {
            continue;
        };
        out.push((
            origin.delta.proposal_id.clone(),
            value.clone(),
            pair,
        ));
    }
    out
}

/// Phase 92 — file the two linked proposals for one
/// supersession candidate. Returns `true` iff BOTH halves
/// land (the `RemoveList` for the old facet AND the
/// `AppendList` for the new facet, each carrying the other
/// half's `proposal_id` in its
/// `supersedes_proposal_id` field).
///
/// LLM-phrase failure on the new facet → return `false`;
/// the supersession is skipped this cycle (the operator may
/// see it next cycle when the structural conditions still
/// hold). An `append_pending` failure on the
/// `RemoveList`-side after the `AppendList`-side succeeded
/// leaves the chain with the `AppendList` already filed —
/// the operator can still review it; the linkage is one-way
/// in that case but the supersession still fires.
async fn file_supersession(
    cand: &crate::persona_consolidation::SupersessionCandidate,
    phraser: &dyn crate::persona_consolidation::PairPhraser,
    proposal_log: &crate::persona_proposal::PersistentPersonaProposalLog,
    source_label: &str,
    now_ms: u64,
) -> bool {
    let Some(new_value) = phraser
        .phrase(&cand.new_pair.0, &cand.new_pair.1)
        .await
    else {
        return false;
    };
    let new_value = new_value.trim().to_string();
    if new_value.is_empty() {
        return false;
    }

    let (new_a, new_c) = &cand.new_pair;
    let new_reason = format!(
        "supersedes proposal `{old}`: co-occurrence pair \
         `{a}` + `{c}` — decayed affinity {aff:.2}; both \
         topics helpful (min score {hmin:.2})",
        old = cand.old_proposal_id,
        a = new_a,
        c = new_c,
        aff = cand.new_affinity,
        hmin = cand.new_helpfulness_min,
    );
    let new_op = crate::persona::ProposedPersonaDelta {
        category:
            crate::persona::PersonaDeltaCategory::LearnedContext,
        op: crate::persona::PersonaDeltaOp::AppendList {
            value: new_value,
        },
        reason: Some(new_reason),
        supersedes_proposal_id: Some(
            cand.old_proposal_id.clone(),
        ),
    };
    if let Err(e) = proposal_log
        .append_pending(
            cand.new_proposal_id.clone(),
            now_ms,
            source_label.to_string(),
            new_op,
        )
        .await
    {
        eprintln!(
            "aivyx persona-consolidation: append_pending \
             failed for new (AppendList) supersession half \
             {}: {e}",
            cand.new_proposal_id,
        );
        return false;
    }

    let remove_id = format!(
        "supersede-remove:{old}",
        old = cand.old_proposal_id,
    );
    let remove_reason = format!(
        "superseded by proposal `{new}`: the original pair \
         `{a}` + `{b}` has decayed; replaced by `{na}` + \
         `{nc}` (the new facet's prose)",
        new = cand.new_proposal_id,
        a = cand.old_pair.0,
        b = cand.old_pair.1,
        na = cand.new_pair.0,
        nc = cand.new_pair.1,
    );
    let remove_op = crate::persona::ProposedPersonaDelta {
        category:
            crate::persona::PersonaDeltaCategory::LearnedContext,
        op: crate::persona::PersonaDeltaOp::RemoveList {
            value: cand.old_facet_value.clone(),
        },
        reason: Some(remove_reason),
        supersedes_proposal_id: Some(
            cand.new_proposal_id.clone(),
        ),
    };
    if let Err(e) = proposal_log
        .append_pending(
            remove_id.clone(),
            now_ms,
            source_label.to_string(),
            remove_op,
        )
        .await
    {
        // The new facet is already filed (one-way linkage).
        // Log and continue — the operator can still review.
        eprintln!(
            "aivyx persona-consolidation: append_pending \
             failed for old (RemoveList) supersession half \
             {}: {e}",
            remove_id,
        );
    }
    true
}

/// Phase 91 — drive the LLM-judged recall pass on the
/// reflection cadence (Q1a). Reads unjudged recall events in
/// the lookback window (oldest-first, up to
/// `max_recalls_per_cycle` HITS), recovers the recalled
/// memory body for each hit, builds the batch, calls the
/// judge once (Q1a — one LLM call per cycle), patches each
/// judgment back to the recall log (Q3a augment — every
/// existing accumulator stays byte-identical).
///
/// Best-effort throughout: a per-hit recovery failure skips
/// that hit; a cycle-wide LLM failure records
/// `llm_unavailable = true` and ends gracefully.
///
/// **v1 simplification.** `RecallJudgeInput.response_text` is
/// filled with the recalled topic name as a context hint —
/// not the model's actual response text (which isn't in the
/// audit chain today). The Phase 91 LLM judgment is therefore
/// based on `(topic, body, topic-as-hint)` in v1; future
/// phases enrich the response context via audit-chain
/// extension or per-turn capture. The Q3a augment posture
/// means even this weaker v1 signal changes no existing
/// accumulator behavior — it is captured for inspection +
/// validated by a future actuator-side phase.
async fn run_recall_judgment_pass(
    deps: &RecallJudgmentDeps,
    sched: &ReflectionScheduleConfig,
    now_ms: u64,
) {
    if !deps.config.enabled {
        return;
    }
    let now_secs = now_ms / 1000;
    let since = now_secs.saturating_sub(sched.lookback_window_secs);

    let mut rows = match deps
        .recall_log
        .events_with_keys_since(since)
        .await
    {
        Ok(r) => r,
        Err(_) => return, // ledger error — quiet best-effort
    };
    // Cap: the operator-set per-cycle bound is on HITS, not
    // events. Walk rows oldest-first and stop once the
    // unjudged-hit budget is exhausted; the remainder rolls
    // to the next cycle.
    let cap = deps.config.max_recalls_per_cycle as usize;
    let mut budget = cap;
    let mut total_unjudged = 0usize;
    let mut judge_inputs: Vec<crate::recall_judgment::RecallJudgeInput> =
        Vec::new();
    // `(row_index, hit_index)` for each input, so the post-
    // judge update knows where to put each result back.
    let mut targets: Vec<(usize, usize)> = Vec::new();
    for (row_i, (_, event)) in rows.iter().enumerate() {
        for (hit_i, hit) in event.hits.iter().enumerate() {
            if hit.judgment.is_some() {
                continue;
            }
            total_unjudged += 1;
            if budget == 0 {
                continue;
            }
            // Best-effort body recovery from the substrate.
            // The recall log carries `(topic, seq)`; we walk
            // the topic's recent entries and find the one
            // matching `seq`. A missing entry (evicted /
            // forgotten) → skip this hit; the budget is
            // unchanged.
            let body = match deps
                .memory
                .get_recent(&hit.topic, 32)
                .await
            {
                Ok(entries) => entries
                    .into_iter()
                    .find(|e| e.seq == hit.seq)
                    .map(|e| e.body),
                Err(_) => None,
            };
            let Some(body) = body else {
                continue;
            };
            judge_inputs.push(
                crate::recall_judgment::RecallJudgeInput {
                    recalled_topic: hit.topic.clone(),
                    recalled_body: body,
                    // v1 — `response_text` placeholder; the
                    // recalled topic itself is a weak
                    // context hint. Future phases enrich.
                    response_text: hit.topic.clone(),
                },
            );
            targets.push((row_i, hit_i));
            budget -= 1;
        }
    }

    let skipped = total_unjudged.saturating_sub(judge_inputs.len());

    if judge_inputs.is_empty() {
        // Nothing to judge this cycle: either every recall is
        // already judged or every unjudged hit failed body
        // recovery. Record the (skipped) stat if a sink is
        // attached so the surface stays legible; no LLM call.
        if let Some(sink) = &deps.stat {
            if let Ok(mut w) = sink.write() {
                *w = Some(
                    crate::recall_judgment::RecallJudgmentStat {
                        ts_secs: now_secs,
                        judged: 0,
                        used: 0,
                        irrelevant: 0,
                        hurt: 0,
                        skipped: skipped as u32,
                        llm_unavailable: false,
                        pairs: Vec::new(),
                    },
                );
            }
        }
        return;
    }

    let judgments = deps.judge.judge(&judge_inputs).await;
    let llm_unavailable = judgments.iter().all(Option::is_none);

    let mut used = 0u32;
    let mut irrelevant = 0u32;
    let mut hurt = 0u32;
    let mut pairs: Vec<(
        String,
        crate::recall_judgment::RecallJudgment,
    )> = Vec::new();
    // Track which rows we actually mutated so we only write
    // them back once each (one PUT per row, no matter how
    // many hits got patched).
    let mut dirty: std::collections::HashSet<usize> =
        std::collections::HashSet::new();
    for ((row_i, hit_i), judgment) in
        targets.iter().zip(judgments.iter())
    {
        let Some(j) = judgment else {
            continue;
        };
        match j {
            crate::recall_judgment::RecallJudgment::Used => {
                used += 1;
            }
            crate::recall_judgment::RecallJudgment::Irrelevant => {
                irrelevant += 1;
            }
            crate::recall_judgment::RecallJudgment::Hurt => {
                hurt += 1;
            }
        }
        let topic = rows[*row_i].1.hits[*hit_i].topic.clone();
        pairs.push((topic, *j));
        rows[*row_i].1.hits[*hit_i].judgment = Some(*j);
        dirty.insert(*row_i);
    }
    let judged = used + irrelevant + hurt;

    // Write back the dirty rows. A persist error per row
    // is logged but never fatal — the next cycle re-picks
    // up the still-unjudged hits.
    for row_i in &dirty {
        let (key, event) = &rows[*row_i];
        if let Err(e) =
            deps.recall_log.update_event(key, event).await
        {
            eprintln!(
                "aivyx recall-judgment: schedule {:?} \
                 update_event failed for row {}: {e}",
                sched.name, row_i,
            );
        }
    }

    let note = if llm_unavailable {
        " (LLM unavailable)"
    } else {
        ""
    };
    eprintln!(
        "aivyx recall-judgment: schedule {:?} — judged {} \
         (used={}, irrelevant={}, hurt={}, skipped={}){note}",
        sched.name, judged, used, irrelevant, hurt, skipped,
    );

    if let Some(sink) = &deps.stat {
        if let Ok(mut w) = sink.write() {
            *w = Some(
                crate::recall_judgment::RecallJudgmentStat {
                    ts_secs: now_secs,
                    judged,
                    used,
                    irrelevant,
                    hurt,
                    skipped: skipped as u32,
                    llm_unavailable,
                    pairs,
                },
            );
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

    // ---- Phase 95 — should_fire_cycle pure helper -----------

    /// Phase 95 — `skip_when_idle = false` short-circuits
    /// the check; every cycle fires regardless of growth.
    /// This is the pre-Phase-95 default — operators who
    /// haven't opted in see byte-identical behaviour.
    #[test]
    fn should_fire_skip_off_always_fires() {
        assert!(should_fire_cycle(0, 1, false));
        assert!(should_fire_cycle(0, 100, false));
        assert!(should_fire_cycle(50, 100, false));
        assert!(should_fire_cycle(u64::MAX, 100, false));
    }

    /// Phase 95 — `skip_when_idle = true` with audit-growth
    /// strictly below threshold skips. The boundary uses
    /// `>=`, so growth-at-threshold fires (next test).
    #[test]
    fn should_fire_skip_on_below_threshold_skips() {
        assert!(!should_fire_cycle(0, 1, true));
        assert!(!should_fire_cycle(4, 5, true));
        assert!(!should_fire_cycle(99, 100, true));
    }

    /// Phase 95 — `skip_when_idle = true` with audit-growth
    /// AT the threshold fires. Boundary semantics: `>=`,
    /// not strictly-greater-than. An operator setting
    /// `min_audit_entries_to_fire = 1` gets "any new entry
    /// fires" not "any entry beyond the first."
    #[test]
    fn should_fire_skip_on_at_threshold_fires() {
        assert!(should_fire_cycle(1, 1, true));
        assert!(should_fire_cycle(5, 5, true));
        assert!(should_fire_cycle(100, 100, true));
    }

    /// Phase 95 — `skip_when_idle = true` with audit-growth
    /// above the threshold fires.
    #[test]
    fn should_fire_skip_on_above_threshold_fires() {
        assert!(should_fire_cycle(2, 1, true));
        assert!(should_fire_cycle(50, 5, true));
        assert!(should_fire_cycle(u64::MAX, 1, true));
    }

    /// Phase 95 — defended: `min_to_fire = 0` always fires
    /// regardless of growth, even with `skip_when_idle =
    /// true`. The loader rejects this combination at config
    /// time, but the helper defends against it reaching
    /// runtime (defense-in-depth).
    #[test]
    fn should_fire_defended_threshold_zero_always_fires() {
        assert!(should_fire_cycle(0, 0, true));
        assert!(should_fire_cycle(0, 0, false));
        assert!(should_fire_cycle(100, 0, true));
    }

    /// Phase 95 — `RecentReflectionStat::default()` is zero/
    /// zero. Pinned because every per-schedule entry in the
    /// shared stats map initializes from this default.
    #[test]
    fn recent_reflection_stat_default_is_zero() {
        let s = RecentReflectionStat::default();
        assert_eq!(s.fired, 0);
        assert_eq!(s.skipped, 0);
    }

    /// Phase 95 — `RecentReflectionStat` round-trips
    /// through serde with all fields present, and decodes
    /// from an empty `{}` document (the wire-compat
    /// shape — pre-Phase-95 daemons send nothing for these
    /// fields).
    #[test]
    fn recent_reflection_stat_serde_wire_compat() {
        let s = RecentReflectionStat {
            fired: 7,
            skipped: 3,
        };
        let json = serde_json::to_string(&s).unwrap();
        let round: RecentReflectionStat =
            serde_json::from_str(&json).unwrap();
        assert_eq!(round, s);
        // Wire-compat decode from absent fields.
        let from_empty: RecentReflectionStat =
            serde_json::from_str("{}").unwrap();
        assert_eq!(from_empty, RecentReflectionStat::default());
    }

    /// Phase 95 — end-to-end multi-cycle integration over
    /// the helpers that the scheduler loop uses. Models a
    /// schedule with `skip_when_idle = true,
    /// min_audit_entries_to_fire = 5` across four cycles:
    ///
    /// - Cycle 1 (audit_len = 0, no prior baseline) →
    ///   unconditional fire; stat = (1, 0); cursor = 0.
    /// - Cycle 2 (audit_len = 3, growth = 3 < 5) → skip;
    ///   stat = (1, 1); cursor unchanged (= 0).
    /// - Cycle 3 (audit_len = 8, growth since cursor = 8
    ///   >= 5) → fire; stat = (2, 1); cursor = 8.
    /// - Cycle 4 (audit_len = 10, growth = 2 < 5) → skip;
    ///   stat = (2, 2); cursor unchanged (= 8).
    ///
    /// Exercises the cursor-not-updated-on-skip invariant,
    /// the first-cycle-unconditional rule, the boundary
    /// `>=` rule, and the accumulating stat shape.
    #[test]
    fn cadence_helpers_multi_cycle_fire_skip_fire_skip() {
        let schedule_name = "test";
        let skip_when_idle = true;
        let min_to_fire: u32 = 5;
        let stats = shared_recent_reflection_stats();
        let mut last_fired_audit_len: HashMap<String, u64> =
            HashMap::new();

        // Cycle 1 — first cycle, no baseline, unconditional
        // fire.
        let should_fire = decide_cadence_action(
            schedule_name,
            skip_when_idle,
            min_to_fire,
            0,
            &last_fired_audit_len,
            &stats,
        );
        assert!(should_fire, "first cycle must fire unconditionally");
        mark_cycle_fired(
            schedule_name,
            0,
            &mut last_fired_audit_len,
            &stats,
        );
        let s = stats.read().unwrap().get(schedule_name).cloned().unwrap();
        assert_eq!(s.fired, 1);
        assert_eq!(s.skipped, 0);
        assert_eq!(*last_fired_audit_len.get(schedule_name).unwrap(), 0);

        // Cycle 2 — audit grew to 3 (growth 3 < threshold 5)
        // → skip.
        let should_fire = decide_cadence_action(
            schedule_name,
            skip_when_idle,
            min_to_fire,
            3,
            &last_fired_audit_len,
            &stats,
        );
        assert!(!should_fire, "growth 3 < threshold 5 must skip");
        let s = stats.read().unwrap().get(schedule_name).cloned().unwrap();
        assert_eq!(s.fired, 1);
        assert_eq!(s.skipped, 1);
        // Cursor must NOT have advanced on skip.
        assert_eq!(*last_fired_audit_len.get(schedule_name).unwrap(), 0);

        // Cycle 3 — audit grew to 8 (growth 8 >= threshold
        // 5) → fire.
        let should_fire = decide_cadence_action(
            schedule_name,
            skip_when_idle,
            min_to_fire,
            8,
            &last_fired_audit_len,
            &stats,
        );
        assert!(should_fire, "growth 8 >= threshold 5 must fire");
        mark_cycle_fired(
            schedule_name,
            8,
            &mut last_fired_audit_len,
            &stats,
        );
        let s = stats.read().unwrap().get(schedule_name).cloned().unwrap();
        assert_eq!(s.fired, 2);
        assert_eq!(s.skipped, 1);
        assert_eq!(*last_fired_audit_len.get(schedule_name).unwrap(), 8);

        // Cycle 4 — audit grew to 10 (growth since cursor 8
        // is 2; 2 < threshold 5) → skip.
        let should_fire = decide_cadence_action(
            schedule_name,
            skip_when_idle,
            min_to_fire,
            10,
            &last_fired_audit_len,
            &stats,
        );
        assert!(!should_fire, "growth 2 < threshold 5 must skip");
        let s = stats.read().unwrap().get(schedule_name).cloned().unwrap();
        assert_eq!(s.fired, 2);
        assert_eq!(s.skipped, 2);
        // Cursor still at last-fired value.
        assert_eq!(*last_fired_audit_len.get(schedule_name).unwrap(), 8);
    }

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

    fn make_toolcall(
        seq: u64,
        turn_id: TurnId,
        ts_ms: u64,
        scope_base: &str,
    ) -> SignedEntry {
        SignedEntry {
            seq,
            appended_at: UNIX_EPOCH + Duration::from_millis(ts_ms),
            event: AuditEvent::ToolCall {
                turn_id,
                tool_id: aivyx_core::ToolId::new(),
                scope_used: aivyx_capability::Scope::parse(scope_base)
                    .expect("known base"),
                input_hash: [0u8; 32],
                outcome: aivyx_core::ToolOutcomeSummary::Completed {
                    verified:
                        aivyx_core::VerificationSummary::NotApplicable,
                },
                duration: Duration::from_millis(10),
                auto_corrected_from: None,
                extracted_from_text: None,
            },
            prev_mac: [0u8; 32],
            mac: [0u8; 32],
        }
    }

    #[test]
    fn builder_collects_distinct_tool_scope_bases() {
        // A turn that calls fs.read, fs.read (dup), then git.read.
        let (started, turn_id, _session) = make_started(0, 1_000);
        let tc1 = make_toolcall(1, turn_id, 1_100, "fs.read");
        let tc2 = make_toolcall(2, turn_id, 1_200, "fs.read");
        let tc3 = make_toolcall(3, turn_id, 1_300, "git.read");
        let ended = make_ended(4, turn_id, 1_500, 3);
        let entries = vec![started, tc1, tc2, tc3, ended];
        let out =
            summarize_recent_outcomes_from_entries(&entries, 3600, 10_000);
        assert_eq!(out.len(), 1);
        // Distinct, first-seen order.
        assert_eq!(out[0].tools, vec!["fs.read", "git.read"]);
    }

    #[test]
    fn builder_no_tools_when_turn_made_none() {
        let (started, turn_id, _s) = make_started(0, 1_000);
        let ended = make_ended(1, turn_id, 1_500, 0);
        let out = summarize_recent_outcomes_from_entries(
            &[started, ended],
            3600,
            10_000,
        );
        assert!(out[0].tools.is_empty());
    }

    #[test]
    fn prompt_renders_tools_when_present() {
        let with_tools = OutcomeSummary {
            session_id: "s".into(),
            turn_id: "t".into(),
            started_at_unix_ms: 1,
            outcome_kind: "completed".into(),
            tool_calls_made: 2,
            duration_ms: 5,
            tools: vec!["gmail.send".into(), "fs.read".into()],
        };
        let p = format_summaries_for_prompt(&[with_tools]);
        assert!(p.contains("tools=[gmail.send, fs.read]"));
        // No-tools turn omits the tools= clause.
        let no_tools = OutcomeSummary {
            session_id: "s".into(),
            turn_id: "t".into(),
            started_at_unix_ms: 1,
            outcome_kind: "completed".into(),
            tool_calls_made: 0,
            duration_ms: 5,
            tools: vec![],
        };
        assert!(!format_summaries_for_prompt(&[no_tools]).contains("tools="));
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
            tools: Vec::new(),
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
            tools: Vec::new(),
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
                    query_text: String::new(),
                    hits: vec![RecallHit {
                        topic: "proj".into(),
                        seq,
                        score: 0.9,
                        cluster: false,
                        judgment: None,
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
                tools: Vec::new(),
            })
            .collect();

        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 1_000_000,
            role_override: None,
            enabled: true,
            skip_when_idle: false,
            min_audit_entries_to_fire: 1,
        };
        let deps = RecallFeedbackDeps {
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            proposal_log: Arc::clone(&proposal_log),
            // cutoff = now_secs - this = 0 → nothing GC'd.
            gc_retain_secs: 2_000,
            helpfulness_ledger: None,
            cooccurrence_ledger: None,
            correction_ledger: None,
            correction_judge: None,
            correction_judgment_max: 0,
            correction_judgment_stat: None,
            attribute_tool_corrections: false,
            use_judgment_signal: false,
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

    // ---- Phase 93 — judgment-signal-driven recall feedback ----

    /// Phase 93 — the recall-feedback pass with
    /// `use_judgment_signal = true` accumulates per-hit
    /// judgment-driven contributions through to the
    /// retention actuator. Three entries under one topic,
    /// each recalled in its own clean (+WEIGHT structural)
    /// turn. The hits carry: Used / Hurt / None judgments.
    /// With the knob on:
    /// - Used → +WEIGHT (judgment) → promoted.
    /// - Hurt → -WEIGHT (judgment overrides positive
    ///   structural) → NOT promoted.
    /// - None → +WEIGHT (structural fallback) → promoted.
    /// Topic net = +WEIGHT (Used) + -WEIGHT (Hurt) + +WEIGHT
    /// (None) = +WEIGHT, below `PROPOSAL_TOPIC_THRESHOLD =
    /// 3*WEIGHT` → no proposal filed (the augment changes
    /// what gets pruned, not the proposal pipeline's
    /// threshold).
    #[tokio::test]
    async fn judgment_signal_drives_per_hit_promotion() {
        use crate::persona_proposal::{
            PersistentPersonaProposalLog, ProposalStatusFilter,
        };
        use crate::recall_log::{
            PersistentRecallLog, RecallEvent, RecallHit,
            RecallJudgment,
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
            "aivyx-judg-signal-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([93u8; 32]),
        )
        .await
        .unwrap();

        let recall_log = Arc::new(PersistentRecallLog::new(
            store.domain(KeyDomain::RecallEvents),
        ));
        let proposal_log = Arc::new(
            PersistentPersonaProposalLog::open(
                store.domain(KeyDomain::PersonaProposals),
                b"judgment-signal-key".to_vec(),
            )
            .await
            .unwrap(),
        );
        let memory: Arc<dyn Memory> =
            Arc::new(InMemoryMemory::new());

        let s = SessionId::new();
        let sid = s.to_string();

        let seq_used = memory
            .put("proj", "used-note")
            .await
            .unwrap();
        let seq_hurt = memory
            .put("proj", "hurt-note")
            .await
            .unwrap();
        let seq_none = memory
            .put("proj", "unjudged-note")
            .await
            .unwrap();

        let judgments = [
            (seq_used, Some(RecallJudgment::Used)),
            (seq_hurt, Some(RecallJudgment::Hurt)),
            (seq_none, None),
        ];
        for (i, (seq, judgment)) in
            judgments.iter().enumerate()
        {
            recall_log
                .append(&RecallEvent {
                    ts_secs: 1000 + (i as u64) * 100,
                    session_id: s,
                    query_text: String::new(),
                    hits: vec![RecallHit {
                        topic: "proj".into(),
                        seq: *seq,
                        score: 0.9,
                        cluster: false,
                        judgment: *judgment,
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
                tools: Vec::new(),
            })
            .collect();

        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 1_000_000,
            role_override: None,
            enabled: true,
            skip_when_idle: false,
            min_audit_entries_to_fire: 1,
        };
        let deps = RecallFeedbackDeps {
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            proposal_log: Arc::clone(&proposal_log),
            gc_retain_secs: 2_000,
            helpfulness_ledger: None,
            cooccurrence_ledger: None,
            correction_ledger: None,
            correction_judge: None,
            correction_judgment_max: 0,
            correction_judgment_stat: None,
            attribute_tool_corrections: false,
            // Phase 93 — the knob under test.
            use_judgment_signal: true,
        };

        run_recall_feedback_pass(&deps, &sched, &summaries, 2_000_000)
            .await;

        // Actuator A — per-hit judgment-driven outcome:
        //   Used → promoted (judgment +WEIGHT)
        //   Hurt → NOT promoted (judgment -WEIGHT overrides +turn)
        //   None → promoted (structural fallback +WEIGHT)
        let groups =
            memory.scan_prefix("", usize::MAX).await.unwrap();
        let proj = groups
            .iter()
            .find(|(t, _)| t == "proj")
            .map(|(_, e)| e.clone())
            .unwrap();
        let used_lr = proj
            .iter()
            .find(|e| e.seq == seq_used)
            .unwrap()
            .last_read_at_secs;
        let hurt_lr = proj
            .iter()
            .find(|e| e.seq == seq_hurt)
            .unwrap()
            .last_read_at_secs;
        let none_lr = proj
            .iter()
            .find(|e| e.seq == seq_none)
            .unwrap()
            .last_read_at_secs;
        assert!(
            used_lr > 0,
            "Used-judged entry must be promoted (got {used_lr})"
        );
        assert_eq!(
            hurt_lr, 0,
            "Hurt-judged entry must NOT be promoted despite \
             the +WEIGHT structural turn signal"
        );
        assert!(
            none_lr > 0,
            "Un-judged entry must fall back to the structural \
             +WEIGHT signal and be promoted (got {none_lr})"
        );

        // Actuator B — topic net is +1 (Used) + -1 (Hurt) +
        // +1 (None) = +1 < PROPOSAL_TOPIC_THRESHOLD = 3*WEIGHT
        // → no Pending proposal filed.
        let pending = proposal_log.list(ProposalStatusFilter::Pending);
        assert!(
            pending.is_empty(),
            "topic net is below the proposal threshold; no \
             proposal should have been filed (got {} pending)",
            pending.len(),
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
                    query_text: String::new(),
                    hits: vec![RecallHit {
                        topic: "proj".into(),
                        seq,
                        score: 0.9,
                        cluster: false,
                        judgment: None,
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
                tools: Vec::new(),
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
            skip_when_idle: false,
            min_audit_entries_to_fire: 1,
        };
        let deps = RecallFeedbackDeps {
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            proposal_log: Arc::clone(&proposal_log),
            gc_retain_secs: 100_000_000_000,
            helpfulness_ledger: Some(Arc::clone(&ledger)),
            cooccurrence_ledger: None,
            correction_ledger: None,
            correction_judge: None,
            correction_judgment_max: 0,
            correction_judgment_stat: None,
            attribute_tool_corrections: false,
            use_judgment_signal: false,
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

    // ---- Phase 172 — correction-ledger fold-in -----------------

    #[tokio::test]
    async fn correction_ledger_folds_only_corrected_turns() {
        use crate::correction_ledger::PersistentCorrectionLedger;
        use crate::persona_proposal::PersistentPersonaProposalLog;
        use crate::recall_log::{
            PersistentRecallLog, RecallEvent, RecallHit,
        };
        use aivyx_memory::{InMemoryMemory, Memory};
        use aivyx_crypto::MasterKey;
        use aivyx_storage::{
            KeyDomain, RedbStorage, Storage, StorageConfig,
        };
        use std::sync::Arc;

        let base =
            std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base).join(format!(
            "aivyx-correction-fold-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([172u8; 32]),
        )
        .await
        .unwrap();

        let recall_log = Arc::new(PersistentRecallLog::new(
            store.domain(KeyDomain::RecallEvents),
        ));
        let ledger = Arc::new(PersistentCorrectionLedger::new(
            store.domain(KeyDomain::CorrectionLedger),
        ));
        let proposal_log = Arc::new(
            PersistentPersonaProposalLog::open(
                store.domain(KeyDomain::PersonaProposals),
                b"correction-fold-key".to_vec(),
            )
            .await
            .unwrap(),
        );
        let memory: Arc<dyn Memory> =
            Arc::new(InMemoryMemory::new());

        // Turn t0 recalls `auth`, completes, and is followed
        // within the window by t1 (same session) → a CORRECTION.
        // Turn t2 recalls `db`, completes cleanly, NO quick
        // follow-up → not a correction. Only `auth` should fold.
        let s = SessionId::new();
        let sid = s.to_string();
        let auth_seq =
            memory.put("auth", "auth note").await.unwrap();
        let db_seq = memory.put("db", "db note").await.unwrap();
        recall_log
            .append(&RecallEvent {
                ts_secs: 1000,
                session_id: s,
                query_text: String::new(),
                hits: vec![RecallHit {
                    topic: "auth".into(),
                    seq: auth_seq,
                    score: 0.9,
                    cluster: false,
                    judgment: None,
                }],
            })
            .await
            .unwrap();
        recall_log
            .append(&RecallEvent {
                ts_secs: 5000,
                session_id: s,
                query_text: String::new(),
                hits: vec![RecallHit {
                    topic: "db".into(),
                    seq: db_seq,
                    score: 0.9,
                    cluster: false,
                    judgment: None,
                }],
            })
            .await
            .unwrap();

        let summaries: Vec<OutcomeSummary> = vec![
            // t0 recalled auth, completed, followed 5s later by t1.
            OutcomeSummary {
                session_id: sid.clone(),
                turn_id: "t0".into(),
                started_at_unix_ms: 1000 * 1000,
                outcome_kind: "completed".into(),
                tool_calls_made: 0,
                duration_ms: 1000,
                tools: Vec::new(),
            },
            OutcomeSummary {
                session_id: sid.clone(),
                turn_id: "t1".into(),
                started_at_unix_ms: 1006 * 1000,
                outcome_kind: "completed".into(),
                tool_calls_made: 0,
                duration_ms: 1000,
                tools: Vec::new(),
            },
            // t2 recalled db, completed, no quick follow-up.
            OutcomeSummary {
                session_id: sid.clone(),
                turn_id: "t2".into(),
                started_at_unix_ms: 5000 * 1000,
                outcome_kind: "completed".into(),
                tool_calls_made: 0,
                duration_ms: 1000,
                tools: Vec::new(),
            },
        ];

        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 10_000_000_000,
            role_override: None,
            enabled: true,
            skip_when_idle: false,
            min_audit_entries_to_fire: 1,
        };
        let deps = RecallFeedbackDeps {
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            proposal_log: Arc::clone(&proposal_log),
            gc_retain_secs: 100_000_000_000,
            helpfulness_ledger: None,
            cooccurrence_ledger: None,
            correction_ledger: Some(Arc::clone(&ledger)),
            correction_judge: None,
            correction_judgment_max: 0,
            correction_judgment_stat: None,
            attribute_tool_corrections: false,
            use_judgment_signal: false,
        };

        run_recall_feedback_pass(
            &deps,
            &sched,
            &summaries,
            1_000_000_000,
        )
        .await;

        // `auth` folded one correction; `db` never folds.
        let auth = ledger
            .topic_corrections("auth", 1_000_000)
            .await
            .unwrap()
            .expect("auth corrected");
        assert!((auth.ewma_count - 1.0).abs() < 1e-3);
        assert_eq!(auth.samples, 1);
        assert!(
            ledger
                .topic_corrections("db", 1_000_000)
                .await
                .unwrap()
                .is_none(),
            "a clean turn with no quick follow-up is never a \
             correction"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Phase 178 — judged correction fold --------------------

    /// A fake `CorrectionJudge` that says `Rework` iff the
    /// follow-up query contains "wrong", else `Praise`.
    struct WrongMeansRework;
    #[async_trait::async_trait]
    impl crate::correction_judgment::CorrectionJudge for WrongMeansRework {
        async fn judge(
            &self,
            inputs: &[crate::correction_judgment::CorrectionJudgeInput],
        ) -> Vec<Option<crate::correction_judgment::CorrectionJudgment>>
        {
            inputs
                .iter()
                .map(|i| {
                    if i.follow_up_query.contains("wrong") {
                        Some(crate::correction_judgment::CorrectionJudgment::Rework)
                    } else {
                        Some(crate::correction_judgment::CorrectionJudgment::Praise)
                    }
                })
                .collect()
        }
    }

    #[tokio::test]
    async fn judged_fold_folds_only_rework_followups() {
        use crate::correction_ledger::PersistentCorrectionLedger;
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
            "aivyx-judged-fold-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([178u8; 32]),
        )
        .await
        .unwrap();
        let recall_log = Arc::new(PersistentRecallLog::new(
            store.domain(KeyDomain::RecallEvents),
        ));
        let ledger = Arc::new(PersistentCorrectionLedger::new(
            store.domain(KeyDomain::CorrectionLedger),
        ));
        let proposal_log = Arc::new(
            PersistentPersonaProposalLog::open(
                store.domain(KeyDomain::PersonaProposals),
                b"judged-fold-key".to_vec(),
            )
            .await
            .unwrap(),
        );
        let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());

        // Two corrected turns, both completed-then-rapid-followup:
        //   auth: followed by a turn whose query says "wrong" → Rework → folds
        //   css:  followed by a turn whose query is praise      → drops
        let s = SessionId::new();
        let sid = s.to_string();
        // Corrected turns recall auth / css; the FOLLOW-UP turns
        // carry the captured queries.
        for (ts, topic, query) in [
            (1000u64, "auth", ""),
            (1006, "x", "that is wrong, redo it"), // follow-up of auth
            (5000, "css", ""),
            (5006, "y", "thanks, perfect"), // follow-up of css
        ] {
            recall_log
                .append(&RecallEvent {
                    ts_secs: ts,
                    session_id: s,
                    query_text: query.to_string(),
                    hits: vec![RecallHit {
                        topic: topic.into(),
                        seq: 1,
                        score: 0.9,
                        cluster: false,
                        judgment: None,
                    }],
                })
                .await
                .unwrap();
        }
        let summaries: Vec<OutcomeSummary> = [
            ("t0", 1000u64),
            ("t1", 1006),
            ("t2", 5000),
            ("t3", 5006),
        ]
        .iter()
        .map(|(id, ts)| OutcomeSummary {
            session_id: sid.clone(),
            turn_id: (*id).into(),
            started_at_unix_ms: ts * 1000,
            outcome_kind: "completed".into(),
            tool_calls_made: 0,
            duration_ms: 1000,
            tools: Vec::new(),
        })
        .collect();

        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 10_000_000_000,
            role_override: None,
            enabled: true,
            skip_when_idle: false,
            min_audit_entries_to_fire: 1,
        };
        let stat = crate::correction_judgment::shared_correction_judgment_stat();
        let deps = RecallFeedbackDeps {
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            proposal_log: Arc::clone(&proposal_log),
            gc_retain_secs: 100_000_000_000,
            helpfulness_ledger: None,
            cooccurrence_ledger: None,
            correction_ledger: Some(Arc::clone(&ledger)),
            correction_judge: Some(Arc::new(WrongMeansRework)),
            correction_judgment_max: 30,
            correction_judgment_stat: Some(stat.clone()),
            attribute_tool_corrections: false,
            use_judgment_signal: false,
        };

        run_recall_feedback_pass(&deps, &sched, &summaries, 1_000_000_000)
            .await;

        // auth (Rework) folded; css (Praise) dropped.
        assert!(ledger
            .topic_corrections("auth", 1_000_000)
            .await
            .unwrap()
            .is_some());
        assert!(
            ledger
                .topic_corrections("css", 1_000_000)
                .await
                .unwrap()
                .is_none(),
            "a praise follow-up must NOT fold as a correction"
        );
        // Stat recorded the cycle.
        let snap = stat.read().unwrap().clone().expect("stat written");
        assert_eq!(snap.rework, 1);
        assert_eq!(snap.praise, 1);
        assert_eq!(snap.judged, 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Phase 179 — additive tool correction fold -------------

    #[tokio::test]
    async fn attribute_tools_folds_tool_keys_for_no_recall_turn() {
        use crate::correction_ledger::PersistentCorrectionLedger;
        use crate::persona_proposal::PersistentPersonaProposalLog;
        use crate::recall_log::PersistentRecallLog;
        use aivyx_crypto::MasterKey;
        use aivyx_memory::{InMemoryMemory, Memory};
        use aivyx_storage::{
            KeyDomain, RedbStorage, Storage, StorageConfig,
        };
        use std::sync::Arc;

        let base =
            std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base).join(format!(
            "aivyx-tool-fold-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([179u8; 32]),
        )
        .await
        .unwrap();
        let recall_log = Arc::new(PersistentRecallLog::new(
            store.domain(KeyDomain::RecallEvents),
        ));
        let ledger = Arc::new(PersistentCorrectionLedger::new(
            store.domain(KeyDomain::CorrectionLedger),
        ));
        let proposal_log = Arc::new(
            PersistentPersonaProposalLog::open(
                store.domain(KeyDomain::PersonaProposals),
                b"tool-fold-key".to_vec(),
            )
            .await
            .unwrap(),
        );
        let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());

        // NO recall events. A corrected turn (t0 completed, t1
        // follows fast) that used gmail.send — invisible to the
        // recall-driven detector, attributable via tools.
        let sid = SessionId::new().to_string();
        let t0 = OutcomeSummary {
            session_id: sid.clone(),
            turn_id: "t0".into(),
            started_at_unix_ms: 100_000,
            outcome_kind: "completed".into(),
            tool_calls_made: 1,
            duration_ms: 1_000,
            tools: vec!["gmail.send".into()],
        };
        let summaries = vec![
            t0,
            OutcomeSummary {
                session_id: sid.clone(),
                turn_id: "t1".into(),
                started_at_unix_ms: 106_000,
                outcome_kind: "completed".into(),
                tool_calls_made: 0,
                duration_ms: 1_000,
                tools: vec![],
            },
        ];

        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 10_000_000_000,
            role_override: None,
            enabled: true,
            skip_when_idle: false,
            min_audit_entries_to_fire: 1,
        };
        let deps = RecallFeedbackDeps {
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            proposal_log: Arc::clone(&proposal_log),
            gc_retain_secs: 100_000_000_000,
            helpfulness_ledger: None,
            cooccurrence_ledger: None,
            correction_ledger: Some(Arc::clone(&ledger)),
            correction_judge: None,
            correction_judgment_max: 0,
            correction_judgment_stat: None,
            attribute_tool_corrections: true,
            use_judgment_signal: false,
        };

        run_recall_feedback_pass(&deps, &sched, &summaries, 1_000_000_000)
            .await;

        // The tool key folded even with zero recall events.
        assert!(ledger
            .topic_corrections("tool:gmail.send", 1_000_000)
            .await
            .unwrap()
            .is_some());

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
                    query_text: String::new(),
                    hits: vec![
                        RecallHit {
                            topic: "deploy".into(),
                            seq: d,
                            score: 0.9,
                            cluster: false,
                            judgment: None,
                        },
                        RecallHit {
                            topic: "rollback".into(),
                            seq: r,
                            score: 0.8,
                            cluster: false,
                            judgment: None,
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
                tools: Vec::new(),
            });
        }

        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 10_000_000_000,
            role_override: None,
            enabled: true,
            skip_when_idle: false,
            min_audit_entries_to_fire: 1,
        };
        let deps = RecallFeedbackDeps {
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            proposal_log: Arc::clone(&proposal_log),
            gc_retain_secs: 100_000_000_000,
            helpfulness_ledger: None,
            cooccurrence_ledger: Some(Arc::clone(&cooc)),
            correction_ledger: None,
            correction_judge: None,
            correction_judgment_max: 0,
            correction_judgment_stat: None,
            attribute_tool_corrections: false,
            use_judgment_signal: false,
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
                query_text: String::new(),
                hits: vec![
                    RecallHit {
                        topic: "alpha".into(),
                        seq: a,
                        score: 0.9,
                        cluster: false,
                        judgment: None,
                    },
                    RecallHit {
                        topic: "beta".into(),
                        seq: b,
                        score: 0.8,
                        cluster: true, // injected sibling
                        judgment: None,
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
            tools: Vec::new(),
        });

        // Turn 2 (control): two PRIMARY topics co-recalled.
        let s2 = SessionId::new();
        let c = memory.put("cee", "c").await.unwrap();
        let d = memory.put("dee", "d").await.unwrap();
        recall_log
            .append(&RecallEvent {
                ts_secs: 1100,
                session_id: s2,
                query_text: String::new(),
                hits: vec![
                    RecallHit {
                        topic: "cee".into(),
                        seq: c,
                        score: 0.9,
                        cluster: false,
                        judgment: None,
                    },
                    RecallHit {
                        topic: "dee".into(),
                        seq: d,
                        score: 0.8,
                        cluster: false,
                        judgment: None,
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
            tools: Vec::new(),
        });

        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 10_000_000_000,
            role_override: None,
            enabled: true,
            skip_when_idle: false,
            min_audit_entries_to_fire: 1,
        };
        let deps = RecallFeedbackDeps {
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            proposal_log: Arc::clone(&proposal_log),
            gc_retain_secs: 100_000_000_000,
            helpfulness_ledger: Some(Arc::clone(&help)),
            cooccurrence_ledger: Some(Arc::clone(&cooc)),
            correction_ledger: None,
            correction_judge: None,
            correction_judgment_max: 0,
            correction_judgment_stat: None,
            attribute_tool_corrections: false,
            use_judgment_signal: false,
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
            skip_when_idle: false,
            min_audit_entries_to_fire: 1,
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
                decay_pair_below_affinity: 1.0,
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
            cooccurrence_ledger: None,
            stat: Some(Arc::clone(&stat)),
        };
        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 86_400,
            role_override: None,
            enabled: true,
            skip_when_idle: false,
            min_audit_entries_to_fire: 1,
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
            cooccurrence_ledger: None,
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
            decay_pair_below_affinity: 1.0,
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
            skip_when_idle: false,
            min_audit_entries_to_fire: 1,
        };

        // 1) No ledger → graceful pure age-only. Both facets
        //    are young → nothing proposed.
        let no_ledger = PersonaLifecycleDeps {
            config: cfg.clone(),
            persona_log: Arc::clone(&persona_log),
            proposal_log: Arc::clone(&proposal_log),
            embedding: Arc::new(NoEmb),
            helpfulness_ledger: None,
            cooccurrence_ledger: None,
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
            cooccurrence_ledger: None,
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

    // ---- Phase 87 — pattern-driven Persona consolidation -------

    #[tokio::test]
    async fn consolidation_pass_files_dedups_and_handles_llm_outage()
    {
        use crate::cooccurrence_ledger::PersistentCooccurrenceLedger;
        use crate::helpfulness_ledger::PersistentHelpfulnessLedger;
        use crate::persona::{
            PersonaDeltaCategory, PersonaDeltaOp,
            ProposedPersonaDelta,
        };
        use crate::persona_consolidation::{
            shared_persona_consolidation_stat, PairPhraser,
        };
        use crate::persona_proposal::{
            PersistentPersonaProposalLog, ProposalStatusFilter,
        };
        use aivyx_crypto::MasterKey;
        use aivyx_storage::{
            KeyDomain, RedbStorage, Storage, StorageConfig,
        };
        use async_trait::async_trait;

        struct OkPhraser;
        #[async_trait]
        impl PairPhraser for OkPhraser {
            async fn phrase(
                &self,
                a: &str,
                b: &str,
            ) -> Option<String> {
                Some(format!(
                    "You consistently work with {a} and {b}."
                ))
            }
        }
        struct DownPhraser;
        #[async_trait]
        impl PairPhraser for DownPhraser {
            async fn phrase(
                &self,
                _a: &str,
                _b: &str,
            ) -> Option<String> {
                None
            }
        }

        let base = std::env::var("TMPDIR")
            .unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base).join(format!(
            "aivyx-pc-integ-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([87u8; 32]),
        )
        .await
        .unwrap();

        let cooc = Arc::new(PersistentCooccurrenceLedger::new(
            store.domain(KeyDomain::CooccurrenceLedger),
        ));
        let help = Arc::new(PersistentHelpfulnessLedger::new(
            store.domain(KeyDomain::HelpfulnessLedger),
        ));
        let plog = Arc::new(
            PersistentPersonaProposalLog::open(
                store.domain(KeyDomain::PersonaProposals),
                b"consolidation-integ-key".to_vec(),
            )
            .await
            .unwrap(),
        );

        // Seed three eligible pairs. All helpful endpoints, all
        // affinity-sufficient, samples >= 2 (after the second
        // record_window).
        let now = 1_000_000u64;
        let now_ms = now * 1000;
        let seeds = [
            (("deploy", "rollback"), 10.0),
            (("frontend", "css"), 8.0),
            (("rust", "borrow"), 6.0),
        ];
        for ((a, b), score) in &seeds {
            cooc.record_window(
                &[((a.to_string(), b.to_string()), *score)],
                now,
            )
            .await
            .unwrap();
            cooc.record_window(
                &[((a.to_string(), b.to_string()), 0.001)],
                now,
            )
            .await
            .unwrap();
        }
        // All six endpoints individually helpful.
        let topics: Vec<(String, f32)> = seeds
            .iter()
            .flat_map(|((a, b), _)| {
                [(a.to_string(), 1.0), (b.to_string(), 1.0)]
            })
            .collect();
        help.record_window(&topics, now).await.unwrap();

        // Pre-stamp ONE pair in the proposal chain to exercise
        // the dedup arm — the canonical id is alphabetical, so
        // (deploy, rollback) hashes as `deploy+rollback`.
        plog.append_pending(
            "consolidate-pair:deploy+rollback".into(),
            now_ms,
            "test-preseed".into(),
            ProposedPersonaDelta {
                category: PersonaDeltaCategory::LearnedContext,
                op: PersonaDeltaOp::AppendList {
                    value: "already filed".into(),
                },
                reason: None,
                supersedes_proposal_id: None,
            },
        )
        .await
        .unwrap();

        let stat = shared_persona_consolidation_stat();
        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 1_000_000,
            role_override: None,
            enabled: true,
            skip_when_idle: false,
            min_audit_entries_to_fire: 1,
        };
        let deps = PersonaConsolidationDeps {
            config: aivyx_config::PersonaConsolidationConfig {
                enabled: true,
                min_affinity: 1.0,
                min_samples: 2,
                min_topic_helpfulness: 0.0,
                max_proposals_per_cycle: 5,
                enable_supersession: false,
            },
            cooccurrence_ledger: Arc::clone(&cooc),
            helpfulness_ledger: Arc::clone(&help),
            proposal_log: Arc::clone(&plog),
            phraser: Arc::new(OkPhraser),
            stat: Some(stat.clone()),
            persona_log: None,
            pair_below_affinity: 1.0,
        };

        // Cycle 1: two new proposals filed (the third was
        // pre-seeded, so it dedups).
        run_persona_consolidation_pass(&deps, &sched, now_ms)
            .await;
        let pending = plog.list(ProposalStatusFilter::Pending);
        assert_eq!(
            pending.len(),
            3,
            "1 pre-seed + 2 newly filed"
        );
        let new_ids: std::collections::HashSet<String> = pending
            .iter()
            .map(|p| p.id.clone())
            .collect();
        assert!(new_ids.contains(
            "consolidate-pair:deploy+rollback"
        ));
        assert!(new_ids
            .contains("consolidate-pair:css+frontend"));
        assert!(
            new_ids.contains("consolidate-pair:borrow+rust")
        );
        {
            let s = stat.read().unwrap();
            let s = s.as_ref().expect("stat populated");
            assert_eq!(s.filed, 2);
            assert!(!s.llm_unavailable);
        }

        // Cycle 2: idempotent — no NEW proposals since every
        // eligible pair is already in the chain.
        run_persona_consolidation_pass(&deps, &sched, now_ms)
            .await;
        let pending2 = plog.list(ProposalStatusFilter::Pending);
        assert_eq!(
            pending2.len(),
            3,
            "second cycle is a no-op (full dedup)"
        );

        // Cycle 3: disabled config — even with new evidence,
        // nothing fires (byte-identical to pre-Phase-87).
        let off = PersonaConsolidationDeps {
            config: aivyx_config::PersonaConsolidationConfig {
                enabled: false,
                ..deps.config.clone()
            },
            ..deps
        };
        run_persona_consolidation_pass(&off, &sched, now_ms)
            .await;
        assert_eq!(
            plog.list(ProposalStatusFilter::Pending).len(),
            3,
            "disabled config is a complete no-op"
        );

        // Cycle 4: armed but LLM unavailable — every survivor's
        // phrasing returns None; nothing files; the surface
        // flag flips so the operator can distinguish "quiet"
        // from "broken". To create new evidence the
        // consolidation pass can act on, introduce a fourth
        // pair (the prior three are all already in the chain).
        cooc.record_window(
            &[(("alpha".into(), "beta".into()), 10.0)],
            now,
        )
        .await
        .unwrap();
        cooc.record_window(
            &[(("alpha".into(), "beta".into()), 0.001)],
            now,
        )
        .await
        .unwrap();
        help.record_window(
            &[("alpha".into(), 1.0), ("beta".into(), 1.0)],
            now,
        )
        .await
        .unwrap();
        let down = PersonaConsolidationDeps {
            config: aivyx_config::PersonaConsolidationConfig {
                enabled: true,
                min_affinity: 1.0,
                min_samples: 2,
                min_topic_helpfulness: 0.0,
                max_proposals_per_cycle: 5,
                enable_supersession: false,
            },
            cooccurrence_ledger: Arc::clone(&cooc),
            helpfulness_ledger: Arc::clone(&help),
            proposal_log: Arc::clone(&plog),
            phraser: Arc::new(DownPhraser),
            stat: Some(stat.clone()),
            persona_log: None,
            pair_below_affinity: 1.0,
        };
        run_persona_consolidation_pass(&down, &sched, now_ms)
            .await;
        assert_eq!(
            plog.list(ProposalStatusFilter::Pending).len(),
            3,
            "LLM-down cycle files nothing"
        );
        {
            let s = stat.read().unwrap();
            let s = s.as_ref().expect("stat populated");
            assert_eq!(s.filed, 0);
            assert!(
                s.llm_unavailable,
                "every survivor's phrasing failed → cycle-wide \
                 LLM unavailable"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Phase 88 — pattern-driven Persona decay ---------------

    #[tokio::test]
    async fn pair_decay_protects_durable_pair_and_retires_drifted_pair()
    {
        use crate::cooccurrence_ledger::PersistentCooccurrenceLedger;
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
            "aivyx-pl-pair-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([88u8; 32]),
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
        // OLD facets (origin = 0; now > age horizon 1000) so
        // age-decay is eligible on every facet whose
        // `reinforced` flag is *false*. The Phase 81/85 rule is
        // "reinforced = any later delta in the same category";
        // the LAST delta in a category is the only one not
        // reinforced. So we put the protection-target LAST so
        // age-decay would naturally fire on it — only the pair
        // signal protects it. The drifted facet sits in the
        // middle (reinforced, so age-decay would NOT fire on
        // it) — the pair signal must be what early-decays it,
        // bypassing the reinforced check.
        let now_ms = 1_000_000_000u64;
        let now_secs = now_ms / 1000; // 1_000_000
        // Padding to clear `min_soft_facets` (2): two old
        // reflection-authored facets, reinforced by later
        // deltas → age-decay suppressed on them. Pure controls.
        persona_log
            .append(delta(
                "p1",
                "filler one",
                "seed",
                0,
            ))
            .await
            .unwrap();
        persona_log
            .append(delta(
                "p2",
                "filler two",
                "seed",
                0,
            ))
            .await
            .unwrap();
        // Drifted pair facet (alpha + beta, affinity 0.3).
        // Reinforced by the later durable delta → age-decay
        // CANNOT fire. Decay must come from the pair signal
        // alone (sustained_pair_decay bypasses reinforced).
        persona_log
            .append(delta(
                "a",
                "you mix alpha + beta",
                "consolidate-pair:alpha+beta",
                0,
            ))
            .await
            .unwrap();
        // Durable pair facet (deploy + rollback, affinity 5.0).
        // LAST delta in LearnedContext → reinforced = false.
        // Age-decay WOULD fire without protection; only the
        // sustained-positive pair signal saves it.
        persona_log
            .append(delta(
                "b",
                "you deploy with rollback in mind",
                "consolidate-pair:deploy+rollback",
                0,
            ))
            .await
            .unwrap();

        // Co-occurrence ledger: deploy+rollback durable,
        // alpha+beta drifted. Stamp at `now_secs` so read-time
        // decay leaves the scores intact.
        let cooc =
            Arc::new(PersistentCooccurrenceLedger::new(
                store.domain(KeyDomain::CooccurrenceLedger),
            ));
        cooc.record_window(
            &[
                (("deploy".into(), "rollback".into()), 5.0),
                (("alpha".into(), "beta".into()), 0.3),
            ],
            now_secs,
        )
        .await
        .unwrap();

        let proposal_log = Arc::new(
            PersistentPersonaProposalLog::open(
                store.domain(KeyDomain::PersonaProposals),
                b"pl-pair-key".to_vec(),
            )
            .await
            .unwrap(),
        );

        let cfg = aivyx_config::PersonaLifecycleConfig {
            enabled: true,
            consolidation_similarity: 0.92,
            decay_max_age_secs: 1_000,
            min_soft_facets: 2,
            decay_unhelpful_threshold: -2.0,
            decay_min_samples: 3,
            decay_pair_below_affinity: 1.0,
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
            skip_when_idle: false,
            min_audit_entries_to_fire: 1,
        };

        // With the co-occurrence ledger present + `signal_decay
        // = true`, Phase 88's two arms engage:
        // - `sustained_pair_decay` early-decays the drifted
        //   pair facet (bypassing the `reinforced` check that
        //   would otherwise suppress age-decay).
        // - `sustained_pair_strong` PROTECTS the durable pair
        //   facet from age-decay (the only un-reinforced facet
        //   in the chain, so age-decay would otherwise fire).
        // The fallback path (no ledger → byte-identical Phase
        // 85 age-only) is covered by the existing Phase 85
        // `helpfulness_decay_uses_recall_fb_provenance` test
        // already in this module.
        let with_ledger = PersonaLifecycleDeps {
            config: cfg.clone(),
            persona_log: Arc::clone(&persona_log),
            proposal_log: Arc::clone(&proposal_log),
            embedding: Arc::new(NoEmb),
            helpfulness_ledger: None,
            cooccurrence_ledger: Some(Arc::clone(&cooc)),
            stat: None,
        };
        run_persona_lifecycle_pass(&with_ledger, &sched, now_ms)
            .await;
        let pending =
            proposal_log.list(ProposalStatusFilter::Pending);
        assert_eq!(
            pending.len(),
            1,
            "exactly one proposal: the drifted pair early-\
             decays; the durable pair is protected from \
             age-decay"
        );
        let good = match &pending[0].proposed_op.op {
            PersonaDeltaOp::RemoveList { value } => value.clone(),
            o => panic!("expected RemoveList, got {o:?}"),
        };
        assert_eq!(
            good, "you mix alpha + beta",
            "drifted pair early-decays via the pair signal"
        );
        // The reason text cites the pair + the decayed
        // affinity (the operator-visible provenance).
        let reason = pending[0]
            .proposed_op
            .reason
            .clone()
            .unwrap_or_default();
        assert!(
            reason
                .contains("co-occurrence pair `alpha` + `beta`"),
            "reason cites the pair: {reason}"
        );
        assert!(
            reason.contains("relationship no longer durable"),
            "reason cites the relationship signal: {reason}"
        );

        // Second cycle is idempotent — same proposal id, dedup
        // hits, nothing new files.
        run_persona_lifecycle_pass(&with_ledger, &sched, now_ms)
            .await;
        assert_eq!(
            proposal_log
                .list(ProposalStatusFilter::Pending)
                .len(),
            1,
            "second cycle is a no-op (dedup against the \
             existing proposal id)"
        );

        // `signal_decay = false` (decay arm disarmed) → no
        // additional proposals fire even with the ledger
        // present. Reuses the same proposal chain so the cycle
        // ABOVE's one proposal is still the only one when
        // we're done.
        let off = PersonaLifecycleDeps {
            config: aivyx_config::PersonaLifecycleConfig {
                signals: aivyx_config::PersonaLifecycleSignals {
                    consolidate: true,
                    decay: false,
                },
                ..cfg
            },
            persona_log,
            proposal_log: Arc::clone(&proposal_log),
            embedding: Arc::new(NoEmb),
            helpfulness_ledger: None,
            cooccurrence_ledger: Some(Arc::clone(&cooc)),
            stat: None,
        };
        run_persona_lifecycle_pass(&off, &sched, now_ms).await;
        assert_eq!(
            proposal_log
                .list(ProposalStatusFilter::Pending)
                .len(),
            1,
            "decay-signal disarmed → no new proposals"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Phase 91 — LLM-judged recall pass ---------------------

    #[tokio::test]
    async fn judgment_pass_records_idempotent_with_dedup_and_cap()
    {
        use crate::recall_judgment::{
            shared_recall_judgment_stat, RecallJudgeInput,
            RecallJudge, RecallJudgment,
        };
        use crate::recall_log::{
            PersistentRecallLog, RecallHit,
        };
        use aivyx_crypto::MasterKey;
        use aivyx_memory::InMemoryMemory;
        use aivyx_storage::{
            KeyDomain, RedbStorage, Storage, StorageConfig,
        };
        use async_trait::async_trait;
        use std::sync::Arc;

        /// Returns `Used` for the first input, `Irrelevant`
        /// for the second, `Hurt` for the third, ... cycling.
        struct DeterministicJudge;
        #[async_trait]
        impl RecallJudge for DeterministicJudge {
            async fn judge(
                &self,
                inputs: &[RecallJudgeInput],
            ) -> Vec<Option<RecallJudgment>> {
                inputs
                    .iter()
                    .enumerate()
                    .map(|(i, _)| match i % 3 {
                        0 => Some(RecallJudgment::Used),
                        1 => Some(RecallJudgment::Irrelevant),
                        _ => Some(RecallJudgment::Hurt),
                    })
                    .collect()
            }
        }
        /// Returns `None` for every input — simulates a
        /// cycle-wide LLM outage.
        struct DownJudge;
        #[async_trait]
        impl RecallJudge for DownJudge {
            async fn judge(
                &self,
                inputs: &[RecallJudgeInput],
            ) -> Vec<Option<RecallJudgment>> {
                vec![None; inputs.len()]
            }
        }

        let base = std::env::var("TMPDIR")
            .unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base).join(format!(
            "aivyx-rj-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([91u8; 32]),
        )
        .await
        .unwrap();

        let recall_log = Arc::new(PersistentRecallLog::new(
            store.domain(KeyDomain::RecallEvents),
        ));
        let memory: Arc<dyn aivyx_memory::Memory> =
            Arc::new(InMemoryMemory::new());

        // Seed three recallable entries — body recovery
        // succeeds for all three.
        let s1 = memory.put("deploy", "the runbook").await.unwrap();
        let s2 =
            memory.put("rollback", "git revert").await.unwrap();
        let s3 = memory.put("auth", "JWT details").await.unwrap();

        // Append one recall event with three unjudged hits.
        let now_secs = 1_000_000u64;
        let now_ms = now_secs * 1000;
        recall_log
            .append(&crate::recall_log::RecallEvent {
                ts_secs: now_secs,
                session_id: aivyx_core::SessionId::new(),
                query_text: String::new(),
                hits: vec![
                    RecallHit {
                        topic: "deploy".into(),
                        seq: s1,
                        score: 0.9,
                        cluster: false,
                        judgment: None,
                    },
                    RecallHit {
                        topic: "rollback".into(),
                        seq: s2,
                        score: 0.8,
                        cluster: false,
                        judgment: None,
                    },
                    RecallHit {
                        topic: "auth".into(),
                        seq: s3,
                        score: 0.7,
                        cluster: false,
                        judgment: None,
                    },
                ],
            })
            .await
            .unwrap();

        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 86_400,
            role_override: None,
            enabled: true,
            skip_when_idle: false,
            min_audit_entries_to_fire: 1,
        };
        let stat = shared_recall_judgment_stat();
        let deps = RecallJudgmentDeps {
            config: aivyx_config::RecallJudgmentConfig {
                enabled: true,
                max_recalls_per_cycle: 10,
            },
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            judge: Arc::new(DeterministicJudge),
            stat: Some(Arc::clone(&stat)),
        };

        // Cycle 1: every hit is unjudged → all three get
        // classified (Used / Irrelevant / Hurt rotation).
        run_recall_judgment_pass(&deps, &sched, now_ms).await;
        let events =
            recall_log.events_since(0).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].hits[0].judgment,
            Some(RecallJudgment::Used),
        );
        assert_eq!(
            events[0].hits[1].judgment,
            Some(RecallJudgment::Irrelevant),
        );
        assert_eq!(
            events[0].hits[2].judgment,
            Some(RecallJudgment::Hurt),
        );
        {
            let s = stat.read().unwrap();
            let s = s.as_ref().expect("stat populated");
            assert_eq!(s.judged, 3);
            assert_eq!(s.used, 1);
            assert_eq!(s.irrelevant, 1);
            assert_eq!(s.hurt, 1);
            assert_eq!(s.skipped, 0);
            assert!(!s.llm_unavailable);
            assert_eq!(s.pairs.len(), 3);
        }

        // Cycle 2: every hit is already judged → idempotent.
        // The pass walks the row, finds no unjudged hits,
        // builds an empty input batch, never calls the LLM.
        // The stat records `judged = 0, skipped = 0`.
        run_recall_judgment_pass(&deps, &sched, now_ms).await;
        let events2 =
            recall_log.events_since(0).await.unwrap();
        // Same three judgments — no mutation.
        assert_eq!(
            events2[0].hits[0].judgment,
            Some(RecallJudgment::Used),
        );
        {
            let s = stat.read().unwrap();
            let s = s.as_ref().expect("stat populated");
            assert_eq!(s.judged, 0);
            assert_eq!(s.skipped, 0);
            assert!(!s.llm_unavailable);
        }

        // Cycle 3: a new event with two unjudged hits + a
        // cap of 1 → exactly one hit gets judged, one is
        // skipped (and rolled to the next cycle).
        recall_log
            .append(&crate::recall_log::RecallEvent {
                ts_secs: now_secs + 1,
                session_id: aivyx_core::SessionId::new(),
                query_text: String::new(),
                hits: vec![
                    RecallHit {
                        topic: "deploy".into(),
                        seq: s1,
                        score: 0.5,
                        cluster: false,
                        judgment: None,
                    },
                    RecallHit {
                        topic: "rollback".into(),
                        seq: s2,
                        score: 0.4,
                        cluster: false,
                        judgment: None,
                    },
                ],
            })
            .await
            .unwrap();
        let capped_deps = RecallJudgmentDeps {
            config: aivyx_config::RecallJudgmentConfig {
                enabled: true,
                max_recalls_per_cycle: 1,
            },
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            judge: Arc::new(DeterministicJudge),
            stat: Some(Arc::clone(&stat)),
        };
        run_recall_judgment_pass(
            &capped_deps,
            &sched,
            now_ms + 1000,
        )
        .await;
        {
            let s = stat.read().unwrap();
            let s = s.as_ref().expect("stat populated");
            assert_eq!(s.judged, 1, "cap = 1 → one judged");
            assert_eq!(s.skipped, 1, "the other rolls over");
        }

        // Cycle 4: `enabled = false` → no-op even with new
        // unjudged hits (a fresh recall event seeded below).
        recall_log
            .append(&crate::recall_log::RecallEvent {
                ts_secs: now_secs + 2,
                session_id: aivyx_core::SessionId::new(),
                query_text: String::new(),
                hits: vec![RecallHit {
                    topic: "deploy".into(),
                    seq: s1,
                    score: 0.3,
                    cluster: false,
                    judgment: None,
                }],
            })
            .await
            .unwrap();
        let off = RecallJudgmentDeps {
            config: aivyx_config::RecallJudgmentConfig {
                enabled: false,
                max_recalls_per_cycle: 10,
            },
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            judge: Arc::new(DeterministicJudge),
            stat: Some(Arc::clone(&stat)),
        };
        // Stat from cycle 3 is preserved (the pass returns
        // immediately on disabled config without touching
        // the sink). Re-read after cycle 4 to confirm.
        let snapshot_before =
            stat.read().unwrap().clone().unwrap();
        run_recall_judgment_pass(&off, &sched, now_ms + 2000)
            .await;
        let snapshot_after =
            stat.read().unwrap().clone().unwrap();
        assert_eq!(snapshot_before, snapshot_after);

        // Cycle 5: LLM-down judge → every survivor returns
        // `None`; `llm_unavailable = true` records, no hit
        // mutates.
        let down = RecallJudgmentDeps {
            config: aivyx_config::RecallJudgmentConfig {
                enabled: true,
                max_recalls_per_cycle: 10,
            },
            recall_log: Arc::clone(&recall_log),
            memory: Arc::clone(&memory),
            judge: Arc::new(DownJudge),
            stat: Some(Arc::clone(&stat)),
        };
        run_recall_judgment_pass(&down, &sched, now_ms + 3000)
            .await;
        {
            let s = stat.read().unwrap();
            let s = s.as_ref().expect("stat populated");
            assert!(
                s.llm_unavailable,
                "every survivor's judgment was `None`"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Phase 92 — pattern-driven supersession ----------------

    #[tokio::test]
    async fn supersession_pass_files_two_linked_proposals_then_dedups()
    {
        use crate::cooccurrence_ledger::PersistentCooccurrenceLedger;
        use crate::helpfulness_ledger::PersistentHelpfulnessLedger;
        use crate::persona::{
            PersonaDelta, PersonaDeltaCategory, PersonaDeltaOp,
            PersistentPersonaLog,
        };
        use crate::persona_consolidation::{
            shared_persona_consolidation_stat, PairPhraser,
        };
        use crate::persona_proposal::{
            PersistentPersonaProposalLog, ProposalStatusFilter,
        };
        use aivyx_crypto::MasterKey;
        use aivyx_storage::{
            KeyDomain, RedbStorage, Storage, StorageConfig,
        };
        use async_trait::async_trait;
        use std::sync::Arc;

        struct DeterministicPhraser;
        #[async_trait]
        impl PairPhraser for DeterministicPhraser {
            async fn phrase(
                &self,
                a: &str,
                b: &str,
            ) -> Option<String> {
                Some(format!(
                    "You consistently work with `{a}` and \
                     `{b}` together."
                ))
            }
        }

        let base = std::env::var("TMPDIR")
            .unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base).join(format!(
            "aivyx-supersede-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([92u8; 32]),
        )
        .await
        .unwrap();

        let cooc =
            Arc::new(PersistentCooccurrenceLedger::new(
                store.domain(KeyDomain::CooccurrenceLedger),
            ));
        let help =
            Arc::new(PersistentHelpfulnessLedger::new(
                store.domain(KeyDomain::HelpfulnessLedger),
            ));
        let proposal_log = Arc::new(
            PersistentPersonaProposalLog::open(
                store.domain(KeyDomain::PersonaProposals),
                b"supersede-key".to_vec(),
            )
            .await
            .unwrap(),
        );
        let persona_log = Arc::new(
            PersistentPersonaLog::open(
                store.domain(KeyDomain::Persona),
                vec![3u8; 32],
            )
            .await
            .unwrap(),
        );

        // Seed an APPLIED `consolidate-pair:auth+jwt` facet
        // on the Persona chain.
        let now_secs = 1_000_000u64;
        let now_ms = now_secs * 1000;
        persona_log
            .append(PersonaDelta {
                delta_id: "d1".into(),
                proposed_at_unix_ms: now_ms,
                approved_at_unix_ms: now_ms,
                proposal_id:
                    "consolidate-pair:auth+jwt".into(),
                category: PersonaDeltaCategory::LearnedContext,
                op: PersonaDeltaOp::AppendList {
                    value: "you work auth with jwt".into(),
                },
            })
            .await
            .unwrap();

        // (auth, jwt) decayed, (auth, sessions) strong + helpful.
        cooc.record_window(
            &[(("auth".into(), "jwt".into()), 0.3)],
            now_secs,
        )
        .await
        .unwrap();
        cooc.record_window(
            &[(("auth".into(), "sessions".into()), 5.0)],
            now_secs,
        )
        .await
        .unwrap();
        cooc.record_window(
            &[(("auth".into(), "sessions".into()), 0.001)],
            now_secs,
        )
        .await
        .unwrap();
        help.record_window(
            &[
                ("auth".into(), 1.0),
                ("jwt".into(), 1.0),
                ("sessions".into(), 1.0),
            ],
            now_secs,
        )
        .await
        .unwrap();

        let sched = aivyx_config::ReflectionScheduleConfig {
            name: "nightly".into(),
            cron: "0 0 3 * * *".into(),
            lookback_window_secs: 86_400,
            role_override: None,
            enabled: true,
            skip_when_idle: false,
            min_audit_entries_to_fire: 1,
        };
        let stat = shared_persona_consolidation_stat();
        let deps = PersonaConsolidationDeps {
            config: aivyx_config::PersonaConsolidationConfig {
                enabled: true,
                min_affinity: 1.0,
                min_samples: 2,
                min_topic_helpfulness: 0.0,
                max_proposals_per_cycle: 5,
                enable_supersession: true,
            },
            cooccurrence_ledger: Arc::clone(&cooc),
            helpfulness_ledger: Arc::clone(&help),
            proposal_log: Arc::clone(&proposal_log),
            phraser: Arc::new(DeterministicPhraser),
            stat: Some(Arc::clone(&stat)),
            persona_log: Some(Arc::clone(&persona_log)),
            pair_below_affinity: 1.0,
        };

        // Cycle 1: two linked proposals file.
        run_persona_consolidation_pass(&deps, &sched, now_ms)
            .await;
        let pending = proposal_log.list(ProposalStatusFilter::Pending);
        // Expect exactly two pending proposals — the
        // RemoveList (under `supersede-remove:…`) and the
        // AppendList (under
        // `consolidate-pair:auth+sessions`).
        assert_eq!(
            pending.len(),
            2,
            "supersession files two linked proposals"
        );
        let ids: std::collections::HashSet<String> = pending
            .iter()
            .map(|p| p.id.clone())
            .collect();
        assert!(
            ids.contains("consolidate-pair:auth+sessions"),
            "the new facet's AppendList is filed under the \
             canonical id"
        );
        assert!(
            ids.contains(
                "supersede-remove:consolidate-pair:auth+jwt",
            ),
            "the old facet's RemoveList is filed under a \
             distinct id linked to the original"
        );
        // The AppendList's `supersedes_proposal_id` points at
        // the original old proposal_id; the RemoveList's
        // points at the new one (cross-linked).
        let append = pending
            .iter()
            .find(|p| {
                p.id == "consolidate-pair:auth+sessions"
            })
            .unwrap();
        assert_eq!(
            append.proposed_op.supersedes_proposal_id,
            Some("consolidate-pair:auth+jwt".to_string()),
        );
        let remove = pending
            .iter()
            .find(|p| {
                p.id == "supersede-remove:consolidate-pair:auth+jwt"
            })
            .unwrap();
        assert_eq!(
            remove.proposed_op.supersedes_proposal_id,
            Some(
                "consolidate-pair:auth+sessions".to_string(),
            ),
        );
        {
            let s = stat.read().unwrap();
            let s = s.as_ref().expect("stat populated");
            assert_eq!(s.superseded, 1);
            assert_eq!(s.filed, 2);
        }

        // Cycle 2: idempotent — the AppendList side dedups
        // against the chain; the supersession detector won't
        // emit a candidate whose new_id is already present.
        run_persona_consolidation_pass(&deps, &sched, now_ms)
            .await;
        let pending2 = proposal_log.list(ProposalStatusFilter::Pending);
        assert_eq!(
            pending2.len(),
            2,
            "second cycle is a no-op (full dedup)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

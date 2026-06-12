//! Last-cycle "trust surface" stats + judgments (Phase 78–91) — moved to
//! `aivyx-ipc` in M.2d.
//!
//! The ephemeral per-cycle outcomes the daemon ships over `GetLearningInsights`
//! (LLM-judged recall, proactive surfacings, persona consolidation / lifecycle,
//! correction judgment, …). Pure data; the passes that compute them (LLM
//! providers, embeddings, storage, the reflection scheduler) stay in
//! `aivyx-channel`.

use serde::{Deserialize, Serialize};

// SoftCategory::as_persona_category maps onto the persona delta taxonomy.
use crate::persona::{PersonaDelta, PersonaDeltaCategory};


/// Phase 91 — the 3-way LLM-judged per-recall classification
/// (Q2a). Mirrors the operator-facing helpfulness shape of
/// the existing structural signal at finer granularity:
///   `Used`       — the response leveraged the recall.
///   `Irrelevant` — the response ignored it; no harm done.
///   `Hurt`       — the recall misled the response.
///
/// Stable string labels for JSON wire-format: `"used"`,
/// `"irrelevant"`, `"hurt"` (snake_case, matching the rest of
/// the IPC enum convention).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RecallJudgment {
    Used,
    Irrelevant,
    Hurt,
}

/// Phase 91 (Q4a) — the last reflection cycle's LLM-judged
/// recall outcome, for the Phase 78 trust surface. Ephemeral
/// (last-cycle only, not persisted); an actuator that
/// silently classifies recalls must stay legible.
///
/// `llm_unavailable` records the cycle-wide degenerate case
/// (the LLM provider returned all-`None` judgments), so a
/// quiet "0 judged" cycle is distinguishable from "0 judged,
/// LLM down."
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallJudgmentStat {
    pub ts_secs: u64,
    /// How many recall hits got a judgment this cycle.
    pub judged: u32,
    /// Of `judged`, how many were classified `Used`.
    pub used: u32,
    /// Of `judged`, how many were classified `Irrelevant`.
    pub irrelevant: u32,
    /// Of `judged`, how many were classified `Hurt`.
    pub hurt: u32,
    /// Recall hits that the per-cycle cap rolled to the next
    /// cycle (`max_recalls_per_cycle` is bounded; the
    /// remainder is judged later). Distinct from
    /// `llm_unavailable`.
    pub skipped: u32,
    /// `true` iff the cycle attempted any judgments but the
    /// LLM returned all-`None` (cycle-wide failure).
    pub llm_unavailable: bool,
    /// `(topic, judgment)` per actually-classified hit, in
    /// classification order. Bounded by `judged`; the Phase 78
    /// surface renders this list directly.
    pub pairs: Vec<(String, RecallJudgment)>,
}

/// Which structural fact produced a surfacing.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
)]
pub enum ProactiveKind {
    /// A memory about to be TTL-evicted.
    TtlExpiry,
    /// A topic whose recalls keep helping.
    RecallCluster,
    /// A `@due:` reminder whose time has arrived.
    DueReminder,
}

/// Phase 80 (Q4a) — one item the last proactive cycle actually
/// dispatched, for the Phase 78 trust surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProactiveSurfaced {
    pub kind: ProactiveKind,
    pub topic: String,
    pub reason: String,
}

/// The last proactive cycle's outcome. Ephemeral (last-cycle
/// only, not persisted) — an autonomous *outbound* action must
/// still be legible, the Phase 78 posture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProactiveStat {
    pub ts_secs: u64,
    pub surfaced: Vec<ProactiveSurfaced>,
    pub deduped: u32,
    pub capped: u32,
}

/// The six reducible soft-list categories — and *only* these.
/// There is deliberately no variant for the scalar identity
/// (`assistant_name` / `operator_profile` /
/// `communication_style`) or for `behavioral_constraints`: the
/// lifecycle layer is structurally incapable of proposing a
/// change to the always-on core (Q4a — the Phase 79 invariant
/// extended).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
)]
pub enum SoftCategory {
    PrimaryUseCases,
    BehavioralPreferences,
    LearnedContext,
    CommunicationAdaptations,
    CharacterTraits,
    RelationshipMilestones,
}

impl SoftCategory {
    /// Stable lower-snake label for ids / breadcrumbs / the
    /// Phase 78 surface.
    pub fn label(self) -> &'static str {
        match self {
            SoftCategory::PrimaryUseCases => "primary_use_cases",
            SoftCategory::BehavioralPreferences => {
                "behavioral_preferences"
            }
            SoftCategory::LearnedContext => "learned_context",
            SoftCategory::CommunicationAdaptations => {
                "communication_adaptations"
            }
            SoftCategory::CharacterTraits => "character_traits",
            SoftCategory::RelationshipMilestones => {
                "relationship_milestones"
            }
        }
    }

    /// The six categories in stable order.
    pub const ALL: [SoftCategory; 6] = [
        SoftCategory::PrimaryUseCases,
        SoftCategory::BehavioralPreferences,
        SoftCategory::LearnedContext,
        SoftCategory::CommunicationAdaptations,
        SoftCategory::CharacterTraits,
        SoftCategory::RelationshipMilestones,
    ];

    /// Map to the persona-chain delta category. Total over the
    /// six soft lists — there is no arm for the always-on core,
    /// so a lifecycle proposal can only ever target a soft list.
    pub fn to_delta_category(self) -> PersonaDeltaCategory {
        match self {
            SoftCategory::PrimaryUseCases => {
                PersonaDeltaCategory::PrimaryUseCases
            }
            SoftCategory::BehavioralPreferences => {
                PersonaDeltaCategory::BehavioralPreferences
            }
            SoftCategory::LearnedContext => {
                PersonaDeltaCategory::LearnedContext
            }
            SoftCategory::CommunicationAdaptations => {
                PersonaDeltaCategory::CommunicationAdaptations
            }
            SoftCategory::CharacterTraits => {
                PersonaDeltaCategory::CharacterTraits
            }
            SoftCategory::RelationshipMilestones => {
                PersonaDeltaCategory::RelationshipMilestones
            }
        }
    }
}

/// One filed lifecycle proposal, for the Phase 78 trust
/// surface (Task 5). Ephemeral last-cycle only — an
/// assistant-initiated identity proposal must stay legible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaLifecycleProposed {
    /// "consolidate" or "decay".
    pub kind: String,
    pub category: SoftCategory,
    /// The soft-list facet value the filed proposal removes.
    pub value: String,
    pub reason: String,
}

/// The last lifecycle cycle's outcome. Ephemeral (last-cycle
/// only, not persisted) — the Phase 78 posture extended to the
/// identity-maintenance layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaLifecycleStat {
    pub ts_secs: u64,
    pub proposed: Vec<PersonaLifecycleProposed>,
    pub deduped: u32,
}

/// Phase 178 (Q4a) — last reflection cycle's correction-judgment
/// outcome, for the Phase 78 trust surface. Ephemeral
/// (last-cycle only). `llm_unavailable` flags the cycle-wide
/// degenerate case so a quiet cycle is distinguishable from an
/// LLM outage.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CorrectionJudgmentStat {
    pub ts_secs: u64,
    /// Judgeable events (a follow-up query was captured) judged
    /// this cycle.
    pub judged: u32,
    pub rework: u32,
    pub praise: u32,
    pub unrelated: u32,
    /// Events folded on the structural fallback (no follow-up
    /// query captured, or the judge failed/over-cap).
    pub structural_fallback: u32,
    pub llm_unavailable: bool,
}

/// Phase 172 — last reflection cycle's correction-consolidation
/// outcome, for the Phase 78 trust surface. Ephemeral
/// (last-cycle only, not persisted); a pattern-driven actuator
/// must still be legible.
///
/// `llm_unavailable` records the cycle-wide degenerate case
/// (the LLM provider could not phrase any survivor), so a quiet
/// "0 filed" cycle is distinguishable from "0 filed, LLM down."
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrectionConsolidationStat {
    pub ts_secs: u64,
    pub filed: u32,
    pub llm_unavailable: bool,
    /// Each actually-filed topic, for the Phase 78 surface.
    /// Stable order = filing order.
    pub topics: Vec<String>,
}

/// Phase 87 (Q4a) — last reflection cycle's consolidation
/// outcome, for the Phase 78 trust surface. Ephemeral
/// (last-cycle only, not persisted); a pattern-driven
/// actuator must still be legible.
///
/// `llm_unavailable` records the cycle-wide degenerate case
/// (the LLM provider could not phrase any survivor), so a
/// quiet "0 filed" cycle is distinguishable from "0 filed,
/// LLM down."
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonaConsolidationStat {
    pub ts_secs: u64,
    pub filed: u32,
    pub deduped: u32,
    pub skipped_unhelpful: u32,
    pub llm_unavailable: bool,
    /// `(A, B)` of each actually-filed pair, for the Phase 78
    /// surface. Stable order = filing order.
    pub pairs: Vec<(String, String)>,
    /// Phase 92 — how many supersession pairs the cycle
    /// filed. Each supersession produces TWO chain entries
    /// (a `RemoveList` for the old facet + an `AppendList`
    /// for the new); `superseded` counts the supersession
    /// EVENTS, not the chain entries. The `filed` count
    /// includes both halves of every supersession plus any
    /// standard Phase 87 consolidations. `#[serde(default)]`
    /// so older frames decode unchanged (Phase 84/91
    /// wire-compat precedent).
    #[serde(default)]
    pub superseded: u32,
}

/// Phase 79 (Q4a) — the last turn's Persona selection, for the
/// Phase 78 trust surface. Ephemeral (last-turn only, not
/// persisted): an adaptive Soul that silently picks which
/// identity to apply must still be legible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaSelectionStat {
    pub ts_secs: u64,
    pub selected: usize,
    pub total: usize,
}

/// Phase 95 — per-schedule accumulating cadence stat. The
/// scheduler increments `fired` on every actual fire and
/// `skipped` on every `should_fire_cycle = false` decision.
/// In-memory across the daemon lifetime; daemon restart
/// resets to all-zero.
///
/// `#[serde(default)]` on each field keeps the IPC round-
/// trip wire-compatible — clients on older versions decode
/// the stat with zeroed missing fields, and pre-Phase-95
/// daemons (which never produce this shape) still satisfy
/// new clients' default-zero expectation.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize,
)]
pub struct RecentReflectionStat {
    #[serde(default)]
    pub fired: u32,
    #[serde(default)]
    pub skipped: u32,
}

/// Phase 84 (Q4a) — the last turn's cluster-aware co-recall
/// outcome, for the Phase 78 trust surface. Ephemeral
/// (last-turn only, not persisted): an associative recall that
/// silently widens context must stay legible. `pairs` is
/// `(driver_topic, injected_sibling_topic)` for what actually
/// landed (post budget-share).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecallClusterStat {
    pub ts_secs: u64,
    pub injected: usize,
    pub pairs: Vec<(String, String)>,
}

// --- recall-feedback digest (Phase 77+) + identity export ---

/// One turn that contributed to a topic's score, as the
/// operator sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContributingTurn {
    pub ts_secs: u64,
    /// Outcome label of the matched turn, or `None` if the
    /// recall matched no turn in the window.
    pub outcome_kind: Option<String>,
    /// Signed contribution (`+`/`−`weight), or `None` for a
    /// no-signal / unmatched recall.
    pub signal: Option<f32>,
    /// The seqs of *this topic's* memories injected on that
    /// turn.
    pub seqs: Vec<u64>,
}

/// Why one Pending (or resolved) recall-driven Persona proposal
/// exists, reconstructed from the recall log (Q4a).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposalProvenance {
    pub proposal_id: String,
    pub topic: String,
    pub status: String,
    /// Net helpfulness across this topic that drove the
    /// proposal (the same sum `proposals_from_tally` thresholded
    /// on).
    pub net_score: f32,
    /// The agent's stated reason on the proposal record.
    pub reason: Option<String>,
    /// The recalls/turns that produced the score, newest first.
    pub contributing: Vec<ContributingTurn>,
}

/// Per-window operational picture of the self-learning loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningDigest {
    /// The lookback the digest was computed over (seconds).
    pub window_secs: u64,
    /// Recalls in the window (any — matched or not).
    pub recalls_total: usize,
    /// Recalls that produced a signal (matched a turn with a
    /// helpful/unhelpful outcome).
    pub recalls_scored: usize,
    /// Distinct `(topic, seq)` entries the retention actuator
    /// would keep warm this window.
    pub promoted: usize,
    /// Distinct scored entries below the promote threshold
    /// (net-negative or too weak) — left to age out.
    pub not_promoted: usize,
    /// Top helpful topics (net score, descending).
    pub top_helpful: Vec<(String, f32)>,
    /// Top unhelpful topics (net score, ascending = most
    /// negative first).
    pub top_unhelpful: Vec<(String, f32)>,
    /// Recall-driven Persona proposals visible in the chain.
    pub proposals_in_window: usize,
    /// Phase 93 — whether the recall-feedback correlator
    /// was running with per-hit judgment override
    /// (`[recall_feedback].use_judgment_signal = true`).
    /// `None` for pre-Phase-93 digests; `Some(false)`
    /// distinguishes "knob explicitly off" from "section
    /// absent" on the surface. `#[serde(default,
    /// skip_serializing_if = "Option::is_none")]` keeps the
    /// IPC wire-compat — older `aivyx learning` clients
    /// decode the digest unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judgment_signal: Option<bool>,
}

/// One Persona delta in the export. `mac` and `prev_mac` from
/// [`SignedPersonaEntry`] are deliberately omitted — they're
/// host-specific.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaExport {
    pub seq: u64,
    pub delta: PersonaDelta,
}

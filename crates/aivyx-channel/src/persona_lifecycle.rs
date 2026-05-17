//! Phase 81 — the structural Persona-lifecycle detector.
//!
//! For 80 phases the Persona ("Soul") only ever grew. This is
//! the pure, deterministic core that decides what should be
//! *proposed* for **consolidation** (near-duplicate soft-list
//! facets) or **decay** (long-unreinforced soft-list facets).
//! It never mutates anything and it never touches the
//! always-on core: [`soft_facets_of`] is the structural choke
//! point — it enumerates *only* the six soft lists, so a
//! scalar identity field or a `behavioral_constraint` can
//! never become an action (the Phase 79 always-on-core
//! invariant extended to the lifecycle layer, proven by test).
//!
//! Best-effort, never a regression (the Phase 79 ethos):
//! consolidation needs embeddings; on no-embed / embed failure
//! it is skipped and decay — which needs no model — still
//! runs. Disabled / empty / below the soft-list floor → no
//! actions. The pass (Task 4) turns each action into a normal
//! Pending `PersonaProposal`; the operator approves/rejects and
//! every action is reversible (`Revert`).

use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use aivyx_config::PersonaLifecycleConfig;
use aivyx_llm::embedding::EmbeddingProvider;

use crate::persona::{
    EffectivePersona, PersonaDeltaCategory, PersonaDeltaOp,
    ProposedPersonaDelta,
};

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

/// Extract every soft-list facet from an effective Persona,
/// tagged with its category, in a stable order.
///
/// **This is the core-protection choke point.** It reads only
/// the six soft lists and never the scalars or
/// `behavioral_constraints`, so nothing downstream can ever
/// target the always-on core — the lifecycle layer's half of
/// the Phase 79 invariant, enforced structurally rather than by
/// a runtime check (proven by `core_protection_*` test).
pub fn soft_facets_of(
    p: &EffectivePersona,
) -> Vec<(SoftCategory, String)> {
    let mut out = Vec::new();
    for (cat, list) in [
        (SoftCategory::PrimaryUseCases, &p.primary_use_cases),
        (
            SoftCategory::BehavioralPreferences,
            &p.behavioral_preferences,
        ),
        (SoftCategory::LearnedContext, &p.learned_context),
        (
            SoftCategory::CommunicationAdaptations,
            &p.communication_adaptations,
        ),
        (SoftCategory::CharacterTraits, &p.character_traits),
        (
            SoftCategory::RelationshipMilestones,
            &p.relationship_milestones,
        ),
    ] {
        for v in list {
            out.push((cat, v.clone()));
        }
    }
    out
}

/// One soft-list facet plus the provenance the detector needs.
/// The pass (Task 4) builds these from the persona log:
/// `origin_ts_secs` is when the facet's originating delta was
/// applied; `reinforced` is true iff a *later* delta touched
/// the same category (active curation → do not decay it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleFacet {
    pub category: SoftCategory,
    pub value: String,
    pub origin_ts_secs: u64,
    pub reinforced: bool,
}

/// What a lifecycle action proposes. Both are expressible
/// entirely in the existing `RemoveList`/`AppendList` ops, so
/// no schema migration and full `Revert`-ability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleActionKind {
    /// Merge `originals` (>= 2 near-duplicates) into `merged`.
    Consolidate {
        originals: Vec<String>,
        merged: String,
    },
    /// Retire one long-unreinforced facet.
    Decay { value: String },
}

/// A single proposed lifecycle action with its provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaLifecycleAction {
    pub kind: LifecycleActionKind,
    pub category: SoftCategory,
    /// Concrete, human-readable provenance for the proposal +
    /// the Phase 78 surface.
    pub reason: String,
}

impl PersonaLifecycleAction {
    /// Deterministic cross-cycle dedup id. Task 4 suppresses an
    /// action that already has a matching pending/approved
    /// proposal so the assistant never re-files the same
    /// identity change every reflection cycle. Stable across
    /// cycles for the same underlying facts.
    pub fn dedup_id(&self) -> String {
        match &self.kind {
            LifecycleActionKind::Consolidate {
                originals, ..
            } => {
                let mut o = originals.clone();
                o.sort();
                format!(
                    "pl:consolidate:{}:{}",
                    self.category.label(),
                    o.join("\u{1f}"),
                )
            }
            LifecycleActionKind::Decay { value } => format!(
                "pl:decay:{}:{}",
                self.category.label(),
                value,
            ),
        }
    }

    /// Compact class label for breadcrumbs / the Phase 78
    /// surface.
    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            LifecycleActionKind::Consolidate { .. } => {
                "consolidate"
            }
            LifecycleActionKind::Decay { .. } => "decay",
        }
    }

    /// Decompose into the operator-gated proposals that
    /// implement it. Each is **one** `(category, op)` — the
    /// existing one-op-per-proposal model — keyed by a
    /// deterministic, cross-cycle-stable proposal id so the
    /// pass never re-files the same identity change.
    ///
    /// `Decay` → a single `RemoveList`. `Consolidate` → a
    /// `RemoveList` for every near-duplicate **except**
    /// `merged` (which is the longest *existing* member, so it
    /// is already in the list — no `AppendList` is needed and
    /// none is emitted; removing a redundant near-duplicate
    /// while the canonical one stays is independently safe and
    /// `Revert`-able per proposal).
    pub fn to_proposals(
        &self,
    ) -> Vec<(String, ProposedPersonaDelta)> {
        let cat = self.category.to_delta_category();
        match &self.kind {
            LifecycleActionKind::Decay { value } => vec![(
                format!(
                    "pl:decay:{}:{}",
                    self.category.label(),
                    value,
                ),
                ProposedPersonaDelta {
                    category: cat,
                    op: PersonaDeltaOp::RemoveList {
                        value: value.clone(),
                    },
                    reason: Some(self.reason.clone()),
                },
            )],
            LifecycleActionKind::Consolidate {
                originals,
                merged,
            } => originals
                .iter()
                .filter(|o| o.as_str() != merged.as_str())
                .map(|o| {
                    (
                        format!(
                            "pl:consolidate:{}:keep={}:rm={}",
                            self.category.label(),
                            merged,
                            o,
                        ),
                        ProposedPersonaDelta {
                            category: cat,
                            op: PersonaDeltaOp::RemoveList {
                                value: o.clone(),
                            },
                            reason: Some(format!(
                                "{} (redundant with kept \
                                 facet {merged:?})",
                                self.reason,
                            )),
                        },
                    )
                })
                .collect(),
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

/// Shared handle the pass writes and the
/// `GetLearningInsights` handler reads. `None` inside = the
/// lifecycle pass has not run this daemon lifetime.
pub type SharedPersonaLifecycleStat =
    Arc<RwLock<Option<PersonaLifecycleStat>>>;

/// Construct an empty shared lifecycle-stat handle.
pub fn shared_persona_lifecycle_stat() -> SharedPersonaLifecycleStat
{
    Arc::new(RwLock::new(None))
}

/// Cosine of two equal-length vectors. Hand-rolled, zero-dep
/// (the Phase 75 "no linalg crate" ethos, same as
/// `persona_context`). Degenerate cases → 0.0 so they rank as
/// "not similar" rather than erroring.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Deterministic, no-LLM merge: keep the longest member (the
/// one most likely to subsume the shorter near-duplicate);
/// ties broken lexicographically smallest. The operator
/// reviews `originals` vs this `merged` and can edit before
/// approving (the proposal path separates proposed vs applied
/// op), and `Revert` undoes an approved merge — so a
/// deterministic pick is safe and never silently loses nuance.
fn deterministic_merge(originals: &[String]) -> String {
    originals
        .iter()
        .max_by(|a, b| {
            a.chars()
                .count()
                .cmp(&b.chars().count())
                .then_with(|| b.cmp(a))
        })
        .cloned()
        .unwrap_or_default()
}

/// The structural detector. The only effect is the embedding
/// call (skipped entirely unless consolidation is on);
/// deterministic given the provider. Returns id-sorted actions.
pub struct PersonaLifecycleDetector {
    config: PersonaLifecycleConfig,
    provider: Arc<dyn EmbeddingProvider>,
}

impl PersonaLifecycleDetector {
    pub fn new(
        config: PersonaLifecycleConfig,
        provider: Arc<dyn EmbeddingProvider>,
    ) -> Self {
        Self { config, provider }
    }

    /// Detect consolidation + decay actions over the soft-list
    /// facets. `now_secs` is used only for decay-age math.
    pub async fn detect(
        &self,
        facets: &[LifecycleFacet],
        now_secs: u64,
    ) -> Vec<PersonaLifecycleAction> {
        let mut actions: Vec<PersonaLifecycleAction> = Vec::new();
        let floor = self.config.min_soft_facets as usize;

        for cat in SoftCategory::ALL {
            let group: Vec<&LifecycleFacet> = facets
                .iter()
                .filter(|f| f.category == cat)
                .collect();
            // Never act on a soft list below the floor — a
            // young Soul has nothing worth tidying.
            if group.len() < floor {
                continue;
            }

            // --- Decay (no model needed) ---
            if self.config.signals.decay {
                for f in &group {
                    let age =
                        now_secs.saturating_sub(f.origin_ts_secs);
                    if age > self.config.decay_max_age_secs
                        && !f.reinforced
                    {
                        actions.push(PersonaLifecycleAction {
                            kind: LifecycleActionKind::Decay {
                                value: f.value.clone(),
                            },
                            category: cat,
                            reason: format!(
                                "unreinforced for {age}s \
                                 (> {}s) with no later {} delta",
                                self.config.decay_max_age_secs,
                                cat.label(),
                            ),
                        });
                    }
                }
            }

            // --- Consolidate (needs embeddings) ---
            if self.config.signals.consolidate && group.len() >= 2
            {
                let inputs: Vec<String> = group
                    .iter()
                    .map(|f| f.value.clone())
                    .collect();
                let vecs =
                    match self.provider.embed(&inputs).await {
                        Ok(v) if v.len() == inputs.len() => v,
                        // Embed failure → skip consolidation
                        // this run (best-effort, never a
                        // regression); decay already ran.
                        _ => continue,
                    };
                // Deterministic leader clustering: for each
                // not-yet-clustered facet in order, pull in any
                // later not-yet-clustered facet whose cosine
                // STRICTLY exceeds the threshold. Conservative
                // (no transitive chaining) and order-stable.
                let n = group.len();
                let mut used = vec![false; n];
                for i in 0..n {
                    if used[i] {
                        continue;
                    }
                    let mut members = vec![i];
                    used[i] = true;
                    for j in (i + 1)..n {
                        if used[j] {
                            continue;
                        }
                        if cosine(&vecs[i], &vecs[j])
                            > self.config.consolidation_similarity
                        {
                            members.push(j);
                            used[j] = true;
                        }
                    }
                    if members.len() >= 2 {
                        let originals: Vec<String> = members
                            .iter()
                            .map(|&m| group[m].value.clone())
                            .collect();
                        let merged =
                            deterministic_merge(&originals);
                        let reason = format!(
                            "{} near-duplicate {} facets \
                             (cosine > {:.2})",
                            originals.len(),
                            cat.label(),
                            self.config.consolidation_similarity,
                        );
                        actions.push(PersonaLifecycleAction {
                            kind:
                                LifecycleActionKind::Consolidate {
                                    originals,
                                    merged,
                                },
                            category: cat,
                            reason,
                        });
                    }
                }
            }
        }

        // Deterministic, stable output order for the pass +
        // cross-cycle dedup.
        actions.sort_by_cached_key(|a| a.dedup_id());
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_config::{
        PersonaLifecycleConfig, PersonaLifecycleSignals,
    };
    use aivyx_llm::embedding::EmbeddingError;
    use async_trait::async_trait;

    /// Same-bucket facets get an identical unit vector (cosine
    /// 1.0 > any threshold → cluster); different buckets are
    /// orthogonal (cosine 0.0). Or fails on demand.
    struct FakeProvider {
        fail: bool,
    }

    #[async_trait]
    impl EmbeddingProvider for FakeProvider {
        async fn embed(
            &self,
            texts: &[String],
        ) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            if self.fail {
                return Err(EmbeddingError::Timeout);
            }
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

    fn cfg(
        consolidate: bool,
        decay: bool,
        min_soft_facets: u32,
        consolidation_similarity: f32,
        decay_max_age_secs: u64,
    ) -> PersonaLifecycleConfig {
        PersonaLifecycleConfig {
            enabled: true,
            consolidation_similarity,
            decay_max_age_secs,
            min_soft_facets,
            signals: PersonaLifecycleSignals {
                consolidate,
                decay,
            },
        }
    }

    fn facet(
        cat: SoftCategory,
        v: &str,
        origin_ts_secs: u64,
        reinforced: bool,
    ) -> LifecycleFacet {
        LifecycleFacet {
            category: cat,
            value: v.to_string(),
            origin_ts_secs,
            reinforced,
        }
    }

    fn det(
        c: PersonaLifecycleConfig,
        fail: bool,
    ) -> PersonaLifecycleDetector {
        PersonaLifecycleDetector::new(
            c,
            Arc::new(FakeProvider { fail }),
        )
    }

    #[tokio::test]
    async fn empty_input_yields_no_actions() {
        let out = det(cfg(true, true, 2, 0.92, 1000), false)
            .detect(&[], 9999)
            .await;
        assert!(out.is_empty());
    }

    /// The structural core-protection invariant: `soft_facets_of`
    /// never surfaces a scalar identity field or a
    /// `behavioral_constraint`, so the always-on core can never
    /// become a lifecycle action (Phase 79 invariant extended).
    #[test]
    fn core_protection_soft_facets_of_excludes_core() {
        let p = EffectivePersona {
            assistant_name: Some("Ada".into()),
            operator_profile: Some("SRE".into()),
            communication_style: Some("terse".into()),
            behavioral_constraints: vec![
                "never deploy without approval".into(),
            ],
            learned_context: vec!["soft one".into()],
            character_traits: vec!["curious".into()],
            ..EffectivePersona::default()
        };
        let facets = soft_facets_of(&p);
        // Only the two soft-list values, nothing else.
        assert_eq!(facets.len(), 2);
        let vals: Vec<&str> =
            facets.iter().map(|(_, v)| v.as_str()).collect();
        assert!(vals.contains(&"soft one"));
        assert!(vals.contains(&"curious"));
        // The scalars and the constraint are structurally
        // absent — they can never become an action.
        assert!(!vals.contains(&"Ada"));
        assert!(!vals.contains(&"SRE"));
        assert!(!vals.contains(&"terse"));
        assert!(
            !vals.contains(&"never deploy without approval")
        );
    }

    #[tokio::test]
    async fn consolidate_merges_near_duplicates() {
        // 3 facets in one category; two share the "dup" bucket.
        let f = vec![
            facet(
                SoftCategory::LearnedContext,
                "dup short",
                0,
                true,
            ),
            facet(
                SoftCategory::LearnedContext,
                "dup the longer variant",
                0,
                true,
            ),
            facet(
                SoftCategory::LearnedContext,
                "other thing",
                0,
                true,
            ),
        ];
        let out = det(cfg(true, false, 2, 0.92, 1000), false)
            .detect(&f, 100)
            .await;
        assert_eq!(out.len(), 1);
        match &out[0].kind {
            LifecycleActionKind::Consolidate {
                originals,
                merged,
            } => {
                assert_eq!(originals.len(), 2);
                assert!(originals
                    .contains(&"dup short".to_string()));
                assert!(originals.contains(
                    &"dup the longer variant".to_string()
                ));
                // Deterministic merge → the longest member.
                assert_eq!(merged, "dup the longer variant");
            }
            other => panic!("expected Consolidate, got {other:?}"),
        }
        assert_eq!(out[0].category, SoftCategory::LearnedContext);
    }

    #[tokio::test]
    async fn decay_retires_old_unreinforced_only() {
        let f = vec![
            // Old + not reinforced → decays.
            facet(
                SoftCategory::CharacterTraits,
                "other stale",
                0,
                false,
            ),
            // Old but reinforced → kept.
            facet(
                SoftCategory::CharacterTraits,
                "other reinforced",
                0,
                true,
            ),
            // Fresh → kept.
            facet(
                SoftCategory::CharacterTraits,
                "other fresh",
                4_900,
                false,
            ),
        ];
        let out = det(cfg(false, true, 2, 0.92, 1000), false)
            .detect(&f, 5_000)
            .await;
        assert_eq!(out.len(), 1);
        match &out[0].kind {
            LifecycleActionKind::Decay { value } => {
                assert_eq!(value, "other stale");
            }
            other => panic!("expected Decay, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn below_min_soft_facets_floor_is_skipped() {
        // A dup pair AND an old unreinforced facet, but only 2
        // facets while the floor is 3 → nothing proposed.
        let f = vec![
            facet(SoftCategory::LearnedContext, "dup a", 0, false),
            facet(SoftCategory::LearnedContext, "dup b", 0, false),
        ];
        let out = det(cfg(true, true, 3, 0.92, 1000), false)
            .detect(&f, 9_999)
            .await;
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn embed_failure_skips_consolidation_keeps_decay() {
        let f = vec![
            facet(SoftCategory::LearnedContext, "dup a", 0, false),
            facet(SoftCategory::LearnedContext, "dup b", 0, false),
        ];
        // Failing provider: no Consolidate, but the two old
        // unreinforced facets still Decay.
        let out = det(cfg(true, true, 2, 0.92, 1000), true)
            .detect(&f, 9_999)
            .await;
        assert!(out.iter().all(|a| matches!(
            a.kind,
            LifecycleActionKind::Decay { .. }
        )));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn consolidate_to_proposals_removes_only_non_canonical() {
        let a = PersonaLifecycleAction {
            kind: LifecycleActionKind::Consolidate {
                originals: vec![
                    "short".into(),
                    "the long canonical".into(),
                    "mid one".into(),
                ],
                merged: "the long canonical".into(),
            },
            category: SoftCategory::LearnedContext,
            reason: "3 near-duplicate learned_context facets"
                .into(),
        };
        let props = a.to_proposals();
        // One RemoveList per non-canonical original; the kept
        // (merged) facet is never appended or removed.
        assert_eq!(props.len(), 2);
        for (id, op) in &props {
            assert!(id.starts_with(
                "pl:consolidate:learned_context:keep="
            ));
            match &op.op {
                PersonaDeltaOp::RemoveList { value } => {
                    assert_ne!(value, "the long canonical");
                }
                other => {
                    panic!("expected RemoveList, got {other:?}")
                }
            }
            assert_eq!(
                op.category,
                PersonaDeltaCategory::LearnedContext
            );
        }
    }

    #[test]
    fn decay_to_proposals_is_one_removelist() {
        let a = PersonaLifecycleAction {
            kind: LifecycleActionKind::Decay {
                value: "stale fact".into(),
            },
            category: SoftCategory::CharacterTraits,
            reason: "unreinforced".into(),
        };
        let props = a.to_proposals();
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].0, "pl:decay:character_traits:stale fact");
        match &props[0].1.op {
            PersonaDeltaOp::RemoveList { value } => {
                assert_eq!(value, "stale fact");
            }
            other => panic!("expected RemoveList, got {other:?}"),
        }
        assert_eq!(
            props[0].1.category,
            PersonaDeltaCategory::CharacterTraits
        );
    }

    #[tokio::test]
    async fn signal_toggles_disable_each_class() {
        let f = vec![
            facet(SoftCategory::LearnedContext, "dup a", 0, false),
            facet(SoftCategory::LearnedContext, "dup b", 0, false),
        ];
        // consolidate off, decay off → nothing.
        let out = det(cfg(false, false, 2, 0.92, 1000), false)
            .detect(&f, 9_999)
            .await;
        assert!(out.is_empty());
    }
}

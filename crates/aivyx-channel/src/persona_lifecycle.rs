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

// moved to the wasm-clean aivyx-ipc crate (Chapter M.2d-2); re-exported here.
pub use aivyx_ipc::insights::{PersonaLifecycleProposed, PersonaLifecycleStat, SoftCategory};

use serde::{Deserialize, Serialize};

use aivyx_config::PersonaLifecycleConfig;
use aivyx_llm::embedding::EmbeddingProvider;

use crate::persona::{
    EffectivePersona, PersonaDeltaOp,
    ProposedPersonaDelta,
};


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
/// Phase 85 — a facet's associated topic helpfulness, resolved
/// by the pass from the Phase 82 ledger *before* the (pure)
/// detector runs. `score` is the decayed EWMA; `samples` the
/// ledger confidence count. `None` on a `LifecycleFacet` means
/// "no signal" (no provenance, no ledger, or unseen topic) →
/// the detector falls back to exact Phase 81 age-only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HelpfulnessHint {
    pub score: f32,
    pub samples: u32,
}

/// Phase 88 — the resolved durable Phase 83 co-occurrence
/// affinity for a facet's underlying pair. Shape mirrors
/// `HelpfulnessHint`. `None` on a `LifecycleFacet` means "no
/// signal" (no `consolidate-pair:` provenance, no ledger, or
/// the pair has been pruned) → the detector's pair arm sits
/// out and the facet follows the Phase 81 age rule (or any
/// `helpfulness` signal it carries).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairAffinityHint {
    pub affinity: f32,
    pub samples: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LifecycleFacet {
    pub category: SoftCategory,
    pub value: String,
    pub origin_ts_secs: u64,
    pub reinforced: bool,
    /// Phase 85 — the recall topic recovered from
    /// `recall-fb:{topic}` provenance, or `None` for
    /// reflection-authored facets (no topic linkage).
    pub recall_topic: Option<String>,
    /// Phase 85 — the resolved durable helpfulness for
    /// `recall_topic` (decayed score + sample count), or
    /// `None` (no signal → age-only).
    pub helpfulness: Option<HelpfulnessHint>,
    /// Phase 88 — the topic pair recovered from
    /// `consolidate-pair:{lo}+{hi}` provenance, canonical
    /// alphabetic order. `None` for any facet that didn't
    /// come from the Phase 87 consolidation actuator.
    pub pair: Option<(String, String)>,
    /// Phase 88 — the resolved durable co-occurrence affinity
    /// for `pair` (decayed score + sample count), or `None`
    /// (no signal → the pair arm sits out).
    pub pair_affinity: Option<PairAffinityHint>,
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
                    supersedes_proposal_id: None,
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
                            supersedes_proposal_id: None,
                        },
                    )
                })
                .collect(),
        }
    }
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

            // --- Decay (no model needed) — Phase 85
            // symmetric helpfulness gate over the Phase 81
            // age rule. `decay_unhelpful_threshold` is
            // negative; its magnitude is the symmetric
            // positive bar. No hint (no provenance / no
            // ledger / unseen topic) → both flags false →
            // exact Phase 81 age-only behaviour.
            if self.config.signals.decay {
                let neg = self.config.decay_unhelpful_threshold;
                let pos = -neg;
                let min_s = self.config.decay_min_samples;
                let pair_floor =
                    self.config.decay_pair_below_affinity;
                for f in &group {
                    let age =
                        now_secs.saturating_sub(f.origin_ts_secs);
                    let age_eligible = age
                        > self.config.decay_max_age_secs
                        && !f.reinforced;
                    let sustained_negative =
                        f.helpfulness.is_some_and(|h| {
                            h.samples >= min_s && h.score <= neg
                        });
                    let sustained_positive =
                        f.helpfulness.is_some_and(|h| {
                            h.samples >= min_s && h.score >= pos
                        });
                    // Phase 88 — symmetric pair-affinity gate.
                    // A `consolidate-pair:` facet whose pair
                    // has fallen below the floor has lost its
                    // justification (the relationship is no
                    // longer durable); a pair still at/above
                    // the floor *protects* the facet from
                    // age-decay (the relationship still holds,
                    // so the identity still applies).
                    let sustained_pair_decay =
                        f.pair_affinity.is_some_and(|p| {
                            p.affinity < pair_floor
                        });
                    let sustained_pair_strong =
                        f.pair_affinity.is_some_and(|p| {
                            p.affinity >= pair_floor
                        });
                    // Trigger: any sustained-negative signal
                    // (helpfulness OR pair-affinity) fires
                    // decay before the age horizon. Protect:
                    // any sustained-positive signal blocks
                    // age-decay. An unsignalled facet (no
                    // `helpfulness` and no `pair_affinity`)
                    // follows the pure Phase 81 age rule.
                    let decay = sustained_negative
                        || sustained_pair_decay
                        || (age_eligible
                            && !sustained_positive
                            && !sustained_pair_strong);
                    if !decay {
                        continue;
                    }
                    let reason = if sustained_pair_decay {
                        let p = f.pair_affinity.unwrap();
                        let (lo, hi) = f
                            .pair
                            .as_ref()
                            .map(|(a, b)| {
                                (a.as_str(), b.as_str())
                            })
                            .unwrap_or(("?", "?"));
                        if age_eligible {
                            format!(
                                "unreinforced for {age}s \
                                 (> {}s) AND co-occurrence \
                                 pair `{lo}` + `{hi}` decayed \
                                 affinity {:.2} (below floor \
                                 {pair_floor:.2}); \
                                 relationship no longer \
                                 durable",
                                self.config.decay_max_age_secs,
                                p.affinity,
                            )
                        } else {
                            format!(
                                "co-occurrence pair `{lo}` + \
                                 `{hi}` decayed affinity \
                                 {:.2} (below floor \
                                 {pair_floor:.2}); \
                                 relationship no longer \
                                 durable — decayed before the \
                                 age horizon",
                                p.affinity,
                            )
                        }
                    } else if sustained_negative {
                        let h = f.helpfulness.unwrap();
                        let topic = f
                            .recall_topic
                            .as_deref()
                            .unwrap_or("?");
                        if age_eligible {
                            format!(
                                "unreinforced for {age}s \
                                 (> {}s) AND topic {topic:?} \
                                 net {:.1} over {} windows \
                                 (sustained low helpfulness)",
                                self.config.decay_max_age_secs,
                                h.score,
                                h.samples,
                            )
                        } else {
                            format!(
                                "topic {topic:?} net {:.1} \
                                 over {} windows (sustained \
                                 low helpfulness — decayed \
                                 before the age horizon)",
                                h.score, h.samples,
                            )
                        }
                    } else {
                        format!(
                            "unreinforced for {age}s (> {}s) \
                             with no later {} delta",
                            self.config.decay_max_age_secs,
                            cat.label(),
                        )
                    };
                    actions.push(PersonaLifecycleAction {
                        kind: LifecycleActionKind::Decay {
                            value: f.value.clone(),
                        },
                        category: cat,
                        reason,
                    });
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
    use crate::persona::PersonaDeltaCategory;
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
            decay_unhelpful_threshold: -2.0,
            decay_min_samples: 3,
            decay_pair_below_affinity: 1.0,
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
            recall_topic: None,
            helpfulness: None,
            pair: None,
            pair_affinity: None,
        }
    }

    /// A facet with `recall-fb` provenance + a resolved
    /// helpfulness hint (the Phase 85 path).
    fn facet_h(
        cat: SoftCategory,
        v: &str,
        origin_ts_secs: u64,
        reinforced: bool,
        topic: &str,
        score: f32,
        samples: u32,
    ) -> LifecycleFacet {
        LifecycleFacet {
            category: cat,
            value: v.to_string(),
            origin_ts_secs,
            reinforced,
            recall_topic: Some(topic.to_string()),
            helpfulness: Some(HelpfulnessHint {
                score,
                samples,
            }),
            pair: None,
            pair_affinity: None,
        }
    }

    /// A facet with `consolidate-pair:` provenance + a resolved
    /// pair-affinity hint (the Phase 88 path). Mirrors
    /// `facet_h` for the helpfulness arm — the lint is a
    /// test-helper accumulator (8 fields the detector reads),
    /// same posture as `facet_h`'s shape.
    #[allow(clippy::too_many_arguments)]
    fn facet_p(
        cat: SoftCategory,
        v: &str,
        origin_ts_secs: u64,
        reinforced: bool,
        a: &str,
        b: &str,
        affinity: f32,
        samples: u32,
    ) -> LifecycleFacet {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        LifecycleFacet {
            category: cat,
            value: v.to_string(),
            origin_ts_secs,
            reinforced,
            recall_topic: None,
            helpfulness: None,
            pair: Some((lo.to_string(), hi.to_string())),
            pair_affinity: Some(PairAffinityHint {
                affinity,
                samples,
            }),
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

    // ---- Phase 85 — symmetric helpfulness gate -----------------

    #[tokio::test]
    async fn negative_helpfulness_triggers_decay_before_age() {
        // A YOUNG facet (not age-eligible) whose topic is
        // sustained-negative (-5 <= -2, 5 >= 3 samples) →
        // decays early; a young no-hint facet does not.
        let f = vec![
            facet_h(
                SoftCategory::LearnedContext,
                "deploy-runbook note",
                9_000,
                false,
                "deploy",
                -5.0,
                5,
            ),
            facet(
                SoftCategory::LearnedContext,
                "young plain",
                9_000,
                false,
            ),
        ];
        let out = det(cfg(false, true, 2, 0.92, 1_000), false)
            .detect(&f, 10_000)
            .await;
        assert_eq!(out.len(), 1);
        match &out[0].kind {
            LifecycleActionKind::Decay { value } => {
                assert_eq!(value, "deploy-runbook note");
            }
            o => panic!("expected Decay, got {o:?}"),
        }
        assert!(out[0].reason.contains(
            "decayed before the age horizon"
        ));
        assert!(out[0].reason.contains("\"deploy\""));
    }

    #[tokio::test]
    async fn positive_helpfulness_protects_age_old_facet() {
        // Both OLD + unreinforced (Phase 81 would decay both).
        // The hinted one's topic is sustained-positive
        // (+5 >= +2) → PROTECTED; the no-hint one still
        // age-decays.
        let f = vec![
            facet_h(
                SoftCategory::CharacterTraits,
                "still-useful trait",
                0,
                false,
                "rust",
                5.0,
                5,
            ),
            facet(
                SoftCategory::CharacterTraits,
                "stale plain trait",
                0,
                false,
            ),
        ];
        let out = det(cfg(false, true, 2, 0.92, 1_000), false)
            .detect(&f, 5_000)
            .await;
        assert_eq!(out.len(), 1);
        match &out[0].kind {
            LifecycleActionKind::Decay { value } => {
                assert_eq!(value, "stale plain trait");
            }
            o => panic!("expected Decay, got {o:?}"),
        }
        // The still-helpful old facet must NOT be proposed.
        assert!(out.iter().all(|a| match &a.kind {
            LifecycleActionKind::Decay { value } =>
                value != "still-useful trait",
            _ => true,
        }));
        // The surviving decay is the plain age-only one.
        assert!(out[0]
            .reason
            .contains("with no later"));
    }

    #[tokio::test]
    async fn thin_evidence_falls_back_to_exact_age_only() {
        // Negative score but samples (1) below the floor (3):
        // the hint is ignored entirely → an OLD facet decays
        // by AGE with the Phase 81 reason (not the helpfulness
        // one); a YOUNG facet is NOT early-triggered.
        let f = vec![
            facet_h(
                SoftCategory::LearnedContext,
                "old thin",
                0,
                false,
                "deploy",
                -9.0,
                1,
            ),
            facet_h(
                SoftCategory::LearnedContext,
                "young thin",
                9_000,
                false,
                "deploy",
                -9.0,
                1,
            ),
        ];
        let out = det(cfg(false, true, 2, 0.92, 1_000), false)
            .detect(&f, 10_000)
            .await;
        assert_eq!(out.len(), 1);
        match &out[0].kind {
            LifecycleActionKind::Decay { value } => {
                assert_eq!(value, "old thin");
            }
            o => panic!("expected Decay, got {o:?}"),
        }
        // Exact Phase 81 reason — thin evidence never cites
        // helpfulness.
        assert!(out[0].reason.contains("with no later"));
        assert!(!out[0]
            .reason
            .contains("sustained low helpfulness"));
    }

    // ---- Phase 88 — pattern-driven decay -----------------------

    /// A `consolidate-pair:` facet whose pair has fallen
    /// **below** `decay_pair_below_affinity` is decayed even
    /// before the age horizon — the relationship that
    /// justified the identity no longer holds.
    #[tokio::test]
    async fn pair_affinity_below_floor_triggers_decay_early() {
        let f = vec![
            // Pad to the floor; these two never decay (young,
            // no pair signal).
            facet(SoftCategory::LearnedContext, "filler-1", 9_000, false),
            facet(SoftCategory::LearnedContext, "filler-2", 9_000, false),
            // Young pair facet with affinity 0.3 < floor 1.0
            // → triggers decay before age.
            facet_p(
                SoftCategory::LearnedContext,
                "deploy + rollback pair note",
                9_000,
                false,
                "deploy",
                "rollback",
                0.3,
                5,
            ),
        ];
        let out = det(cfg(false, true, 2, 0.92, 1_000), false)
            .detect(&f, 10_000)
            .await;
        assert_eq!(out.len(), 1);
        match &out[0].kind {
            LifecycleActionKind::Decay { value } => {
                assert_eq!(value, "deploy + rollback pair note");
            }
            o => panic!("expected Decay, got {o:?}"),
        }
        // Reason cites the pair + the decayed affinity.
        assert!(out[0]
            .reason
            .contains("co-occurrence pair `deploy` + `rollback`"));
        assert!(out[0]
            .reason
            .contains("relationship no longer durable"));
        assert!(out[0]
            .reason
            .contains("decayed before the age horizon"));
    }

    /// A still-durable pair (affinity ≥ floor) on an OLD
    /// `consolidate-pair:` facet **protects** it from age-
    /// decay — the symmetric move to Phase 85's helpfulness
    /// protection.
    #[tokio::test]
    async fn pair_affinity_at_or_above_floor_protects_age_old_facet() {
        let f = vec![
            facet(SoftCategory::LearnedContext, "filler-1", 9_000, false),
            facet(SoftCategory::LearnedContext, "filler-2", 9_000, false),
            // Age 10000 > horizon 1000, unreinforced; would
            // age-decay normally. Pair affinity 5.0 ≥ floor
            // 1.0 → protected.
            facet_p(
                SoftCategory::LearnedContext,
                "still durable",
                0,
                false,
                "deploy",
                "rollback",
                5.0,
                10,
            ),
            // Same shape but NO pair signal → still decays by
            // age (control case: protection requires the
            // signal to be present).
            facet(
                SoftCategory::LearnedContext,
                "no pair signal",
                0,
                false,
            ),
        ];
        let out = det(cfg(false, true, 2, 0.92, 1_000), false)
            .detect(&f, 10_000)
            .await;
        // Only the no-pair-signal facet decays; the protected
        // one survives.
        assert_eq!(out.len(), 1);
        match &out[0].kind {
            LifecycleActionKind::Decay { value } => {
                assert_eq!(value, "no pair signal");
            }
            o => panic!("expected Decay, got {o:?}"),
        }
        assert!(out[0].reason.contains("with no later"));
    }

    /// Absent `pair_affinity` falls back to exact Phase
    /// 81/85 behaviour — a `consolidate-pair:` facet whose
    /// ledger entry is gone (or whose ledger is absent
    /// entirely) follows age-only.
    #[tokio::test]
    async fn absent_pair_affinity_falls_back_to_age_only() {
        let f = vec![
            facet(SoftCategory::LearnedContext, "filler-1", 9_000, false),
            facet(SoftCategory::LearnedContext, "filler-2", 9_000, false),
            // Pair set but pair_affinity = None (signal not
            // resolved). YOUNG facet → no decay (no signal
            // can early-trigger; age horizon not crossed).
            LifecycleFacet {
                category: SoftCategory::LearnedContext,
                value: "no-signal young pair".into(),
                origin_ts_secs: 9_000,
                reinforced: false,
                recall_topic: None,
                helpfulness: None,
                pair: Some(("deploy".into(), "rollback".into())),
                pair_affinity: None,
            },
            // OLD pair-without-signal facet → age-decay still
            // fires (protection requires a positive signal).
            LifecycleFacet {
                category: SoftCategory::LearnedContext,
                value: "no-signal old pair".into(),
                origin_ts_secs: 0,
                reinforced: false,
                recall_topic: None,
                helpfulness: None,
                pair: Some(("deploy".into(), "rollback".into())),
                pair_affinity: None,
            },
        ];
        let out = det(cfg(false, true, 2, 0.92, 1_000), false)
            .detect(&f, 10_000)
            .await;
        assert_eq!(out.len(), 1);
        match &out[0].kind {
            LifecycleActionKind::Decay { value } => {
                assert_eq!(value, "no-signal old pair");
            }
            o => panic!("expected Decay, got {o:?}"),
        }
        // Age-only reason — no pair signal to cite.
        assert!(out[0].reason.contains("with no later"));
        assert!(!out[0].reason.contains("co-occurrence pair"));
    }

    /// The pair arm and the helpfulness arm are independent.
    /// A single facet only ever carries one provenance, but
    /// the detector must still handle a hypothetical with
    /// both signals: either sustained-negative fires decay;
    /// either sustained-positive blocks age-decay.
    #[tokio::test]
    async fn pair_and_helpfulness_signals_combine_orwise() {
        let f = vec![
            facet(SoftCategory::LearnedContext, "filler-1", 9_000, false),
            facet(SoftCategory::LearnedContext, "filler-2", 9_000, false),
            // Both signals positive on an OLD facet →
            // protected (either-strong-protects).
            LifecycleFacet {
                category: SoftCategory::LearnedContext,
                value: "both strong old".into(),
                origin_ts_secs: 0,
                reinforced: false,
                recall_topic: Some("deploy".into()),
                helpfulness: Some(HelpfulnessHint {
                    score: 5.0,
                    samples: 10,
                }),
                pair: Some(("deploy".into(), "rollback".into())),
                pair_affinity: Some(PairAffinityHint {
                    affinity: 5.0,
                    samples: 10,
                }),
            },
            // Helpfulness positive (protect-eligible) but pair
            // sub-floor (decay-eligible) on a YOUNG facet →
            // decay still fires (sustained-negative wins over
            // sustained-positive — the OR-trigger).
            LifecycleFacet {
                category: SoftCategory::LearnedContext,
                value: "mixed young".into(),
                origin_ts_secs: 9_000,
                reinforced: false,
                recall_topic: Some("deploy".into()),
                helpfulness: Some(HelpfulnessHint {
                    score: 5.0,
                    samples: 10,
                }),
                pair: Some(("deploy".into(), "rollback".into())),
                pair_affinity: Some(PairAffinityHint {
                    affinity: 0.2,
                    samples: 10,
                }),
            },
        ];
        let out = det(cfg(false, true, 2, 0.92, 1_000), false)
            .detect(&f, 10_000)
            .await;
        assert_eq!(out.len(), 1);
        match &out[0].kind {
            LifecycleActionKind::Decay { value } => {
                assert_eq!(value, "mixed young");
            }
            o => panic!("expected Decay, got {o:?}"),
        }
        assert!(out[0]
            .reason
            .contains("relationship no longer durable"));
    }
}

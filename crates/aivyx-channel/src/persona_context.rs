//! Phase 79 — adaptive Persona: the contextual refiner.
//!
//! `PersonaContextRefiner` is the concrete
//! [`aivyx_core::llm_planner::SystemPromptRefiner`]: each turn
//! it selects the Persona facets semantically relevant to the
//! user's message and re-assembles the system prompt with only
//! those, instead of dumping the whole accreted Soul every
//! time. The Phase 79 Q2a invariant (core identity +
//! `behavioral_constraints` always injected in full) is
//! enforced structurally by `profile_prompt::reduce_persona`,
//! not here.
//!
//! Everything is best-effort and never a regression: below the
//! size threshold, or with no embedding / an embed failure,
//! `refine` returns `None` and the planner keeps its
//! byte-identical full-Persona base prompt (Q3a).

use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use aivyx_core::llm_planner::SystemPromptRefiner;
use aivyx_llm::embedding::EmbeddingProvider;

use crate::conversation_window::{
    assemble_for, SharedConversationWindows,
};
use crate::persona::SharedEffectivePersona;
use crate::profile_prompt::{
    assemble_session_prompt_selected, reducible_facet_count,
};

/// Below this many reducible facets the Soul is not big enough
/// to be worth bounding — inject it whole (Q3a). The feature is
/// invisible until it adds value.
pub const DEFAULT_SIZE_THRESHOLD: usize = 12;
/// Max soft facets injected per turn once selection engages.
pub const DEFAULT_TOP_K: usize = 12;
/// Cosine floor: a facet below this is not "relevant to this
/// turn" and is dropped even if `top_k` is unfilled (same
/// rationale as the Phase 76 recall floor).
pub const DEFAULT_MIN_SIMILARITY: f32 = 0.20;

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

/// Shared handle the refiner writes and the
/// `GetLearningInsights` handler reads. `None` inside = no
/// adaptive selection has run yet this daemon lifetime.
pub type SharedPersonaSelectionStat =
    Arc<RwLock<Option<PersonaSelectionStat>>>;

/// Construct an empty shared selection-stat handle.
pub fn shared_persona_selection_stat() -> SharedPersonaSelectionStat {
    Arc::new(RwLock::new(None))
}

/// Cosine of two equal-length vectors. Hand-rolled, zero-dep
/// (the Phase 75 "no linalg crate" ethos). Returns 0.0 for the
/// degenerate cases so they rank as "not relevant" rather than
/// erroring.
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

pub struct PersonaContextRefiner {
    profile: aivyx_config::Profile,
    persona: SharedEffectivePersona,
    role_name: String,
    role_prompt: String,
    provider: Arc<dyn EmbeddingProvider>,
    size_threshold: usize,
    top_k: usize,
    min_similarity: f32,
    /// Phase 79 (Q4a) — optional last-selection sink for the
    /// Phase 78 surface. `None` → breadcrumb-only.
    stat: Option<SharedPersonaSelectionStat>,
    /// Phase 86 — optional per-session recent-turns buffer. When
    /// `Some` and `recall_window_turns > 1`, the embedded query
    /// is the assembled conversation window instead of the bare
    /// user message; otherwise byte-identical pre-Phase-86 path.
    conversation_windows: Option<SharedConversationWindows>,
    /// Phase 86 — operator-tunable window depth (turns of prior
    /// context to concatenate before `current`). `1` (the
    /// default) disables the window — byte-identical fallback.
    recall_window_turns: usize,
}

impl PersonaContextRefiner {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        profile: aivyx_config::Profile,
        persona: SharedEffectivePersona,
        role_name: String,
        role_prompt: String,
        provider: Arc<dyn EmbeddingProvider>,
        size_threshold: usize,
        top_k: usize,
        min_similarity: f32,
    ) -> Self {
        Self {
            profile,
            persona,
            role_name,
            role_prompt,
            provider,
            size_threshold,
            top_k,
            min_similarity,
            stat: None,
            conversation_windows: None,
            recall_window_turns: 1,
        }
    }

    /// Phase 79 (Q4a) — attach the shared last-selection stat
    /// so the Phase 78 learning surface can show what the
    /// adaptive Soul did. Builder; the binary calls this with
    /// the same handle it passes into `DaemonConfig`.
    pub fn with_stat(
        mut self,
        stat: SharedPersonaSelectionStat,
    ) -> Self {
        self.stat = Some(stat);
        self
    }

    /// Phase 86 — attach the shared per-session conversation
    /// windows + the operator-set window depth. Builder; the
    /// binary calls this with the daemon-startup handle. When
    /// `recall_window_turns <= 1` the refiner is byte-identical
    /// to pre-Phase-86 even if a handle is attached.
    pub fn with_conversation_windows(
        mut self,
        windows: SharedConversationWindows,
        recall_window_turns: usize,
    ) -> Self {
        self.conversation_windows = Some(windows);
        self.recall_window_turns = recall_window_turns;
        self
    }

    /// Production constructor — module-default thresholds.
    pub fn with_defaults(
        profile: aivyx_config::Profile,
        persona: SharedEffectivePersona,
        role_name: String,
        role_prompt: String,
        provider: Arc<dyn EmbeddingProvider>,
    ) -> Self {
        Self::new(
            profile,
            persona,
            role_name,
            role_prompt,
            provider,
            DEFAULT_SIZE_THRESHOLD,
            DEFAULT_TOP_K,
            DEFAULT_MIN_SIMILARITY,
        )
    }
}

#[async_trait]
impl SystemPromptRefiner for PersonaContextRefiner {
    async fn refine(
        &self,
        user_message: &str,
        session_id: aivyx_core::SessionId,
    ) -> Option<String> {
        // Snapshot under the read lock, then drop it before any
        // await (never hold a std RwLock across .await).
        let snapshot = {
            let guard = self.persona.read().ok()?;
            guard.clone()
        };

        // Q3a — small Soul: nothing worth selecting over, inject
        // it whole (planner keeps its base prompt → None).
        if reducible_facet_count(&snapshot) < self.size_threshold {
            return None;
        }

        // Unique reducible facet strings, order-preserving.
        let mut facets: Vec<String> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        for list in [
            &snapshot.primary_use_cases,
            &snapshot.behavioral_preferences,
            &snapshot.learned_context,
            &snapshot.communication_adaptations,
            &snapshot.character_traits,
            &snapshot.relationship_milestones,
        ] {
            for s in list {
                if seen.insert(s.as_str()) {
                    facets.push(s.clone());
                }
            }
        }
        if facets.is_empty() {
            return None;
        }

        // Phase 86 — relevance query is the assembled
        // conversation window when opt-in is engaged; otherwise
        // the bare user message (byte-identical to pre-Phase-86).
        let query_text = assemble_for(
            self.conversation_windows.as_ref(),
            session_id,
            self.recall_window_turns,
            user_message,
        )
        .unwrap_or_else(|| user_message.to_string());

        // One embed call: query first, then every facet.
        let mut inputs = Vec::with_capacity(facets.len() + 1);
        inputs.push(query_text);
        inputs.extend(facets.iter().cloned());
        let vecs = match self.provider.embed(&inputs).await {
            Ok(v) if v.len() == inputs.len() => v,
            // Embed failure / short response → byte-identical
            // fallback (base prompt unchanged).
            _ => return None,
        };

        let qvec = &vecs[0];
        let mut scored: Vec<(usize, f32)> = facets
            .iter()
            .enumerate()
            .map(|(i, _)| (i, cosine(qvec, &vecs[i + 1])))
            .filter(|(_, s)| *s >= self.min_similarity)
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(self.top_k);

        let kept: HashSet<String> = scored
            .iter()
            .map(|(i, _)| facets[*i].clone())
            .collect();

        // Phase 79 (Q4a) — per-turn breadcrumb, same operator-
        // visible convention as recall / GC / backfill. The
        // structured Phase 78-surface extension is Task 6.
        eprintln!(
            "aivyx persona: injected {}/{} facets",
            kept.len(),
            facets.len()
        );
        if let Some(stat) = &self.stat {
            if let Ok(mut w) = stat.write() {
                *w = Some(PersonaSelectionStat {
                    ts_secs: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                    selected: kept.len(),
                    total: facets.len(),
                });
            }
        }

        let keep = |s: &str| kept.contains(s);
        Some(assemble_session_prompt_selected(
            &self.profile,
            &snapshot,
            &keep,
            &self.role_name,
            &self.role_prompt,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::{shared_effective_persona, EffectivePersona};
    use aivyx_llm::embedding::EmbeddingError;

    /// Maps a text to [1,0] if it contains "deploy", else
    /// [0,1]; or fails on demand. Deterministic so ranking is
    /// predictable.
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
            // Three orthogonal buckets so an "unrelated" query
            // is genuinely orthogonal to every facet (a 2-bucket
            // fake makes the non-deploy query maximally similar
            // to the misc facets, which can't model "relevant to
            // nothing").
            Ok(texts
                .iter()
                .map(|t| {
                    let l = t.to_lowercase();
                    if l.contains("deploy") {
                        vec![1.0, 0.0, 0.0]
                    } else if l.contains("misc") {
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

    fn profile() -> aivyx_config::Profile {
        aivyx_config::Profile::default()
    }

    fn big_persona() -> EffectivePersona {
        // 14 reducible facets (> default threshold 12); a
        // protected constraint + scalar identity.
        let mut lc: Vec<String> = (0..12)
            .map(|i| format!("misc note {i}"))
            .collect();
        lc.push("deploy runbook lives in wiki".to_string());
        lc.push("deploy window is Friday".to_string());
        EffectivePersona {
            assistant_name: Some("Ada".into()),
            operator_profile: Some("SRE".into()),
            communication_style: Some("terse".into()),
            primary_use_cases: vec![],
            behavioral_preferences: vec![],
            behavioral_constraints: vec![
                "never deploy without approval".into(),
            ],
            learned_context: lc,
            communication_adaptations: vec![],
            character_traits: vec![],
            relationship_milestones: vec![],
        }
    }

    fn refiner(
        persona: EffectivePersona,
        fail: bool,
        size_threshold: usize,
    ) -> PersonaContextRefiner {
        PersonaContextRefiner::new(
            profile(),
            shared_effective_persona(persona),
            "default".into(),
            "ROLE PROMPT".into(),
            Arc::new(FakeProvider { fail }),
            size_threshold,
            12,
            0.20,
        )
    }

    #[tokio::test]
    async fn small_persona_falls_back_to_none() {
        // 2 facets, threshold 12 → None (full inject).
        let p = EffectivePersona {
            learned_context: vec!["a".into(), "b".into()],
            ..EffectivePersona::default()
        };
        let out = refiner(p, false, 12)
            .refine("anything", aivyx_core::SessionId::new())
            .await;
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn embed_failure_falls_back_to_none() {
        let out = refiner(big_persona(), true, 12)
            .refine("how do I deploy", aivyx_core::SessionId::new())
            .await;
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn selects_relevant_facets_and_keeps_core_and_constraints() {
        let out = refiner(big_persona(), false, 12)
            .refine("how do I deploy", aivyx_core::SessionId::new())
            .await
            .expect("large persona + ok embed → Some");

        // The two "deploy" facets are relevant and kept.
        assert!(out.contains("deploy runbook lives in wiki"));
        assert!(out.contains("deploy window is Friday"));
        // A non-relevant soft facet is dropped.
        assert!(!out.contains("misc note 0"));
        // Invariant: protected constraint + scalar identity are
        // ALWAYS present even though they were never "selected".
        assert!(out.contains("never deploy without approval"));
        assert!(out.contains("Ada"));
    }

    #[tokio::test]
    async fn unrelated_query_strips_to_core_only() {
        // No facet matches; large Soul still bounds to just the
        // always-on core + constraints (the adaptive point).
        let out = refiner(big_persona(), false, 12)
            .refine("tell me a joke", aivyx_core::SessionId::new())
            .await
            .expect("Some");
        assert!(!out.contains("deploy runbook lives in wiki"));
        assert!(!out.contains("misc note 3"));
        // Core/constraint still there.
        assert!(out.contains("never deploy without approval"));
    }

    #[tokio::test]
    async fn empty_persona_is_none() {
        let out = refiner(EffectivePersona::default(), false, 0)
            .refine("hi", aivyx_core::SessionId::new())
            .await;
        // threshold 0 but zero facets → still None (nothing to
        // select).
        assert!(out.is_none());
    }
}

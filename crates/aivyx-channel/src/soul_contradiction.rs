//! Chapter Accord — contradiction detection over the Persona ("Soul").
//!
//! The identity-stack sibling of [`crate::contradiction`] (Chapter Concord,
//! which does this for memory). The Soul accretes operator-approved reflection
//! deltas; the lifecycle layer merges near-*duplicates* (Phase 87) and decays
//! the *unreinforced* (Phase 85), but nothing ever flags two approved facets
//! that flatly *contradict* — a seeded "communicate concisely" living next to a
//! later "give thorough, detailed explanations", both injected every turn. Nor
//! does anything notice the Soul drifting *against the operator's declared
//! Profile constraints* ("be candid, never flatter me" vs a learned "warm and
//! encouraging").
//!
//! One batched LLM call, on demand (`aivyx persona conflicts` / the Studio),
//! zero background cost, no config. Best-effort: a parse/LLM failure yields no
//! conflicts, never an error. This module only *finds*; resolution removes the
//! losing facet via a normal `RemoveList` persona delta (operator-authored,
//! revertible), which lives in the daemon handler.

use std::sync::Arc;

use serde::Deserialize;

use aivyx_ipc::persona::EffectivePersona;

// Re-export the wire types so CLI/Studio (which depend on aivyx-channel, not
// aivyx-ipc directly) can name them — mirrors `contradiction::MemoryConflict`.
pub use aivyx_ipc::soul_conflict::{SoulConflict, SoulFacet};
use aivyx_llm::{ContentBlock, LlmMessage, LlmProvider, LlmRequest};
use aivyx_core::CancellationToken;

/// The five removable soft-list categories the Soul accretes (the always-on
/// operator `behavioral_constraints` core is NOT here — it's the immutable
/// reference side for cross-layer conflicts). Each is `(wire_label, accessor)`.
type Lists<'a> = [(&'static str, &'a [String]); 5];

fn soft_lists(p: &EffectivePersona) -> Lists<'_> {
    [
        ("character_traits", &p.character_traits),
        ("communication_adaptations", &p.communication_adaptations),
        ("behavioral_preferences", &p.behavioral_preferences),
        ("learned_context", &p.learned_context),
        ("relationship_milestones", &p.relationship_milestones),
    ]
}

/// Bounds on one detection pass — keep the single LLM call cheap and its prompt
/// inside a small local model's context.
pub struct SoulContradictionConfig {
    pub max_facet_chars: usize,
    pub max_conflicts: usize,
    pub max_tokens: u32,
}

impl Default for SoulContradictionConfig {
    fn default() -> Self {
        Self {
            max_facet_chars: 200,
            max_conflicts: 30,
            max_tokens: 700,
        }
    }
}

/// What the model returns per conflict, before validation. Each side names its
/// own `(category, value)` so a conflict can span two categories or reference a
/// profile constraint.
#[derive(Debug, Deserialize)]
struct RawConflict {
    category_a: String,
    value_a: String,
    category_b: String,
    value_b: String,
    #[serde(default)]
    reason: String,
}

/// LLM-backed contradiction detector over an [`EffectivePersona`] snapshot.
pub struct SoulContradictionDetector {
    provider: Arc<dyn LlmProvider>,
    model: String,
    config: SoulContradictionConfig,
}

impl SoulContradictionDetector {
    pub fn new(provider: Arc<dyn LlmProvider>, model: impl Into<String>) -> Self {
        Self {
            provider,
            model: model.into(),
            config: SoulContradictionConfig::default(),
        }
    }

    pub fn with_config(mut self, config: SoulContradictionConfig) -> Self {
        self.config = config;
        self
    }

    fn system_prompt() -> &'static str {
        "You audit an AI assistant's evolving PERSONA (its \"Soul\") for \
         CONTRADICTIONS. You are given the assistant's learned facets, grouped \
         by category, and — separately — the operator's declared \
         profile_constraint rules (which are FIXED and authoritative). Find \
         pairs that are genuinely INCOMPATIBLE as standing guidance for how the \
         assistant should behave or what it believes about the operator — where \
         following one means violating the other (e.g. \"communicate very \
         concisely\" vs \"always give thorough, detailed explanations\"; a \
         learned \"warm and effusive\" vs a profile_constraint \"never flatter \
         me, be candid\"). Output ONLY a JSON array of `{\"category_a\":\"...\",\
         \"value_a\":\"...\",\"category_b\":\"...\",\"value_b\":\"...\",\
         \"reason\":\"...\"}`, where each `(category, value)` MUST be one of the \
         facets or constraints listed below, copied VERBATIM, and the two sides \
         must be different. When one side is an operator rule, use category \
         \"profile_constraint\". `reason` is one short clause naming the \
         incompatibility. Report ONLY real contradictions — NOT facets that \
         merely differ, add nuance, or cover different situations. If there are \
         none, output `[]`. No prose, no markdown fences."
    }

    /// Render the persona snapshot into the user prompt. Pure + testable.
    fn user_prompt(p: &EffectivePersona, max_chars: usize) -> String {
        let clip = |s: &str| -> String {
            let one = s.replace('\n', " ");
            if one.chars().count() > max_chars {
                one.chars().take(max_chars).collect::<String>() + "…"
            } else {
                one
            }
        };
        let mut s = String::from("Audit this Persona for contradictions.\n\n## learned facets\n");
        let mut any = false;
        for (label, values) in soft_lists(p) {
            for v in values {
                any = true;
                s.push_str(&format!("- category={label} | {}\n", clip(v)));
            }
        }
        if !any {
            s.push_str("(none)\n");
        }
        s.push_str("\n## operator profile_constraint rules (fixed)\n");
        if p.behavioral_constraints.is_empty() {
            s.push_str("(none)\n");
        } else {
            for c in &p.behavioral_constraints {
                s.push_str(&format!("- category=profile_constraint | {}\n", clip(c)));
            }
        }
        s.push_str("\nOutput the JSON conflict array now.");
        s
    }

    /// Tolerant parse: locate the outermost `[ … ]` and decode. Empty on any
    /// failure — best-effort.
    fn parse(raw: &str) -> Vec<RawConflict> {
        let (Some(start), Some(end)) = (raw.find('['), raw.rfind(']')) else {
            return Vec::new();
        };
        if end <= start {
            return Vec::new();
        }
        serde_json::from_str::<Vec<RawConflict>>(&raw[start..=end]).unwrap_or_default()
    }

    /// Validate raw conflicts against the snapshot: each `(category, value)`
    /// must be a real facet or constraint, the two sides distinct, ordered
    /// canonically, and deduped by id. A `profile_constraint` side is always
    /// placed as `b` (immutable) and marks the conflict `cross_layer`.
    fn validate(
        p: &EffectivePersona,
        raws: Vec<RawConflict>,
        cap: usize,
    ) -> Vec<SoulConflict> {
        use std::collections::HashSet;
        let lists = soft_lists(p);
        // Does `(category, value)` name a real learned facet?
        let is_facet = |cat: &str, val: &str| -> bool {
            lists
                .iter()
                .any(|(label, values)| *label == cat && values.iter().any(|v| v == val))
        };
        let is_constraint = |cat: &str, val: &str| -> bool {
            cat == SoulFacet::PROFILE_CONSTRAINT
                && p.behavioral_constraints.iter().any(|c| c == val)
        };
        let exists = |cat: &str, val: &str| is_facet(cat, val) || is_constraint(cat, val);

        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<SoulConflict> = Vec::new();
        for rc in raws {
            if rc.category_a == rc.category_b && rc.value_a == rc.value_b {
                continue; // a side paired with itself
            }
            if !exists(&rc.category_a, &rc.value_a) || !exists(&rc.category_b, &rc.value_b) {
                continue; // hallucinated facet — drop
            }
            let a_is_constraint = rc.category_a == SoulFacet::PROFILE_CONSTRAINT;
            let b_is_constraint = rc.category_b == SoulFacet::PROFILE_CONSTRAINT;
            // Two constraints contradicting each other are the operator's to
            // reconcile, not ours to remove — skip.
            if a_is_constraint && b_is_constraint {
                continue;
            }
            // Canonical order: the immutable profile_constraint (if any) is
            // always side `b`; otherwise order by (category, value) for a
            // stable id.
            let ((cat_a, val_a), (cat_b, val_b), cross) = if a_is_constraint {
                (
                    (rc.category_b.clone(), rc.value_b.clone()),
                    (rc.category_a.clone(), rc.value_a.clone()),
                    true,
                )
            } else if b_is_constraint {
                (
                    (rc.category_a.clone(), rc.value_a.clone()),
                    (rc.category_b.clone(), rc.value_b.clone()),
                    true,
                )
            } else {
                // both learned facets — order deterministically
                let ka = (rc.category_a.as_str(), rc.value_a.as_str());
                let kb = (rc.category_b.as_str(), rc.value_b.as_str());
                if ka <= kb {
                    (
                        (rc.category_a.clone(), rc.value_a.clone()),
                        (rc.category_b.clone(), rc.value_b.clone()),
                        false,
                    )
                } else {
                    (
                        (rc.category_b.clone(), rc.value_b.clone()),
                        (rc.category_a.clone(), rc.value_a.clone()),
                        false,
                    )
                }
            };
            let id = SoulConflict::make_id(&cat_a, &val_a, &cat_b, &val_b);
            if !seen.insert(id.clone()) {
                continue;
            }
            out.push(SoulConflict {
                id,
                a: SoulFacet { category: cat_a, value: val_a },
                b: SoulFacet { category: cat_b, value: val_b },
                reason: rc.reason.trim().to_string(),
                cross_layer: cross,
            });
            if out.len() >= cap {
                break;
            }
        }
        out
    }

    /// Run one detection pass over the snapshot. Best-effort: empty on any
    /// LLM/parse failure, or when there are fewer than two facets to compare.
    pub async fn detect(&self, persona: &EffectivePersona) -> Vec<SoulConflict> {
        let facet_count: usize = soft_lists(persona).iter().map(|(_, v)| v.len()).sum();
        // Need at least two things that could conflict (two facets, or one
        // facet + one constraint).
        if facet_count == 0 || (facet_count < 2 && persona.behavioral_constraints.is_empty()) {
            return Vec::new();
        }
        let user = Self::user_prompt(persona, self.config.max_facet_chars);
        let messages = vec![LlmMessage::User {
            content: vec![ContentBlock::Text { text: user }],
        }];
        let request = LlmRequest {
            system: Some(Self::system_prompt()),
            messages: &messages,
            tools: &[],
            model: &self.model,
            max_tokens: self.config.max_tokens,
            temperature: Some(0.0),
        };
        let cancel = CancellationToken::new();
        let raw = match self.provider.chat_stream(request, &cancel).await {
            Ok(stream) => match stream.finish().await {
                Ok(aivyx_llm::LlmStepEnd::FinalMessage { text, .. }) => text,
                _ => return Vec::new(),
            },
            Err(_) => return Vec::new(),
        };
        Self::validate(persona, Self::parse(&raw), self.config.max_conflicts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn persona() -> EffectivePersona {
        let mut p = EffectivePersona::default();
        p.character_traits = vec!["communicate concisely".into(), "warm and effusive".into()];
        p.behavioral_preferences = vec!["give thorough, detailed explanations".into()];
        p.behavioral_constraints = vec!["never flatter me, be candid".into()];
        p
    }

    #[test]
    fn user_prompt_lists_facets_and_constraints() {
        let s = SoulContradictionDetector::user_prompt(&persona(), 200);
        assert!(s.contains("category=character_traits | communicate concisely"));
        assert!(s.contains("category=behavioral_preferences | give thorough"));
        assert!(s.contains("category=profile_constraint | never flatter me"));
    }

    #[test]
    fn validate_keeps_facet_vs_facet_and_orders_canonically() {
        let raws = vec![RawConflict {
            category_a: "behavioral_preferences".into(),
            value_a: "give thorough, detailed explanations".into(),
            category_b: "character_traits".into(),
            value_b: "communicate concisely".into(),
            reason: "concise vs thorough".into(),
        }];
        let out = SoulContradictionDetector::validate(&persona(), raws, 30);
        assert_eq!(out.len(), 1);
        assert!(!out[0].cross_layer);
        // canonical order: (category,value) ascending → behavioral_preferences
        // sorts before character_traits.
        assert_eq!(out[0].a.category, "behavioral_preferences");
        assert_eq!(out[0].b.category, "character_traits");
    }

    #[test]
    fn validate_flags_cross_layer_and_puts_constraint_as_b() {
        let raws = vec![RawConflict {
            category_a: "profile_constraint".into(),
            value_a: "never flatter me, be candid".into(),
            category_b: "character_traits".into(),
            value_b: "warm and effusive".into(),
            reason: "flattery vs candor".into(),
        }];
        let out = SoulContradictionDetector::validate(&persona(), raws, 30);
        assert_eq!(out.len(), 1);
        assert!(out[0].cross_layer, "constraint side ⇒ cross_layer");
        assert_eq!(out[0].a.category, "character_traits", "learned facet is removable side a");
        assert!(out[0].b.is_profile_constraint(), "constraint is immutable side b");
    }

    #[test]
    fn validate_drops_hallucinated_facets() {
        let raws = vec![RawConflict {
            category_a: "character_traits".into(),
            value_a: "a trait that was never learned".into(),
            category_b: "character_traits".into(),
            value_b: "communicate concisely".into(),
            reason: "x".into(),
        }];
        assert!(SoulContradictionDetector::validate(&persona(), raws, 30).is_empty());
    }

    #[test]
    fn validate_dedups_and_skips_constraint_vs_constraint() {
        let raws = vec![
            RawConflict {
                category_a: "profile_constraint".into(),
                value_a: "never flatter me, be candid".into(),
                category_b: "profile_constraint".into(),
                value_b: "never flatter me, be candid".into(),
                reason: "self".into(),
            },
        ];
        assert!(SoulContradictionDetector::validate(&persona(), raws, 30).is_empty());
    }
}

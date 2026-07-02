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

/// What the model returns per conflict, before validation. Each side is the
/// integer index `[N]` of a listed facet/rule — NOT its verbatim text, so a
/// paraphrasing local model can still reference it reliably (the Concord
/// `seq`-reference trick; requiring verbatim value text proved fragile live).
#[derive(Debug, Deserialize)]
struct RawConflict {
    a: usize,
    b: usize,
    #[serde(default)]
    reason: String,
}

/// One indexed item shown to the model: its display index, category, and value.
/// `is_rule` marks the immutable operator profile_constraint side.
struct Item {
    category: String,
    value: String,
    is_rule: bool,
}

/// Flatten the persona snapshot into the indexed item list the prompt shows and
/// `validate` maps back through. Learned soft-list facets first, then the
/// operator's profile_constraint rules.
fn items_of(p: &EffectivePersona) -> Vec<Item> {
    let mut items = Vec::new();
    for (label, values) in soft_lists(p) {
        for v in values {
            items.push(Item {
                category: label.to_string(),
                value: v.clone(),
                is_rule: false,
            });
        }
    }
    for c in &p.behavioral_constraints {
        items.push(Item {
            category: SoulFacet::PROFILE_CONSTRAINT.to_string(),
            value: c.clone(),
            is_rule: true,
        });
    }
    items
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
         CONTRADICTIONS. You are given a NUMBERED list of the assistant's \
         standing guidance: learned facets, and the operator's FIXED rules \
         (marked RULE). Find pairs that are genuinely INCOMPATIBLE as standing \
         guidance — where following one means violating the other (e.g. \"be \
         extremely concise\" vs \"always give long, detailed explanations\"; a \
         learned \"warm and effusive\" vs a RULE \"never flatter me, be \
         candid\"). Refer to each item by its number in brackets. Output ONLY a \
         JSON array of `{\"a\":N,\"b\":M,\"reason\":\"...\"}`, where N and M are \
         the item numbers of the two incompatible items (different numbers). \
         `reason` is one short clause naming the incompatibility. Report ONLY \
         real contradictions — NOT items that merely differ, add nuance, or \
         cover different situations. If there are none, output `[]`. No prose, \
         no markdown fences."
    }

    /// Render the indexed item list into the user prompt. Pure + testable.
    fn user_prompt(items: &[Item], max_chars: usize) -> String {
        let clip = |s: &str| -> String {
            let one = s.replace('\n', " ");
            if one.chars().count() > max_chars {
                one.chars().take(max_chars).collect::<String>() + "…"
            } else {
                one
            }
        };
        let mut s = String::from(
            "Audit this Persona for contradictions. Items (refer to each by its \
             [number]):\n\n",
        );
        for (i, it) in items.iter().enumerate() {
            let tag = if it.is_rule { "RULE" } else { &it.category };
            s.push_str(&format!("[{i}] ({tag}) {}\n", clip(&it.value)));
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

    /// Validate raw conflicts against the indexed item list: each index must be
    /// in range, the two sides distinct, ordered canonically, and deduped by id.
    /// A `profile_constraint` (rule) side is always placed as `b` (immutable)
    /// and marks the conflict `cross_layer`; two rules are skipped (the
    /// operator's to reconcile, not ours to remove).
    fn validate(items: &[Item], raws: Vec<RawConflict>, cap: usize) -> Vec<SoulConflict> {
        use std::collections::HashSet;
        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<SoulConflict> = Vec::new();
        for rc in raws {
            if rc.a == rc.b {
                continue;
            }
            let (Some(ia), Some(ib)) = (items.get(rc.a), items.get(rc.b)) else {
                continue; // out-of-range index — drop
            };
            if ia.is_rule && ib.is_rule {
                continue;
            }
            // Canonical order: an immutable rule is always side `b`; otherwise
            // order by (category, value) for a stable id.
            let (fa, fb, cross) = if ia.is_rule {
                (ib, ia, true)
            } else if ib.is_rule {
                (ia, ib, true)
            } else {
                let ka = (ia.category.as_str(), ia.value.as_str());
                let kb = (ib.category.as_str(), ib.value.as_str());
                if ka <= kb { (ia, ib, false) } else { (ib, ia, false) }
            };
            let id = SoulConflict::make_id(&fa.category, &fa.value, &fb.category, &fb.value);
            if !seen.insert(id.clone()) {
                continue;
            }
            out.push(SoulConflict {
                id,
                a: SoulFacet { category: fa.category.clone(), value: fa.value.clone() },
                b: SoulFacet { category: fb.category.clone(), value: fb.value.clone() },
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
    /// LLM/parse failure, or when there are fewer than two items to compare.
    pub async fn detect(&self, persona: &EffectivePersona) -> Vec<SoulConflict> {
        let items = items_of(persona);
        if items.len() < 2 {
            return Vec::new();
        }
        let user = Self::user_prompt(&items, self.config.max_facet_chars);
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
        let Ok(mut stream) = self.provider.chat_stream(request, &cancel).await else {
            return Vec::new();
        };
        // The stream MUST be drained before `finish()` (the ollama provider
        // errors otherwise) — mirrors Chapter Concord's consumption.
        while stream.next_event().await.map(|e| e.is_some()).unwrap_or(false) {}
        let raw = match stream.finish().await {
            Ok(aivyx_llm::LlmStepEnd::FinalMessage { text, .. }) => text,
            _ => return Vec::new(),
        };
        Self::validate(&items, Self::parse(&raw), self.config.max_conflicts)
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

    // Item layout for persona(): [0] character_traits "communicate concisely",
    // [1] character_traits "warm and effusive", [2] behavioral_preferences
    // "give thorough, detailed explanations", [3] RULE "never flatter me…".

    #[test]
    fn user_prompt_numbers_facets_and_marks_rules() {
        let s = SoulContradictionDetector::user_prompt(&items_of(&persona()), 200);
        assert!(s.contains("[0] (character_traits) communicate concisely"));
        assert!(s.contains("[2] (behavioral_preferences) give thorough"));
        assert!(s.contains("[3] (RULE) never flatter me"), "rules are tagged RULE: {s}");
    }

    #[test]
    fn validate_keeps_facet_vs_facet_and_orders_canonically() {
        let items = items_of(&persona());
        // model referenced items 2 and 0 (order-insensitive input)
        let raws = vec![RawConflict { a: 2, b: 0, reason: "concise vs thorough".into() }];
        let out = SoulContradictionDetector::validate(&items, raws, 30);
        assert_eq!(out.len(), 1);
        assert!(!out[0].cross_layer);
        // canonical order: behavioral_preferences sorts before character_traits.
        assert_eq!(out[0].a.category, "behavioral_preferences");
        assert_eq!(out[0].b.category, "character_traits");
    }

    #[test]
    fn validate_flags_cross_layer_and_puts_rule_as_b() {
        let items = items_of(&persona());
        let raws = vec![RawConflict { a: 3, b: 1, reason: "flattery vs candor".into() }];
        let out = SoulContradictionDetector::validate(&items, raws, 30);
        assert_eq!(out.len(), 1);
        assert!(out[0].cross_layer, "rule side ⇒ cross_layer");
        assert_eq!(out[0].a.category, "character_traits", "learned facet is removable side a");
        assert!(out[0].b.is_profile_constraint(), "rule is immutable side b");
    }

    #[test]
    fn validate_drops_out_of_range_and_self_pairs() {
        let items = items_of(&persona());
        assert!(SoulContradictionDetector::validate(
            &items,
            vec![RawConflict { a: 99, b: 0, reason: "x".into() }],
            30,
        )
        .is_empty());
        assert!(SoulContradictionDetector::validate(
            &items,
            vec![RawConflict { a: 2, b: 2, reason: "self".into() }],
            30,
        )
        .is_empty());
    }

    #[test]
    fn validate_dedups_same_pair() {
        let items = items_of(&persona());
        let raws = vec![
            RawConflict { a: 0, b: 2, reason: "one".into() },
            RawConflict { a: 2, b: 0, reason: "same pair, swapped".into() },
        ];
        assert_eq!(
            SoulContradictionDetector::validate(&items, raws, 30).len(),
            1,
            "the same pair in either order dedups to one conflict"
        );
    }

    #[test]
    fn validate_skips_rule_vs_rule() {
        let mut p = persona();
        p.behavioral_constraints.push("always agree with me".into()); // 2nd rule → item[4]
        let items = items_of(&p);
        let raws = vec![RawConflict { a: 3, b: 4, reason: "two rules".into() }];
        assert!(
            SoulContradictionDetector::validate(&items, raws, 30).is_empty(),
            "two operator rules are the operator's to reconcile, not removable"
        );
    }
}

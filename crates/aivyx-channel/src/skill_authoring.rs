//! Chapter Praxis (PX.1) — the specialized-skill authoring engine.
//!
//! Where the agent stops only *knowing* and starts being *able*: it reads
//! its own consolidated knowledge about a subject it understands well —
//! the [[Codex]] `WikiPage` (what it knows) + the [[Lattice]] typed graph
//! neighbourhood (how it connects) — and **synthesizes a specialized
//! skill** for it, proposed for the operator to approve.
//!
//! A sibling of the Whetstone refinement engine
//! ([`crate::skill_refinement`]): same governance (a Pending persona
//! proposal in the existing Agents approve/edit/reject UI), same
//! propose-only, opt-in, best-effort posture. The difference is the
//! *source* and the *output* — Whetstone sharpens an underperforming
//! existing skill; Praxis authors a **new** one from knowledge, for a
//! topic that has none. It is the first real consumer of the WH.1
//! `LearnedSkill.domain` field (the specialization tag).

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use aivyx_core::CancellationToken;
use aivyx_llm::{LlmMessage, LlmProvider, LlmRequest, LlmStepEnd};

use crate::knowledge_graph::PersistentGraphStore;
use crate::knowledge_wiki::PersistentWikiStore;
use crate::persona::{
    LearnedSkill, PersonaDeltaCategory, PersonaDeltaOp, ProposedPersonaDelta,
    SkillAuthor, SkillProvenance,
};
use crate::persona_proposal::PersistentPersonaProposalLog;

// The `[skill_authoring]` config lives in `aivyx-config`; re-exported here.
pub use aivyx_config::SkillAuthoringConfig;

/// The synthesized body of a specialized skill (the LLM's output). The
/// engine supplies the `name`/`domain` (the topic) and the provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftedSkill {
    pub trigger: String,
    pub procedure: String,
}

/// Synthesizes a specialized skill from a topic's consolidated knowledge.
/// Abstracted so the engine is testable without a live model; the
/// production impl is [`LlmSpecializationDrafter`].
#[async_trait]
pub trait SpecializationDrafter: Send + Sync {
    /// Draft a skill for `topic` from its wiki `summary` + rendered graph
    /// `relations`. `None` to skip (draft failure / empty / ungrounded).
    async fn draft(
        &self,
        topic: &str,
        summary: &str,
        relations: &str,
    ) -> Option<DraftedSkill>;
}

/// One authoring pass's outcome.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SkillAuthoringStat {
    /// Specialized skills authored (proposals filed) this pass.
    pub filed: usize,
    /// Wiki pages considered.
    pub considered: usize,
    /// Topics skipped: thin page or sparse graph neighbourhood.
    pub skipped_thin: usize,
    /// Topics skipped: already covered by an existing skill.
    pub skipped_covered: usize,
    /// Topics skipped: an authoring proposal already exists (deduped).
    pub deduped: usize,
}

/// Chapter Praxis — author specialized skills for knowledge-rich, skill-
/// less topics.
///
/// `learned_skills_raw` is the effective persona's raw `LearnedSkill`
/// JSON (decoded for the dedup check: a topic already owning a skill — by
/// name or `domain` — is never re-authored).
#[allow(clippy::too_many_arguments)]
pub async fn propose_specialized_skills(
    wiki_store: &PersistentWikiStore,
    graph_store: &PersistentGraphStore,
    learned_skills_raw: &[String],
    drafter: &dyn SpecializationDrafter,
    proposal_log: &PersistentPersonaProposalLog,
    config: &SkillAuthoringConfig,
    source_label: &str,
    now_ms: u64,
) -> SkillAuthoringStat {
    let mut stat = SkillAuthoringStat::default();
    if !config.enabled {
        return stat;
    }
    // Topics already owning a skill (by name or domain) — never re-author.
    let mut covered: HashSet<String> = HashSet::new();
    for raw in learned_skills_raw {
        if let Some(s) = LearnedSkill::from_json_value(raw) {
            covered.insert(s.name.clone());
            if let Some(d) = s.domain {
                covered.insert(d);
            }
        }
    }

    let pages = match wiki_store.all_pages().await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("aivyx skill-authoring: wiki all_pages failed: {e}");
            return stat;
        }
    };

    for page in pages {
        if stat.filed >= config.max_per_cycle {
            break;
        }
        stat.considered += 1;

        // Substance floor — a stub page isn't skill-worthy.
        if page.summary.chars().count() < config.min_summary_chars {
            stat.skipped_thin += 1;
            continue;
        }
        // Skill-less — never compete with an existing skill for the topic.
        if covered.contains(&page.topic) {
            stat.skipped_covered += 1;
            continue;
        }
        // Deterministic id → a re-run (or a prior pending/rejected
        // proposal) dedups instead of nagging.
        let proposal_id = format!("skill-author:{}", page.topic);
        if proposal_log.get(&proposal_id).is_some() {
            stat.deduped += 1;
            continue;
        }
        // Graph neighbourhood — evidence it's a connected, procedural
        // subject (not an isolated fact).
        let edges = graph_store.out_edges(&page.topic).await.unwrap_or_default();
        if edges.len() < config.min_edges {
            stat.skipped_thin += 1;
            continue;
        }
        let relations = edges
            .iter()
            .map(|t| format!("{} {} {}", t.subject, t.predicate, t.object))
            .collect::<Vec<_>>()
            .join("\n");

        let Some(drafted) = drafter.draft(&page.topic, &page.summary, &relations).await
        else {
            continue;
        };
        let trigger = drafted.trigger.trim().to_string();
        let procedure = drafted.procedure.trim().to_string();
        if trigger.is_empty() || procedure.is_empty() {
            continue;
        }

        let skill = LearnedSkill {
            name: page.topic.clone(),
            trigger,
            procedure,
            version: 1,
            provenance: SkillProvenance {
                author: SkillAuthor::Agent,
                reason: Some(format!(
                    "authored from the `{}` knowledge page + graph",
                    page.topic,
                )),
            },
            refined_from: None,
            domain: Some(page.topic.clone()),
        };
        let op = ProposedPersonaDelta {
            category: PersonaDeltaCategory::LearnedSkill,
            op: PersonaDeltaOp::AppendList { value: skill.to_json_value() },
            reason: Some(format!(
                "specialized skill authored from consolidated knowledge about `{}`",
                page.topic,
            )),
            supersedes_proposal_id: None,
        };
        if let Err(e) = proposal_log
            .append_pending(proposal_id, now_ms, source_label.to_string(), op)
            .await
        {
            eprintln!("aivyx skill-authoring: append failed for {}: {e}", page.topic);
            continue;
        }
        stat.filed += 1;
    }
    stat
}

/// Production [`SpecializationDrafter`] over the agent's own
/// `LlmProvider`. Asks for a grounded `{trigger, procedure}` JSON; any
/// failure maps to `None` (per-topic skip — best-effort).
pub struct LlmSpecializationDrafter {
    provider: Arc<dyn LlmProvider>,
    model: String,
}

impl LlmSpecializationDrafter {
    pub fn new(provider: Arc<dyn LlmProvider>, model: String) -> Self {
        Self { provider, model }
    }
}

const AUTHOR_MAX_TOKENS: u32 = 600;
const AUTHOR_SYSTEM_PROMPT: &str = "You write a reusable SKILL for an AI \
assistant — a named procedure it will follow when a trigger matches — \
from what it already knows about one topic (a knowledge summary + typed \
relations it extracted from its own memory). Ground the skill ONLY in the \
provided knowledge; do not invent facts. Output ONLY a JSON object: \
{\"trigger\":\"one or two sentences: when this skill applies\",\
\"procedure\":\"concrete step-by-step instructions that use the stated \
relations\"}. No prose outside the JSON, no markdown fences.";

/// Tolerant parse of the drafter's reply: the outermost `{...}` as a
/// `{trigger, procedure}` object.
fn parse_drafted(text: &str) -> Option<DraftedSkill> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    let raw = &text[start..=end];
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let trigger = v.get("trigger")?.as_str()?.trim().to_string();
    let procedure = v.get("procedure")?.as_str()?.trim().to_string();
    if trigger.is_empty() || procedure.is_empty() {
        return None;
    }
    Some(DraftedSkill { trigger, procedure })
}

#[async_trait]
impl SpecializationDrafter for LlmSpecializationDrafter {
    async fn draft(
        &self,
        topic: &str,
        summary: &str,
        relations: &str,
    ) -> Option<DraftedSkill> {
        let user = format!(
            "Topic: {topic}\n\nWhat the assistant knows (summary):\n{summary}\n\n\
             Typed relations (subject predicate object):\n{relations}\n\n\
             Write the skill as the specified JSON.",
        );
        let messages = vec![LlmMessage::user_text(user)];
        let request = LlmRequest {
            model: &self.model,
            system: Some(AUTHOR_SYSTEM_PROMPT),
            messages: &messages,
            tools: &[],
            max_tokens: AUTHOR_MAX_TOKENS,
            temperature: Some(0.3),
        };
        let cancel = CancellationToken::new();
        let mut stream = self.provider.chat_stream(request, &cancel).await.ok()?;
        while let Ok(Some(_)) = stream.next_event().await {}
        match stream.finish().await.ok()? {
            LlmStepEnd::FinalMessage { text, .. } => parse_drafted(&text),
            LlmStepEnd::ToolCalls { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{KeyDomain, RedbStorage, StorageConfig};

    use crate::knowledge_graph::GraphTriple;

    struct FixedDrafter(Option<DraftedSkill>);
    #[async_trait]
    impl SpecializationDrafter for FixedDrafter {
        async fn draft(&self, _t: &str, _s: &str, _r: &str) -> Option<DraftedSkill> {
            self.0.clone()
        }
    }

    struct Harness {
        wiki: Arc<PersistentWikiStore>,
        graph: Arc<PersistentGraphStore>,
        proposals: PersistentPersonaProposalLog,
    }

    async fn harness() -> Harness {
        let base = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base)
            .join(format!("aivyx-author-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = RedbStorage::open(
            StorageConfig::new(dir.join("s.redb")),
            MasterKey::from_raw([5u8; 32]),
        )
        .await
        .unwrap();
        Harness {
            wiki: Arc::new(PersistentWikiStore::new(store.domain(KeyDomain::KnowledgeWiki))),
            graph: Arc::new(PersistentGraphStore::new(store.domain(KeyDomain::KnowledgeGraph))),
            proposals: PersistentPersonaProposalLog::open(
                store.domain(KeyDomain::PersonaProposals),
                vec![0u8; 32],
            )
            .await
            .unwrap(),
        }
    }

    fn cfg() -> SkillAuthoringConfig {
        SkillAuthoringConfig { enabled: true, min_summary_chars: 50, min_edges: 2, max_per_cycle: 2 }
    }

    fn drafted() -> Option<DraftedSkill> {
        Some(DraftedSkill {
            trigger: "when deploying".into(),
            procedure: "1. run ci 2. ship".into(),
        })
    }

    fn page(topic: &str, summary: String) -> aivyx_ipc::wiki::WikiPage {
        aivyx_ipc::wiki::WikiPage {
            topic: topic.into(),
            summary,
            source_seqs: vec![1],
            entry_count: 1,
            backlinks: vec![],
            updated_at: 1,
            source_fingerprint: 1,
        }
    }

    async fn seed_rich_topic(h: &Harness, topic: &str) {
        // summary clears the 50-char floor.
        h.wiki.put_page(&page(topic, "A".repeat(80))).await.unwrap();
        for obj in ["ci", "tests"] {
            h.graph
                .put_triple(&GraphTriple {
                    subject: topic.into(),
                    predicate: "depends-on".into(),
                    object: obj.into(),
                    source_seqs: vec![1],
                    mentions: 1,
                    updated_at: 1,
                })
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn rich_skill_less_topic_yields_a_specialized_proposal() {
        let h = harness().await;
        seed_rich_topic(&h, "deploy").await;
        let stat = propose_specialized_skills(
            &h.wiki, &h.graph, &[], &FixedDrafter(drafted()),
            &h.proposals, &cfg(), "reflection", 1000,
        )
        .await;
        assert_eq!(stat.filed, 1);
        let p = h.proposals.get("skill-author:deploy").expect("proposal filed");
        assert_eq!(p.proposed_op.category, PersonaDeltaCategory::LearnedSkill);
        match &p.proposed_op.op {
            PersonaDeltaOp::AppendList { value } => {
                let s = LearnedSkill::from_json_value(value).unwrap();
                assert_eq!(s.name, "deploy");
                assert_eq!(s.domain.as_deref(), Some("deploy"));
                assert_eq!(s.provenance.author, SkillAuthor::Agent);
                assert_eq!(s.version, 1);
                assert_eq!(s.procedure, "1. run ci 2. ship");
            }
            other => panic!("expected AppendList, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn thin_page_sparse_graph_covered_and_disabled_file_nothing() {
        let h = harness().await;
        // Thin page (below the floor) + a sparse graph.
        h.wiki.put_page(&page("stub", "tiny".into())).await.unwrap();
        // Rich page but only ONE edge → sparse neighbourhood.
        h.wiki.put_page(&page("lonely", "A".repeat(80))).await.unwrap();
        h.graph
            .put_triple(&GraphTriple {
                subject: "lonely".into(), predicate: "is".into(), object: "alone".into(),
                source_seqs: vec![1], mentions: 1, updated_at: 1,
            })
            .await
            .unwrap();
        let s = propose_specialized_skills(
            &h.wiki, &h.graph, &[], &FixedDrafter(drafted()), &h.proposals, &cfg(), "r", 1000,
        )
        .await;
        assert_eq!(s.filed, 0);
        assert_eq!(s.skipped_thin, 2);

        // A rich topic already covered by a skill (domain match) → skipped.
        seed_rich_topic(&h, "deploy").await;
        let existing = vec![LearnedSkill {
            name: "my-deploy".into(),
            trigger: "x".into(),
            procedure: "y".into(),
            domain: Some("deploy".into()),
            ..Default::default()
        }
        .to_json_value()];
        let s2 = propose_specialized_skills(
            &h.wiki, &h.graph, &existing, &FixedDrafter(drafted()), &h.proposals, &cfg(), "r", 1000,
        )
        .await;
        assert_eq!(s2.filed, 0);
        assert_eq!(s2.skipped_covered, 1);

        // Disabled config → nothing.
        let off = SkillAuthoringConfig { enabled: false, ..cfg() };
        let s3 = propose_specialized_skills(
            &h.wiki, &h.graph, &[], &FixedDrafter(drafted()), &h.proposals, &off, "r", 1000,
        )
        .await;
        assert_eq!(s3.filed, 0);
    }

    #[tokio::test]
    async fn authoring_is_deduped_on_rerun() {
        let h = harness().await;
        seed_rich_topic(&h, "deploy").await;
        let first = propose_specialized_skills(
            &h.wiki, &h.graph, &[], &FixedDrafter(drafted()), &h.proposals, &cfg(), "r", 1000,
        )
        .await;
        assert_eq!(first.filed, 1);
        let second = propose_specialized_skills(
            &h.wiki, &h.graph, &[], &FixedDrafter(drafted()), &h.proposals, &cfg(), "r", 2000,
        )
        .await;
        assert_eq!(second.filed, 0);
        assert_eq!(second.deduped, 1);
    }

    #[test]
    fn parse_drafted_tolerates_prose_around_json() {
        let out = parse_drafted(
            "Here you go:\n{\"trigger\":\"on X\",\"procedure\":\"do Y\"} — hope it helps",
        )
        .unwrap();
        assert_eq!(out.trigger, "on X");
        assert_eq!(out.procedure, "do Y");
        assert!(parse_drafted("no json here").is_none());
        assert!(parse_drafted("{\"trigger\":\"\",\"procedure\":\"y\"}").is_none());
    }
}

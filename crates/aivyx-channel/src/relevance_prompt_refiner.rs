//! Phase 117 Task 3 — Tool/skill relevance prompt refiner.
//!
//! Implements `aivyx_core::llm_planner::SystemPromptRefiner`
//! to inject the Phase 116 relevance section into the
//! planner's system prompt per turn. Reuses the Phase 79
//! refiner slot on `LlmPlannerConfig` — no new trait, no new
//! agent surface. The trait's Phase 117 extension
//! (`base_prompt: &str`) lets this refiner compose its
//! output as `base + addendum` without rebuilding the base
//! from scratch.
//!
//! ## Composition with Phase 79
//!
//! Only one `system_prompt_refiner` slot exists on
//! `LlmPlannerConfig`. Operators who configure BOTH Phase 79
//! adaptive Persona AND Phase 117 tool/skill relevance need a
//! composite refiner that chains both. The composite path is
//! Task 5's responsibility; this module ships the
//! standalone Phase 117 refiner.

use std::sync::Arc;

use async_trait::async_trait;

use aivyx_config::ToolRelevanceConfig;
use aivyx_core::llm_planner::SystemPromptRefiner;
use aivyx_core::SessionId;

use crate::tool_relevance_ledger::{
    render_relevance_section, PersistentToolRelevanceLedger,
};

/// Phase 117 Task 3 — wraps the Phase 116 ledger handle +
/// config into a `SystemPromptRefiner` implementation that
/// can be plugged into `LlmPlannerConfig`.
pub struct RelevancePromptRefiner {
    ledger: Arc<PersistentToolRelevanceLedger>,
    config: ToolRelevanceConfig,
}

impl RelevancePromptRefiner {
    pub fn new(
        ledger: Arc<PersistentToolRelevanceLedger>,
        config: ToolRelevanceConfig,
    ) -> Self {
        RelevancePromptRefiner { ledger, config }
    }
}

#[async_trait]
impl SystemPromptRefiner for RelevancePromptRefiner {
    async fn refine(
        &self,
        user_message: &str,
        _session_id: SessionId,
        base_prompt: &str,
    ) -> Option<String> {
        // Build the deterministic ledger key from the user
        // input + the human-readable display form.
        let max_kw = self.config.max_keywords as usize;
        let keyword_key =
            aivyx_core::relevance::keyword_key(user_message, max_kw);
        if keyword_key.is_empty() {
            return None;
        }
        // Display form replaces `|` separators with `, `.
        let keyword_key_display = keyword_key.replace('|', ", ");

        let section = render_relevance_section(
            &self.ledger,
            &keyword_key,
            &keyword_key_display,
            self.config.min_outcomes_to_show,
            self.config.top_k_per_section as usize,
        )
        .await;

        if section.trim().is_empty() {
            return None;
        }

        // Compose: base prompt + relevance addendum. If the
        // base ends without a trailing newline, normalize so
        // the two blocks stay visually separated.
        let mut composed = String::with_capacity(
            base_prompt.len() + section.len() + 2,
        );
        composed.push_str(base_prompt.trim_end());
        if !composed.is_empty() {
            composed.push_str("\n\n");
        }
        composed.push_str(section.trim_end());
        Some(composed)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_relevance_ledger::RelevanceSurfaceKind;
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};

    async fn scratch_refiner() -> (
        std::path::PathBuf,
        Arc<PersistentToolRelevanceLedger>,
    ) {
        let parent = std::env::temp_dir().join(format!(
            "aivyx-phase117-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&parent).expect("scratch dir");
        let storage: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(parent.join("store.redb")),
            MasterKey::from_raw([7u8; 32]),
        )
        .await
        .expect("scratch storage");
        let handle = storage.domain(KeyDomain::ToolRelevanceLedger);
        let ledger =
            Arc::new(PersistentToolRelevanceLedger::new(handle));
        (parent, ledger)
    }

    fn default_config() -> ToolRelevanceConfig {
        ToolRelevanceConfig {
            enabled: true,
            max_keywords: 5,
            min_outcomes_to_show: 2,
            top_k_per_section: 5,
        }
    }

    #[tokio::test]
    async fn refine_returns_none_for_empty_user_message() {
        let (_dir, ledger) = scratch_refiner().await;
        let refiner =
            RelevancePromptRefiner::new(ledger, default_config());
        let result = refiner
            .refine("", SessionId::new(), "base prompt")
            .await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn refine_returns_none_for_message_with_no_keywords() {
        let (_dir, ledger) = scratch_refiner().await;
        let refiner =
            RelevancePromptRefiner::new(ledger, default_config());
        // All stopwords + short tokens → no usable key.
        let result = refiner
            .refine("the a to it", SessionId::new(), "base")
            .await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn refine_returns_none_when_ledger_has_no_entry() {
        let (_dir, ledger) = scratch_refiner().await;
        let refiner =
            RelevancePromptRefiner::new(ledger, default_config());
        let result = refiner
            .refine(
                "research the rust codebase",
                SessionId::new(),
                "base",
            )
            .await;
        // Ledger is empty → render_relevance_section returns ""
        // → refine returns None.
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn refine_composes_base_prompt_plus_relevance_addendum() {
        let (_dir, ledger) = scratch_refiner().await;
        // Build a ledger entry the user message will key on.
        for _ in 0..3 {
            ledger
                .record_outcome(
                    // Keyword key for "research the rust codebase":
                    // tokens = research, rust, codebase → lex-sorted:
                    // codebase | research | rust
                    "codebase|research|rust",
                    RelevanceSurfaceKind::Tool,
                    "memory.read",
                    true,
                    100,
                )
                .await
                .unwrap();
        }
        let refiner = RelevancePromptRefiner::new(
            Arc::clone(&ledger),
            default_config(),
        );
        let result = refiner
            .refine(
                "research the rust codebase",
                SessionId::new(),
                "## Role: researcher\n\nDo good work.",
            )
            .await
            .expect("refined Some(...)");
        // Base preserved.
        assert!(result.contains("## Role: researcher"));
        assert!(result.contains("Do good work."));
        // Relevance section appended.
        assert!(result.contains("## Tools recently used for similar tasks"));
        assert!(result.contains("memory.read: 3 successes"));
        // Display key joined with ", " not "|".
        assert!(result.contains("Based on keywords: codebase, research, rust"));
    }

    #[tokio::test]
    async fn refine_with_empty_base_prompt_still_returns_section() {
        let (_dir, ledger) = scratch_refiner().await;
        for _ in 0..2 {
            ledger
                .record_outcome(
                    "deployment",
                    RelevanceSurfaceKind::Tool,
                    "shell.exec",
                    true,
                    100,
                )
                .await
                .unwrap();
        }
        let refiner = RelevancePromptRefiner::new(
            Arc::clone(&ledger),
            default_config(),
        );
        let result = refiner
            .refine("deployment", SessionId::new(), "")
            .await
            .expect("non-empty result");
        assert!(result.contains("## Tools recently used for similar tasks"));
        // No leading double-newline when base is empty.
        assert!(!result.starts_with("\n\n"));
    }

    #[tokio::test]
    async fn refine_respects_min_outcomes_threshold() {
        let (_dir, ledger) = scratch_refiner().await;
        // Single success; default min_outcomes_to_show = 2 → filtered out.
        ledger
            .record_outcome(
                "alpha",
                RelevanceSurfaceKind::Tool,
                "memory.read",
                true,
                100,
            )
            .await
            .unwrap();
        let refiner = RelevancePromptRefiner::new(
            Arc::clone(&ledger),
            default_config(),
        );
        let result = refiner
            .refine("alpha", SessionId::new(), "base")
            .await;
        assert!(
            result.is_none(),
            "below-threshold should yield None: {result:?}"
        );
    }
}

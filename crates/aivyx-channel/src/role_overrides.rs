//! Runtime role overrides for Phase 30 — Runtime Role Mutation.
//!
//! `RoleOverrides` captures append-only system-prompt extensions and
//! tool-allowlist mutations. A shared `Arc<RwLock<RoleOverrides>>` is
//! captured by the planner factory closure and read on each turn
//! construction, so changes take effect on the next turn without
//! session restart.

use std::sync::{Arc, RwLock};

use aivyx_core::LlmPlannerConfig;

/// Runtime mutations to the agent's role configuration.
///
/// Append-only for prompts (the original system prompt is always
/// preserved). Add/remove for the tool allowlist.
#[derive(Debug, Clone, Default)]
pub struct RoleOverrides {
    /// Text appended to the system prompt on each turn. When `None`,
    /// the original prompt is used unchanged.
    pub prompt_appendix: Option<String>,

    /// Tool names to add to the active role's allowlist.
    pub allowlist_additions: Vec<String>,

    /// Tool names to remove from the active role's allowlist.
    pub allowlist_removals: Vec<String>,
}

impl RoleOverrides {
    /// Returns `true` if no overrides have been set.
    pub fn is_empty(&self) -> bool {
        self.prompt_appendix.is_none()
            && self.allowlist_additions.is_empty()
            && self.allowlist_removals.is_empty()
    }
}

/// Convenience alias for the shared handle tools and the planner
/// factory both hold.
pub type SharedRoleOverrides = Arc<RwLock<RoleOverrides>>;

/// Create a new empty shared handle.
pub fn shared_role_overrides() -> SharedRoleOverrides {
    Arc::new(RwLock::new(RoleOverrides::default()))
}

/// Apply the current overrides to a per-turn planner config clone.
///
/// Called by the planner factory closure on each turn construction.
/// - `prompt_appendix`: appended to the existing system prompt with
///   a separator. The original prompt is always preserved.
/// - `allowlist_additions`: tools added to the allowlist.
/// - `allowlist_removals`: tools removed from the allowlist.
pub fn apply_to_planner_config(overrides: &RoleOverrides, config: &mut LlmPlannerConfig) {
    if let Some(ref appendix) = overrides.prompt_appendix {
        let base = config.system_prompt.take().unwrap_or_default();
        config.system_prompt = Some(format!("{base}\n\n{appendix}"));
    }

    if !overrides.allowlist_additions.is_empty() || !overrides.allowlist_removals.is_empty() {
        let mut allowlist = config.tool_allowlist.take().unwrap_or_default();
        for name in &overrides.allowlist_additions {
            allowlist.insert(name.clone());
        }
        for name in &overrides.allowlist_removals {
            allowlist.remove(name);
        }
        config.tool_allowlist = Some(allowlist);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_overrides_are_empty() {
        let o = RoleOverrides::default();
        assert!(o.is_empty());
    }

    #[test]
    fn prompt_appendix_makes_non_empty() {
        let mut o = RoleOverrides::default();
        o.prompt_appendix = Some("Prefer short commands.".into());
        assert!(!o.is_empty());
    }

    #[test]
    fn allowlist_additions_make_non_empty() {
        let mut o = RoleOverrides::default();
        o.allowlist_additions.push("shell.exec".into());
        assert!(!o.is_empty());
    }

    #[test]
    fn shared_handle_round_trips() {
        let shared = shared_role_overrides();
        {
            let mut w = shared.write().unwrap();
            w.prompt_appendix = Some("test".into());
            w.allowlist_additions.push("fs.read".into());
            w.allowlist_removals.push("fs.delete".into());
        }
        let r = shared.read().unwrap();
        assert_eq!(r.prompt_appendix.as_deref(), Some("test"));
        assert_eq!(r.allowlist_additions, vec!["fs.read"]);
        assert_eq!(r.allowlist_removals, vec!["fs.delete"]);
    }

    #[test]
    fn apply_prompt_appendix_preserves_original() {
        let mut config = LlmPlannerConfig::new("test-model".to_string())
            .with_system_prompt("You are helpful.");
        let mut o = RoleOverrides::default();
        o.prompt_appendix = Some("Be concise.".into());
        apply_to_planner_config(&o, &mut config);
        assert_eq!(
            config.system_prompt.as_deref(),
            Some("You are helpful.\n\nBe concise.")
        );
    }

    #[test]
    fn apply_allowlist_additions_and_removals() {
        let mut existing = std::collections::BTreeSet::new();
        existing.insert("fs.read".into());
        existing.insert("net.fetch".into());
        let mut config = LlmPlannerConfig::new("test-model".to_string())
            .with_tool_allowlist(Some(existing));
        let mut o = RoleOverrides::default();
        o.allowlist_additions.push("shell.exec".into());
        o.allowlist_removals.push("net.fetch".into());
        apply_to_planner_config(&o, &mut config);
        let list = config.tool_allowlist.unwrap();
        assert!(list.contains("fs.read"));
        assert!(list.contains("shell.exec"));
        assert!(!list.contains("net.fetch"));
    }

    #[test]
    fn apply_empty_overrides_is_noop() {
        let mut config = LlmPlannerConfig::new("test-model".to_string())
            .with_system_prompt("Original.");
        let o = RoleOverrides::default();
        apply_to_planner_config(&o, &mut config);
        assert_eq!(config.system_prompt.as_deref(), Some("Original."));
    }
}

//! Runtime role overrides for Phase 30 — Runtime Role Mutation.
//!
//! `RoleOverrides` captures append-only system-prompt extensions and
//! tool-allowlist mutations. A shared `Arc<RwLock<RoleOverrides>>` is
//! captured by the planner factory closure and read on each turn
//! construction, so changes take effect on the next turn without
//! session restart.

use std::sync::{Arc, RwLock};

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
}

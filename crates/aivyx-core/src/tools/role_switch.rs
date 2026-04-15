//! `RoleSwitchTool` — Phase 14 Task 2 placeholder for the
//! sub-agent role-switching primitive (**PRODUCT.md P1**).
//!
//! ## What it does (Task 2 shape)
//!
//! Task 2 ships the **scope plumbing, tool registration, and
//! dispatch gate** for role-switching. It does **not** yet open a
//! sub-session — that's Task 3. The tool advertises itself under
//! the name `"role.switch"`, takes `{ target, task }` input,
//! declares a `role.switch:<target>` required scope from the
//! input, and relies on the turn-loop's existing scope gate to
//! produce `ToolOutcome::Denied` when the active role's envelope
//! does not hold the scope. When the scope check passes, Task 2's
//! `execute` returns a placeholder `ToolOutcome::Completed` with
//! a JSON summary saying "role-switch requested, wiring lands in
//! Task 3" — enough to validate the dispatch gate end-to-end
//! before Task 3 builds the actual sub-session machinery on top.
//!
//! ## Why the dispatch gate is free
//!
//! `Tool::required_scope(&input)` is documented as pure and is
//! called by the turn loop **before** `execute` runs. For
//! `role.switch`, the function just reads `input["target"]` and
//! returns `Scope::parse(&format!("role.switch:{target}"))`. The
//! loop's existing scope gate then intersects the derived scope
//! with the agent's effective capability set and short-circuits
//! to `ToolOutcome::Denied { scope, held }` if the intersection
//! is empty. **No explicit denial path is written inside
//! `execute`** — it falls out of the existing architecture.
//! Phase 14 Task 2's "check the active role's envelope" cut in
//! the phase doc describes a code path that didn't need to be
//! written, because the turn-loop contract already did it.
//!
//! ## Input shape
//!
//! ```json
//! { "target": "researcher", "task": "read file X and summarize" }
//! ```
//!
//! - `target`: the role name to switch into. Must name a role
//!   declared in the loaded config; validation happens in Task 3
//!   when the sub-session is constructed.
//! - `task`: the initial user message the child's planner sees.
//!   Q2's "clean-slate child conversation history" decision
//!   means nothing else crosses the boundary.
//!
//! ## Task 3 forward-pointer
//!
//! Task 3 replaces the placeholder `execute` body with either
//! (a) an inline sub-session run (streak-preserving baseline) or
//! (b) a signal back to the session layer via an additive
//! `TurnOutcome::SwitchRoleRequested` variant (fallback path).
//! See PHASE_14.md §"Streaks at risk" and Task 3 for details.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;

use crate::{Tool, ToolContext, ToolId, ToolOutcome, Verification};

/// The Phase 14 Task 2 placeholder implementation of the
/// `role.switch` tool. See the module docs for the full picture.
#[derive(Debug)]
pub struct RoleSwitchTool {
    id: ToolId,
    schema: Value,
}

impl Default for RoleSwitchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl RoleSwitchTool {
    pub fn new() -> Self {
        RoleSwitchTool {
            id: ToolId::new(),
            schema: role_switch_input_schema(),
        }
    }
}

#[async_trait]
impl Tool for RoleSwitchTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "role.switch"
    }

    fn description(&self) -> &str {
        "Switch the agent into a child role for a bounded sub-session. \
         Input is a JSON object with two string fields: `target` (the \
         role name to switch into) and `task` (the initial user message \
         the child's planner sees). The child runs under the target \
         role's attenuated capability envelope per PRODUCT.md P1; when \
         the sub-task completes, the agent returns to the parent role. \
         The child's capability envelope is computed by \
         `assemble_role_envelope` from the parent's chain, so a child \
         cannot hold any scope the parent did not transitively grant it."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, input: &Value) -> Scope {
        // Pull the target role name from input. If it's missing or
        // not a string, return a deny scope no role can hold so the
        // loop's scope gate produces a clean `Denied` with the
        // malformed-input base distinguishable from a policy
        // denial. Same pattern as fs.rs's `deny_scope_for`.
        let Some(target) = input.get("target").and_then(|v| v.as_str()) else {
            return role_switch_deny_scope();
        };

        // A target that contains `:` or other characters that
        // would confuse scope parsing collapses to the deny scope.
        // Role names in Aivyx configs are bare identifiers; a
        // dispatched call with a `:` in the target is either a
        // malformed planner output or an injection attempt, and
        // the right response is "deny, don't parse-pun."
        if target.contains(':') || target.is_empty() {
            return role_switch_deny_scope();
        }

        Scope::parse(&format!("role.switch:{target}"))
            .unwrap_or_else(role_switch_deny_scope)
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        // Task 2 placeholder: the scope gate has already passed by
        // the time we get here. Task 3 will replace this body with
        // the real sub-session opener. For Task 2, return a
        // Completed outcome with a diagnostic payload so the
        // turn-loop test can assert "gate allowed, placeholder
        // fired, wiring pending."
        let target = input
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let task = input
            .get("task")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        ToolOutcome::Completed {
            output: json!({
                "status": "role-switch requested, wiring lands in Phase 14 Task 3",
                "target": target,
                "task": task,
            }),
            // Task 2 hasn't actually switched anything, so there is
            // no effect to verify. NotApplicable is the right
            // variant — Verified would be a lie, Unverified would
            // imply there's a real effect that we failed to check.
            verified: Verification::NotApplicable,
        }
    }
}

/// Produce a deny scope for malformed `role.switch` input. The
/// qualifier is a reserved sentinel no real role name can collide
/// with (contains a `/` which role names forbid), so any
/// CapabilitySet that does not contain this exact sentinel denies.
/// Mirrors `fs.rs::deny_scope_for`'s pattern.
fn role_switch_deny_scope() -> Scope {
    // The deny scope parses cleanly (base `role.switch` is in
    // KNOWN_BASES and the qualifier isn't the forbidden `*`), and
    // the sentinel is long + contains `/` so no operator config
    // would ever name a role this. The `/` also pushes dispatch
    // through `QualifierKind::PathGlob`, which means an operator
    // who somehow wanted to grant it would need an explicit
    // glob match — deny-by-default is preserved.
    Scope::parse("role.switch:__aivyx_deny__/invalid-input")
        .expect("role.switch deny scope must parse")
}

fn role_switch_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "target": {
                "type": "string",
                "description": "Role name to switch into. Must be a role \
                    declared in the loaded config; must not contain a \
                    colon. The child runs under the target role's \
                    attenuated capability envelope per PRODUCT.md P1."
            },
            "task": {
                "type": "string",
                "description": "Initial user message the child's planner \
                    sees. Q2's clean-slate decision means the parent's \
                    conversation history is NOT threaded through."
            }
        },
        "required": ["target", "task"]
    })
}

#[cfg(test)]
mod tests {
    //! Pure-fn tests for `RoleSwitchTool`. These pin the
    //! `required_scope` contract — the dispatch gate's input,
    //! essentially — without requiring a full turn-loop setup. A
    //! separate turn-loop test (in `planner.rs` or a session-layer
    //! test) pins the end-to-end allowed/denied behavior against a
    //! real `CapabilitySet`.

    use super::*;
    use aivyx_capability::{CapabilitySet, Scope};

    fn scope(s: &str) -> Scope {
        Scope::parse(s).expect("test scope must parse")
    }

    #[test]
    fn tool_name_and_id_are_stable() {
        let t = RoleSwitchTool::new();
        assert_eq!(t.name(), "role.switch");
        assert_ne!(t.id().0, uuid::Uuid::nil());
    }

    #[test]
    fn required_scope_reads_target_and_produces_qualified_scope() {
        let t = RoleSwitchTool::new();
        let input = json!({ "target": "researcher", "task": "summarize" });
        let s = t.required_scope(&input);
        assert_eq!(s.base(), "role.switch");
        assert_eq!(s.qualifier(), Some("researcher"));
    }

    #[test]
    fn required_scope_denies_missing_target() {
        let t = RoleSwitchTool::new();
        let input = json!({ "task": "no target here" });
        let s = t.required_scope(&input);
        assert_eq!(
            s.qualifier(),
            Some("__aivyx_deny__/invalid-input"),
            "malformed input must produce the reserved deny sentinel"
        );
    }

    #[test]
    fn required_scope_denies_colon_in_target() {
        // A `:` in the target would pun against scope parsing:
        // `role.switch:evil:researcher` would parse as qualifier
        // `evil:researcher`, which SimpleGlob would then treat as
        // an arbitrary string. Closing that hole at required_scope
        // time is cheaper than teaching the dispatch gate about
        // composite qualifiers.
        let t = RoleSwitchTool::new();
        let input = json!({ "target": "evil:researcher", "task": "x" });
        let s = t.required_scope(&input);
        assert_eq!(s.qualifier(), Some("__aivyx_deny__/invalid-input"));
    }

    #[test]
    fn scope_gate_denies_when_role_lacks_target_scope() {
        // A `coder` role holding `role.switch:researcher` asking
        // to switch into `scribe` (which it did not declare). The
        // derived needed scope is `role.switch:scribe`, and the
        // held set's `role.switch:researcher` does NOT grant it
        // (SimpleGlob equality mismatch, Rule 3). The CapabilitySet
        // intersection therefore reports no grant.
        let held =
            CapabilitySet::from_scopes([scope("role.switch:researcher")]);
        let t = RoleSwitchTool::new();
        let input = json!({ "target": "scribe", "task": "x" });
        let needed = t.required_scope(&input);
        assert!(
            !held.grants(&needed),
            "role holding role.switch:researcher must not be granted \
             role.switch:scribe via the scope gate"
        );
    }

    #[test]
    fn scope_gate_allows_when_role_holds_matching_qualified_scope() {
        let held =
            CapabilitySet::from_scopes([scope("role.switch:researcher")]);
        let t = RoleSwitchTool::new();
        let input = json!({ "target": "researcher", "task": "x" });
        let needed = t.required_scope(&input);
        assert!(held.grants(&needed));
    }

    #[test]
    fn scope_gate_allows_when_role_holds_unqualified_role_switch() {
        // Rule 2: held unqualified grants qualified needed. The
        // `default` root role declaring unqualified `role.switch`
        // lets descendants declare qualified forms and have them
        // inherit cleanly under the assemble_role_envelope walk.
        let held = CapabilitySet::from_scopes([scope("role.switch")]);
        let t = RoleSwitchTool::new();
        let input = json!({ "target": "anything", "task": "x" });
        let needed = t.required_scope(&input);
        assert!(held.grants(&needed));
    }
}

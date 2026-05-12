//! `ToolProxy` — implements `aivyx_core::Tool` by delegating to a
//! [`ToolProcessBridge`].
//!
//! One proxy per registered tool. A single bridge may back many
//! proxies (one tool process can register many tools). The proxy
//! is what the daemon registers into `ToolRegistry`; from the turn
//! loop's perspective, it is indistinguishable from an in-tree tool.
//!
//! Phase 49 — Tool Process IPC Foundation (P12). Foundation phase
//! ships the third-party path; first-party in-process unification
//! is deferred.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::bridge::{InvocationOutcome, ToolProcessBridge};
use crate::wire::Verification as WireVerification;

/// Bridges one registered tool (one `ToolDescriptor`) onto the
/// `aivyx_core::Tool` trait.
pub struct ToolProxy {
    id: ToolId,
    /// Public name as the planner sees it — matches the tool
    /// process's `ToolDescriptor.name`.
    name: String,
    description: String,
    input_schema: Value,
    /// Pre-parsed `Scope` used for every invocation. Falls back to
    /// a fresh parse on each call if the daemon decided to honor
    /// an operator-supplied override. The Scope is captured at
    /// registration time so the planner does not have to re-parse
    /// on every dispatch.
    required_scope: Scope,
    /// Shared handle on the underlying bridge. The bridge owns the
    /// child process and the reader task; many proxies may share
    /// one bridge.
    bridge: Arc<ToolProcessBridge>,
}

impl ToolProxy {
    /// Construct a proxy from a descriptor + shared bridge.
    ///
    /// Returns `None` if `required_scope` does not parse — callers
    /// should log + skip the tool rather than crashing.
    pub fn new(
        bridge: Arc<ToolProcessBridge>,
        name: String,
        description: String,
        input_schema: Value,
        required_scope_str: &str,
    ) -> Option<Self> {
        let required_scope = Scope::parse(required_scope_str)?;
        Some(ToolProxy {
            id: ToolId::new(),
            name,
            description,
            input_schema,
            required_scope,
            bridge,
        })
    }

    /// Same as [`Self::new`] but takes an operator-supplied
    /// override scope. The caller is responsible for verifying
    /// the override is `is_granted_by(declared)` — the bridge's
    /// loader does that check.
    pub fn with_override_scope(
        bridge: Arc<ToolProcessBridge>,
        name: String,
        description: String,
        input_schema: Value,
        scope: Scope,
    ) -> Self {
        ToolProxy {
            id: ToolId::new(),
            name,
            description,
            input_schema,
            required_scope: scope,
            bridge,
        }
    }
}

#[async_trait]
impl Tool for ToolProxy {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn input_schema(&self) -> &Value {
        &self.input_schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        self.required_scope.clone()
    }

    async fn execute(&self, input: Value, context: &ToolContext<'_>) -> ToolOutcome {
        // The turn loop has already enforced the capability check
        // and the role allowlist before we get here. Our job is to
        // round-trip the invocation through the bridge and map the
        // result back into a ToolOutcome.

        // Cancellation: if the turn was already cancelled before
        // we started, short-circuit. Mid-invocation cancellation is
        // wired below via tokio::select.
        if context.cancellation.is_cancelled() {
            return ToolOutcome::Failed(AivyxError::Cancelled);
        }

        let turn_id = context.turn_id.to_string();
        let bridge = Arc::clone(&self.bridge);
        let tool_name = self.name.clone();

        // Issue the invocation. If cancellation fires while the
        // call is in flight, send CancelInvocation upstream and
        // bail.
        let invoke_future = bridge.invoke(&tool_name, input, &turn_id);
        let cancelled = context.cancellation.cancelled();

        let outcome = tokio::select! {
            biased;
            _ = cancelled => {
                // Best-effort: the bridge does not currently expose
                // the auto-generated call_id to the caller, so we
                // can't send a targeted CancelInvocation here yet.
                // The dropped invoke_future causes the pending slot
                // to be cleaned up when the child eventually
                // responds. Wiring a per-call cancellation hook is
                // a deferred refinement.
                return ToolOutcome::Failed(AivyxError::Cancelled);
            }
            res = invoke_future => res,
        };

        match outcome {
            Ok(InvocationOutcome::Completed { verified, output }) => {
                ToolOutcome::Completed {
                    output,
                    verified: map_verification(verified),
                }
            }
            Ok(InvocationOutcome::ToolError { code, message }) => {
                ToolOutcome::Failed(AivyxError::Internal(format!(
                    "tool `{tool_name}` returned error [{code}]: {message}"
                )))
            }
            Err(e) => ToolOutcome::Failed(AivyxError::Internal(format!(
                "tool bridge error for `{tool_name}`: {e}"
            ))),
        }
    }
}

fn map_verification(v: WireVerification) -> Verification {
    match v {
        WireVerification::Verified => Verification::Verified,
        WireVerification::Unverified => Verification::Unverified,
        WireVerification::NotApplicable => Verification::NotApplicable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_verification_covers_all_variants() {
        assert!(matches!(
            map_verification(WireVerification::Verified),
            Verification::Verified
        ));
        assert!(matches!(
            map_verification(WireVerification::Unverified),
            Verification::Unverified
        ));
        assert!(matches!(
            map_verification(WireVerification::NotApplicable),
            Verification::NotApplicable
        ));
    }
}

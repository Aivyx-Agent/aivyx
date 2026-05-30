//! Multi-tool IPC harness — Phase 125 mirror of the Phase 123
//! aivyx-gmail implementation.
//!
//! Phase 123 surfaced the SDK gap that
//! [`aivyx_tool::run_tool_as_subprocess`] is single-tool and
//! every third-party tool process serving multiple tools has
//! to reimplement the dispatch. Phase 123's exit doc
//! recommended lifting `run_tools_as_subprocess` into
//! `aivyx-tool` as substrate; that lift hasn't shipped yet
//! at Phase 125 open, so this harness is a second duplicated
//! copy (after aivyx-gmail's). The duplication is the cost of
//! deferring the lift; documented honestly here so the next
//! phase that consumes the substrate (or lifts it) sees the
//! precedent.
//!
//! When the lift lands, both `crates/aivyx-gmail/src/harness.rs`
//! and `crates/aivyx-toolkit/src/harness.rs` collapse to
//! one-line re-exports of the lifted helper. Until then, this
//! file is the exact same shape as `aivyx-gmail`'s harness
//! with the `tool_process_name` updated to identify this
//! binary.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;
use tokio::io::{stdin, stdout};
use tokio::sync::Mutex;

use aivyx_capability::TrustTier;
use aivyx_core::{
    AgentId, CancellationToken, ChannelContext, ChannelError, ChannelPlatform,
    NullAuditHook, SessionId, StreamEvent, Tool, ToolContext, ToolOutcome,
    TurnId, TurnOutcome, Verification,
};
use aivyx_tool::{
    frame::{read_frame, write_frame, FrameError},
    wire::{
        DaemonToTool, ToolDescriptor, ToolEventPayload, ToolToDaemon,
        Verification as WireVerification,
    },
};

#[derive(Debug, Error)]
pub enum HarnessError {
    #[error("framing error: {0}")]
    Frame(#[from] FrameError),
    #[error("expected ToolHello, got {0:?}")]
    HandshakeUnexpected(Box<DaemonToTool>),
    #[error("unexpected EOF {0}")]
    UnexpectedEof(&'static str),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("tool list must contain at least one tool")]
    EmptyToolList,
    #[error("duplicate tool name {0:?} in tool list — names must be unique within a process")]
    DuplicateToolName(String),
}

/// Run multiple [`aivyx_core::Tool`] impls as a single tool
/// process. Same protocol as
/// [`aivyx_tool::run_tool_as_subprocess`] but the
/// `ToolRegister` frame carries `Vec<ToolDescriptor>` and the
/// dispatch loop routes each `InvokeTool { tool_name, ... }`
/// to the matching tool by name.
///
/// Invariants enforced on entry:
/// - `tools` must be non-empty.
/// - Tool names must be unique within `tools` (Gmail registers
///   `gmail.search`, `gmail.read`, etc — no duplicates by
///   construction, but the harness rejects defensively).
pub async fn run_multi_tool_subprocess(
    tools: Vec<Arc<dyn Tool>>,
    tool_process_name: impl Into<String>,
) -> Result<(), HarnessError> {
    if tools.is_empty() {
        return Err(HarnessError::EmptyToolList);
    }

    // Build the name-keyed dispatch map up front; rejecting
    // duplicates at this stage means the IPC loop never has
    // to disambiguate.
    let mut by_name: HashMap<String, Arc<dyn Tool>> = HashMap::new();
    for tool in &tools {
        let name = tool.name().to_string();
        if by_name.contains_key(&name) {
            return Err(HarnessError::DuplicateToolName(name));
        }
        by_name.insert(name, Arc::clone(tool));
    }

    let mut stdin = stdin();
    let stdout_sink = Arc::new(Mutex::new(stdout()));

    // Handshake — read ToolHello.
    let body = read_frame(&mut stdin)
        .await
        .map_err(HarnessError::Frame)?
        .ok_or(HarnessError::UnexpectedEof("during handshake"))?;
    let hello: DaemonToTool = serde_json::from_str(&body)
        .map_err(|e| HarnessError::Decode(e.to_string()))?;
    if !matches!(hello, DaemonToTool::ToolHello { .. }) {
        return Err(HarnessError::HandshakeUnexpected(Box::new(hello)));
    }

    // Build + send ToolRegister with every tool's descriptor.
    let descriptors: Vec<ToolDescriptor> = tools
        .iter()
        .map(|t| ToolDescriptor {
            name: t.name().to_string(),
            description: t.description().to_string(),
            input_schema: t.input_schema().clone(),
            required_scope: t
                .required_scope(&serde_json::json!({}))
                .to_string(),
        })
        .collect();
    let register = ToolToDaemon::ToolRegister {
        tool_process_name: tool_process_name.into(),
        tools: descriptors,
    };
    {
        let mut guard = stdout_sink.lock().await;
        write_frame(&mut *guard, &register)
            .await
            .map_err(HarnessError::Frame)?;
    }

    // Dispatch loop.
    let pending_cancels: Arc<Mutex<HashMap<String, CancellationToken>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let by_name = Arc::new(by_name);

    loop {
        let body = match read_frame(&mut stdin).await {
            Ok(Some(b)) => b,
            Ok(None) => return Ok(()),
            Err(e) => return Err(HarnessError::Frame(e)),
        };
        let msg: DaemonToTool = match serde_json::from_str(&body) {
            Ok(m) => m,
            Err(_) => continue, // unknown variant — forward-compat skip
        };
        match msg {
            DaemonToTool::ToolHello { .. } => {
                // Spurious — handshake already consumed it.
            }
            DaemonToTool::InvokeTool {
                call_id,
                tool_name,
                input,
                turn_id: _,
            } => {
                let by_name = Arc::clone(&by_name);
                let stdout_sink = Arc::clone(&stdout_sink);
                let pending_cancels = Arc::clone(&pending_cancels);
                let cancellation = CancellationToken::new();
                {
                    let mut guard = pending_cancels.lock().await;
                    guard.insert(call_id.clone(), cancellation.clone());
                }
                let call_id_for_task = call_id.clone();
                tokio::spawn(async move {
                    let reply = match by_name.get(&tool_name) {
                        Some(tool) => {
                            let channel = NoopChannel {
                                session: SessionId::new(),
                                cancellation: cancellation.clone(),
                                _call_id: call_id_for_task.clone(),
                                _stdout_sink: Arc::clone(&stdout_sink),
                            };
                            let audit = NullAuditHook;
                            let ctx = ToolContext {
                                agent_id: AgentId::new(),
                                session_id: SessionId::new(),
                                turn_id: TurnId::new(),
                                channel: &channel,
                                audit: &audit,
                                cancellation: &cancellation,
                            };
                            let outcome = tool.execute(input, &ctx).await;
                            outcome_to_wire(call_id_for_task.clone(), outcome)
                        }
                        None => ToolToDaemon::ToolError {
                            call_id: call_id_for_task.clone(),
                            code: "unknown_tool".into(),
                            message: format!(
                                "tool {tool_name:?} not registered by this process"
                            ),
                        },
                    };
                    {
                        let mut guard = stdout_sink.lock().await;
                        let _ = write_frame(&mut *guard, &reply).await;
                    }
                    pending_cancels.lock().await.remove(&call_id_for_task);
                });
            }
            DaemonToTool::CancelInvocation { call_id } => {
                let guard = pending_cancels.lock().await;
                if let Some(tok) = guard.get(&call_id) {
                    tok.cancel();
                }
            }
            DaemonToTool::ToolShutdown => return Ok(()),
        }
    }
}

/// No-op `ChannelContext` for tool-side dispatch. Phase 125's
/// toolkit tools don't stream progress events — each is a
/// single HTTP call or file operation. A streaming Chapter G
/// integration (e.g. a future long-running task) would replace
/// this with a real channel that ships
/// `ToolEventPayload::OutputChunk` frames back to the daemon.
struct NoopChannel {
    session: SessionId,
    cancellation: CancellationToken,
    _call_id: String,
    _stdout_sink: Arc<Mutex<tokio::io::Stdout>>,
}

#[async_trait]
impl ChannelContext for NoopChannel {
    fn channel_name(&self) -> &str {
        "aivyx-toolkit-harness"
    }
    fn platform(&self) -> ChannelPlatform {
        ChannelPlatform::Local
    }
    fn trust_tier(&self) -> TrustTier {
        // Daemon-side enforcement already ran before InvokeTool
        // reached us; the child tier is a placeholder here.
        TrustTier::Trusted
    }
    fn session_id(&self) -> SessionId {
        self.session
    }
    async fn stream_event(&self, _event: StreamEvent<'_>) -> Result<(), ChannelError> {
        // Gmail tools don't emit streaming progress; drop.
        Ok(())
    }
    async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), ChannelError> {
        // Per-tool-call channel; no turn-level finalize work.
        Ok(())
    }
    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

// ---------------------------------------------------------------
// Outcome → wire conversion. Mirrors
// `aivyx_tool::harness::outcome_to_wire` (which is private to
// that crate); duplicated here under the Phase 123 SDK-validation
// finding documented in the module preamble. A future
// generalization in aivyx-tool can absorb both call sites.
// ---------------------------------------------------------------

fn verification_to_wire(v: Verification) -> WireVerification {
    match v {
        Verification::Verified => WireVerification::Verified,
        Verification::Unverified => WireVerification::Unverified,
        Verification::NotApplicable => WireVerification::NotApplicable,
    }
}

fn outcome_to_wire(call_id: String, outcome: ToolOutcome) -> ToolToDaemon {
    match outcome {
        ToolOutcome::Completed { output, verified } => ToolToDaemon::ToolResult {
            call_id,
            verified: verification_to_wire(verified),
            output,
        },
        ToolOutcome::Denied { scope, .. } => ToolToDaemon::ToolError {
            call_id,
            code: "scope_denied".into(),
            message: format!(
                "tool refused: scope {scope} not held (this should not happen — \
                 the parent enforces capability before InvokeTool)"
            ),
        },
        ToolOutcome::NotInRole { tool_name } => ToolToDaemon::ToolError {
            call_id,
            code: "not_in_role".into(),
            message: format!("tool {tool_name} not in active role's allowlist"),
        },
        ToolOutcome::RequiresEscalation { reason } => ToolToDaemon::ToolError {
            call_id,
            code: "requires_escalation".into(),
            message: reason,
        },
        ToolOutcome::Failed(err) => ToolToDaemon::ToolError {
            call_id,
            code: "tool_failed".into(),
            message: err.to_string(),
        },
    }
}

// Suppress unused warnings on the event payload import — it's
// the public surface a future streaming Chapter F integration
// would consume from NoopChannel; pinned here so the type stays
// reachable from the binary's compile target.
#[allow(dead_code)]
fn _payload_anchor() -> ToolEventPayload {
    ToolEventPayload::Status {
        status: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;

    use aivyx_capability::Scope;
    use aivyx_core::ToolId;

    struct EchoTool {
        id: ToolId,
        name: String,
        schema: serde_json::Value,
    }

    impl EchoTool {
        fn new(name: &str) -> Self {
            Self {
                id: ToolId::new(),
                name: name.to_string(),
                schema: json!({"type":"object"}),
            }
        }
    }

    #[async_trait]
    impl Tool for EchoTool {
        fn id(&self) -> ToolId {
            self.id
        }
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "echo"
        }
        fn input_schema(&self) -> &serde_json::Value {
            &self.schema
        }
        fn required_scope(&self, _input: &serde_json::Value) -> Scope {
            Scope::parse("memory.read").unwrap()
        }
        async fn execute(
            &self,
            input: serde_json::Value,
            _ctx: &ToolContext<'_>,
        ) -> ToolOutcome {
            ToolOutcome::Completed {
                output: json!({"echoed": input, "from": self.name}),
                verified: Verification::NotApplicable,
            }
        }
    }

    #[test]
    fn outcome_to_wire_completed_yields_tool_result() {
        let outcome = ToolOutcome::Completed {
            output: json!({"k": 1}),
            verified: Verification::Verified,
        };
        match outcome_to_wire("c-1".into(), outcome) {
            ToolToDaemon::ToolResult { call_id, verified, output } => {
                assert_eq!(call_id, "c-1");
                assert!(matches!(verified, WireVerification::Verified));
                assert_eq!(output["k"], 1);
            }
            other => panic!("expected ToolResult; got {other:?}"),
        }
    }

    #[test]
    fn outcome_to_wire_failed_yields_tool_error_with_code_tool_failed() {
        let outcome = ToolOutcome::Failed(aivyx_core::AivyxError::Cancelled);
        match outcome_to_wire("c-2".into(), outcome) {
            ToolToDaemon::ToolError { call_id, code, .. } => {
                assert_eq!(call_id, "c-2");
                assert_eq!(code, "tool_failed");
            }
            other => panic!("expected ToolError; got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_tool_list_rejected_with_dedicated_error() {
        let err = run_multi_tool_subprocess(vec![], "x")
            .await
            .expect_err("must error");
        assert!(matches!(err, HarnessError::EmptyToolList));
    }

    #[tokio::test]
    async fn duplicate_tool_names_rejected() {
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(EchoTool::new("dup")),
            Arc::new(EchoTool::new("dup")),
        ];
        let err = run_multi_tool_subprocess(tools, "x")
            .await
            .expect_err("must error");
        match err {
            HarnessError::DuplicateToolName(name) => assert_eq!(name, "dup"),
            other => panic!("expected DuplicateToolName; got {other:?}"),
        }
    }
}

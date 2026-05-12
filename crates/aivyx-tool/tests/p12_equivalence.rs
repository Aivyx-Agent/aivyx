//! Phase 50 Task 5 — P12 equivalence conformance.
//!
//! Drives the same `FsReadTool` invocation through two paths:
//!
//! 1. **In-process:** direct `FsReadTool::execute(...)`.
//! 2. **Subprocess:** the same `FsReadTool`, wrapped via
//!    `run_tool_as_subprocess` in the
//!    `fs_read_subprocess_fixture` binary, driven through
//!    `ToolProcessBridge` + `ToolProxy`.
//!
//! Asserts the two `ToolOutcome` values are equivalent: same
//! variant, same JSON output, same `Verification`. That assertion
//! is the canonical proof of **PRODUCT.md P12** — first-party
//! tools speak the same protocol third-party tools speak;
//! extractable without rewriting.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use aivyx_capability::TrustTier;
use aivyx_core::{
    AgentId, AuditHook, AuditTag, CancellationToken, ChannelContext, ChannelError,
    ChannelPlatform, FsReadToolConfig, SessionId, StreamEvent, Tool, ToolContext,
    ToolOutcome, TurnId, TurnOutcome,
};
use aivyx_tool::{ToolProcessBridge, ToolProcessConfig, ToolProxy};

// ---------------------------------------------------------------------------
// Test channel + audit (mirrors the proxy_e2e fakes)
// ---------------------------------------------------------------------------

struct FakeChannel {
    session: SessionId,
    token: CancellationToken,
    #[allow(dead_code)]
    events: Mutex<Vec<String>>,
}

impl FakeChannel {
    fn new() -> Self {
        FakeChannel {
            session: SessionId::new(),
            token: CancellationToken::new(),
            events: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl ChannelContext for FakeChannel {
    fn channel_name(&self) -> &str {
        "p12-test"
    }
    fn platform(&self) -> ChannelPlatform {
        ChannelPlatform::Local
    }
    fn trust_tier(&self) -> TrustTier {
        TrustTier::Trusted
    }
    fn session_id(&self) -> SessionId {
        self.session
    }
    async fn stream_event(&self, event: StreamEvent<'_>) -> Result<(), ChannelError> {
        self.events.lock().unwrap().push(format!("{event:?}"));
        Ok(())
    }
    async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), ChannelError> {
        Ok(())
    }
    fn cancellation_token(&self) -> CancellationToken {
        self.token.clone()
    }
}

struct NullAudit;
impl AuditHook for NullAudit {
    fn on_event(&self, _tag: AuditTag) {}
}

// ---------------------------------------------------------------------------
// Scratch directory helper (same pattern as the rest of the workspace)
// ---------------------------------------------------------------------------

struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn new() -> Self {
        let base = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let unique = format!("aivyx-p12-{}", uuid::Uuid::new_v4());
        let path = PathBuf::from(base).join(unique);
        std::fs::create_dir_all(&path).expect("scratch dir mkdir");
        ScratchDir { path }
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// ---------------------------------------------------------------------------
// The proof
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fs_read_in_process_and_subprocess_produce_equivalent_outcomes() {
    let scratch = ScratchDir::new();
    let target = scratch.path.join("hello.txt");
    let content = "phase-50 conformance: extractable without rewriting\n";
    std::fs::write(&target, content).expect("seed file");

    // The fixture binary path comes from Cargo's CARGO_BIN_EXE
    // env var (set during `cargo test`).
    let fixture_path = env!("CARGO_BIN_EXE_fs_read_subprocess_fixture");

    // Build the in-process tool.
    let in_process_tool = FsReadToolConfig::new(&scratch.path)
        .build()
        .expect("FsReadTool builds against the scratch dir");

    let input = serde_json::json!({"path": target.to_string_lossy()});

    // ---- (1) In-process path ------------------------------------------
    let channel_in = FakeChannel::new();
    let audit_in = NullAudit;
    let token_in = channel_in.cancellation_token();
    let ctx_in = ToolContext {
        agent_id: AgentId::new(),
        session_id: channel_in.session,
        turn_id: TurnId::new(),
        channel: &channel_in,
        audit: &audit_in,
        cancellation: &token_in,
    };
    let in_process_outcome = in_process_tool.execute(input.clone(), &ctx_in).await;

    // ---- (2) Subprocess path ------------------------------------------
    let cfg = ToolProcessConfig {
        name: "fs.read-subprocess-fixture".into(),
        command: fixture_path.to_string(),
        args: vec![scratch.path.to_string_lossy().into_owned()],
        env: vec![],
    };
    let bridge = ToolProcessBridge::spawn(cfg)
        .await
        .expect("subprocess fixture spawn");
    let bridge = Arc::new(bridge);

    let descriptor = bridge
        .descriptors()
        .first()
        .expect("fixture must register one tool")
        .clone();
    let proxy = ToolProxy::new(
        Arc::clone(&bridge),
        descriptor.name.clone(),
        descriptor.description.clone(),
        descriptor.input_schema.clone(),
        &descriptor.required_scope,
    )
    .expect("descriptor.required_scope must parse");

    let channel_out = FakeChannel::new();
    let audit_out = NullAudit;
    let token_out = channel_out.cancellation_token();
    let ctx_out = ToolContext {
        agent_id: AgentId::new(),
        session_id: channel_out.session,
        turn_id: TurnId::new(),
        channel: &channel_out,
        audit: &audit_out,
        cancellation: &token_out,
    };
    let subprocess_outcome = proxy.execute(input, &ctx_out).await;

    let _ = bridge.shutdown().await;

    // ---- The assertion ------------------------------------------------
    // Both outcomes must be Completed with the same output and
    // the same Verification. That equivalence is P12's
    // "extractable without rewriting" property.
    match (&in_process_outcome, &subprocess_outcome) {
        (
            ToolOutcome::Completed { output: o1, verified: v1 },
            ToolOutcome::Completed { output: o2, verified: v2 },
        ) => {
            assert_eq!(
                o1, o2,
                "in-process and subprocess outputs must be identical; \
                 in-process={o1:?}, subprocess={o2:?}",
            );
            // Verification round-trips through Wire form across the
            // process boundary — both ends are aivyx_core::Verification
            // so equality is structural.
            assert_eq!(
                format!("{v1:?}"),
                format!("{v2:?}"),
                "Verification must be identical",
            );
        }
        _ => panic!(
            "expected both Completed; in-process={in_process_outcome:?}, \
             subprocess={subprocess_outcome:?}",
        ),
    }
}

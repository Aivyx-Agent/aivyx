//! End-to-end test for `ToolProxy`: spawn an inline Python tool
//! process, register its single tool, dispatch via the proxy as
//! the turn loop would, assert the round-trip.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use aivyx_capability::TrustTier;
use aivyx_core::{
    AgentId, AuditHook, AuditTag, CancellationToken, ChannelContext, ChannelError,
    ChannelPlatform, SessionId, StreamEvent, Tool, ToolContext, ToolOutcome, TurnId,
    TurnOutcome,
};
use aivyx_tool::{ToolProcessBridge, ToolProcessConfig, ToolProxy};

const PYTHON_TOOL: &str = r#"
import sys, json, struct

def read_frame():
    hdr = sys.stdin.buffer.read(4)
    if not hdr or len(hdr) < 4:
        return None
    (n,) = struct.unpack(">I", hdr)
    return json.loads(sys.stdin.buffer.read(n).decode("utf-8"))

def write_frame(msg):
    body = json.dumps(msg).encode("utf-8")
    sys.stdout.buffer.write(struct.pack(">I", len(body)) + body)
    sys.stdout.buffer.flush()

# Handshake
hello = read_frame()
assert hello["type"] == "ToolHello"
write_frame({
    "type": "ToolRegister",
    "tool_process_name": "wordcount-tool",
    "tools": [{
        "name": "wordcount",
        "description": "Count words in a string.",
        "input_schema": {
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
        },
        "required_scope": "memory.read",
    }]
})

# Serve one invocation, then exit.
inv = read_frame()
text = inv["input"]["text"]
write_frame({
    "type": "ToolResult",
    "call_id": inv["call_id"],
    "verified": "NotApplicable",
    "output": {"words": len(text.split()), "chars": len(text)}
})
sys.exit(0)
"#;

// ---------------------------------------------------------------------------
// Test channel fake (mirrors aivyx-core::tools::web_fetch::CapturingChannel)
// ---------------------------------------------------------------------------

struct FakeChannel {
    session: SessionId,
    token: CancellationToken,
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
        "test"
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

// ---------------------------------------------------------------------------
// Null audit hook
// ---------------------------------------------------------------------------

struct NullAudit;
impl AuditHook for NullAudit {
    fn on_event(&self, _tag: AuditTag) {}
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn spawn_inline_python() -> Option<Arc<ToolProcessBridge>> {
    let config = ToolProcessConfig {
        name: "wordcount".into(),
        command: "python3".into(),
        args: vec!["-c".into(), PYTHON_TOOL.into()],
        env: vec![],
    };
    match ToolProcessBridge::spawn(config).await {
        Ok(b) => Some(Arc::new(b)),
        Err(e) => {
            eprintln!("skipping: python3 unavailable: {e}");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn proxy_completes_round_trip_against_python_tool() {
    let Some(bridge) = spawn_inline_python().await else {
        return;
    };

    let descriptor = bridge.descriptors()[0].clone();
    let proxy = ToolProxy::new(
        Arc::clone(&bridge),
        descriptor.name.clone(),
        descriptor.description.clone(),
        descriptor.input_schema.clone(),
        &descriptor.required_scope,
    )
    .expect("required_scope must parse");

    assert_eq!(proxy.name(), "wordcount");
    assert_eq!(proxy.description(), "Count words in a string.");

    // Build a ToolContext as the turn loop would.
    let channel = FakeChannel::new();
    let audit = NullAudit;
    let session = channel.session;
    let token = channel.cancellation_token();
    let ctx = ToolContext {
        agent_id: AgentId::new(),
        session_id: session,
        turn_id: TurnId::new(),
        channel: &channel,
        audit: &audit,
        cancellation: &token,
    };

    let input = serde_json::json!({"text": "hello phase 49"});
    let outcome = proxy.execute(input, &ctx).await;

    match outcome {
        ToolOutcome::Completed { output, verified } => {
            assert_eq!(output["words"], 3);
            assert_eq!(output["chars"], "hello phase 49".len());
            assert!(matches!(
                verified,
                aivyx_core::Verification::NotApplicable
            ));
        }
        other => panic!("expected Completed, got {other:?}"),
    }
}

#[tokio::test]
async fn proxy_with_unparseable_scope_returns_none() {
    let Some(bridge) = spawn_inline_python().await else {
        return;
    };
    let descriptor = bridge.descriptors()[0].clone();
    let result = ToolProxy::new(
        bridge,
        descriptor.name,
        descriptor.description,
        descriptor.input_schema,
        "not.a.real.scope.base",
    );
    assert!(result.is_none(), "unparseable scope must yield None");
}

#[tokio::test]
async fn proxy_returns_failed_when_already_cancelled() {
    let Some(bridge) = spawn_inline_python().await else {
        return;
    };
    let descriptor = bridge.descriptors()[0].clone();
    let proxy = ToolProxy::new(
        Arc::clone(&bridge),
        descriptor.name.clone(),
        descriptor.description.clone(),
        descriptor.input_schema.clone(),
        &descriptor.required_scope,
    )
    .unwrap();

    let channel = FakeChannel::new();
    let audit = NullAudit;
    let token = channel.cancellation_token();
    // Pre-cancel.
    token.cancel();

    let ctx = ToolContext {
        agent_id: AgentId::new(),
        session_id: channel.session,
        turn_id: TurnId::new(),
        channel: &channel,
        audit: &audit,
        cancellation: &token,
    };
    let outcome = proxy
        .execute(serde_json::json!({"text": "x"}), &ctx)
        .await;
    assert!(
        matches!(
            outcome,
            ToolOutcome::Failed(aivyx_core::AivyxError::Cancelled)
        ),
        "expected Failed(Cancelled), got {outcome:?}"
    );
}


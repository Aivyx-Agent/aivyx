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
        sandbox: None,
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
        message_origin: aivyx_core::MessageOrigin::Operator,
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
        message_origin: aivyx_core::MessageOrigin::Operator,
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


// ---------------------------------------------------------------------------
// Phase 50 — ToolEvent → channel relay
// ---------------------------------------------------------------------------

const PYTHON_TOOL_WITH_EVENTS: &str = r#"
import sys, json, struct

def read_frame():
    hdr = sys.stdin.buffer.read(4)
    if not hdr or len(hdr) < 4: return None
    (n,) = struct.unpack(">I", hdr)
    return json.loads(sys.stdin.buffer.read(n).decode("utf-8"))

def write_frame(msg):
    body = json.dumps(msg).encode("utf-8")
    sys.stdout.buffer.write(struct.pack(">I", len(body)) + body)
    sys.stdout.buffer.flush()

assert read_frame()["type"] == "ToolHello"
write_frame({"type":"ToolRegister","tool_process_name":"event-tool","tools":[
    {"name":"streamy","description":"emits chunks.","input_schema":{"type":"object"},"required_scope":"memory.read"}
]})

inv = read_frame()
# Emit two OutputChunks plus a Status, then the terminal result.
write_frame({"type":"ToolEvent","call_id":inv["call_id"],
             "event":{"kind":"Status","status":"thinking..."}})
write_frame({"type":"ToolEvent","call_id":inv["call_id"],
             "event":{"kind":"OutputChunk","chunk":"part-1 "}})
write_frame({"type":"ToolEvent","call_id":inv["call_id"],
             "event":{"kind":"OutputChunk","chunk":"part-2"}})
write_frame({"type":"ToolResult","call_id":inv["call_id"],
             "verified":"NotApplicable","output":{"done":True}})
sys.exit(0)
"#;

#[tokio::test]
async fn proxy_relays_tool_events_to_channel() {
    let config = ToolProcessConfig {
        name: "events".into(),
        command: "python3".into(),
        args: vec!["-c".into(), PYTHON_TOOL_WITH_EVENTS.into()],
        env: vec![],
        sandbox: None,
    };
    let bridge = match ToolProcessBridge::spawn(config).await {
        Ok(b) => Arc::new(b),
        Err(_) => return,
    };
    let descriptor = bridge.descriptors()[0].clone();
    let proxy = ToolProxy::new(
        Arc::clone(&bridge),
        descriptor.name.clone(),
        descriptor.description.clone(),
        descriptor.input_schema.clone(),
        &descriptor.required_scope,
    )
    .expect("scope parses");

    let channel = FakeChannel::new();
    let audit = NullAudit;
    let token = channel.cancellation_token();
    let ctx = ToolContext {
        agent_id: AgentId::new(),
        session_id: channel.session,
        turn_id: TurnId::new(),
        channel: &channel,
        audit: &audit,
        cancellation: &token,
        message_origin: aivyx_core::MessageOrigin::Operator,
    };

    let outcome = proxy.execute(serde_json::json!({"x": 1}), &ctx).await;
    assert!(matches!(outcome, ToolOutcome::Completed { .. }));

    let captured = channel.events.lock().unwrap().clone();
    // Look for Status + two ToolOutput events in order.
    let has_status = captured.iter().any(|s| s.contains("Status") && s.contains("thinking"));
    let chunks: Vec<_> = captured
        .iter()
        .filter(|s| s.contains("ToolOutput"))
        .cloned()
        .collect();
    assert!(has_status, "expected Status event, captured: {captured:?}");
    assert_eq!(chunks.len(), 2, "expected 2 ToolOutput events, captured: {captured:?}");
    assert!(chunks[0].contains("part-1"));
    assert!(chunks[1].contains("part-2"));
}

// ---------------------------------------------------------------------------
// Phase 50 — Per-call CancelInvocation
// ---------------------------------------------------------------------------

const PYTHON_TOOL_RESPECTS_CANCEL: &str = r#"
import sys, json, struct, time

def read_frame():
    hdr = sys.stdin.buffer.read(4)
    if not hdr or len(hdr) < 4: return None
    (n,) = struct.unpack(">I", hdr)
    return json.loads(sys.stdin.buffer.read(n).decode("utf-8"))

def write_frame(msg):
    body = json.dumps(msg).encode("utf-8")
    sys.stdout.buffer.write(struct.pack(">I", len(body)) + body)
    sys.stdout.buffer.flush()

assert read_frame()["type"] == "ToolHello"
write_frame({"type":"ToolRegister","tool_process_name":"slow-tool","tools":[
    {"name":"slow","description":"slow.","input_schema":{"type":"object"},"required_scope":"memory.read"}
]})

inv = read_frame()
# Wait for a CancelInvocation; if it arrives, reply with ToolError.
import select
deadline = time.time() + 3.0
saw_cancel = False
while time.time() < deadline:
    rlist, _, _ = select.select([sys.stdin.buffer], [], [], 0.1)
    if rlist:
        next_frame = read_frame()
        if next_frame and next_frame["type"] == "CancelInvocation":
            write_frame({"type":"ToolError","call_id":inv["call_id"],
                         "code":"cancelled","message":"got CancelInvocation"})
            saw_cancel = True
            break

if not saw_cancel:
    # Timeout — never got cancelled. Reply normally so the test
    # surfaces "cancellation wasn't propagated" rather than hang.
    write_frame({"type":"ToolResult","call_id":inv["call_id"],
                 "verified":"NotApplicable","output":{"cancel_received":False}})

sys.exit(0)
"#;

#[tokio::test]
async fn proxy_sends_cancel_invocation_when_token_fires() {
    let config = ToolProcessConfig {
        name: "slow".into(),
        command: "python3".into(),
        args: vec!["-c".into(), PYTHON_TOOL_RESPECTS_CANCEL.into()],
        env: vec![],
        sandbox: None,
    };
    let bridge = match ToolProcessBridge::spawn(config).await {
        Ok(b) => Arc::new(b),
        Err(_) => return,
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

    // Cancel after a short delay so the invocation is in flight.
    let token_for_cancel = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        token_for_cancel.cancel();
    });

    let ctx = ToolContext {
        agent_id: AgentId::new(),
        session_id: channel.session,
        turn_id: TurnId::new(),
        channel: &channel,
        audit: &audit,
        cancellation: &token,
        message_origin: aivyx_core::MessageOrigin::Operator,
    };

    let outcome = proxy.execute(serde_json::json!({}), &ctx).await;
    assert!(
        matches!(
            outcome,
            ToolOutcome::Failed(aivyx_core::AivyxError::Cancelled)
        ),
        "expected Failed(Cancelled), got {outcome:?}"
    );
}

// ---------------------------------------------------------------------------
// Phase 52 — Sandbox wrapper integration
// ---------------------------------------------------------------------------

const PYTHON_TOOL_INLINE_FOR_SANDBOX: &str = r#"
import sys, json, struct

def read_frame():
    hdr = sys.stdin.buffer.read(4)
    if not hdr or len(hdr) < 4: return None
    (n,) = struct.unpack(">I", hdr)
    return json.loads(sys.stdin.buffer.read(n).decode("utf-8"))

def write_frame(msg):
    body = json.dumps(msg).encode("utf-8")
    sys.stdout.buffer.write(struct.pack(">I", len(body)) + body)
    sys.stdout.buffer.flush()

assert read_frame()["type"] == "ToolHello"
write_frame({"type":"ToolRegister","tool_process_name":"sandbox-test","tools":[
    {"name":"echo","description":"Echo.","input_schema":{"type":"object"},"required_scope":"memory.read"}
]})

inv = read_frame()
write_frame({"type":"ToolResult","call_id":inv["call_id"],
             "verified":"NotApplicable","output":{"echoed":inv["input"]}})
sys.exit(0)
"#;

/// Phase 52 — the wrapper layer works end-to-end. Uses POSIX
/// `env` as a no-op wrapper so this test runs anywhere
/// `cargo test` runs, without depending on bwrap/firejail/docker
/// being installed.
#[tokio::test]
async fn sandbox_wrapper_passes_through_stdio_end_to_end() {
    use aivyx_tool::SandboxConfig;

    let config = ToolProcessConfig {
        name: "wrapped-echo".into(),
        command: "python3".into(),
        args: vec!["-c".into(), PYTHON_TOOL_INLINE_FOR_SANDBOX.into()],
        env: vec![],
        sandbox: Some(SandboxConfig {
            wrapper: "env".into(),
            // env [NAME=VALUE...] COMMAND ARGS... — completely
            // transparent: just sets an env var and execs the
            // command. Universal POSIX shape; proves the
            // wrapper-then-command spawn path works without
            // depending on a real sandbox tool.
            args: vec!["AIVYX_SANDBOX_PROBE=1".into()],
        }),
    };

    let bridge = match ToolProcessBridge::spawn(config).await {
        Ok(b) => Arc::new(b),
        Err(e) => {
            eprintln!("skipping: spawn failed (python3 or env missing): {e}");
            return;
        }
    };
    let descriptor = bridge.descriptors()[0].clone();
    let proxy = ToolProxy::new(
        Arc::clone(&bridge),
        descriptor.name.clone(),
        descriptor.description.clone(),
        descriptor.input_schema.clone(),
        &descriptor.required_scope,
    )
    .expect("scope parses");

    let channel = FakeChannel::new();
    let audit = NullAudit;
    let token = channel.cancellation_token();
    let ctx = ToolContext {
        agent_id: AgentId::new(),
        session_id: channel.session,
        turn_id: TurnId::new(),
        channel: &channel,
        audit: &audit,
        cancellation: &token,
        message_origin: aivyx_core::MessageOrigin::Operator,
    };

    let outcome = proxy
        .execute(serde_json::json!({"wrapped": true}), &ctx)
        .await;

    match outcome {
        ToolOutcome::Completed { output, .. } => {
            assert_eq!(
                output["echoed"]["wrapped"], true,
                "wrapped tool must round-trip input/output identically",
            );
        }
        other => panic!("expected Completed, got {other:?}"),
    }
}

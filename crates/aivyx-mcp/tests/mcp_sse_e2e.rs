//! MCP non-stdio transport integration tests — Phase 32 Task 5.
//!
//! Exercises the `McpServerBridge` + `McpToolProxy` stack over a
//! channel-backed `McpTransport` mock. This proves that the transport
//! abstraction introduced in Phase 32 Task 2 works end-to-end: the
//! bridge and proxy are fully transport-agnostic.
//!
//! The channel mock simulates what `SseTransport` does (POST-equivalent
//! for send, SSE-event-equivalent for receive) without real HTTP,
//! keeping tests fast and dependency-free.

use std::sync::Arc;

use aivyx_mcp::McpServerBridge;
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Channel-backed McpTransport mock
// ---------------------------------------------------------------------------

/// Simulates a remote MCP server. The `send` method processes a
/// JSON-RPC request and pushes the response into the channel that
/// `receive` reads from.
struct ChannelTransport {
    tx: mpsc::Sender<String>,
    rx: tokio::sync::Mutex<mpsc::Receiver<String>>,
}

/// Handle a single MCP JSON-RPC request, returning the JSON response.
fn handle_jsonrpc(body: &str) -> Option<String> {
    let msg: serde_json::Value = serde_json::from_str(body).ok()?;
    let method = msg["method"].as_str()?;
    let id = msg["id"].as_u64()?;

    let resp = match method {
        "initialize" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "serverInfo": {"name": "mock-sse-mcp", "version": "0.1.0"},
                "capabilities": {"tools": {}}
            }
        }),
        "notifications/initialized" => return None,
        "tools/list" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": [
                    {
                        "name": "echo",
                        "description": "Echoes the input text back",
                        "inputSchema": {
                            "type": "object",
                            "properties": {"text": {"type": "string"}},
                            "required": ["text"]
                        }
                    },
                    {
                        "name": "add",
                        "description": "Adds two numbers",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "a": {"type": "number"},
                                "b": {"type": "number"}
                            },
                            "required": ["a", "b"]
                        }
                    }
                ]
            }
        }),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or("");
            let args = &msg["params"]["arguments"];
            match name {
                "echo" => {
                    let text = args["text"].as_str().unwrap_or("");
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{"type": "text", "text": text}],
                            "isError": false
                        }
                    })
                }
                "add" => {
                    let a = args["a"].as_f64().unwrap_or(0.0);
                    let b = args["b"].as_f64().unwrap_or(0.0);
                    let sum = (a + b) as i64;
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{"type": "text", "text": sum.to_string()}],
                            "isError": false
                        }
                    })
                }
                _ => serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": format!("unknown tool: {name}")}],
                        "isError": true
                    }
                }),
            }
        }
        "shutdown" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": null
        }),
        _ => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": format!("method not found: {method}")}
        }),
    };

    Some(serde_json::to_string(&resp).unwrap())
}

#[async_trait::async_trait]
impl aivyx_mcp::McpTransport for ChannelTransport {
    async fn send(&self, message: &str) -> Result<(), String> {
        if let Some(resp) = handle_jsonrpc(message.trim()) {
            self.tx
                .send(resp)
                .await
                .map_err(|e| format!("channel send: {e}"))
        } else {
            // Notification — no response expected.
            Ok(())
        }
    }

    async fn receive(&self) -> Result<String, String> {
        let mut rx = self.rx.lock().await;
        rx.recv()
            .await
            .ok_or_else(|| "channel closed".to_string())
    }
}

fn channel_transport() -> Arc<ChannelTransport> {
    let (tx, rx) = mpsc::channel(64);
    Arc::new(ChannelTransport {
        tx,
        rx: tokio::sync::Mutex::new(rx),
    })
}

// ---------------------------------------------------------------------------
// Tests — prove the bridge works identically over a non-stdio transport
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bridge_discovers_tools_over_channel_transport() {
    let transport = channel_transport();
    let bridge = McpServerBridge::from_transport(transport, "channel-mock")
        .await
        .expect("bridge from channel transport must start");

    let tools = bridge.discover_tools().await.expect("discover");
    assert_eq!(tools.len(), 2);

    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"echo"));
    assert!(names.contains(&"add"));

    bridge.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn bridge_calls_echo_over_channel_transport() {
    let transport = channel_transport();
    let bridge = McpServerBridge::from_transport(transport, "channel-mock")
        .await
        .expect("bridge must start");

    let result = bridge
        .call_tool("echo", serde_json::json!({"text": "hello transport"}))
        .await
        .expect("call_tool echo");

    assert!(!result.is_error);
    assert_eq!(result.content[0].text.as_deref(), Some("hello transport"));

    bridge.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn bridge_calls_add_over_channel_transport() {
    let transport = channel_transport();
    let bridge = McpServerBridge::from_transport(transport, "channel-mock")
        .await
        .expect("bridge must start");

    let result = bridge
        .call_tool("add", serde_json::json!({"a": 5, "b": 12}))
        .await
        .expect("call_tool add");

    assert!(!result.is_error);
    assert_eq!(result.content[0].text.as_deref(), Some("17"));

    bridge.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn bridge_unknown_tool_over_channel_transport() {
    let transport = channel_transport();
    let bridge = McpServerBridge::from_transport(transport, "channel-mock")
        .await
        .expect("bridge must start");

    let result = bridge
        .call_tool("nonexistent", serde_json::json!({}))
        .await
        .expect("call_tool must succeed at transport level");

    assert!(result.is_error);
    assert!(result.content[0]
        .text
        .as_deref()
        .unwrap()
        .contains("unknown tool"));

    bridge.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn bridge_server_name_over_channel_transport() {
    let transport = channel_transport();
    let bridge = McpServerBridge::from_transport(transport, "my-remote-server")
        .await
        .expect("bridge must start");

    assert_eq!(bridge.server_name(), "my-remote-server");
    bridge.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn proxy_tool_execute_over_channel_transport() {
    use aivyx_core::{
        AgentId, AuditTag, CancellationToken, ChannelContext, ChannelPlatform,
        SessionId, StreamEvent, ToolContext, ToolOutcome, TurnId, TurnOutcome,
    };

    struct NoopChannel;
    #[async_trait::async_trait]
    impl ChannelContext for NoopChannel {
        fn channel_name(&self) -> &str { "test" }
        fn platform(&self) -> ChannelPlatform { ChannelPlatform::Local }
        fn trust_tier(&self) -> aivyx_capability::TrustTier { aivyx_capability::TrustTier::Trusted }
        fn session_id(&self) -> SessionId { SessionId::new() }
        async fn stream_event(&self, _: StreamEvent<'_>) -> Result<(), aivyx_core::ChannelError> { Ok(()) }
        async fn finalize(&self, _: &TurnOutcome) -> Result<(), aivyx_core::ChannelError> { Ok(()) }
        fn cancellation_token(&self) -> CancellationToken { CancellationToken::new() }
    }

    struct NoopAudit;
    impl aivyx_core::AuditHook for NoopAudit {
        fn on_event(&self, _: AuditTag) {}
    }

    let transport = channel_transport();
    let bridge = McpServerBridge::from_transport(transport, "channel-mock")
        .await
        .expect("bridge must start");

    let tools = bridge.discover_tools().await.expect("discover");
    let echo = tools.iter().find(|t| t.name() == "echo").expect("echo");

    let cancel = CancellationToken::new();
    let channel = NoopChannel;
    let audit = NoopAudit;
    let ctx = ToolContext {
        agent_id: AgentId::new(),
        session_id: SessionId::new(),
        turn_id: TurnId::new(),
        channel: &channel,
        audit: &audit,
        cancellation: &cancel,
    };

    let outcome = echo
        .execute(serde_json::json!({"text": "transport ping"}), &ctx)
        .await;

    match outcome {
        ToolOutcome::Completed { output, .. } => {
            assert_eq!(output["text"], "transport ping");
        }
        other => panic!("expected Completed, got {other:?}"),
    }

    bridge.shutdown().await.expect("shutdown");
}

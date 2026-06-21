//! MCP client adapter integration tests — Phase 23 Task 3.
//!
//! Exercises the full stdio transport path against a mock MCP server
//! (`mock_mcp_server.py`). The mock implements `initialize`, `tools/list`,
//! and `tools/call` for two tools: `echo` and `add`.

use std::path::PathBuf;

use aivyx_mcp::McpServerBridge;

fn mock_server_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("mock_mcp_server.py")
}

#[tokio::test]
async fn bridge_discovers_tools_from_mock_server() {
    let bridge = McpServerBridge::start(
        "python3",
        &[mock_server_path().to_str().unwrap()],
        "mock",
    )
    .await
    .expect("bridge must start");

    let tools = bridge.discover_tools().await.expect("discover must succeed");
    assert_eq!(tools.len(), 2, "mock server exposes two tools");

    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"echo"), "must find echo tool");
    assert!(names.contains(&"add"), "must find add tool");

    bridge.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn bridge_lists_tool_schemas() {
    let bridge = McpServerBridge::start(
        "python3",
        &[mock_server_path().to_str().unwrap()],
        "mock",
    )
    .await
    .expect("bridge must start");

    let defs = bridge.list_tools().await.expect("list_tools");
    assert_eq!(defs.len(), 2);

    let echo = defs.iter().find(|d| d.name == "echo").expect("echo def");
    assert_eq!(echo.description.as_deref(), Some("Echoes the input text back"));
    assert!(echo.input_schema["properties"]["text"].is_object());

    bridge.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn bridge_calls_echo_tool() {
    let bridge = McpServerBridge::start(
        "python3",
        &[mock_server_path().to_str().unwrap()],
        "mock",
    )
    .await
    .expect("bridge must start");

    let result = bridge
        .call_tool("echo", serde_json::json!({"text": "hello world"}))
        .await
        .expect("call_tool echo");

    assert!(!result.is_error);
    assert_eq!(result.content.len(), 1);
    assert_eq!(result.content[0].text.as_deref(), Some("hello world"));

    bridge.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn bridge_calls_add_tool() {
    let bridge = McpServerBridge::start(
        "python3",
        &[mock_server_path().to_str().unwrap()],
        "mock",
    )
    .await
    .expect("bridge must start");

    let result = bridge
        .call_tool("add", serde_json::json!({"a": 3, "b": 7}))
        .await
        .expect("call_tool add");

    assert!(!result.is_error);
    assert_eq!(result.content[0].text.as_deref(), Some("10"));

    bridge.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn bridge_unknown_tool_returns_error() {
    let bridge = McpServerBridge::start(
        "python3",
        &[mock_server_path().to_str().unwrap()],
        "mock",
    )
    .await
    .expect("bridge must start");

    let result = bridge
        .call_tool("nonexistent", serde_json::json!({}))
        .await
        .expect("call_tool must succeed at transport level");

    assert!(result.is_error, "unknown tool must set isError");
    assert!(
        result.content[0]
            .text
            .as_deref()
            .unwrap()
            .contains("unknown tool"),
    );

    bridge.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn proxy_tool_implements_trait_correctly() {
    use aivyx_capability::Scope;

    let bridge = McpServerBridge::start(
        "python3",
        &[mock_server_path().to_str().unwrap()],
        "mock",
    )
    .await
    .expect("bridge must start");

    let tools = bridge.discover_tools().await.expect("discover");

    let echo = tools.iter().find(|t| t.name() == "echo").expect("echo tool");

    assert_eq!(echo.description(), "Echoes the input text back");
    assert!(echo.input_schema()["properties"]["text"].is_object());

    let scope = echo.required_scope(&serde_json::json!({"text": "hi"}));
    assert_eq!(
        scope,
        Scope::parse("mcp.call:mock:echo").expect("scope must parse"),
    );

    bridge.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn proxy_tool_execute_returns_completed() {
    use aivyx_core::{
        AgentId, AuditTag, CancellationToken, ChannelContext, ChannelPlatform,
        SessionId, StreamEvent, ToolContext, ToolOutcome, TurnId, TurnOutcome,
    };

    struct NoopChannel;
    #[async_trait::async_trait]
    impl ChannelContext for NoopChannel {
        fn channel_name(&self) -> &str {
            "test"
        }
        fn platform(&self) -> ChannelPlatform {
            ChannelPlatform::Local
        }
        fn trust_tier(&self) -> aivyx_capability::TrustTier {
            aivyx_capability::TrustTier::Trusted
        }
        fn session_id(&self) -> SessionId {
            SessionId::new()
        }
        async fn stream_event(&self, _event: StreamEvent<'_>) -> Result<(), aivyx_core::ChannelError> {
            Ok(())
        }
        async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), aivyx_core::ChannelError> {
            Ok(())
        }
        fn cancellation_token(&self) -> CancellationToken {
            CancellationToken::new()
        }
    }

    struct NoopAudit;
    impl aivyx_core::AuditHook for NoopAudit {
        fn on_event(&self, _tag: AuditTag) {}
    }

    let bridge = McpServerBridge::start(
        "python3",
        &[mock_server_path().to_str().unwrap()],
        "mock",
    )
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
        .execute(serde_json::json!({"text": "ping"}), &ctx)
        .await;

    match outcome {
        ToolOutcome::Completed { output, .. } => {
            assert_eq!(output["text"], "ping");
        }
        other => panic!("expected Completed, got {other:?}"),
    }

    bridge.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn server_name_accessible() {
    let bridge = McpServerBridge::start(
        "python3",
        &[mock_server_path().to_str().unwrap()],
        "test-server",
    )
    .await
    .expect("bridge must start");

    assert_eq!(bridge.server_name(), "test-server");
    bridge.shutdown().await.expect("shutdown");
}

// ---------------------------------------------------------------------------
// Phase 55 — sandbox wrapper integration
// ---------------------------------------------------------------------------

/// The Phase 55 sandbox wiring works end-to-end. Uses POSIX
/// `env` as a no-op wrapper so this test runs anywhere
/// `cargo test` runs, without depending on bwrap / firejail /
/// docker being installed.
///
/// Equivalent to running:
///
///     env AIVYX_MCP_SANDBOX_PROBE=1 python3 mock_mcp_server.py
///
/// — i.e., set a probe env var, then exec the real MCP server.
/// The bridge handshake (initialize) + tool discovery must work
/// identically to the no-sandbox path.
#[tokio::test]
async fn bridge_handshakes_through_env_sandbox_wrapper() {
    use aivyx_mcp::SandboxConfig;

    let sandbox = SandboxConfig {
        wrapper: "env".into(),
        args: vec!["AIVYX_MCP_SANDBOX_PROBE=1".into()],
    };
    let bridge = aivyx_mcp::McpServerBridge::start_with_sandbox(
        "python3",
        &[mock_server_path().to_str().unwrap()],
        &[],
        Some(&sandbox),
        "sandboxed-mock",
    )
    .await
    .expect("bridge must start through env wrapper");

    let tools = bridge
        .discover_tools()
        .await
        .expect("tool discovery must work through the wrapper");
    assert_eq!(
        tools.len(),
        2,
        "wrapper must not interfere with tool discovery (got {} tools)",
        tools.len(),
    );

    bridge.shutdown().await.expect("shutdown");
}

/// Sanity check: the `bridge.server_name()` identity is preserved
/// through the sandboxed path.
#[tokio::test]
async fn sandboxed_bridge_preserves_server_name() {
    use aivyx_mcp::SandboxConfig;

    let sandbox = SandboxConfig {
        wrapper: "env".into(),
        args: vec![],
    };
    let bridge = aivyx_mcp::McpServerBridge::start_with_sandbox(
        "python3",
        &[mock_server_path().to_str().unwrap()],
        &[],
        Some(&sandbox),
        "named-sandboxed",
    )
    .await
    .expect("bridge must start");

    assert_eq!(bridge.server_name(), "named-sandboxed");
    bridge.shutdown().await.expect("shutdown");
}

//! Chapter Brigade (BG.5) — end-to-end drive of the REAL `aivyx-kitchen-toolkit`
//! binary over the multi-tool harness IPC, against an in-process mock
//! PostgREST. Mirrors the Abacus AB.5 precedent (and `aivyx-tool`'s
//! `proxy_e2e.rs`): spawn the binary via `ToolProcessBridge`, wrap one tool in a
//! `ToolProxy`, and `execute()` it exactly as the daemon's turn loop would —
//! proving the whole path (config load → harness register → IPC invoke →
//! KitchenDB RPC → shaped result) works against a built artifact, not just unit
//! mocks. Gracefully skips if the binary can't be spawned (sandbox), like the
//! SDK's python e2e.

use std::sync::Arc;

use async_trait::async_trait;

use aivyx_capability::TrustTier;
use aivyx_core::{
    AgentId, AuditHook, AuditTag, CancellationToken, ChannelContext, ChannelError,
    ChannelPlatform, SessionId, StreamEvent, Tool, ToolContext, ToolOutcome, TurnId, TurnOutcome,
    Verification,
};
use aivyx_tool::{ToolProcessBridge, ToolProcessConfig, ToolProxy};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

// --- a one-shot mock PostgREST -------------------------------------------

async fn spawn_mock(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        if let Ok((mut sock, _)) = listener.accept().await {
            let (read_half, mut write_half) = sock.split();
            let mut reader = BufReader::new(read_half);
            let mut line = String::new();
            reader.read_line(&mut line).await.ok();
            let mut content_length = 0usize;
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).await.unwrap_or(0) == 0 || h == "\r\n" {
                    break;
                }
                if let Some(idx) = h.find(':') {
                    if h[..idx].eq_ignore_ascii_case("content-length") {
                        content_length = h[idx + 1..].trim().parse().unwrap_or(0);
                    }
                }
            }
            if content_length > 0 {
                let mut buf = vec![0u8; content_length];
                let _ = reader.read_exact(&mut buf).await;
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{}",
                body.len(),
                body,
            );
            write_half.write_all(resp.as_bytes()).await.ok();
            write_half.flush().await.ok();
        }
    });
    format!("http://127.0.0.1:{port}")
}

// --- test channel + audit fakes (mirror proxy_e2e.rs) --------------------

struct FakeChannel {
    session: SessionId,
    token: CancellationToken,
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
    async fn stream_event(&self, _event: StreamEvent<'_>) -> Result<(), ChannelError> {
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

fn scratch_home() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "aivyx-kitchen-e2e-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst),
    ));
    std::fs::create_dir_all(dir.join(".aivyx-pa/tool-processes/kitchen")).unwrap();
    dir
}

#[tokio::test]
async fn real_binary_serves_inventory_list_over_the_harness() {
    // Mock KitchenDB returns two inventory rows for get_inventory.
    let base = spawn_mock(r#"[{"sku":"TOM-01","qty":4},{"sku":"OIL-02","qty":1}]"#).await;

    // A temp HOME with a config.toml pointing at the mock.
    let home = scratch_home();
    std::fs::write(
        home.join(".aivyx-pa/tool-processes/kitchen/config.toml"),
        format!("[kitchen_db]\nbase_url = \"{base}\"\napi_key = \"K\"\norganization_id = \"ORG\"\n"),
    )
    .unwrap();

    let config = ToolProcessConfig {
        name: "kitchen".into(),
        command: env!("CARGO_BIN_EXE_aivyx-kitchen-toolkit").into(),
        args: vec![],
        env: vec![("HOME".into(), home.to_string_lossy().to_string())],
        sandbox: None,
        notification_sink: None,
    };
    let bridge = match ToolProcessBridge::spawn(config).await {
        Ok(b) => Arc::new(b),
        Err(e) => {
            eprintln!("skipping: could not spawn aivyx-kitchen-toolkit: {e}");
            return;
        }
    };

    // The harness registered the full kitchen surface.
    let descriptors = bridge.descriptors();
    assert_eq!(descriptors.len(), 11, "expected 11 kitchen tools registered");
    let list = descriptors
        .iter()
        .find(|d| d.name == "kitchen.inventory.list")
        .expect("kitchen.inventory.list registered")
        .clone();

    let proxy = ToolProxy::new(
        Arc::clone(&bridge),
        list.name.clone(),
        list.description.clone(),
        list.input_schema.clone(),
        &list.required_scope,
    )
    .expect("required_scope parses");
    assert_eq!(proxy.required_scope(&serde_json::json!({})).to_string(), "kitchen.read");

    let channel = FakeChannel { session: SessionId::new(), token: CancellationToken::new() };
    let audit = NullAudit;
    let ctx = ToolContext {
        agent_id: AgentId::new(),
        session_id: channel.session,
        turn_id: TurnId::new(),
        channel: &channel,
        audit: &audit,
        cancellation: &channel.token,
        message_origin: aivyx_core::MessageOrigin::Operator,
    };

    match proxy.execute(serde_json::json!({}), &ctx).await {
        ToolOutcome::Completed { output, verified } => {
            assert_eq!(output["count"], 2);
            assert_eq!(output["items"][0]["sku"], "TOM-01");
            assert!(matches!(verified, Verification::NotApplicable));
        }
        other => panic!("expected Completed, got {other:?}"),
    }

    std::fs::remove_dir_all(&home).ok();
}

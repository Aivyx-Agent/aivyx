//! Web UI channel — Phase 39.
//!
//! A localhost-only web chat interface that connects to the daemon
//! over the existing IPC protocol via WebSocket. The web server is
//! a background task spawned inside `run_daemon` (same pattern as
//! the webhook listener). Each WebSocket connection bridges directly
//! to the daemon's Unix socket at the frame level — sending
//! `FrontendMessage` frames and forwarding `DaemonEnvelope` frames
//! in real time.
//!
//! `WebDaemonChannel` is the `ChannelContext` stub returned by the
//! daemon's `ChannelFactory` when a `FrontendType::Web` connection
//! arrives. Like `TelegramDaemonChannel`, its `stream_event` and
//! `finalize` are no-ops — the `IpcChannelBridge` handles
//! forwarding those over IPC.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixStream};

use aivyx_core::{
    CancellationToken, ChannelContext, ChannelError, ChannelPlatform, SessionId, StreamEvent,
    TurnOutcome,
};

use crate::daemon_server::DaemonError;
use crate::daemon_ipc::{
    decode_frame, encode_frame, DaemonEnvelope, FrameError, FrontendMessage, FrontendType,
};
use crate::notify_webui::{DesktopNotificationFrame, WebUiBroadcaster};

/// Default web UI port. Adjacent to webhook (7842).
pub const DEFAULT_WEB_UI_PORT: u16 = 7843;

/// Embedded HTML/CSS/JS frontend. Single file, no external deps.
const HTML: &str = include_str!("web_ui_static.html");

// ---------------------------------------------------------------------------
// WebDaemonChannel — identity stub for the daemon's ChannelFactory
// ---------------------------------------------------------------------------

/// Lightweight `ChannelContext` stub that returns `Trusted` trust
/// tier and `Local` platform. Used by the daemon's `ChannelFactory`
/// when a `FrontendType::Web` connection arrives. The stub's
/// `stream_event` and `finalize` are no-ops — the `IpcChannelBridge`
/// handles forwarding those over IPC.
///
/// `ChannelPlatform::Local` is correct because the web UI is bound
/// to `127.0.0.1` only — the existing `Local` doc says "CLI,
/// desktop app, local REST on 127.0.0.1."
pub struct WebDaemonChannel {
    session: SessionId,
    token: CancellationToken,
}

impl WebDaemonChannel {
    pub fn new() -> Self {
        WebDaemonChannel {
            session: SessionId::new(),
            token: CancellationToken::new(),
        }
    }
}

impl Default for WebDaemonChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ChannelContext for WebDaemonChannel {
    fn channel_name(&self) -> &str {
        "aivyx-web"
    }

    fn platform(&self) -> ChannelPlatform {
        ChannelPlatform::Local
    }

    fn trust_tier(&self) -> aivyx_capability::TrustTier {
        aivyx_capability::TrustTier::Trusted
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

// ---------------------------------------------------------------------------
// Web UI HTTP + WebSocket server
// ---------------------------------------------------------------------------

/// Run the web UI server. Binds `127.0.0.1:<port>`, serves the
/// embedded HTML at `GET /`, and upgrades `GET /ws` to WebSocket.
/// Each WS connection bridges to the daemon's Unix socket at
/// `socket_path`.
///
/// Uses `tokio-tungstenite` directly on the TCP stream — each
/// incoming connection is peeked to determine if it's a WebSocket
/// upgrade request for `/ws` or an HTTP request for `/`. This
/// avoids pulling hyper into the WebSocket path.
///
/// This future never returns normally — it runs until `shutdown` is
/// cancelled.
pub async fn run_web_ui_server(
    socket_path: PathBuf,
    port: u16,
    shutdown: CancellationToken,
    web_ui_broadcaster: Option<Arc<WebUiBroadcaster>>,
) -> Result<(), DaemonError> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|source| DaemonError::Bind {
            path: addr.to_string(),
            source,
        })?;

    eprintln!("aivyx web ui: listening on http://{addr}");

    let socket_path = Arc::new(socket_path);

    loop {
        let (stream, _remote) = tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok(conn) => conn,
                    Err(e) => {
                        eprintln!("aivyx web ui: accept error: {e}");
                        continue;
                    }
                }
            }
            _ = shutdown.cancelled() => return Ok(()),
        };

        let conn_socket_path = Arc::clone(&socket_path);
        let conn_broadcaster = web_ui_broadcaster.clone();

        tokio::spawn(async move {
            if let Err(e) =
                handle_connection(stream, &conn_socket_path, conn_broadcaster).await
            {
                eprintln!("aivyx web ui: connection error: {e}");
            }
        });
    }
}

/// Handle a single TCP connection. Peek at the first bytes to
/// determine the HTTP path, then either serve HTML or upgrade to
/// WebSocket.
async fn handle_connection(
    stream: tokio::net::TcpStream,
    socket_path: &Path,
    web_ui_broadcaster: Option<Arc<WebUiBroadcaster>>,
) -> Result<(), DaemonError> {
    // Peek at the HTTP request line to determine the path.
    // We read up to 1024 bytes to get the full request line.
    let mut peek_buf = [0u8; 1024];
    let n = stream
        .peek(&mut peek_buf)
        .await?;
    let request_line = String::from_utf8_lossy(&peek_buf[..n]);

    if request_line.starts_with("GET /ws") {
        // WebSocket upgrade — let tokio-tungstenite handle the
        // HTTP 101 handshake and the WebSocket framing.
        let ws_stream = tokio_tungstenite::accept_async(stream)
            .await
            .map_err(|e| DaemonError::WebSocket(format!("ws handshake: {e}")))?;

        handle_websocket(ws_stream, socket_path, web_ui_broadcaster).await
    } else if request_line.starts_with("GET / ")
        || request_line.starts_with("GET / HTTP")
    {
        // Serve the embedded HTML page over plain HTTP.
        serve_html(stream).await
    } else {
        // 404 for everything else.
        serve_404(stream).await
    }
}

/// Serve the embedded HTML page as an HTTP response.
async fn serve_html(mut stream: tokio::net::TcpStream) -> Result<(), DaemonError> {
    let body = HTML.as_bytes();
    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(body).await?;
    Ok(())
}

/// Serve a 404 Not Found response.
async fn serve_404(mut stream: tokio::net::TcpStream) -> Result<(), DaemonError> {
    let body = b"not found";
    let response = format!(
        "HTTP/1.1 404 Not Found\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(body).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// WebSocket ↔ daemon IPC bridge
// ---------------------------------------------------------------------------

/// Bridge a single WebSocket connection to the daemon's Unix socket.
/// Runs two concurrent loops:
/// 1. WS→IPC: reads JSON from WebSocket, encodes as IPC frames,
///    sends to the daemon.
/// 2. IPC→WS: reads IPC frames from the daemon, sends JSON over WS.
async fn handle_websocket(
    ws_stream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    socket_path: &Path,
    web_ui_broadcaster: Option<Arc<WebUiBroadcaster>>,
) -> Result<(), DaemonError> {
    // Connect to the daemon's Unix socket.
    let unix_stream = UnixStream::connect(socket_path).await?;
    let (mut unix_reader, mut unix_writer) = unix_stream.into_split();

    // Read DaemonReady from the daemon.
    let mut ipc_buf = Vec::with_capacity(4096);
    loop {
        let mut tmp = [0u8; 4096];
        let n = unix_reader.read(&mut tmp).await?;
        if n == 0 {
            return Err(DaemonError::Protocol(
                "daemon disconnected before DaemonReady".into(),
            ));
        }
        ipc_buf.extend_from_slice(&tmp[..n]);
        match decode_frame::<DaemonEnvelope>(&ipc_buf) {
            Ok((DaemonEnvelope::DaemonReady { .. }, consumed)) => {
                ipc_buf.drain(..consumed);
                break;
            }
            Err(FrameError::IncompleteBuf) => continue,
            Ok((other, _)) => {
                return Err(DaemonError::Protocol(format!(
                    "expected DaemonReady, got {other:?}"
                )));
            }
            Err(e) => return Err(e.into()),
        }
    }

    // Send StartSession with FrontendType::Web.
    let start = FrontendMessage::StartSession {
        role: None,
        frontend_type: Some(FrontendType::Web),
    };
    let frame = encode_frame(&start)?;
    unix_writer.write_all(&frame).await?;

    // Read SessionStarted — forward it to the WebSocket as JSON.
    let session_id: String = loop {
        let mut tmp = [0u8; 4096];
        let n = unix_reader.read(&mut tmp).await?;
        if n == 0 {
            return Err(DaemonError::Protocol(
                "daemon disconnected before SessionStarted".into(),
            ));
        }
        ipc_buf.extend_from_slice(&tmp[..n]);
        match decode_frame::<DaemonEnvelope>(&ipc_buf) {
            Ok((DaemonEnvelope::SessionStarted { session_id }, consumed)) => {
                ipc_buf.drain(..consumed);
                break session_id;
            }
            Err(FrameError::IncompleteBuf) => continue,
            Ok((other, _)) => {
                return Err(DaemonError::Protocol(format!(
                    "expected SessionStarted, got {other:?}"
                )));
            }
            Err(e) => return Err(e.into()),
        }
    };

    // Split the WebSocket stream. The sink is shared with the
    // optional broadcast relay loop (Phase 69 Task 5), so wrap it
    // behind a Mutex like the unix writer.
    let (ws_sink, mut ws_source) = ws_stream.split();
    let ws_sink = Arc::new(tokio::sync::Mutex::new(ws_sink));

    // Send SessionStarted to the browser.
    let session_started_json = serde_json::json!({
        "type": "SessionStarted",
        "session_id": session_id,
    });
    {
        let mut sink = ws_sink.lock().await;
        let _ = sink
            .send(tokio_tungstenite::tungstenite::Message::Text(
                session_started_json.to_string().into(),
            ))
            .await;
    }

    // Three concurrent loops: WS→IPC, IPC→WS, and (when a
    // broadcaster is configured) Broadcast→WS for Phase 69
    // desktop notifications.
    let unix_writer = Arc::new(tokio::sync::Mutex::new(unix_writer));

    // IPC→WS: read daemon frames, forward as JSON over WebSocket.
    let ipc_to_ws = {
        let ws_sink = Arc::clone(&ws_sink);
        async move {
            let mut buf = ipc_buf; // reuse the buffer from handshake
            loop {
                // Try to decode any buffered frames first.
                loop {
                    match decode_frame::<DaemonEnvelope>(&buf) {
                        Ok((envelope, consumed)) => {
                            buf.drain(..consumed);
                            let json = match serde_json::to_string(&envelope) {
                                Ok(j) => j,
                                Err(e) => {
                                    eprintln!("aivyx web ui: serialize error: {e}");
                                    continue;
                                }
                            };
                            let mut sink = ws_sink.lock().await;
                            if sink
                                .send(tokio_tungstenite::tungstenite::Message::Text(
                                    json.into(),
                                ))
                                .await
                                .is_err()
                            {
                                return; // WebSocket closed
                            }
                        }
                        Err(FrameError::IncompleteBuf) => break,
                        Err(e) => {
                            eprintln!("aivyx web ui: ipc decode error: {e}");
                            return;
                        }
                    }
                }

                // Read more bytes from the daemon.
                let mut tmp = [0u8; 4096];
                match unix_reader.read(&mut tmp).await {
                    Ok(0) => return, // daemon disconnected
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    Err(_) => return,
                }
            }
        }
    };

    // WS→IPC: read JSON from WebSocket, encode as IPC frames, send
    // to daemon.
    let ws_to_ipc = {
        let unix_writer = Arc::clone(&unix_writer);
        async move {
            while let Some(msg_result) = ws_source.next().await {
                let msg = match msg_result {
                    Ok(m) => m,
                    Err(_) => return, // WebSocket error
                };

                let text = match msg {
                    tokio_tungstenite::tungstenite::Message::Text(t) => t,
                    tokio_tungstenite::tungstenite::Message::Close(_) => return,
                    _ => continue, // ignore binary, ping, pong
                };

                // Parse as FrontendMessage and re-encode as IPC frame.
                let frontend_msg: FrontendMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("aivyx web ui: invalid WS message: {e}");
                        continue;
                    }
                };

                let frame = match encode_frame(&frontend_msg) {
                    Ok(f) => f,
                    Err(e) => {
                        eprintln!("aivyx web ui: encode error: {e}");
                        continue;
                    }
                };

                let mut w = unix_writer.lock().await;
                if w.write_all(&frame).await.is_err() {
                    return; // daemon disconnected
                }
            }
        }
    };

    // Broadcast→WS (Phase 69 Task 5): if a WebUiBroadcaster is
    // configured, subscribe a fresh receiver and relay every
    // DesktopNotificationFrame onto the WS as
    // DaemonEnvelope::DesktopNotification. On `Lagged` we drop
    // the missed frames silently — desktop notifications for a
    // tab the operator isn't watching are by definition
    // discardable.
    let broadcast_to_ws = {
        let ws_sink = Arc::clone(&ws_sink);
        let mut rx_opt = web_ui_broadcaster.as_ref().map(|bc| bc.subscribe());
        async move {
            let Some(rx) = rx_opt.as_mut() else {
                // No broadcaster wired — this future never
                // completes, so the `select!` only fires on the
                // other two arms.
                std::future::pending::<()>().await;
                return;
            };
            loop {
                match rx.recv().await {
                    Ok(DesktopNotificationFrame { title, body }) => {
                        let envelope = DaemonEnvelope::DesktopNotification { title, body };
                        let json = match serde_json::to_string(&envelope) {
                            Ok(j) => j,
                            Err(e) => {
                                eprintln!("aivyx web ui: broadcast serialize error: {e}");
                                continue;
                            }
                        };
                        let mut sink = ws_sink.lock().await;
                        if sink
                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                json.into(),
                            ))
                            .await
                            .is_err()
                        {
                            return; // WebSocket closed
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Skipped some frames; keep listening.
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        }
    };

    // Run all three loops concurrently. When any exits, the
    // connection is done.
    tokio::select! {
        _ = ipc_to_ws => {}
        _ = ws_to_ipc => {}
        _ = broadcast_to_ws => {}
    }

    // Send Disconnect to the daemon (best effort).
    if let Ok(frame) = encode_frame(&FrontendMessage::Disconnect) {
        let mut w = unix_writer.lock().await;
        let _ = w.write_all(&frame).await;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_channel_platform_is_local() {
        let ch = WebDaemonChannel::new();
        assert_eq!(ch.platform(), ChannelPlatform::Local);
    }

    #[test]
    fn web_channel_trust_tier_is_trusted() {
        let ch = WebDaemonChannel::new();
        assert_eq!(ch.trust_tier(), aivyx_capability::TrustTier::Trusted);
    }

    #[test]
    fn web_channel_name() {
        let ch = WebDaemonChannel::new();
        assert_eq!(ch.channel_name(), "aivyx-web");
    }

    #[test]
    fn default_port_is_7843() {
        assert_eq!(DEFAULT_WEB_UI_PORT, 7843);
    }

    #[test]
    fn html_is_non_empty() {
        assert!(!HTML.is_empty(), "embedded HTML must not be empty");
        assert!(HTML.contains("<html"), "embedded HTML must contain <html tag");
    }

    /// Phase 47 — the embedded HTML must wire the four inspection panes
    /// (chat / missions / audit / sessions) and the Query/QueryResponse
    /// JS layer. Pure substring smoke tests — they protect against
    /// accidental gutting of the tab strip during future refactors.
    #[test]
    fn html_contains_phase_47_tab_structure() {
        assert!(HTML.contains("data-pane=\"chat\""));
        assert!(HTML.contains("data-pane=\"missions\""));
        assert!(HTML.contains("data-pane=\"audit\""));
        assert!(HTML.contains("data-pane=\"sessions\""));
    }

    #[test]
    fn html_contains_phase_47_query_dispatch() {
        // The JS layer must build Query frames and handle QueryResponse.
        assert!(HTML.contains("'Query'"), "must construct Query frames");
        assert!(
            HTML.contains("'QueryResponse'"),
            "must dispatch on QueryResponse type"
        );
        // The three query kinds the UI auto-issues on tab activation.
        assert!(HTML.contains("'ListMissions'"));
        assert!(HTML.contains("'ListAuditEntries'"));
        assert!(HTML.contains("'ListSessions'"));
        assert!(HTML.contains("'VerifyAuditChain'"));
    }

    /// Phase 70 — the embedded HTML must wire the Proposals
    /// pane: tab presence, filter chips, ListPersonaProposals
    /// query dispatch, and ResolvePersonaProposal envelope
    /// construction with all three resolution variants.
    #[test]
    fn html_contains_phase_70_proposals_pane_wiring() {
        // Tab structure.
        assert!(
            HTML.contains("data-pane=\"proposals\""),
            "must declare the proposals tab/pane"
        );
        // Filter chips.
        assert!(
            HTML.contains("data-proposal-filter=\"pending\""),
            "must include a pending filter chip"
        );
        assert!(
            HTML.contains("data-proposal-filter=\"all\""),
            "must include an all-status filter chip"
        );
        // IPC dispatch.
        assert!(
            HTML.contains("'ListPersonaProposals'"),
            "must dispatch ListPersonaProposals queries"
        );
        assert!(
            HTML.contains("'ResolvePersonaProposal'"),
            "must construct ResolvePersonaProposal envelopes"
        );
        // Three resolution kinds wired in the UI.
        assert!(
            HTML.contains("'Approve'"),
            "must offer the Approve resolution"
        );
        assert!(
            HTML.contains("'ApproveWithEdit'"),
            "must offer the ApproveWithEdit (Q3a edit-then-approve) resolution"
        );
        assert!(
            HTML.contains("'Reject'"),
            "must offer the Reject resolution"
        );
        // Response handler.
        assert!(
            HTML.contains("'PersonaProposalResolved'"),
            "must dispatch on PersonaProposalResolved type"
        );
    }

    /// Phase 69 — the embedded HTML must wire the desktop
    /// notification handler, the permission prompt banner, and
    /// the in-page toast stack. Substring smoke tests guard
    /// against accidental gutting of the Reach-Phase-4 surface.
    #[test]
    fn html_contains_phase_69_desktop_notification_wiring() {
        // WS receive-side handler.
        assert!(
            HTML.contains("'DesktopNotification'"),
            "must dispatch on DesktopNotification type"
        );
        assert!(
            HTML.contains("showDesktopNotification"),
            "must define the browser-Notification helper"
        );
        assert!(
            HTML.contains("showToast"),
            "must define the in-page toast helper"
        );
        // Permission prompt + banner controls.
        assert!(
            HTML.contains("notif-permission-banner"),
            "must include the permission prompt banner element"
        );
        assert!(
            HTML.contains("Notification.requestPermission"),
            "must request browser notification permission"
        );
        // Toast stack container.
        assert!(
            HTML.contains("toast-stack"),
            "must include the toast stack container"
        );
    }
}

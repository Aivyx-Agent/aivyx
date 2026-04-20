//! Web UI channel — Phase 39.
//!
//! A localhost-only web chat interface that connects to the daemon
//! over the existing IPC protocol via WebSocket. The web server is
//! a background task spawned inside `run_daemon` (same pattern as
//! the webhook listener). Each WebSocket connection creates a
//! `DaemonSession` that bridges to the daemon's Unix socket.
//!
//! `WebDaemonChannel` is the `ChannelContext` stub returned by the
//! daemon's `ChannelFactory` when a `FrontendType::Web` connection
//! arrives. Like `TelegramDaemonChannel`, its `stream_event` and
//! `finalize` are no-ops — the `IpcChannelBridge` handles
//! forwarding those over IPC.

use aivyx_core::{
    CancellationToken, ChannelContext, ChannelError, ChannelPlatform, SessionId, StreamEvent,
    TurnOutcome,
};

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
}

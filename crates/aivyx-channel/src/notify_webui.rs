//! Phase 69 — Web UI desktop notification backend (Reach Phase 4),
//! generalized in Chapter Mission Control to also carry live mission
//! updates.
//!
//! Fourth notify backend after Telegram (Phase 62), webhook
//! (Phase 62), and email (Phase 68). Delivers OS-level desktop
//! notifications to whichever browser tabs have the localhost
//! Web UI (`127.0.0.1:7843`) open, by fanning frames out over a
//! [`tokio::sync::broadcast`] channel and relaying them onto
//! every connected Web UI WebSocket as
//! [`crate::daemon_ipc::DaemonMessage::DesktopNotification`]. The same
//! channel also carries live team-mission phase/step-state pushes,
//! relayed as `DaemonMessage::TeamMissionUpdated` — see [`WebUiBroadcastFrame`]
//! below.
//!
//! ## Shape
//!
//! - [`WebUiBroadcaster`] wraps a single
//!   `broadcast::Sender<WebUiBroadcastFrame>`. Constructed
//!   once at daemon startup and `Arc`-shared between:
//!     1. The [`NotifyWebUiBackend`] registered in the notify
//!        dispatcher (push side, `DesktopNotification` frames only).
//!     2. The Web UI WS handler in `web_ui.rs`, which subscribes
//!        a fresh receiver per connection (pop side).
//! - [`WebUiBroadcastFrame`] is the internal channel-carried
//!   enum (Chapter Mission Control) — [`DesktopNotificationFrame`]
//!   is one of its variants; the WS write-side translates each
//!   variant into its matching `DaemonMessage`/`DaemonEnvelope`
//!   shape before encoding the on-wire frame.
//! - [`NotifyWebUiBackend`] implements [`NotifyBackend::send`] by
//!   pushing a frame onto the broadcaster. Per Phase 69 Q1(a),
//!   `Ok(())` is returned even when no receivers are subscribed
//!   — broadcast-style fire-and-forget. The audit chain
//!   (Phase 67) still records every dispatch.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::notify_dispatcher::{NotifyBackend, NotifyError};

/// Default capacity of the underlying broadcast channel. Sized
/// for short bursts (the agent firing a small batch of notifies
/// in a single turn); subscribers that fall behind by more than
/// this many frames will see a `RecvError::Lagged` and skip
/// frames, which is acceptable for desktop notifications (a
/// lagged tab is one the operator isn't watching).
pub const DEFAULT_BROADCAST_CAPACITY: usize = 64;

/// Frame carried over the broadcast channel. Converted to
/// [`crate::daemon_ipc::DaemonMessage::DesktopNotification`] at
/// the WS write-site, so the on-wire shape stays in
/// `daemon_ipc.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopNotificationFrame {
    pub title: String,
    pub body: String,
}

/// Chapter Mission Control — the broadcast channel now carries either kind
/// of Web UI push. `WebUiBroadcaster` itself stays a single channel/single
/// subscribe-point per connection (not two separate broadcasters + a
/// `select!` per connection) — the WS relay loop matches on this enum and
/// forwards each variant onto its own `DaemonEnvelope` shape.
#[derive(Debug, Clone, PartialEq)]
pub enum WebUiBroadcastFrame {
    DesktopNotification(DesktopNotificationFrame),
    /// A team mission's live state changed (a step started/finished, or the
    /// mission's phase transitioned) — carries the already-projected view,
    /// computed daemon-side where the live running-step signal is in scope
    /// (see `TeamMissionRecord::to_view_with_running` in `aivyx-ipc`).
    TeamMissionUpdated(aivyx_ipc::TeamMissionView),
}

/// Broadcaster handle. Cheap-clonable (the inner sender is
/// already `Clone`); the daemon's startup path wraps it in an
/// `Arc` so both the dispatcher and the WS handler can hold the
/// same instance without juggling clones at every site.
pub struct WebUiBroadcaster {
    sender: broadcast::Sender<WebUiBroadcastFrame>,
}

impl WebUiBroadcaster {
    /// Build a broadcaster with [`DEFAULT_BROADCAST_CAPACITY`].
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_BROADCAST_CAPACITY)
    }

    /// Build a broadcaster with an explicit channel capacity.
    /// Useful in tests that want to exercise the lagged-receiver
    /// path with a small capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _initial_rx) = broadcast::channel(capacity);
        // The initial receiver is dropped immediately; subscribers
        // are added by the WS handler per browser connection.
        Self { sender }
    }

    /// Subscribe a fresh receiver. The Web UI WS handler calls
    /// this once per accepted browser connection.
    pub fn subscribe(&self) -> broadcast::Receiver<WebUiBroadcastFrame> {
        self.sender.subscribe()
    }

    /// Count of currently subscribed receivers. Used in tests
    /// and the dispatcher's diagnostic Debug impl.
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }

    /// Push a frame onto the channel. Returns `Ok(())` even
    /// when there are zero subscribers (per Phase 69 Q1(a)).
    pub fn broadcast(&self, frame: WebUiBroadcastFrame) -> Result<(), NotifyError> {
        // `broadcast::Sender::send` returns `Err(SendError(...))`
        // only when there are no active receivers. Per Q1(a)
        // that's an Ok outcome — the notification fired into a
        // room with no listeners, which is by design.
        let _ = self.sender.send(frame);
        Ok(())
    }
}

impl Default for WebUiBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for WebUiBroadcaster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebUiBroadcaster")
            .field("receiver_count", &self.receiver_count())
            .finish()
    }
}

/// [`NotifyBackend`] implementation that pushes onto an
/// `Arc<WebUiBroadcaster>`. One instance per `[[notify_target]]
/// kind = "web-ui"` block — but since the broadcaster is the
/// single source of truth, multiple Web UI targets all funnel
/// into the same fan-out.
pub struct NotifyWebUiBackend {
    broadcaster: Arc<WebUiBroadcaster>,
}

impl NotifyWebUiBackend {
    pub fn new(broadcaster: Arc<WebUiBroadcaster>) -> Self {
        Self { broadcaster }
    }
}

#[async_trait]
impl NotifyBackend for NotifyWebUiBackend {
    async fn send(
        &self,
        message: &str,
        subject: Option<&str>,
    ) -> Result<(), NotifyError> {
        // Web UI desktop notifications carry both a title and a
        // body; if the caller didn't provide a subject we fall
        // back to a generic "Aivyx" title so the browser
        // notification has something to display in its header.
        let title = subject.unwrap_or("Aivyx").to_string();
        let body = message.to_string();
        self.broadcaster
            .broadcast(WebUiBroadcastFrame::DesktopNotification(
                DesktopNotificationFrame { title, body },
            ))
    }

    fn kind(&self) -> &'static str {
        "web-ui"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn broadcast_with_zero_subscribers_is_ok() {
        let bc = Arc::new(WebUiBroadcaster::new());
        let backend = NotifyWebUiBackend::new(Arc::clone(&bc));
        // Zero receivers — per Q1(a), this must not error.
        assert_eq!(bc.receiver_count(), 0);
        backend
            .send("build done", Some("Aivyx"))
            .await
            .expect("Ok with no subscribers");
    }

    #[tokio::test]
    async fn broadcast_reaches_single_subscriber() {
        let bc = Arc::new(WebUiBroadcaster::new());
        let mut rx = bc.subscribe();
        let backend = NotifyWebUiBackend::new(Arc::clone(&bc));
        backend
            .send("trigger fired", Some("Schedule"))
            .await
            .expect("send");
        let frame = rx.recv().await.expect("recv");
        match frame {
            WebUiBroadcastFrame::DesktopNotification(DesktopNotificationFrame {
                title,
                body,
            }) => {
                assert_eq!(title, "Schedule");
                assert_eq!(body, "trigger fired");
            }
            other => panic!("expected DesktopNotification, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn broadcast_fans_out_to_all_subscribers() {
        let bc = Arc::new(WebUiBroadcaster::new());
        let mut rx1 = bc.subscribe();
        let mut rx2 = bc.subscribe();
        let mut rx3 = bc.subscribe();
        assert_eq!(bc.receiver_count(), 3);
        let backend = NotifyWebUiBackend::new(Arc::clone(&bc));
        backend.send("hello", None).await.expect("send");
        for rx in [&mut rx1, &mut rx2, &mut rx3] {
            let frame = rx.recv().await.expect("recv");
            match frame {
                WebUiBroadcastFrame::DesktopNotification(DesktopNotificationFrame {
                    title,
                    body,
                }) => {
                    assert_eq!(title, "Aivyx");
                    assert_eq!(body, "hello");
                }
                other => panic!("expected DesktopNotification, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn missing_subject_falls_back_to_aivyx_title() {
        let bc = Arc::new(WebUiBroadcaster::new());
        let mut rx = bc.subscribe();
        let backend = NotifyWebUiBackend::new(Arc::clone(&bc));
        backend.send("no subject", None).await.expect("send");
        let frame = rx.recv().await.expect("recv");
        match frame {
            WebUiBroadcastFrame::DesktopNotification(DesktopNotificationFrame {
                title,
                body,
            }) => {
                assert_eq!(title, "Aivyx");
                assert_eq!(body, "no subject");
            }
            other => panic!("expected DesktopNotification, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn kind_reports_web_ui() {
        let bc = Arc::new(WebUiBroadcaster::new());
        let backend = NotifyWebUiBackend::new(bc);
        assert_eq!(backend.kind(), "web-ui");
    }

    #[test]
    fn debug_impl_includes_receiver_count() {
        let bc = WebUiBroadcaster::new();
        let _rx = bc.subscribe();
        let s = format!("{bc:?}");
        assert!(s.contains("receiver_count"), "got: {s}");
        assert!(s.contains("1"), "got: {s}");
    }

    #[tokio::test]
    async fn broadcast_relays_a_team_mission_updated_frame() {
        use aivyx_ipc::{TeamMissionPhase, TeamMissionView};
        let bc = Arc::new(WebUiBroadcaster::new());
        let mut rx = bc.subscribe();
        let view = TeamMissionView {
            id: "m1".into(),
            goal: "test".into(),
            lead: "coordinator".into(),
            phase: TeamMissionPhase::Executing,
            pending_gate: None,
            halt_reason: None,
            verify_attempts: 0,
            progress: 0,
            steps: vec![],
        };
        bc.broadcast(WebUiBroadcastFrame::TeamMissionUpdated(view.clone()))
            .expect("broadcast");
        match rx.recv().await.expect("recv") {
            WebUiBroadcastFrame::TeamMissionUpdated(got) => assert_eq!(got, view),
            other => panic!("expected TeamMissionUpdated, got {other:?}"),
        }
    }
}

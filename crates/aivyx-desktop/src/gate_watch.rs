//! Background approval-gate watcher.
//!
//! A lightweight WebSocket client to the daemon (`ws://127.0.0.1:7843/ws`) that
//! polls the same `TeamMissionList` the Studio polls and fires a native OS
//! notification whenever a mission enters `AwaitingApproval` with a pending
//! gate. This is the headline value of the desktop app: you get told a mission
//! needs your sign-off **even when no window is open**.
//!
//! It runs on its own thread/runtime and never blocks the UI. Clicking a
//! notification's "Open Studio" action raises the window via the event-loop
//! proxy (a [`tao::event_loop::EventLoopProxy`] carrying [`UserEvent`]).

use std::collections::HashSet;
use std::time::Duration;

use aivyx_ipc::protocol::{
    DaemonEnvelope, FrontendMessage, QueryPayload, QueryResponsePayload,
};
use aivyx_ipc::TeamMissionPhase;
use futures_util::{SinkExt, StreamExt};
use tao::event_loop::EventLoopProxy;
use tokio_tungstenite::tungstenite::Message;

use crate::UserEvent;

const WS_URL: &str = "ws://127.0.0.1:7843/ws";

/// The daemon ws endpoint, derived from `AIVYX_STUDIO_URL` when set
/// (remote appliance) — same override the webview honors in `main.rs`.
fn ws_url() -> String {
    match std::env::var("AIVYX_STUDIO_URL") {
        Ok(u) => {
            let host = u
                .trim_start_matches("http://")
                .trim_start_matches("https://")
                .trim_end_matches('/')
                .split('/')
                .next()
                .unwrap_or("127.0.0.1:7843")
                .to_string();
            format!("ws://{host}/ws")
        }
        Err(_) => WS_URL.to_string(),
    }
}
const POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Backoff between reconnect attempts when the daemon is down or drops us.
const RECONNECT_DELAY: Duration = Duration::from_secs(3);

/// Run the watcher forever, reconnecting on any error or drop.
pub async fn run(proxy: EventLoopProxy<UserEvent>) {
    loop {
        if let Err(e) = watch_once(&proxy).await {
            eprintln!("aivyx-desktop: gate watcher disconnected ({e}); retrying…");
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

/// One connection's lifetime: handshake, then poll missions and notify on each
/// newly-seen gate until the socket drops.
async fn watch_once(proxy: &EventLoopProxy<UserEvent>) -> Result<(), Box<dyn std::error::Error>> {
    let (ws, _resp) = tokio_tungstenite::connect_async(ws_url()).await?;
    let (mut write, mut read) = ws.split();

    // Mirror the Studio's handshake (read-only queries may not require it, but
    // it keeps the shell a well-behaved client).
    let start = serde_json::to_string(&FrontendMessage::StartSession {
        role: None,
        frontend_type: None,
    })?;
    write.send(Message::text(start)).await?;

    // Gates already surfaced this connection — `"<mission_id>::<gate>"`, so we
    // notify once per distinct gate, not on every poll.
    let mut seen: HashSet<String> = HashSet::new();
    let mut ticker = tokio::time::interval(POLL_INTERVAL);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let q = serde_json::to_string(&FrontendMessage::Query {
                    id: "dt-missions".to_string(),
                    payload: QueryPayload::TeamMissionList,
                })?;
                write.send(Message::text(q)).await?;
            }
            msg = read.next() => {
                let Some(msg) = msg else { break };  // socket closed
                let Message::Text(text) = msg? else { continue };
                let Ok(env) = serde_json::from_str::<DaemonEnvelope>(text.as_str()) else {
                    continue;
                };
                if let DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::TeamMissionList { missions },
                    ..
                } = env
                {
                    for m in missions {
                        if m.phase != TeamMissionPhase::AwaitingApproval {
                            continue;
                        }
                        let Some(gate) = m.pending_gate.clone() else { continue };
                        let key = format!("{}::{}", m.id, gate);
                        if seen.insert(key) {
                            notify_gate(&m.goal, &gate, proxy.clone());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Fire one OS notification for a gate. The notification carries an "Open
/// Studio" action; waiting for that action blocks, so it runs on its own
/// short-lived thread (gates are infrequent).
fn notify_gate(goal: &str, gate: &str, proxy: EventLoopProxy<UserEvent>) {
    let body = format!("{goal}\n⚑ {gate}");
    std::thread::spawn(move || {
        match notify_rust::Notification::new()
            .summary("Aivyx — approval needed")
            .body(&body)
            .action("open", "Open Studio")
            .show()
        {
            Ok(handle) => handle.wait_for_action(|action| {
                if action == "open" {
                    let _ = proxy.send_event(UserEvent::ShowWindow);
                }
            }),
            Err(e) => eprintln!("aivyx-desktop: notification failed: {e}"),
        }
    });
}

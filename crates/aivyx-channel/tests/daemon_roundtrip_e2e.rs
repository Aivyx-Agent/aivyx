//! Phase 16 Task 3 — daemon IPC round-trip end-to-end test.
//!
//! Proves the Phase 16 IPC protocol carries one turn end-to-end:
//! a PoC daemon server running on a background tokio task accepts
//! a connection, receives `StartSession` + `SubmitInput`, runs the
//! turn through a scripted agent, streams `StreamEvent` frames
//! back, and sends `TurnComplete`. The client collects all events
//! and asserts the streamed text matches.
//!
//! The test uses a real Unix domain socket (not mocked I/O) so the
//! framing, serialization, and async I/O all exercise the same code
//! paths the production daemon will use.
//!
//! | Seam              | Production                   | Test                  |
//! |-------------------|------------------------------|-----------------------|
//! | Agent             | `ConcreteAgent` + LLM        | `FakeStreamingAgent`  |
//! | Socket path       | `$XDG_RUNTIME_DIR/aivyx/...` | `$TMPDIR/<unique>`    |
//! | ChannelContext     | `LocalChannel<Stdout>`       | `LocalChannel<Vec>`   |

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use aivyx_capability::CapabilitySet;
use aivyx_channel::daemon_client::run_poc_client;
use aivyx_channel::daemon_ipc::StreamEventPayload;
use aivyx_channel::daemon_server::run_poc_daemon;
use aivyx_channel::LocalChannel;
use aivyx_core::{
    Agent, AgentId, ChannelContext, Message, StreamEvent, TurnOutcome,
};

// ---------------------------------------------------------------------------
// Fake agent that streams two text chunks and completes.
// ---------------------------------------------------------------------------

struct FakeStreamingAgent {
    id: AgentId,
    caps: CapabilitySet,
}

#[async_trait]
impl Agent for FakeStreamingAgent {
    fn id(&self) -> AgentId {
        self.id
    }

    fn capabilities(&self) -> &CapabilitySet {
        &self.caps
    }

    async fn turn(
        &self,
        _message: Message,
        channel: &dyn ChannelContext,
    ) -> TurnOutcome {
        // Stream two text chunks so the test can verify ordering.
        let _ = channel.stream_event(StreamEvent::Text("Hello ")).await;
        let _ = channel.stream_event(StreamEvent::Text("from daemon!")).await;

        TurnOutcome::Completed {
            final_message: "Hello from daemon!".into(),
            tool_calls_made: 0,
            duration: Duration::from_millis(1),
        }
    }
}

// ---------------------------------------------------------------------------
// Scratch directory for the Unix socket.
// ---------------------------------------------------------------------------

struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn new() -> Self {
        let tmp = std::env::var("TMPDIR")
            .or_else(|_| std::env::var("TEMP"))
            .unwrap_or_else(|_| "/tmp".to_string());
        let path = PathBuf::from(tmp).join(format!(
            "aivyx-daemon-e2e-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("scratch dir must be creatable");
        ScratchDir { path }
    }

    fn socket_path(&self) -> PathBuf {
        self.path.join("daemon.sock")
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn one_turn_round_trips_over_ipc() {
    let scratch = ScratchDir::new();
    let socket_path = scratch.socket_path();

    let agent: Arc<dyn Agent> = Arc::new(FakeStreamingAgent {
        id: AgentId::new(),
        caps: CapabilitySet::empty(),
    });

    let channel = Arc::new(LocalChannel::new("daemon-test", Vec::<u8>::new()));

    // Spawn the daemon server on a background task.
    let daemon_socket = socket_path.clone();
    let daemon_agent = Arc::clone(&agent);
    let daemon_channel = Arc::clone(&channel);
    let daemon_handle = tokio::spawn(async move {
        run_poc_daemon(&daemon_socket, daemon_agent, daemon_channel)
            .await
            .expect("daemon must complete successfully");
    });

    // Give the daemon a moment to bind the socket.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Run the client.
    let result = run_poc_client(&socket_path, None, "hello".to_string())
        .await
        .expect("client must complete successfully");

    // Assert the daemon sent DaemonReady with version "0.1".
    assert_eq!(
        result.daemon_version.as_deref(),
        Some("0.1"),
        "daemon must send version 0.1"
    );

    // Assert session_id is non-empty.
    assert!(
        !result.session_id.is_empty(),
        "session_id must be non-empty"
    );

    // Assert we received exactly two Text stream events in order.
    assert_eq!(
        result.events.len(),
        2,
        "expected 2 stream events, got {}: {:?}",
        result.events.len(),
        result.events,
    );
    assert_eq!(
        result.events[0],
        StreamEventPayload::Text {
            text: "Hello ".into()
        },
    );
    assert_eq!(
        result.events[1],
        StreamEventPayload::Text {
            text: "from daemon!".into()
        },
    );

    // Assert the outcome message.
    assert!(
        result.outcome.contains("Hello from daemon!"),
        "outcome must contain the final message, got: {}",
        result.outcome,
    );

    // Wait for the daemon task to finish cleanly.
    tokio::time::timeout(Duration::from_secs(5), daemon_handle)
        .await
        .expect("daemon must finish within 5s")
        .expect("daemon task must not panic");
}

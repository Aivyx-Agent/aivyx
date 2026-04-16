//! Daemon IPC round-trip end-to-end tests.
//!
//! Phase 16 Task 3 proved the IPC protocol carries one turn.
//! Phase 17 Task 2 extends to multi-turn sessions, graceful
//! shutdown, and frontend disconnect.
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
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use aivyx_capability::CapabilitySet;
use aivyx_channel::daemon_client::run_poc_client;
use aivyx_channel::daemon_ipc::{
    decode_frame, encode_frame, DaemonEnvelope, FrameError, FrontendMessage, StreamEventPayload,
};
use aivyx_channel::daemon_server::{run_daemon, run_poc_daemon};
use aivyx_channel::LocalChannel;
use aivyx_core::{
    Agent, AgentId, CancellationToken, ChannelContext, Message, StreamEvent, TurnOutcome,
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

// ---------------------------------------------------------------------------
// Phase 17 Task 2 — multi-turn session test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_turn_session_streams_both_turns() {
    let scratch = ScratchDir::new();
    let socket_path = scratch.socket_path();

    let agent: Arc<dyn Agent> = Arc::new(FakeStreamingAgent {
        id: AgentId::new(),
        caps: CapabilitySet::empty(),
    });

    let channel = Arc::new(LocalChannel::new("daemon-test", Vec::<u8>::new()));
    let shutdown = CancellationToken::new();

    let daemon_socket = socket_path.clone();
    let daemon_agent = Arc::clone(&agent);
    let daemon_channel = Arc::clone(&channel);
    let daemon_shutdown = shutdown.clone();
    let daemon_handle = tokio::spawn(async move {
        run_daemon(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect and handshake manually for multi-turn control.
    let stream = UnixStream::connect(&socket_path)
        .await
        .expect("connect to daemon");
    let (mut reader, mut writer) = stream.into_split();
    let mut buf = Vec::with_capacity(4096);

    // Read DaemonReady.
    read_more(&mut reader, &mut buf).await;
    let (envelope, consumed): (DaemonEnvelope, _) =
        decode_frame(&buf).expect("decode DaemonReady");
    buf.drain(..consumed);
    assert!(matches!(envelope, DaemonEnvelope::DaemonReady { .. }));

    // StartSession.
    let frame = encode_frame(&FrontendMessage::StartSession { role: None }).unwrap();
    writer.write_all(&frame).await.unwrap();

    let sid: String = loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((DaemonEnvelope::SessionStarted { session_id }, consumed)) => {
                buf.drain(..consumed);
                break session_id;
            }
            Err(FrameError::IncompleteBuf) => read_more(&mut reader, &mut buf).await,
            other => panic!("expected SessionStarted, got {other:?}"),
        }
    };

    // --- Turn 1 ---
    let frame = encode_frame(&FrontendMessage::SubmitInput {
        session_id: sid.clone(),
        text: "turn one".into(),
    })
    .unwrap();
    writer.write_all(&frame).await.unwrap();

    let (events_1, outcome_1) = collect_turn_events(&mut reader, &mut buf).await;
    assert_eq!(events_1.len(), 2, "turn 1 events: {events_1:?}");
    assert!(
        outcome_1.contains("Hello from daemon!"),
        "turn 1 outcome: {outcome_1}"
    );

    // --- Turn 2 ---
    let frame = encode_frame(&FrontendMessage::SubmitInput {
        session_id: sid.clone(),
        text: "turn two".into(),
    })
    .unwrap();
    writer.write_all(&frame).await.unwrap();

    let (events_2, outcome_2) = collect_turn_events(&mut reader, &mut buf).await;
    assert_eq!(events_2.len(), 2, "turn 2 events: {events_2:?}");
    assert!(
        outcome_2.contains("Hello from daemon!"),
        "turn 2 outcome: {outcome_2}"
    );

    // Disconnect cleanly.
    let frame = encode_frame(&FrontendMessage::Disconnect).unwrap();
    writer.write_all(&frame).await.unwrap();

    tokio::time::timeout(Duration::from_secs(5), daemon_handle)
        .await
        .expect("daemon must finish within 5s")
        .expect("daemon task must not panic");
}

// ---------------------------------------------------------------------------
// Phase 17 Task 2 — graceful shutdown test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graceful_shutdown_sends_shutting_down() {
    let scratch = ScratchDir::new();
    let socket_path = scratch.socket_path();

    let agent: Arc<dyn Agent> = Arc::new(FakeStreamingAgent {
        id: AgentId::new(),
        caps: CapabilitySet::empty(),
    });

    let channel = Arc::new(LocalChannel::new("daemon-test", Vec::<u8>::new()));
    let shutdown = CancellationToken::new();

    let daemon_socket = socket_path.clone();
    let daemon_agent = Arc::clone(&agent);
    let daemon_channel = Arc::clone(&channel);
    let daemon_shutdown = shutdown.clone();
    let daemon_handle = tokio::spawn(async move {
        run_daemon(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let stream = UnixStream::connect(&socket_path)
        .await
        .expect("connect to daemon");
    let (mut reader, mut _writer) = stream.into_split();
    let mut buf = Vec::with_capacity(4096);

    // Read DaemonReady.
    read_more(&mut reader, &mut buf).await;
    let (_envelope, consumed): (DaemonEnvelope, _) =
        decode_frame(&buf).expect("decode DaemonReady");
    buf.drain(..consumed);

    // Trigger shutdown from outside.
    shutdown.cancel();

    // The daemon should send ShuttingDown before closing.
    let shutting_down = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match decode_frame::<DaemonEnvelope>(&buf) {
                Ok((DaemonEnvelope::ShuttingDown { reason }, _consumed)) => {
                    return reason;
                }
                Err(FrameError::IncompleteBuf) => {
                    let mut tmp = [0u8; 4096];
                    match reader.read(&mut tmp).await {
                        Ok(0) => return "connection closed without ShuttingDown".into(),
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                        Err(e) => return format!("read error: {e}"),
                    }
                }
                Ok((other, consumed)) => {
                    buf.drain(..consumed);
                    panic!("unexpected message after shutdown: {other:?}");
                }
                Err(e) => panic!("decode error: {e}"),
            }
        }
    })
    .await
    .expect("must receive ShuttingDown within 5s");

    assert!(
        shutting_down.contains("shutdown"),
        "reason must mention shutdown, got: {shutting_down}"
    );

    tokio::time::timeout(Duration::from_secs(5), daemon_handle)
        .await
        .expect("daemon must finish within 5s")
        .expect("daemon task must not panic");
}

// ---------------------------------------------------------------------------
// Phase 17 Task 2 — frontend disconnect test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn frontend_disconnect_stops_daemon_cleanly() {
    let scratch = ScratchDir::new();
    let socket_path = scratch.socket_path();

    let agent: Arc<dyn Agent> = Arc::new(FakeStreamingAgent {
        id: AgentId::new(),
        caps: CapabilitySet::empty(),
    });

    let channel = Arc::new(LocalChannel::new("daemon-test", Vec::<u8>::new()));
    let shutdown = CancellationToken::new();

    let daemon_socket = socket_path.clone();
    let daemon_agent = Arc::clone(&agent);
    let daemon_channel = Arc::clone(&channel);
    let daemon_shutdown = shutdown.clone();
    let daemon_handle = tokio::spawn(async move {
        run_daemon(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect, read DaemonReady, then immediately drop the connection.
    {
        let stream = UnixStream::connect(&socket_path)
            .await
            .expect("connect to daemon");
        let (mut reader, _writer) = stream.into_split();
        let mut buf = Vec::with_capacity(4096);
        read_more(&mut reader, &mut buf).await;
        // Connection drops here when `stream` (via reader/_writer) goes out of scope.
    }

    // The daemon should exit cleanly when the frontend disconnects.
    tokio::time::timeout(Duration::from_secs(5), daemon_handle)
        .await
        .expect("daemon must finish within 5s after frontend disconnect")
        .expect("daemon task must not panic");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn read_more(reader: &mut tokio::net::unix::OwnedReadHalf, buf: &mut Vec<u8>) {
    let mut tmp = [0u8; 4096];
    let n = reader.read(&mut tmp).await.expect("read_more");
    assert!(n > 0, "unexpected EOF in read_more");
    buf.extend_from_slice(&tmp[..n]);
}

async fn collect_turn_events(
    reader: &mut tokio::net::unix::OwnedReadHalf,
    buf: &mut Vec<u8>,
) -> (Vec<StreamEventPayload>, String) {
    let mut events = Vec::new();
    loop {
        match decode_frame::<DaemonEnvelope>(buf) {
            Ok((DaemonEnvelope::StreamEvent { event, .. }, consumed)) => {
                buf.drain(..consumed);
                events.push(event);
            }
            Ok((DaemonEnvelope::TurnComplete { outcome, .. }, consumed)) => {
                buf.drain(..consumed);
                return (events, outcome);
            }
            Ok((DaemonEnvelope::Error { code, message }, _)) => {
                panic!("daemon error ({code}): {message}");
            }
            Err(FrameError::IncompleteBuf) => {
                read_more(reader, buf).await;
            }
            Ok((other, consumed)) => {
                buf.drain(..consumed);
                panic!("unexpected message during turn: {other:?}");
            }
            Err(e) => panic!("decode error: {e}"),
        }
    }
}

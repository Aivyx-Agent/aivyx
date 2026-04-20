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
use aivyx_channel::daemon_client::{daemon_is_running, run_poc_client, DaemonSession};
use aivyx_channel::{run_daemon_session, run_daemon_session_connected, DaemonSessionConfig};
use aivyx_channel::daemon_ipc::{
    decode_frame, encode_frame, DaemonEnvelope, FrameError, FrontendMessage, StreamEventPayload,
};
use aivyx_channel::daemon_ipc::FrontendType;
use aivyx_channel::daemon_server::{run_daemon, run_daemon_compat, run_poc_daemon, ChannelFactory};
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
        run_daemon_compat(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
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
    let frame = encode_frame(&FrontendMessage::StartSession { role: None, frontend_type: None }).unwrap();
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
        mission_id: None,
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
        mission_id: None,
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

    shutdown.cancel();
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
        run_daemon_compat(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
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
        run_daemon_compat(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
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

    // The handler task exits on disconnect; cancel the daemon's
    // accept loop so the daemon itself shuts down.
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), daemon_handle)
        .await
        .expect("daemon must finish within 5s after shutdown")
        .expect("daemon task must not panic");
}

// ---------------------------------------------------------------------------
// Phase 17 Task 4 — DaemonSession multi-turn test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn daemon_session_multi_turn_via_client_library() {
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
        run_daemon_compat(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut session = DaemonSession::connect(&socket_path, None, None)
        .await
        .expect("DaemonSession::connect must succeed");

    assert_eq!(session.daemon_version.as_deref(), Some("0.1"));
    assert!(!session.session_id.is_empty());

    // Turn 1.
    let (events_1, outcome_1) = session
        .submit_input("first turn".into())
        .await
        .expect("turn 1 must succeed");
    assert_eq!(events_1.len(), 2, "turn 1 events: {events_1:?}");
    assert!(outcome_1.contains("Hello from daemon!"));

    // Turn 2.
    let (events_2, outcome_2) = session
        .submit_input("second turn".into())
        .await
        .expect("turn 2 must succeed");
    assert_eq!(events_2.len(), 2, "turn 2 events: {events_2:?}");
    assert!(outcome_2.contains("Hello from daemon!"));

    session.disconnect().await.expect("disconnect must succeed");

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), daemon_handle)
        .await
        .expect("daemon must finish within 5s")
        .expect("daemon task must not panic");
}

// ---------------------------------------------------------------------------
// Phase 17 Task 4 — daemon_is_running utility test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn daemon_is_running_returns_false_for_absent_socket() {
    let scratch = ScratchDir::new();
    let socket_path = scratch.socket_path();
    assert!(
        !daemon_is_running(&socket_path).await,
        "daemon_is_running must return false when no daemon is listening"
    );
}

// ---------------------------------------------------------------------------
// Phase 18 Task 2 — run_daemon_session REPL integration test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_daemon_session_renders_two_turns() {
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
        run_daemon_compat(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let config = DaemonSessionConfig {
        socket_path: socket_path.clone(),
        role: None,
        prompt: "> ".into(),
        banner: Some("test banner".into()),
        cancel_flag: None,
        frontend_type: None,
    };

    // Two input lines, then EOF.
    let input = std::io::Cursor::new(b"first turn\nsecond turn\n");
    let mut output = Vec::<u8>::new();

    let report = run_daemon_session(config, input, &mut output)
        .await
        .expect("run_daemon_session must succeed");

    assert_eq!(report.turns_run, 2, "must run exactly 2 turns");
    assert!(report.last_outcome.is_some(), "must have a last outcome");

    let output_str = String::from_utf8(output).expect("output must be valid UTF-8");
    assert!(
        output_str.contains("test banner"),
        "output must contain banner, got: {output_str}"
    );
    assert!(
        output_str.contains("Hello "),
        "output must contain streamed text, got: {output_str}"
    );
    assert!(
        output_str.contains("from daemon!"),
        "output must contain streamed text, got: {output_str}"
    );
    // Three prompts: before turn 1, before turn 2, before the EOF read.
    assert_eq!(
        output_str.matches("> ").count(),
        3,
        "must have exactly 3 prompts in output, got: {output_str}"
    );

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), daemon_handle)
        .await
        .expect("daemon must finish within 5s")
        .expect("daemon task must not panic");
}

// ---------------------------------------------------------------------------
// Phase 18 Task 2 — run_daemon_session banner-only test (empty input)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_daemon_session_with_no_input_prints_banner_only() {
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
        run_daemon_compat(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let config = DaemonSessionConfig {
        socket_path: socket_path.clone(),
        role: None,
        prompt: "> ".into(),
        banner: Some("daemon-mode banner".into()),
        cancel_flag: None,
        frontend_type: None,
    };

    // Empty input — immediate EOF.
    let input = std::io::Cursor::new(b"");
    let mut output = Vec::<u8>::new();

    let report = run_daemon_session(config, input, &mut output)
        .await
        .expect("run_daemon_session must succeed");

    assert_eq!(report.turns_run, 0, "no turns should run on empty input");
    assert!(report.last_outcome.is_none(), "no outcome on empty input");

    let output_str = String::from_utf8(output).expect("output must be valid UTF-8");
    assert!(
        output_str.contains("daemon-mode banner"),
        "output must contain banner, got: {output_str}"
    );

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), daemon_handle)
        .await
        .expect("daemon must finish within 5s")
        .expect("daemon task must not panic");
}

// ---------------------------------------------------------------------------
// Phase 18 Task 3 — run_daemon_session_connected (pre-connected) test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_daemon_session_connected_with_cancel_handle() {
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
        run_daemon_compat(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Pre-connect (as the binary does in daemon mode).
    let session = DaemonSession::connect(&socket_path, None, None)
        .await
        .expect("connect must succeed");

    let cancel_handle = session.cancel_handle();
    assert!(!session.session_id.is_empty());

    // Verify the cancel handle is cloneable and has the right session ID.
    let _handle2 = cancel_handle.clone();

    let config = DaemonSessionConfig {
        socket_path: socket_path.clone(),
        role: None,
        prompt: "> ".into(),
        banner: Some("pre-connected test".into()),
        cancel_flag: None,
        frontend_type: None,
    };

    let input = std::io::Cursor::new(b"hello\n");
    let mut output = Vec::<u8>::new();

    let report = run_daemon_session_connected(session, config, input, &mut output)
        .await
        .expect("run_daemon_session_connected must succeed");

    assert_eq!(report.turns_run, 1);

    let output_str = String::from_utf8(output).expect("valid UTF-8");
    assert!(output_str.contains("pre-connected test"));
    assert!(output_str.contains("Hello "));
    assert!(output_str.contains("from daemon!"));

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), daemon_handle)
        .await
        .expect("daemon must finish within 5s")
        .expect("daemon task must not panic");
}

#[tokio::test]
async fn cancel_flag_resets_between_turns() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

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
        run_daemon_compat(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let session = DaemonSession::connect(&socket_path, None, None)
        .await
        .expect("connect must succeed");

    // Simulate a prior cancel: flag starts true.
    let cancel_flag = Arc::new(AtomicBool::new(true));

    let config = DaemonSessionConfig {
        socket_path: socket_path.clone(),
        role: None,
        prompt: "> ".into(),
        banner: Some("flag-reset test".into()),
        cancel_flag: Some(Arc::clone(&cancel_flag)),
        frontend_type: None,
    };

    let input = std::io::Cursor::new(b"turn1\nturn2\n");
    let mut output = Vec::<u8>::new();

    let report = run_daemon_session_connected(session, config, input, &mut output)
        .await
        .expect("session must succeed");

    assert_eq!(report.turns_run, 2);
    // After the last turn completes, the flag should still be false
    // (the REPL resets it before each submit_input).
    assert!(!cancel_flag.load(Ordering::Relaxed));

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), daemon_handle)
        .await
        .expect("daemon must finish within 5s")
        .expect("daemon task must not panic");
}

// ---------------------------------------------------------------------------
// Phase 19 Task 2 — two concurrent connections
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_concurrent_connections() {
    use aivyx_channel::daemon_server::{run_daemon, ChannelFactory};

    let scratch = ScratchDir::new();
    let socket_path = scratch.socket_path();

    let agent: Arc<dyn Agent> = Arc::new(FakeStreamingAgent {
        id: AgentId::new(),
        caps: CapabilitySet::empty(),
    });

    let channel = Arc::new(LocalChannel::new("daemon-test", Vec::<u8>::new()));
    let shutdown = CancellationToken::new();

    let channel_for_factory: Arc<dyn aivyx_core::ChannelContext + Send + Sync> = channel;
    let factory: ChannelFactory = Arc::new(move |_| Arc::clone(&channel_for_factory));

    let daemon_socket = socket_path.clone();
    let daemon_agent = Arc::clone(&agent);
    let daemon_factory = Arc::clone(&factory);
    let daemon_shutdown = shutdown.clone();
    let daemon_handle = tokio::spawn(async move {
        run_daemon(&daemon_socket, daemon_agent, daemon_factory, daemon_shutdown, None, None, None, None, None, None)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect two clients concurrently.
    let mut session_a = DaemonSession::connect(&socket_path, None, None)
        .await
        .expect("connection A must succeed");
    let mut session_b = DaemonSession::connect(&socket_path, None, None)
        .await
        .expect("connection B must succeed");

    // Both sessions should have different session IDs.
    assert_ne!(session_a.session_id, session_b.session_id);

    // Submit turns on both connections.
    let (events_a, outcome_a) = session_a
        .submit_input("from A".into())
        .await
        .expect("turn A must succeed");
    let (events_b, outcome_b) = session_b
        .submit_input("from B".into())
        .await
        .expect("turn B must succeed");

    assert_eq!(events_a.len(), 2);
    assert_eq!(events_b.len(), 2);
    assert!(outcome_a.contains("Hello from daemon!"));
    assert!(outcome_b.contains("Hello from daemon!"));

    let _ = session_a.disconnect().await;
    let _ = session_b.disconnect().await;

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), daemon_handle)
        .await
        .expect("daemon must finish within 5s")
        .expect("daemon task must not panic");
}

// ---------------------------------------------------------------------------
// Phase 19 Task 2 — connection after disconnect
// ---------------------------------------------------------------------------

#[tokio::test]
async fn connection_after_disconnect() {
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
        run_daemon_compat(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // First connection: one turn, then disconnect.
    let mut session_1 = DaemonSession::connect(&socket_path, None, None)
        .await
        .expect("connection 1 must succeed");
    let (events_1, _) = session_1
        .submit_input("first".into())
        .await
        .expect("turn 1 must succeed");
    assert_eq!(events_1.len(), 2);
    let _ = session_1.disconnect().await;

    // Small delay to let the handler task finish.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Second connection: the daemon should still be accepting.
    let mut session_2 = DaemonSession::connect(&socket_path, None, None)
        .await
        .expect("connection 2 must succeed after first disconnected");
    let (events_2, _) = session_2
        .submit_input("second".into())
        .await
        .expect("turn 2 must succeed");
    assert_eq!(events_2.len(), 2);
    let _ = session_2.disconnect().await;

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), daemon_handle)
        .await
        .expect("daemon must finish within 5s")
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

// ---------------------------------------------------------------------------
// PlatformEchoAgent — echoes the channel's platform in the turn outcome.
// ---------------------------------------------------------------------------

struct PlatformEchoAgent {
    id: AgentId,
    caps: CapabilitySet,
}

#[async_trait]
impl Agent for PlatformEchoAgent {
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
        let platform = format!("{:?}", channel.platform());
        let tier = format!("{:?}", channel.trust_tier());
        let text = format!("platform={platform} tier={tier}");
        let _ = channel.stream_event(StreamEvent::Text(&text)).await;

        TurnOutcome::Completed {
            final_message: text,
            tool_calls_made: 0,
            duration: Duration::from_millis(1),
        }
    }
}

// ---------------------------------------------------------------------------
// TelegramDaemonChannel — identity stub for Telegram frontend type tests.
// ---------------------------------------------------------------------------

struct TestTelegramChannel {
    session: aivyx_core::SessionId,
    token: CancellationToken,
}

impl TestTelegramChannel {
    fn new() -> Self {
        TestTelegramChannel {
            session: aivyx_core::SessionId::new(),
            token: CancellationToken::new(),
        }
    }
}

#[async_trait]
impl ChannelContext for TestTelegramChannel {
    fn channel_name(&self) -> &str {
        "test-telegram-daemon"
    }

    fn platform(&self) -> aivyx_core::ChannelPlatform {
        aivyx_core::ChannelPlatform::Telegram
    }

    fn trust_tier(&self) -> aivyx_capability::TrustTier {
        aivyx_capability::TrustTier::SemiTrusted
    }

    fn session_id(&self) -> aivyx_core::SessionId {
        self.session
    }

    async fn stream_event(
        &self,
        _event: StreamEvent<'_>,
    ) -> Result<(), aivyx_core::ChannelError> {
        Ok(())
    }

    async fn finalize(
        &self,
        _outcome: &TurnOutcome,
    ) -> Result<(), aivyx_core::ChannelError> {
        Ok(())
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.token.clone()
    }
}

// ---------------------------------------------------------------------------
// Phase 19 Task 3 — Telegram frontend type dispatches through ChannelFactory.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn telegram_frontend_type_gets_telegram_channel() {
    let scratch = ScratchDir::new();
    let socket_path = scratch.socket_path();

    let agent: Arc<dyn Agent> = Arc::new(PlatformEchoAgent {
        id: AgentId::new(),
        caps: CapabilitySet::empty(),
    });

    let factory: ChannelFactory = Arc::new(|ft| match ft {
        FrontendType::Telegram => Arc::new(TestTelegramChannel::new()),
        FrontendType::Local | FrontendType::Web => {
            Arc::new(LocalChannel::new("test-local", Vec::<u8>::new()))
        }
    });

    let shutdown = CancellationToken::new();
    let daemon_socket = socket_path.clone();
    let daemon_agent = Arc::clone(&agent);
    let daemon_shutdown = shutdown.clone();
    let daemon_handle = tokio::spawn(async move {
        run_daemon(&daemon_socket, daemon_agent, factory, daemon_shutdown, None, None, None, None, None, None)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut session = DaemonSession::connect(
        &socket_path,
        None,
        Some(FrontendType::Telegram),
    )
    .await
    .expect("connect must succeed");

    let (events, outcome) = session
        .submit_input("hello".to_string())
        .await
        .expect("submit must succeed");

    assert!(
        outcome.contains("Telegram"),
        "outcome must report Telegram platform: {outcome}"
    );
    assert!(
        outcome.contains("SemiTrusted"),
        "outcome must report SemiTrusted tier: {outcome}"
    );
    assert!(
        !events.is_empty(),
        "must receive at least one stream event"
    );

    let _ = session.disconnect().await;

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), daemon_handle).await;
}

#[tokio::test]
async fn mixed_local_and_telegram_frontends_on_same_daemon() {
    let scratch = ScratchDir::new();
    let socket_path = scratch.socket_path();

    let agent: Arc<dyn Agent> = Arc::new(PlatformEchoAgent {
        id: AgentId::new(),
        caps: CapabilitySet::empty(),
    });

    let factory: ChannelFactory = Arc::new(|ft| match ft {
        FrontendType::Telegram => Arc::new(TestTelegramChannel::new()),
        FrontendType::Local | FrontendType::Web => {
            Arc::new(LocalChannel::new("test-local", Vec::<u8>::new()))
        }
    });

    let shutdown = CancellationToken::new();
    let daemon_socket = socket_path.clone();
    let daemon_agent = Arc::clone(&agent);
    let daemon_shutdown = shutdown.clone();
    let daemon_handle = tokio::spawn(async move {
        run_daemon(&daemon_socket, daemon_agent, factory, daemon_shutdown, None, None, None, None, None, None)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect a Local frontend.
    let mut local_session = DaemonSession::connect(
        &socket_path,
        None,
        Some(FrontendType::Local),
    )
    .await
    .expect("local connect must succeed");

    // Connect a Telegram frontend.
    let mut tg_session = DaemonSession::connect(
        &socket_path,
        None,
        Some(FrontendType::Telegram),
    )
    .await
    .expect("telegram connect must succeed");

    // Submit turns on both.
    let (_local_events, local_outcome) = local_session
        .submit_input("hi".to_string())
        .await
        .expect("local submit must succeed");

    let (_tg_events, tg_outcome) = tg_session
        .submit_input("hi".to_string())
        .await
        .expect("telegram submit must succeed");

    // Local should report Local platform + Trusted tier.
    assert!(
        local_outcome.contains("Local"),
        "local outcome must report Local platform: {local_outcome}"
    );
    assert!(
        local_outcome.contains("Trusted"),
        "local outcome must report Trusted tier: {local_outcome}"
    );

    // Telegram should report Telegram platform + SemiTrusted tier.
    assert!(
        tg_outcome.contains("Telegram"),
        "telegram outcome must report Telegram platform: {tg_outcome}"
    );
    assert!(
        tg_outcome.contains("SemiTrusted"),
        "telegram outcome must report SemiTrusted tier: {tg_outcome}"
    );

    // Different session IDs.
    assert_ne!(
        local_session.session_id, tg_session.session_id,
        "different frontends must get different session IDs"
    );

    let _ = local_session.disconnect().await;
    let _ = tg_session.disconnect().await;

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), daemon_handle).await;
}

// ---------------------------------------------------------------------------
// Phase 20 Task 2 — daemon_stop triggers graceful shutdown
// ---------------------------------------------------------------------------

#[tokio::test]
async fn daemon_stop_triggers_graceful_shutdown() {
    let scratch = ScratchDir::new();
    let socket_path = scratch.socket_path();

    let agent: Arc<dyn Agent> = Arc::new(FakeStreamingAgent {
        id: AgentId::new(),
        caps: CapabilitySet::empty(),
    });

    let channel = Arc::new(LocalChannel::new("daemon-stop-test", Vec::<u8>::new()));
    let shutdown = CancellationToken::new();

    let daemon_socket = socket_path.clone();
    let daemon_agent = Arc::clone(&agent);
    let daemon_channel = Arc::clone(&channel);
    let daemon_shutdown = shutdown.clone();
    let daemon_handle = tokio::spawn(async move {
        run_daemon_compat(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        daemon_is_running(&socket_path).await,
        "daemon must be running before stop"
    );

    let reason = aivyx_channel::daemon_client::daemon_stop(&socket_path)
        .await
        .expect("daemon_stop must succeed");
    assert!(
        reason.contains("operator requested"),
        "shutdown reason must mention operator: {reason}"
    );

    tokio::time::timeout(Duration::from_secs(5), daemon_handle)
        .await
        .expect("daemon must exit within 5s after stop")
        .expect("daemon task must not panic");
}

#[tokio::test]
async fn daemon_status_reports_running_daemon() {
    let scratch = ScratchDir::new();
    let socket_path = scratch.socket_path();

    let agent: Arc<dyn Agent> = Arc::new(FakeStreamingAgent {
        id: AgentId::new(),
        caps: CapabilitySet::empty(),
    });

    let channel = Arc::new(LocalChannel::new("daemon-status-test", Vec::<u8>::new()));
    let shutdown = CancellationToken::new();

    let daemon_socket = socket_path.clone();
    let daemon_agent = Arc::clone(&agent);
    let daemon_channel = Arc::clone(&channel);
    let daemon_shutdown = shutdown.clone();
    let daemon_handle = tokio::spawn(async move {
        run_daemon_compat(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let info = aivyx_channel::daemon_client::daemon_status(&socket_path).await;
    assert!(info.running, "daemon must report as running");
    assert_eq!(
        info.version.as_deref(),
        Some("0.1"),
        "daemon must report protocol version 0.1"
    );

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), daemon_handle).await;
}

#[tokio::test]
async fn daemon_status_reports_not_running_for_absent_socket() {
    let scratch = ScratchDir::new();
    let socket_path = scratch.socket_path();

    let info = aivyx_channel::daemon_client::daemon_status(&socket_path).await;
    assert!(!info.running, "daemon must report as not running");
    assert!(info.version.is_none(), "version must be None when not running");
}

// ---------------------------------------------------------------------------
// Phase 20 Task 3 — PID file lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pid_file_appears_on_daemon_start_and_disappears_on_stop() {
    let scratch = ScratchDir::new();
    let socket_path = scratch.socket_path();
    let pid_path = socket_path.with_extension("pid");

    assert!(
        !pid_path.exists(),
        "PID file must not exist before daemon start"
    );

    let agent: Arc<dyn Agent> = Arc::new(FakeStreamingAgent {
        id: AgentId::new(),
        caps: CapabilitySet::empty(),
    });

    let channel = Arc::new(LocalChannel::new("pid-test", Vec::<u8>::new()));
    let shutdown = CancellationToken::new();

    let daemon_socket = socket_path.clone();
    let daemon_agent = Arc::clone(&agent);
    let daemon_channel = Arc::clone(&channel);
    let daemon_shutdown = shutdown.clone();
    let daemon_handle = tokio::spawn(async move {
        run_daemon_compat(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(pid_path.exists(), "PID file must exist while daemon runs");
    let pid_content = std::fs::read_to_string(&pid_path)
        .expect("PID file must be readable");
    let pid: u32 = pid_content.trim().parse()
        .expect("PID file must contain a valid u32");
    assert!(pid > 0, "PID must be positive");

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), daemon_handle)
        .await
        .expect("daemon must exit within 5s")
        .expect("daemon task must not panic");

    assert!(
        !pid_path.exists(),
        "PID file must be removed after daemon shutdown"
    );
}

#[tokio::test]
async fn daemon_status_includes_pid_from_pid_file() {
    let scratch = ScratchDir::new();
    let socket_path = scratch.socket_path();

    let agent: Arc<dyn Agent> = Arc::new(FakeStreamingAgent {
        id: AgentId::new(),
        caps: CapabilitySet::empty(),
    });

    let channel = Arc::new(LocalChannel::new("pid-status-test", Vec::<u8>::new()));
    let shutdown = CancellationToken::new();

    let daemon_socket = socket_path.clone();
    let daemon_agent = Arc::clone(&agent);
    let daemon_channel = Arc::clone(&channel);
    let daemon_shutdown = shutdown.clone();
    let daemon_handle = tokio::spawn(async move {
        run_daemon_compat(&daemon_socket, daemon_agent, daemon_channel, daemon_shutdown)
            .await
            .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let info = aivyx_channel::daemon_client::daemon_status(&socket_path).await;
    assert!(info.running, "daemon must report as running");
    assert!(info.pid.is_some(), "daemon status must include PID");
    assert!(info.pid.unwrap() > 0, "PID must be positive");

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), daemon_handle).await;
}

#[test]
fn read_pid_file_returns_none_for_missing_file() {
    let result = aivyx_channel::daemon_client::read_pid_file(
        std::path::Path::new("/nonexistent/daemon.pid")
    );
    assert!(result.is_none());
}

#[test]
fn read_pid_file_returns_none_for_non_numeric_content() {
    let dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let path = std::path::PathBuf::from(dir)
        .join(format!("aivyx-pid-test-{pid}-{nanos}.pid"));
    std::fs::write(&path, "not-a-number").expect("write test PID file");
    let result = aivyx_channel::daemon_client::read_pid_file(&path);
    let _ = std::fs::remove_file(&path);
    assert!(result.is_none());
}

// ---------------------------------------------------------------------------
// FakeEscalatingAgent — returns TurnOutcome::Escalated on the first turn,
// then Completed on subsequent turns (simulating the resume after gate
// approval).
// ---------------------------------------------------------------------------

struct FakeEscalatingAgent {
    id: AgentId,
    caps: CapabilitySet,
    turn_count: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl Agent for FakeEscalatingAgent {
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
        let n = self.turn_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            let _ = channel.stream_event(StreamEvent::Text("escalating...")).await;
            TurnOutcome::Escalated {
                reason: "requires approval".into(),
                pending_tool: aivyx_core::ToolId::new(),
                tool_calls_made: 1,
            }
        } else {
            let _ = channel.stream_event(StreamEvent::Text("resumed after approval")).await;
            TurnOutcome::Completed {
                final_message: "mission continued".into(),
                tool_calls_made: 0,
                duration: Duration::from_millis(1),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Escalation→gate turn-loop wiring integration test (Phase 23 Task 2).
//
// Verifies: submit a turn with mission_id → agent escalates →
// daemon creates gate + emits ApprovalGate → resolve gate approved →
// daemon resumes turn → TurnComplete.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn escalation_gate_wiring_approve_resumes_turn() {
    use aivyx_channel::mission::{self, MissionRecord};
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};

    let scratch = ScratchDir::new();
    let socket_path = scratch.socket_path();
    let daemon_socket = socket_path.clone();

    let store_path = scratch.path.join("test.redb");
    let storage: Arc<dyn Storage> = RedbStorage::open(
        StorageConfig::new(store_path),
        MasterKey::from_raw([7u8; 32]),
    )
    .await
    .expect("storage must open");
    let mission_handle = storage.domain(KeyDomain::Missions);

    let mission_id = format!("m-{}", uuid::Uuid::new_v4());
    let record = MissionRecord::new(
        mission_id.clone(),
        "default".into(),
        "test mission".into(),
    );
    mission::create_mission(&mission_handle, &record)
        .await
        .expect("create mission must succeed");

    let mut record = mission::get_mission(&mission_handle, &mission_id)
        .await
        .expect("get mission")
        .expect("mission must exist");
    mission::transition_to_running(&mut record).expect("start mission");
    mission::update_mission(&mission_handle, &record)
        .await
        .expect("persist started mission");

    let verify_handle = storage.domain(KeyDomain::Missions);

    let agent: Arc<dyn Agent + Send + Sync> = Arc::new(FakeEscalatingAgent {
        id: AgentId::new(),
        caps: CapabilitySet::empty(),
        turn_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let daemon_agent = Arc::clone(&agent);
    let shutdown = CancellationToken::new();
    let daemon_shutdown = shutdown.clone();

    let factory: ChannelFactory = Arc::new(move |_ft| {
        let ch: Arc<dyn ChannelContext + Send + Sync> =
            Arc::new(LocalChannel::new("gate-test", Vec::<u8>::new()));
        ch
    });

    let daemon_handle = tokio::spawn(async move {
        run_daemon(
            &daemon_socket,
            daemon_agent,
            factory,
            daemon_shutdown,
            Some(mission_handle),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("daemon must complete successfully");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let stream = UnixStream::connect(&socket_path)
        .await
        .expect("connect to daemon");
    let (mut reader, mut writer) = stream.into_split();

    let mut buf = Vec::new();

    // --- Handshake ---
    read_more(&mut reader, &mut buf).await;
    match decode_frame::<DaemonEnvelope>(&buf) {
        Ok((DaemonEnvelope::DaemonReady { .. }, consumed)) => {
            buf.drain(..consumed);
        }
        other => panic!("expected DaemonReady, got {other:?}"),
    }

    let start = FrontendMessage::StartSession {
        role: None,
        frontend_type: Some(FrontendType::Local),
    };
    let frame = encode_frame(&start).unwrap();
    writer.write_all(&frame).await.unwrap();

    loop {
        read_more(&mut reader, &mut buf).await;
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((DaemonEnvelope::SessionStarted { session_id, .. }, consumed)) => {
                buf.drain(..consumed);
                let _ = session_id;
                break;
            }
            Err(FrameError::IncompleteBuf) => continue,
            other => panic!("expected SessionStarted, got {other:?}"),
        }
    }

    // --- Turn 1: submit with mission_id → expect escalation + gate ---
    let submit = FrontendMessage::SubmitInput {
        session_id: "s1".into(),
        text: "do something risky".into(),
        mission_id: Some(mission_id.clone()),
    };
    let frame = encode_frame(&submit).unwrap();
    writer.write_all(&frame).await.unwrap();

    let mut gate_event: Option<(String, String)> = None;
    #[allow(unused_assignments)] // Initial false is the default; overwritten in loop body
    let mut saw_escalated_outcome = false;
    let mut events = Vec::new();

    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((DaemonEnvelope::StreamEvent { event, .. }, consumed)) => {
                buf.drain(..consumed);
                if let StreamEventPayload::ApprovalGate {
                    ref mission_id,
                    ref gate_id,
                    ..
                } = event
                {
                    gate_event = Some((mission_id.clone(), gate_id.clone()));
                }
                events.push(event);
            }
            Ok((DaemonEnvelope::TurnComplete { outcome, .. }, consumed)) => {
                buf.drain(..consumed);
                assert!(
                    outcome.contains("escalated"),
                    "expected escalated outcome, got: {outcome}"
                );
                saw_escalated_outcome = true;
                break;
            }
            Ok((DaemonEnvelope::Error { code, message }, _)) => {
                panic!("daemon error ({code}): {message}");
            }
            Err(FrameError::IncompleteBuf) => {
                read_more(&mut reader, &mut buf).await;
            }
            Ok((other, consumed)) => {
                buf.drain(..consumed);
                panic!("unexpected message: {other:?}");
            }
            Err(e) => panic!("decode error: {e}"),
        }
    }

    assert!(saw_escalated_outcome, "must see escalated TurnComplete");
    let (gate_mid, gate_gid) =
        gate_event.expect("must receive ApprovalGate stream event");
    assert_eq!(gate_mid, mission_id, "gate mission_id must match");

    // Verify mission is now GatePending in storage.
    let stored = mission::get_mission(&verify_handle, &mission_id)
        .await
        .expect("get mission")
        .expect("mission must exist");
    assert_eq!(
        stored.state,
        aivyx_channel::mission::MissionState::GatePending,
        "mission must be GatePending after escalation"
    );
    assert_eq!(stored.gates.len(), 1, "must have exactly one gate");

    // --- Resolve gate (approved) → expect resume turn ---
    let resolve = FrontendMessage::ResolveGate {
        mission_id: mission_id.clone(),
        gate_id: gate_gid.clone(),
        approved: true,
    };
    let frame = encode_frame(&resolve).unwrap();
    writer.write_all(&frame).await.unwrap();

    let mut saw_gate_resolved = false;
    #[allow(unused_assignments)]
    let mut saw_resume_complete = false;

    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((DaemonEnvelope::GateResolved { approved, .. }, consumed)) => {
                buf.drain(..consumed);
                assert!(approved, "gate must be approved");
                saw_gate_resolved = true;
            }
            Ok((DaemonEnvelope::StreamEvent { event, .. }, consumed)) => {
                buf.drain(..consumed);
                events.push(event);
            }
            Ok((DaemonEnvelope::TurnComplete { outcome, .. }, consumed)) => {
                buf.drain(..consumed);
                assert!(
                    outcome.contains("mission continued"),
                    "resume outcome: {outcome}"
                );
                saw_resume_complete = true;
                break;
            }
            Ok((DaemonEnvelope::Error { code, message }, _)) => {
                panic!("daemon error during resume ({code}): {message}");
            }
            Err(FrameError::IncompleteBuf) => {
                read_more(&mut reader, &mut buf).await;
            }
            Ok((other, consumed)) => {
                buf.drain(..consumed);
                panic!("unexpected message during resume: {other:?}");
            }
            Err(e) => panic!("decode error during resume: {e}"),
        }
    }

    assert!(saw_gate_resolved, "must see GateResolved");
    assert!(saw_resume_complete, "must see resume TurnComplete");

    // Verify mission is back to Running after gate approval.
    let stored = mission::get_mission(&verify_handle, &mission_id)
        .await
        .expect("get mission")
        .expect("mission must exist");
    assert_eq!(
        stored.state,
        aivyx_channel::mission::MissionState::Running,
        "mission must be Running after approved gate"
    );

    // Clean up.
    let disconnect = FrontendMessage::Disconnect;
    let frame = encode_frame(&disconnect).unwrap();
    let _ = writer.write_all(&frame).await;

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), daemon_handle).await;
}

#[tokio::test]
async fn escalation_gate_wiring_reject_fails_mission() {
    use aivyx_channel::mission::{self, MissionRecord};
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};

    let scratch = ScratchDir::new();
    let socket_path = scratch.socket_path();
    let daemon_socket = socket_path.clone();

    let store_path = scratch.path.join("test.redb");
    let storage: Arc<dyn Storage> = RedbStorage::open(
        StorageConfig::new(store_path),
        MasterKey::from_raw([7u8; 32]),
    )
    .await
    .expect("storage must open");
    let mission_handle = storage.domain(KeyDomain::Missions);

    let mission_id = format!("m-{}", uuid::Uuid::new_v4());
    let record = MissionRecord::new(
        mission_id.clone(),
        "default".into(),
        "test mission".into(),
    );
    mission::create_mission(&mission_handle, &record)
        .await
        .expect("create mission");

    let mut record = mission::get_mission(&mission_handle, &mission_id)
        .await
        .expect("get")
        .expect("exists");
    mission::transition_to_running(&mut record).expect("start");
    mission::update_mission(&mission_handle, &record)
        .await
        .expect("persist");

    let verify_handle = storage.domain(KeyDomain::Missions);

    let agent: Arc<dyn Agent + Send + Sync> = Arc::new(FakeEscalatingAgent {
        id: AgentId::new(),
        caps: CapabilitySet::empty(),
        turn_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let daemon_agent = Arc::clone(&agent);
    let shutdown = CancellationToken::new();
    let daemon_shutdown = shutdown.clone();

    let factory: ChannelFactory = Arc::new(move |_ft| {
        let ch: Arc<dyn ChannelContext + Send + Sync> =
            Arc::new(LocalChannel::new("gate-test", Vec::<u8>::new()));
        ch
    });

    let daemon_handle = tokio::spawn(async move {
        run_daemon(
            &daemon_socket,
            daemon_agent,
            factory,
            daemon_shutdown,
            Some(mission_handle),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("daemon must complete");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let stream = UnixStream::connect(&socket_path)
        .await
        .expect("connect");
    let (mut reader, mut writer) = stream.into_split();
    let mut buf = Vec::new();

    // Handshake.
    read_more(&mut reader, &mut buf).await;
    match decode_frame::<DaemonEnvelope>(&buf) {
        Ok((DaemonEnvelope::DaemonReady { .. }, consumed)) => buf.drain(..consumed),
        other => panic!("expected DaemonReady: {other:?}"),
    };

    let start = FrontendMessage::StartSession {
        role: None,
        frontend_type: Some(FrontendType::Local),
    };
    writer.write_all(&encode_frame(&start).unwrap()).await.unwrap();
    loop {
        read_more(&mut reader, &mut buf).await;
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((DaemonEnvelope::SessionStarted { .. }, consumed)) => {
                buf.drain(..consumed);
                break;
            }
            Err(FrameError::IncompleteBuf) => continue,
            other => panic!("expected SessionStarted: {other:?}"),
        }
    }

    // Submit with mission → triggers escalation.
    let submit = FrontendMessage::SubmitInput {
        session_id: "s1".into(),
        text: "do something".into(),
        mission_id: Some(mission_id.clone()),
    };
    writer.write_all(&encode_frame(&submit).unwrap()).await.unwrap();

    let mut gate_gid = String::new();
    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((DaemonEnvelope::StreamEvent { event, .. }, consumed)) => {
                buf.drain(..consumed);
                if let StreamEventPayload::ApprovalGate { gate_id, .. } = &event {
                    gate_gid = gate_id.clone();
                }
            }
            Ok((DaemonEnvelope::TurnComplete { .. }, consumed)) => {
                buf.drain(..consumed);
                break;
            }
            Err(FrameError::IncompleteBuf) => read_more(&mut reader, &mut buf).await,
            Ok((other, consumed)) => {
                buf.drain(..consumed);
                panic!("unexpected: {other:?}");
            }
            Err(e) => panic!("decode: {e}"),
        }
    }
    assert!(!gate_gid.is_empty(), "must have gate_id");

    // Reject the gate.
    let resolve = FrontendMessage::ResolveGate {
        mission_id: mission_id.clone(),
        gate_id: gate_gid,
        approved: false,
    };
    writer.write_all(&encode_frame(&resolve).unwrap()).await.unwrap();

    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((DaemonEnvelope::GateResolved { approved, .. }, consumed)) => {
                buf.drain(..consumed);
                assert!(!approved, "gate must be rejected");
                break;
            }
            Err(FrameError::IncompleteBuf) => read_more(&mut reader, &mut buf).await,
            Ok((other, consumed)) => {
                buf.drain(..consumed);
                panic!("unexpected during reject: {other:?}");
            }
            Err(e) => panic!("decode: {e}"),
        }
    }

    // Verify mission is Failed after rejection.
    let stored = mission::get_mission(&verify_handle, &mission_id)
        .await
        .expect("get mission")
        .expect("must exist");
    assert_eq!(
        stored.state,
        aivyx_channel::mission::MissionState::Failed,
        "mission must be Failed after gate rejection"
    );

    let disconnect = FrontendMessage::Disconnect;
    let _ = writer.write_all(&encode_frame(&disconnect).unwrap()).await;
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(5), daemon_handle).await;
}

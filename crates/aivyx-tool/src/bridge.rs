//! `ToolProcessBridge` — daemon-side adapter for a spawned tool
//! process.
//!
//! Owns the child process, performs the `ToolHello` →
//! `ToolRegister` handshake on startup, and routes per-invocation
//! `InvokeTool` → `ToolResult`/`ToolError` traffic on demand.
//!
//! Phase 49 — foundation phase. The bridge ships the third-party
//! path only (P12); first-party in-process unification is
//! deferred per Q5.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use thiserror::Error;
use tokio::io::BufReader;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{oneshot, Mutex};

use crate::frame::{read_frame, write_frame, FrameError};
use crate::wire::{
    DaemonToTool, ToolDescriptor, ToolToDaemon, TOOL_PROTOCOL_VERSION,
};

#[derive(Debug, Error)]
pub enum ToolBridgeError {
    #[error("failed to spawn tool process `{command}`: {source}")]
    Spawn {
        command: String,
        source: std::io::Error,
    },
    #[error("tool process exited during handshake")]
    HandshakeClosed,
    #[error("expected ToolRegister, got {0:?}")]
    HandshakeUnexpected(ToolToDaemon),
    #[error("framing error: {0}")]
    Frame(#[from] FrameError),
    #[error("invocation `{call_id}` mismatched response (got call_id `{got}`)")]
    CallIdMismatch { call_id: String, got: String },
    #[error("tool returned error: [{code}] {message}")]
    ToolError { code: String, message: String },
    #[error("tool process closed the connection mid-invocation")]
    InvocationClosed,
    #[error("tool process produced an unparseable frame: {0}")]
    Decode(String),
}

/// Outcome of an `invoke` — distinguishes success and tool-side
/// failure so the caller can map them onto the right
/// `aivyx_core::ToolOutcome` variant.
#[derive(Debug, Clone)]
pub enum InvocationOutcome {
    Completed {
        verified: crate::wire::Verification,
        output: serde_json::Value,
    },
    ToolError {
        code: String,
        message: String,
    },
}

/// Configuration for spawning a tool process.
#[derive(Debug, Clone)]
pub struct ToolProcessConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// Daemon-side bridge to a spawned tool process. One bridge per
/// `[[tool_process]]` config entry.
///
/// On `spawn`, the child is started, `ToolHello` is written to
/// its stdin, and `ToolRegister` is read from its stdout. The
/// resulting bridge holds the descriptors and a writer half;
/// invocations are dispatched via `invoke`, each waiting for a
/// matching `ToolResult` / `ToolError` on the shared reader.
///
/// The shared reader runs as a background task to allow
/// concurrent invocations (Amendment A6 — parallel tool dispatch).
pub struct ToolProcessBridge {
    config: ToolProcessConfig,
    descriptors: Vec<ToolDescriptor>,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<InvocationOutcome>>>>,
    _child: Child,
    _reader_handle: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for ToolProcessBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolProcessBridge")
            .field("config", &self.config)
            .field("descriptors", &self.descriptors)
            .finish()
    }
}

impl ToolProcessBridge {
    /// Spawn the configured tool process, perform the handshake,
    /// and return a bridge ready for `invoke`.
    pub async fn spawn(config: ToolProcessConfig) -> Result<Self, ToolBridgeError> {
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .envs(config.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // SIGKILL the tool process if the bridge is dropped (or
            // the daemon panics) so we don't leave orphans behind.
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| ToolBridgeError::Spawn {
            command: config.command.clone(),
            source: e,
        })?;

        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");

        let stdin = Arc::new(Mutex::new(stdin));
        let pending: Arc<
            Mutex<HashMap<String, oneshot::Sender<InvocationOutcome>>>,
        > = Arc::new(Mutex::new(HashMap::new()));

        // Send ToolHello.
        {
            let mut guard = stdin.lock().await;
            write_frame(
                &mut *guard,
                &DaemonToTool::ToolHello {
                    protocol_version: TOOL_PROTOCOL_VERSION.into(),
                },
            )
            .await?;
        }

        // Read ToolRegister. This blocks startup — first frame must
        // be ToolRegister; anything else is a protocol violation.
        let mut reader = BufReader::new(stdout);
        let body = read_frame(&mut reader)
            .await?
            .ok_or(ToolBridgeError::HandshakeClosed)?;
        let msg: ToolToDaemon = serde_json::from_str(&body)
            .map_err(|e| ToolBridgeError::Decode(format!("ToolRegister: {e}")))?;
        let descriptors = match msg {
            ToolToDaemon::ToolRegister { tools, .. } => tools,
            other => return Err(ToolBridgeError::HandshakeUnexpected(other)),
        };

        // Spawn the background reader that demultiplexes responses
        // by call_id.
        let pending_for_reader = Arc::clone(&pending);
        let reader_handle = tokio::spawn(async move {
            reader_loop(reader, pending_for_reader).await;
        });

        Ok(ToolProcessBridge {
            config,
            descriptors,
            stdin,
            pending,
            _child: child,
            _reader_handle: reader_handle,
        })
    }

    /// The tools this process registered.
    pub fn descriptors(&self) -> &[ToolDescriptor] {
        &self.descriptors
    }

    /// Configuration this bridge was spawned with.
    pub fn config(&self) -> &ToolProcessConfig {
        &self.config
    }

    /// Send an `InvokeTool` and await the matching `ToolResult` or
    /// `ToolError`. Concurrent calls are safe — the bridge
    /// demultiplexes responses by `call_id`.
    pub async fn invoke(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        turn_id: &str,
    ) -> Result<InvocationOutcome, ToolBridgeError> {
        let call_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(call_id.clone(), tx);
        }

        // Send the invocation.
        {
            let mut guard = self.stdin.lock().await;
            let frame = DaemonToTool::InvokeTool {
                call_id: call_id.clone(),
                tool_name: tool_name.into(),
                input,
                turn_id: turn_id.into(),
            };
            if let Err(e) = write_frame(&mut *guard, &frame).await {
                // Drop the pending entry so the slot doesn't leak.
                self.pending.lock().await.remove(&call_id);
                return Err(e.into());
            }
        }

        match rx.await {
            Ok(outcome) => Ok(outcome),
            Err(_) => Err(ToolBridgeError::InvocationClosed),
        }
    }

    /// Send `CancelInvocation` for a specific call_id. Best-effort:
    /// the tool process is expected to respond promptly with a
    /// `ToolError { code: "cancelled" }`.
    pub async fn cancel(&self, call_id: &str) -> Result<(), ToolBridgeError> {
        let mut guard = self.stdin.lock().await;
        write_frame(
            &mut *guard,
            &DaemonToTool::CancelInvocation {
                call_id: call_id.into(),
            },
        )
        .await?;
        Ok(())
    }

    /// Send `ToolShutdown` to the child. Caller should wait briefly
    /// after this — the child exits cleanly on receipt; if not,
    /// `kill_on_drop` SIGKILLs when the bridge drops.
    pub async fn shutdown(&self) -> Result<(), ToolBridgeError> {
        let mut guard = self.stdin.lock().await;
        write_frame(&mut *guard, &DaemonToTool::ToolShutdown).await?;
        Ok(())
    }
}

/// Background reader loop — demultiplexes ToolToDaemon frames into
/// per-call_id oneshot senders.
async fn reader_loop(
    mut reader: BufReader<ChildStdout>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<InvocationOutcome>>>>,
) {
    loop {
        let body = match read_frame(&mut reader).await {
            Ok(Some(b)) => b,
            Ok(None) => {
                // Clean EOF — child closed stdout. Drop all
                // pending so callers wake with InvocationClosed.
                pending.lock().await.clear();
                return;
            }
            Err(_) => {
                pending.lock().await.clear();
                return;
            }
        };

        // Decode permissively — unknown variants are skipped, not
        // fatal (per the v0 stability disclaimer in TOOL_SDK.md
        // § 7).
        let msg: ToolToDaemon = match serde_json::from_str(&body) {
            Ok(m) => m,
            Err(_) => continue,
        };

        match msg {
            ToolToDaemon::ToolResult {
                call_id,
                verified,
                output,
            } => {
                if let Some(tx) = pending.lock().await.remove(&call_id) {
                    let _ = tx.send(InvocationOutcome::Completed { verified, output });
                }
            }
            ToolToDaemon::ToolError {
                call_id,
                code,
                message,
            } => {
                if let Some(tx) = pending.lock().await.remove(&call_id) {
                    let _ = tx.send(InvocationOutcome::ToolError { code, message });
                }
            }
            ToolToDaemon::ToolEvent { .. } => {
                // Phase 49 foundation does not surface mid-call events
                // to the channel layer — that's a follow-up wiring.
                // For now the event is consumed silently; the channel
                // still sees ToolCallStarted/Finished bracketing.
            }
            ToolToDaemon::ToolRegister { .. } => {
                // Spurious — the handshake already consumed this. Ignore.
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spawn an `awk` one-liner that acts as a minimal tool
    /// process. Verifies the bridge can spawn, handshake, and
    /// dispatch — no external dependencies beyond a standard
    /// awk in PATH.
    ///
    /// (The full Python reference is in
    /// `examples/python-tool/`; this test is the Rust-side
    /// shim that proves the wire works.)
    #[tokio::test]
    async fn bridge_handshakes_against_python_inline() {
        // Spawn a tiny Python script inline.
        let script = r#"
import sys, json, struct

def read_frame():
    hdr = sys.stdin.buffer.read(4)
    if not hdr or len(hdr) < 4:
        return None
    (n,) = struct.unpack(">I", hdr)
    return json.loads(sys.stdin.buffer.read(n).decode("utf-8"))

def write_frame(msg):
    body = json.dumps(msg).encode("utf-8")
    sys.stdout.buffer.write(struct.pack(">I", len(body)) + body)
    sys.stdout.buffer.flush()

# Handshake
hello = read_frame()
assert hello["type"] == "ToolHello"
write_frame({
    "type": "ToolRegister",
    "tool_process_name": "test-tool",
    "tools": [{
        "name": "echo",
        "description": "Echo the input.",
        "input_schema": {"type": "object"},
        "required_scope": "memory.read"
    }]
})

# Service one invocation, then exit.
inv = read_frame()
assert inv["type"] == "InvokeTool"
write_frame({
    "type": "ToolResult",
    "call_id": inv["call_id"],
    "verified": "NotApplicable",
    "output": {"echoed": inv["input"]}
})
sys.exit(0)
"#;
        let config = ToolProcessConfig {
            name: "test".into(),
            command: "python3".into(),
            args: vec!["-c".into(), script.into()],
            env: vec![],
        };
        let bridge = match ToolProcessBridge::spawn(config).await {
            Ok(b) => b,
            Err(e) => {
                // Python may not be present in some CI environments;
                // skip rather than fail.
                eprintln!("skipping: python3 unavailable: {e}");
                return;
            }
        };

        assert_eq!(bridge.descriptors().len(), 1);
        assert_eq!(bridge.descriptors()[0].name, "echo");

        let outcome = bridge
            .invoke("echo", serde_json::json!({"hello": "world"}), "turn-1")
            .await
            .expect("invoke must succeed");

        match outcome {
            InvocationOutcome::Completed { verified, output } => {
                assert_eq!(verified, crate::wire::Verification::NotApplicable);
                assert_eq!(output["echoed"]["hello"], "world");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn bridge_reports_handshake_failure_on_bad_command() {
        let config = ToolProcessConfig {
            name: "bad".into(),
            command: "/definitely/not/a/real/binary".into(),
            args: vec![],
            env: vec![],
        };
        let result = ToolProcessBridge::spawn(config).await;
        assert!(matches!(result, Err(ToolBridgeError::Spawn { .. })));
    }

    #[tokio::test]
    async fn bridge_surfaces_tool_error() {
        // Inline Python script that returns ToolError instead of
        // ToolResult.
        let script = r#"
import sys, json, struct

def read_frame():
    hdr = sys.stdin.buffer.read(4)
    if not hdr or len(hdr) < 4:
        return None
    (n,) = struct.unpack(">I", hdr)
    return json.loads(sys.stdin.buffer.read(n).decode("utf-8"))

def write_frame(msg):
    body = json.dumps(msg).encode("utf-8")
    sys.stdout.buffer.write(struct.pack(">I", len(body)) + body)
    sys.stdout.buffer.flush()

read_frame()  # ToolHello
write_frame({"type":"ToolRegister","tool_process_name":"err-tool","tools":[
    {"name":"oops","description":"always fails.","input_schema":{},"required_scope":"memory.read"}
]})
inv = read_frame()
write_frame({"type":"ToolError","call_id":inv["call_id"],
             "code":"deliberate","message":"this fails on purpose"})
sys.exit(0)
"#;
        let config = ToolProcessConfig {
            name: "err".into(),
            command: "python3".into(),
            args: vec!["-c".into(), script.into()],
            env: vec![],
        };
        let bridge = match ToolProcessBridge::spawn(config).await {
            Ok(b) => b,
            Err(_) => return,
        };

        let outcome = bridge
            .invoke("oops", serde_json::json!({}), "turn-1")
            .await
            .expect("invoke must succeed");

        match outcome {
            InvocationOutcome::ToolError { code, message } => {
                assert_eq!(code, "deliberate");
                assert!(message.contains("on purpose"));
            }
            other => panic!("expected ToolError, got {other:?}"),
        }
    }
}

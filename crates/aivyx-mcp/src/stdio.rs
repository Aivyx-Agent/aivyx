//! Stdio transport — communicates with an MCP server over
//! stdin/stdout of a child process using newline-delimited
//! JSON-RPC 2.0.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Child;
use tokio::sync::Mutex;

use crate::transport_trait::McpTransport;

/// Owns a child process and implements `McpTransport` over its
/// stdin/stdout pipes. The child is spawned with `kill_on_drop(true)`
/// so it is cleaned up when the transport is dropped.
pub struct StdioTransport {
    #[allow(dead_code)]
    child: Mutex<Child>,
    writer: Arc<Mutex<tokio::process::ChildStdin>>,
    reader: Arc<Mutex<BufReader<tokio::process::ChildStdout>>>,
}

impl StdioTransport {
    /// Spawn a child process and take ownership of its stdio handles.
    pub async fn start(
        command: &str,
        args: &[&str],
    ) -> Result<Self, String> {
        let mut child = tokio::process::Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("spawn MCP server: {e}"))?;

        let stdin = child.stdin.take().ok_or("no stdin on child")?;
        let stdout = child.stdout.take().ok_or("no stdout on child")?;

        Ok(StdioTransport {
            child: Mutex::new(child),
            writer: Arc::new(Mutex::new(stdin)),
            reader: Arc::new(Mutex::new(BufReader::new(stdout))),
        })
    }

    /// Kill the child process. Called during bridge shutdown.
    pub async fn kill(&self) {
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send(&self, message: &str) -> Result<(), String> {
        let mut w = self.writer.lock().await;
        w.write_all(message.as_bytes())
            .await
            .map_err(|e| format!("write to MCP server: {e}"))?;
        w.flush()
            .await
            .map_err(|e| format!("flush to MCP server: {e}"))?;
        Ok(())
    }

    async fn receive(&self) -> Result<String, String> {
        let mut line = String::new();
        let mut r = self.reader.lock().await;
        r.read_line(&mut line)
            .await
            .map_err(|e| format!("read from MCP server: {e}"))?;

        if line.is_empty() {
            return Err("MCP server closed stdout".into());
        }

        // Strip trailing newline for consistency with the trait contract.
        if line.ends_with('\n') {
            line.pop();
        }

        Ok(line)
    }
}

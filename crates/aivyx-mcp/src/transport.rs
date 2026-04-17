//! Stdio transport — spawns an MCP server as a child process and
//! communicates over stdin/stdout with newline-delimited JSON-RPC 2.0.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use aivyx_core::Tool;

use crate::jsonrpc::{Request, Response};
use crate::protocol::{
    ClientCapabilities, ClientInfo, InitializeParams, InitializeResult, McpToolDef,
    ToolsCallParams, ToolsCallResult, ToolsListResult,
};
use crate::proxy::McpToolProxy;

pub struct McpServerBridge {
    child: Child,
    writer: Arc<Mutex<tokio::process::ChildStdin>>,
    reader: Arc<Mutex<BufReader<tokio::process::ChildStdout>>>,
    next_id: Arc<std::sync::atomic::AtomicU64>,
    server_name: String,
}

impl McpServerBridge {
    pub async fn start(
        command: &str,
        args: &[&str],
        server_name: impl Into<String>,
    ) -> Result<Self, String> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn MCP server: {e}"))?;

        let stdin = child.stdin.take().ok_or("no stdin on child")?;
        let stdout = child.stdout.take().ok_or("no stdout on child")?;

        let mut bridge = McpServerBridge {
            child,
            writer: Arc::new(Mutex::new(stdin)),
            reader: Arc::new(Mutex::new(BufReader::new(stdout))),
            next_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            server_name: server_name.into(),
        };

        bridge.initialize().await?;
        Ok(bridge)
    }

    async fn initialize(&mut self) -> Result<InitializeResult, String> {
        let params = InitializeParams {
            protocol_version: "2024-11-05".into(),
            capabilities: ClientCapabilities {},
            client_info: ClientInfo {
                name: "aivyx".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
        };
        let result: InitializeResult = self
            .call("initialize", Some(serde_json::to_value(&params).unwrap()))
            .await?;

        self.send_notification("notifications/initialized", None)
            .await?;

        Ok(result)
    }

    pub async fn list_tools(&self) -> Result<Vec<McpToolDef>, String> {
        let result: ToolsListResult = self.call("tools/list", None).await?;
        Ok(result.tools)
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolsCallResult, String> {
        let params = ToolsCallParams {
            name: name.into(),
            arguments,
        };
        self.call(
            "tools/call",
            Some(serde_json::to_value(&params).unwrap()),
        )
        .await
    }

    pub async fn discover_tools(
        &self,
    ) -> Result<Vec<Arc<dyn Tool>>, String> {
        let defs = self.list_tools().await?;
        let tools: Vec<Arc<dyn Tool>> = defs
            .into_iter()
            .map(|def| {
                let proxy = McpToolProxy::new(
                    self.server_name.clone(),
                    def,
                    Arc::clone(&self.writer),
                    Arc::clone(&self.reader),
                    Arc::clone(&self.next_id),
                );
                Arc::new(proxy) as Arc<dyn Tool>
            })
            .collect();
        Ok(tools)
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    pub async fn shutdown(mut self) -> Result<(), String> {
        let _ = self.call::<serde_json::Value>("shutdown", None).await;
        let _ = self.send_notification("exit", None).await;
        let _ = self.child.kill().await;
        Ok(())
    }

    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<T, String> {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let req = Request::new(id, method, params);
        let mut line = serde_json::to_string(&req)
            .map_err(|e| format!("serialize request: {e}"))?;
        line.push('\n');

        {
            let mut w = self.writer.lock().await;
            w.write_all(line.as_bytes())
                .await
                .map_err(|e| format!("write to MCP server: {e}"))?;
            w.flush()
                .await
                .map_err(|e| format!("flush to MCP server: {e}"))?;
        }

        let mut resp_line = String::new();
        {
            let mut r = self.reader.lock().await;
            r.read_line(&mut resp_line)
                .await
                .map_err(|e| format!("read from MCP server: {e}"))?;
        }

        if resp_line.is_empty() {
            return Err("MCP server closed stdout".into());
        }

        let resp: Response = serde_json::from_str(&resp_line)
            .map_err(|e| format!("parse MCP response: {e}"))?;

        if resp.id != id {
            return Err(format!(
                "MCP response id mismatch: expected {id}, got {}",
                resp.id
            ));
        }

        if let Some(err) = resp.error {
            return Err(format!("MCP server error: {err}"));
        }

        let result = resp
            .result
            .ok_or_else(|| "MCP response has no result".to_string())?;
        serde_json::from_value(result)
            .map_err(|e| format!("deserialize MCP result: {e}"))
    }

    async fn send_notification(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Notification {
            jsonrpc: &'static str,
            method: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            params: Option<serde_json::Value>,
        }
        let notif = Notification {
            jsonrpc: "2.0",
            method: method.into(),
            params,
        };
        let mut line = serde_json::to_string(&notif)
            .map_err(|e| format!("serialize notification: {e}"))?;
        line.push('\n');

        let mut w = self.writer.lock().await;
        w.write_all(line.as_bytes())
            .await
            .map_err(|e| format!("write notification: {e}"))?;
        w.flush()
            .await
            .map_err(|e| format!("flush notification: {e}"))?;
        Ok(())
    }
}

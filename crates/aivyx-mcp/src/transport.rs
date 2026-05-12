//! `McpServerBridge` — protocol-level bridge to an MCP server.
//!
//! Handles MCP lifecycle (initialize, tools/list, tools/call, shutdown)
//! over an abstract `McpTransport`. The bridge owns the transport and
//! the JSON-RPC ID counter; tool proxies share both via `Arc`.

use std::sync::Arc;

use aivyx_core::Tool;

use crate::jsonrpc::{Request, Response};
use crate::protocol::{
    ClientCapabilities, ClientInfo, InitializeParams, InitializeResult, McpToolDef,
    ToolsCallParams, ToolsCallResult, ToolsListResult,
};
use crate::proxy::McpToolProxy;
use crate::transport_trait::McpTransport;

pub struct McpServerBridge {
    transport: Arc<dyn McpTransport>,
    next_id: Arc<std::sync::atomic::AtomicU64>,
    server_name: String,
}

impl McpServerBridge {
    /// Create a bridge from an already-connected transport. Runs the
    /// MCP `initialize` handshake before returning.
    pub async fn from_transport(
        transport: Arc<dyn McpTransport>,
        server_name: impl Into<String>,
    ) -> Result<Self, String> {
        let mut bridge = McpServerBridge {
            transport,
            next_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            server_name: server_name.into(),
        };
        bridge.initialize().await?;
        Ok(bridge)
    }

    /// Convenience: spawn a child process over stdio and initialize.
    /// This is the Phase 23 entry point preserved for backwards compat.
    ///
    /// Phase 55: `sandbox` is `None` here for backwards
    /// compatibility — callers that want sandboxing should use
    /// [`Self::start_with_sandbox`].
    pub async fn start(
        command: &str,
        args: &[&str],
        server_name: impl Into<String>,
    ) -> Result<Self, String> {
        Self::start_with_sandbox(command, args, None, server_name).await
    }

    /// Phase 55 — spawn an MCP server through an optional command
    /// wrapper. When `sandbox` is `Some`, the bridge spawns
    /// `wrapper wrapper_args... command command_args...` instead of
    /// the bare command. See `docs/TOOL_SDK.md` §9 for worked
    /// examples.
    pub async fn start_with_sandbox(
        command: &str,
        args: &[&str],
        sandbox: Option<&crate::stdio::SandboxConfig>,
        server_name: impl Into<String>,
    ) -> Result<Self, String> {
        let stdio = crate::stdio::StdioTransport::start(command, args, sandbox).await?;
        Self::from_transport(Arc::new(stdio), server_name).await
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
                    Arc::clone(&self.transport),
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

    /// Returns the transport handle. Used by the binary to access
    /// transport-specific shutdown (e.g. killing a stdio child).
    pub fn transport(&self) -> &Arc<dyn McpTransport> {
        &self.transport
    }

    pub async fn shutdown(self) -> Result<(), String> {
        let _ = self.call::<serde_json::Value>("shutdown", None).await;
        let _ = self.send_notification("exit", None).await;
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

        self.transport.send(&line).await?;

        let resp_line = self.transport.receive().await?;

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

        self.transport.send(&line).await
    }
}

//! `McpServerBridge` — protocol-level bridge to an MCP server.
//!
//! Handles MCP lifecycle (initialize, tools/list, tools/call, shutdown)
//! over a shared [`McpConn`]. The bridge and every proxy it produces
//! share one `McpConn` (transport + id counter + pending list-changed
//! set + round-trip lock) via `Arc`.

use std::sync::Arc;

use aivyx_core::Tool;

use crate::conn::McpConn;
use crate::notifications::{list_changed_flag, ListKind};
use crate::prompt_proxy::{McpPromptProxy, PromptTool};
use crate::protocol::{
    ClientCapabilities, ClientInfo, InitializeParams, InitializeResult, McpPromptDef,
    McpResourceDef, McpToolDef, PromptsGetParams, PromptsGetResult, PromptsListResult,
    ResourceContents, ResourcesListResult, ResourcesReadParams, ResourcesReadResult,
    ServerCapabilities, ToolsCallParams, ToolsCallResult, ToolsListResult,
};
use crate::proxy::McpToolProxy;
use crate::resource_proxy::{McpResourceProxy, ResourceTool};
use crate::transport_trait::McpTransport;

pub struct McpServerBridge {
    conn: Arc<McpConn>,
    server_name: String,
    /// Server capabilities from `initialize` — gates which features the
    /// client probes (tools / resources / prompts).
    capabilities: Option<ServerCapabilities>,
    /// The protocol version the server returned at `initialize`.
    protocol_version: String,
}

impl McpServerBridge {
    /// Create a bridge from an already-connected transport. Runs the
    /// MCP `initialize` handshake before returning, recording the
    /// server's declared capabilities + negotiated protocol version.
    pub async fn from_transport(
        transport: Arc<dyn McpTransport>,
        server_name: impl Into<String>,
    ) -> Result<Self, String> {
        let mut bridge = McpServerBridge {
            conn: McpConn::new(transport),
            server_name: server_name.into(),
            capabilities: None,
            protocol_version: String::new(),
        };
        let init = bridge.initialize().await?;
        bridge.protocol_version = init.protocol_version;
        bridge.capabilities = init.capabilities;
        Ok(bridge)
    }

    /// Whether the server declared the `tools` capability. Lenient: a
    /// server that sent **no** capabilities object at all is still
    /// probed for tools (pre-capability-declaration servers), but one
    /// that declared capabilities *without* `tools` is not.
    pub fn supports_tools(&self) -> bool {
        match &self.capabilities {
            None => true,
            Some(c) => c.tools.is_some(),
        }
    }

    /// Whether the server declared the `resources` capability. Strict:
    /// resources are only probed when explicitly declared.
    pub fn supports_resources(&self) -> bool {
        self.capabilities
            .as_ref()
            .and_then(|c| c.resources.as_ref())
            .is_some()
    }

    /// Whether the server declared the `prompts` capability. Strict.
    pub fn supports_prompts(&self) -> bool {
        self.capabilities
            .as_ref()
            .and_then(|c| c.prompts.as_ref())
            .is_some()
    }

    /// Whether the server advertised `tools.listChanged` — i.e. it will
    /// push `notifications/tools/list_changed` when its tool set
    /// changes. (Resources / prompts have parallel accessors.)
    pub fn tools_list_changed_declared(&self) -> bool {
        self.capabilities
            .as_ref()
            .map(|c| list_changed_flag(&c.tools))
            .unwrap_or(false)
    }

    pub fn resources_list_changed_declared(&self) -> bool {
        self.capabilities
            .as_ref()
            .map(|c| list_changed_flag(&c.resources))
            .unwrap_or(false)
    }

    pub fn prompts_list_changed_declared(&self) -> bool {
        self.capabilities
            .as_ref()
            .map(|c| list_changed_flag(&c.prompts))
            .unwrap_or(false)
    }

    /// Peek at the primitives whose lists have changed since the last
    /// drain (does not clear).
    pub fn pending_list_changed(&self) -> Vec<ListKind> {
        self.conn.pending()
    }

    /// Drain the pending-refresh set — the primitives a consumer should
    /// now re-discover. The daemon integration: after activity, if this
    /// is non-empty, call [`Self::rediscover`] and re-register the tools.
    pub fn take_pending_list_changed(&self) -> Vec<ListKind> {
        self.conn.take_pending()
    }

    /// Re-run discovery against the live server, returning the current
    /// bridged tool set. Idempotent; this is what a `*/list_changed`
    /// notification should trigger.
    pub async fn rediscover(&self) -> Result<Vec<Arc<dyn Tool>>, String> {
        self.discover_tools().await
    }

    /// The protocol version the server returned at `initialize`.
    pub fn negotiated_protocol_version(&self) -> &str {
        &self.protocol_version
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
        Self::start_with_sandbox(command, args, &[], None, None, server_name).await
    }

    /// Phase 55 — spawn an MCP server through an optional command
    /// wrapper. When `sandbox` is `Some`, the bridge spawns
    /// `wrapper wrapper_args... command command_args...` instead of
    /// the bare command. See `docs/TOOL_SDK.md` §9 for worked
    /// examples.
    pub async fn start_with_sandbox(
        command: &str,
        args: &[&str],
        env: &[(String, String)],
        sandbox: Option<&crate::stdio::SandboxConfig>,
        stderr_log: Option<&crate::stdio::StderrLog>,
        server_name: impl Into<String>,
    ) -> Result<Self, String> {
        let stdio =
            crate::stdio::StdioTransport::start(command, args, env, sandbox, stderr_log).await?;
        Self::from_transport(Arc::new(stdio), server_name).await
    }

    async fn initialize(&mut self) -> Result<InitializeResult, String> {
        let params = InitializeParams {
            protocol_version: "2024-11-05".into(),
            capabilities: ClientCapabilities {},
            client_info: ClientInfo {
                name: "aivyx-pa".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
        };
        let result: InitializeResult = self
            .conn
            .call("initialize", Some(serde_json::to_value(&params).unwrap()))
            .await?;

        self.conn
            .send_notification("notifications/initialized", None)
            .await?;

        Ok(result)
    }

    pub async fn list_tools(&self) -> Result<Vec<McpToolDef>, String> {
        let result: ToolsListResult = self.conn.call("tools/list", None).await?;
        Ok(result.tools)
    }

    /// `resources/list` — the resources the server exposes. Only valid
    /// when [`Self::supports_resources`] is true.
    pub async fn list_resources(&self) -> Result<Vec<McpResourceDef>, String> {
        let result: ResourcesListResult = self.conn.call("resources/list", None).await?;
        Ok(result.resources)
    }

    /// `resources/read` — the contents of one resource by URI.
    pub async fn read_resource(&self, uri: &str) -> Result<Vec<ResourceContents>, String> {
        let params = ResourcesReadParams { uri: uri.to_string() };
        let result: ResourcesReadResult = self
            .conn
            .call("resources/read", Some(serde_json::to_value(&params).unwrap()))
            .await?;
        Ok(result.contents)
    }

    /// `prompts/list` — the prompt templates the server exposes. Only
    /// valid when [`Self::supports_prompts`] is true.
    pub async fn list_prompts(&self) -> Result<Vec<McpPromptDef>, String> {
        let result: PromptsListResult = self.conn.call("prompts/list", None).await?;
        Ok(result.prompts)
    }

    /// `prompts/get` — retrieve one prompt rendered with `arguments`.
    pub async fn get_prompt(
        &self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<PromptsGetResult, String> {
        let params = PromptsGetParams {
            name: name.to_string(),
            arguments,
        };
        self.conn.call("prompts/get", Some(serde_json::to_value(&params).unwrap()))
            .await
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
        self.conn.call(
            "tools/call",
            Some(serde_json::to_value(&params).unwrap()),
        )
        .await
    }

    /// Discover every Aivyx `Tool` bridged from this server: one per
    /// declared tool (when the server supports `tools`), the two
    /// resource-access tools (`…resources.list` / `.read`) when it
    /// declares `resources`, and the two prompt tools
    /// (`…prompts.list` / `.get`) when it declares `prompts`. Gating on
    /// the declared capabilities means we never probe a method the
    /// server doesn't offer.
    pub async fn discover_tools(&self) -> Result<Vec<Arc<dyn Tool>>, String> {
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();

        if self.supports_tools() {
            for def in self.list_tools().await? {
                tools.push(Arc::new(McpToolProxy::new(
                    self.server_name.clone(),
                    def,
                    Arc::clone(&self.conn),
                )) as Arc<dyn Tool>);
            }
        }

        if self.supports_resources() {
            for kind in [ResourceTool::List, ResourceTool::Read] {
                tools.push(Arc::new(McpResourceProxy::new(
                    self.server_name.clone(),
                    kind,
                    Arc::clone(&self.conn),
                )) as Arc<dyn Tool>);
            }
        }

        if self.supports_prompts() {
            for kind in [PromptTool::List, PromptTool::Get] {
                tools.push(Arc::new(McpPromptProxy::new(
                    self.server_name.clone(),
                    kind,
                    Arc::clone(&self.conn),
                )) as Arc<dyn Tool>);
            }
        }

        Ok(tools)
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Returns the transport handle. Used by the binary to access
    /// transport-specific shutdown (e.g. killing a stdio child).
    pub fn transport(&self) -> &Arc<dyn McpTransport> {
        self.conn.transport()
    }

    pub async fn shutdown(&self) -> Result<(), String> {
        let _ = self.conn.call::<serde_json::Value>("shutdown", None).await;
        let _ = self.conn.send_notification("exit", None).await;
        Ok(())
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::MockTransport;

    fn init_reply(caps: &str) -> String {
        format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2024-11-05","serverInfo":{{"name":"t","version":"1"}},"capabilities":{caps}}}}}"#
        )
    }

    async fn bridge(replies: Vec<String>) -> McpServerBridge {
        McpServerBridge::from_transport(Arc::new(MockTransport::new(replies)), "wiki")
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn records_capabilities_and_protocol_version() {
        let b = bridge(vec![init_reply(r#"{"tools":{},"resources":{}}"#)]).await;
        assert_eq!(b.negotiated_protocol_version(), "2024-11-05");
        assert!(b.supports_tools());
        assert!(b.supports_resources());
    }

    #[tokio::test]
    async fn no_caps_object_is_lenient_for_tools_strict_for_resources() {
        let reply = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05"}}"#.to_string();
        let b = bridge(vec![reply]).await;
        assert!(b.supports_tools(), "legacy server with no caps still probed for tools");
        assert!(!b.supports_resources(), "resources never probed unless declared");
    }

    #[tokio::test]
    async fn declared_caps_without_tools_disables_tool_probing() {
        let b = bridge(vec![init_reply(r#"{"resources":{}}"#)]).await;
        assert!(!b.supports_tools());
        assert!(b.supports_resources());
    }

    #[tokio::test]
    async fn discover_adds_resource_tools_when_declared() {
        // resources-only server: two resource tools, no tools/list probe.
        let b = bridge(vec![init_reply(r#"{"resources":{}}"#)]).await;
        let tools = b.discover_tools().await.unwrap();
        let names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
        assert_eq!(tools.len(), 2);
        assert!(names.contains(&"mcp.wiki.resources.list".to_string()));
        assert!(names.contains(&"mcp.wiki.resources.read".to_string()));
    }

    #[tokio::test]
    async fn discover_skips_resources_when_not_declared() {
        let b = bridge(vec![
            init_reply(r#"{"tools":{}}"#),
            r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}}"#.to_string(),
        ])
        .await;
        let tools = b.discover_tools().await.unwrap();
        let names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
        assert_eq!(names, vec!["echo".to_string()]);
    }

    #[tokio::test]
    async fn list_resources_parses() {
        let b = bridge(vec![
            init_reply(r#"{"resources":{}}"#),
            r#"{"jsonrpc":"2.0","id":2,"result":{"resources":[{"uri":"file:///a.md","name":"A","mimeType":"text/markdown"}]}}"#.to_string(),
        ])
        .await;
        let res = b.list_resources().await.unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].uri, "file:///a.md");
    }

    #[tokio::test]
    async fn discover_adds_all_three_primitives_when_declared() {
        let b = bridge(vec![
            init_reply(r#"{"tools":{},"resources":{},"prompts":{}}"#),
            // tools/list (id 2) — the only probe at discovery time.
            r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}}"#.to_string(),
        ])
        .await;
        assert!(b.supports_prompts());
        let names: Vec<String> = b
            .discover_tools()
            .await
            .unwrap()
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        // echo + 2 resource + 2 prompt tools.
        assert_eq!(names.len(), 5);
        assert!(names.contains(&"mcp.wiki.prompts.list".to_string()));
        assert!(names.contains(&"mcp.wiki.prompts.get".to_string()));
        assert!(names.contains(&"mcp.wiki.resources.read".to_string()));
        assert!(names.contains(&"echo".to_string()));
    }

    #[tokio::test]
    async fn prompts_skipped_when_not_declared() {
        let b = bridge(vec![init_reply(r#"{"resources":{}}"#)]).await;
        assert!(!b.supports_prompts());
        let names: Vec<String> = b
            .discover_tools()
            .await
            .unwrap()
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        assert!(!names.iter().any(|n| n.contains("prompts")));
    }

    #[tokio::test]
    async fn list_prompts_parses() {
        let b = bridge(vec![
            init_reply(r#"{"prompts":{}}"#),
            r#"{"jsonrpc":"2.0","id":2,"result":{"prompts":[{"name":"review-pr","description":"Review a PR"}]}}"#.to_string(),
        ])
        .await;
        let p = b.list_prompts().await.unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "review-pr");
    }

    #[tokio::test]
    async fn list_changed_capability_is_detected() {
        let b = bridge(vec![init_reply(
            r#"{"tools":{"listChanged":true},"resources":{}}"#,
        )])
        .await;
        assert!(b.tools_list_changed_declared());
        assert!(!b.resources_list_changed_declared(), "declared but no flag");
        assert!(!b.prompts_list_changed_declared(), "not declared");
    }

    #[tokio::test]
    async fn call_demuxes_interleaved_notification_and_records_list_changed() {
        // A server pushes a tools/list_changed *before* the tools/list
        // response. The old code errored on id-mismatch; now it skips
        // the notification, returns the response, and records the refresh.
        let b = bridge(vec![
            init_reply(r#"{"tools":{"listChanged":true}}"#),
            r#"{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}"#.to_string(),
            r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","inputSchema":{}}]}}"#.to_string(),
        ])
        .await;
        let tools = b.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        // The interleaved notification was captured as a pending refresh.
        assert_eq!(b.pending_list_changed(), vec![ListKind::Tools]);
    }

    #[tokio::test]
    async fn take_pending_list_changed_drains() {
        let b = bridge(vec![
            init_reply(r#"{"tools":{}}"#),
            r#"{"jsonrpc":"2.0","method":"notifications/prompts/list_changed"}"#.to_string(),
            r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}"#.to_string(),
        ])
        .await;
        b.list_tools().await.unwrap();
        assert_eq!(b.take_pending_list_changed(), vec![ListKind::Prompts]);
        // Draining clears it.
        assert!(b.take_pending_list_changed().is_empty());
    }

    #[tokio::test]
    async fn call_skips_unrelated_server_notification() {
        let b = bridge(vec![
            init_reply(r#"{"tools":{}}"#),
            r#"{"jsonrpc":"2.0","method":"notifications/message","params":{"level":"info"}}"#.to_string(),
            r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}"#.to_string(),
        ])
        .await;
        // Does not error on the interleaved log notification.
        assert!(b.list_tools().await.unwrap().is_empty());
        assert!(b.pending_list_changed().is_empty());
    }
}

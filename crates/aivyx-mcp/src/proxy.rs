//! McpToolProxy — one Tool trait impl per discovered MCP server tool.

use std::sync::Arc;

use async_trait::async_trait;

use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

use crate::conn::McpConn;
use crate::protocol::{McpToolDef, ToolsCallParams, ToolsCallResult};

pub struct McpToolProxy {
    id: ToolId,
    server_name: String,
    def: McpToolDef,
    conn: Arc<McpConn>,
}

impl McpToolProxy {
    pub fn new(server_name: String, def: McpToolDef, conn: Arc<McpConn>) -> Self {
        McpToolProxy {
            id: ToolId::new(),
            server_name,
            def,
            conn,
        }
    }

    async fn call_remote(
        &self,
        arguments: serde_json::Value,
    ) -> Result<ToolsCallResult, String> {
        let params = ToolsCallParams {
            name: self.def.name.clone(),
            arguments,
        };
        self.conn
            .call("tools/call", Some(serde_json::to_value(&params).unwrap()))
            .await
    }
}

#[async_trait]
impl Tool for McpToolProxy {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        &self.def.name
    }

    // Chapter Bulwark — a third-party MCP server's output is untrusted content.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        self.def
            .description
            .as_deref()
            .unwrap_or("MCP tool (no description)")
    }

    fn input_schema(&self) -> &serde_json::Value {
        &self.def.input_schema
    }

    fn required_scope(&self, _input: &serde_json::Value) -> Scope {
        let qualified = format!("mcp.call:{}:{}", self.server_name, self.def.name);
        Scope::parse(&qualified).unwrap_or_else(|| {
            Scope::parse("mcp.call").expect("mcp.call must be a known base")
        })
    }

    // Every MCP-server-sourced tool is here because the operator
    // explicitly configured a `[[mcp_server]]` entry -- that
    // configuration act IS the opt-in.
    fn auto_grantable_in_backcompat_floor(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        _context: &ToolContext<'_>,
    ) -> ToolOutcome {
        match self.call_remote(input).await {
            Ok(result) => {
                if result.is_error {
                    let text = result
                        .content
                        .iter()
                        .filter_map(|b| b.text.as_deref())
                        .collect::<Vec<_>>()
                        .join("\n");
                    ToolOutcome::Failed(AivyxError::Tool {
                        tool: self.id,
                        detail: format!(
                            "MCP tool {} reported error: {text}",
                            self.def.name
                        ),
                    })
                } else {
                    let text = result
                        .content
                        .iter()
                        .filter_map(|b| b.text.as_deref())
                        .collect::<Vec<_>>()
                        .join("\n");
                    ToolOutcome::Completed {
                        output: serde_json::json!({ "text": text }),
                        verified: Verification::Unverified,
                    }
                }
            }
            Err(e) => ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("MCP call to {} failed: {e}", self.def.name),
            }),
        }
    }
}

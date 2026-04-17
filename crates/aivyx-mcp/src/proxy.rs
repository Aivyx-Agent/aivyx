//! McpToolProxy — one Tool trait impl per discovered MCP server tool.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::BufReader;
use tokio::sync::Mutex;

use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

use crate::jsonrpc::{Request, Response};
use crate::protocol::{McpToolDef, ToolsCallParams, ToolsCallResult};

pub struct McpToolProxy {
    id: ToolId,
    server_name: String,
    def: McpToolDef,
    writer: Arc<Mutex<tokio::process::ChildStdin>>,
    reader: Arc<Mutex<BufReader<tokio::process::ChildStdout>>>,
    next_id: Arc<std::sync::atomic::AtomicU64>,
}

impl McpToolProxy {
    pub fn new(
        server_name: String,
        def: McpToolDef,
        writer: Arc<Mutex<tokio::process::ChildStdin>>,
        reader: Arc<Mutex<BufReader<tokio::process::ChildStdout>>>,
        next_id: Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        McpToolProxy {
            id: ToolId::new(),
            server_name,
            def,
            writer,
            reader,
            next_id,
        }
    }

    async fn call_remote(
        &self,
        arguments: serde_json::Value,
    ) -> Result<ToolsCallResult, String> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let params = ToolsCallParams {
            name: self.def.name.clone(),
            arguments,
        };
        let req = Request::new(
            id,
            "tools/call",
            Some(serde_json::to_value(&params).unwrap()),
        );
        let mut line = serde_json::to_string(&req)
            .map_err(|e| format!("serialize: {e}"))?;
        line.push('\n');

        {
            let mut w = self.writer.lock().await;
            w.write_all(line.as_bytes())
                .await
                .map_err(|e| format!("write: {e}"))?;
            w.flush().await.map_err(|e| format!("flush: {e}"))?;
        }

        let mut resp_line = String::new();
        {
            let mut r = self.reader.lock().await;
            r.read_line(&mut resp_line)
                .await
                .map_err(|e| format!("read: {e}"))?;
        }

        if resp_line.is_empty() {
            return Err("MCP server closed stdout".into());
        }

        let resp: Response = serde_json::from_str(&resp_line)
            .map_err(|e| format!("parse response: {e}"))?;

        if let Some(err) = resp.error {
            return Err(format!("{err}"));
        }

        let result = resp
            .result
            .ok_or_else(|| "no result in response".to_string())?;
        serde_json::from_value(result)
            .map_err(|e| format!("deserialize result: {e}"))
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

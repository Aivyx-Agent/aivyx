//! `McpResourceProxy` — bridges an MCP server's **resources** capability
//! into two Aivyx tools so the agent can read context the server
//! exposes (not just call its tools):
//!
//! - `mcp.<server>.resources.list` → `resources/list`
//! - `mcp.<server>.resources.read` (input `{ uri }`) → `resources/read`
//!
//! Both reuse the existing `mcp.call` capability base (a resource read
//! is a call to the MCP server), so no new scope base is needed. The
//! bridge only registers these when the server declares the `resources`
//! capability at `initialize`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::conn::McpConn;
use crate::protocol::{McpResourceDef, ResourceContents, ResourcesListResult, ResourcesReadResult};

/// Shape a `resources/list` result into tool output. Pure.
fn list_output(resources: &[McpResourceDef]) -> Value {
    json!({
        "resources": serde_json::to_value(resources).unwrap_or(Value::Null),
        "count": resources.len(),
    })
}

/// Shape a `resources/read` result into tool output, flattening the
/// text contents for the LLM. Pure.
fn read_output(uri: &str, contents: &[ResourceContents]) -> Value {
    let text = contents
        .iter()
        .filter_map(|c| c.text.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    json!({
        "uri": uri,
        "text": text,
        "contents": serde_json::to_value(contents).unwrap_or(Value::Null),
    })
}

/// Which resource operation a proxy exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceTool {
    List,
    Read,
}

impl ResourceTool {
    fn suffix(self) -> &'static str {
        match self {
            ResourceTool::List => "list",
            ResourceTool::Read => "read",
        }
    }
}

pub struct McpResourceProxy {
    id: ToolId,
    server_name: String,
    kind: ResourceTool,
    name: String,
    schema: Value,
    conn: Arc<McpConn>,
}

impl McpResourceProxy {
    pub fn new(server_name: String, kind: ResourceTool, conn: Arc<McpConn>) -> Self {
        let name = format!("mcp.{}.resources.{}", server_name, kind.suffix());
        let schema = match kind {
            ResourceTool::List => json!({
                "type": "object", "properties": {}, "additionalProperties": false
            }),
            ResourceTool::Read => json!({
                "type": "object",
                "properties": { "uri": { "type": "string" } },
                "required": ["uri"],
                "additionalProperties": false
            }),
        };
        McpResourceProxy {
            id: ToolId::new(),
            server_name,
            kind,
            name,
            schema,
            conn,
        }
    }

    fn failed(&self, detail: String) -> ToolOutcome {
        ToolOutcome::Failed(AivyxError::Tool {
            tool: self.id,
            detail,
        })
    }
}

#[async_trait]
impl Tool for McpResourceProxy {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    // Chapter Bulwark — MCP resource content is untrusted external content.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        match self.kind {
            ResourceTool::List => {
                "List the resources this MCP server exposes (uri, name, description, \
                 mimeType). No input."
            }
            ResourceTool::Read => {
                "Read one resource's contents by URI. Input: { \"uri\": string }. \
                 Returns the resource's text (and the raw contents)."
            }
        }
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        let qualified = format!("mcp.call:{}:resources.{}", self.server_name, self.kind.suffix());
        Scope::parse(&qualified)
            .unwrap_or_else(|| Scope::parse("mcp.call").expect("mcp.call must be a known base"))
    }

    // Every MCP-server-sourced tool is here because the operator
    // explicitly configured a `[[mcp_server]]` entry -- that
    // configuration act IS the opt-in.
    fn auto_grantable_in_backcompat_floor(&self) -> bool {
        true
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        match self.kind {
            ResourceTool::List => {
                match self.conn.call::<ResourcesListResult>("resources/list", None).await {
                    Ok(r) => ToolOutcome::Completed {
                        output: list_output(&r.resources),
                        verified: Verification::Unverified,
                    },
                    Err(e) => self.failed(format!("resources/list on {} failed: {e}", self.server_name)),
                }
            }
            ResourceTool::Read => {
                let uri = match input.get("uri").and_then(Value::as_str) {
                    Some(u) if !u.trim().is_empty() => u.to_string(),
                    _ => return self.failed("`uri` (string) is required".into()),
                };
                match self
                    .conn
                    .call::<ResourcesReadResult>("resources/read", Some(json!({ "uri": uri })))
                    .await
                {
                    Ok(r) => ToolOutcome::Completed {
                        output: read_output(&uri, &r.contents),
                        verified: Verification::Unverified,
                    },
                    Err(e) => self.failed(format!("resources/read on {} failed: {e}", self.server_name)),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::MockTransport;

    fn proxy(kind: ResourceTool) -> McpResourceProxy {
        McpResourceProxy::new(
            "wiki".into(),
            kind,
            McpConn::new(Arc::new(MockTransport::new(vec![]))),
        )
    }

    #[test]
    fn names_schemas_and_scopes() {
        let list = proxy(ResourceTool::List);
        assert_eq!(list.name(), "mcp.wiki.resources.list");
        assert_eq!(list.required_scope(&Value::Null).base(), "mcp.call");

        let read = proxy(ResourceTool::Read);
        assert_eq!(read.name(), "mcp.wiki.resources.read");
        // read requires a uri in its schema.
        assert_eq!(read.input_schema()["required"][0], json!("uri"));
    }

    #[test]
    fn list_output_counts_and_serializes() {
        let resources = vec![McpResourceDef {
            uri: "file:///a.md".into(),
            name: Some("A".into()),
            description: None,
            mime_type: Some("text/markdown".into()),
        }];
        let out = list_output(&resources);
        assert_eq!(out["count"], json!(1));
        assert_eq!(out["resources"][0]["uri"], json!("file:///a.md"));
    }

    #[test]
    fn read_output_flattens_text() {
        let contents = vec![
            ResourceContents {
                uri: "file:///a.md".into(),
                mime_type: Some("text/markdown".into()),
                text: Some("line one".into()),
                blob: None,
            },
            ResourceContents {
                uri: "file:///a.md".into(),
                mime_type: None,
                text: Some("line two".into()),
                blob: None,
            },
        ];
        let out = read_output("file:///a.md", &contents);
        assert_eq!(out["uri"], json!("file:///a.md"));
        assert_eq!(out["text"], json!("line one\nline two"));
        assert_eq!(out["contents"].as_array().unwrap().len(), 2);
    }
}

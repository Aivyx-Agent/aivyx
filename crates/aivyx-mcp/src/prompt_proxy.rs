//! `McpPromptProxy` — bridges an MCP server's **prompts** capability
//! into two Aivyx tools so the agent can discover + retrieve the
//! reusable prompt templates a server publishes:
//!
//! - `mcp.<server>.prompts.list` → `prompts/list`
//! - `mcp.<server>.prompts.get` (input `{ name, arguments? }`) → `prompts/get`
//!
//! Like the resource proxy, both reuse the existing `mcp.call` scope
//! (retrieving a prompt is a call to the server), and the bridge only
//! registers them when the server declares the `prompts` capability.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::conn::McpConn;
use crate::protocol::{
    McpPromptDef, PromptMessage, PromptsGetParams, PromptsGetResult, PromptsListResult,
};

/// Shape a `prompts/list` result into tool output. Pure.
fn list_output(prompts: &[McpPromptDef]) -> Value {
    json!({
        "prompts": serde_json::to_value(prompts).unwrap_or(Value::Null),
        "count": prompts.len(),
    })
}

/// Shape a `prompts/get` result into tool output, flattening the
/// rendered messages' text for the LLM. Pure.
fn get_output(name: &str, description: Option<&str>, messages: &[PromptMessage]) -> Value {
    let text = messages
        .iter()
        .filter_map(|m| m.content.text.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    json!({
        "prompt": name,
        "description": description,
        "text": text,
        "messages": serde_json::to_value(messages).unwrap_or(Value::Null),
    })
}

/// Which prompt operation a proxy exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptTool {
    List,
    Get,
}

impl PromptTool {
    fn suffix(self) -> &'static str {
        match self {
            PromptTool::List => "list",
            PromptTool::Get => "get",
        }
    }
}

pub struct McpPromptProxy {
    id: ToolId,
    server_name: String,
    kind: PromptTool,
    name: String,
    schema: Value,
    conn: Arc<McpConn>,
}

impl McpPromptProxy {
    pub fn new(server_name: String, kind: PromptTool, conn: Arc<McpConn>) -> Self {
        let name = format!("mcp.{}.prompts.{}", server_name, kind.suffix());
        let schema = match kind {
            PromptTool::List => json!({
                "type": "object", "properties": {}, "additionalProperties": false
            }),
            PromptTool::Get => json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "arguments": { "type": "object" }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
        };
        McpPromptProxy {
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
impl Tool for McpPromptProxy {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    // Chapter Bulwark — MCP prompt content is untrusted external content.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        match self.kind {
            PromptTool::List => {
                "List the reusable prompt templates this MCP server exposes (name, \
                 description, arguments). No input."
            }
            PromptTool::Get => {
                "Retrieve one prompt template, rendered with arguments. Input: \
                 { \"name\": string, \"arguments\"?: object }. Returns the rendered \
                 messages' text."
            }
        }
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        let qualified = format!("mcp.call:{}:prompts.{}", self.server_name, self.kind.suffix());
        Scope::parse(&qualified)
            .unwrap_or_else(|| Scope::parse("mcp.call").expect("mcp.call must be a known base"))
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        match self.kind {
            PromptTool::List => {
                match self.conn.call::<PromptsListResult>("prompts/list", None).await {
                    Ok(r) => ToolOutcome::Completed {
                        output: list_output(&r.prompts),
                        verified: Verification::Unverified,
                    },
                    Err(e) => self.failed(format!("prompts/list on {} failed: {e}", self.server_name)),
                }
            }
            PromptTool::Get => {
                let name = match input.get("name").and_then(Value::as_str) {
                    Some(n) if !n.trim().is_empty() => n.to_string(),
                    _ => return self.failed("`name` (string) is required".into()),
                };
                let params = PromptsGetParams {
                    name: name.clone(),
                    arguments: input.get("arguments").cloned(),
                };
                match self
                    .conn
                    .call::<PromptsGetResult>(
                        "prompts/get",
                        Some(serde_json::to_value(&params).unwrap()),
                    )
                    .await
                {
                    Ok(r) => ToolOutcome::Completed {
                        output: get_output(&name, r.description.as_deref(), &r.messages),
                        verified: Verification::Unverified,
                    },
                    Err(e) => self.failed(format!("prompts/get on {} failed: {e}", self.server_name)),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PromptContent;
    use crate::testutil::MockTransport;

    fn proxy(kind: PromptTool) -> McpPromptProxy {
        McpPromptProxy::new(
            "review".into(),
            kind,
            McpConn::new(Arc::new(MockTransport::new(vec![]))),
        )
    }

    #[test]
    fn names_schemas_and_scopes() {
        let list = proxy(PromptTool::List);
        assert_eq!(list.name(), "mcp.review.prompts.list");
        assert_eq!(list.required_scope(&Value::Null).base(), "mcp.call");

        let get = proxy(PromptTool::Get);
        assert_eq!(get.name(), "mcp.review.prompts.get");
        assert_eq!(get.input_schema()["required"][0], json!("name"));
    }

    #[test]
    fn list_output_counts_and_serializes() {
        let prompts = vec![McpPromptDef {
            name: "review-pr".into(),
            description: Some("Review a pull request".into()),
            arguments: vec![],
        }];
        let out = list_output(&prompts);
        assert_eq!(out["count"], json!(1));
        assert_eq!(out["prompts"][0]["name"], json!("review-pr"));
    }

    #[test]
    fn get_output_flattens_message_text() {
        let messages = vec![
            PromptMessage {
                role: "user".into(),
                content: PromptContent { content_type: "text".into(), text: Some("step one".into()) },
            },
            PromptMessage {
                role: "user".into(),
                content: PromptContent { content_type: "text".into(), text: Some("step two".into()) },
            },
        ];
        let out = get_output("review-pr", Some("Review a PR"), &messages);
        assert_eq!(out["prompt"], json!("review-pr"));
        assert_eq!(out["text"], json!("step one\nstep two"));
        assert_eq!(out["messages"].as_array().unwrap().len(), 2);
    }
}

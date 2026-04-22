//! `aivyx mcp-server <name>` — bundled MCP server runner (Phase 46).
//!
//! Runs an MCP-compliant stdio server inside the `aivyx` binary.
//! Reads newline-delimited JSON-RPC 2.0 from stdin, writes responses
//! to stdout. Currently supports one server name: `"web-search"`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

// ---------------------------------------------------------------------------
// JSON-RPC types (server-side: Deserialize requests, Serialize responses)
// ---------------------------------------------------------------------------

/// Incoming JSON-RPC request or notification. Notifications have `id: None`.
#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<u64>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

impl JsonRpcResponse {
    fn ok(id: u64, result: Value) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: u64, code: i64, message: impl Into<String>) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

struct ToolDef {
    name: &'static str,
    description: &'static str,
    input_schema: Value,
    handler: fn(Value) -> Result<String, String>,
}

fn echo_handler(args: Value) -> Result<String, String> {
    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    Ok(message.to_string())
}

fn web_search_tools() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "echo",
        description: "Echo the input message (smoke test tool).",
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Text to echo back."
                }
            },
            "required": ["message"]
        }),
        handler: echo_handler,
    }]
}

// ---------------------------------------------------------------------------
// Stdio harness
// ---------------------------------------------------------------------------

/// Entry point called from `run()` in `aivyx.rs`.
pub async fn run_mcp_server(name: &str) -> Result<(), String> {
    let tools = match name {
        "web-search" => web_search_tools(),
        other => {
            return Err(format!(
                "unknown MCP server name: `{other}`. Supported: web-search"
            ))
        }
    };

    eprintln!("aivyx mcp-server: starting {name} server on stdio");
    run_stdio_loop(&tools).await
}

async fn run_stdio_loop(tools: &[ToolDef]) -> Result<(), String> {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();

    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("stdin read: {e}"))?;

        if n == 0 {
            // EOF — client closed stdin.
            break;
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                // Per JSON-RPC spec, parse errors use id: null. We use id: 0
                // since our response type requires u64.
                let resp = JsonRpcResponse::err(0, -32700, format!("parse error: {e}"));
                write_response(&mut stdout, &resp).await?;
                continue;
            }
        };

        // Notifications (no id) — handle and continue without response.
        if req.id.is_none() {
            match req.method.as_str() {
                "notifications/initialized" => {} // Expected after initialize.
                "exit" => break,
                _ => {} // Ignore unknown notifications.
            }
            continue;
        }

        let id = req.id.unwrap();
        let resp = dispatch(id, &req.method, req.params, tools);
        write_response(&mut stdout, &resp).await?;

        // After responding to "shutdown", wait for "exit" notification.
        if req.method == "shutdown" {
            break;
        }
    }

    Ok(())
}

fn dispatch(id: u64, method: &str, params: Option<Value>, tools: &[ToolDef]) -> JsonRpcResponse {
    match method {
        "initialize" => {
            let result = serde_json::json!({
                "protocolVersion": "2024-11-05",
                "serverInfo": {
                    "name": "aivyx-web-search",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "tools": {}
                }
            });
            JsonRpcResponse::ok(id, result)
        }

        "tools/list" => {
            let tool_defs: Vec<Value> = tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": t.input_schema
                    })
                })
                .collect();
            JsonRpcResponse::ok(id, serde_json::json!({ "tools": tool_defs }))
        }

        "tools/call" => {
            let params = match params {
                Some(p) => p,
                None => return JsonRpcResponse::err(id, -32602, "missing params"),
            };
            let tool_name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Object(Default::default()));

            let tool = tools.iter().find(|t| t.name == tool_name);
            match tool {
                Some(t) => match (t.handler)(arguments) {
                    Ok(text) => JsonRpcResponse::ok(
                        id,
                        serde_json::json!({
                            "content": [{"type": "text", "text": text}],
                            "isError": false
                        }),
                    ),
                    Err(e) => JsonRpcResponse::ok(
                        id,
                        serde_json::json!({
                            "content": [{"type": "text", "text": e}],
                            "isError": true
                        }),
                    ),
                },
                None => JsonRpcResponse::err(
                    id,
                    -32602,
                    format!("unknown tool: {tool_name}"),
                ),
            }
        }

        "shutdown" => JsonRpcResponse::ok(id, Value::Null),

        other => JsonRpcResponse::err(id, -32601, format!("method not found: {other}")),
    }
}

async fn write_response(
    stdout: &mut tokio::io::Stdout,
    resp: &JsonRpcResponse,
) -> Result<(), String> {
    let mut line =
        serde_json::to_string(resp).map_err(|e| format!("serialize response: {e}"))?;
    line.push('\n');
    stdout
        .write_all(line.as_bytes())
        .await
        .map_err(|e| format!("stdout write: {e}"))?;
    stdout
        .flush()
        .await
        .map_err(|e| format!("stdout flush: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_rpc_parse_roundtrip() {
        let json = r#"{"jsonrpc":"2.0","id":42,"method":"tools/list"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.id, Some(42));
        assert_eq!(req.method, "tools/list");

        let resp = JsonRpcResponse::ok(42, serde_json::json!({"tools": []}));
        let out = serde_json::to_string(&resp).unwrap();
        assert!(out.contains("\"id\":42"));
        assert!(out.contains("\"tools\":[]"));
        // Error field should be absent (skip_serializing_if).
        assert!(!out.contains("error"));
    }

    #[test]
    fn dispatch_initialize_returns_server_info() {
        let tools = web_search_tools();
        let resp = dispatch(1, "initialize", None, &tools);
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "aivyx-web-search");
    }

    #[test]
    fn dispatch_tools_list_returns_echo() {
        let tools = web_search_tools();
        let resp = dispatch(2, "tools/list", None, &tools);
        let result = resp.result.unwrap();
        let tool_list = result["tools"].as_array().unwrap();
        assert_eq!(tool_list.len(), 1);
        assert_eq!(tool_list[0]["name"], "echo");
    }

    #[test]
    fn dispatch_tools_call_echo() {
        let tools = web_search_tools();
        let params = serde_json::json!({
            "name": "echo",
            "arguments": {"message": "hello world"}
        });
        let resp = dispatch(3, "tools/call", Some(params), &tools);
        let result = resp.result.unwrap();
        assert_eq!(result["isError"], false);
        let content = result["content"].as_array().unwrap();
        assert_eq!(content[0]["text"], "hello world");
    }

    #[test]
    fn dispatch_shutdown_returns_null() {
        let tools = web_search_tools();
        let resp = dispatch(4, "shutdown", None, &tools);
        assert!(resp.result.unwrap().is_null());
        assert!(resp.error.is_none());
    }

    #[test]
    fn dispatch_unknown_method_returns_error() {
        let tools = web_search_tools();
        let resp = dispatch(5, "bogus/method", None, &tools);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    #[test]
    fn dispatch_unknown_tool_returns_error() {
        let tools = web_search_tools();
        let params = serde_json::json!({
            "name": "nonexistent",
            "arguments": {}
        });
        let resp = dispatch(6, "tools/call", Some(params), &tools);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }
}

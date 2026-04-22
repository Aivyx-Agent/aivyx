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
// Tool schema definitions
// ---------------------------------------------------------------------------

struct ToolSchema {
    name: &'static str,
    description: &'static str,
    input_schema: Value,
}

fn web_search_tool_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "web_search",
            description: "Search the web and return a list of results with titles, URLs, and snippets.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query."
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of results to return (default 5, max 20)."
                    }
                },
                "required": ["query"]
            }),
        },
        ToolSchema {
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
        },
    ]
}

// ---------------------------------------------------------------------------
// DuckDuckGo HTML search
// ---------------------------------------------------------------------------

const DDG_HTML_URL: &str = "https://html.duckduckgo.com/html/";
const DEFAULT_MAX_RESULTS: usize = 5;
const MAX_RESULTS_CAP: usize = 20;

#[derive(Debug, Clone, Serialize)]
struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

/// Parse DuckDuckGo HTML search results page.
///
/// The HTML structure uses `<a class="result__a">` for titles/links and
/// `<a class="result__snippet">` for snippets. We extract these with
/// simple string scanning — no HTML parser dependency needed.
fn parse_ddg_html(html: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();

    // Split on result link anchors. Each `class="result__a"` marks a result.
    for chunk in html.split("class=\"result__a\"").skip(1) {
        // Extract href from the anchor: href="..."
        let url = extract_attr(chunk, "href=\"");
        if url.is_empty() {
            continue;
        }

        // Extract title: text between > and </a>
        let title = extract_inner_text(chunk);

        // Look for snippet in this region
        let snippet = if let Some(snip_pos) = chunk.find("class=\"result__snippet\"") {
            let snip_chunk = &chunk[snip_pos..];
            strip_html_tags(&extract_inner_text(snip_chunk))
        } else {
            String::new()
        };

        results.push(SearchResult {
            title: strip_html_tags(&title),
            url: clean_ddg_url(&url),
            snippet,
        });
    }

    results
}

/// Extract attribute value following the given prefix (e.g. `href="`).
fn extract_attr(s: &str, prefix: &str) -> String {
    if let Some(start) = s.find(prefix) {
        let rest = &s[start + prefix.len()..];
        if let Some(end) = rest.find('"') {
            return html_decode(&rest[..end]);
        }
    }
    String::new()
}

/// Extract text between first `>` and `</a>`.
fn extract_inner_text(s: &str) -> String {
    if let Some(start) = s.find('>') {
        let rest = &s[start + 1..];
        if let Some(end) = rest.find("</a>") {
            return rest[..end].to_string();
        }
        // Fallback: take until next `<`
        if let Some(end) = rest.find('<') {
            return rest[..end].to_string();
        }
    }
    String::new()
}

/// Strip HTML tags from a string.
fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

/// DDG wraps outbound URLs in a redirect. Extract the real URL if present.
fn clean_ddg_url(url: &str) -> String {
    // DDG links look like: //duckduckgo.com/l/?uddg=https%3A%2F%2F...&rut=...
    if let Some(pos) = url.find("uddg=") {
        let rest = &url[pos + 5..];
        let end = rest.find('&').unwrap_or(rest.len());
        return html_decode(&url_decode(&rest[..end]));
    }
    url.to_string()
}

/// Minimal percent-decoding for URL values.
fn url_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let hi = chars.next().unwrap_or('0');
            let lo = chars.next().unwrap_or('0');
            let hex = format!("{hi}{lo}");
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                out.push(byte as char);
            } else {
                out.push('%');
                out.push(hi);
                out.push(lo);
            }
        } else if ch == '+' {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

/// Decode basic HTML entities.
fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
}

/// Minimal percent-encoding for query parameters.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(byte & 0xf) as usize]));
            }
        }
    }
    out
}

async fn handle_web_search(args: Value) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: query".to_string())?;

    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).min(MAX_RESULTS_CAP))
        .unwrap_or(DEFAULT_MAX_RESULTS);

    let url = format!("{}?q={}", DDG_HTML_URL, url_encode(query));

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; aivyx/1.0)")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("build HTTP client: {e}"))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("DuckDuckGo request failed: {e}"))?;

    let html = resp
        .text()
        .await
        .map_err(|e| format!("read DuckDuckGo response: {e}"))?;

    let results: Vec<SearchResult> = parse_ddg_html(&html)
        .into_iter()
        .take(max_results)
        .collect();

    serde_json::to_string_pretty(&results)
        .map_err(|e| format!("serialize results: {e}"))
}

fn handle_echo(args: Value) -> Result<String, String> {
    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    Ok(message.to_string())
}

// ---------------------------------------------------------------------------
// Stdio harness
// ---------------------------------------------------------------------------

/// Entry point called from `run()` in `aivyx.rs`.
pub async fn run_mcp_server(name: &str) -> Result<(), String> {
    let schemas = match name {
        "web-search" => web_search_tool_schemas(),
        other => {
            return Err(format!(
                "unknown MCP server name: `{other}`. Supported: web-search"
            ))
        }
    };

    eprintln!("aivyx mcp-server: starting {name} server on stdio");
    run_stdio_loop(&schemas).await
}

async fn run_stdio_loop(schemas: &[ToolSchema]) -> Result<(), String> {
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
            break;
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse::err(0, -32700, format!("parse error: {e}"));
                write_response(&mut stdout, &resp).await?;
                continue;
            }
        };

        if req.id.is_none() {
            match req.method.as_str() {
                "notifications/initialized" => {}
                "exit" => break,
                _ => {}
            }
            continue;
        }

        let id = req.id.unwrap();
        let resp = dispatch(id, &req.method, req.params, schemas).await;
        write_response(&mut stdout, &resp).await?;

        if req.method == "shutdown" {
            break;
        }
    }

    Ok(())
}

async fn dispatch(
    id: u64,
    method: &str,
    params: Option<Value>,
    schemas: &[ToolSchema],
) -> JsonRpcResponse {
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
            let tool_defs: Vec<Value> = schemas
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

        "tools/call" => dispatch_tool_call(id, params).await,

        "shutdown" => JsonRpcResponse::ok(id, Value::Null),

        other => JsonRpcResponse::err(id, -32601, format!("method not found: {other}")),
    }
}

async fn dispatch_tool_call(id: u64, params: Option<Value>) -> JsonRpcResponse {
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

    let result = match tool_name {
        "web_search" => handle_web_search(arguments).await,
        "echo" => handle_echo(arguments),
        _ => return JsonRpcResponse::err(id, -32602, format!("unknown tool: {tool_name}")),
    };

    match result {
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
        assert!(!out.contains("error"));
    }

    #[tokio::test]
    async fn dispatch_initialize_returns_server_info() {
        let schemas = web_search_tool_schemas();
        let resp = dispatch(1, "initialize", None, &schemas).await;
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "aivyx-web-search");
    }

    #[tokio::test]
    async fn dispatch_tools_list_returns_web_search() {
        let schemas = web_search_tool_schemas();
        let resp = dispatch(2, "tools/list", None, &schemas).await;
        let result = resp.result.unwrap();
        let tool_list = result["tools"].as_array().unwrap();
        assert_eq!(tool_list.len(), 2);
        assert_eq!(tool_list[0]["name"], "web_search");
        assert_eq!(tool_list[1]["name"], "echo");
    }

    #[tokio::test]
    async fn dispatch_tools_call_echo() {
        let schemas = web_search_tool_schemas();
        let params = serde_json::json!({
            "name": "echo",
            "arguments": {"message": "hello world"}
        });
        let resp = dispatch(3, "tools/call", Some(params), &schemas).await;
        let result = resp.result.unwrap();
        assert_eq!(result["isError"], false);
        let content = result["content"].as_array().unwrap();
        assert_eq!(content[0]["text"], "hello world");
    }

    #[tokio::test]
    async fn dispatch_shutdown_returns_null() {
        let schemas = web_search_tool_schemas();
        let resp = dispatch(4, "shutdown", None, &schemas).await;
        assert!(resp.result.unwrap().is_null());
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn dispatch_unknown_method_returns_error() {
        let schemas = web_search_tool_schemas();
        let resp = dispatch(5, "bogus/method", None, &schemas).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    #[tokio::test]
    async fn dispatch_unknown_tool_returns_error() {
        let schemas = web_search_tool_schemas();
        let params = serde_json::json!({
            "name": "nonexistent",
            "arguments": {}
        });
        let resp = dispatch(6, "tools/call", Some(params), &schemas).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    // -----------------------------------------------------------------------
    // Phase 46 Task 3 — DuckDuckGo parser tests
    // -----------------------------------------------------------------------

    #[test]
    fn ddg_parser_extracts_results() {
        let html = r#"
        <div class="result">
            <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage1&amp;rut=abc">
                Example Page One
            </a>
            <a class="result__snippet">This is the first snippet.</a>
        </div>
        <div class="result">
            <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage2&amp;rut=def">
                Example <b>Page</b> Two
            </a>
            <a class="result__snippet">Second snippet with <b>bold</b> text.</a>
        </div>
        "#;
        let results = parse_ddg_html(html);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Example Page One");
        assert_eq!(results[0].url, "https://example.com/page1");
        assert_eq!(results[0].snippet, "This is the first snippet.");
        assert_eq!(results[1].title, "Example Page Two");
        assert_eq!(results[1].url, "https://example.com/page2");
        assert_eq!(results[1].snippet, "Second snippet with bold text.");
    }

    #[test]
    fn ddg_parser_empty_page() {
        let html = "<html><body>No results found</body></html>";
        let results = parse_ddg_html(html);
        assert!(results.is_empty());
    }

    #[test]
    fn ddg_parser_malformed_html() {
        // Partial result with missing href — should be skipped.
        let html = r#"
        <a class="result__a">No href here</a>
        <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fgood.com&amp;rut=x">
            Good result
        </a>
        <a class="result__snippet">A snippet.</a>
        "#;
        let results = parse_ddg_html(html);
        // First result has empty href -> skipped, second is valid.
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://good.com");
    }

    #[test]
    fn web_search_schema_valid() {
        let schemas = web_search_tool_schemas();
        let ws = schemas.iter().find(|s| s.name == "web_search").unwrap();
        let props = ws.input_schema["properties"].as_object().unwrap();
        assert!(props.contains_key("query"));
        assert!(props.contains_key("max_results"));
        let required = ws.input_schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "query");
    }

    #[test]
    fn web_search_max_results_capped() {
        // Verify capping logic inline — the handler caps at MAX_RESULTS_CAP.
        let val = serde_json::json!({"query": "test", "max_results": 100});
        let max = val
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(MAX_RESULTS_CAP))
            .unwrap_or(DEFAULT_MAX_RESULTS);
        assert_eq!(max, 20);

        // Default when absent.
        let val2 = serde_json::json!({"query": "test"});
        let max2 = val2
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(MAX_RESULTS_CAP))
            .unwrap_or(DEFAULT_MAX_RESULTS);
        assert_eq!(max2, 5);
    }

    #[test]
    fn url_encode_basic() {
        assert_eq!(url_encode("hello world"), "hello+world");
        assert_eq!(url_encode("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn url_decode_basic() {
        assert_eq!(url_decode("https%3A%2F%2Fexample.com"), "https://example.com");
        assert_eq!(url_decode("hello+world"), "hello world");
    }

    #[test]
    fn html_decode_entities() {
        assert_eq!(html_decode("a &amp; b"), "a & b");
        assert_eq!(html_decode("&lt;tag&gt;"), "<tag>");
    }

    #[test]
    fn strip_html_tags_works() {
        assert_eq!(strip_html_tags("hello <b>world</b>!"), "hello world!");
        assert_eq!(strip_html_tags("<a href=\"x\">link</a>"), "link");
    }

    #[test]
    fn clean_ddg_url_extracts_target() {
        let ddg = "//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org&rut=abc";
        assert_eq!(clean_ddg_url(ddg), "https://rust-lang.org");
    }

    #[test]
    fn clean_ddg_url_passthrough() {
        assert_eq!(clean_ddg_url("https://direct.com"), "https://direct.com");
    }
}

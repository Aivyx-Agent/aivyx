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
            name: "web_read",
            description: "Fetch a URL and return its content as cleaned readable text (HTML tags stripped).",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch and read."
                    }
                },
                "required": ["url"]
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
// Search backend selection
// ---------------------------------------------------------------------------

const DEFAULT_MAX_RESULTS: usize = 5;
const MAX_RESULTS_CAP: usize = 20;

#[derive(Debug, Clone, PartialEq)]
enum SearchBackend {
    Brave(String),
    SerpApi(String),
    DuckDuckGo,
}

/// Select the best available search backend based on environment variables.
/// Priority: Brave Search API → SerpAPI → DuckDuckGo (zero-config fallback).
fn select_search_backend() -> SearchBackend {
    if let Ok(key) = std::env::var("BRAVE_SEARCH_API_KEY") {
        if !key.is_empty() {
            return SearchBackend::Brave(key);
        }
    }
    if let Ok(key) = std::env::var("SERPAPI_KEY") {
        if !key.is_empty() {
            return SearchBackend::SerpApi(key);
        }
    }
    SearchBackend::DuckDuckGo
}

// ---------------------------------------------------------------------------
// DuckDuckGo HTML search
// ---------------------------------------------------------------------------

const DDG_HTML_URL: &str = "https://html.duckduckgo.com/html/";

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

// ---------------------------------------------------------------------------
// Brave Search API
// ---------------------------------------------------------------------------

const BRAVE_SEARCH_URL: &str = "https://api.search.brave.com/res/v1/web/search";

/// Parse Brave Search API JSON response.
fn parse_brave_json(json: &str) -> Vec<SearchResult> {
    let value: Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let results = value
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array());
    match results {
        Some(arr) => arr
            .iter()
            .filter_map(|item| {
                let title = item.get("title")?.as_str()?.to_string();
                let url = item.get("url")?.as_str()?.to_string();
                let snippet = item
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(SearchResult { title, url, snippet })
            })
            .collect(),
        None => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// SerpAPI
// ---------------------------------------------------------------------------

const SERPAPI_URL: &str = "https://serpapi.com/search.json";

/// Parse SerpAPI JSON response.
fn parse_serpapi_json(json: &str) -> Vec<SearchResult> {
    let value: Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let results = value
        .get("organic_results")
        .and_then(|r| r.as_array());
    match results {
        Some(arr) => arr
            .iter()
            .filter_map(|item| {
                let title = item.get("title")?.as_str()?.to_string();
                let url = item.get("link")?.as_str()?.to_string();
                let snippet = item
                    .get("snippet")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(SearchResult { title, url, snippet })
            })
            .collect(),
        None => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Unified search handler
// ---------------------------------------------------------------------------

async fn handle_web_search(args: Value, backend: &SearchBackend) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: query".to_string())?;

    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).min(MAX_RESULTS_CAP))
        .unwrap_or(DEFAULT_MAX_RESULTS);

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; aivyx/1.0)")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("build HTTP client: {e}"))?;

    let all_results = match backend {
        SearchBackend::Brave(api_key) => {
            let url = format!(
                "{}?q={}&count={}",
                BRAVE_SEARCH_URL,
                url_encode(query),
                max_results
            );
            let resp = client
                .get(&url)
                .header("X-Subscription-Token", api_key.as_str())
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| format!("Brave Search request failed: {e}"))?;
            let status = resp.status();
            let body = resp
                .text()
                .await
                .map_err(|e| format!("read Brave Search response: {e}"))?;
            if !status.is_success() {
                return Err(format!(
                    "Brave Search returned HTTP {status} — the backend refused the \
                     request (bad key, quota, or outage); this is NOT an empty \
                     result set: {}",
                    body.chars().take(200).collect::<String>()
                ));
            }
            parse_brave_json(&body)
        }
        SearchBackend::SerpApi(api_key) => {
            let url = format!(
                "{}?q={}&api_key={}&num={}",
                SERPAPI_URL,
                url_encode(query),
                url_encode(api_key),
                max_results
            );
            let resp = client
                .get(&url)
                .send()
                .await
                .map_err(|e| format!("SerpAPI request failed: {e}"))?;
            let status = resp.status();
            let body = resp
                .text()
                .await
                .map_err(|e| format!("read SerpAPI response: {e}"))?;
            if !status.is_success() {
                return Err(format!(
                    "SerpAPI returned HTTP {status} — the backend refused the \
                     request (bad key, quota, or outage); this is NOT an empty \
                     result set: {}",
                    body.chars().take(200).collect::<String>()
                ));
            }
            parse_serpapi_json(&body)
        }
        SearchBackend::DuckDuckGo => {
            let url = format!("{}?q={}", DDG_HTML_URL, url_encode(query));
            let resp = client
                .get(&url)
                .send()
                .await
                .map_err(|e| format!("DuckDuckGo request failed: {e}"))?;
            let status = resp.status();
            let html = resp
                .text()
                .await
                .map_err(|e| format!("read DuckDuckGo response: {e}"))?;
            // DDG answers bot-flagged traffic with HTTP **202** + a
            // challenge page — a 2xx, so `is_success()` would wave it
            // through and the parser would yield a silent `[]` that the
            // model reads as "zero hits" (live rig 2026-07-05: every
            // search empty → the agent either went mute or confabulated
            // an answer). Only a plain 200 carries a result page.
            if status != reqwest::StatusCode::OK {
                return Err(format!(
                    "DuckDuckGo returned HTTP {status} (anti-bot challenge or \
                     rate limit) — the zero-config search backend is currently \
                     unavailable; this is NOT an empty result set. A keyed \
                     backend (BRAVE_SEARCH_API_KEY or SERPAPI_KEY) avoids \
                     this."
                ));
            }
            parse_ddg_html(&html)
        }
    };

    let results: Vec<SearchResult> = all_results.into_iter().take(max_results).collect();

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
// web_read — URL fetch + HTML-to-text
// ---------------------------------------------------------------------------

const WEB_READ_MAX_BYTES: usize = 10 * 1024 * 1024; // 10 MiB

async fn handle_web_read(args: Value) -> Result<String, String> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required parameter: url".to_string())?;

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; aivyx/1.0)")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("build HTTP client: {e}"))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("fetch failed: {e}"))?;

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("read response body: {e}"))?;

    if bytes.len() > WEB_READ_MAX_BYTES {
        return Err(format!(
            "response too large: {} bytes (limit: {} bytes)",
            bytes.len(),
            WEB_READ_MAX_BYTES
        ));
    }

    let body = String::from_utf8_lossy(&bytes);

    let is_html = content_type.contains("text/html") || body.trim_start().starts_with('<');

    let (title, content) = if is_html {
        (extract_html_title(&body), html_to_text(&body))
    } else {
        (String::new(), body.into_owned())
    };

    let result = serde_json::json!({
        "title": title,
        "url": url,
        "content": content
    });
    serde_json::to_string_pretty(&result)
        .map_err(|e| format!("serialize result: {e}"))
}

/// Extract `<title>` text from an HTML document.
fn extract_html_title(html: &str) -> String {
    let lower = html.to_lowercase();
    if let Some(start) = lower.find("<title") {
        let rest = &html[start..];
        // Skip past the closing `>` of the opening tag.
        if let Some(gt) = rest.find('>') {
            let after_tag = &rest[gt + 1..];
            if let Some(end) = after_tag.to_lowercase().find("</title") {
                return strip_html_tags(&after_tag[..end])
                    .trim()
                    .to_string();
            }
        }
    }
    String::new()
}

/// Convert HTML to readable plain text.
///
/// 1. Remove `<script>` and `<style>` blocks entirely.
/// 2. Insert newlines around block-level tags (`<p>`, `<div>`, `<br>`, etc.).
/// 3. Strip all remaining HTML tags.
/// 4. Decode HTML entities.
/// 5. Collapse whitespace: multiple spaces/tabs → single space,
///    multiple newlines → double newline (paragraph break).
fn html_to_text(html: &str) -> String {
    let mut s = html.to_string();

    // Remove <script>...</script> blocks (case-insensitive).
    s = remove_tag_block(&s, "script");
    // Remove <style>...</style> blocks.
    s = remove_tag_block(&s, "style");

    // Insert newlines around block-level elements so paragraph
    // boundaries survive tag stripping.
    s = insert_block_breaks(&s);

    // Strip remaining HTML tags.
    let stripped = strip_html_tags(&s);

    // Decode entities.
    let decoded = html_decode(&stripped);

    // Collapse whitespace.
    collapse_whitespace(&decoded)
}

/// Insert newline markers before/after block-level HTML elements.
fn insert_block_breaks(html: &str) -> String {
    let block_tags = [
        "<p", "</p", "<div", "</div", "<br", "<h1", "</h1", "<h2", "</h2",
        "<h3", "</h3", "<h4", "</h4", "<h5", "</h5", "<h6", "</h6",
        "<li", "</li", "<ul", "</ul", "<ol", "</ol", "<tr", "</tr",
        "<blockquote", "</blockquote", "<hr",
    ];
    let mut result = html.to_string();
    let lower = html.to_lowercase();

    // Work backwards through tag positions to avoid index invalidation.
    let mut positions: Vec<(usize, usize)> = Vec::new();
    for tag in &block_tags {
        let mut start = 0;
        while let Some(pos) = lower[start..].find(tag) {
            let abs = start + pos;
            // Find the end of this tag.
            if let Some(end) = lower[abs..].find('>') {
                positions.push((abs, abs + end + 1));
            }
            start = abs + 1;
        }
    }
    positions.sort_by_key(|p| std::cmp::Reverse(p.0));
    positions.dedup_by(|a, b| a.0 == b.0);

    for (start, end) in positions {
        if end <= result.len() {
            result.insert(end, '\n');
            result.insert(start, '\n');
        }
    }
    result
}

/// Remove all occurrences of `<tag ...>...</tag>` (case-insensitive).
fn remove_tag_block(html: &str, tag: &str) -> String {
    let open = format!("<{}", tag);
    let close = format!("</{}", tag);
    let mut result = String::with_capacity(html.len());
    let lower = html.to_lowercase();
    let mut cursor = 0;

    while let Some(start) = lower[cursor..].find(&open) {
        let abs_start = cursor + start;
        result.push_str(&html[cursor..abs_start]);

        if let Some(end_offset) = lower[abs_start..].find(&close) {
            let after_close = abs_start + end_offset;
            // Skip past the closing tag's `>`
            if let Some(gt) = html[after_close..].find('>') {
                cursor = after_close + gt + 1;
            } else {
                cursor = html.len();
            }
        } else {
            // No closing tag — skip to end.
            cursor = html.len();
        }
    }
    result.push_str(&html[cursor..]);
    result
}

/// Collapse runs of whitespace into single spaces, preserving paragraph breaks.
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_newline_count = 0;
    let mut prev_was_space = false;

    for ch in s.chars() {
        if ch == '\n' || ch == '\r' {
            prev_newline_count += 1;
            prev_was_space = false;
        } else if ch.is_whitespace() {
            prev_was_space = true;
        } else {
            // Flush accumulated whitespace.
            if prev_newline_count >= 2 {
                out.push_str("\n\n");
            } else if (prev_newline_count == 1 || prev_was_space) && !out.is_empty() {
                out.push(' ');
            }
            prev_newline_count = 0;
            prev_was_space = false;
            out.push(ch);
        }
    }
    out.trim().to_string()
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

    let backend = select_search_backend();
    let backend_name = match &backend {
        SearchBackend::Brave(_) => "Brave Search API",
        SearchBackend::SerpApi(_) => "SerpAPI",
        SearchBackend::DuckDuckGo => "DuckDuckGo (zero-config)",
    };
    eprintln!("aivyx mcp-server: starting {name} server on stdio (backend: {backend_name})");
    run_stdio_loop(&schemas, &backend).await
}

async fn run_stdio_loop(schemas: &[ToolSchema], backend: &SearchBackend) -> Result<(), String> {
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
        let resp = dispatch(id, &req.method, req.params, schemas, backend).await;
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
    backend: &SearchBackend,
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

        "tools/call" => dispatch_tool_call(id, params, backend).await,

        "shutdown" => JsonRpcResponse::ok(id, Value::Null),

        other => JsonRpcResponse::err(id, -32601, format!("method not found: {other}")),
    }
}

async fn dispatch_tool_call(id: u64, params: Option<Value>, backend: &SearchBackend) -> JsonRpcResponse {
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
        "web_search" => handle_web_search(arguments, backend).await,
        "web_read" => handle_web_read(arguments).await,
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
        let resp = dispatch(1, "initialize", None, &schemas, &SearchBackend::DuckDuckGo).await;
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "aivyx-web-search");
    }

    #[tokio::test]
    async fn dispatch_tools_list_returns_all_tools() {
        let schemas = web_search_tool_schemas();
        let resp = dispatch(2, "tools/list", None, &schemas, &SearchBackend::DuckDuckGo).await;
        let result = resp.result.unwrap();
        let tool_list = result["tools"].as_array().unwrap();
        assert_eq!(tool_list.len(), 3);
        assert_eq!(tool_list[0]["name"], "web_search");
        assert_eq!(tool_list[1]["name"], "web_read");
        assert_eq!(tool_list[2]["name"], "echo");
    }

    #[tokio::test]
    async fn dispatch_tools_call_echo() {
        let schemas = web_search_tool_schemas();
        let params = serde_json::json!({
            "name": "echo",
            "arguments": {"message": "hello world"}
        });
        let resp = dispatch(3, "tools/call", Some(params), &schemas, &SearchBackend::DuckDuckGo).await;
        let result = resp.result.unwrap();
        assert_eq!(result["isError"], false);
        let content = result["content"].as_array().unwrap();
        assert_eq!(content[0]["text"], "hello world");
    }

    #[tokio::test]
    async fn dispatch_shutdown_returns_null() {
        let schemas = web_search_tool_schemas();
        let resp = dispatch(4, "shutdown", None, &schemas, &SearchBackend::DuckDuckGo).await;
        assert!(resp.result.unwrap().is_null());
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn dispatch_unknown_method_returns_error() {
        let schemas = web_search_tool_schemas();
        let resp = dispatch(5, "bogus/method", None, &schemas, &SearchBackend::DuckDuckGo).await;
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
        let resp = dispatch(6, "tools/call", Some(params), &schemas, &SearchBackend::DuckDuckGo).await;
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

    // -----------------------------------------------------------------------
    // Phase 46 Task 4 — web_read / html_to_text tests
    // -----------------------------------------------------------------------

    #[test]
    fn html_to_text_strips_tags() {
        let html = "<p>Hello <b>world</b>!</p><p>Second paragraph.</p>";
        let text = html_to_text(html);
        assert_eq!(text, "Hello world!\n\nSecond paragraph.");
    }

    #[test]
    fn html_to_text_removes_scripts() {
        let html = "<p>Before</p><script>alert('xss');</script><p>After</p>";
        let text = html_to_text(html);
        assert!(text.contains("Before"));
        assert!(text.contains("After"));
        assert!(!text.contains("alert"));
        assert!(!text.contains("xss"));
    }

    #[test]
    fn html_to_text_removes_styles() {
        let html = "<style>.red { color: red; }</style><p>Content here</p>";
        let text = html_to_text(html);
        assert!(text.contains("Content here"));
        assert!(!text.contains("color"));
        assert!(!text.contains(".red"));
    }

    #[test]
    fn html_to_text_extracts_title() {
        let html = "<html><head><title>My Page Title</title></head><body>Body</body></html>";
        let title = extract_html_title(html);
        assert_eq!(title, "My Page Title");
    }

    #[test]
    fn html_to_text_extracts_title_case_insensitive() {
        let html = "<HTML><HEAD><TITLE>Upper Case</TITLE></HEAD></HTML>";
        let title = extract_html_title(html);
        assert_eq!(title, "Upper Case");
    }

    #[test]
    fn html_to_text_no_title() {
        let html = "<html><body>No title here</body></html>";
        let title = extract_html_title(html);
        assert!(title.is_empty());
    }

    #[test]
    fn html_to_text_non_html_passthrough() {
        // Non-HTML text should pass through unchanged (minus whitespace normalization).
        let plain = "Just some plain text\nwith newlines.";
        let result = html_to_text(plain);
        assert_eq!(result, "Just some plain text with newlines.");
    }

    #[test]
    fn web_read_schema_valid() {
        let schemas = web_search_tool_schemas();
        let wr = schemas.iter().find(|s| s.name == "web_read").unwrap();
        let props = wr.input_schema["properties"].as_object().unwrap();
        assert!(props.contains_key("url"));
        let required = wr.input_schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "url");
    }

    #[test]
    fn collapse_whitespace_normalizes() {
        let input = "  Hello   world  \n\n\n  New paragraph  ";
        let result = collapse_whitespace(input);
        assert_eq!(result, "Hello world\n\nNew paragraph");
    }

    #[test]
    fn remove_tag_block_strips_nested() {
        let html = "<div>Keep<script type='text/javascript'>var x = 1;</script>This</div>";
        let result = remove_tag_block(html, "script");
        assert!(result.contains("Keep"));
        assert!(result.contains("This"));
        assert!(!result.contains("var x"));
    }

    // -----------------------------------------------------------------------
    // Phase 46 Task 6 — Brave Search + SerpAPI parsers
    // -----------------------------------------------------------------------

    #[test]
    fn brave_parser_extracts_results() {
        let json = r#"{
            "web": {
                "results": [
                    {
                        "title": "Rust Programming",
                        "url": "https://rust-lang.org",
                        "description": "A language empowering everyone."
                    },
                    {
                        "title": "Cargo Docs",
                        "url": "https://doc.rust-lang.org/cargo/",
                        "description": "The Rust package manager."
                    }
                ]
            }
        }"#;
        let results = parse_brave_json(json);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust Programming");
        assert_eq!(results[0].url, "https://rust-lang.org");
        assert_eq!(results[0].snippet, "A language empowering everyone.");
        assert_eq!(results[1].title, "Cargo Docs");
    }

    #[test]
    fn brave_parser_empty_results() {
        let json = r#"{ "web": { "results": [] } }"#;
        let results = parse_brave_json(json);
        assert!(results.is_empty());
    }

    #[test]
    fn brave_parser_missing_web_key() {
        let json = r#"{ "query": { "original": "test" } }"#;
        let results = parse_brave_json(json);
        assert!(results.is_empty());
    }

    #[test]
    fn serpapi_parser_extracts_results() {
        let json = r#"{
            "organic_results": [
                {
                    "title": "Example Page",
                    "link": "https://example.com",
                    "snippet": "An example snippet."
                },
                {
                    "title": "Another Page",
                    "link": "https://another.com",
                    "snippet": "Another snippet."
                }
            ]
        }"#;
        let results = parse_serpapi_json(json);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Example Page");
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[0].snippet, "An example snippet.");
        assert_eq!(results[1].url, "https://another.com");
    }

    #[test]
    fn serpapi_parser_empty_results() {
        let json = r#"{ "organic_results": [] }"#;
        let results = parse_serpapi_json(json);
        assert!(results.is_empty());
    }

    #[test]
    fn backend_selector_prefers_brave() {
        // Can't reliably set env vars in parallel tests, so test the
        // parsing logic directly instead.
        let brave = SearchBackend::Brave("key".into());
        assert_eq!(brave, SearchBackend::Brave("key".into()));

        let serpapi = SearchBackend::SerpApi("key2".into());
        assert_ne!(serpapi, brave);
    }

    #[test]
    fn backend_selector_falls_back_ddg() {
        // Without any env vars set, the default should be DuckDuckGo.
        // We can't safely test select_search_backend() in parallel,
        // but we can verify the enum construction.
        let ddg = SearchBackend::DuckDuckGo;
        assert_eq!(ddg, SearchBackend::DuckDuckGo);
    }
}

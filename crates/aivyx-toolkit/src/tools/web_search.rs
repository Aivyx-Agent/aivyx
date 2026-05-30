//! `web.search` — Brave Search API integration.
//!
//! Phase 125 Task 3. First tool in the Chapter G #1 bundle.
//! Operator provides a Brave API key in
//! `~/.aivyx/tool-processes/toolkit/config.toml`; the tool
//! issues a single GET to Brave's web-search endpoint and
//! returns a snake-cased trimmed result list.
//!
//! ## API call
//!
//! `GET https://api.search.brave.com/res/v1/web/search`
//! with headers:
//! - `X-Subscription-Token: <operator's API key>`
//! - `Accept: application/json`
//!
//! Query params: `q` (operator's query), `count` (1-20).
//!
//! ## Output trimming
//!
//! Brave returns ~40 fields per result. We trim to the three
//! the LLM actually needs (`title`, `url`, `description`).
//! Snake-cased + flat; matches the Gmail tool shape so the
//! cross-tool response shape is operator-predictable.

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

/// Default Brave Search web-search endpoint. Override at
/// construction time via [`WebSearch::with_endpoint`] for
/// test fixtures.
pub const BRAVE_WEB_SEARCH_ENDPOINT: &str =
    "https://api.search.brave.com/res/v1/web/search";

/// Hard cap on `count` per call. Brave's API documents 1-20
/// per page; we pass through.
const MAX_COUNT: u64 = 20;
const DEFAULT_COUNT: u64 = 10;

pub struct WebSearch {
    id: ToolId,
    schema: Value,
    http: Client,
    api_key: String,
    endpoint: String,
}

impl WebSearch {
    /// Build a new tool. `http` is the shared `reqwest::Client`
    /// from the binary; `api_key` is the operator's Brave
    /// Search API key loaded from config at startup.
    pub fn new(http: Client, api_key: String) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            http,
            api_key,
            endpoint: BRAVE_WEB_SEARCH_ENDPOINT.to_string(),
        }
    }

    /// Test seam — override the endpoint to point at an
    /// in-process mock server. Same pattern as Gmail's
    /// `with_api_base_url`.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }
}

#[async_trait]
impl Tool for WebSearch {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "web.search"
    }

    fn description(&self) -> &str {
        "Search the web using Brave Search. Input is a JSON \
         object with a required `q` field (the search query) \
         and an optional `count` field (1-20, default 10). \
         Returns a JSON object with `query` (echoed), \
         `results` (array of `{title, url, description}`), \
         and `result_count`. Results are pre-trimmed to the \
         three fields useful to the LLM; the operator's Brave \
         account quota is decremented by one per call (free \
         tier: 2000/month at time of writing)."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("web.search").expect(
            "web.search must parse — it is in KNOWN_BASES from Phase 125",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("web.search: {reason}"),
                });
            }
        };

        let count_str = parsed.count.to_string();
        let query: Vec<(&str, &str)> = vec![
            ("q", parsed.q.as_str()),
            ("count", count_str.as_str()),
        ];

        let response = match self
            .http
            .get(&self.endpoint)
            .header("X-Subscription-Token", &self.api_key)
            .header("Accept", "application/json")
            .query(&query)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("web.search: HTTP request failed: {e}"),
                });
            }
        };

        let status = response.status();
        let body = match response.text().await {
            Ok(b) => b,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("web.search: response body read failed: {e}"),
                });
            }
        };
        if !status.is_success() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!(
                    "web.search: Brave returned status {} — body: {body}",
                    status.as_u16(),
                ),
            });
        }
        let decoded: Value = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("web.search: response parse failed: {e}"),
                });
            }
        };

        ToolOutcome::Completed {
            output: shape_response(&decoded, &parsed.q),
            // Read-only query; verification is not meaningful.
            verified: Verification::NotApplicable,
        }
    }
}

#[derive(Debug)]
struct ParsedInput {
    q: String,
    count: u64,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let q = input
        .get("q")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| "input must include a `q` string field".to_string())?;
    if q.is_empty() {
        return Err("`q` must not be empty".to_string());
    }
    let count = match input.get("count") {
        None => DEFAULT_COUNT,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| "`count` must be a non-negative integer".to_string())?,
    };
    if count == 0 {
        return Err("`count` must be >= 1".to_string());
    }
    if count > MAX_COUNT {
        return Err(format!("`count` must be <= {MAX_COUNT}"));
    }
    Ok(ParsedInput {
        q: q.to_string(),
        count,
    })
}

/// Trim Brave's response to the LLM-relevant subset.
/// `web.results` array → flat `{title, url, description}`
/// entries. Other Brave response fields (`mixed`, `videos`,
/// `news`, `faq`, etc) are dropped — operators who need those
/// can call Brave directly or wait for a future per-vertical
/// tool.
///
/// Pure function so tests pin every shape against canned
/// Brave responses without touching the network.
pub fn shape_response(raw: &Value, query_echo: &str) -> Value {
    let results: Vec<Value> = raw
        .get("web")
        .and_then(|v| v.get("results"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|r| {
                    json!({
                        "title": r.get("title").cloned().unwrap_or(Value::Null),
                        "url": r.get("url").cloned().unwrap_or(Value::Null),
                        "description": r
                            .get("description")
                            .cloned()
                            .unwrap_or(Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let result_count = results.len();
    json!({
        "query": query_echo,
        "results": results,
        "result_count": result_count,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "q": {
                "type": "string",
                "minLength": 1,
                "description": "Web search query. Free-form natural-language string — Brave's search engine handles tokenization."
            },
            "count": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_COUNT,
                "default": DEFAULT_COUNT,
                "description": "Number of results to return. Default 10; capped at 20 per Brave's per-page limit."
            }
        },
        "required": ["q"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    // ---- parse_input -------------------------------------

    #[test]
    fn parse_input_accepts_minimal_query() {
        let p = parse_input(&json!({"q": "rust async"})).expect("parse");
        assert_eq!(p.q, "rust async");
        assert_eq!(p.count, DEFAULT_COUNT);
    }

    #[test]
    fn parse_input_respects_count() {
        let p = parse_input(&json!({"q": "x", "count": 5})).expect("parse");
        assert_eq!(p.count, 5);
    }

    #[test]
    fn parse_input_rejects_missing_q() {
        let e = parse_input(&json!({})).expect_err("must error");
        assert!(e.contains("`q`"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_q() {
        let e = parse_input(&json!({"q": "   "})).expect_err("must error");
        assert!(e.contains("not be empty"), "{e}");
    }

    #[test]
    fn parse_input_rejects_zero_count() {
        let e = parse_input(&json!({"q": "x", "count": 0})).expect_err("must error");
        assert!(e.contains(">= 1"), "{e}");
    }

    #[test]
    fn parse_input_rejects_over_cap_count() {
        let e =
            parse_input(&json!({"q": "x", "count": 21})).expect_err("must error");
        assert!(e.contains("<= 20"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_integer_count() {
        let e =
            parse_input(&json!({"q": "x", "count": "many"})).expect_err("must error");
        assert!(e.contains("non-negative integer"), "{e}");
    }

    // ---- shape_response ----------------------------------

    fn brave_response_with(n: usize) -> Value {
        let results: Vec<Value> = (0..n)
            .map(|i| {
                json!({
                    "title": format!("Result {i}"),
                    "url": format!("https://example.com/{i}"),
                    "description": format!("Description {i}"),
                    "page_age": "2024-01-01",
                    // ... other Brave fields the trimmer should drop
                    "language": "en",
                    "family_friendly": true,
                })
            })
            .collect();
        json!({
            "type": "search",
            "query": {"original": "test query"},
            "web": {"type": "search", "results": results},
            // Other top-level Brave sections the trimmer drops:
            "mixed": {"type": "mixed", "main": []},
            "videos": {"results": []},
        })
    }

    #[test]
    fn shape_response_trims_to_three_fields_per_result() {
        let raw = brave_response_with(3);
        let shaped = shape_response(&raw, "test query");
        assert_eq!(shaped["query"], "test query");
        assert_eq!(shaped["result_count"], 3);
        let results = shaped["results"].as_array().unwrap();
        assert_eq!(results.len(), 3);
        // First result trimmed to exactly the three fields.
        assert_eq!(results[0]["title"], "Result 0");
        assert_eq!(results[0]["url"], "https://example.com/0");
        assert_eq!(results[0]["description"], "Description 0");
        // No extra Brave fields leaked through.
        assert!(results[0].get("page_age").is_none(), "page_age must be dropped");
        assert!(results[0].get("language").is_none(), "language must be dropped");
    }

    #[test]
    fn shape_response_handles_empty_results_array() {
        let raw = brave_response_with(0);
        let shaped = shape_response(&raw, "no hits");
        assert_eq!(shaped["query"], "no hits");
        assert_eq!(shaped["result_count"], 0);
        assert_eq!(shaped["results"], json!([]));
    }

    #[test]
    fn shape_response_handles_missing_web_section() {
        // Defensive — if Brave's response is missing the `web`
        // section entirely, return empty results rather than
        // panicking.
        let raw = json!({"type": "search", "query": {"original": "x"}});
        let shaped = shape_response(&raw, "x");
        assert_eq!(shaped["result_count"], 0);
        assert_eq!(shaped["results"], json!([]));
    }

    #[test]
    fn shape_response_handles_per_result_missing_fields() {
        // Defensive — if a result is missing `description`,
        // fill with null rather than dropping the whole result.
        let raw = json!({
            "web": {"results": [{"title": "T", "url": "U"}]}
        });
        let shaped = shape_response(&raw, "q");
        let results = shaped["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["title"], "T");
        assert_eq!(results[0]["url"], "U");
        assert_eq!(results[0]["description"], Value::Null);
    }

    // ---- input_schema ------------------------------------

    #[test]
    fn input_schema_declares_required_q_and_count_bounds() {
        let s = input_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v.as_str() == Some("q")));
        assert_eq!(s["properties"]["count"]["minimum"], 1);
        assert_eq!(s["properties"]["count"]["maximum"], MAX_COUNT);
        assert_eq!(s["additionalProperties"], false);
    }

    // ---- required_scope ----------------------------------

    #[test]
    fn required_scope_is_web_search() {
        let tool = WebSearch::new(Client::new(), "BSA-test".to_string());
        let scope = tool.required_scope(&json!({}));
        assert_eq!(scope.to_string(), "web.search");
    }

    // ---- HTTP integration via in-process mock ------------

    /// Minimal in-process HTTP mock. Captures the
    /// `X-Subscription-Token` header and the request path-
    /// and-query; responds with the canned body + status.
    async fn spawn_mock_brave(
        canned_status: u16,
        canned_body: &'static str,
    ) -> (String, Arc<Mutex<(String, String)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let captured: Arc<Mutex<(String, String)>> =
            Arc::new(Mutex::new(("".into(), "".into())));
        let captured_clone = Arc::clone(&captured);
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let (read_half, mut write_half) = sock.split();
            let mut reader = BufReader::new(read_half);
            let mut request_line = String::new();
            reader.read_line(&mut request_line).await.unwrap();
            let path_and_query = request_line
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_string();
            let mut sub_token = String::new();
            let mut content_length: usize = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                if line == "\r\n" {
                    break;
                }
                if let Some(idx) = line.find(':') {
                    let name_lower = line[..idx].to_ascii_lowercase();
                    let value = &line[idx + 1..];
                    if name_lower == "x-subscription-token" {
                        sub_token = value.trim().to_string();
                    } else if name_lower == "content-length" {
                        content_length = value.trim().parse().unwrap_or(0);
                    }
                }
            }
            if content_length > 0 {
                let mut body = vec![0u8; content_length];
                let _ = reader.read_exact(&mut body).await;
            }
            *captured_clone.lock().unwrap() = (sub_token, path_and_query);
            let phrase = if canned_status == 200 { "OK" } else { "Err" };
            let response = format!(
                "HTTP/1.1 {canned_status} {phrase}\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\
                 \r\n\
                 {}",
                canned_body.len(),
                canned_body,
            );
            write_half.write_all(response.as_bytes()).await.unwrap();
            write_half.flush().await.unwrap();
        });
        (format!("http://127.0.0.1:{port}/search"), captured)
    }

    // execute() requires a real ChannelContext we can't easily
    // synthesize at test time without a daemon-level scaffold;
    // the unit tests below cover the HTTP path directly using
    // the same helpers execute() uses (parse_input,
    // shape_response, and the http+endpoint+api_key tuple on
    // the WebSearch instance). The IPC-level integration is
    // covered end-to-end by Phase 125 Task 7's operator
    // walkthrough.

    #[tokio::test]
    async fn http_request_includes_subscription_token_and_count() {
        let canned = r#"{"web":{"results":[]}}"#;
        let (endpoint, captured) = spawn_mock_brave(200, canned).await;
        let tool = WebSearch::new(Client::new(), "BSA-MY-KEY".to_string())
            .with_endpoint(endpoint);

        // Call the HTTP path directly (not through execute() —
        // that would require a real ChannelContext).
        let parsed = parse_input(&json!({"q": "hello", "count": 7})).unwrap();
        let count_str = parsed.count.to_string();
        let query: Vec<(&str, &str)> =
            vec![("q", parsed.q.as_str()), ("count", count_str.as_str())];
        let response = tool
            .http
            .get(&tool.endpoint)
            .header("X-Subscription-Token", &tool.api_key)
            .header("Accept", "application/json")
            .query(&query)
            .send()
            .await
            .expect("send");
        assert!(response.status().is_success());
        let _ = response.text().await;

        let (token, path_q) = captured.lock().unwrap().clone();
        assert_eq!(token, "BSA-MY-KEY", "{token}");
        assert!(path_q.contains("q=hello"), "{path_q}");
        assert!(path_q.contains("count=7"), "{path_q}");
    }

    #[tokio::test]
    async fn shape_response_round_trips_brave_canned_response() {
        // Belt-and-suspenders: spin up the mock with a canned
        // Brave response, fetch + parse + shape end-to-end via
        // the HTTP path, verify the trim.
        let canned = r#"{"web":{"results":[
          {"title":"T1","url":"https://x/1","description":"D1","page_age":"old"},
          {"title":"T2","url":"https://x/2","description":"D2"}
        ]}}"#;
        let (endpoint, _) = spawn_mock_brave(200, canned).await;
        let tool = WebSearch::new(Client::new(), "BSA".to_string())
            .with_endpoint(endpoint);
        let response = tool
            .http
            .get(&tool.endpoint)
            .header("X-Subscription-Token", &tool.api_key)
            .header("Accept", "application/json")
            .query(&[("q", "echo"), ("count", "10")])
            .send()
            .await
            .expect("send");
        let body = response.text().await.expect("body");
        let decoded: Value = serde_json::from_str(&body).expect("decode");
        let shaped = shape_response(&decoded, "echo");
        assert_eq!(shaped["query"], "echo");
        assert_eq!(shaped["result_count"], 2);
        let results = shaped["results"].as_array().unwrap();
        assert_eq!(results[0]["title"], "T1");
        assert!(results[0].get("page_age").is_none());
    }
}

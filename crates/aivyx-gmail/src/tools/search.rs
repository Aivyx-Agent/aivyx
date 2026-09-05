//! `gmail.search` — Gmail message-list query.
//!
//! Phase 123 Task 4. The first Gmail tool. Implements
//! [`aivyx_core::Tool`]; served by the multi-tool harness in
//! [`crate::harness`].
//!
//! ## API call
//!
//! Single GET to `users/me/messages` with the operator-supplied
//! `q` parameter (Gmail query DSL — e.g. `from:alice
//! is:unread`). Returns the message IDs only — enrichment via
//! `gmail.read` (Task 5) which returns the full message body.
//!
//! The single-call shape is the "easy wins first" pick at
//! Phase 123 sign-off: a future phase can add a richer
//! `gmail.search_with_metadata` if operators surface pressure
//! for snippet-in-search-results.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

use crate::gmail_client::SharedGmailClient;

/// Hard cap on `max_results` per call. Gmail's API supports
/// up to 500 per page, but for a single tool call we cap at
/// 100 — keeps the response size predictable for the LLM
/// without preventing operator-scripted bulk fetches.
const MAX_RESULTS_CAP: u64 = 100;
const DEFAULT_MAX_RESULTS: u64 = 25;

/// `gmail.search` tool. Holds a shared Gmail client; the tool
/// itself is stateless beyond the client + a fixed input
/// schema.
pub struct GmailSearch {
    id: ToolId,
    schema: Value,
    client: SharedGmailClient,
}

impl GmailSearch {
    pub fn new(client: SharedGmailClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for GmailSearch {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "gmail.search"
    }

    // Chapter Picket follow-up (Finding 3) — search results include
    // message snippets and subjects, externally authored email
    // content that may carry a prompt-injection payload.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Search Gmail messages using Gmail's query DSL. Input \
         is a JSON object with a required `q` field (the Gmail \
         query string — e.g. `from:alice is:unread`, \
         `subject:invoice`, `label:starred has:attachment`), \
         an optional `max_results` field (default 25, capped \
         at 100), and an optional `include_spam_trash` field \
         (default false). Returns a JSON object with a \
         `messages` array of `{id, thread_id}` pairs, a \
         `result_size_estimate` count, and an optional \
         `next_page_token` for pagination. Message bodies and \
         snippets are NOT in the response — pass each `id` to \
         `gmail.read` for the full message."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("email.read").expect(
            "email.read must parse — it is in KNOWN_BASES from Phase 123",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("gmail.search: {reason}"),
                });
            }
        };

        // Build query string for the Gmail API.
        let max_results_str = parsed.max_results.to_string();
        let include_spam_trash_str = if parsed.include_spam_trash {
            "true"
        } else {
            "false"
        };
        let query: Vec<(&str, &str)> = vec![
            ("q", parsed.q.as_str()),
            ("maxResults", max_results_str.as_str()),
            ("includeSpamTrash", include_spam_trash_str),
        ];

        let response = match self
            .client
            .get("/users/me/messages", &query)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("gmail.search: API call failed: {e}"),
                });
            }
        };
        let body = match self.client.decode_json(response).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("gmail.search: response decode failed: {e}"),
                });
            }
        };

        // Transform Gmail's response into a stable
        // snake-case shape; tools-side JSON ergonomics
        // matter more than camelCase API fidelity.
        let messages: Vec<Value> = body
            .get("messages")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|m| {
                        json!({
                            "id": m.get("id").cloned().unwrap_or(Value::Null),
                            "thread_id": m.get("threadId").cloned().unwrap_or(Value::Null),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let result_size_estimate = body
            .get("resultSizeEstimate")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let next_page_token = body
            .get("nextPageToken")
            .and_then(|v| v.as_str())
            .map(String::from);

        let mut output = json!({
            "messages": messages,
            "result_size_estimate": result_size_estimate,
        });
        if let Some(token) = next_page_token {
            output["next_page_token"] = Value::String(token);
        }

        ToolOutcome::Completed {
            output,
            // `Verified` is wrong — we don't double-check the
            // search results landed. `NotApplicable` is right
            // for read-only queries per the TOOL_SDK Verification
            // semantics: "verification is not meaningful (read-
            // only query)."
            verified: Verification::NotApplicable,
        }
    }
}

/// Parsed + validated input. Reject malformed payloads at this
/// boundary so the execute() body operates on already-clean
/// values.
#[derive(Debug)]
struct ParsedInput {
    q: String,
    max_results: u64,
    include_spam_trash: bool,
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
    let max_results = match input.get("max_results") {
        None => DEFAULT_MAX_RESULTS,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| "`max_results` must be a non-negative integer".to_string())?,
    };
    if max_results == 0 {
        return Err("`max_results` must be >= 1".to_string());
    }
    let max_results = max_results.min(MAX_RESULTS_CAP);
    let include_spam_trash = match input.get("include_spam_trash") {
        None => false,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| "`include_spam_trash` must be a boolean".to_string())?,
    };
    Ok(ParsedInput {
        q: q.to_string(),
        max_results,
        include_spam_trash,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "q": {
                "type": "string",
                "description": "Gmail query DSL — e.g. `in:inbox is:unread`, `from:alice@example.com`, `subject:invoice has:attachment`. Required."
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_RESULTS_CAP,
                "default": DEFAULT_MAX_RESULTS,
                "description": "Maximum messages to return in this call. Capped at 100."
            },
            "include_spam_trash": {
                "type": "boolean",
                "default": false,
                "description": "Include results from Spam and Trash. Default false."
            }
        },
        "required": ["q"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail_search_output_is_untrusted_for_bulwark() {
        let client = std::sync::Arc::new(crate::GmailClient::new(
            reqwest::Client::new(),
            crate::OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            crate::TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            std::path::PathBuf::from("/tmp/unused"),
        ));
        let tool = GmailSearch::new(client);
        assert!(tool.output_is_untrusted());
    }

    #[test]
    fn parse_input_accepts_minimal_query() {
        let input = json!({"q": "in:inbox"});
        let p = parse_input(&input).expect("parse");
        assert_eq!(p.q, "in:inbox");
        assert_eq!(p.max_results, DEFAULT_MAX_RESULTS);
        assert!(!p.include_spam_trash);
    }

    #[test]
    fn parse_input_caps_max_results_at_100() {
        let input = json!({"q": "in:inbox", "max_results": 500});
        let p = parse_input(&input).expect("parse");
        assert_eq!(p.max_results, MAX_RESULTS_CAP);
    }

    #[test]
    fn parse_input_honors_include_spam_trash() {
        let input = json!({"q": "in:inbox", "include_spam_trash": true});
        let p = parse_input(&input).expect("parse");
        assert!(p.include_spam_trash);
    }

    #[test]
    fn parse_input_rejects_missing_q() {
        let input = json!({"max_results": 10});
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("`q`"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_q() {
        let input = json!({"q": "   "});
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("not be empty"), "{e}");
    }

    #[test]
    fn parse_input_rejects_zero_max_results() {
        let input = json!({"q": "x", "max_results": 0});
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains(">= 1"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_integer_max_results() {
        let input = json!({"q": "x", "max_results": "lots"});
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("non-negative integer"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_boolean_include_spam_trash() {
        let input = json!({"q": "x", "include_spam_trash": "yes"});
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("boolean"), "{e}");
    }

    #[test]
    fn input_schema_declares_q_required_and_bounds_max_results() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required array");
        assert!(req.iter().any(|v| v.as_str() == Some("q")));
        assert_eq!(schema["properties"]["max_results"]["maximum"], MAX_RESULTS_CAP);
        assert_eq!(schema["properties"]["max_results"]["minimum"], 1);
        // additionalProperties: false so the schema-validation
        // gate at the daemon side rejects payloads carrying
        // unknown fields (catches typos at the planner layer
        // before they reach our tool).
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn required_scope_is_email_read() {
        let client = std::sync::Arc::new(crate::GmailClient::new(
            reqwest::Client::new(),
            crate::OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            crate::TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            std::path::PathBuf::from("/tmp/unused"),
        ));
        let tool = GmailSearch::new(client);
        let scope = tool.required_scope(&json!({}));
        assert_eq!(scope.to_string(), "email.read");
    }
}

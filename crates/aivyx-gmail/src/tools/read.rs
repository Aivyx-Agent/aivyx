//! `gmail.read` — fetch one full Gmail message by ID.
//!
//! Phase 123 Task 5. Same `email.read` scope as `gmail.search`
//! — the contract is symmetric: a role granted `email.read`
//! can both search the inbox AND read any message it has the
//! ID for.
//!
//! ## API call
//!
//! Single GET to `users/me/messages/{id}?format=full`. Gmail
//! returns the full payload tree (recursive MIME structure
//! with headers, body parts, attachment metadata). This tool
//! flattens that into an LLM-friendly snake-cased shape:
//!
//! ```json
//! {
//!   "id": "...", "thread_id": "...",
//!   "label_ids": ["INBOX", "UNREAD"],
//!   "snippet": "first ~200 chars of preview",
//!   "internal_date_ms": 1703772345678,
//!   "headers": {"from": "...", "to": "...", "subject": "..."},
//!   "body_text": "extracted plain-text body or empty",
//!   "body_html": "extracted HTML body or empty",
//!   "attachments": [
//!     {"filename": "...", "mime_type": "...",
//!      "size_bytes": N, "attachment_id": "..."}
//!   ]
//! }
//! ```
//!
//! ## Attachment download is NOT in this tool
//!
//! The `attachment_id` is returned but attachment bytes are
//! not downloaded. A future tool (`gmail.attachment.read` or
//! similar) can handle bulk binary fetches if operators
//! surface pressure. For Phase 123 substrate, message body
//! text + attachment metadata is enough for an LLM to triage
//! inbox content.

use async_trait::async_trait;
use base64::prelude::{Engine, BASE64_URL_SAFE_NO_PAD};
use serde_json::{json, Map, Value};

use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

use crate::gmail_client::SharedGmailClient;

pub struct GmailRead {
    id: ToolId,
    schema: Value,
    client: SharedGmailClient,
}

impl GmailRead {
    pub fn new(client: SharedGmailClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for GmailRead {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "gmail.read"
    }

    // Chapter Picket follow-up (Finding 3) — the email body and
    // headers are externally authored content that may carry a
    // prompt-injection payload.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Read one Gmail message by ID. Input is a JSON object \
         with a required `id` field (the message ID returned by \
         `gmail.search`). Returns a flattened JSON object: \
         `id`, `thread_id`, `label_ids`, `snippet`, \
         `internal_date_ms` (UNIX millis), `headers` (flat map \
         with lowercased common-header names like `from`, `to`, \
         `subject`, `date`, `message_id`, `cc`, `bcc`, \
         `reply_to`, `in_reply_to`, `references`), `body_text` \
         (extracted plain-text body or empty string), \
         `body_html` (extracted HTML body or empty string), and \
         `attachments` (array of `{filename, mime_type, \
         size_bytes, attachment_id}`). Attachment bytes are \
         NOT downloaded — fetch separately if needed."
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
        let id = match parse_input(&input) {
            Ok(id) => id,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("gmail.read: {reason}"),
                });
            }
        };

        let path = format!("/users/me/messages/{id}");
        let query = [("format", "full")];
        let response = match self.client.get(&path, &query).await {
            Ok(r) => r,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("gmail.read: API call failed: {e}"),
                });
            }
        };
        let body = match self.client.decode_json(response).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("gmail.read: response decode failed: {e}"),
                });
            }
        };

        ToolOutcome::Completed {
            output: shape_message(&body),
            // Read-only fetch; no double-check possible.
            verified: Verification::NotApplicable,
        }
    }
}

fn parse_input(input: &Value) -> Result<String, String> {
    let id = input
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| "input must include an `id` string field".to_string())?;
    if id.is_empty() {
        return Err("`id` must not be empty".to_string());
    }
    // Gmail message IDs are URL-safe-base64 hex-ish strings;
    // reject anything with a slash so the URL path stays sane
    // (defense-in-depth — the Gmail API would also reject).
    if id.contains('/') || id.contains('?') || id.contains('#') {
        return Err("`id` contains invalid characters".to_string());
    }
    Ok(id.to_string())
}

/// Transform Gmail's raw `messages.get` response into the
/// flattened LLM-friendly shape. Pure function so unit tests
/// can pin every field shape against canned Gmail responses
/// without touching the network.
pub fn shape_message(raw: &Value) -> Value {
    let id = raw.get("id").cloned().unwrap_or(Value::Null);
    let thread_id = raw.get("threadId").cloned().unwrap_or(Value::Null);
    let snippet = raw.get("snippet").cloned().unwrap_or(Value::String(String::new()));
    let label_ids = raw
        .get("labelIds")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let internal_date_ms = raw
        .get("internalDate")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<i64>().ok())
        .map(Value::from)
        .unwrap_or(Value::Null);

    let payload = raw.get("payload");
    let headers = payload
        .map(extract_headers)
        .unwrap_or_else(|| Value::Object(Map::new()));
    let (body_text, body_html, attachments) = payload
        .map(walk_payload)
        .unwrap_or_else(|| (String::new(), String::new(), Vec::new()));

    json!({
        "id": id,
        "thread_id": thread_id,
        "label_ids": label_ids,
        "snippet": snippet,
        "internal_date_ms": internal_date_ms,
        "headers": headers,
        "body_text": body_text,
        "body_html": body_html,
        "attachments": attachments,
    })
}

/// Flatten the `payload.headers` array into a lowercase-keyed
/// map. Common headers (from / to / cc / bcc / reply-to /
/// in-reply-to / references / subject / date / message-id)
/// land with `lower-kebab → lower_snake` translation so LLMs
/// can reference them in stable field syntax.
fn extract_headers(payload: &Value) -> Value {
    let arr = match payload.get("headers").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Value::Object(Map::new()),
    };
    let mut out = Map::new();
    for entry in arr {
        let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let value = entry.get("value").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let key = normalize_header_name(name);
        // For repeated headers (Received, etc) keep the FIRST
        // occurrence — Gmail returns them in receive-order and
        // the first is the most-recent hop in this hop-direction.
        out.entry(key).or_insert_with(|| Value::String(value.to_string()));
    }
    Value::Object(out)
}

/// `Message-ID` → `message_id`; `In-Reply-To` → `in_reply_to`;
/// `Reply-To` → `reply_to`. Pure-Rust ASCII-lower + `-` → `_`.
fn normalize_header_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch == '-' {
            out.push('_');
        } else {
            out.extend(ch.to_lowercase());
        }
    }
    out
}

/// Walk the recursive payload tree extracting:
/// - `body_text`: concatenated `text/plain` parts.
/// - `body_html`: concatenated `text/html` parts.
/// - `attachments`: parts with a non-empty `filename` field.
fn walk_payload(payload: &Value) -> (String, String, Vec<Value>) {
    let mut body_text = String::new();
    let mut body_html = String::new();
    let mut attachments: Vec<Value> = Vec::new();
    walk_payload_inner(payload, &mut body_text, &mut body_html, &mut attachments);
    (body_text, body_html, attachments)
}

fn walk_payload_inner(
    node: &Value,
    body_text: &mut String,
    body_html: &mut String,
    attachments: &mut Vec<Value>,
) {
    let mime_type = node
        .get("mimeType")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let filename = node
        .get("filename")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Attachment? (filename non-empty means this part is an
    // attachment, not an inline body part.)
    if !filename.is_empty() {
        let body_obj = node.get("body");
        let size_bytes = body_obj
            .and_then(|b| b.get("size"))
            .and_then(|s| s.as_u64())
            .unwrap_or(0);
        let attachment_id = body_obj
            .and_then(|b| b.get("attachmentId"))
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .to_string();
        attachments.push(json!({
            "filename": filename,
            "mime_type": mime_type,
            "size_bytes": size_bytes,
            "attachment_id": attachment_id,
        }));
        // Don't descend into attachment parts — their body
        // bytes aren't returned in `format=full` (only the
        // `attachmentId` is).
        return;
    }

    // Inline body part?
    if mime_type.starts_with("text/") {
        let data = node
            .get("body")
            .and_then(|b| b.get("data"))
            .and_then(|d| d.as_str())
            .unwrap_or("");
        if !data.is_empty() {
            let decoded = decode_base64url_lossy(data);
            if mime_type.starts_with("text/plain") {
                if !body_text.is_empty() {
                    body_text.push('\n');
                }
                body_text.push_str(&decoded);
            } else if mime_type.starts_with("text/html") {
                if !body_html.is_empty() {
                    body_html.push('\n');
                }
                body_html.push_str(&decoded);
            }
            // Other text/* types (text/calendar, text/csv) are
            // silently dropped at v1 — they'd land under
            // attachments if Gmail set a filename.
        }
    }

    // Recurse into multipart parts.
    if let Some(parts) = node.get("parts").and_then(|p| p.as_array()) {
        for part in parts {
            walk_payload_inner(part, body_text, body_html, attachments);
        }
    }
}

/// Decode Gmail's base64url body data. Accepts both padded
/// and unpadded forms by stripping `=` first; tolerates
/// invalid input by returning a debug placeholder rather than
/// failing the whole message (one bad part shouldn't sink the
/// whole read).
fn decode_base64url_lossy(s: &str) -> String {
    let stripped: String = s.chars().filter(|c| *c != '=' && !c.is_whitespace()).collect();
    match BASE64_URL_SAFE_NO_PAD.decode(stripped.as_bytes()) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => format!("<aivyx-gmail: undecodable base64url body, {} chars raw>", s.len()),
    }
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "minLength": 1,
                "description": "Gmail message ID — typically returned by `gmail.search` in the `messages[].id` field."
            }
        },
        "required": ["id"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::prelude::{Engine, BASE64_URL_SAFE_NO_PAD};

    #[test]
    fn gmail_read_output_is_untrusted_for_bulwark() {
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
        let tool = GmailRead::new(client);
        assert!(tool.output_is_untrusted());
    }

    fn b64url(s: &str) -> String {
        BASE64_URL_SAFE_NO_PAD.encode(s.as_bytes())
    }

    #[test]
    fn parse_input_accepts_well_formed_id() {
        let p = parse_input(&json!({"id": "18a2c3b4d5e6f"})).expect("parse");
        assert_eq!(p, "18a2c3b4d5e6f");
    }

    #[test]
    fn parse_input_rejects_missing_id() {
        let e = parse_input(&json!({})).expect_err("must error");
        assert!(e.contains("`id`"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_id() {
        let e = parse_input(&json!({"id": "   "})).expect_err("must error");
        assert!(e.contains("not be empty"), "{e}");
    }

    #[test]
    fn parse_input_rejects_url_metachars() {
        for bad in ["a/b", "a?b", "a#b"] {
            let e = parse_input(&json!({"id": bad})).expect_err(bad);
            assert!(e.contains("invalid characters"), "{e}");
        }
    }

    #[test]
    fn normalize_header_name_lowercases_and_translates_hyphens() {
        assert_eq!(normalize_header_name("From"), "from");
        assert_eq!(normalize_header_name("In-Reply-To"), "in_reply_to");
        assert_eq!(normalize_header_name("Message-ID"), "message_id");
        assert_eq!(normalize_header_name("X-Custom-Header"), "x_custom_header");
    }

    #[test]
    fn extract_headers_flattens_array_and_lowercases_keys() {
        let payload = json!({
            "headers": [
                {"name": "From", "value": "alice@example.com"},
                {"name": "To", "value": "bob@example.com"},
                {"name": "Subject", "value": "hi"},
                {"name": "Message-ID", "value": "<id-x@example.com>"},
            ]
        });
        let h = extract_headers(&payload);
        assert_eq!(h["from"], "alice@example.com");
        assert_eq!(h["to"], "bob@example.com");
        assert_eq!(h["subject"], "hi");
        assert_eq!(h["message_id"], "<id-x@example.com>");
    }

    #[test]
    fn extract_headers_keeps_first_occurrence_of_repeated_header() {
        let payload = json!({
            "headers": [
                {"name": "Received", "value": "hop-A"},
                {"name": "Received", "value": "hop-B"},
            ]
        });
        let h = extract_headers(&payload);
        assert_eq!(h["received"], "hop-A");
    }

    #[test]
    fn walk_payload_text_plain_only() {
        let payload = json!({
            "mimeType": "text/plain",
            "body": {"data": b64url("hello world"), "size": 11},
        });
        let (text, html, atts) = walk_payload(&payload);
        assert_eq!(text, "hello world");
        assert!(html.is_empty());
        assert!(atts.is_empty());
    }

    #[test]
    fn walk_payload_multipart_alternative_extracts_both_bodies() {
        let payload = json!({
            "mimeType": "multipart/alternative",
            "parts": [
                {
                    "mimeType": "text/plain",
                    "body": {"data": b64url("plain version")}
                },
                {
                    "mimeType": "text/html",
                    "body": {"data": b64url("<p>html version</p>")}
                }
            ]
        });
        let (text, html, atts) = walk_payload(&payload);
        assert_eq!(text, "plain version");
        assert_eq!(html, "<p>html version</p>");
        assert!(atts.is_empty());
    }

    #[test]
    fn walk_payload_extracts_attachment_metadata_without_body() {
        let payload = json!({
            "mimeType": "multipart/mixed",
            "parts": [
                {
                    "mimeType": "text/plain",
                    "body": {"data": b64url("see attached")}
                },
                {
                    "mimeType": "application/pdf",
                    "filename": "report.pdf",
                    "body": {
                        "size": 12345,
                        "attachmentId": "ATTACH-ID-X"
                    }
                }
            ]
        });
        let (text, html, atts) = walk_payload(&payload);
        assert_eq!(text, "see attached");
        assert!(html.is_empty());
        assert_eq!(atts.len(), 1);
        assert_eq!(atts[0]["filename"], "report.pdf");
        assert_eq!(atts[0]["mime_type"], "application/pdf");
        assert_eq!(atts[0]["size_bytes"], 12345);
        assert_eq!(atts[0]["attachment_id"], "ATTACH-ID-X");
    }

    #[test]
    fn walk_payload_handles_nested_multipart() {
        let payload = json!({
            "mimeType": "multipart/mixed",
            "parts": [
                {
                    "mimeType": "multipart/alternative",
                    "parts": [
                        {
                            "mimeType": "text/plain",
                            "body": {"data": b64url("nested plain")}
                        },
                        {
                            "mimeType": "text/html",
                            "body": {"data": b64url("<i>nested html</i>")}
                        }
                    ]
                },
                {
                    "mimeType": "image/png",
                    "filename": "inline.png",
                    "body": {"size": 999, "attachmentId": "ATT2"}
                }
            ]
        });
        let (text, html, atts) = walk_payload(&payload);
        assert_eq!(text, "nested plain");
        assert_eq!(html, "<i>nested html</i>");
        assert_eq!(atts.len(), 1);
    }

    #[test]
    fn walk_payload_concatenates_repeated_text_parts() {
        let payload = json!({
            "mimeType": "multipart/mixed",
            "parts": [
                {
                    "mimeType": "text/plain",
                    "body": {"data": b64url("first")}
                },
                {
                    "mimeType": "text/plain",
                    "body": {"data": b64url("second")}
                }
            ]
        });
        let (text, _, _) = walk_payload(&payload);
        assert_eq!(text, "first\nsecond");
    }

    #[test]
    fn walk_payload_handles_empty_body_data_gracefully() {
        let payload = json!({
            "mimeType": "text/plain",
            "body": {"size": 0}
        });
        let (text, _, _) = walk_payload(&payload);
        assert!(text.is_empty());
    }

    #[test]
    fn decode_base64url_lossy_accepts_unpadded() {
        let encoded = b64url("hi");
        assert_eq!(decode_base64url_lossy(&encoded), "hi");
    }

    #[test]
    fn decode_base64url_lossy_strips_padding() {
        // "ab" → "YWI=" (padded standard b64) → strip = → decode.
        let padded = "YWI=";
        assert_eq!(decode_base64url_lossy(padded), "ab");
    }

    #[test]
    fn decode_base64url_lossy_returns_placeholder_on_garbage() {
        let result = decode_base64url_lossy("@@@not-base64@@@");
        assert!(result.contains("undecodable"), "{result}");
    }

    #[test]
    fn shape_message_canonicalizes_full_response() {
        let raw = json!({
            "id": "msg-id-x",
            "threadId": "thread-id-x",
            "labelIds": ["INBOX", "UNREAD"],
            "snippet": "preview snippet",
            "internalDate": "1703772345678",
            "payload": {
                "mimeType": "text/plain",
                "headers": [
                    {"name": "From", "value": "alice@x.com"},
                    {"name": "Subject", "value": "hello"},
                ],
                "body": {"data": b64url("hi bob")}
            }
        });
        let shaped = shape_message(&raw);
        assert_eq!(shaped["id"], "msg-id-x");
        assert_eq!(shaped["thread_id"], "thread-id-x");
        assert_eq!(shaped["label_ids"], json!(["INBOX", "UNREAD"]));
        assert_eq!(shaped["snippet"], "preview snippet");
        assert_eq!(shaped["internal_date_ms"], 1703772345678i64);
        assert_eq!(shaped["headers"]["from"], "alice@x.com");
        assert_eq!(shaped["headers"]["subject"], "hello");
        assert_eq!(shaped["body_text"], "hi bob");
        assert_eq!(shaped["body_html"], "");
        assert_eq!(shaped["attachments"], json!([]));
    }

    #[test]
    fn shape_message_handles_missing_payload_gracefully() {
        // Minimal response (e.g. format=minimal would return this shape).
        let raw = json!({"id": "x", "threadId": "y"});
        let shaped = shape_message(&raw);
        assert_eq!(shaped["id"], "x");
        assert_eq!(shaped["headers"], json!({}));
        assert_eq!(shaped["body_text"], "");
        assert_eq!(shaped["attachments"], json!([]));
    }

    #[test]
    fn shape_message_internal_date_parses_string_as_int() {
        // Gmail returns internalDate as a STRING (not a number).
        let raw = json!({
            "id": "x",
            "internalDate": "1700000000000"
        });
        let shaped = shape_message(&raw);
        assert_eq!(shaped["internal_date_ms"], 1700000000000i64);
    }
}

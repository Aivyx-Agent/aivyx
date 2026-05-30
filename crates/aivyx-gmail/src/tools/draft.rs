//! `gmail.draft` — create a Gmail draft (no send).
//!
//! Phase 123 Task 6 — the safe write surface. Creates a
//! draft in the operator's Gmail account; the draft does not
//! leave Gmail until the operator clicks Send in the Gmail
//! UI. The trust step is intentionally explicit: even with
//! `email.write` granted, the agent cannot dispatch an email
//! through this tool.
//!
//! ## API call
//!
//! POST `users/me/drafts` with `{message: {raw, threadId?}}`.
//! `raw` is the base64url-encoded RFC 5322 MIME built by
//! [`crate::mime`]; `threadId` is the optional Gmail thread
//! ID for placing the draft inside an existing conversation.
//!
//! ## Capability scope
//!
//! `email.write`. Trusted-tier-only by default (see
//! `aivyx-capability::CEILING_TRUSTED`). Operators who want a
//! SemiTrusted role to draft can grant `email.write`
//! explicitly via the role's `capability_scopes`.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

use crate::gmail_client::SharedGmailClient;
use crate::mime::{build_for_api, MimeMessageParams};

pub struct GmailDraft {
    id: ToolId,
    schema: Value,
    client: SharedGmailClient,
}

impl GmailDraft {
    pub fn new(client: SharedGmailClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for GmailDraft {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "gmail.draft"
    }

    fn description(&self) -> &str {
        "Create a Gmail draft. The draft lives in the \
         operator's Drafts folder; it does NOT leave Gmail \
         until the operator clicks Send in the Gmail UI. \
         Input is a JSON object with required `to`, `subject`, \
         and `body_text` fields, an optional \
         `in_reply_to_message_id` (RFC 5322 Message-ID of the \
         message being replied to — populates `In-Reply-To` + \
         `References` headers), and an optional `thread_id` \
         (Gmail thread ID for placing the draft inside an \
         existing conversation; typically obtained from \
         `gmail.search` or `gmail.read`). Returns \
         `{draft_id, message_id, thread_id}`. CR/LF in header \
         fields is rejected (header-injection defense)."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("email.write").expect(
            "email.write must parse — it is in KNOWN_BASES from Phase 123",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("gmail.draft: {reason}"),
                });
            }
        };

        let raw_encoded = match build_for_api(&parsed.mime) {
            Ok(s) => s,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("gmail.draft: MIME build failed: {e}"),
                });
            }
        };

        let mut message_obj = json!({"raw": raw_encoded});
        if let Some(thread_id) = &parsed.thread_id {
            message_obj["threadId"] = Value::String(thread_id.clone());
        }
        let body = json!({"message": message_obj});

        let response = match self.client.post_json("/users/me/drafts", &body).await {
            Ok(r) => r,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("gmail.draft: API call failed: {e}"),
                });
            }
        };
        let decoded = match self.client.decode_json(response).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("gmail.draft: response decode failed: {e}"),
                });
            }
        };

        let draft_id = decoded
            .get("id")
            .cloned()
            .unwrap_or(Value::Null);
        let message = decoded.get("message");
        let message_id = message
            .and_then(|m| m.get("id"))
            .cloned()
            .unwrap_or(Value::Null);
        let thread_id = message
            .and_then(|m| m.get("threadId"))
            .cloned()
            .unwrap_or(Value::Null);

        ToolOutcome::Completed {
            output: json!({
                "draft_id": draft_id,
                "message_id": message_id,
                "thread_id": thread_id,
            }),
            // The Gmail API confirmed the draft by returning
            // an ID; we didn't re-fetch to double-check the
            // content. `Unverified` is honest here per the SDK
            // contract: the server acknowledged but we didn't
            // verify.
            verified: Verification::Unverified,
        }
    }
}

#[derive(Debug)]
struct ParsedInput {
    mime: MimeMessageParams,
    thread_id: Option<String>,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let to = required_string(input, "to")?;
    let subject = required_string(input, "subject")?;
    let body_text = required_string(input, "body_text")?;
    let in_reply_to_message_id = optional_string(input, "in_reply_to_message_id")?;
    let thread_id = optional_string(input, "thread_id")?;
    Ok(ParsedInput {
        mime: MimeMessageParams {
            to,
            subject,
            body_text,
            in_reply_to_message_id,
            // `gmail.draft` does NOT accept `from` — Gmail
            // always uses the authenticated user for drafts.
            // `gmail.send` (Task 7) is where `from` lives.
            from: None,
        },
        thread_id,
    })
}

fn required_string(input: &Value, field: &str) -> Result<String, String> {
    let s = input
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("input must include a `{field}` string field"))?;
    if s.trim().is_empty() {
        return Err(format!("`{field}` must not be empty"));
    }
    Ok(s.to_string())
}

fn optional_string(input: &Value, field: &str) -> Result<Option<String>, String> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            if s.trim().is_empty() {
                Ok(None)
            } else {
                Ok(Some(s.clone()))
            }
        }
        Some(_) => Err(format!("`{field}` must be a string if present")),
    }
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "to": {
                "type": "string",
                "minLength": 1,
                "description": "Recipient address. May include a display name (`Alice <alice@example.com>`). Must contain `@`."
            },
            "subject": {
                "type": "string",
                "minLength": 1,
                "description": "Subject line. Non-ASCII subjects are RFC 2047 encoded automatically."
            },
            "body_text": {
                "type": "string",
                "minLength": 1,
                "description": "Plain-text body. Encoded as UTF-8 with Content-Transfer-Encoding base64."
            },
            "in_reply_to_message_id": {
                "type": "string",
                "description": "RFC 5322 Message-ID of the message being replied to. Populates `In-Reply-To` + `References` headers. Bare IDs are wrapped in `<>` automatically."
            },
            "thread_id": {
                "type": "string",
                "description": "Gmail thread ID for placing the draft inside an existing conversation. Obtain from `gmail.search.messages[].thread_id` or `gmail.read.thread_id`."
            }
        },
        "required": ["to", "subject", "body_text"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_input_accepts_minimal_payload() {
        let input = json!({
            "to": "bob@example.com",
            "subject": "hi",
            "body_text": "body"
        });
        let p = parse_input(&input).expect("parse");
        assert_eq!(p.mime.to, "bob@example.com");
        assert_eq!(p.mime.subject, "hi");
        assert_eq!(p.mime.body_text, "body");
        assert!(p.mime.in_reply_to_message_id.is_none());
        assert!(p.thread_id.is_none());
        assert!(p.mime.from.is_none(), "gmail.draft must NOT set from");
    }

    #[test]
    fn parse_input_accepts_threading_fields() {
        let input = json!({
            "to": "bob@example.com",
            "subject": "Re: hi",
            "body_text": "reply",
            "in_reply_to_message_id": "<orig-id@host>",
            "thread_id": "thread-123"
        });
        let p = parse_input(&input).expect("parse");
        assert_eq!(
            p.mime.in_reply_to_message_id.as_deref(),
            Some("<orig-id@host>")
        );
        assert_eq!(p.thread_id.as_deref(), Some("thread-123"));
    }

    #[test]
    fn parse_input_rejects_missing_to() {
        let input = json!({"subject": "x", "body_text": "y"});
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("`to`"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_subject() {
        let input = json!({"to": "bob@x.com", "subject": "  ", "body_text": "y"});
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("`subject`"), "{e}");
        assert!(e.contains("empty"), "{e}");
    }

    #[test]
    fn parse_input_treats_empty_optional_as_absent() {
        // An operator-supplied empty string for `thread_id`
        // should be treated as absent, not as a literal empty
        // thread ID (which would break Gmail's API call).
        let input = json!({
            "to": "bob@x.com",
            "subject": "x",
            "body_text": "y",
            "thread_id": ""
        });
        let p = parse_input(&input).expect("parse");
        assert!(p.thread_id.is_none());
    }

    #[test]
    fn parse_input_rejects_non_string_optional() {
        let input = json!({
            "to": "bob@x.com",
            "subject": "x",
            "body_text": "y",
            "thread_id": 12345
        });
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("`thread_id`"), "{e}");
        assert!(e.contains("string"), "{e}");
    }

    #[test]
    fn input_schema_declares_required_fields_and_no_extras() {
        let schema = input_schema();
        let req: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(req.contains(&"to"));
        assert!(req.contains(&"subject"));
        assert!(req.contains(&"body_text"));
        // Threading fields are NOT required.
        assert!(!req.contains(&"in_reply_to_message_id"));
        assert!(!req.contains(&"thread_id"));
        // additionalProperties: false so unknown fields are
        // rejected at the planner gate.
        assert_eq!(schema["additionalProperties"], false);
        // `from` is NOT in the schema — drafts use the
        // authenticated user; `gmail.send` is where `from`
        // lives.
        assert!(schema["properties"].get("from").is_none());
    }

    #[test]
    fn required_scope_is_email_write() {
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
        let tool = GmailDraft::new(client);
        let scope = tool.required_scope(&json!({}));
        assert_eq!(scope.to_string(), "email.write");
    }
}

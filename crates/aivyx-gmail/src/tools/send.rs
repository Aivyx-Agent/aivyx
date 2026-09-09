//! `gmail.send` — send a Gmail message directly.
//!
//! Phase 123 Task 7 — full surface (Q2c non-Recommended at
//! sign-off). Bypasses the Drafts folder; the message goes
//! out immediately. **No undo from Aivyx** — Gmail's UI
//! offers a 5-30s undo window after send, but that's on the
//! operator.
//!
//! ## Trust gating
//!
//! Capability scope `email.send`. Per
//! `aivyx-capability::CEILING_TRUSTED`, this scope is only
//! grantable on Trusted-tier channels (Local) by default,
//! mirroring `shell.exec` / `notify.send` (Phase 62 Q2(a)).
//! A SemiTrusted Telegram/Discord/Slack adapter would have
//! `email.send` filtered out of the effective capability
//! envelope at the ceiling intersection step, before any
//! tool dispatch.
//!
//! Operators who want a SemiTrusted role to send can grant
//! `email.send` explicitly via the role's `capability_scopes`
//! — but the contract makes that a deliberate, role-config
//! decision, not a default.
//!
//! ## API call
//!
//! POST `users/me/messages/send` with `{raw, threadId?}`.
//! Unlike `users.drafts.create`, the request body is a flat
//! Message resource (no `{message: {...}}` wrapper). Response
//! is the full Message including `labelIds` (which Gmail
//! sets to include `SENT`).

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

use crate::gmail_client::SharedGmailClient;
use crate::mime::{build_for_api, MimeMessageParams};

pub struct GmailSend {
    id: ToolId,
    schema: Value,
    client: SharedGmailClient,
}

impl GmailSend {
    pub fn new(client: SharedGmailClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for GmailSend {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "gmail.send"
    }

    fn description(&self) -> &str {
        "Send a Gmail message directly. **The message goes out \
         immediately — Aivyx PA has no undo.** Input is a JSON \
         object with required `to`, `subject`, `body_text` \
         fields, an optional `from` (send-as alias the \
         operator's Gmail account has authorized; defaults to \
         the primary address), an optional \
         `in_reply_to_message_id` for `In-Reply-To` + \
         `References` header threading, and an optional \
         `thread_id` for Gmail thread placement. Returns \
         `{message_id, thread_id, label_ids}`. Capability \
         scope: `email.send` — Trusted-tier-only by default \
         (mirrors shell.exec gating); requires explicit \
         role grant on remote-channel roles."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("email.send").expect(
            "email.send must parse — it is in KNOWN_BASES from Phase 123",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("gmail.send: {reason}"),
                });
            }
        };

        let raw_encoded = match build_for_api(&parsed.mime) {
            Ok(s) => s,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("gmail.send: MIME build failed: {e}"),
                });
            }
        };

        let mut body = json!({"raw": raw_encoded});
        if let Some(thread_id) = &parsed.thread_id {
            body["threadId"] = Value::String(thread_id.clone());
        }

        let response = match self
            .client
            .post_json("/users/me/messages/send", &body)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("gmail.send: API call failed: {e}"),
                });
            }
        };
        let decoded = match self.client.decode_json(response).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("gmail.send: response decode failed: {e}"),
                });
            }
        };

        let message_id = decoded.get("id").cloned().unwrap_or(Value::Null);
        let thread_id = decoded.get("threadId").cloned().unwrap_or(Value::Null);
        let label_ids = decoded
            .get("labelIds")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));

        ToolOutcome::Completed {
            output: json!({
                "message_id": message_id,
                "thread_id": thread_id,
                "label_ids": label_ids,
            }),
            // The API responded with a message ID; we did not
            // re-fetch to double-check the message landed in
            // the recipient's mailbox. `Unverified` per the
            // SDK contract — consistent with `gmail.draft`.
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
    // `from` IS honored here (unlike gmail.draft) — send-as
    // aliases the operator's Gmail account has authorized.
    let from = optional_string(input, "from")?;
    Ok(ParsedInput {
        mime: MimeMessageParams {
            to,
            subject,
            body_text,
            in_reply_to_message_id,
            from,
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
                "description": "Recipient address. Must contain `@`."
            },
            "subject": {
                "type": "string",
                "minLength": 1,
                "description": "Subject line. Non-ASCII subjects are RFC 2047 encoded automatically."
            },
            "body_text": {
                "type": "string",
                "minLength": 1,
                "description": "Plain-text body. UTF-8; encoded as Content-Transfer-Encoding base64."
            },
            "from": {
                "type": "string",
                "description": "Send-as alias the operator's Gmail account has authorized. If omitted, Google uses the primary address."
            },
            "in_reply_to_message_id": {
                "type": "string",
                "description": "RFC 5322 Message-ID being replied to. Populates `In-Reply-To` + `References` headers."
            },
            "thread_id": {
                "type": "string",
                "description": "Gmail thread ID for placing the sent message inside an existing conversation."
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
        assert!(p.mime.from.is_none());
        assert!(p.thread_id.is_none());
    }

    #[test]
    fn parse_input_honors_from_for_send_as_alias() {
        let input = json!({
            "to": "bob@example.com",
            "subject": "hi",
            "body_text": "body",
            "from": "alias@example.com"
        });
        let p = parse_input(&input).expect("parse");
        assert_eq!(p.mime.from.as_deref(), Some("alias@example.com"));
    }

    #[test]
    fn parse_input_accepts_threading_fields() {
        let input = json!({
            "to": "bob@example.com",
            "subject": "Re: hi",
            "body_text": "reply body",
            "in_reply_to_message_id": "<orig@host>",
            "thread_id": "thread-xyz"
        });
        let p = parse_input(&input).expect("parse");
        assert_eq!(
            p.mime.in_reply_to_message_id.as_deref(),
            Some("<orig@host>")
        );
        assert_eq!(p.thread_id.as_deref(), Some("thread-xyz"));
    }

    #[test]
    fn parse_input_rejects_missing_to() {
        let input = json!({"subject": "x", "body_text": "y"});
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("`to`"), "{e}");
    }

    #[test]
    fn parse_input_rejects_missing_body() {
        let input = json!({"to": "bob@x.com", "subject": "x"});
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("`body_text`"), "{e}");
    }

    #[test]
    fn parse_input_treats_empty_from_as_absent() {
        // Operator-supplied empty `from` should fall back to
        // the authenticated user's primary address, not send
        // with a literal empty From header (which would be
        // rejected by the mime validator anyway).
        let input = json!({
            "to": "bob@x.com",
            "subject": "x",
            "body_text": "y",
            "from": ""
        });
        let p = parse_input(&input).expect("parse");
        assert!(p.mime.from.is_none());
    }

    #[test]
    fn parse_input_rejects_non_string_optional() {
        let input = json!({
            "to": "bob@x.com",
            "subject": "x",
            "body_text": "y",
            "from": 42
        });
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("`from`"), "{e}");
        assert!(e.contains("string"), "{e}");
    }

    #[test]
    fn input_schema_includes_from_unlike_draft_schema() {
        let schema = input_schema();
        assert!(schema["properties"].get("from").is_some());
        // Required fields are the same as draft.
        let req: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(req.contains(&"to"));
        assert!(req.contains(&"subject"));
        assert!(req.contains(&"body_text"));
        // `from` is OPTIONAL, not required.
        assert!(!req.contains(&"from"));
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn required_scope_is_email_send() {
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
        let tool = GmailSend::new(client);
        let scope = tool.required_scope(&json!({}));
        assert_eq!(scope.to_string(), "email.send");
    }

    #[test]
    fn description_warns_about_no_undo() {
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
        let tool = GmailSend::new(client);
        let desc = tool.description();
        // The tool description goes to the LLM; the no-undo
        // warning is load-bearing for the model's decision
        // to call this vs gmail.draft.
        assert!(
            desc.contains("no undo") || desc.contains("immediately"),
            "description should warn about send irreversibility; got: {desc}"
        );
    }
}

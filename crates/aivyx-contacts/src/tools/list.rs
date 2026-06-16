//! `contacts.list` — paginated list of the user's contacts.
//!
//! CT.3. Single GET to `people/me/connections`. Returns one
//! page plus a `next_page_token` (CT contract F-1: one page +
//! token, like Drive's `list_folder` — cheaper, and the model
//! can ask for more).
//!
//! ## API call
//!
//! `GET /people/me/connections?personFields=<fields>&pageSize=<n>&pageToken=<tok>`
//! Returns `{ "connections": [Person…], "nextPageToken": "…" }`.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::contacts_client::SharedContactsClient;
use crate::tools::person::{trim_person, PERSON_FIELDS};

const MAX_RESULTS_CAP: u64 = 100;
const DEFAULT_MAX_RESULTS: u64 = 50;

pub struct ContactsList {
    id: ToolId,
    schema: Value,
    client: SharedContactsClient,
}

impl ContactsList {
    pub fn new(client: SharedContactsClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for ContactsList {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "contacts.list"
    }

    fn description(&self) -> &str {
        "List the user's Google contacts, one page at a time. \
         Input is a JSON object with an optional `max_results` \
         (default 50, capped at 100) and an optional `page_token` \
         (from a prior call's `next_page_token`). Returns a JSON \
         object with `contacts` (array of `{resource_name, etag, \
         display_name, emails, phones, organizations}`), \
         `next_page_token` (null when no more pages), and \
         `count`. For a targeted lookup prefer `contacts.search`."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("contacts.read")
            .expect("contacts.read must parse — it is in KNOWN_BASES from Chapter Contacts")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("contacts.list: {reason}"),
                });
            }
        };

        let mut query: Vec<(&str, String)> = vec![
            ("personFields", PERSON_FIELDS.to_string()),
            ("pageSize", parsed.max_results.to_string()),
        ];
        if let Some(token) = parsed.page_token {
            query.push(("pageToken", token));
        }

        let body: Value = match self.client.get_json("/people/me/connections", &query).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("contacts.list: API call failed: {e}"),
                });
            }
        };

        let contacts: Vec<Value> = body
            .get("connections")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().map(trim_person).collect())
            .unwrap_or_default();
        let next_page_token = body
            .get("nextPageToken")
            .and_then(Value::as_str)
            .map(String::from);

        let output = json!({
            "contacts": contacts,
            "next_page_token": next_page_token,
            "count": contacts.len(),
        });

        ToolOutcome::Completed {
            output,
            verified: Verification::NotApplicable,
        }
    }
}

#[derive(Debug)]
struct ParsedInput {
    max_results: u64,
    page_token: Option<String>,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let max_results = match obj.get("max_results") {
        None => DEFAULT_MAX_RESULTS,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| "`max_results` must be a non-negative integer".to_string())?,
    };
    if max_results == 0 {
        return Err("`max_results` must be >= 1".to_string());
    }
    let page_token = match obj.get("page_token") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::String(_)) => None,
        Some(_) => return Err("`page_token` must be a string".to_string()),
    };
    Ok(ParsedInput {
        max_results: max_results.min(MAX_RESULTS_CAP),
        page_token,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_RESULTS_CAP,
                "default": DEFAULT_MAX_RESULTS,
                "description": "Maximum contacts per page. Capped at 100."
            },
            "page_token": {
                "type": ["string", "null"],
                "description": "Continuation token from a prior call's `next_page_token`."
            }
        },
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_input_defaults_when_empty() {
        let p = parse_input(&json!({})).expect("parse");
        assert_eq!(p.max_results, DEFAULT_MAX_RESULTS);
        assert!(p.page_token.is_none());
    }

    #[test]
    fn parse_input_caps_max_results() {
        let p = parse_input(&json!({"max_results": 9999})).expect("parse");
        assert_eq!(p.max_results, MAX_RESULTS_CAP);
    }

    #[test]
    fn parse_input_rejects_zero_max_results() {
        let e = parse_input(&json!({"max_results": 0})).expect_err("err");
        assert!(e.contains(">= 1"), "{e}");
    }

    #[test]
    fn parse_input_keeps_page_token() {
        let p = parse_input(&json!({"page_token": "tok123"})).expect("parse");
        assert_eq!(p.page_token.as_deref(), Some("tok123"));
    }

    #[test]
    fn parse_input_empty_page_token_is_none() {
        let p = parse_input(&json!({"page_token": ""})).expect("parse");
        assert!(p.page_token.is_none());
    }

    #[test]
    fn input_schema_has_no_required_fields() {
        let s = input_schema();
        assert!(s.get("required").is_none());
        assert_eq!(s["additionalProperties"], false);
    }
}

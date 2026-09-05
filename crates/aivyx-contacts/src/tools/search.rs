//! `contacts.search` — fuzzy lookup over the user's contacts.
//!
//! CT.3. The name-resolution primitive: turn "Dana" into a
//! resolved person with a `resource_name` the other tools
//! consume. Single GET to the People API `people:searchContacts`
//! endpoint with the operator's query and the shared field mask.
//!
//! ## API call
//!
//! `GET /people:searchContacts?query=<q>&readMask=<fields>&pageSize=<n>`
//! Returns `{ "results": [ { "person": {…} }, … ] }`; we unwrap
//! each `person` and trim it via [`super::person::trim_person`].

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::contacts_client::SharedContactsClient;
use crate::tools::person::{trim_person, PERSON_FIELDS};

const MAX_RESULTS_CAP: u64 = 30;
const DEFAULT_MAX_RESULTS: u64 = 10;

pub struct ContactsSearch {
    id: ToolId,
    schema: Value,
    client: SharedContactsClient,
}

impl ContactsSearch {
    pub fn new(client: SharedContactsClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for ContactsSearch {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "contacts.search"
    }

    // Chapter Picket follow-up (Finding 3) — same rationale as
    // contacts.get: externally authored contact fields.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Search the user's Google contacts by name, email, phone, \
         or organization. Input is a JSON object with a required \
         `query` field (the search text) and an optional \
         `max_results` (default 10, capped at 30). Returns a JSON \
         object with `results` (array of `{resource_name, etag, \
         display_name, emails, phones, organizations}`) and \
         `result_count`. Use the returned `resource_name` with \
         `contacts.get` / `contacts.update` / `contacts.delete`."
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
                    detail: format!("contacts.search: {reason}"),
                });
            }
        };

        let query: Vec<(&str, String)> = vec![
            ("query", parsed.query),
            ("readMask", PERSON_FIELDS.to_string()),
            ("pageSize", parsed.max_results.to_string()),
        ];

        let body: Value = match self.client.get_json("/people:searchContacts", &query).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("contacts.search: API call failed: {e}"),
                });
            }
        };

        let results = extract_results(&body);
        let output = json!({
            "results": results,
            "result_count": results.len(),
        });

        ToolOutcome::Completed {
            output,
            verified: Verification::NotApplicable,
        }
    }
}

/// Unwrap the `results[].person` envelope `searchContacts`
/// returns and trim each person. Tolerates a missing/empty
/// `results` array (no matches) by returning an empty vec.
pub(crate) fn extract_results(body: &Value) -> Vec<Value> {
    body.get("results")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|r| r.get("person"))
                .map(trim_person)
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug)]
struct ParsedInput {
    query: String,
    max_results: u64,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let query = match obj.get("query") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        Some(Value::String(_)) | None => {
            return Err("`query` is required and must be a non-empty string".to_string())
        }
        Some(_) => return Err("`query` must be a string".to_string()),
    };
    let max_results = match obj.get("max_results") {
        None => DEFAULT_MAX_RESULTS,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| "`max_results` must be a non-negative integer".to_string())?,
    };
    if max_results == 0 {
        return Err("`max_results` must be >= 1".to_string());
    }
    Ok(ParsedInput {
        query,
        max_results: max_results.min(MAX_RESULTS_CAP),
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Search text — matches against contact name, email, phone, or organization."
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_RESULTS_CAP,
                "default": DEFAULT_MAX_RESULTS,
                "description": "Maximum contacts to return. Capped at 30."
            }
        },
        "required": ["query"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool() -> ContactsSearch {
        use crate::{ContactsClient, OAuthConfig, TokenSet};
        use std::sync::Arc;
        let client = Arc::new(ContactsClient::new(
            reqwest::Client::new(),
            OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            std::path::PathBuf::from("/tmp/unused"),
        ));
        ContactsSearch::new(client)
    }

    #[test]
    fn contacts_search_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }

    #[test]
    fn parse_input_requires_query() {
        let e = parse_input(&json!({})).expect_err("must error");
        assert!(e.contains("`query`"), "{e}");
    }

    #[test]
    fn parse_input_rejects_blank_query() {
        let e = parse_input(&json!({"query": "  "})).expect_err("must error");
        assert!(e.contains("`query`"), "{e}");
    }

    #[test]
    fn parse_input_trims_query_and_defaults_max() {
        let p = parse_input(&json!({"query": "  Dana "})).expect("parse");
        assert_eq!(p.query, "Dana");
        assert_eq!(p.max_results, DEFAULT_MAX_RESULTS);
    }

    #[test]
    fn parse_input_caps_max_results() {
        let p = parse_input(&json!({"query": "x", "max_results": 999})).expect("parse");
        assert_eq!(p.max_results, MAX_RESULTS_CAP);
    }

    #[test]
    fn parse_input_rejects_zero_max_results() {
        let e = parse_input(&json!({"query": "x", "max_results": 0})).expect_err("err");
        assert!(e.contains(">= 1"), "{e}");
    }

    #[test]
    fn extract_results_unwraps_person_envelope() {
        let body = json!({
            "results": [
                {"person": {"resourceName": "people/c1", "names": [{"displayName": "Dana"}]}},
                {"person": {"resourceName": "people/c2"}}
            ]
        });
        let r = extract_results(&body);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0]["resource_name"], "people/c1");
        assert_eq!(r[0]["display_name"], "Dana");
    }

    #[test]
    fn extract_results_empty_when_no_matches() {
        assert!(extract_results(&json!({})).is_empty());
        assert!(extract_results(&json!({"results": []})).is_empty());
    }

    #[test]
    fn input_schema_requires_query_only() {
        let s = input_schema();
        assert_eq!(s["required"], json!(["query"]));
        assert_eq!(s["additionalProperties"], false);
    }
}

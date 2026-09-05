//! `contacts.get` — full detail for one resolved contact.
//!
//! CT.3. Single GET to `people/{resourceName}`. The
//! `resource_name` comes from a prior `contacts.search` /
//! `contacts.list` result.
//!
//! ## API call
//!
//! `GET /people/{resourceName}?personFields=<fields>` — returns
//! a `Person` directly (no envelope), trimmed via
//! [`super::person::trim_person`].

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::contacts_client::SharedContactsClient;
use crate::tools::person::{trim_person, PERSON_FIELDS};
use crate::tools::resource_name::parse_resource_name;

pub struct ContactsGet {
    id: ToolId,
    schema: Value,
    client: SharedContactsClient,
}

impl ContactsGet {
    pub fn new(client: SharedContactsClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for ContactsGet {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "contacts.get"
    }

    // Chapter Picket follow-up (Finding 3) — contact fields (name,
    // organization, emails, phones) are externally authored and may
    // carry a prompt-injection payload.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Fetch full detail for one Google contact. Input is a \
         JSON object with a required `resource_name` field (e.g. \
         `people/c123`, from a `contacts.search` / `contacts.list` \
         result). Returns the trimmed contact `{resource_name, \
         etag, display_name, emails, phones, organizations}`. The \
         `etag` is required by `contacts.update`."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("contacts.read")
            .expect("contacts.read must parse — it is in KNOWN_BASES from Chapter Contacts")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let resource_name = match parse_resource_name(&input) {
            Ok(r) => r,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("contacts.get: {reason}"),
                });
            }
        };

        let path = format!("/{resource_name}");
        let query: Vec<(&str, String)> = vec![("personFields", PERSON_FIELDS.to_string())];

        let body: Value = match self.client.get_json(&path, &query).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("contacts.get: API call failed: {e}"),
                });
            }
        };

        ToolOutcome::Completed {
            output: trim_person(&body),
            verified: Verification::NotApplicable,
        }
    }
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "resource_name": {
                "type": "string",
                "description": "People API resource name, e.g. `people/c123`."
            }
        },
        "required": ["resource_name"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool() -> ContactsGet {
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
        ContactsGet::new(client)
    }

    #[test]
    fn contacts_get_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }

    #[test]
    fn parse_resource_name_accepts_people_id() {
        let r = parse_resource_name(&json!({"resource_name": "people/c123"})).expect("ok");
        assert_eq!(r, "people/c123");
    }

    #[test]
    fn parse_resource_name_trims() {
        let r = parse_resource_name(&json!({"resource_name": "  people/c1 "})).expect("ok");
        assert_eq!(r, "people/c1");
    }

    #[test]
    fn parse_resource_name_requires_field() {
        let e = parse_resource_name(&json!({})).expect_err("err");
        assert!(e.contains("`resource_name`"), "{e}");
    }

    #[test]
    fn parse_resource_name_rejects_non_people_prefix() {
        let e = parse_resource_name(&json!({"resource_name": "contactGroups/x"})).expect_err("err");
        assert!(e.contains("people/"), "{e}");
    }

    #[test]
    fn parse_resource_name_rejects_traversal() {
        let e = parse_resource_name(&json!({"resource_name": "people/../foo"})).expect_err("err");
        assert!(e.contains("people/"), "{e}");
    }

    #[test]
    fn input_schema_requires_resource_name() {
        let s = input_schema();
        assert_eq!(s["required"], json!(["resource_name"]));
        assert_eq!(s["additionalProperties"], false);
    }
}

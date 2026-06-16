//! `contacts.create` — add a person to the user's contacts.
//!
//! CT.4. `contacts.write` (Trusted-tier only by default). Single
//! POST to `people:createContact` with a Person body built from
//! the friendly flat input via
//! [`super::person::build_contact_fields`].
//!
//! ## API call
//!
//! `POST /people:createContact?personFields=<fields>` with the
//! Person JSON body. Returns the created Person, trimmed.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::contacts_client::SharedContactsClient;
use crate::tools::person::{build_contact_fields, trim_person, PERSON_FIELDS};

pub struct ContactsCreate {
    id: ToolId,
    schema: Value,
    client: SharedContactsClient,
}

impl ContactsCreate {
    pub fn new(client: SharedContactsClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for ContactsCreate {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "contacts.create"
    }

    fn description(&self) -> &str {
        "Add a new person to the user's Google contacts. Input is \
         a JSON object with at least one of: `given_name`, \
         `family_name`, `emails` (array of strings), `phones` \
         (array of strings), `organization`, `organization_title`. \
         Returns the created contact `{resource_name, etag, \
         display_name, emails, phones, organizations}`."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("contacts.write")
            .expect("contacts.write must parse — it is in KNOWN_BASES from Chapter Contacts")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let obj = match input.as_object() {
            Some(o) => o,
            None => {
                return self.fail("input must be a JSON object");
            }
        };
        let (body, _fields) = match build_contact_fields(obj) {
            Ok(v) => v,
            Err(reason) => return self.fail(&reason),
        };

        let query: Vec<(&str, String)> = vec![("personFields", PERSON_FIELDS.to_string())];
        let created: Value = match self
            .client
            .post_json("/people:createContact", &query, &body)
            .await
        {
            Ok(v) => v,
            Err(e) => return self.fail(&format!("API call failed: {e}")),
        };

        ToolOutcome::Completed {
            output: trim_person(&created),
            verified: Verification::NotApplicable,
        }
    }
}

impl ContactsCreate {
    fn fail(&self, detail: &str) -> ToolOutcome {
        ToolOutcome::Failed(AivyxError::Tool {
            tool: self.id,
            detail: format!("contacts.create: {detail}"),
        })
    }
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "given_name": {"type": "string", "description": "First name."},
            "family_name": {"type": "string", "description": "Last name."},
            "emails": {"type": "array", "items": {"type": "string"}, "description": "Email addresses."},
            "phones": {"type": "array", "items": {"type": "string"}, "description": "Phone numbers."},
            "organization": {"type": "string", "description": "Company / organization name."},
            "organization_title": {"type": "string", "description": "Job title at the organization."}
        },
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_tool() -> ContactsCreate {
        use crate::{ContactsClient, OAuthConfig, TokenSet};
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
        ContactsCreate::new(client)
    }

    #[test]
    fn required_scope_is_contacts_write() {
        assert_eq!(
            make_tool().required_scope(&json!({})).to_string(),
            "contacts.write"
        );
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "contacts.create");
    }
}

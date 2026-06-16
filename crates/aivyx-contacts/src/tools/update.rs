//! `contacts.update` — modify an existing contact.
//!
//! CT.4. `contacts.write` (Trusted-tier only by default). Single
//! PATCH to `people/{resourceName}:updateContact`. Per the CT
//! contract F-2 lean, the caller supplies only the fields to
//! change and the `updatePersonFields` mask is derived from the
//! keys present. The People API requires the current `etag` for
//! optimistic concurrency — every read tool surfaces it.
//!
//! ## API call
//!
//! `PATCH /people/{resourceName}:updateContact?updatePersonFields=<mask>&personFields=<fields>`
//! with body = the partial Person plus `etag`. Returns the
//! updated Person, trimmed.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::contacts_client::SharedContactsClient;
use crate::tools::person::{build_contact_fields, trim_person, PERSON_FIELDS};
use crate::tools::resource_name::validate_resource_name;

pub struct ContactsUpdate {
    id: ToolId,
    schema: Value,
    client: SharedContactsClient,
}

impl ContactsUpdate {
    pub fn new(client: SharedContactsClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }

    fn fail(&self, detail: &str) -> ToolOutcome {
        ToolOutcome::Failed(AivyxError::Tool {
            tool: self.id,
            detail: format!("contacts.update: {detail}"),
        })
    }
}

#[async_trait]
impl Tool for ContactsUpdate {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "contacts.update"
    }

    fn description(&self) -> &str {
        "Update an existing Google contact. Input is a JSON object \
         with required `resource_name` (e.g. `people/c123`) and \
         `etag` (from a prior `contacts.get` / `contacts.search` / \
         `contacts.list` — guards against lost updates), plus at \
         least one field to change: `given_name`, `family_name`, \
         `emails` (array — replaces all), `phones` (array — \
         replaces all), `organization`, `organization_title`. \
         Only the supplied fields are modified. Returns the \
         updated contact."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("contacts.write")
            .expect("contacts.write must parse — it is in KNOWN_BASES from Chapter Contacts")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => return self.fail(&reason),
        };

        let mut body = parsed.body;
        if let Value::Object(ref mut map) = body {
            map.insert("etag".into(), Value::String(parsed.etag));
        }

        let mask = parsed.fields.join(",");
        let path = format!("/{}:updateContact", parsed.resource_name);
        let query: Vec<(&str, String)> = vec![
            ("updatePersonFields", mask),
            ("personFields", PERSON_FIELDS.to_string()),
        ];

        let updated: Value = match self.client.patch_json(&path, &query, &body).await {
            Ok(v) => v,
            Err(e) => return self.fail(&format!("API call failed: {e}")),
        };

        ToolOutcome::Completed {
            output: trim_person(&updated),
            verified: Verification::NotApplicable,
        }
    }
}

#[derive(Debug)]
struct ParsedInput {
    resource_name: String,
    etag: String,
    body: Value,
    fields: Vec<&'static str>,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;

    let resource_name = match obj.get("resource_name") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        Some(Value::String(_)) | None => {
            return Err("`resource_name` is required and must be a non-empty string".to_string())
        }
        Some(_) => return Err("`resource_name` must be a string".to_string()),
    };
    validate_resource_name(&resource_name)?;

    let etag = match obj.get("etag") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        Some(Value::String(_)) | None => {
            return Err(
                "`etag` is required (from a prior read) and must be a non-empty string"
                    .to_string(),
            )
        }
        Some(_) => return Err("`etag` must be a string".to_string()),
    };

    let (body, fields) = build_contact_fields(obj)?;

    Ok(ParsedInput {
        resource_name,
        etag,
        body,
        fields,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "resource_name": {"type": "string", "description": "People API resource name, e.g. `people/c123`."},
            "etag": {"type": "string", "description": "Current etag from a prior read (optimistic-concurrency guard)."},
            "given_name": {"type": "string"},
            "family_name": {"type": "string"},
            "emails": {"type": "array", "items": {"type": "string"}, "description": "Replaces all email addresses."},
            "phones": {"type": "array", "items": {"type": "string"}, "description": "Replaces all phone numbers."},
            "organization": {"type": "string"},
            "organization_title": {"type": "string"}
        },
        "required": ["resource_name", "etag"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_input_requires_resource_name() {
        let e = parse_input(&json!({"etag": "x", "given_name": "A"})).expect_err("err");
        assert!(e.contains("`resource_name`"), "{e}");
    }

    #[test]
    fn parse_input_requires_etag() {
        let e = parse_input(&json!({"resource_name": "people/c1", "given_name": "A"}))
            .expect_err("err");
        assert!(e.contains("`etag`"), "{e}");
    }

    #[test]
    fn parse_input_requires_a_changed_field() {
        let e = parse_input(&json!({"resource_name": "people/c1", "etag": "v1"}))
            .expect_err("err");
        assert!(e.contains("at least one"), "{e}");
    }

    #[test]
    fn parse_input_rejects_bad_resource_name() {
        let e = parse_input(&json!({"resource_name": "groups/x", "etag": "v1", "given_name": "A"}))
            .expect_err("err");
        assert!(e.contains("people/"), "{e}");
    }

    #[test]
    fn parse_input_builds_body_and_mask_from_supplied_keys() {
        let p = parse_input(&json!({
            "resource_name": "people/c1",
            "etag": "v1",
            "emails": ["a@b.com"]
        }))
        .expect("parse");
        assert_eq!(p.fields, vec!["emailAddresses"]);
        assert_eq!(p.body["emailAddresses"][0]["value"], "a@b.com");
        assert_eq!(p.etag, "v1");
    }

    #[test]
    fn input_schema_requires_resource_name_and_etag() {
        let s = input_schema();
        assert_eq!(s["required"], json!(["resource_name", "etag"]));
        assert_eq!(s["additionalProperties"], false);
    }
}

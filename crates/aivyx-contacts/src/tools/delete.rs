//! `contacts.delete` — remove a contact. **Irreversible.**
//!
//! CT.4. `contacts.write` (Trusted-tier only by default), and
//! additionally **confirm-first** at the tool level: the input
//! must carry `confirm: true` or the tool refuses without
//! calling the API (the Documents-`delete` policy). Single
//! DELETE to `people/{resourceName}:deleteContact`.
//!
//! The People API DELETE is idempotent — the client treats
//! 404/410 as already-gone so re-deleting reports success.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::contacts_client::SharedContactsClient;
use crate::tools::resource_name::parse_resource_name;

pub struct ContactsDelete {
    id: ToolId,
    schema: Value,
    client: SharedContactsClient,
}

impl ContactsDelete {
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
            detail: format!("contacts.delete: {detail}"),
        })
    }
}

#[async_trait]
impl Tool for ContactsDelete {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "contacts.delete"
    }

    fn description(&self) -> &str {
        "Delete a person from the user's Google contacts. \
         IRREVERSIBLE. Input is a JSON object with required \
         `resource_name` (e.g. `people/c123`) and `confirm` \
         (must be the boolean `true` — the tool refuses without \
         it). Returns `{resource_name, deleted: true}`."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("contacts.write")
            .expect("contacts.write must parse — it is in KNOWN_BASES from Chapter Contacts")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let resource_name = match parse_input(&input) {
            Ok(r) => r,
            Err(reason) => return self.fail(&reason),
        };

        let path = format!("/{resource_name}:deleteContact");
        match self.client.delete(&path).await {
            Ok(_status) => ToolOutcome::Completed {
                output: json!({ "resource_name": resource_name, "deleted": true }),
                verified: Verification::NotApplicable,
            },
            Err(e) => self.fail(&format!("API call failed: {e}")),
        }
    }
}

/// Validate the resource name and enforce the confirm-first
/// gate. Returns the validated resource name on success.
fn parse_input(input: &Value) -> Result<String, String> {
    let resource_name = parse_resource_name(input)?;
    // SAFETY: parse_resource_name already proved input is an object.
    let confirmed = input
        .get("confirm")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !confirmed {
        return Err(
            "delete is irreversible — re-issue with `confirm: true` to proceed".to_string(),
        );
    }
    Ok(resource_name)
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "resource_name": {"type": "string", "description": "People API resource name, e.g. `people/c123`."},
            "confirm": {"type": "boolean", "description": "Must be `true` — guards an irreversible delete."}
        },
        "required": ["resource_name", "confirm"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_without_confirm() {
        let e = parse_input(&json!({"resource_name": "people/c1"})).expect_err("err");
        assert!(e.contains("confirm"), "{e}");
    }

    #[test]
    fn refuses_when_confirm_false() {
        let e = parse_input(&json!({"resource_name": "people/c1", "confirm": false}))
            .expect_err("err");
        assert!(e.contains("confirm"), "{e}");
    }

    #[test]
    fn proceeds_with_confirm_true() {
        let r = parse_input(&json!({"resource_name": "people/c1", "confirm": true})).expect("ok");
        assert_eq!(r, "people/c1");
    }

    #[test]
    fn validates_resource_name_before_confirm_gate() {
        // A bad resource name fails regardless of confirm.
        let e = parse_input(&json!({"resource_name": "groups/x", "confirm": true}))
            .expect_err("err");
        assert!(e.contains("people/"), "{e}");
    }

    #[test]
    fn input_schema_requires_resource_name_and_confirm() {
        let s = input_schema();
        assert_eq!(s["required"], json!(["resource_name", "confirm"]));
        assert_eq!(s["additionalProperties"], false);
    }
}

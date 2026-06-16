//! Shared People API resource-name validation.
//!
//! People resource names are `people/<id>`. Requiring that
//! prefix (and rejecting `..`) keeps a tool from being coerced
//! into building a request path that escapes the `people`
//! collection. Used by `contacts.get`, `contacts.update`, and
//! `contacts.delete`.

use serde_json::Value;

/// Validate a bare resource-name string.
pub fn validate_resource_name(name: &str) -> Result<(), String> {
    if !name.starts_with("people/") || name.contains("..") {
        return Err(format!(
            "`resource_name` must look like `people/<id>` (got {name:?})"
        ));
    }
    Ok(())
}

/// Extract and validate the required `resource_name` field from
/// a tool input object.
pub fn parse_resource_name(input: &Value) -> Result<String, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let name = match obj.get("resource_name") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        Some(Value::String(_)) | None => {
            return Err("`resource_name` is required and must be a non-empty string".to_string())
        }
        Some(_) => return Err("`resource_name` must be a string".to_string()),
    };
    validate_resource_name(&name)?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_people_id() {
        assert!(validate_resource_name("people/c123").is_ok());
    }

    #[test]
    fn rejects_other_collection() {
        assert!(validate_resource_name("contactGroups/x").is_err());
    }

    #[test]
    fn rejects_traversal() {
        assert!(validate_resource_name("people/../x").is_err());
    }

    #[test]
    fn parse_trims_and_validates() {
        let r = parse_resource_name(&json!({"resource_name": "  people/c1 "})).expect("ok");
        assert_eq!(r, "people/c1");
    }

    #[test]
    fn parse_requires_field() {
        let e = parse_resource_name(&json!({})).expect_err("err");
        assert!(e.contains("`resource_name`"), "{e}");
    }
}

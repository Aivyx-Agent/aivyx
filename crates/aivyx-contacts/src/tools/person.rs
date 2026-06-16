//! Shared `Person` output trimming.
//!
//! The People API returns deeply-nested `Person` objects where
//! every field is an array of `{metadata, value, ...}` records.
//! [`trim_person`] flattens one into the flat, snake-cased shape
//! every contacts tool returns — matching the `web.search` /
//! Gmail precedent so the cross-tool response shape is
//! operator-predictable:
//!
//! ```json
//! {
//!   "resource_name": "people/c123",
//!   "etag": "%EgU…",
//!   "display_name": "Dana Lee",
//!   "emails": ["dana@example.com"],
//!   "phones": ["+1 555 0100"],
//!   "organizations": ["Acme — Designer"]
//! }
//! ```
//!
//! `etag` is surfaced on every read so `contacts.update` (CT.4)
//! can round-trip the People API's optimistic-concurrency token.

use serde_json::{json, Value};

/// The People API field mask every contacts tool requests —
/// the four fields [`trim_person`] surfaces. Passed as
/// `readMask` (search) or `personFields` (list / get / update).
pub const PERSON_FIELDS: &str = "names,emailAddresses,phoneNumbers,organizations";

/// Flatten a raw People API `Person` JSON object into the
/// trimmed snake-cased shape Aivyx tools return.
pub fn trim_person(person: &Value) -> Value {
    json!({
        "resource_name": person.get("resourceName").and_then(Value::as_str).unwrap_or_default(),
        "etag": person.get("etag").and_then(Value::as_str).unwrap_or_default(),
        "display_name": display_name(person),
        "emails": string_values(person, "emailAddresses"),
        "phones": string_values(person, "phoneNumbers"),
        "organizations": organizations(person),
    })
}

/// The primary `displayName` if present, else the first name
/// record's `displayName`, else empty.
fn display_name(person: &Value) -> String {
    let names = match person.get("names").and_then(Value::as_array) {
        Some(n) => n,
        None => return String::new(),
    };
    // Prefer the metadata.primary name; fall back to the first.
    names
        .iter()
        .find(|n| {
            n.get("metadata")
                .and_then(|m| m.get("primary"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .or_else(|| names.first())
        .and_then(|n| n.get("displayName"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Collect the `value` field from each record in a People API
/// array field (`emailAddresses`, `phoneNumbers`), skipping
/// blanks.
fn string_values(person: &Value, field: &str) -> Vec<String> {
    person
        .get(field)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|r| r.get("value").and_then(Value::as_str))
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Collect organizations as `"<name> — <title>"` (either part
/// may be absent).
fn organizations(person: &Value) -> Vec<String> {
    person
        .get("organizations")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    let name = o.get("name").and_then(Value::as_str).unwrap_or_default();
                    let title =
                        o.get("title").and_then(Value::as_str).unwrap_or_default();
                    match (name.is_empty(), title.is_empty()) {
                        (true, true) => None,
                        (false, true) => Some(name.to_string()),
                        (true, false) => Some(title.to_string()),
                        (false, false) => Some(format!("{name} — {title}")),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({
            "resourceName": "people/c123",
            "etag": "%EgUBAj0LCS4=",
            "names": [
                {"displayName": "Old Name", "metadata": {"primary": false}},
                {"displayName": "Dana Lee", "metadata": {"primary": true}}
            ],
            "emailAddresses": [
                {"value": "dana@example.com", "metadata": {"primary": true}},
                {"value": ""}
            ],
            "phoneNumbers": [{"value": "+1 555 0100"}],
            "organizations": [{"name": "Acme", "title": "Designer"}]
        })
    }

    #[test]
    fn trims_to_flat_snake_cased_shape() {
        let out = trim_person(&sample());
        assert_eq!(out["resource_name"], "people/c123");
        assert_eq!(out["etag"], "%EgUBAj0LCS4=");
        assert_eq!(out["display_name"], "Dana Lee"); // primary preferred
        assert_eq!(out["emails"], json!(["dana@example.com"])); // blank dropped
        assert_eq!(out["phones"], json!(["+1 555 0100"]));
        assert_eq!(out["organizations"], json!(["Acme — Designer"]));
    }

    #[test]
    fn missing_fields_become_empty() {
        let out = trim_person(&json!({"resourceName": "people/c9"}));
        assert_eq!(out["display_name"], "");
        assert_eq!(out["emails"], json!([]));
        assert_eq!(out["phones"], json!([]));
        assert_eq!(out["organizations"], json!([]));
        assert_eq!(out["etag"], "");
    }

    #[test]
    fn falls_back_to_first_name_when_no_primary() {
        let p = json!({"names": [{"displayName": "Only Name"}]});
        assert_eq!(trim_person(&p)["display_name"], "Only Name");
    }

    #[test]
    fn organization_with_only_name_or_title() {
        let p = json!({"organizations": [{"name": "Acme"}, {"title": "Designer"}]});
        assert_eq!(
            trim_person(&p)["organizations"],
            json!(["Acme", "Designer"])
        );
    }
}

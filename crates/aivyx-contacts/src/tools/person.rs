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

/// Build a People API `Person` request body from the friendly,
/// flat input `contacts.create` / `contacts.update` accept, and
/// report which People API field names were touched (for the
/// `updatePersonFields` mask — CT contract F-2: derive the mask
/// from the keys supplied).
///
/// Accepted input keys (all optional individually, but at least
/// one must be present):
/// - `given_name`, `family_name` → `names`
/// - `emails` (array of strings) → `emailAddresses`
/// - `phones` (array of strings) → `phoneNumbers`
/// - `organization`, `organization_title` → `organizations`
///
/// Returns `(body, fields)` where `body` is the partial Person
/// and `fields` are the distinct People API field names present.
pub fn build_contact_fields(
    input: &serde_json::Map<String, Value>,
) -> Result<(Value, Vec<&'static str>), String> {
    let mut body = serde_json::Map::new();
    let mut fields: Vec<&'static str> = Vec::new();

    // names
    let given = opt_string(input, "given_name")?;
    let family = opt_string(input, "family_name")?;
    if given.is_some() || family.is_some() {
        let mut name = serde_json::Map::new();
        if let Some(g) = given {
            name.insert("givenName".into(), Value::String(g));
        }
        if let Some(f) = family {
            name.insert("familyName".into(), Value::String(f));
        }
        body.insert("names".into(), json!([Value::Object(name)]));
        fields.push("names");
    }

    // emailAddresses
    if let Some(emails) = opt_string_array(input, "emails")? {
        let arr: Vec<Value> = emails
            .into_iter()
            .map(|v| json!({ "value": v }))
            .collect();
        body.insert("emailAddresses".into(), Value::Array(arr));
        fields.push("emailAddresses");
    }

    // phoneNumbers
    if let Some(phones) = opt_string_array(input, "phones")? {
        let arr: Vec<Value> = phones
            .into_iter()
            .map(|v| json!({ "value": v }))
            .collect();
        body.insert("phoneNumbers".into(), Value::Array(arr));
        fields.push("phoneNumbers");
    }

    // organizations
    let org = opt_string(input, "organization")?;
    let title = opt_string(input, "organization_title")?;
    if org.is_some() || title.is_some() {
        let mut o = serde_json::Map::new();
        if let Some(n) = org {
            o.insert("name".into(), Value::String(n));
        }
        if let Some(t) = title {
            o.insert("title".into(), Value::String(t));
        }
        body.insert("organizations".into(), json!([Value::Object(o)]));
        fields.push("organizations");
    }

    if fields.is_empty() {
        return Err(
            "supply at least one of: given_name, family_name, emails, phones, organization"
                .to_string(),
        );
    }
    Ok((Value::Object(body), fields))
}

fn opt_string(
    input: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<String>, String> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

fn opt_string_array(
    input: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<Vec<String>>, String> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(arr)) => {
            let mut out = Vec::new();
            for v in arr {
                match v.as_str() {
                    Some(s) if !s.trim().is_empty() => out.push(s.trim().to_string()),
                    Some(_) => {}
                    None => return Err(format!("`{key}` must be an array of strings")),
                }
            }
            if out.is_empty() {
                Ok(None)
            } else {
                Ok(Some(out))
            }
        }
        Some(_) => Err(format!("`{key}` must be an array of strings")),
    }
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

    fn obj(v: Value) -> serde_json::Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn build_fields_maps_friendly_input_to_person() {
        let (body, fields) = build_contact_fields(&obj(json!({
            "given_name": "Dana",
            "family_name": "Lee",
            "emails": ["dana@example.com"],
            "phones": ["+1 555 0100"],
            "organization": "Acme",
            "organization_title": "Designer"
        })))
        .expect("build");
        assert_eq!(body["names"][0]["givenName"], "Dana");
        assert_eq!(body["names"][0]["familyName"], "Lee");
        assert_eq!(body["emailAddresses"][0]["value"], "dana@example.com");
        assert_eq!(body["phoneNumbers"][0]["value"], "+1 555 0100");
        assert_eq!(body["organizations"][0]["name"], "Acme");
        assert_eq!(body["organizations"][0]["title"], "Designer");
        assert_eq!(
            fields,
            vec!["names", "emailAddresses", "phoneNumbers", "organizations"]
        );
    }

    #[test]
    fn build_fields_derives_mask_from_supplied_keys_only() {
        let (_body, fields) =
            build_contact_fields(&obj(json!({"emails": ["a@b.com"]}))).expect("build");
        assert_eq!(fields, vec!["emailAddresses"]);
    }

    #[test]
    fn build_fields_rejects_empty_input() {
        let e = build_contact_fields(&obj(json!({}))).expect_err("err");
        assert!(e.contains("at least one"), "{e}");
    }

    #[test]
    fn build_fields_rejects_non_string_email_entry() {
        let e = build_contact_fields(&obj(json!({"emails": [42]}))).expect_err("err");
        assert!(e.contains("emails"), "{e}");
    }

    #[test]
    fn build_fields_drops_blank_array_entries() {
        // All-blank emails array → treated as absent → no fields,
        // so an otherwise-empty input errors.
        let e = build_contact_fields(&obj(json!({"emails": ["  ", ""]}))).expect_err("err");
        assert!(e.contains("at least one"), "{e}");
    }
}

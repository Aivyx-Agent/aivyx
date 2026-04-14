//! Hand-rolled JSON-Schema validator for tool inputs.
//!
//! Phase 10 task 2 — runtime validation. Every `Tool` already
//! returns a real schema via `Tool::input_schema()`, but until this
//! module lands that schema is purely *advisory* to the LLM: the
//! turn loop accepts whatever JSON the planner emits and relies on
//! each tool's `required_scope` and `execute` to defend themselves.
//! That defense is genuinely in place — see the deny-scope paths in
//! `fs.read`, `fs.write`, `memory.read`, `memory.write`, and
//! `memory.forget` — but it is tool-specific and ad-hoc.
//!
//! This validator adds a **second fence**, run in the turn loop
//! before `required_scope`, that rejects structurally malformed
//! input uniformly. A tool author who forgets the deny-scope path
//! still gets a safe default from the loop.
//!
//! ## Scope of the subset
//!
//! Intentionally tiny — only the keywords the in-tree tools
//! actually emit from their `input_schema()`:
//!
//! - top-level `{"type": "object", "properties": {...}, "required":
//!   [...], "additionalProperties": false}`
//! - per-field `{"type": "string"}`, `{"type": "integer"}`
//! - `{"enum": [...]}` on string fields
//! - `{"minimum": N, "maximum": N}` on integer fields
//!
//! Anything else in the schema (notably `description`) is ignored
//! — present for the LLM's benefit, irrelevant to validation.
//!
//! ## What this validator deliberately does NOT do
//!
//! - Nested objects / arrays of objects. No in-tree tool uses them.
//! - `oneOf` / `anyOf` / `allOf`. The `memory.read` "topic xor
//!   topics" rule is cross-field and handled by
//!   `classify_memory_read_input` inside the tool, not here.
//! - Coercion. `"1"` is not an integer. The LLM should emit
//!   well-typed JSON; we reject sloppiness loudly.
//! - Defaulting. Missing-but-optional fields stay missing; the
//!   tool fills in its own defaults in `execute`.
//!
//! ## Why hand-rolled instead of `jsonschema`
//!
//! The `jsonschema` crate is ~5k LOC of transitive deps and
//! implements draft-2020-12, plus ref resolution, plus format
//! validators, plus a meta-schema. This subset is <150 lines and
//! has zero new deps. If a future tool needs something outside
//! this subset — it can extend the module, or we swap to the full
//! crate. Neither is urgent today.

use serde_json::Value;

/// A structured reason the input failed validation. Converted to a
/// single-line human-readable `detail` string by `Display` so the
/// turn loop can feed it into `ToolOutcome::Failed(AivyxError::Tool
/// { detail, .. })` without extra formatting boilerplate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// The schema or the input was not an object at the top level.
    /// At least one of the two must be malformed — well-formed tool
    /// schemas always open with `{"type": "object"}`, and the LLM
    /// tool-call surface always delivers a JSON object payload.
    ///
    /// `found` is `String` rather than `&'static str` because the
    /// schema's declared type can be a runtime string (e.g. the
    /// schema said `"type": "array"`). Cheap — this is error-path
    /// only, not hot-path.
    NotAnObject { found: String },

    /// A field listed in the schema's `required` array was missing
    /// from the input.
    MissingRequired { field: String },

    /// The input had a field that the schema does not allow (only
    /// emitted when the schema sets `additionalProperties: false`).
    UnknownField { field: String },

    /// A field was present but of the wrong JSON type.
    WrongType {
        field: String,
        expected: &'static str,
        found: &'static str,
    },

    /// A string field had a value outside the declared `enum`.
    NotInEnum {
        field: String,
        allowed: Vec<String>,
        found: String,
    },

    /// An integer field was below `minimum`.
    BelowMinimum {
        field: String,
        minimum: i64,
        found: i64,
    },

    /// An integer field was above `maximum`.
    AboveMaximum {
        field: String,
        maximum: i64,
        found: i64,
    },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationError::NotAnObject { found } => {
                write!(f, "schema/input must be a JSON object, got {found}")
            }
            ValidationError::MissingRequired { field } => {
                write!(f, "missing required field `{field}`")
            }
            ValidationError::UnknownField { field } => {
                write!(f, "unknown field `{field}` (additionalProperties: false)")
            }
            ValidationError::WrongType {
                field,
                expected,
                found,
            } => {
                write!(
                    f,
                    "field `{field}` has wrong type: expected {expected}, got {found}"
                )
            }
            ValidationError::NotInEnum {
                field,
                allowed,
                found,
            } => write!(
                f,
                "field `{field}` value `{found}` is not in allowed set {allowed:?}"
            ),
            ValidationError::BelowMinimum {
                field,
                minimum,
                found,
            } => write!(
                f,
                "field `{field}` value {found} is below minimum {minimum}"
            ),
            ValidationError::AboveMaximum {
                field,
                maximum,
                found,
            } => write!(
                f,
                "field `{field}` value {found} is above maximum {maximum}"
            ),
        }
    }
}

impl std::error::Error for ValidationError {}

/// Validate `input` against `schema`. Returns `Ok(())` on success,
/// or the first `ValidationError` encountered. Order of field
/// checks is insertion order of the `properties` map — callers
/// should not depend on *which* error fires first when an input
/// has multiple problems, only that at least one fires.
pub fn validate(schema: &Value, input: &Value) -> Result<(), ValidationError> {
    // ---- Top-level object shape ---------------------------------
    let schema_obj = schema.as_object().ok_or_else(|| ValidationError::NotAnObject {
        found: kind_of(schema).to_string(),
    })?;
    let input_obj = input.as_object().ok_or_else(|| ValidationError::NotAnObject {
        found: kind_of(input).to_string(),
    })?;

    // If the schema doesn't declare `type: object` (or declares it
    // differently), fall through as "no object-level constraint" —
    // same behavior as an advisory-only schema. This is permissive
    // on purpose; the invariant we care about is that *declared*
    // fields are enforced, not that every schema opens with a type
    // key.
    if let Some(ty) = schema_obj.get("type").and_then(Value::as_str)
        && ty != "object"
    {
        return Err(ValidationError::NotAnObject {
            found: ty.to_string(),
        });
    }

    // ---- required --------------------------------------------------
    if let Some(required) = schema_obj.get("required").and_then(Value::as_array) {
        for name in required {
            let Some(name_str) = name.as_str() else {
                continue; // ignore malformed required entries
            };
            if !input_obj.contains_key(name_str) {
                return Err(ValidationError::MissingRequired {
                    field: name_str.to_string(),
                });
            }
        }
    }

    // ---- additionalProperties: false ------------------------------
    let properties = schema_obj
        .get("properties")
        .and_then(Value::as_object);
    let additional_props_allowed = schema_obj
        .get("additionalProperties")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if !additional_props_allowed {
        if let Some(props) = properties {
            for field in input_obj.keys() {
                if !props.contains_key(field) {
                    return Err(ValidationError::UnknownField {
                        field: field.clone(),
                    });
                }
            }
        }
    }

    // ---- per-field type / enum / minimum / maximum ----------------
    if let Some(props) = properties {
        for (field_name, field_schema) in props {
            let Some(value) = input_obj.get(field_name) else {
                continue; // missing optional — `required` above handles the required case
            };
            let Some(field_schema_obj) = field_schema.as_object() else {
                continue; // malformed per-field schema — advisory only
            };

            let field_type = field_schema_obj.get("type").and_then(Value::as_str);
            match field_type {
                Some("string") => {
                    let Some(s) = value.as_str() else {
                        return Err(ValidationError::WrongType {
                            field: field_name.clone(),
                            expected: "string",
                            found: kind_of(value),
                        });
                    };
                    if let Some(allowed) = field_schema_obj
                        .get("enum")
                        .and_then(Value::as_array)
                    {
                        let allowed_strs: Vec<String> = allowed
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect();
                        if !allowed_strs.iter().any(|a| a == s) {
                            return Err(ValidationError::NotInEnum {
                                field: field_name.clone(),
                                allowed: allowed_strs,
                                found: s.to_string(),
                            });
                        }
                    }
                }
                Some("integer") => {
                    // `serde_json` parses JSON numbers without a
                    // decimal point as integers. A JSON `1.0` is a
                    // float and must NOT validate against
                    // `{"type": "integer"}`.
                    let Some(n) = value.as_i64() else {
                        return Err(ValidationError::WrongType {
                            field: field_name.clone(),
                            expected: "integer",
                            found: kind_of(value),
                        });
                    };
                    if let Some(min) = field_schema_obj
                        .get("minimum")
                        .and_then(Value::as_i64)
                        && n < min
                    {
                        return Err(ValidationError::BelowMinimum {
                            field: field_name.clone(),
                            minimum: min,
                            found: n,
                        });
                    }
                    if let Some(max) = field_schema_obj
                        .get("maximum")
                        .and_then(Value::as_i64)
                        && n > max
                    {
                        return Err(ValidationError::AboveMaximum {
                            field: field_name.clone(),
                            maximum: max,
                            found: n,
                        });
                    }
                }
                Some(_) | None => {
                    // Unknown or missing type keyword — accept. The
                    // in-tree tools don't emit anything outside
                    // {string, integer}, so there's nothing to
                    // check. A future tool that adds a boolean or
                    // array field extends this match.
                }
            }
        }
    }

    Ok(())
}

/// Short human-readable JSON kind for error messages.
fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- Base: empty schema accepts empty input --------------------

    #[test]
    fn empty_schema_accepts_empty_object() {
        assert!(validate(&json!({"type": "object"}), &json!({})).is_ok());
    }

    #[test]
    fn non_object_input_is_rejected() {
        let schema = json!({"type": "object"});
        let err = validate(&schema, &json!(42)).unwrap_err();
        match err {
            ValidationError::NotAnObject { found } => assert_eq!(found, "number"),
            other => panic!("expected NotAnObject, got {other:?}"),
        }
    }

    #[test]
    fn non_object_schema_is_rejected() {
        let err = validate(&json!(42), &json!({})).unwrap_err();
        assert!(matches!(err, ValidationError::NotAnObject { .. }));
    }

    // ---- required --------------------------------------------------

    #[test]
    fn required_field_missing_fails() {
        let schema = json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        });
        let err = validate(&schema, &json!({})).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::MissingRequired { field } if field == "path"
        ));
    }

    #[test]
    fn required_field_present_passes() {
        let schema = json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        });
        assert!(validate(&schema, &json!({"path": "notes/today.md"})).is_ok());
    }

    // ---- additionalProperties -------------------------------------

    #[test]
    fn additional_properties_false_rejects_unknown_field() {
        let schema = json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "additionalProperties": false
        });
        let err = validate(&schema, &json!({"path": "a", "rogue": 1})).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownField { field } if field == "rogue"
        ));
    }

    #[test]
    fn additional_properties_default_true_allows_unknown_field() {
        // No `additionalProperties` key at all — JSON-Schema default
        // is permissive. This matches fs.read's current schema,
        // which does NOT declare additionalProperties.
        let schema = json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        });
        assert!(
            validate(&schema, &json!({"path": "a", "rogue": 1})).is_ok(),
            "unset additionalProperties must default to permissive"
        );
    }

    // ---- string type / enum ---------------------------------------

    #[test]
    fn wrong_type_on_string_field_fails() {
        let schema = json!({
            "type": "object",
            "properties": { "path": { "type": "string" } }
        });
        let err = validate(&schema, &json!({"path": 42})).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::WrongType { field, expected: "string", found: "number" }
                if field == "path"
        ));
    }

    #[test]
    fn enum_string_allows_declared_value() {
        let schema = json!({
            "type": "object",
            "properties": {
                "topics": { "type": "string", "enum": ["*"] }
            }
        });
        assert!(validate(&schema, &json!({"topics": "*"})).is_ok());
    }

    #[test]
    fn enum_string_rejects_undeclared_value() {
        let schema = json!({
            "type": "object",
            "properties": {
                "topics": { "type": "string", "enum": ["*"] }
            }
        });
        let err = validate(&schema, &json!({"topics": "notes"})).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::NotInEnum { field, .. } if field == "topics"
        ));
    }

    // ---- integer type / min / max ---------------------------------

    #[test]
    fn integer_type_accepts_whole_number() {
        let schema = json!({
            "type": "object",
            "properties": { "limit": { "type": "integer" } }
        });
        assert!(validate(&schema, &json!({"limit": 16})).is_ok());
    }

    #[test]
    fn integer_type_rejects_float_literal() {
        // JSON's `1.0` is a float, and {"type": "integer"} must not
        // accept it. serde_json exposes this via as_i64() returning
        // None for floats with a decimal point.
        let schema = json!({
            "type": "object",
            "properties": { "limit": { "type": "integer" } }
        });
        let err = validate(&schema, &json!({"limit": 1.5})).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::WrongType { field, expected: "integer", .. }
                if field == "limit"
        ));
    }

    #[test]
    fn integer_type_rejects_string_that_looks_like_a_number() {
        // No coercion — `"5"` is a string, not an integer.
        let schema = json!({
            "type": "object",
            "properties": { "limit": { "type": "integer" } }
        });
        let err = validate(&schema, &json!({"limit": "5"})).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::WrongType { expected: "integer", found: "string", .. }
        ));
    }

    #[test]
    fn integer_below_minimum_fails() {
        let schema = json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "minimum": 1, "maximum": 64 }
            }
        });
        let err = validate(&schema, &json!({"limit": 0})).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::BelowMinimum { field, minimum: 1, found: 0 }
                if field == "limit"
        ));
    }

    #[test]
    fn integer_above_maximum_fails() {
        let schema = json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "minimum": 1, "maximum": 64 }
            }
        });
        let err = validate(&schema, &json!({"limit": 65})).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::AboveMaximum { field, maximum: 64, found: 65 }
                if field == "limit"
        ));
    }

    #[test]
    fn integer_at_boundaries_passes() {
        let schema = json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "minimum": 1, "maximum": 64 }
            }
        });
        assert!(validate(&schema, &json!({"limit": 1})).is_ok());
        assert!(validate(&schema, &json!({"limit": 64})).is_ok());
    }

    // ---- Real in-tree schemas: end-to-end sanity ------------------

    #[test]
    fn real_fs_read_schema_accepts_good_input() {
        // Mirror of `read_input_schema_value()` in tools/fs.rs.
        let schema = json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        });
        assert!(validate(&schema, &json!({"path": "notes/today.md"})).is_ok());
    }

    #[test]
    fn real_fs_write_schema_rejects_missing_content() {
        let schema = json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        });
        let err = validate(&schema, &json!({"path": "a"})).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::MissingRequired { field } if field == "content"
        ));
    }

    #[test]
    fn real_memory_read_schema_accepts_wildcard_shape() {
        // Approximate mirror of `read_input_schema_value()` in
        // aivyx-memory. Limits are sentinel here — the real tool
        // uses MAX_READ_LIMIT = 64.
        let schema = json!({
            "type": "object",
            "properties": {
                "topic": { "type": "string" },
                "topics": { "type": "string", "enum": ["*"] },
                "limit": { "type": "integer", "minimum": 1, "maximum": 64 }
            },
            "additionalProperties": false
        });
        assert!(validate(&schema, &json!({"topics": "*"})).is_ok());
        assert!(validate(&schema, &json!({"topic": "notes", "limit": 8})).is_ok());
        // Bad enum value.
        assert!(validate(&schema, &json!({"topics": "everything"})).is_err());
        // Bad limit.
        assert!(validate(&schema, &json!({"topic": "notes", "limit": 0})).is_err());
    }

    #[test]
    fn real_memory_read_schema_mutual_exclusion_is_not_enforced_here() {
        // The "topic xor topics" rule is NOT in the schema subset —
        // it's handled by `classify_memory_read_input` inside the
        // memory tool. Both-fields-set must pass *this* validator
        // so the classifier gets to see it and map it to the deny
        // scope. If validation ever rejected both-fields-set, the
        // classifier's Invalid path would become unreachable and
        // its deny-scope guarantee would silently rot.
        let schema = json!({
            "type": "object",
            "properties": {
                "topic": { "type": "string" },
                "topics": { "type": "string", "enum": ["*"] }
            },
            "additionalProperties": false
        });
        assert!(
            validate(&schema, &json!({"topic": "notes", "topics": "*"})).is_ok(),
            "mutual-exclusion must remain a tool-level concern"
        );
    }
}

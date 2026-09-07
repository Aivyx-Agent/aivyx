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
//! - per-field `{"type": "string"}`, `{"type": "integer"}`,
//!   `{"type": "boolean"}`
//! - per-field `{"type": "object", ...}` (Phase 11 Task 3 — one
//!   level of nesting, used by `shell.exec` for its `args` block).
//!   Nested objects recursively honor `required` and
//!   `additionalProperties: false` at each level independently,
//!   and validation errors thread a dotted JSON-pointer path
//!   (e.g., `args.cwd: expected string`) so audit messages stay
//!   readable. Nesting depth is not bounded by the validator —
//!   the recursion follows whatever the schema declares — but
//!   the in-tree tools only go one level deep and there is no
//!   in-tree reason to go further.
//! - `{"enum": [...]}` on string fields
//! - `{"minimum": N, "maximum": N}` on integer fields
//!
//! Anything else in the schema (notably `description`) is ignored
//! — present for the LLM's benefit, irrelevant to validation.
//!
//! ## What this validator deliberately does NOT do
//!
//! - Arrays, or arrays of objects. No in-tree tool uses them.
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
///
/// Nested objects (Phase 11 Task 3) are validated recursively and
/// error `field` values are dotted JSON-pointer-style paths
/// (`args.cwd`, `args.timeout_ms`) so audit messages identify the
/// exact offending location regardless of nesting depth.
pub fn validate(schema: &Value, input: &Value) -> Result<(), ValidationError> {
    validate_at(schema, input, "")
}

/// Recursive implementation of [`validate`]. `path` is the dotted
/// JSON-pointer prefix to prepend to any error's `field`. At the
/// top level `path` is empty; for a nested `args` subobject it is
/// `"args"`; for `args.cwd` inside `args` it would be `"args.cwd"`.
///
/// Extracted as a free function so the top-level `validate` stays a
/// pure entry point and tests that want to exercise path-prefixing
/// directly can call this helper without poking at private struct
/// state. Kept `pub(crate)` rather than `pub` so the recursive
/// surface does not leak into `aivyx-core`'s public API.
pub(crate) fn validate_at(
    schema: &Value,
    input: &Value,
    path: &str,
) -> Result<(), ValidationError> {
    // ---- Top-level (or nested) object shape ---------------------
    let schema_obj = schema
        .as_object()
        .ok_or_else(|| ValidationError::NotAnObject {
            found: kind_of(schema).to_string(),
        })?;
    let input_obj = input
        .as_object()
        .ok_or_else(|| ValidationError::NotAnObject {
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
                    field: join_path(path, name_str),
                });
            }
        }
    }

    // ---- additionalProperties: false ------------------------------
    let properties = schema_obj.get("properties").and_then(Value::as_object);
    let additional_props_allowed = schema_obj
        .get("additionalProperties")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if !additional_props_allowed {
        if let Some(props) = properties {
            for field in input_obj.keys() {
                if !props.contains_key(field) {
                    return Err(ValidationError::UnknownField {
                        field: join_path(path, field),
                    });
                }
            }
        }
    }

    // ---- per-field type / enum / minimum / maximum / nested ------
    if let Some(props) = properties {
        for (field_name, field_schema) in props {
            let Some(value) = input_obj.get(field_name) else {
                continue; // missing optional — `required` above handles the required case
            };
            let Some(field_schema_obj) = field_schema.as_object() else {
                continue; // malformed per-field schema — advisory only
            };
            let field_path = join_path(path, field_name);

            let field_type = field_schema_obj.get("type").and_then(Value::as_str);
            match field_type {
                Some("string") => {
                    let Some(s) = value.as_str() else {
                        return Err(ValidationError::WrongType {
                            field: field_path,
                            expected: "string",
                            found: kind_of(value),
                        });
                    };
                    if let Some(allowed) = field_schema_obj.get("enum").and_then(Value::as_array) {
                        let allowed_strs: Vec<String> = allowed
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect();
                        if !allowed_strs.iter().any(|a| a == s) {
                            return Err(ValidationError::NotInEnum {
                                field: field_path,
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
                            field: field_path,
                            expected: "integer",
                            found: kind_of(value),
                        });
                    };
                    if let Some(min) = field_schema_obj.get("minimum").and_then(Value::as_i64)
                        && n < min
                    {
                        return Err(ValidationError::BelowMinimum {
                            field: field_path,
                            minimum: min,
                            found: n,
                        });
                    }
                    if let Some(max) = field_schema_obj.get("maximum").and_then(Value::as_i64)
                        && n > max
                    {
                        return Err(ValidationError::AboveMaximum {
                            field: field_path,
                            maximum: max,
                            found: n,
                        });
                    }
                }
                Some("boolean") => {
                    if !value.is_boolean() {
                        return Err(ValidationError::WrongType {
                            field: field_path,
                            expected: "boolean",
                            found: kind_of(value),
                        });
                    }
                }
                Some("object") => {
                    // Phase 11 Task 3 — recurse into the nested
                    // object's own schema. Each nesting level gets
                    // its own independent `required` and
                    // `additionalProperties` check via the
                    // recursion's top-level shape walk above, and
                    // the dotted `field_path` becomes the new
                    // recursion-local prefix so errors surface with
                    // the full dotted JSON pointer.
                    if !value.is_object() {
                        return Err(ValidationError::WrongType {
                            field: field_path,
                            expected: "object",
                            found: kind_of(value),
                        });
                    }
                    validate_at(field_schema, value, &field_path)?;
                }
                Some(_) | None => {
                    // Unknown or missing type keyword — accept. A
                    // future tool that adds an array field extends
                    // this match.
                }
            }
        }
    }

    Ok(())
}

/// Dotted JSON-pointer join. Empty prefix yields the bare field
/// name (so a top-level `cmd` field stays `"cmd"`, not `".cmd"`),
/// which preserves the exact error shape Phase 10 tests pinned.
fn join_path(prefix: &str, field: &str) -> String {
    if prefix.is_empty() {
        field.to_string()
    } else {
        format!("{prefix}.{field}")
    }
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
            ValidationError::WrongType {
                expected: "integer",
                found: "string",
                ..
            }
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

    // ---- Phase 11 Task 3 — nested-object support ------------------

    /// A fixture mirroring `shell.exec`'s advertised nested schema:
    /// top-level `{cmd, args: {cwd, timeout_ms}}`. `cmd` required,
    /// `args` optional; when present, `args.cwd` and `args.timeout_ms`
    /// are both optional, `additionalProperties: false` at both
    /// levels.
    fn shell_exec_nested_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "cmd": { "type": "string" },
                "args": {
                    "type": "object",
                    "properties": {
                        "cwd": { "type": "string" },
                        "timeout_ms": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 600_000
                        }
                    },
                    "additionalProperties": false
                }
            },
            "required": ["cmd"],
            "additionalProperties": false
        })
    }

    #[test]
    fn nested_schema_accepts_flat_required_only_shape() {
        // The nested `args` object is optional — a call with just
        // `cmd` must validate. This is the no-args shell invocation.
        assert!(validate(&shell_exec_nested_schema(), &json!({"cmd": "ls"})).is_ok());
    }

    #[test]
    fn nested_schema_accepts_nested_shape() {
        assert!(
            validate(
                &shell_exec_nested_schema(),
                &json!({
                    "cmd": "ls",
                    "args": {
                        "cwd": "/repo",
                        "timeout_ms": 5000
                    }
                })
            )
            .is_ok()
        );
    }

    #[test]
    fn nested_schema_rejects_wrong_nested_type() {
        // `args.cwd` is an integer — the inner schema requires string.
        // The error `field` must be the dotted path `args.cwd`, not
        // just `cwd` — otherwise audit messages for nested schemas
        // would be ambiguous when two levels share a field name.
        let err = validate(
            &shell_exec_nested_schema(),
            &json!({"cmd": "ls", "args": {"cwd": 42}}),
        )
        .unwrap_err();
        match err {
            ValidationError::WrongType {
                field,
                expected,
                found,
            } => {
                assert_eq!(field, "args.cwd");
                assert_eq!(expected, "string");
                assert_eq!(found, "number");
            }
            other => panic!("expected WrongType for args.cwd, got {other:?}"),
        }
    }

    #[test]
    fn nested_schema_rejects_nested_additional_property() {
        // `args.rogue` is not declared, and the nested schema sets
        // `additionalProperties: false`. The path must be
        // `args.rogue`.
        let err = validate(
            &shell_exec_nested_schema(),
            &json!({"cmd": "ls", "args": {"rogue": "x"}}),
        )
        .unwrap_err();
        match err {
            ValidationError::UnknownField { field } => {
                assert_eq!(field, "args.rogue");
            }
            other => panic!("expected UnknownField for args.rogue, got {other:?}"),
        }
    }

    #[test]
    fn nested_schema_rejects_top_level_additional_property() {
        // Top-level `additionalProperties: false` must still fire
        // for top-level unknown keys — adding nested support must
        // not have demoted the top-level gate.
        let err = validate(
            &shell_exec_nested_schema(),
            &json!({"cmd": "ls", "rogue": 1}),
        )
        .unwrap_err();
        match err {
            ValidationError::UnknownField { field } => {
                assert_eq!(field, "rogue");
            }
            other => panic!("expected UnknownField for rogue, got {other:?}"),
        }
    }

    #[test]
    fn nested_schema_rejects_nested_integer_out_of_range() {
        // The `args.timeout_ms` integer has a `minimum` and `maximum`;
        // a value of 0 must fire BelowMinimum with the dotted path.
        let err = validate(
            &shell_exec_nested_schema(),
            &json!({"cmd": "ls", "args": {"timeout_ms": 0}}),
        )
        .unwrap_err();
        match err {
            ValidationError::BelowMinimum { field, .. } => {
                assert_eq!(field, "args.timeout_ms");
            }
            other => panic!("expected BelowMinimum for args.timeout_ms, got {other:?}"),
        }
    }

    #[test]
    fn nested_schema_rejects_args_of_wrong_type() {
        // Passing `args` as a string instead of an object must fire
        // WrongType at the `args` path, not recurse into a non-
        // object.
        let err = validate(
            &shell_exec_nested_schema(),
            &json!({"cmd": "ls", "args": "oops"}),
        )
        .unwrap_err();
        match err {
            ValidationError::WrongType {
                field,
                expected,
                found,
            } => {
                assert_eq!(field, "args");
                assert_eq!(expected, "object");
                assert_eq!(found, "string");
            }
            other => panic!("expected WrongType for args, got {other:?}"),
        }
    }

    #[test]
    fn nested_required_field_missing_at_nested_level() {
        // Construct a schema where the nested object has a required
        // field, and verify the error dotted-path is correct. The
        // `shell.exec` schema makes nothing required inside `args`,
        // so this fixture is synthetic — it pins the "nested required
        // checks produce dotted paths" contract independently.
        let schema = json!({
            "type": "object",
            "properties": {
                "outer": {
                    "type": "object",
                    "properties": {
                        "inner": { "type": "string" }
                    },
                    "required": ["inner"]
                }
            },
            "required": ["outer"]
        });
        let err = validate(&schema, &json!({"outer": {}})).unwrap_err();
        match err {
            ValidationError::MissingRequired { field } => {
                assert_eq!(field, "outer.inner");
            }
            other => panic!("expected MissingRequired for outer.inner, got {other:?}"),
        }
    }

    #[test]
    fn boolean_type_accepts_true_and_false() {
        let schema = json!({
            "type": "object",
            "properties": { "flag": { "type": "boolean" } }
        });
        assert!(validate(&schema, &json!({"flag": true})).is_ok());
        assert!(validate(&schema, &json!({"flag": false})).is_ok());
    }

    #[test]
    fn boolean_type_rejects_non_boolean() {
        let schema = json!({
            "type": "object",
            "properties": { "flag": { "type": "boolean" } }
        });
        let err = validate(&schema, &json!({"flag": "true"})).unwrap_err();
        match err {
            ValidationError::WrongType {
                field,
                expected: "boolean",
                found: "string",
            } => {
                assert_eq!(field, "flag");
            }
            other => panic!("expected WrongType boolean, got {other:?}"),
        }
    }

    #[test]
    fn join_path_preserves_flat_shape_for_empty_prefix() {
        // Backwards-compat anchor: before Task 3 all errors had bare
        // field names. `join_path("", "cmd")` must return `"cmd"`,
        // not `".cmd"`, so every Phase 10 test that pinned a bare
        // field name keeps matching.
        assert_eq!(join_path("", "cmd"), "cmd");
        assert_eq!(join_path("args", "cwd"), "args.cwd");
        assert_eq!(join_path("outer.middle", "leaf"), "outer.middle.leaf");
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

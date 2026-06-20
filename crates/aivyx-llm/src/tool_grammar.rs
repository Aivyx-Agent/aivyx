//! Tool-call grammar generation — Chapter Stencil (ST.1).
//!
//! Grammar-constrained decoding forces a local model to emit a
//! **valid, real-named tool call by construction** instead of
//! hoping the prompt lands. [`tool_call_grammar`] turns the
//! turn's registered tools into a JSON Schema that admits *only*
//! a well-formed call — the constraint a provider then hands to
//! its decoder (for `mistralrs`, `Constraint::JsonSchema`).
//!
//! The function is **pure and provider-agnostic**: `&[LlmToolDescriptor]`
//! in, [`serde_json::Value`] out, no engine, no I/O. It lives at
//! the crate root (not under `mistral_rs/`) precisely so a future
//! `llama-server` `/completion` GBNF path can reuse it unchanged.
//!
//! ## Shape
//!
//! A `oneOf` discriminated union — one branch per tool, plus a
//! reserved [`RESPOND_SENTINEL`] branch:
//!
//! ```jsonc
//! { "oneOf": [
//!   { "type": "object",
//!     "properties": { "name": { "const": "fs.read" },
//!                     "arguments": { /* fs.read's input_schema verbatim */ } },
//!     "required": ["name", "arguments"], "additionalProperties": false },
//!   // …one branch per registered tool…
//!   { "type": "object",                                   // the sentinel
//!     "properties": { "name": { "const": "respond" },
//!                     "arguments": { "type": "object",
//!                                    "properties": { "text": { "type": "string" } },
//!                                    "required": ["text"] } },
//!     "required": ["name", "arguments"], "additionalProperties": false }
//! ] }
//! ```
//!
//! Each `name` is pinned to a `const` drawn from the registry, so
//! a **hallucinated name can never match**; that branch's
//! `arguments` is the matching tool's `input_schema` *verbatim*,
//! so **malformed arguments can never match**. Because every
//! branch's `name` const is distinct, an instance satisfies at
//! most one branch — `oneOf` (exactly-one) is the correct
//! combinator, and an unknown name or bad args satisfies *zero*
//! branches → invalid.
//!
//! ## The `respond` sentinel
//!
//! A constrained decoder is *forced* into the grammar, so without
//! an escape the model could never reply in plain text. The
//! reserved [`RESPOND_SENTINEL`] branch is that escape: the
//! provider (ST.3) unwraps a `{"name":"respond","arguments":{"text":…}}`
//! "call" back into an ordinary assistant text message rather than
//! dispatching it. The sentinel name has **no dot**, while every
//! real Aivyx tool is `namespace.action` — so it can never collide
//! with a registered tool.

use serde_json::{json, Value};

use crate::LlmToolDescriptor;

/// Reserved pseudo-tool name the grammar admits so a constrained
/// model can decline to call a real tool and reply in plain text.
/// Has no `.` separator, so it cannot collide with a real
/// `namespace.action` tool name. ST.3's provider wiring unwraps a
/// call to this name back into an assistant text message.
pub const RESPOND_SENTINEL: &str = "respond";

/// Build a JSON Schema that admits **only** a valid tool call for
/// one of `tools` (or the [`RESPOND_SENTINEL`] text escape).
///
/// Pure and engine-free — the result is handed to a decoder as a
/// generation constraint. With an empty `tools` slice the grammar
/// still admits the sentinel, so a constrained turn can always
/// produce *something* well-formed.
pub fn tool_call_grammar(tools: &[LlmToolDescriptor]) -> Value {
    let mut branches: Vec<Value> = tools
        .iter()
        .map(|t| tool_branch(&t.name, &t.input_schema))
        .collect();
    branches.push(respond_branch());
    json!({ "oneOf": branches })
}

/// One union branch: `name` pinned to `name`, `arguments` set to
/// the tool's input schema verbatim.
fn tool_branch(name: &str, input_schema: &Value) -> Value {
    // Tools' input schemas are objects at the top level (the
    // registry validates this at registration). If a schema ever
    // isn't an object it can't be a meaningful `arguments`
    // constraint, so fall back to a permissive object — still
    // pinning the *name*, which is the load-bearing guarantee.
    let arguments = if input_schema.is_object() {
        input_schema.clone()
    } else {
        json!({ "type": "object" })
    };
    json!({
        "type": "object",
        "properties": {
            "name": { "const": name },
            "arguments": arguments,
        },
        "required": ["name", "arguments"],
        "additionalProperties": false,
    })
}

/// The reserved text-reply escape branch.
fn respond_branch() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": { "const": RESPOND_SENTINEL },
            "arguments": {
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
                "additionalProperties": false,
            },
        },
        "required": ["name", "arguments"],
        "additionalProperties": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two sample tools with realistic object input schemas.
    fn sample_tools() -> Vec<LlmToolDescriptor> {
        vec![
            LlmToolDescriptor {
                name: "fs.read".to_string(),
                description: "Read a file".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"],
                    "additionalProperties": false,
                }),
            },
            LlmToolDescriptor {
                name: "web.fetch".to_string(),
                description: "Fetch a URL".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "url": { "type": "string" } },
                    "required": ["url"],
                    "additionalProperties": false,
                }),
            },
        ]
    }

    /// Compile the generated grammar into a validator. Asserts the
    /// grammar is itself a well-formed schema (the precondition for
    /// every other test).
    fn validator_for(tools: &[LlmToolDescriptor]) -> jsonschema::Validator {
        let grammar = tool_call_grammar(tools);
        jsonschema::validator_for(&grammar)
            .expect("tool_call_grammar must produce a valid JSON Schema")
    }

    #[test]
    fn admits_a_valid_tool_call() {
        let v = validator_for(&sample_tools());
        assert!(v.is_valid(&json!({
            "name": "fs.read",
            "arguments": { "path": "/etc/hosts" },
        })));
        // The second tool too — both branches are reachable.
        assert!(v.is_valid(&json!({
            "name": "web.fetch",
            "arguments": { "url": "https://example.com" },
        })));
    }

    #[test]
    fn rejects_an_unknown_tool_name() {
        let v = validator_for(&sample_tools());
        // The exact failure prompting could never fix: a name that
        // isn't in the registry. No branch's `const` matches.
        assert!(!v.is_valid(&json!({
            "name": "fs.write_file", // gemma4's classic hallucination
            "arguments": { "path": "x", "content": "y" },
        })));
        assert!(!v.is_valid(&json!({
            "name": "browser_navigate",
            "arguments": {},
        })));
    }

    #[test]
    fn rejects_malformed_arguments() {
        let v = validator_for(&sample_tools());
        // Real name, missing the required `path`.
        assert!(!v.is_valid(&json!({
            "name": "fs.read",
            "arguments": {},
        })));
        // Real name, wrong type for `path`.
        assert!(!v.is_valid(&json!({
            "name": "fs.read",
            "arguments": { "path": 123 },
        })));
        // Real name, an argument the schema doesn't allow.
        assert!(!v.is_valid(&json!({
            "name": "fs.read",
            "arguments": { "path": "/x", "bogus": true },
        })));
    }

    #[test]
    fn admits_the_respond_sentinel() {
        let v = validator_for(&sample_tools());
        assert!(v.is_valid(&json!({
            "name": RESPOND_SENTINEL,
            "arguments": { "text": "I've finished the task." },
        })));
        // …even with no tools registered — the escape is always
        // available so a constrained turn can produce plain text.
        let v_empty = validator_for(&[]);
        assert!(v_empty.is_valid(&json!({
            "name": "respond",
            "arguments": { "text": "hello" },
        })));
        // But the sentinel still has to be well-formed.
        assert!(!v_empty.is_valid(&json!({
            "name": "respond",
            "arguments": {},
        })));
    }

    #[test]
    fn rejects_missing_top_level_fields_and_cross_tool_args() {
        let v = validator_for(&sample_tools());
        // No `arguments` at all.
        assert!(!v.is_valid(&json!({ "name": "fs.read" })));
        // No `name` at all.
        assert!(!v.is_valid(&json!({ "arguments": { "path": "/x" } })));
        // fs.read's name with web.fetch's arguments — the name
        // const selects the fs.read branch, whose schema rejects
        // `url`/forbids the missing `path`.
        assert!(!v.is_valid(&json!({
            "name": "fs.read",
            "arguments": { "url": "https://example.com" },
        })));
    }
}

//! Textual tool-call extraction — Phase 126 substrate.
//!
//! Some LLM providers emit tool-call JSON in **response
//! text** rather than the protocol's native `tool_calls`
//! array. Phase 124's live verification observed this with
//! qwen3.6:27b:
//!
//! ```text
//! <tool_code>
//!   {"name": "fs.write", "arguments": {"path": "test.txt", "content": "..."}}
//! </tool_code>
//! ```
//!
//! And with gemma4:31b (different wrapper, different shape):
//!
//! ```text
//! <tool_call>
//!   {"tool": "fs.write_file", "parameters": {"path": "test.txt", "content": "..."}}
//! </tool_call>
//! ```
//!
//! The extractor in this module is a pure-function parser
//! that recognizes both wrapper tags and both JSON shapes,
//! returning a `Vec<ExtractedToolCall>` the planner can
//! dispatch as if they were real protocol tool calls.
//!
//! ## Composition with Phase 120 fuzzy-recovery
//!
//! Extraction only validates JSON shape; it does NOT
//! validate the tool name against the registered tool set.
//! When the extracted name is unknown, the planner's
//! existing Phase 120 fuzzy-recovery substrate fires:
//! `fs.write_file` → `fs.write` at Jaccard similarity 2/3
//! ≈ 0.667 (operator-configurable threshold).
//!
//! ## Wrapper-tag mismatch
//!
//! `<tool_code>...</tool_call>` (mismatched open/close
//! tags) does NOT match either pattern; the block is
//! dropped silently. This is by design — the wrappers are
//! identified by paired matching, not by closing-tag
//! tolerance.
//!
//! ## False-positive defense
//!
//! The planner only invokes extraction when the LLM's
//! `tool_calls` array is empty. An operator pasting a
//! literal `<tool_code>` block into a chat about
//! tool-call syntax doesn't trigger extraction because
//! the model's response would carry no protocol tool
//! calls AND its text would include the operator's
//! pasted block. The planner-side gate is documented
//! separately; this module's job is the parse.

use serde_json::Value;

/// One tool-call recovered from response text.
///
/// `wrapper_tag` is the open-tag identifier (`"tool_code"`
/// or `"tool_call"`) — the planner populates
/// `AuditTag::ToolCall.extracted_from_text` with this so
/// auditors can distinguish extracted calls from protocol
/// calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedToolCall {
    pub tool_name: String,
    pub arguments: Value,
    pub wrapper_tag: String,
}

/// Wrapper tags recognized by this extractor. Order matters
/// only for the documentation; the extractor scans the text
/// once and identifies whichever wrapper appears in order.
const RECOGNIZED_WRAPPERS: &[&str] = &["tool_code", "tool_call"];

/// Scan `text` for every textual tool-call block and return
/// the extracted calls in source order.
///
/// Empty `text` or text with no matching wrappers returns
/// an empty `Vec`. Malformed JSON inside an otherwise-valid
/// wrapper block is dropped silently — a permissive parse
/// is preferable to surfacing parse errors as call failures
/// since the wrapper match might have been spurious anyway
/// (e.g. inside a fenced code block discussing tool-call
/// syntax).
pub fn extract_tool_calls(text: &str) -> Vec<ExtractedToolCall> {
    let mut out: Vec<ExtractedToolCall> = Vec::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let Some(start) = find_next_open_tag(text, cursor) else {
            break;
        };
        // start = (offset_of_<, tag_name, after_close_>)
        let (lt_pos, tag, content_start) = start;
        // Look for the matching close tag.
        let close_tag = format!("</{tag}>");
        let Some(close_rel) = text[content_start..].find(&close_tag) else {
            // Unclosed wrapper — skip past this `<` so we
            // don't loop forever.
            cursor = lt_pos + 1;
            continue;
        };
        let close_abs = content_start + close_rel;
        let inner = &text[content_start..close_abs];
        if let Some(call) = parse_inner(inner.trim(), tag) {
            out.push(call);
        }
        cursor = close_abs + close_tag.len();
    }
    out
}

/// Find the next opening tag matching any recognized
/// wrapper. Returns `(byte_offset_of_<, tag_name,
/// byte_offset_after_>)` or `None` if no match in
/// `text[start..]`.
fn find_next_open_tag<'a>(
    text: &'a str,
    start: usize,
) -> Option<(usize, &'a str, usize)> {
    let mut best: Option<(usize, &'a str, usize)> = None;
    for tag in RECOGNIZED_WRAPPERS {
        let needle = format!("<{tag}>");
        if let Some(rel) = text[start..].find(&needle) {
            let abs = start + rel;
            let after = abs + needle.len();
            best = Some(match best {
                None => (abs, *tag, after),
                Some((prev_abs, _, _)) if abs < prev_abs => (abs, *tag, after),
                Some(prev) => prev,
            });
        }
    }
    best
}

/// Parse the inner JSON of a wrapper block. Accepts both
/// the `{"name": ..., "arguments": ...}` shape (qwen3 /
/// Anthropic-style) and the `{"tool": ...,
/// "parameters": ...}` shape (gemma4 / common alternative).
/// Either shape produces an `ExtractedToolCall`; other
/// shapes return `None` (silent drop).
fn parse_inner(inner: &str, wrapper_tag: &str) -> Option<ExtractedToolCall> {
    if inner.is_empty() {
        return None;
    }
    let value: Value = serde_json::from_str(inner).ok()?;
    let obj = value.as_object()?;

    // Try {"name", "arguments"} first.
    if let (Some(Value::String(name)), Some(args)) =
        (obj.get("name"), obj.get("arguments"))
    {
        if !name.trim().is_empty() {
            return Some(ExtractedToolCall {
                tool_name: name.trim().to_string(),
                arguments: args.clone(),
                wrapper_tag: wrapper_tag.to_string(),
            });
        }
    }
    // Fall back to {"tool", "parameters"}.
    if let (Some(Value::String(tool)), Some(params)) =
        (obj.get("tool"), obj.get("parameters"))
    {
        if !tool.trim().is_empty() {
            return Some(ExtractedToolCall {
                tool_name: tool.trim().to_string(),
                arguments: params.clone(),
                wrapper_tag: wrapper_tag.to_string(),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- single-block extraction ----------------------

    #[test]
    fn extracts_tool_code_with_name_arguments_shape() {
        let text = r#"
            <tool_code>
              {"name": "fs.write", "arguments": {"path": "x.txt", "content": "hi"}}
            </tool_code>
        "#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[0].wrapper_tag, "tool_code");
        assert_eq!(calls[0].arguments["path"], "x.txt");
        assert_eq!(calls[0].arguments["content"], "hi");
    }

    #[test]
    fn extracts_tool_call_with_tool_parameters_shape() {
        // gemma4's observed shape.
        let text = r#"
            <tool_call>
              {"tool": "fs.write_file", "parameters": {"path": "x", "content": "y"}}
            </tool_call>
        "#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write_file");
        assert_eq!(calls[0].wrapper_tag, "tool_call");
        assert_eq!(calls[0].arguments["path"], "x");
    }

    #[test]
    fn extracts_tool_call_with_name_arguments_shape() {
        // Models may use either wrapper with either shape.
        let text = r#"
            <tool_call>
              {"name": "memory.read", "arguments": {"topic": "x"}}
            </tool_call>
        "#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "memory.read");
        assert_eq!(calls[0].wrapper_tag, "tool_call");
    }

    #[test]
    fn extracts_tool_code_with_tool_parameters_shape() {
        let text = r#"
            <tool_code>
              {"tool": "fs.read", "parameters": {"path": "x"}}
            </tool_code>
        "#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.read");
    }

    // ---- multiple blocks ------------------------------

    #[test]
    fn extracts_multiple_blocks_in_source_order() {
        let text = r#"
            First call:
            <tool_code>{"name": "fs.read", "arguments": {"path": "a"}}</tool_code>
            Then another:
            <tool_call>{"tool": "memory.read", "parameters": {"topic": "b"}}</tool_call>
        "#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].tool_name, "fs.read");
        assert_eq!(calls[0].wrapper_tag, "tool_code");
        assert_eq!(calls[1].tool_name, "memory.read");
        assert_eq!(calls[1].wrapper_tag, "tool_call");
    }

    #[test]
    fn extracts_two_adjacent_blocks() {
        let text = r#"<tool_code>{"name": "fs.read", "arguments": {}}</tool_code><tool_code>{"name": "fs.write", "arguments": {}}</tool_code>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].tool_name, "fs.read");
        assert_eq!(calls[1].tool_name, "fs.write");
    }

    // ---- malformed / dropped cases --------------------

    #[test]
    fn drops_malformed_json_silently() {
        let text = "<tool_code>not json at all</tool_code>";
        let calls = extract_tool_calls(text);
        assert!(calls.is_empty());
    }

    #[test]
    fn drops_empty_block() {
        let text = "<tool_code></tool_code>";
        let calls = extract_tool_calls(text);
        assert!(calls.is_empty());
    }

    #[test]
    fn drops_block_with_neither_shape() {
        // Valid JSON but doesn't match name/arguments or
        // tool/parameters.
        let text = r#"<tool_code>{"foo": "bar"}</tool_code>"#;
        let calls = extract_tool_calls(text);
        assert!(calls.is_empty());
    }

    #[test]
    fn drops_block_with_empty_tool_name() {
        let text = r#"<tool_code>{"name": "", "arguments": {}}</tool_code>"#;
        let calls = extract_tool_calls(text);
        assert!(calls.is_empty());
    }

    #[test]
    fn drops_block_with_whitespace_only_tool_name() {
        let text = r#"<tool_code>{"name": "   ", "arguments": {}}</tool_code>"#;
        let calls = extract_tool_calls(text);
        assert!(calls.is_empty());
    }

    #[test]
    fn drops_block_with_non_string_tool_name() {
        let text = r#"<tool_code>{"name": 42, "arguments": {}}</tool_code>"#;
        let calls = extract_tool_calls(text);
        assert!(calls.is_empty());
    }

    #[test]
    fn drops_unclosed_wrapper() {
        let text = "<tool_code>{\"name\": \"x\", \"arguments\": {}}";
        let calls = extract_tool_calls(text);
        assert!(
            calls.is_empty(),
            "unclosed wrapper produces no extraction"
        );
    }

    #[test]
    fn drops_mismatched_open_close_tags() {
        // <tool_code> ... </tool_call> — close-tag mismatch.
        let text = r#"<tool_code>{"name": "x", "arguments": {}}</tool_call>"#;
        let calls = extract_tool_calls(text);
        assert!(
            calls.is_empty(),
            "wrapper close-tag must match open-tag exactly; mismatched pair dropped"
        );
    }

    // ---- prose context --------------------------------

    #[test]
    fn extracts_block_surrounded_by_prose() {
        let text = "I'll save the file for you.\n\n<tool_code>{\"name\":\"fs.write\",\"arguments\":{\"path\":\"x\"}}</tool_code>\n\nDone — let me know if you need anything else.";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
    }

    #[test]
    fn empty_text_returns_no_calls() {
        assert!(extract_tool_calls("").is_empty());
    }

    #[test]
    fn text_with_no_wrappers_returns_no_calls() {
        let text = "Just regular prose with no tool-call markers.";
        assert!(extract_tool_calls(text).is_empty());
    }

    // ---- argument structure preservation --------------

    #[test]
    fn preserves_arguments_as_arbitrary_value() {
        // Arguments can be any JSON value, not just an object.
        let text = r#"<tool_code>{"name": "x", "arguments": [1, 2, 3]}</tool_code>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments, json!([1, 2, 3]));
    }

    #[test]
    fn preserves_nested_object_arguments() {
        let text = r#"<tool_code>{"name": "fs.write", "arguments": {"path": "a", "meta": {"size": 42, "owner": "x"}}}</tool_code>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["path"], "a");
        assert_eq!(calls[0].arguments["meta"]["size"], 42);
        assert_eq!(calls[0].arguments["meta"]["owner"], "x");
    }

    // ---- edge: whitespace + line-formatting -----------

    #[test]
    fn tolerates_inner_whitespace_around_json() {
        let text = "<tool_code>\n  \n  {\"name\": \"fs.read\", \"arguments\": {}}\n  \n</tool_code>";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.read");
    }

    #[test]
    fn tolerates_compact_no_whitespace_block() {
        let text = r#"<tool_code>{"name":"x","arguments":{}}</tool_code>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
    }

    // ---- ordering when both wrappers present ----------

    #[test]
    fn extracts_tool_call_then_tool_code_in_source_order() {
        // tool_call first by position; tool_code second.
        let text = r#"<tool_call>{"tool":"a.x","parameters":{}}</tool_call> then <tool_code>{"name":"b.y","arguments":{}}</tool_code>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].tool_name, "a.x");
        assert_eq!(calls[0].wrapper_tag, "tool_call");
        assert_eq!(calls[1].tool_name, "b.y");
        assert_eq!(calls[1].wrapper_tag, "tool_code");
    }

    // ---- name-trimming -------------------------------

    #[test]
    fn trims_whitespace_around_tool_name() {
        let text = r#"<tool_code>{"name": "  fs.write  ", "arguments": {}}</tool_code>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].tool_name, "fs.write",
            "leading/trailing whitespace trimmed"
        );
    }
}

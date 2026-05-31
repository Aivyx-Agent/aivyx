//! Textual tool-call extraction — Phase 126 / 127 substrate.
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
//! Phase 127 expanded the substrate with the Qwen3-Coder XML
//! shape observed in Ollama issue #14745 (qwen3.5:9b) — XML
//! inside the `<tool_call>` wrapper instead of JSON:
//!
//! ```text
//! <tool_call>
//! <function=fs.write>
//! <parameter=path>test.txt</parameter>
//! <parameter=content>phase 127 verification</parameter>
//! </function>
//! </tool_call>
//! ```
//!
//! The extractor in this module is a pure-function parser
//! that recognizes both wrapper tags and three inner shapes,
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
///
/// `inner_format` records which inner-shape parser matched
/// — `"json-name-arguments"`, `"json-tool-parameters"`, or
/// `"qwen3-coder-xml"`. Phase 127 introduced the field so
/// future tasks can thread it into audit forensics; for
/// now it's an internal distinguisher useful for tests
/// and for the eventual audit-side wiring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedToolCall {
    pub tool_name: String,
    pub arguments: Value,
    pub wrapper_tag: String,
    pub inner_format: String,
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

/// Parse the inner content of a wrapper block. Three
/// inner-shape parsers are tried in priority order:
///
/// 1. `{"name": ..., "arguments": ...}` (Hermes-style;
///    qwen3, Anthropic, etc.). Either wrapper.
/// 2. `{"tool": ..., "parameters": ...}` (gemma4 /
///    common alternative). Either wrapper.
/// 3. Qwen3-Coder XML inner — `<function=NAME>
///    <parameter=K>V</parameter>...</function>`. Only
///    inside `<tool_call>` per the empirical literature
///    (Ollama issue #14745).
///
/// First match wins; other shapes return `None` (silent
/// drop).
fn parse_inner(inner: &str, wrapper_tag: &str) -> Option<ExtractedToolCall> {
    if inner.is_empty() {
        return None;
    }

    // Try JSON shapes first.
    if let Ok(value) = serde_json::from_str::<Value>(inner) {
        if let Some(obj) = value.as_object() {
            // {"name", "arguments"} shape.
            if let (Some(Value::String(name)), Some(args)) =
                (obj.get("name"), obj.get("arguments"))
            {
                if !name.trim().is_empty() {
                    return Some(ExtractedToolCall {
                        tool_name: name.trim().to_string(),
                        arguments: args.clone(),
                        wrapper_tag: wrapper_tag.to_string(),
                        inner_format: "json-name-arguments".to_string(),
                    });
                }
            }
            // {"tool", "parameters"} shape.
            if let (Some(Value::String(tool)), Some(params)) =
                (obj.get("tool"), obj.get("parameters"))
            {
                if !tool.trim().is_empty() {
                    return Some(ExtractedToolCall {
                        tool_name: tool.trim().to_string(),
                        arguments: params.clone(),
                        wrapper_tag: wrapper_tag.to_string(),
                        inner_format: "json-tool-parameters".to_string(),
                    });
                }
            }
        }
    }

    // Fall back to Qwen3-Coder XML, restricted to the
    // `<tool_call>` wrapper. The empirical observation
    // (Ollama issue #14745 — qwen3.5:9b) is that the XML
    // inner appears inside `<tool_call>` specifically;
    // `<tool_code>` is the markdown-fence-like form used
    // for JSON-shape emission. Keeping the XML parser
    // scoped to `<tool_call>` avoids false-positive XML
    // matches inside other wrapper kinds.
    if wrapper_tag == "tool_call" {
        if let Some(call) = parse_qwen3_coder_xml(inner, wrapper_tag) {
            return Some(call);
        }
    }

    None
}

/// Parse one Qwen3-Coder-style XML function call. Format:
///
/// ```text
/// <function=fs.write>
/// <parameter=path>test.txt</parameter>
/// <parameter=content>phase 127 verification</parameter>
/// </function>
/// ```
///
/// Returns the first complete `<function=...>...</function>`
/// block found. Parameter VALUEs are JSON-coerced where
/// the literal text parses as a JSON scalar/array/object;
/// otherwise the value is treated as a raw string. Repeated
/// parameter names take the LAST value (override semantics).
///
/// Malformed input — missing `<function=>` open tag,
/// unclosed function block, parameter blocks outside any
/// function — returns `None`.
fn parse_qwen3_coder_xml(inner: &str, wrapper_tag: &str) -> Option<ExtractedToolCall> {
    let func_open_prefix = "<function=";
    let func_open_start = inner.find(func_open_prefix)?;
    // Function name runs from after `<function=` to the
    // next `>`. Bail if the open tag is unterminated.
    let after_prefix = func_open_start + func_open_prefix.len();
    let name_end_rel = inner[after_prefix..].find('>')?;
    let func_name = inner[after_prefix..after_prefix + name_end_rel]
        .trim()
        .to_string();
    if func_name.is_empty() {
        return None;
    }
    let body_start = after_prefix + name_end_rel + 1;

    // Locate the matching `</function>` close. The Qwen3-
    // Coder format doesn't nest functions, so a flat scan
    // for the first `</function>` after `body_start` is
    // correct.
    let close_tag = "</function>";
    let close_rel = inner[body_start..].find(close_tag)?;
    let body = &inner[body_start..body_start + close_rel];

    // Walk the body, extracting each `<parameter=KEY>V</parameter>`.
    let mut args = serde_json::Map::<String, Value>::new();
    let mut cursor = 0;
    let param_open_prefix = "<parameter=";
    let param_close = "</parameter>";
    while cursor < body.len() {
        let Some(open_rel) = body[cursor..].find(param_open_prefix) else {
            break;
        };
        let open_start = cursor + open_rel;
        let after_param_prefix = open_start + param_open_prefix.len();
        let Some(key_end_rel) = body[after_param_prefix..].find('>') else {
            // Unterminated parameter open tag; bail out of
            // the walk but still return whatever we
            // accumulated.
            break;
        };
        let key = body[after_param_prefix..after_param_prefix + key_end_rel]
            .trim()
            .to_string();
        let value_start = after_param_prefix + key_end_rel + 1;
        let Some(close_rel) = body[value_start..].find(param_close) else {
            break;
        };
        let raw_value = &body[value_start..value_start + close_rel];
        if !key.is_empty() {
            args.insert(key, coerce_xml_param_value(raw_value));
        }
        cursor = value_start + close_rel + param_close.len();
    }

    Some(ExtractedToolCall {
        tool_name: func_name,
        arguments: Value::Object(args),
        wrapper_tag: wrapper_tag.to_string(),
        inner_format: "qwen3-coder-xml".to_string(),
    })
}

/// JSON-coerce a Qwen3-Coder XML parameter value.
///
/// The literal value text is tried as a JSON document; if
/// it parses to a structured type (number, bool, null,
/// array, object) the parsed Value is returned. If it
/// parses to a JSON string (quoted literal), the unquoted
/// string is returned. Otherwise the raw text is wrapped
/// as a JSON string verbatim.
///
/// This matches the conservative posture of llama.cpp's
/// Qwen3-Coder parser: numeric-looking strings remain
/// strings unless the literal is unquoted, so
/// `<parameter=path>test.txt</parameter>` stays a string
/// while `<parameter=count>42</parameter>` becomes a
/// number. Operators can force a string by adding quotes:
/// `<parameter=count>"42"</parameter>`.
fn coerce_xml_param_value(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::String(String::new());
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(v) => v,
        Err(_) => Value::String(raw.to_string()),
    }
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

    // ====================================================
    // Phase 127 Task 2 — Qwen3-Coder XML inner-shape parser.
    //
    // Format observed in Ollama issue #14745 (qwen3.5:9b) and
    // Continue discussion #10534 (qwen3-coder-30b), among
    // others:
    //
    //   <tool_call>
    //   <function=NAME>
    //   <parameter=KEY>VALUE</parameter>
    //   ...
    //   </function>
    //   </tool_call>
    //
    // The XML inner is only tried inside the `<tool_call>`
    // wrapper, not `<tool_code>`. Single function block per
    // wrapper for the MVP — multi-function-per-wrapper
    // batching is deferred (no empirical evidence of it in
    // the wild yet).
    // ====================================================

    #[test]
    fn phase_127_qwen3_coder_xml_single_string_param() {
        let text = r#"<tool_call><function=fs.write><parameter=path>test.txt</parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[0].wrapper_tag, "tool_call");
        assert_eq!(calls[0].inner_format, "qwen3-coder-xml");
        assert_eq!(calls[0].arguments["path"], "test.txt");
    }

    #[test]
    fn phase_127_qwen3_coder_xml_multiple_params() {
        let text = r#"
            <tool_call>
            <function=fs.write>
            <parameter=path>test.txt</parameter>
            <parameter=content>phase 127 verification</parameter>
            </function>
            </tool_call>
        "#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[0].arguments["path"], "test.txt");
        assert_eq!(calls[0].arguments["content"], "phase 127 verification");
    }

    #[test]
    fn phase_127_qwen3_coder_xml_no_params() {
        let text =
            r#"<tool_call><function=time.now></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "time.now");
        assert_eq!(calls[0].arguments, json!({}));
    }

    #[test]
    fn phase_127_qwen3_coder_xml_value_coercion_int() {
        let text = r#"<tool_call><function=task.set><parameter=count>42</parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["count"], 42);
    }

    #[test]
    fn phase_127_qwen3_coder_xml_value_coercion_float() {
        let text = r#"<tool_call><function=task.set><parameter=ratio>0.75</parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        let v = &calls[0].arguments["ratio"];
        assert!(v.is_f64(), "value should coerce to float; got {v:?}");
        assert_eq!(v.as_f64(), Some(0.75));
    }

    #[test]
    fn phase_127_qwen3_coder_xml_value_coercion_bool_true() {
        let text = r#"<tool_call><function=feature.set><parameter=enabled>true</parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["enabled"], true);
    }

    #[test]
    fn phase_127_qwen3_coder_xml_value_coercion_bool_false() {
        let text = r#"<tool_call><function=feature.set><parameter=enabled>false</parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["enabled"], false);
    }

    #[test]
    fn phase_127_qwen3_coder_xml_value_coercion_null() {
        let text = r#"<tool_call><function=memory.read><parameter=topic>null</parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        // `null` is a JSON-recognized scalar; coercion
        // converts to Value::Null.
        assert!(calls[0].arguments["topic"].is_null());
    }

    #[test]
    fn phase_127_qwen3_coder_xml_value_coercion_string_with_dots() {
        // "test.txt" is not valid JSON; falls back to raw
        // string. This is the load-bearing case for paths
        // (the operator's most common parameter type).
        let text = r#"<tool_call><function=fs.write><parameter=path>test.txt</parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["path"], "test.txt");
        assert!(calls[0].arguments["path"].is_string());
    }

    #[test]
    fn phase_127_qwen3_coder_xml_quoted_string_unwrapped() {
        // Operator-quoted "42" stays a string after JSON
        // coercion (parses to Value::String).
        let text = r#"<tool_call><function=task.set><parameter=count>"42"</parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["count"], "42");
        assert!(calls[0].arguments["count"].is_string());
    }

    #[test]
    fn phase_127_qwen3_coder_xml_value_coercion_json_array() {
        let text = r#"<tool_call><function=task.set><parameter=tags>[1,2,3]</parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["tags"], json!([1, 2, 3]));
    }

    #[test]
    fn phase_127_qwen3_coder_xml_value_coercion_json_object() {
        let text = r#"<tool_call><function=task.set><parameter=meta>{"size":42,"owner":"x"}</parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["meta"]["size"], 42);
        assert_eq!(calls[0].arguments["meta"]["owner"], "x");
    }

    #[test]
    fn phase_127_qwen3_coder_xml_mixed_value_types() {
        let text = r#"
            <tool_call>
            <function=health.check.add>
            <parameter=url>https://example.com</parameter>
            <parameter=interval_minutes>15</parameter>
            <parameter=enabled>true</parameter>
            </function>
            </tool_call>
        "#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "health.check.add");
        assert_eq!(calls[0].arguments["url"], "https://example.com");
        assert_eq!(calls[0].arguments["interval_minutes"], 15);
        assert_eq!(calls[0].arguments["enabled"], true);
    }

    #[test]
    fn phase_127_qwen3_coder_xml_dotted_function_name_preserved() {
        let text = r#"<tool_call><function=fs.delete><parameter=path>x</parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].tool_name, "fs.delete",
            "dotted tool name preserved verbatim"
        );
    }

    #[test]
    fn phase_127_qwen3_coder_xml_whitespace_around_params() {
        let text = r#"<tool_call><function=fs.write><parameter=path>  spaced.txt  </parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        // Raw value preserved as-is (operators may rely on
        // whitespace for prose-like content). JSON coercion
        // is tried on the trimmed text, but since
        // `spaced.txt` doesn't parse as JSON the raw value
        // (including leading/trailing whitespace) is what
        // lands.
        assert_eq!(
            calls[0].arguments["path"],
            "  spaced.txt  ",
            "raw value preserved when not JSON-parseable"
        );
    }

    #[test]
    fn phase_127_qwen3_coder_xml_empty_param_value() {
        let text = r#"<tool_call><function=task.create><parameter=note></parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["note"], "");
    }

    #[test]
    fn phase_127_qwen3_coder_xml_repeated_param_takes_last() {
        let text = r#"<tool_call><function=fs.write><parameter=path>first</parameter><parameter=path>second</parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].arguments["path"], "second",
            "repeated parameter name takes the last value"
        );
    }

    #[test]
    fn phase_127_qwen3_coder_xml_multiline_param_value() {
        let text = "<tool_call><function=fs.write><parameter=content>line one\nline two\nline three</parameter></function></tool_call>";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].arguments["content"],
            "line one\nline two\nline three"
        );
    }

    #[test]
    fn phase_127_qwen3_coder_xml_drops_missing_close_function() {
        // Function open with no close — drop.
        let text = r#"<tool_call><function=fs.write><parameter=path>x</parameter></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert!(
            calls.is_empty(),
            "missing </function> close means XML inner doesn't match; JSON shapes also fail; dropped"
        );
    }

    #[test]
    fn phase_127_qwen3_coder_xml_drops_unclosed_parameter() {
        // Parameter open with no close — parameter is
        // skipped but the function block still produces an
        // ExtractedToolCall with the remaining valid params.
        // For this test, the function has only one (broken)
        // param, so it produces a call with empty args.
        let text = r#"<tool_call><function=fs.write><parameter=path>x</function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(
            calls.len(),
            1,
            "function block parses even if one parameter is malformed; broken param dropped"
        );
        // The malformed parameter contributed nothing.
        assert!(calls[0].arguments.as_object().unwrap().is_empty());
    }

    #[test]
    fn phase_127_qwen3_coder_xml_drops_empty_function_name() {
        let text = r#"<tool_call><function=><parameter=path>x</parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert!(
            calls.is_empty(),
            "empty function name dropped"
        );
    }

    #[test]
    fn phase_127_qwen3_coder_xml_only_inside_tool_call_wrapper() {
        // Same XML inner inside `<tool_code>` MUST NOT
        // extract — Phase 127's parser is restricted to
        // `<tool_call>` per the empirical literature.
        let text = r#"<tool_code><function=fs.write><parameter=path>x</parameter></function></tool_code>"#;
        let calls = extract_tool_calls(text);
        assert!(
            calls.is_empty(),
            "Qwen3-Coder XML parser is scoped to <tool_call>; <tool_code> wrapper does NOT trigger XML fallback"
        );
    }

    #[test]
    fn phase_127_json_shape_preferred_over_xml_inside_tool_call() {
        // When `<tool_call>` contains valid JSON, the JSON
        // parser wins. The XML parser only fires after BOTH
        // JSON shapes fail.
        let text = r#"<tool_call>{"name": "fs.write", "arguments": {"path": "x"}}</tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(
            calls[0].inner_format, "json-name-arguments",
            "JSON shape wins priority over XML fallback"
        );
    }

    #[test]
    fn phase_127_inner_format_tagging_for_existing_shapes() {
        // Existing JSON shapes produce the expected
        // `inner_format` tags. Coverage check for the
        // Phase 127 field added to ExtractedToolCall.
        let text_a = r#"<tool_code>{"name": "x", "arguments": {}}</tool_code>"#;
        let text_b = r#"<tool_call>{"tool": "y", "parameters": {}}</tool_call>"#;
        assert_eq!(
            extract_tool_calls(text_a)[0].inner_format,
            "json-name-arguments"
        );
        assert_eq!(
            extract_tool_calls(text_b)[0].inner_format,
            "json-tool-parameters"
        );
    }

    #[test]
    fn phase_127_qwen3_coder_xml_with_surrounding_prose() {
        // The model often emits explanatory prose before
        // and after the tool call; the parser ignores
        // surrounding text outside the wrapper.
        let text = r#"
            Let me write that file for you.

            <tool_call>
            <function=fs.write>
            <parameter=path>test.txt</parameter>
            <parameter=content>phase 127</parameter>
            </function>
            </tool_call>

            That should do it.
        "#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[0].arguments["content"], "phase 127");
    }

    #[test]
    fn phase_127_qwen3_coder_xml_underscore_param_name() {
        // Parameter names can contain underscores — common
        // in tool schemas (e.g. `interval_minutes`).
        let text = r#"<tool_call><function=health.check.add><parameter=interval_minutes>10</parameter></function></tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["interval_minutes"], 10);
    }
}

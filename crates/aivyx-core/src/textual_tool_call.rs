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

/// Wrapper spec — open and close literals plus the audit-
/// facing identifier (`tag`) that lands in
/// `ExtractedToolCall.wrapper_tag`. The open/close are
/// independent literals because some wrappers have
/// asymmetric forms; Phi-4-mini's `<|tool_call|>` /
/// `<|/tool_call|>` (slash INSIDE the bars, not before
/// them) can't be derived from a single name string.
struct WrapperSpec {
    /// Identifier that lands in
    /// `ExtractedToolCall.wrapper_tag`. Kept stable so
    /// auditors can grep on it across the chain.
    tag: &'static str,
    open: &'static str,
    close: &'static str,
}

/// Wrappers recognized by this extractor, in priority
/// order for documentation. The extractor scans the text
/// once and picks whichever wrapper opens earliest in
/// the source; first-by-byte-offset wins.
const WRAPPERS: &[WrapperSpec] = &[
    WrapperSpec {
        tag: "tool_code",
        open: "<tool_code>",
        close: "</tool_code>",
    },
    WrapperSpec {
        tag: "tool_call",
        open: "<tool_call>",
        close: "</tool_call>",
    },
    // Phi-4-mini wrapper — JSON list inside special-token
    // bars. Inner shape is always a JSON array of
    // `{"name", "arguments"}` objects per Microsoft's
    // PhiCookBook and Ollama's phi4-mini modelfile.
    WrapperSpec {
        tag: "|tool_call|",
        open: "<|tool_call|>",
        close: "<|/tool_call|>",
    },
    // Gemma 3 markdown fence — `\`\`\`tool_code` open,
    // `\`\`\`` close. Inner is Python-call syntax
    // (`func.name(k1=v1, k2=v2)`), which the
    // `parse_python_call` parser translates into JSON
    // arguments. Multiple calls per fence are extracted
    // independently. Falls back to JSON shapes if the
    // inner isn't valid Python-call syntax (covers
    // operators who paste JSON inside `tool_code` fences).
    //
    // Tag is `"tool_code_fence"` (not `"tool_code"`) so
    // auditors can grep cleanly against the bare
    // `<tool_code>` HTML-style wrapper.
    WrapperSpec {
        tag: "tool_code_fence",
        open: "```tool_code",
        close: "```",
    },
];

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
///
/// Most wrappers produce 0 or 1 call. The Phi-4-mini
/// wrapper (`<|tool_call|>...<|/tool_call|>`) can produce
/// N calls when the model emits a JSON list of parallel
/// tool invocations.
pub fn extract_tool_calls(text: &str) -> Vec<ExtractedToolCall> {
    extract_tool_calls_with_hint(text, None)
}

/// Phase 127 Task 6 — extraction with a family-hint
/// prioritization signal.
///
/// `family_hint` is the model's training-family identifier
/// (typically what Ollama's `/api/show` returns as
/// `details.family`). The hint biases the inner-shape
/// priority order for wrappers that accept multiple inner
/// shapes — specifically:
///
/// - Qwen-family hints (`"qwen35"`, `"qwen3"`,
///   `"qwen3-coder"`, anything beginning with `"qwen"`)
///   cause `<tool_call>` content to try Qwen3-Coder XML
///   FIRST instead of JSON. For valid inputs this doesn't
///   change behavior (the parsers don't overlap on
///   well-formed text), but it's load-bearing for
///   malformed-but-recoverable inputs and communicates
///   intent for future parser additions.
/// - Other family hints (gemma, phi, llama, mistral) use
///   the default order today; reserved for future
///   per-family bias when overlapping parsers ship.
/// - `None` or unrecognized families use the default
///   order. This is what callers without family
///   information (or non-Ollama providers) pass.
///
/// The hint is a HINT, not a contract — every parser is
/// still tried for every wrapper match, just in a
/// reordered priority. This matches the "permissive
/// fallback" posture: if the family-preferred parser
/// misses, the other parsers still get their chance.
pub fn extract_tool_calls_with_hint(
    text: &str,
    family_hint: Option<&str>,
) -> Vec<ExtractedToolCall> {
    let bias = classify_family(family_hint);
    let mut out: Vec<ExtractedToolCall> = Vec::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let Some(open_match) = find_next_open(text, cursor) else {
            break;
        };
        let (lt_pos, spec, content_start) = open_match;
        let Some(close_rel) = text[content_start..].find(spec.close) else {
            // Unclosed wrapper — skip past this `<` so we
            // don't loop forever.
            cursor = lt_pos + 1;
            continue;
        };
        let close_abs = content_start + close_rel;
        let inner = &text[content_start..close_abs];
        out.extend(parse_inner(inner.trim(), spec.tag, bias));
        cursor = close_abs + spec.close.len();
    }

    // Phase 127 Task 5 — bare-JSON fallback. Runs ONLY
    // when no wrapper-based extraction matched. The guard
    // is load-bearing: bare-JSON detection is the highest
    // false-positive risk path in the substrate because
    // operators (and models in prose) frequently mention
    // JSON inline. The guard requires the entire response
    // content (after optional leading `<think>` block) to
    // be exactly one top-level JSON object matching a
    // tool-call shape — JSON embedded in prose, JSON
    // followed by prose, or multiple concatenated JSON
    // objects all fail to extract.
    if out.is_empty() {
        if let Some(call) = try_bare_json(text) {
            out.push(call);
        }
    }
    out
}

/// Family-hint classification. Maps the loose family
/// strings Ollama returns into a small set of bias enum
/// values the parsers can dispatch on. Unknown families
/// (including `None`) map to `Default`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FamilyBias {
    /// Default order — JSON shapes first inside wrappers
    /// that accept multiple inner shapes. Applies to
    /// `None`, unknown families, and explicitly cloud-
    /// family-flavored providers (anthropic, openai).
    Default,
    /// Qwen family — prefer Qwen3-Coder XML inside
    /// `<tool_call>` over JSON shapes. Matches the
    /// empirical training format for qwen3.5+
    /// (per Ollama issues #14493 / #14745).
    QwenCoder,
}

fn classify_family(hint: Option<&str>) -> FamilyBias {
    let Some(raw) = hint else {
        return FamilyBias::Default;
    };
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.starts_with("qwen") {
        FamilyBias::QwenCoder
    } else {
        FamilyBias::Default
    }
}

/// Bare-JSON fallback. Returns `Some(call)` only if the
/// response content — after trimming whitespace and an
/// optional leading `<think>...</think>` thinking-mode
/// prefix — is exactly one top-level JSON object
/// matching either of the tool-call shapes
/// (`{name, arguments}` or `{tool, parameters}`).
///
/// `wrapper_tag` is `"(bare)"` to communicate "no wrapper
/// detected" cleanly in audit dumps.
fn try_bare_json(text: &str) -> Option<ExtractedToolCall> {
    let body = strip_leading_think_block(text).trim();
    if body.is_empty() {
        return None;
    }
    // Strict: parse the whole body. If there's trailing
    // text after the JSON object, this returns Err and
    // we drop — the load-bearing FP guard.
    let value: Value = serde_json::from_str(body).ok()?;
    if !value.is_object() {
        return None;
    }
    if let Some(call) = parse_json_name_arguments(&value, "(bare)", "json-name-arguments") {
        return Some(call);
    }
    parse_json_tool_parameters(&value, "(bare)")
}

/// If `text`, after leading-whitespace trimming, begins
/// with `<think>` and the block is closed with
/// `</think>`, return the suffix starting after
/// `</think>`. Otherwise return `text` unchanged. Only
/// the first leading block is stripped — interior
/// thinking blocks are left for the JSON parser to
/// reject naturally.
fn strip_leading_think_block(text: &str) -> &str {
    let trimmed = text.trim_start();
    let open = "<think>";
    let close = "</think>";
    let Some(rest) = trimmed.strip_prefix(open) else {
        return text;
    };
    let Some(close_rel) = rest.find(close) else {
        // Unclosed `<think>` — don't strip; let parser
        // fail naturally.
        return text;
    };
    &rest[close_rel + close.len()..]
}

/// Find the next opening wrapper in `text[start..]`. Returns
/// `(byte_offset_of_open_start, spec, byte_offset_after_open)`
/// or `None`.
fn find_next_open(text: &str, start: usize) -> Option<(usize, &'static WrapperSpec, usize)> {
    let mut best: Option<(usize, &'static WrapperSpec, usize)> = None;
    for spec in WRAPPERS {
        if let Some(rel) = text[start..].find(spec.open) {
            let abs = start + rel;
            let after = abs + spec.open.len();
            best = Some(match best {
                None => (abs, spec, after),
                Some((prev_abs, _, _)) if abs < prev_abs => (abs, spec, after),
                Some(prev) => prev,
            });
        }
    }
    best
}

/// Parse the inner content of a wrapper block. Inner
/// shape is dispatched per-wrapper, with `bias` reordering
/// the inner-shape priority for wrappers that accept
/// multiple shapes:
///
/// - `<|tool_call|>` (Phi-4-mini) — JSON list of
///   `{"name", "arguments"}` objects. Single shape; bias
///   has no effect.
/// - `<tool_code>` — JSON `{"name", "arguments"}` then
///   `{"tool", "parameters"}`. Bias has no effect today
///   (no XML or python-call shape accepted on this
///   wrapper).
/// - `<tool_call>` — JSON shapes and Qwen3-Coder XML.
///   `QwenCoder` bias tries XML FIRST; `Default` tries
///   JSON first.
/// - `tool_code_fence` (Gemma 3 markdown) — Python-call
///   first, JSON fallback. Single primary shape; bias
///   has no effect.
///
/// First match wins per-element; unmatched shapes drop
/// silently.
fn parse_inner(inner: &str, wrapper_tag: &str, bias: FamilyBias) -> Vec<ExtractedToolCall> {
    let mut out = Vec::new();
    if inner.is_empty() {
        return out;
    }

    // Phi-4-mini list-wrapper has its own shape and does
    // NOT fall back to single-object JSON or XML. Keeping
    // it scoped makes the wrapper_tag distinction
    // meaningful for audit.
    if wrapper_tag == "|tool_call|" {
        if let Ok(arr) = serde_json::from_str::<Vec<Value>>(inner) {
            for elem in arr {
                if let Some(call) =
                    parse_json_name_arguments(&elem, wrapper_tag, "json-list-name-arguments")
                {
                    out.push(call);
                }
            }
        }
        return out;
    }

    // Gemma 3 markdown-fence wrapper: try Python-call
    // syntax first (the training format), then fall back
    // to JSON shapes so operators who pasted JSON inside
    // a `tool_code` fence still extract.
    if wrapper_tag == "tool_code_fence" {
        let py_calls = parse_python_call(inner, wrapper_tag);
        if !py_calls.is_empty() {
            return py_calls;
        }
        // Fall through to the JSON path below.
    }

    // `<tool_call>` with QwenCoder bias: try XML first.
    // The XML parser only matches genuine Qwen3-Coder
    // format (`<function=...>`); JSON content doesn't
    // satisfy it. So the bias matters only for inputs
    // that ARE Qwen3-Coder XML — for JSON content the
    // result is identical to default order, just with one
    // extra failed XML attempt.
    if wrapper_tag == "tool_call" && bias == FamilyBias::QwenCoder {
        if let Some(call) = parse_qwen3_coder_xml(inner, wrapper_tag) {
            out.push(call);
            return out;
        }
    }

    // JSON shapes for `<tool_code>` and `<tool_call>` (and
    // for `tool_code_fence` fall-through).
    if let Ok(value) = serde_json::from_str::<Value>(inner) {
        if let Some(call) = parse_json_name_arguments(&value, wrapper_tag, "json-name-arguments") {
            out.push(call);
            return out;
        }
        if let Some(call) = parse_json_tool_parameters(&value, wrapper_tag) {
            out.push(call);
            return out;
        }
    }

    // Fall back to Qwen3-Coder XML inside `<tool_call>`
    // when the QwenCoder bias didn't already try it.
    // `<tool_code>` is the markdown-fence-like form used
    // for JSON-shape emission per the empirical literature
    // — keeping the XML parser scoped to `<tool_call>`
    // avoids false-positive XML matches inside other
    // wrapper kinds.
    if wrapper_tag == "tool_call" && bias != FamilyBias::QwenCoder {
        if let Some(call) = parse_qwen3_coder_xml(inner, wrapper_tag) {
            out.push(call);
        }
    }

    out
}

/// Match the `{"name", "arguments"}` JSON shape on a
/// single `Value`. Returns `None` if the value isn't an
/// object with both keys, or if the name is empty/non-
/// string. `inner_format` is the audit tag the caller
/// wants ascribed (it differs between the single-object
/// path and the JSON-list path so the audit chain can
/// distinguish them).
fn parse_json_name_arguments(
    value: &Value,
    wrapper_tag: &str,
    inner_format: &str,
) -> Option<ExtractedToolCall> {
    let obj = value.as_object()?;
    let Value::String(name) = obj.get("name")? else {
        return None;
    };
    let args = obj.get("arguments")?;
    if name.trim().is_empty() {
        return None;
    }
    Some(ExtractedToolCall {
        tool_name: name.trim().to_string(),
        arguments: args.clone(),
        wrapper_tag: wrapper_tag.to_string(),
        inner_format: inner_format.to_string(),
    })
}

/// Match the `{"tool", "parameters"}` JSON shape on a
/// single `Value`. Same `None` conditions as the
/// `name`/`arguments` matcher.
fn parse_json_tool_parameters(value: &Value, wrapper_tag: &str) -> Option<ExtractedToolCall> {
    let obj = value.as_object()?;
    let Value::String(tool) = obj.get("tool")? else {
        return None;
    };
    let params = obj.get("parameters")?;
    if tool.trim().is_empty() {
        return None;
    }
    Some(ExtractedToolCall {
        tool_name: tool.trim().to_string(),
        arguments: params.clone(),
        wrapper_tag: wrapper_tag.to_string(),
        inner_format: "json-tool-parameters".to_string(),
    })
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

// ========================================================
// Phase 127 Task 4 — Gemma 3 Python-call parser.
//
// Gemma 3 (per Google's "function calling with Gemma"
// docs) emits tool invocations as Python expression
// syntax inside a ```tool_code``` markdown fence:
//
//   ```tool_code
//   fs.write(path='test.txt', content='hi')
//   ```
//
// This parser walks the inner with a small hand-written
// recursive-descent grammar (no Python AST dependency).
// One or more calls per fence are supported; each call
// becomes an `ExtractedToolCall` with arguments translated
// from Python kwargs into a JSON object. Unknown
// identifiers, malformed syntax, and unterminated strings
// all silently drop the offending call — matching the
// Phase 126 permissive-parse posture.
//
// Supported value forms:
//   - 'single-quoted' / "double-quoted" strings (with
//     `\\`, `\'`, `\"`, `\n`, `\t`, `\r` escapes)
//   - Integers (`42`, `-3`) and floats (`0.5`, `-1.0`,
//     `1e3`)
//   - Booleans (`True`, `False`) → JSON true/false
//   - `None` → JSON null
//   - Lists (`[v, ...]`) and dicts (`{"k": v, ...}`)
//     recursing on value
//
// Known limitations of the MVP:
//   - Dict keys must be STRING literals (Python allows
//     numeric/bool keys; tool kwargs realistically never
//     do).
//   - No bytestring (`b'...'`), no raw string (`r'...'`),
//     no f-string. These don't appear in tool-call
//     emissions.
//   - No identifier values — only literals. A model
//     emitting `func(arg=variable)` drops; tool args are
//     always literal.
// ========================================================

/// Parse the inner of a `\`\`\`tool_code` fence as a
/// sequence of Python-style function calls. Returns one
/// `ExtractedToolCall` per parsed call; an empty Vec means
/// the inner didn't match the grammar (the caller falls
/// back to alternative parsers).
fn parse_python_call(inner: &str, wrapper_tag: &str) -> Vec<ExtractedToolCall> {
    let mut out = Vec::new();
    let mut p = PyParser::new(inner);
    loop {
        p.skip_whitespace_and_separators();
        if p.at_end() {
            break;
        }
        match p.parse_call() {
            Some((name, args)) => {
                out.push(ExtractedToolCall {
                    tool_name: name,
                    arguments: Value::Object(args),
                    wrapper_tag: wrapper_tag.to_string(),
                    inner_format: "python-call".to_string(),
                });
            }
            // Couldn't parse from this position — bail.
            // Previously-parsed calls are kept; the parser
            // is permissive (drops only what didn't parse).
            None => break,
        }
    }
    out
}

struct PyParser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> PyParser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// Like `skip_whitespace` but also consumes `;`
    /// separators between calls. Gemma 3 typically uses
    /// newlines between calls but we tolerate semicolons
    /// for robustness.
    fn skip_whitespace_and_separators(&mut self) {
        loop {
            let before = self.pos;
            self.skip_whitespace();
            while self.peek() == Some(b';') {
                self.pos += 1;
            }
            if self.pos == before {
                break;
            }
        }
    }

    fn match_byte(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// `IDENT := [A-Za-z_][A-Za-z_0-9]*`
    fn parse_ident(&mut self) -> Option<String> {
        let start = self.pos;
        let first = self.peek()?;
        if !(first.is_ascii_alphabetic() || first == b'_') {
            return None;
        }
        self.pos += 1;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        Some(
            std::str::from_utf8(&self.src[start..self.pos])
                .ok()?
                .to_string(),
        )
    }

    /// `DOTTED := IDENT ('.' IDENT)*`
    fn parse_dotted_ident(&mut self) -> Option<String> {
        let start = self.pos;
        self.parse_ident()?;
        loop {
            let save = self.pos;
            if self.peek() != Some(b'.') {
                break;
            }
            self.pos += 1;
            if self.parse_ident().is_none() {
                self.pos = save;
                break;
            }
        }
        Some(
            std::str::from_utf8(&self.src[start..self.pos])
                .ok()?
                .to_string(),
        )
    }

    /// `CALL := DOTTED '(' [ARGS] ')'`. Returns
    /// `(name, kwargs_map)`.
    fn parse_call(&mut self) -> Option<(String, serde_json::Map<String, Value>)> {
        let name = self.parse_dotted_ident()?;
        self.skip_whitespace();
        if !self.match_byte(b'(') {
            return None;
        }
        let mut args = serde_json::Map::<String, Value>::new();
        loop {
            self.skip_whitespace();
            if self.match_byte(b')') {
                break;
            }
            let key = self.parse_ident()?;
            self.skip_whitespace();
            if !self.match_byte(b'=') {
                return None;
            }
            self.skip_whitespace();
            let val = self.parse_value()?;
            args.insert(key, val);
            self.skip_whitespace();
            if self.match_byte(b',') {
                continue;
            }
            if self.match_byte(b')') {
                break;
            }
            // Neither `,` nor `)` — syntax error.
            return None;
        }
        Some((name, args))
    }

    /// `VALUE := STRING | NUMBER | BOOL | NONE | LIST | DICT`
    fn parse_value(&mut self) -> Option<Value> {
        self.skip_whitespace();
        let c = self.peek()?;
        match c {
            b'"' | b'\'' => self.parse_string(),
            b'[' => self.parse_list(),
            b'{' => self.parse_dict(),
            b'-' | b'+' => self.parse_number(),
            d if d.is_ascii_digit() => self.parse_number(),
            a if a.is_ascii_alphabetic() || a == b'_' => self.parse_keyword(),
            _ => None,
        }
    }

    /// `STRING := ' ... ' | " ... "` with backslash escapes
    /// (`\\`, `\'`, `\"`, `\n`, `\t`, `\r`). Unknown escape
    /// sequences are preserved verbatim (`\x` → `\x`) so
    /// the parser doesn't silently corrupt operator
    /// payloads.
    fn parse_string(&mut self) -> Option<Value> {
        let quote = self.advance()?;
        if quote != b'"' && quote != b'\'' {
            return None;
        }
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let c = self.advance()?;
            if c == quote {
                let s = String::from_utf8(buf).ok()?;
                return Some(Value::String(s));
            }
            if c == b'\\' {
                let next = self.advance()?;
                match next {
                    b'\\' => buf.push(b'\\'),
                    b'\'' => buf.push(b'\''),
                    b'"' => buf.push(b'"'),
                    b'n' => buf.push(b'\n'),
                    b't' => buf.push(b'\t'),
                    b'r' => buf.push(b'\r'),
                    other => {
                        buf.push(b'\\');
                        buf.push(other);
                    }
                }
            } else {
                buf.push(c);
            }
        }
    }

    /// `NUMBER := [+-]? DIGITS ( '.' DIGITS )? ( [eE] [+-]? DIGITS )?`
    /// (Note: `.5` without leading digit is NOT supported.
    /// Gemma 3 emits canonical numbers; the model would
    /// have to be unusual to emit `.5`.)
    fn parse_number(&mut self) -> Option<Value> {
        let start = self.pos;
        if matches!(self.peek(), Some(b'+') | Some(b'-')) {
            self.pos += 1;
        }
        let int_start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.pos == int_start {
            // No digits after the sign — not a number.
            return None;
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') {
            is_float = true;
            self.pos += 1;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            let exp_start = self.pos;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
            if self.pos == exp_start {
                // Trailing exponent with no digits — invalid.
                return None;
            }
        }
        let s = std::str::from_utf8(&self.src[start..self.pos]).ok()?;
        if is_float {
            let f = s.parse::<f64>().ok()?;
            serde_json::Number::from_f64(f).map(Value::Number)
        } else {
            s.parse::<i64>().ok().map(|i| Value::Number(i.into()))
        }
    }

    /// `BOOL | NONE`. Any other identifier is rejected
    /// (Python supports identifier expressions but tool
    /// kwargs realistically only use literals).
    fn parse_keyword(&mut self) -> Option<Value> {
        let start = self.pos;
        let ident = self.parse_ident()?;
        match ident.as_str() {
            "True" => Some(Value::Bool(true)),
            "False" => Some(Value::Bool(false)),
            "None" => Some(Value::Null),
            _ => {
                // Not a recognized literal; rewind so the
                // outer parser fails cleanly.
                self.pos = start;
                None
            }
        }
    }

    /// `LIST := '[' [VALUE (',' VALUE)*] ']'`
    fn parse_list(&mut self) -> Option<Value> {
        if !self.match_byte(b'[') {
            return None;
        }
        let mut elements: Vec<Value> = Vec::new();
        loop {
            self.skip_whitespace();
            if self.match_byte(b']') {
                break;
            }
            let v = self.parse_value()?;
            elements.push(v);
            self.skip_whitespace();
            if self.match_byte(b',') {
                continue;
            }
            if self.match_byte(b']') {
                break;
            }
            return None;
        }
        Some(Value::Array(elements))
    }

    /// `DICT := '{' [STRING ':' VALUE (',' STRING ':' VALUE)*] '}'`.
    /// Keys must be string literals.
    fn parse_dict(&mut self) -> Option<Value> {
        if !self.match_byte(b'{') {
            return None;
        }
        let mut entries = serde_json::Map::<String, Value>::new();
        loop {
            self.skip_whitespace();
            if self.match_byte(b'}') {
                break;
            }
            let key_val = self.parse_string()?;
            let Value::String(key) = key_val else {
                return None;
            };
            self.skip_whitespace();
            if !self.match_byte(b':') {
                return None;
            }
            self.skip_whitespace();
            let val = self.parse_value()?;
            entries.insert(key, val);
            self.skip_whitespace();
            if self.match_byte(b',') {
                continue;
            }
            if self.match_byte(b'}') {
                break;
            }
            return None;
        }
        Some(Value::Object(entries))
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
        assert!(calls.is_empty(), "unclosed wrapper produces no extraction");
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
        let text =
            "<tool_code>\n  \n  {\"name\": \"fs.read\", \"arguments\": {}}\n  \n</tool_code>";
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
        let text = r#"<tool_call><function=time.now></function></tool_call>"#;
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
            calls[0].arguments["path"], "  spaced.txt  ",
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
        assert!(calls.is_empty(), "empty function name dropped");
    }

    #[test]
    fn phase_127_qwen3_coder_xml_only_inside_tool_call_wrapper() {
        // Same XML inner inside `<tool_code>` MUST NOT
        // extract — Phase 127's parser is restricted to
        // `<tool_call>` per the empirical literature.
        let text =
            r#"<tool_code><function=fs.write><parameter=path>x</parameter></function></tool_code>"#;
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

    // ====================================================
    // Phase 127 Task 3 — Phi-4-mini `<|tool_call|>` wrapper.
    //
    // Format per Microsoft's PhiCookBook + Ollama's
    // phi4-mini modelfile template:
    //
    //   <|tool_call|>[{"name":"fn1","arguments":{...}},
    //                 {"name":"fn2","arguments":{...}}]<|/tool_call|>
    //
    // Inner is ALWAYS a JSON array (even for single calls),
    // so the wrapper produces 0..N ExtractedToolCalls per
    // block. The wrapper open and close are asymmetric —
    // `<|tool_call|>` open vs `<|/tool_call|>` close
    // (slash INSIDE the bars, not before them).
    //
    // `wrapper_tag` is the literal `"|tool_call|"` (with
    // bars) so auditors can distinguish from the bare
    // `<tool_call>` wrapper grep-cleanly.
    // ====================================================

    #[test]
    fn phase_127_phi4_mini_single_call_in_list() {
        let text =
            r#"<|tool_call|>[{"name": "fs.write", "arguments": {"path": "x.txt"}}]<|/tool_call|>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[0].wrapper_tag, "|tool_call|");
        assert_eq!(calls[0].inner_format, "json-list-name-arguments");
        assert_eq!(calls[0].arguments["path"], "x.txt");
    }

    #[test]
    fn phase_127_phi4_mini_batch_calls_in_list() {
        // Phi-4-mini's parallel-call form — multiple
        // function invocations in the same list.
        let text = r#"<|tool_call|>[
            {"name": "fs.write", "arguments": {"path": "a.txt", "content": "1"}},
            {"name": "fs.write", "arguments": {"path": "b.txt", "content": "2"}},
            {"name": "memory.read", "arguments": {"topic": "x"}}
        ]<|/tool_call|>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[0].arguments["path"], "a.txt");
        assert_eq!(calls[1].tool_name, "fs.write");
        assert_eq!(calls[1].arguments["path"], "b.txt");
        assert_eq!(calls[2].tool_name, "memory.read");
        assert_eq!(
            calls[2].inner_format, "json-list-name-arguments",
            "every list element gets the same inner_format tag"
        );
    }

    #[test]
    fn phase_127_phi4_mini_empty_list_returns_no_calls() {
        // Empty list is syntactically valid but produces
        // zero extractions — Phi-4-mini may emit this when
        // the model decides not to call any tool but its
        // chat template still includes the wrapper.
        let text = r#"<|tool_call|>[]<|/tool_call|>"#;
        let calls = extract_tool_calls(text);
        assert!(calls.is_empty());
    }

    #[test]
    fn phase_127_phi4_mini_single_object_not_a_list_drops() {
        // `<|tool_call|>` is strictly list-shaped per the
        // Phi-4-mini training format. A single-object inner
        // (no array wrapper) should NOT extract — that
        // emission would be a `<tool_call>{...}</tool_call>`
        // case, not a Phi-4-mini case.
        let text = r#"<|tool_call|>{"name": "fs.write", "arguments": {}}<|/tool_call|>"#;
        let calls = extract_tool_calls(text);
        assert!(
            calls.is_empty(),
            "Phi-4-mini wrapper only accepts JSON-list inner; single object drops"
        );
    }

    #[test]
    fn phase_127_phi4_mini_malformed_json_drops() {
        let text = r#"<|tool_call|>not valid json<|/tool_call|>"#;
        let calls = extract_tool_calls(text);
        assert!(calls.is_empty());
    }

    #[test]
    fn phase_127_phi4_mini_list_element_missing_shape_skipped() {
        // List with mixed valid + invalid elements:
        // valid elements extract, invalid ones are skipped
        // (matches the Phase 126 permissive-parse posture).
        let text = r#"<|tool_call|>[
            {"name": "fs.write", "arguments": {"path": "a"}},
            {"name": "", "arguments": {}},
            {"name": "memory.read", "arguments": {"topic": "x"}}
        ]<|/tool_call|>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(
            calls.len(),
            2,
            "empty-name element skipped; surrounding valid elements still extract"
        );
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[1].tool_name, "memory.read");
    }

    #[test]
    fn phase_127_phi4_mini_mixed_with_other_wrappers_in_source_order() {
        // A response can include both a Phi-4-mini list and
        // a standard `<tool_call>` JSON block (e.g. when an
        // operator pipes a model behind a bridge that
        // emulates Phi-4-mini for one tool and standard for
        // another). Both extract in source order.
        let text = r#"<|tool_call|>[{"name": "a.x", "arguments": {}}]<|/tool_call|> then <tool_call>{"name": "b.y", "arguments": {}}</tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].wrapper_tag, "|tool_call|");
        assert_eq!(calls[0].tool_name, "a.x");
        assert_eq!(calls[1].wrapper_tag, "tool_call");
        assert_eq!(calls[1].tool_name, "b.y");
    }

    #[test]
    fn phase_127_phi4_mini_whitespace_tolerance() {
        let text = r#"<|tool_call|>
            [
                {"name": "fs.write", "arguments": {"path": "x"}}
            ]
        <|/tool_call|>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
    }

    #[test]
    fn phase_127_phi4_mini_unclosed_wrapper_drops() {
        // `<|tool_call|>` opens but no `<|/tool_call|>`
        // close — skip past the open and continue scanning.
        let text = r#"<|tool_call|>[{"name": "fs.write", "arguments": {}}]"#;
        let calls = extract_tool_calls(text);
        assert!(
            calls.is_empty(),
            "unclosed Phi-4-mini wrapper is dropped, matching the Phase 126 unclosed-wrapper posture"
        );
    }

    #[test]
    fn phase_127_phi4_mini_does_not_match_standard_tool_call_wrapper() {
        // Sanity check: a bare `<tool_call>` block does
        // NOT trip the Phi-4-mini path even though both
        // share the substring "tool_call". The wrapper
        // literals are distinct (`<tool_call>` vs
        // `<|tool_call|>`) and matched verbatim.
        let text = r#"<tool_call>{"name": "fs.write", "arguments": {"path": "x"}}</tool_call>"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].wrapper_tag, "tool_call",
            "bare-bracket tool_call wrapper resolves to the standard tag, not Phi-4-mini"
        );
        assert_eq!(
            calls[0].inner_format, "json-name-arguments",
            "and uses the single-object JSON path, not the list path"
        );
    }

    // ====================================================
    // Phase 127 Task 4 — Gemma 3 ```tool_code` python-fence
    // wrapper + Python-call inner-format parser.
    //
    // Gemma 3 emits tool calls as Python expression syntax
    // inside a markdown `tool_code` fence:
    //
    //   ```tool_code
    //   fs.write(path='test.txt', content='hi')
    //   ```
    //
    // The wrapper tag is `"tool_code_fence"` (distinct from
    // the bare `<tool_code>` wrapper) and the inner_format
    // is `"python-call"`.
    // ====================================================

    #[test]
    fn phase_127_gemma3_python_fence_single_call() {
        let text = "```tool_code\nfs.write(path='test.txt', content='hi')\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[0].wrapper_tag, "tool_code_fence");
        assert_eq!(calls[0].inner_format, "python-call");
        assert_eq!(calls[0].arguments["path"], "test.txt");
        assert_eq!(calls[0].arguments["content"], "hi");
    }

    #[test]
    fn phase_127_gemma3_python_fence_no_args() {
        let text = "```tool_code\ntime.now()\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "time.now");
        assert_eq!(calls[0].arguments, json!({}));
    }

    #[test]
    fn phase_127_gemma3_python_fence_int_arg() {
        let text = "```tool_code\ntask.set(count=42)\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["count"], 42);
    }

    #[test]
    fn phase_127_gemma3_python_fence_negative_int() {
        let text = "```tool_code\nmath.shift(offset=-3)\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["offset"], -3);
    }

    #[test]
    fn phase_127_gemma3_python_fence_float_arg() {
        let text = "```tool_code\ntask.set(ratio=0.75)\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["ratio"].as_f64(), Some(0.75));
    }

    #[test]
    fn phase_127_gemma3_python_fence_float_with_exponent() {
        let text = "```tool_code\nfn(big=1.5e3)\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["big"].as_f64(), Some(1500.0));
    }

    #[test]
    fn phase_127_gemma3_python_fence_bool_true() {
        let text = "```tool_code\nfeature.set(enabled=True)\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["enabled"], true);
    }

    #[test]
    fn phase_127_gemma3_python_fence_bool_false() {
        let text = "```tool_code\nfeature.set(enabled=False)\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["enabled"], false);
    }

    #[test]
    fn phase_127_gemma3_python_fence_none_value() {
        let text = "```tool_code\nmemory.read(topic=None)\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert!(calls[0].arguments["topic"].is_null());
    }

    #[test]
    fn phase_127_gemma3_python_fence_list_arg() {
        let text = "```tool_code\ntask.set(tags=[1, 2, 3])\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["tags"], json!([1, 2, 3]));
    }

    #[test]
    fn phase_127_gemma3_python_fence_nested_dict_arg() {
        let text = r#"```tool_code
task.set(meta={"size": 42, "owner": "alice"})
```"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["meta"]["size"], 42);
        assert_eq!(calls[0].arguments["meta"]["owner"], "alice");
    }

    #[test]
    fn phase_127_gemma3_python_fence_single_quoted_string() {
        let text = "```tool_code\nfs.write(path='single.txt')\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["path"], "single.txt");
    }

    #[test]
    fn phase_127_gemma3_python_fence_double_quoted_string() {
        let text = "```tool_code\nfs.write(path=\"double.txt\")\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["path"], "double.txt");
    }

    #[test]
    fn phase_127_gemma3_python_fence_string_with_escapes() {
        // Escape sequences inside string literals.
        let text = r#"```tool_code
fs.write(content='line one\nline two\t\\tabbed')
```"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].arguments["content"],
            "line one\nline two\t\\tabbed"
        );
    }

    #[test]
    fn phase_127_gemma3_python_fence_dotted_function_name() {
        // Three-level dotted name (`health.check.add`).
        let text =
            "```tool_code\nhealth.check.add(url='https://example.com', interval_minutes=15)\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "health.check.add");
        assert_eq!(calls[0].arguments["interval_minutes"], 15);
    }

    #[test]
    fn phase_127_gemma3_python_fence_multi_call_per_fence() {
        // Multiple calls inside one fence, separated by
        // newlines. Each becomes its own ExtractedToolCall.
        let text =
            "```tool_code\nfs.write(path='a')\nfs.read(path='b')\nmemory.read(topic='c')\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[1].tool_name, "fs.read");
        assert_eq!(calls[2].tool_name, "memory.read");
    }

    #[test]
    fn phase_127_gemma3_python_fence_multi_call_with_semicolons() {
        // Tolerates `;` as a separator (Gemma 3 usually
        // uses newlines, but defensive).
        let text = "```tool_code\nfs.write(path='a'); fs.read(path='b')\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[1].tool_name, "fs.read");
    }

    #[test]
    fn phase_127_gemma3_python_fence_multiline_call() {
        // Python-style multi-line call with kwargs on
        // separate lines.
        let text = r#"```tool_code
fs.write(
    path='multiline.txt',
    content='hi'
)
```"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["path"], "multiline.txt");
    }

    #[test]
    fn phase_127_gemma3_python_fence_drops_unclosed_paren() {
        let text = "```tool_code\nfs.write(path='x'\n```";
        let calls = extract_tool_calls(text);
        assert!(calls.is_empty());
    }

    #[test]
    fn phase_127_gemma3_python_fence_drops_missing_equals() {
        let text = "```tool_code\nfs.write('positional-arg')\n```";
        let calls = extract_tool_calls(text);
        assert!(
            calls.is_empty(),
            "positional args (no `key=value`) drop — tool kwargs only"
        );
    }

    #[test]
    fn phase_127_gemma3_python_fence_drops_unmatched_quote() {
        let text = "```tool_code\nfs.write(path='unterminated)\n```";
        let calls = extract_tool_calls(text);
        assert!(calls.is_empty());
    }

    #[test]
    fn phase_127_gemma3_python_fence_drops_identifier_value() {
        // Identifier values (variables) aren't supported —
        // tool kwargs realistically only use literals. A
        // call with an identifier value drops.
        let text = "```tool_code\nfs.write(path=variable_ref)\n```";
        let calls = extract_tool_calls(text);
        assert!(calls.is_empty());
    }

    #[test]
    fn phase_127_gemma3_python_fence_empty_list_and_dict() {
        let text = "```tool_code\nfn(tags=[], meta={})\n```";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["tags"], json!([]));
        assert_eq!(calls[0].arguments["meta"], json!({}));
    }

    #[test]
    fn phase_127_gemma3_python_fence_falls_back_to_json() {
        // Operators sometimes paste JSON into a tool_code
        // fence; the wrapper falls back to JSON shapes
        // when Python-call doesn't match.
        let text = r#"```tool_code
{"name": "fs.write", "arguments": {"path": "x"}}
```"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(
            calls[0].wrapper_tag, "tool_code_fence",
            "wrapper_tag preserves the fence variant even when JSON-shape inner matches"
        );
        assert_eq!(calls[0].inner_format, "json-name-arguments");
    }

    #[test]
    fn phase_127_gemma3_python_fence_only_extracts_tool_code_language() {
        // `\`\`\`python` (or any other language tag) does
        // NOT extract — only `\`\`\`tool_code` is the
        // Gemma 3 protocol fence.
        let text = "```python\nfs.write(path='x')\n```";
        let calls = extract_tool_calls(text);
        assert!(
            calls.is_empty(),
            "only `tool_code` language fence triggers Python-call extraction"
        );
    }

    #[test]
    fn phase_127_gemma3_python_fence_with_surrounding_prose() {
        let text = "Let me save that for you.\n\n```tool_code\nfs.write(path='note.txt', content='saved')\n```\n\nDone.";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[0].arguments["content"], "saved");
    }

    // ====================================================
    // Phase 127 Task 5 — bare-JSON fallback with FP guard.
    //
    // Some Ollama models (qwen3:32b per issue #11662) emit
    // tool-call JSON with NO wrapper at all. The bare-JSON
    // path catches this — but only when the entire response
    // is exactly one JSON object (optionally preceded by a
    // `<think>...</think>` thinking-mode prefix). JSON
    // embedded in prose or followed by prose drops.
    //
    // `wrapper_tag` is `"(bare)"` so audit can distinguish.
    // ====================================================

    #[test]
    fn phase_127_bare_json_pure_extracts() {
        let text = r#"{"name": "fs.write", "arguments": {"path": "x.txt"}}"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[0].wrapper_tag, "(bare)");
        assert_eq!(calls[0].inner_format, "json-name-arguments");
        assert_eq!(calls[0].arguments["path"], "x.txt");
    }

    #[test]
    fn phase_127_bare_json_with_leading_whitespace() {
        let text = "   \n  {\"name\": \"fs.read\", \"arguments\": {}}";
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.read");
    }

    #[test]
    fn phase_127_bare_json_with_trailing_whitespace() {
        let text = r#"{"name": "fs.read", "arguments": {}}

        "#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.read");
    }

    #[test]
    fn phase_127_bare_json_after_think_block() {
        let text = r#"<think>
I should call fs.write to save that.
</think>
{"name": "fs.write", "arguments": {"path": "out.txt"}}"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[0].wrapper_tag, "(bare)");
    }

    #[test]
    fn phase_127_bare_json_tool_parameters_shape() {
        // The `{tool, parameters}` alternative shape also
        // extracts via the bare path.
        let text = r#"{"tool": "fs.write", "parameters": {"path": "x"}}"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[0].inner_format, "json-tool-parameters");
        assert_eq!(calls[0].wrapper_tag, "(bare)");
    }

    #[test]
    fn phase_127_bare_json_embedded_in_prose_drops() {
        // Load-bearing FP guard: model says "The answer is
        // {...}" with prose surrounding JSON — drop.
        let text =
            r#"The answer is {"name": "fs.write", "arguments": {}}, in case you were wondering."#;
        let calls = extract_tool_calls(text);
        assert!(
            calls.is_empty(),
            "JSON embedded in prose must NOT extract via bare path"
        );
    }

    #[test]
    fn phase_127_bare_json_followed_by_prose_drops() {
        let text = r#"{"name": "fs.write", "arguments": {}} — I think that's the right call."#;
        let calls = extract_tool_calls(text);
        assert!(
            calls.is_empty(),
            "trailing prose after JSON drops the extraction"
        );
    }

    #[test]
    fn phase_127_bare_json_preceded_by_prose_drops() {
        let text = r#"Here's the call: {"name": "fs.write", "arguments": {}}"#;
        let calls = extract_tool_calls(text);
        assert!(
            calls.is_empty(),
            "leading prose before JSON drops (think-block stripping won't trigger because prose isn't `<think>`)"
        );
    }

    #[test]
    fn phase_127_bare_json_wrong_shape_drops() {
        // Valid JSON but doesn't match either tool-call
        // shape.
        let text = r#"{"foo": "bar", "baz": 42}"#;
        let calls = extract_tool_calls(text);
        assert!(calls.is_empty());
    }

    #[test]
    fn phase_127_bare_json_empty_content_no_extraction() {
        assert!(extract_tool_calls("").is_empty());
        assert!(extract_tool_calls("   \n\t   ").is_empty());
    }

    #[test]
    fn phase_127_bare_json_just_think_no_payload_drops() {
        let text = "<think>\nI'm not sure what to call.\n</think>";
        let calls = extract_tool_calls(text);
        assert!(
            calls.is_empty(),
            "empty body after stripping <think> block drops"
        );
    }

    #[test]
    fn phase_127_bare_json_array_at_top_level_drops() {
        // A JSON array is NOT a tool-call shape on the
        // bare path. (Phi-4-mini's JSON-list emission goes
        // through the `<|tool_call|>` wrapper.)
        let text = r#"[{"name": "fs.write", "arguments": {}}]"#;
        let calls = extract_tool_calls(text);
        assert!(
            calls.is_empty(),
            "top-level JSON array does not extract via bare path"
        );
    }

    #[test]
    fn phase_127_bare_json_two_objects_concat_drops() {
        // Two JSON objects back-to-back — ambiguous; the
        // bare path requires exactly ONE top-level value.
        let text = r#"{"name":"a","arguments":{}}{"name":"b","arguments":{}}"#;
        let calls = extract_tool_calls(text);
        assert!(
            calls.is_empty(),
            "two concatenated JSON objects fail to parse as one value; drop"
        );
    }

    #[test]
    fn phase_127_bare_json_only_runs_when_wrappers_match_nothing() {
        // If ANY wrapper-based extraction succeeds, the
        // bare-JSON fallback does NOT run — even when the
        // text contains additional bare JSON elsewhere.
        // Bare-JSON is strictly a "no wrappers anywhere"
        // fallback.
        let text = r#"<tool_call>{"name":"wrapped","arguments":{}}</tool_call> Then {"name":"bare","arguments":{}}"#;
        let calls = extract_tool_calls(text);
        assert_eq!(
            calls.len(),
            1,
            "only the wrapped call extracts; bare suffix is ignored"
        );
        assert_eq!(calls[0].tool_name, "wrapped");
        assert_eq!(calls[0].wrapper_tag, "tool_call");
    }

    // ====================================================
    // Phase 127 Task 6 — family-hint architecture.
    //
    // `extract_tool_calls_with_hint(text, family_hint)`
    // exposes a family-hint signal that can bias inner-
    // shape priority for wrappers accepting multiple
    // shapes. Today this matters concretely for `<tool_call>`
    // (Qwen-family hints try XML first instead of JSON).
    // Other family hints (gemma, phi, llama, mistral) use
    // the default order today and are reserved for future
    // per-family bias.
    //
    // The hint is permissive — every parser is still tried;
    // family hint just reorders the priority. Behavior for
    // valid inputs is unchanged regardless of hint (parsers
    // don't ambiguously match well-formed content).
    // ====================================================

    #[test]
    fn phase_127_hint_qwen_family_extracts_xml_inside_tool_call() {
        // Same XML content as the Task 2 baseline test, but
        // routed via the hint-aware entry point with the
        // qwen35 family hint. Confirms the family-hint path
        // also produces the expected extraction.
        let text =
            r#"<tool_call><function=fs.write><parameter=path>x</parameter></function></tool_call>"#;
        let calls = extract_tool_calls_with_hint(text, Some("qwen35"));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[0].inner_format, "qwen3-coder-xml");
    }

    #[test]
    fn phase_127_hint_default_extracts_xml_inside_tool_call() {
        // Same XML content WITHOUT the family hint — the
        // fallback path still picks up XML (after JSON
        // shapes fail). Confirms the no-hint default
        // behavior is unchanged.
        let text =
            r#"<tool_call><function=fs.write><parameter=path>x</parameter></function></tool_call>"#;
        let calls = extract_tool_calls_with_hint(text, None);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(calls[0].inner_format, "qwen3-coder-xml");
    }

    #[test]
    fn phase_127_hint_qwen_family_json_still_works() {
        // JSON content inside `<tool_call>` with QwenCoder
        // hint: XML attempt fails, JSON shapes still match.
        // Confirms the hint is non-fatal — wrong-priority
        // is still recoverable via the other parsers.
        let text = r#"<tool_call>{"name": "fs.write", "arguments": {"path": "x"}}</tool_call>"#;
        let calls = extract_tool_calls_with_hint(text, Some("qwen35"));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "fs.write");
        assert_eq!(
            calls[0].inner_format, "json-name-arguments",
            "JSON shape matched after XML attempt failed under QwenCoder bias"
        );
    }

    #[test]
    fn phase_127_hint_unknown_family_uses_default_order() {
        // An unrecognized family string maps to Default
        // bias. Same extraction behavior as None.
        let text = r#"<tool_call>{"name": "fs.write", "arguments": {}}</tool_call>"#;
        let calls = extract_tool_calls_with_hint(text, Some("totally-made-up-model"));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].inner_format, "json-name-arguments");
    }

    #[test]
    fn phase_127_hint_empty_string_treated_as_default() {
        let text = r#"<tool_call>{"name": "fs.write", "arguments": {}}</tool_call>"#;
        let calls = extract_tool_calls_with_hint(text, Some(""));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].inner_format, "json-name-arguments");
    }

    #[test]
    fn phase_127_hint_qwen_variants_all_classify_as_qwen() {
        // All `qwen*` family strings route to QwenCoder
        // bias. Confirms the prefix-match classifier.
        let xml = r#"<tool_call><function=fs.write></function></tool_call>"#;
        for variant in ["qwen", "qwen3", "qwen35", "qwen3-coder", "Qwen35", "QWEN3"] {
            let calls = extract_tool_calls_with_hint(xml, Some(variant));
            assert_eq!(
                calls.len(),
                1,
                "variant {variant:?} should classify as Qwen and extract"
            );
            assert_eq!(calls[0].inner_format, "qwen3-coder-xml");
        }
    }

    #[test]
    fn phase_127_hint_legacy_extract_tool_calls_unchanged() {
        // The zero-hint shim (`extract_tool_calls`) must
        // produce identical results to
        // `extract_tool_calls_with_hint(text, None)`.
        let cases = [
            r#"<tool_call>{"name": "a", "arguments": {}}</tool_call>"#,
            r#"<tool_code>{"name": "b", "arguments": {}}</tool_code>"#,
            r#"<|tool_call|>[{"name": "c", "arguments": {}}]<|/tool_call|>"#,
            "```tool_code\nd.x()\n```",
            r#"{"name": "e", "arguments": {}}"#,
        ];
        for text in cases {
            let legacy = extract_tool_calls(text);
            let with_none = extract_tool_calls_with_hint(text, None);
            assert_eq!(
                legacy.len(),
                with_none.len(),
                "legacy entry must match no-hint variant for: {text}"
            );
            for (a, b) in legacy.iter().zip(with_none.iter()) {
                assert_eq!(a.tool_name, b.tool_name);
                assert_eq!(a.wrapper_tag, b.wrapper_tag);
                assert_eq!(a.inner_format, b.inner_format);
            }
        }
    }

    #[test]
    fn phase_127_bare_json_unclosed_think_block_falls_through() {
        // Unclosed `<think>` block — don't strip; the
        // remaining text isn't pure JSON, so we drop.
        let text = r#"<think>thinking forever {"name": "fs.write", "arguments": {}}"#;
        let calls = extract_tool_calls(text);
        assert!(calls.is_empty());
    }

    #[test]
    fn phase_127_gemma3_python_fence_mixed_value_types() {
        // The end-to-end test: a realistic call with one
        // of each value type the grammar supports.
        let text = r#"```tool_code
health.check.add(url='https://example.com', interval_minutes=15, enabled=True, alert=None, tags=['critical', 'oncall'], thresholds={"latency_ms": 500})
```"#;
        let calls = extract_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "health.check.add");
        assert_eq!(calls[0].arguments["url"], "https://example.com");
        assert_eq!(calls[0].arguments["interval_minutes"], 15);
        assert_eq!(calls[0].arguments["enabled"], true);
        assert!(calls[0].arguments["alert"].is_null());
        assert_eq!(calls[0].arguments["tags"], json!(["critical", "oncall"]));
        assert_eq!(calls[0].arguments["thresholds"]["latency_ms"], 500);
    }
}

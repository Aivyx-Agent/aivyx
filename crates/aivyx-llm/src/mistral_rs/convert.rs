//! Pure conversion between Aivyx's request/response
//! types and mistralrs's. Substrate-tier — no async,
//! no IO. Directly unit-testable without a model load.

use std::collections::HashMap;

use mistralrs::{Function, RequestBuilder, TextMessageRole, Tool, ToolChoice, ToolType};
use serde_json::Value;

use crate::{ContentBlock, LlmMessage, LlmToolDescriptor};

/// Flatten an Aivyx [`LlmMessage::User`] content vector
/// into a single text string. Phase 134 ships text-only;
/// image content blocks are stringified to a "[image]"
/// marker for now and surfaced as a known scope cap.
pub fn flatten_user_content(content: &[ContentBlock]) -> String {
    let mut out = String::new();
    for block in content {
        match block {
            ContentBlock::Text { text } => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(text);
            }
            ContentBlock::ImageBase64 { .. } => {
                // Phase 134 scope cap — images are stripped to
                // a placeholder. Phase 135+ adds real multimodal
                // support via mistralrs's image content type.
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("[image]");
            }
        }
    }
    out
}

/// Map an Aivyx [`LlmToolDescriptor`] to a mistralrs
/// [`Tool`]. The input schema is forwarded verbatim
/// because mistralrs expects the same JSON-Schema shape
/// Aivyx's tools already emit.
pub fn to_mistralrs_tool(desc: &LlmToolDescriptor) -> Result<Tool, String> {
    // mistralrs's Function::parameters is `HashMap<String, Value>`
    // (the top-level keys of the schema), not the wrapped object
    // Aivyx emits. Most Aivyx tool schemas already deserialize to
    // an object at the top level; extract the keys.
    let parameters = match &desc.input_schema {
        Value::Object(map) => {
            let hm: HashMap<String, Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            Some(hm)
        }
        // Non-object schemas don't fit mistralrs's Function shape.
        // Aivyx's tool registry validates input_schema is an
        // object at registration time, so this branch is
        // defensive.
        _ => {
            return Err(format!(
                "tool `{}` has a non-object input_schema; mistralrs requires object",
                desc.name
            ));
        }
    };
    Ok(Tool {
        tp: ToolType::Function,
        function: Function {
            description: Some(desc.description.clone()),
            name: desc.name.clone(),
            parameters,
        },
    })
}

/// Add a single Aivyx [`LlmMessage`] to a mistralrs
/// [`RequestBuilder`]. Returns the updated builder.
///
/// Tool-result messages map to mistralrs's
/// `add_tool_message` with the `call_id` preserved so
/// the conversation round-trips.
pub fn append_message_to_builder(
    builder: RequestBuilder,
    message: &LlmMessage,
) -> RequestBuilder {
    match message {
        LlmMessage::User { content } => {
            let text = flatten_user_content(content);
            builder.add_message(TextMessageRole::User, text)
        }
        LlmMessage::Assistant { text, tool_calls } => {
            if tool_calls.is_empty() {
                builder.add_message(TextMessageRole::Assistant, text.clone())
            } else {
                // The assistant emitted tool calls. mistralrs
                // wants the original ToolCallResponse shape on the
                // assistant message; we reconstruct from the
                // record we stored on the planner side.
                let mistralrs_calls: Vec<_> = tool_calls
                    .iter()
                    .enumerate()
                    .map(|(idx, tc)| mistralrs::ToolCallResponse {
                        index: idx,
                        id: tc.call_id.clone(),
                        tp: mistralrs::ToolCallType::Function,
                        function: mistralrs::CalledFunction {
                            name: tc.tool_name.clone(),
                            arguments: tc.input.to_string(),
                        },
                    })
                    .collect();
                builder.add_message_with_tool_call(
                    TextMessageRole::Assistant,
                    text.clone(),
                    mistralrs_calls,
                )
            }
        }
        LlmMessage::ToolResult {
            call_id, content, ..
        } => builder.add_tool_message(content.clone(), call_id.clone()),
    }
}

/// Apply Aivyx's tool catalog onto a mistralrs builder.
/// Empty tool list leaves the builder unchanged (the
/// LLM does plain text generation).
pub fn apply_tools(
    builder: RequestBuilder,
    tools: &[LlmToolDescriptor],
) -> Result<RequestBuilder, String> {
    if tools.is_empty() {
        return Ok(builder);
    }
    let converted: Result<Vec<_>, _> = tools.iter().map(to_mistralrs_tool).collect();
    Ok(builder.set_tools(converted?).set_tool_choice(ToolChoice::Auto))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LlmToolCallRecord;
    use serde_json::json;

    #[test]
    fn flatten_user_content_single_text() {
        let blocks = vec![ContentBlock::text("hello")];
        assert_eq!(flatten_user_content(&blocks), "hello");
    }

    #[test]
    fn flatten_user_content_multiple_text_joined_with_newline() {
        let blocks = vec![ContentBlock::text("part one"), ContentBlock::text("part two")];
        assert_eq!(flatten_user_content(&blocks), "part one\npart two");
    }

    #[test]
    fn flatten_user_content_image_becomes_placeholder() {
        let blocks = vec![
            ContentBlock::text("look at this:"),
            ContentBlock::image_from_bytes("image/png", b"fakebytes"),
        ];
        let out = flatten_user_content(&blocks);
        assert!(out.contains("look at this:"));
        assert!(out.contains("[image]"));
    }

    #[test]
    fn flatten_user_content_does_not_double_newline_when_input_ends_with_one() {
        let blocks = vec![ContentBlock::text("line\n"), ContentBlock::text("next")];
        assert_eq!(flatten_user_content(&blocks), "line\nnext");
    }

    #[test]
    fn flatten_user_content_empty_yields_empty_string() {
        assert_eq!(flatten_user_content(&[]), "");
    }

    #[test]
    fn to_mistralrs_tool_forwards_schema_verbatim() {
        let desc = LlmToolDescriptor {
            name: "fs.read".to_string(),
            description: "Read a file from disk".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                },
                "required": ["path"],
            }),
        };
        let tool = to_mistralrs_tool(&desc).expect("ok");
        assert_eq!(tool.function.name, "fs.read");
        assert_eq!(
            tool.function.description.as_deref(),
            Some("Read a file from disk")
        );
        let params = tool.function.parameters.as_ref().expect("present");
        assert!(params.contains_key("type"));
        assert!(params.contains_key("properties"));
        assert!(params.contains_key("required"));
    }

    #[test]
    fn to_mistralrs_tool_rejects_non_object_schema() {
        let desc = LlmToolDescriptor {
            name: "weird".to_string(),
            description: "schema isn't an object".to_string(),
            input_schema: json!("nope"),
        };
        let e = to_mistralrs_tool(&desc).expect_err("must error");
        assert!(e.contains("weird"), "{e}");
        assert!(e.contains("object"), "{e}");
    }

    #[test]
    fn apply_tools_empty_is_noop() {
        let builder = RequestBuilder::new();
        // Just verify it returns Ok without panicking.
        apply_tools(builder, &[]).expect("ok");
    }

    #[test]
    fn apply_tools_with_one_tool_succeeds() {
        let builder = RequestBuilder::new();
        let tools = vec![LlmToolDescriptor {
            name: "fs.read".to_string(),
            description: "read".to_string(),
            input_schema: json!({"type": "object", "properties": {}}),
        }];
        apply_tools(builder, &tools).expect("ok");
    }

    #[test]
    fn append_user_message_round_trips_text() {
        let builder = RequestBuilder::new();
        let msg = LlmMessage::user_text("ping");
        let _builder = append_message_to_builder(builder, &msg);
        // No public accessor on RequestBuilder to verify the
        // message landed; the test verifies the conversion
        // doesn't panic on the type-shape happy path.
    }

    #[test]
    fn append_assistant_message_with_tool_calls_uses_tool_call_form() {
        let builder = RequestBuilder::new();
        let msg = LlmMessage::Assistant {
            text: String::new(),
            tool_calls: vec![LlmToolCallRecord {
                call_id: "call_abc".to_string(),
                tool_name: "fs.read".to_string(),
                input: json!({"path": "/etc/hosts"}),
            }],
        };
        let _builder = append_message_to_builder(builder, &msg);
        // Same no-public-accessor caveat — this test verifies
        // the non-empty tool_calls branch compiles + executes.
    }

    #[test]
    fn append_tool_result_message_maps_to_tool_message() {
        let builder = RequestBuilder::new();
        let msg = LlmMessage::ToolResult {
            call_id: "call_abc".to_string(),
            content: "127.0.0.1 localhost".to_string(),
            is_error: false,
        };
        let _builder = append_message_to_builder(builder, &msg);
    }
}

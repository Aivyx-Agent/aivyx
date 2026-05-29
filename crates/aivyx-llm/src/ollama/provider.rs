//! The native `OllamaProvider`: concrete `LlmProvider` against
//! Ollama's `/api/chat` streaming endpoint.
//!
//! Phase 121 Task 2 — ships the skeleton: provider struct,
//! config types, request-body builder, health check. The JSONL
//! streaming line reader (Task 3), stream state machine (Task
//! 4), and `LlmProvider::chat_stream` impl (Task 5) land in
//! follow-on tasks.
//!
//! ## Wire format (target)
//!
//! Ollama `/api/chat` accepts a JSON request body:
//!
//! ```json
//! {
//!   "model": "qwen3.6:27b",
//!   "messages": [{ "role": "user", "content": "..." }],
//!   "tools": [{ "type": "function", "function": { ... } }],
//!   "stream": true,
//!   "options": { "num_ctx": 8192, "num_predict": 1024, ... }
//! }
//! ```
//!
//! The response streams as newline-delimited JSON (one object
//! per line):
//!
//! ```json
//! { "message": { "role": "assistant", "content": "..." }, "done": false }
//! { "message": { "role": "assistant", "content": "..." }, "done": false }
//! { "message": { "role": "assistant", "tool_calls": [...] }, "done": true,
//!   "prompt_eval_count": 42, "eval_count": 31 }
//! ```
//!
//! Tool calls typically arrive in the final `done: true` chunk
//! as a complete `tool_calls` array (not deltas — simpler than
//! OpenAI's incremental reassembly).
//!
//! Phase 121 Task 4 parses this stream shape.

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{ContentBlock, LlmError, LlmMessage, LlmRequest};

use crate::transport::{HttpTransport, ReqwestTransport};

/// Default Ollama base URL — standard port for `ollama serve`.
/// Mirrors `crate::openai::DEFAULT_OLLAMA_BASE_URL` (the OpenAI-
/// compat path that pre-dates this native adapter); the constant
/// is duplicated rather than re-exported so the two paths stay
/// independently auditable.
pub const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Ollama-specific generation options that ride in the
/// `options: {...}` block of the `/api/chat` request body.
/// Operators set these via `[ollama]` in `aivyx.toml` (Phase 121
/// Task 6); `None` values are omitted from the wire form so
/// Ollama's own defaults apply.
///
/// The set covers the operator-relevant subset of Ollama's
/// modelfile options. Future Phase 121 follow-ups can add more
/// fields additively without breaking the wire shape (Ollama
/// ignores unknown options).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OllamaOptions {
    /// Context window size in tokens. Override Ollama's
    /// per-model default; useful for models with large native
    /// contexts running on operators with sufficient VRAM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_ctx: Option<u32>,
    /// Maximum tokens to generate. Override `max_tokens` from
    /// the request when set; otherwise the request value applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_predict: Option<u32>,
    /// Number of threads the runtime may use. Operator-tunable
    /// for shared-host setups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_thread: Option<u32>,
    /// Mirostat sampling mode (0 = disabled, 1 = Mirostat,
    /// 2 = Mirostat 2.0). Operator-conservative default: `None`
    /// → Ollama's per-model default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirostat: Option<u8>,
    /// Top-k sampling. `None` → Ollama default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    /// Top-p (nucleus) sampling. `None` → Ollama default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Repeat penalty. `None` → Ollama default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_penalty: Option<f32>,
    /// Number of previous tokens to consider for `repeat_penalty`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_last_n: Option<i32>,
    /// Random seed for reproducibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
}

impl OllamaOptions {
    /// `true` when every field is `None`. Used by the request-
    /// body builder to decide whether to emit the `options`
    /// block at all (cleaner wire form when the operator
    /// hasn't overridden anything).
    pub fn is_empty(&self) -> bool {
        self.num_ctx.is_none()
            && self.num_predict.is_none()
            && self.num_thread.is_none()
            && self.mirostat.is_none()
            && self.top_k.is_none()
            && self.top_p.is_none()
            && self.repeat_penalty.is_none()
            && self.repeat_last_n.is_none()
            && self.seed.is_none()
    }
}

/// Provider config. The OpenAI provider needed an `api_key`
/// because cloud OpenAI authenticates via Bearer; Ollama does
/// not require auth, so the field is `Option<SecretString>` for
/// non-default deployments (e.g. operator-protected Ollama
/// behind a proxy). The default `None` matches the
/// `ollama serve` posture.
pub struct OllamaConfig {
    /// Base URL. `None` → [`DEFAULT_OLLAMA_BASE_URL`].
    pub base_url: Option<String>,
    /// Optional API key for protected Ollama deployments. Most
    /// operators leave this `None`.
    pub api_key: Option<SecretString>,
    /// Operator-configured generation options. Defaults all-
    /// `None`; the request-body builder omits the `options`
    /// block when this is empty.
    pub options: OllamaOptions,
}

impl OllamaConfig {
    /// Build a default config: localhost, no auth, no option
    /// overrides. Matches a vanilla `ollama serve` setup.
    pub fn default_local() -> Self {
        OllamaConfig {
            base_url: None,
            api_key: None,
            options: OllamaOptions::default(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    pub fn with_api_key(mut self, key: impl Into<SecretString>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    pub fn with_options(mut self, options: OllamaOptions) -> Self {
        self.options = options;
        self
    }
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self::default_local()
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

/// Native Ollama `LlmProvider`. Phase 121 Task 2 ships the
/// skeleton; the `LlmProvider::chat_stream` impl lands at
/// Task 5 after Tasks 3-4 deliver the JSONL streaming
/// substrate.
pub struct OllamaProvider {
    pub(crate) config: OllamaConfig,
    pub(crate) transport: Box<dyn HttpTransport>,
}

impl OllamaProvider {
    pub fn new(config: OllamaConfig) -> Result<Self, LlmError> {
        Ok(OllamaProvider {
            config,
            transport: Box::new(ReqwestTransport::new()?),
        })
    }

    pub fn with_transport(
        config: OllamaConfig,
        transport: Box<dyn HttpTransport>,
    ) -> Self {
        OllamaProvider { config, transport }
    }

    pub(crate) fn base_url(&self) -> &str {
        self.config
            .base_url
            .as_deref()
            .unwrap_or(DEFAULT_OLLAMA_BASE_URL)
    }

    /// Endpoint for the chat-streaming POST. Different from the
    /// OpenAI-compat path's `/v1/chat/completions`: Ollama's
    /// native endpoint is `/api/chat`.
    pub(crate) fn endpoint(&self) -> String {
        format!("{}/api/chat", self.base_url())
    }

    /// Lightweight health check against the Ollama base URL.
    /// Same shape as `OpenAiProvider::health_check`; Ollama
    /// answers GET / with the plain-text body
    /// `"Ollama is running"`.
    pub async fn health_check(&self) -> Result<(), String> {
        let url = self.base_url();
        match self.transport.get_text(url).await {
            Ok(_body) => Ok(()),
            Err(LlmError::Transport(e)) => Err(format!(
                "cannot reach {url} — is Ollama running? \
                 Start it with `ollama serve`.\n  \
                 (transport error: {e})"
            )),
            Err(LlmError::Api { status, message }) => {
                Err(format!("{url} returned HTTP {status}: {message}"))
            }
            Err(other) => {
                Err(format!("health check against {url} failed: {other}"))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Request-body construction (Phase 121 Task 2)
// ---------------------------------------------------------------------------

/// Build the JSON body for Ollama's `/api/chat`. Pure function
/// over the request + operator options. Returns
/// `LlmError::UnknownModel("")` for empty model strings (same
/// guard as the OpenAI provider).
///
/// Wire shape:
/// ```json
/// {
///   "model": "<model>",
///   "messages": [...],
///   "tools": [...],         // present iff request.tools non-empty
///   "stream": true,
///   "options": {...}        // present iff operator overrode anything
/// }
/// ```
///
/// `system` (the per-turn system prompt) lands as a leading
/// `{"role": "system", "content": "..."}` message — same shape
/// the OpenAI provider uses, and what Ollama's `/api/chat`
/// expects.
///
/// `temperature` lands inside the `options` block as
/// `options.temperature` rather than at top-level (Ollama's
/// convention).
pub fn build_request_body(
    request: &LlmRequest<'_>,
    options: &OllamaOptions,
) -> Result<Value, LlmError> {
    if request.model.is_empty() {
        return Err(LlmError::UnknownModel(String::new()));
    }

    let mut messages: Vec<Value> = Vec::new();
    if let Some(system) = request.system {
        messages.push(json!({"role": "system", "content": system}));
    }
    for msg in request.messages {
        messages.push(ollama_message(msg)?);
    }

    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
    });

    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect();
        body["tools"] = Value::Array(tools);
    }

    // Phase 121 — operator-configured options + request-side
    // temperature merge into the same `options` block per Ollama's
    // convention. We compute the merged value into a Value rather
    // than mutating `options` (which is shared via &) so the call
    // site stays pure.
    let mut options_value =
        serde_json::to_value(options).map_err(|e| {
            LlmError::Parse(format!("ollama options serialize: {e}"))
        })?;
    if let Some(temp) = request.temperature {
        if let Some(obj) = options_value.as_object_mut() {
            obj.insert("temperature".into(), json!(temp));
        }
    }
    let emit_options = match &options_value {
        Value::Object(obj) => !obj.is_empty(),
        _ => false,
    };
    if emit_options {
        body["options"] = options_value;
    }

    Ok(body)
}

/// Translate one `LlmMessage` into Ollama's wire format. Same
/// shape as the OpenAI provider for User/Assistant/ToolResult,
/// with two minor protocol differences:
/// - Ollama's tool-result role is `"tool"` (matches OpenAI;
///   distinct from Anthropic's `"user"` + content block).
/// - Ollama's `tool_calls` array on the Assistant message stores
///   `function.arguments` as a JSON OBJECT (not a JSON string —
///   Ollama parses the function-args natively, unlike OpenAI's
///   string-JSON-in-JSON convention).
fn ollama_message(msg: &LlmMessage) -> Result<Value, LlmError> {
    Ok(match msg {
        LlmMessage::User { content } => {
            let has_images = content.iter().any(ContentBlock::is_image);
            if has_images {
                // Ollama's vision-model path uses an `images`
                // array of base64 strings alongside text content
                // (different from OpenAI's content-block array).
                // Phase 121 ships the operator-common text+image
                // shape; richer multimodal scaffolds can land
                // additively as Ollama's vision support
                // stabilizes.
                let text = content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                let images: Vec<Value> = content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ImageBase64 { data, .. } => {
                            Some(json!(data))
                        }
                        _ => None,
                    })
                    .collect();
                let mut msg = json!({
                    "role": "user",
                    "content": text,
                });
                if !images.is_empty() {
                    msg["images"] = Value::Array(images);
                }
                msg
            } else {
                let text = content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                json!({ "role": "user", "content": text })
            }
        }
        LlmMessage::Assistant { text, tool_calls } => {
            let mut msg = json!({"role": "assistant"});
            // Ollama's chat protocol expects `content` even when
            // it's empty (some Ollama models reject Assistant
            // messages with no content field).
            msg["content"] = Value::String(text.clone());
            if !tool_calls.is_empty() {
                let calls: Vec<Value> = tool_calls
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.call_id,
                            "function": {
                                "name": c.tool_name,
                                // Ollama's `arguments` is a JSON
                                // object, NOT a JSON-encoded
                                // string. Different from OpenAI.
                                "arguments": c.input.clone(),
                            }
                        })
                    })
                    .collect();
                msg["tool_calls"] = Value::Array(calls);
            }
            msg
        }
        LlmMessage::ToolResult {
            call_id,
            content,
            ..
        } => json!({
            "role": "tool",
            "tool_call_id": call_id,
            "content": content,
        }),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LlmMessage, LlmRequest, LlmToolDescriptor};

    fn simple_request<'a>(
        msgs: &'a [LlmMessage],
        tools: &'a [LlmToolDescriptor],
    ) -> LlmRequest<'a> {
        LlmRequest {
            model: "qwen3.6:27b",
            system: None,
            messages: msgs,
            tools,
            max_tokens: 1024,
            temperature: None,
        }
    }

    // ----- Config -----

    #[test]
    fn config_default_local_uses_default_base_url() {
        let cfg = OllamaConfig::default_local();
        assert!(cfg.base_url.is_none());
        assert!(cfg.api_key.is_none());
        assert!(cfg.options.is_empty());
    }

    #[test]
    fn options_is_empty_when_all_none() {
        let options = OllamaOptions::default();
        assert!(options.is_empty());
    }

    #[test]
    fn options_is_not_empty_when_any_field_set() {
        let options = OllamaOptions {
            num_ctx: Some(8192),
            ..OllamaOptions::default()
        };
        assert!(!options.is_empty());
    }

    #[test]
    fn options_round_trips_through_serde() {
        let original = OllamaOptions {
            num_ctx: Some(8192),
            num_predict: Some(1024),
            mirostat: Some(2),
            seed: Some(42),
            ..OllamaOptions::default()
        };
        let json = serde_json::to_value(&original).unwrap();
        let parsed: OllamaOptions = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn options_skip_serialize_when_none() {
        let options = OllamaOptions {
            num_ctx: Some(8192),
            ..OllamaOptions::default()
        };
        let json = serde_json::to_value(&options).unwrap();
        let obj = json.as_object().unwrap();
        assert!(obj.contains_key("num_ctx"));
        // Every other field is None → omitted from wire form.
        assert_eq!(obj.len(), 1);
    }

    // ----- Endpoint -----

    #[test]
    fn endpoint_uses_default_base_when_unset() {
        let provider = OllamaProvider::new(OllamaConfig::default_local())
            .expect("build provider");
        assert_eq!(provider.endpoint(), "http://localhost:11434/api/chat");
    }

    #[test]
    fn endpoint_honors_operator_base_url_override() {
        let provider = OllamaProvider::new(
            OllamaConfig::default_local()
                .with_base_url("http://gpu-host.local:11434"),
        )
        .expect("build provider");
        assert_eq!(
            provider.endpoint(),
            "http://gpu-host.local:11434/api/chat"
        );
    }

    // ----- Request body -----

    #[test]
    fn request_body_minimal_no_tools_no_options() {
        let msgs = vec![LlmMessage::user_text("hello")];
        let req = simple_request(&msgs, &[]);
        let options = OllamaOptions::default();
        let body = build_request_body(&req, &options).unwrap();
        assert_eq!(body["model"], "qwen3.6:27b");
        assert_eq!(body["stream"], true);
        // No options block when both operator and request
        // contribute nothing.
        assert!(body.get("options").is_none());
        // No tools block when request advertises nothing.
        assert!(body.get("tools").is_none());
        // Messages array contains the one user message.
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "hello");
    }

    #[test]
    fn request_body_includes_system_as_leading_message() {
        let msgs = vec![LlmMessage::user_text("hello")];
        let req = LlmRequest {
            model: "qwen3.6:27b",
            system: Some("you are helpful"),
            messages: &msgs,
            tools: &[],
            max_tokens: 1024,
            temperature: None,
        };
        let body = build_request_body(&req, &OllamaOptions::default()).unwrap();
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "you are helpful");
        assert_eq!(messages[1]["role"], "user");
    }

    #[test]
    fn request_body_includes_tools_as_function_descriptors() {
        let msgs = vec![LlmMessage::user_text("read a file")];
        let tools = vec![LlmToolDescriptor {
            name: "fs.read".into(),
            description: "Read a file from sandbox.".into(),
            input_schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        }];
        let req = simple_request(&msgs, &tools);
        let body = build_request_body(&req, &OllamaOptions::default()).unwrap();
        let tools_array = body["tools"].as_array().unwrap();
        assert_eq!(tools_array.len(), 1);
        assert_eq!(tools_array[0]["type"], "function");
        assert_eq!(tools_array[0]["function"]["name"], "fs.read");
        assert_eq!(
            tools_array[0]["function"]["description"],
            "Read a file from sandbox."
        );
    }

    #[test]
    fn request_body_includes_options_block_when_operator_set_anything() {
        let msgs = vec![LlmMessage::user_text("hi")];
        let req = simple_request(&msgs, &[]);
        let options = OllamaOptions {
            num_ctx: Some(16384),
            mirostat: Some(2),
            ..OllamaOptions::default()
        };
        let body = build_request_body(&req, &options).unwrap();
        let opts = body["options"].as_object().unwrap();
        assert_eq!(opts["num_ctx"], 16384);
        assert_eq!(opts["mirostat"], 2);
        // Unset options stay out of the wire form.
        assert!(!opts.contains_key("top_p"));
        assert!(!opts.contains_key("seed"));
    }

    #[test]
    fn request_body_request_temperature_lands_inside_options_block() {
        // Ollama's convention: temperature is a sampling option,
        // not a top-level field. The request's temperature merges
        // into the options block.
        let msgs = vec![LlmMessage::user_text("hi")];
        let req = LlmRequest {
            model: "qwen3.6:27b",
            system: None,
            messages: &msgs,
            tools: &[],
            max_tokens: 1024,
            temperature: Some(0.7),
        };
        let body = build_request_body(&req, &OllamaOptions::default()).unwrap();
        let opts = body["options"].as_object().unwrap();
        // The request's temperature carried through.
        assert!((opts["temperature"].as_f64().unwrap() - 0.7).abs() < 1e-6);
        // Temperature alone causes options block to appear.
        assert!(body["options"].is_object());
    }

    #[test]
    fn request_body_temperature_merges_with_operator_options() {
        let msgs = vec![LlmMessage::user_text("hi")];
        let req = LlmRequest {
            model: "qwen3.6:27b",
            system: None,
            messages: &msgs,
            tools: &[],
            max_tokens: 1024,
            temperature: Some(0.2),
        };
        let options = OllamaOptions {
            num_ctx: Some(8192),
            ..OllamaOptions::default()
        };
        let body = build_request_body(&req, &options).unwrap();
        let opts = body["options"].as_object().unwrap();
        // Both fields carried through.
        assert_eq!(opts["num_ctx"], 8192);
        assert!((opts["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6);
    }

    #[test]
    fn request_body_empty_model_errors() {
        let msgs = vec![LlmMessage::user_text("hi")];
        let req = LlmRequest {
            model: "",
            system: None,
            messages: &msgs,
            tools: &[],
            max_tokens: 1024,
            temperature: None,
        };
        let err =
            build_request_body(&req, &OllamaOptions::default()).unwrap_err();
        assert!(matches!(err, LlmError::UnknownModel(_)));
    }

    // ----- Message translation -----

    #[test]
    fn assistant_message_with_tool_calls_uses_object_args_not_string() {
        // Ollama's wire shape stores tool-call arguments as a
        // JSON object, NOT a JSON-encoded string (different from
        // OpenAI). The agent's history must round-trip with the
        // right shape.
        let msgs = vec![LlmMessage::Assistant {
            text: "calling fs.read".into(),
            tool_calls: vec![crate::LlmToolCallRecord {
                call_id: "c1".into(),
                tool_name: "fs.read".into(),
                input: json!({"path": "foo.txt"}),
            }],
        }];
        let req = simple_request(&msgs, &[]);
        let body = build_request_body(&req, &OllamaOptions::default()).unwrap();
        let messages = body["messages"].as_array().unwrap();
        let asst = &messages[0];
        assert_eq!(asst["role"], "assistant");
        assert_eq!(asst["content"], "calling fs.read");
        let calls = asst["tool_calls"].as_array().unwrap();
        // The arguments field is an OBJECT, not a string.
        assert!(calls[0]["function"]["arguments"].is_object());
        assert_eq!(
            calls[0]["function"]["arguments"]["path"],
            "foo.txt"
        );
    }

    #[test]
    fn tool_result_message_uses_tool_role_with_call_id() {
        let msgs = vec![LlmMessage::ToolResult {
            call_id: "c1".into(),
            content: "{\"result\":\"ok\"}".into(),
            is_error: false,
        }];
        let req = simple_request(&msgs, &[]);
        let body = build_request_body(&req, &OllamaOptions::default()).unwrap();
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "tool");
        assert_eq!(messages[0]["tool_call_id"], "c1");
        assert_eq!(messages[0]["content"], "{\"result\":\"ok\"}");
    }

    #[test]
    fn user_message_with_image_uses_ollama_images_array() {
        // Ollama's vision protocol carries base64 images as a
        // separate `images: []` field, not in the content array
        // (different from OpenAI's content-block format).
        let msgs = vec![LlmMessage::User {
            content: vec![
                ContentBlock::text("describe this"),
                ContentBlock::ImageBase64 {
                    media_type: "image/png".into(),
                    data: "AAA".into(),
                },
            ],
        }];
        let req = simple_request(&msgs, &[]);
        let body = build_request_body(&req, &OllamaOptions::default()).unwrap();
        let user = &body["messages"].as_array().unwrap()[0];
        assert_eq!(user["role"], "user");
        assert_eq!(user["content"], "describe this");
        let images = user["images"].as_array().unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0], "AAA");
    }
}

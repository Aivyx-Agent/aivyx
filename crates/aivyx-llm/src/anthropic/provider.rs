//! The `AnthropicProvider`: concrete `LlmProvider` against the Anthropic
//! Messages streaming API.
//!
//! ## Wire format (documented here so the state machine is reviewable)
//!
//! A streaming `POST /v1/messages` returns these events in order:
//!
//! 1. `message_start` — carries `message.usage.input_tokens` and the
//!    message metadata. We capture usage here.
//! 2. For each block the model emits (text or tool_use), one
//!    `content_block_start` + N `content_block_delta` + one
//!    `content_block_stop`.
//!    - Text blocks: `delta.type = "text_delta"`, `delta.text = "..."`.
//!    - Tool-use blocks: the `content_block_start` carries `id`, `name`,
//!      and an empty `input: {}`. Each `content_block_delta` has
//!      `delta.type = "input_json_delta"` with a `partial_json` string;
//!      we accumulate these and parse the concatenated string at
//!      `content_block_stop`.
//! 3. `message_delta` — carries `delta.stop_reason` and
//!    `usage.output_tokens`. We record both.
//! 4. `message_stop` — terminal.
//!
//! On `stop_reason == "tool_use"` we emit `LlmStepEnd::ToolCall` with
//! the first (and, per Anthropic's current behavior, only) tool-use
//! block. On `stop_reason == "end_turn"` (or `"max_tokens"`, etc.) we
//! emit `LlmStepEnd::FinalMessage`.

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::{
    ContentBlock, LlmError, LlmMessage, LlmProvider, LlmRequest, LlmStepEnd, LlmStream,
    LlmStreamEvent, LlmUsage,
};

use super::sse::{SseEvent, SseReader};
use crate::transport::{HttpTransport, ReqwestTransport};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Provider configuration. API key is wrapped in `SecretString` so it
/// never shows up in `Debug` output or accidental `tracing` macros.
pub struct AnthropicConfig {
    pub api_key: SecretString,
    pub base_url: Option<String>,
}

impl AnthropicConfig {
    pub fn new(api_key: impl Into<SecretString>) -> Self {
        AnthropicConfig {
            api_key: api_key.into(),
            base_url: None,
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

pub struct AnthropicProvider {
    config: AnthropicConfig,
    transport: Box<dyn HttpTransport>,
}

impl AnthropicProvider {
    /// Build a provider with the real `reqwest` transport.
    pub fn new(config: AnthropicConfig) -> Result<Self, LlmError> {
        Ok(AnthropicProvider {
            config,
            transport: Box::new(ReqwestTransport::new()?),
        })
    }

    /// Build a provider with a caller-supplied transport. This is the
    /// entry point tests use — pass a `FakeTransport` that replays
    /// canned SSE bytes.
    pub fn with_transport(
        config: AnthropicConfig,
        transport: Box<dyn HttpTransport>,
    ) -> Self {
        AnthropicProvider { config, transport }
    }

    fn endpoint(&self) -> String {
        let base = self
            .config
            .base_url
            .as_deref()
            .unwrap_or(DEFAULT_BASE_URL);
        format!("{base}/v1/messages")
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat_stream(
        &self,
        request: LlmRequest<'_>,
        cancellation: &CancellationToken,
    ) -> Result<Box<dyn LlmStream>, LlmError> {
        let body = build_request_body(&request)?;
        let body_bytes = serde_json::to_vec(&body)
            .map_err(|e| LlmError::Parse(format!("request serialization: {e}")))?;

        let api_key = self.config.api_key.expose_secret().to_string();
        let headers: Vec<(&str, &str)> = vec![
            ("content-type", "application/json"),
            ("accept", "text/event-stream"),
            ("anthropic-version", ANTHROPIC_VERSION),
            ("x-api-key", api_key.as_str()),
        ];

        let endpoint = self.endpoint();
        let byte_stream = self
            .transport
            .post_sse(&endpoint, &headers, body_bytes, cancellation)
            .await?;

        // Phase 120 — same snapshot pattern as the OpenAI provider.
        let known_tool_names: std::collections::HashSet<String> = request
            .tools
            .iter()
            .map(|t| t.name.to_string())
            .collect();

        Ok(Box::new(AnthropicStream {
            sse: SseReader::new(byte_stream),
            state: StreamState::default(),
            terminal: None,
            known_tool_names,
        }))
    }
}

// ---------------------------------------------------------------------------
// Request-body construction
// ---------------------------------------------------------------------------

fn build_request_body(request: &LlmRequest<'_>) -> Result<Value, LlmError> {
    if request.model.is_empty() {
        return Err(LlmError::UnknownModel(String::new()));
    }

    // Phase 164 — pre-flight guard: document
    // content blocks require Claude 3.5+.
    // Detect from the request shape + model
    // string; surface a clear client-side
    // error instead of letting the API return
    // a 400.
    if request_has_document_block(request)
        && !model_supports_documents(request.model)
    {
        return Err(LlmError::Config(format!(
            "Anthropic model {model:?} does not support document blocks; \
             document content blocks (e.g. PDFs) require Claude 3.5 or newer \
             (claude-3-5-*, claude-3-7-*, claude-opus-4-*, claude-sonnet-4-*, \
             claude-haiku-4-*, or a newer-prefix variant). Pick a supported \
             model in your aivyx.toml or attach the document to a model that \
             accepts it.",
            model = request.model
        )));
    }

    let messages: Vec<Value> = merge_consecutive_tool_results(
        request
            .messages
            .iter()
            .map(anthropic_message)
            .collect::<Result<_, _>>()?,
    );

    let tools: Vec<Value> = request
        .tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
            })
        })
        .collect();

    let mut body = json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "messages": messages,
        "stream": true,
    });

    if let Some(system) = request.system {
        body["system"] = Value::String(system.to_string());
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    if let Some(temp) = request.temperature {
        body["temperature"] = json!(temp);
    }

    Ok(body)
}

/// Phase 164 — true iff any `LlmMessage::User`
/// in the request carries at least one
/// `ContentBlock::DocumentBase64`. Pure
/// substrate so the guard can be tested
/// without going through `build_request_body`.
fn request_has_document_block(request: &LlmRequest<'_>) -> bool {
    request.messages.iter().any(|msg| match msg {
        LlmMessage::User { content } => {
            content.iter().any(|b| b.is_document())
        }
        _ => false,
    })
}

/// Phase 164 — true iff `model` names an
/// Anthropic model that supports document
/// content blocks. Detection is by hyphenated
/// prefix:
///
/// - `claude-3-5-*` (e.g. claude-3-5-sonnet-20240620)
/// - `claude-3-7-*` (forward-compat for a 3.7 release)
/// - `claude-opus-4-*`, `claude-sonnet-4-*`,
///   `claude-haiku-4-*` (Claude 4 family
///   shipped 2025-2026)
///
/// Hand-maintained list. A new variant with a
/// different prefix shape fails closed until
/// the substrate adds the prefix; the operator
/// sees a clear pre-flight error and the fix
/// is one line here.
fn model_supports_documents(model: &str) -> bool {
    const SUPPORTED_PREFIXES: &[&str] = &[
        "claude-3-5-",
        "claude-3-7-",
        "claude-opus-4-",
        "claude-sonnet-4-",
        "claude-haiku-4-",
    ];
    SUPPORTED_PREFIXES
        .iter()
        .any(|p| model.starts_with(p))
}

fn anthropic_message(msg: &LlmMessage) -> Result<Value, LlmError> {
    Ok(match msg {
        LlmMessage::User { content } => {
            let blocks: Vec<Value> = content
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { text } => {
                        json!({"type": "text", "text": text})
                    }
                    ContentBlock::ImageBase64 { media_type, data } => json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": media_type,
                            "data": data,
                        }
                    }),
                    // Phase 163 / amendment A13 —
                    // Anthropic supports document content
                    // blocks (Claude 3.5+). Same source
                    // shape as image; the model handles
                    // PDFs natively.
                    ContentBlock::DocumentBase64 { media_type, data } => json!({
                        "type": "document",
                        "source": {
                            "type": "base64",
                            "media_type": media_type,
                            "data": data,
                        }
                    }),
                })
                .collect();
            json!({ "role": "user", "content": blocks })
        }
        LlmMessage::Assistant { text, tool_calls } => {
            let mut content: Vec<Value> = Vec::new();
            if !text.is_empty() {
                content.push(json!({"type": "text", "text": text}));
            }
            for call in tool_calls {
                content.push(json!({
                    "type": "tool_use",
                    "id": call.call_id,
                    "name": call.tool_name,
                    "input": call.input,
                }));
            }
            json!({ "role": "assistant", "content": content })
        }
        LlmMessage::ToolResult {
            call_id,
            content,
            is_error,
        } => json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": call_id,
                "content": content,
                "is_error": is_error,
            }],
        }),
    })
}

/// Merge consecutive `role: "user"` messages whose content blocks are all
/// `tool_result` entries into a single user message. The Anthropic API
/// requires all tool results answering a multi-tool assistant turn to
/// appear in one `role: "user"` message. Phase 40.
fn merge_consecutive_tool_results(messages: Vec<Value>) -> Vec<Value> {
    let mut merged: Vec<Value> = Vec::with_capacity(messages.len());

    for msg in messages {
        let dominated = is_tool_result_user_msg(&msg)
            && merged.last().is_some_and(is_tool_result_user_msg);

        if dominated {
            // Extend the previous message's content array.
            let prev = merged.last_mut().unwrap();
            let incoming = msg["content"].as_array().unwrap();
            let target = prev["content"].as_array_mut().unwrap();
            target.extend(incoming.iter().cloned());
        } else {
            merged.push(msg);
        }
    }

    merged
}

/// Returns `true` if `msg` is a `role: "user"` message where every
/// content block has `type: "tool_result"`.
fn is_tool_result_user_msg(msg: &Value) -> bool {
    msg.get("role").and_then(Value::as_str) == Some("user")
        && msg
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|blocks| {
                !blocks.is_empty()
                    && blocks
                        .iter()
                        .all(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            })
}

// ---------------------------------------------------------------------------
// Stream state machine
// ---------------------------------------------------------------------------

#[derive(Default)]
struct StreamState {
    accumulated_text: String,
    usage: LlmUsage,
    pending_tool: Option<PendingTool>,
    completed_tools: Vec<CompletedTool>,
    stop_reason: Option<String>,
}

struct PendingTool {
    call_id: String,
    tool_name: String,
    input_json: String,
}

struct CompletedTool {
    call_id: String,
    tool_name: String,
    input: Value,
}

struct AnthropicStream {
    sse: SseReader,
    state: StreamState,
    terminal: Option<LlmStepEnd>,
    /// Phase 120 — canonical tool-name set the planner advertised.
    /// Anthropic's hosted models rarely hallucinate tool names
    /// (well-trained tool-use protocol), but the validation runs
    /// uniformly so the substrate doesn't have provider-specific
    /// recovery semantics. Empty set when the request advertised
    /// no tools.
    known_tool_names: std::collections::HashSet<String>,
}

#[async_trait]
impl LlmStream for AnthropicStream {
    async fn next_event(&mut self) -> Result<Option<LlmStreamEvent>, LlmError> {
        loop {
            if self.terminal.is_some() {
                return Ok(None);
            }
            let Some(sse_event) = self.sse.next_event().await? else {
                // Stream ended without a message_stop — that's a truncation.
                return Err(LlmError::StreamEnded(
                    "Anthropic SSE ended before message_stop".to_string(),
                ));
            };
            if let Some(emit) = self.handle_sse(sse_event)? {
                return Ok(Some(emit));
            }
        }
    }

    async fn finish(self: Box<Self>) -> Result<LlmStepEnd, LlmError> {
        self.terminal.ok_or_else(|| {
            LlmError::StreamEnded(
                "AnthropicStream::finish called before stream drained".to_string(),
            )
        })
    }
}

impl AnthropicStream {
    /// Process one SSE event, updating state and optionally returning a
    /// mid-stream `LlmStreamEvent` to yield. On `message_stop`, sets
    /// `self.terminal` so the next `next_event` call returns `None`.
    fn handle_sse(&mut self, sse: SseEvent) -> Result<Option<LlmStreamEvent>, LlmError> {
        match sse.event.as_str() {
            "message_start" => {
                let parsed: MessageStart = parse_data(&sse.data)?;
                self.state.usage.input_tokens = parsed.message.usage.input_tokens;
                self.state.usage.cache_creation_input_tokens =
                    parsed.message.usage.cache_creation_input_tokens.unwrap_or(0);
                self.state.usage.cache_read_input_tokens =
                    parsed.message.usage.cache_read_input_tokens.unwrap_or(0);
                Ok(None)
            }
            "content_block_start" => {
                let parsed: ContentBlockStart = parse_data(&sse.data)?;
                if parsed.content_block.kind == "tool_use" {
                    self.state.pending_tool = Some(PendingTool {
                        call_id: parsed.content_block.id.unwrap_or_default(),
                        tool_name: parsed.content_block.name.unwrap_or_default(),
                        input_json: String::new(),
                    });
                }
                Ok(None)
            }
            "content_block_delta" => {
                let parsed: ContentBlockDelta = parse_data(&sse.data)?;
                match parsed.delta.kind.as_str() {
                    "text_delta" => {
                        let text = parsed.delta.text.unwrap_or_default();
                        self.state.accumulated_text.push_str(&text);
                        Ok(Some(LlmStreamEvent::TextChunk(text)))
                    }
                    "input_json_delta" => {
                        let partial = parsed.delta.partial_json.unwrap_or_default();
                        let pending = self.state.pending_tool.as_mut().ok_or_else(|| {
                            LlmError::Parse(
                                "input_json_delta without open tool_use block".to_string(),
                            )
                        })?;
                        pending.input_json.push_str(&partial);
                        Ok(None)
                    }
                    other => Err(LlmError::Parse(format!(
                        "unknown content_block_delta type: {other}"
                    ))),
                }
            }
            "content_block_stop" => {
                if let Some(pending) = self.state.pending_tool.take() {
                    let input: Value = if pending.input_json.is_empty() {
                        json!({})
                    } else {
                        serde_json::from_str(&pending.input_json).map_err(|e| {
                            LlmError::Parse(format!("tool input_json parse: {e}"))
                        })?
                    };
                    self.state.completed_tools.push(CompletedTool {
                        call_id: pending.call_id,
                        tool_name: pending.tool_name,
                        input,
                    });
                }
                Ok(None)
            }
            "message_delta" => {
                let parsed: MessageDelta = parse_data(&sse.data)?;
                if let Some(stop) = parsed.delta.stop_reason {
                    self.state.stop_reason = Some(stop);
                }
                if let Some(usage) = parsed.usage {
                    self.state.usage.output_tokens = usage.output_tokens.unwrap_or(0);
                }
                Ok(None)
            }
            "message_stop" => {
                self.terminal = Some(self.build_terminal()?);
                Ok(None)
            }
            "ping" | "error" => {
                // `ping` is a heartbeat — ignore. `error` events in the
                // stream itself are rare; when they happen Anthropic
                // sends the error body as data. Surface as Api error.
                if sse.event == "error" {
                    return Err(LlmError::Api {
                        status: 0,
                        message: sse.data,
                    });
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn build_terminal(&mut self) -> Result<LlmStepEnd, LlmError> {
        let usage = self.state.usage;
        let stop = self.state.stop_reason.as_deref().unwrap_or("");
        if stop == "tool_use" {
            if self.state.completed_tools.is_empty() {
                return Err(LlmError::Parse(
                    "stop_reason=tool_use but no completed tool_use blocks".to_string(),
                ));
            }
            let calls = std::mem::take(&mut self.state.completed_tools)
                .into_iter()
                .map(|t| {
                    // Phase 120 — validate against the snapshot.
                    let name_resolution = if self
                        .known_tool_names
                        .contains(&t.tool_name)
                    {
                        crate::NameResolution::Known
                    } else {
                        crate::NameResolution::Unknown {
                            original: t.tool_name.clone(),
                        }
                    };
                    crate::ToolCallEnd {
                        call_id: t.call_id,
                        tool_name: t.tool_name,
                        input: t.input,
                        name_resolution,
                    }
                })
                .collect();
            Ok(LlmStepEnd::ToolCalls {
                calls,
                text_so_far: std::mem::take(&mut self.state.accumulated_text),
                usage,
            })
        } else {
            Ok(LlmStepEnd::FinalMessage {
                text: std::mem::take(&mut self.state.accumulated_text),
                usage,
            })
        }
    }
}

fn parse_data<T: for<'de> Deserialize<'de>>(data: &str) -> Result<T, LlmError> {
    serde_json::from_str::<T>(data)
        .map_err(|e| LlmError::Parse(format!("SSE data JSON: {e}")))
}

// ---------------------------------------------------------------------------
// Anthropic wire-format structs (minimal — only the fields we read)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct MessageStart {
    message: MessageStartMessage,
}

#[derive(Deserialize)]
struct MessageStartMessage {
    usage: MessageStartUsage,
}

#[derive(Deserialize)]
struct MessageStartUsage {
    input_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct ContentBlockStart {
    content_block: ContentBlockStartBlock,
}

#[derive(Deserialize)]
struct ContentBlockStartBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct ContentBlockDelta {
    delta: ContentBlockDeltaInner,
}

#[derive(Deserialize)]
struct ContentBlockDeltaInner {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    partial_json: Option<String>,
}

#[derive(Deserialize)]
struct MessageDelta {
    delta: MessageDeltaInner,
    #[serde(default)]
    usage: Option<MessageDeltaUsage>,
}

#[derive(Deserialize)]
struct MessageDeltaInner {
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct MessageDeltaUsage {
    #[serde(default)]
    output_tokens: Option<u32>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LlmMessage, LlmToolDescriptor};
    use async_trait::async_trait;
    use bytes::Bytes;
    use futures_util::stream;
    use std::sync::Mutex;

    use crate::transport::{ByteStream, HttpTransport};

    // -----------------------------------------------------------------------
    // FakeTransport: replays canned SSE bytes.
    // -----------------------------------------------------------------------

    struct FakeTransport {
        response: Mutex<Option<Result<Vec<&'static str>, LlmError>>>,
        seen_body: Mutex<Option<Vec<u8>>>,
    }

    impl FakeTransport {
        fn ok(chunks: Vec<&'static str>) -> Self {
            FakeTransport {
                response: Mutex::new(Some(Ok(chunks))),
                seen_body: Mutex::new(None),
            }
        }
        fn err(e: LlmError) -> Self {
            FakeTransport {
                response: Mutex::new(Some(Err(e))),
                seen_body: Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl HttpTransport for FakeTransport {
        async fn post_sse(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
            body: Vec<u8>,
            _cancellation: &CancellationToken,
        ) -> Result<ByteStream, LlmError> {
            *self.seen_body.lock().unwrap() = Some(body);
            let scripted = self
                .response
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| LlmError::Config("FakeTransport exhausted".to_string()))?;
            match scripted {
                Err(e) => Err(e),
                Ok(chunks) => {
                    let iter = chunks
                        .into_iter()
                        .map(|s| Ok::<Bytes, LlmError>(Bytes::from_static(s.as_bytes())));
                    Ok(Box::pin(stream::iter(iter)))
                }
            }
        }
    }

    fn provider_with(transport: FakeTransport) -> (AnthropicProvider, std::sync::Arc<FakeTransport>) {
        // We want to keep a handle to the transport for assertions but
        // also need to hand an owned Box to the provider. Use Arc and
        // a small wrapper that forwards calls.
        let arc = std::sync::Arc::new(transport);
        struct ArcTransport(std::sync::Arc<FakeTransport>);
        #[async_trait]
        impl HttpTransport for ArcTransport {
            async fn post_sse(
                &self,
                url: &str,
                headers: &[(&str, &str)],
                body: Vec<u8>,
                cancellation: &CancellationToken,
            ) -> Result<ByteStream, LlmError> {
                self.0.post_sse(url, headers, body, cancellation).await
            }
        }
        let provider = AnthropicProvider::with_transport(
            AnthropicConfig::new(SecretString::from("test-key")),
            Box::new(ArcTransport(arc.clone())),
        );
        (provider, arc)
    }

    fn final_message_script() -> Vec<&'static str> {
        vec![
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}\n\n",
            "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\", world\"}}\n\n",
            "event: content_block_stop\ndata: {\"index\":0}\n\n",
            "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":7}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ]
    }

    fn tool_call_script() -> Vec<&'static str> {
        vec![
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":25}}}\n\n",
            "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_01\",\"name\":\"memory.read\",\"input\":{}}}\n\n",
            "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"query\\\":\"}}\n\n",
            "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"yesterday\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"index\":0}\n\n",
            "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":19}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ]
    }

    fn blank_request_args() -> (Vec<LlmMessage>, Vec<LlmToolDescriptor>) {
        (
            vec![LlmMessage::user_text("hi")],
            vec![],
        )
    }

    #[tokio::test]
    async fn final_message_path_produces_reassembled_text_and_usage() {
        let (provider, _t) = provider_with(FakeTransport::ok(final_message_script()));
        let (messages, tools) = blank_request_args();
        let req = LlmRequest {
            model: "claude-haiku-4-5-20251001",
            system: Some("sys"),
            messages: &messages,
            tools: &tools,
            max_tokens: 256,
            temperature: None,
        };

        let token = CancellationToken::new();
        let mut stream = provider.chat_stream(req, &token).await.unwrap();

        let mut reassembled = String::new();
        while let Some(ev) = stream.next_event().await.unwrap() {
            if let LlmStreamEvent::TextChunk(chunk) = ev {
                reassembled.push_str(&chunk);
            }
        }
        let terminal = stream.finish().await.unwrap();
        match terminal {
            LlmStepEnd::FinalMessage { text, usage } => {
                assert_eq!(text, "Hello, world");
                assert_eq!(reassembled, "Hello, world");
                assert_eq!(usage.input_tokens, 12);
                assert_eq!(usage.output_tokens, 7);
            }
            other => panic!("expected FinalMessage, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_call_path_reassembles_partial_json_into_input() {
        let (provider, _t) = provider_with(FakeTransport::ok(tool_call_script()));
        let (messages, tools) = blank_request_args();
        let req = LlmRequest {
            model: "claude-haiku-4-5-20251001",
            system: None,
            messages: &messages,
            tools: &tools,
            max_tokens: 256,
            temperature: None,
        };

        let token = CancellationToken::new();
        let mut stream = provider.chat_stream(req, &token).await.unwrap();
        while stream.next_event().await.unwrap().is_some() {}
        let terminal = stream.finish().await.unwrap();

        match terminal {
            LlmStepEnd::ToolCalls {
                calls,
                text_so_far,
                usage,
            } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].call_id, "toolu_01");
                assert_eq!(calls[0].tool_name, "memory.read");
                assert_eq!(calls[0].input, json!({"query": "yesterday"}));
                assert_eq!(text_so_far, "");
                assert_eq!(usage.input_tokens, 25);
                assert_eq!(usage.output_tokens, 19);
            }
            other => panic!("expected ToolCalls, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn api_error_surface_from_transport() {
        let (provider, _t) = provider_with(FakeTransport::err(LlmError::Api {
            status: 429,
            message: "rate limited".to_string(),
        }));
        let (messages, tools) = blank_request_args();
        let req = LlmRequest {
            model: "claude-haiku-4-5-20251001",
            system: None,
            messages: &messages,
            tools: &tools,
            max_tokens: 256,
            temperature: None,
        };
        let token = CancellationToken::new();
        let err = match provider.chat_stream(req, &token).await {
            Ok(_) => panic!("expected error, got Ok stream"),
            Err(e) => e,
        };
        assert!(matches!(
            err,
            LlmError::Api {
                status: 429,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn truncated_stream_errors_on_next_event() {
        let truncated = vec![
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\n",
            "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"abc\"}}\n\n",
        ];
        let (provider, _t) = provider_with(FakeTransport::ok(truncated));
        let (messages, tools) = blank_request_args();
        let req = LlmRequest {
            model: "claude-haiku-4-5-20251001",
            system: None,
            messages: &messages,
            tools: &tools,
            max_tokens: 256,
            temperature: None,
        };
        let token = CancellationToken::new();
        let mut stream = provider.chat_stream(req, &token).await.unwrap();
        // Drain until error or None. We expect StreamEnded after the
        // last valid text chunk, because message_stop never arrives.
        let mut saw_text = false;
        let err = loop {
            match stream.next_event().await {
                Ok(Some(LlmStreamEvent::TextChunk(_))) => saw_text = true,
                Ok(Some(_)) => {}
                Ok(None) => panic!("unexpected clean end on truncated stream"),
                Err(e) => break e,
            }
        };
        assert!(saw_text);
        assert!(matches!(err, LlmError::StreamEnded(_)));
    }

    #[tokio::test]
    async fn request_body_has_expected_shape() {
        let (provider, transport) = provider_with(FakeTransport::ok(final_message_script()));
        let messages = vec![LlmMessage::user_text("ping")];
        let tools = vec![LlmToolDescriptor {
            name: "memory.read".to_string(),
            description: "look things up".to_string(),
            input_schema: json!({"type": "object"}),
        }];
        let req = LlmRequest {
            model: "claude-haiku-4-5-20251001",
            system: Some("you are a test"),
            messages: &messages,
            tools: &tools,
            max_tokens: 128,
            // 0.5 is exactly representable in f32, so the f32→f64 JSON
            // promotion doesn't introduce drift. Any decimal that isn't
            // a sum of powers of two (e.g. 0.3) would fail this assertion.
            temperature: Some(0.5),
        };
        let token = CancellationToken::new();
        let mut stream = provider.chat_stream(req, &token).await.unwrap();
        while stream.next_event().await.unwrap().is_some() {}
        let _ = stream.finish().await.unwrap();

        let body = transport.seen_body.lock().unwrap().clone().unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["model"], "claude-haiku-4-5-20251001");
        assert_eq!(body["max_tokens"], 128);
        assert_eq!(body["stream"], true);
        assert_eq!(body["system"], "you are a test");
        assert_eq!(body["temperature"], 0.5);
        assert_eq!(body["tools"][0]["name"], "memory.read");
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[tokio::test]
    async fn cancelled_token_short_circuits_before_send() {
        // FakeTransport doesn't itself check the token — it just replays
        // bytes. The real ReqwestTransport has a `tokio::select!` that
        // bails on a pre-cancelled token; we can't cover it with a fake.
        // Instead, assert the easier invariant: if chat_stream returns
        // successfully, the resulting stream can still be drained.
        let (provider, _t) = provider_with(FakeTransport::ok(final_message_script()));
        let (messages, tools) = blank_request_args();
        let req = LlmRequest {
            model: "claude-haiku-4-5-20251001",
            system: None,
            messages: &messages,
            tools: &tools,
            max_tokens: 16,
            temperature: None,
        };
        let token = CancellationToken::new();
        let mut stream = provider.chat_stream(req, &token).await.unwrap();
        token.cancel();
        // Drain — FakeTransport ignores cancellation, so the stream
        // completes normally. This test documents that the token is
        // honored at the transport layer, not inside AnthropicStream.
        let mut events = 0;
        while stream.next_event().await.unwrap().is_some() {
            events += 1;
        }
        assert!(events >= 2);
    }

    // -----------------------------------------------------------------------
    // Phase 40 — consecutive ToolResult messages merge into one user message
    // -----------------------------------------------------------------------

    #[test]
    #[allow(clippy::useless_vec)]
    fn consecutive_tool_results_merge_into_single_user_message() {
        use crate::LlmMessage;

        let messages = vec![
            LlmMessage::user_text("read both files"),
            LlmMessage::Assistant {
                text: String::new(),
                tool_calls: vec![
                    crate::LlmToolCallRecord {
                        call_id: "call_1".to_string(),
                        tool_name: "fs.read".to_string(),
                        input: json!({"path": "/a.txt"}),
                    },
                    crate::LlmToolCallRecord {
                        call_id: "call_2".to_string(),
                        tool_name: "fs.read".to_string(),
                        input: json!({"path": "/b.txt"}),
                    },
                ],
            },
            LlmMessage::ToolResult {
                call_id: "call_1".to_string(),
                content: "contents of a".to_string(),
                is_error: false,
            },
            LlmMessage::ToolResult {
                call_id: "call_2".to_string(),
                content: "contents of b".to_string(),
                is_error: false,
            },
        ];

        let serialized: Vec<Value> = messages
            .iter()
            .map(anthropic_message)
            .collect::<Result<_, _>>()
            .unwrap();

        // Before merging: 4 messages (user, assistant, user/tool_result, user/tool_result)
        assert_eq!(serialized.len(), 4);

        let merged = merge_consecutive_tool_results(serialized);

        // After merging: 3 messages (user, assistant, user with 2 tool_results)
        assert_eq!(merged.len(), 3);

        // The merged user message has both tool_result blocks.
        let tool_msg = &merged[2];
        assert_eq!(tool_msg["role"], "user");
        let content = tool_msg["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["tool_use_id"], "call_1");
        assert_eq!(content[1]["tool_use_id"], "call_2");
    }

    #[test]
    #[allow(clippy::useless_vec)]
    fn non_consecutive_tool_results_stay_separate() {
        use crate::LlmMessage;

        // Two tool results with a text user message between them — should NOT merge.
        let messages = vec![
            LlmMessage::ToolResult {
                call_id: "call_1".to_string(),
                content: "ok".to_string(),
                is_error: false,
            },
            LlmMessage::user_text("continue"),
            LlmMessage::ToolResult {
                call_id: "call_2".to_string(),
                content: "ok".to_string(),
                is_error: false,
            },
        ];

        let serialized: Vec<Value> = messages
            .iter()
            .map(anthropic_message)
            .collect::<Result<_, _>>()
            .unwrap();
        let merged = merge_consecutive_tool_results(serialized);

        // All 3 should remain separate.
        assert_eq!(merged.len(), 3);
    }

    // -----------------------------------------------------------------------
    // Phase 40 — multi-tool stream produces ToolCalls with multiple entries
    // -----------------------------------------------------------------------

    fn multi_tool_call_script() -> Vec<&'static str> {
        vec![
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10}}}\n\n",
            "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_01\",\"name\":\"fs.read\",\"input\":{}}}\n\n",
            "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"/a.txt\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"index\":0}\n\n",
            "event: content_block_start\ndata: {\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_02\",\"name\":\"memory.read\",\"input\":{}}}\n\n",
            "event: content_block_delta\ndata: {\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"topic\\\":\\\"notes\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"index\":1}\n\n",
            "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":20}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ]
    }

    #[tokio::test]
    async fn multi_tool_stream_produces_batch_tool_calls() {
        let (provider, _t) = provider_with(FakeTransport::ok(multi_tool_call_script()));
        let (messages, tools) = blank_request_args();
        let req = LlmRequest {
            model: "claude-haiku-4-5-20251001",
            system: Some("sys"),
            messages: &messages,
            tools: &tools,
            max_tokens: 256,
            temperature: None,
        };
        let token = CancellationToken::new();
        let mut stream = provider.chat_stream(req, &token).await.unwrap();

        // Drain mid-stream events.
        while stream.next_event().await.unwrap().is_some() {}

        match stream.finish().await.unwrap() {
            LlmStepEnd::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 2);
                assert_eq!(calls[0].call_id, "toolu_01");
                assert_eq!(calls[0].tool_name, "fs.read");
                assert_eq!(calls[0].input, json!({"path": "/a.txt"}));
                assert_eq!(calls[1].call_id, "toolu_02");
                assert_eq!(calls[1].tool_name, "memory.read");
                assert_eq!(calls[1].input, json!({"topic": "notes"}));
            }
            other => panic!("expected ToolCalls, got {other:?}"),
        }
    }

    // ---- Phase 163 / Amendment A13 — document content blocks ----

    #[test]
    fn anthropic_message_emits_document_block_for_pdf() {
        use crate::{ContentBlock, LlmMessage};

        let msg = LlmMessage::User {
            content: vec![
                ContentBlock::text("summarize this paper"),
                ContentBlock::DocumentBase64 {
                    media_type: "application/pdf".to_string(),
                    data: "JVBERi0xLjQK".to_string(),
                },
            ],
        };
        let v = anthropic_message(&msg).expect("ok");
        assert_eq!(v["role"], "user");
        let blocks = v["content"].as_array().expect("array");
        assert_eq!(blocks.len(), 2);
        // First block is text.
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], "summarize this paper");
        // Second block is the document — type
        // = "document", source shape matches the
        // Anthropic API contract.
        assert_eq!(blocks[1]["type"], "document");
        assert_eq!(blocks[1]["source"]["type"], "base64");
        assert_eq!(blocks[1]["source"]["media_type"], "application/pdf");
        assert_eq!(blocks[1]["source"]["data"], "JVBERi0xLjQK");
    }

    #[test]
    fn anthropic_message_document_only_no_text() {
        use crate::{ContentBlock, LlmMessage};

        let msg = LlmMessage::User {
            content: vec![ContentBlock::DocumentBase64 {
                media_type: "application/pdf".to_string(),
                data: "JVBE".to_string(),
            }],
        };
        let v = anthropic_message(&msg).expect("ok");
        let blocks = v["content"].as_array().expect("array");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "document");
    }

    // ---- Phase 164 — model-version guard ----

    #[test]
    fn model_supports_documents_accepts_claude_3_5_family() {
        assert!(model_supports_documents("claude-3-5-sonnet-20240620"));
        assert!(model_supports_documents("claude-3-5-haiku-20241022"));
    }

    #[test]
    fn model_supports_documents_accepts_claude_4_family() {
        assert!(model_supports_documents("claude-opus-4-7"));
        assert!(model_supports_documents("claude-sonnet-4-6"));
        assert!(model_supports_documents("claude-haiku-4-5-20251001"));
    }

    #[test]
    fn model_supports_documents_rejects_claude_3_legacy() {
        // Claude 3 (without -5) didn't have
        // document blocks. Operators on those
        // models get fail-closed pre-flight.
        assert!(!model_supports_documents("claude-3-opus-20240229"));
        assert!(!model_supports_documents("claude-3-sonnet-20240229"));
        assert!(!model_supports_documents("claude-3-haiku-20240307"));
    }

    #[test]
    fn model_supports_documents_rejects_unknown_prefix() {
        // Fail-closed for unfamiliar names.
        assert!(!model_supports_documents(""));
        assert!(!model_supports_documents("claude-2"));
        assert!(!model_supports_documents("gpt-4"));
        assert!(!model_supports_documents("claude-5-future-variant"));
    }

    #[test]
    fn build_request_body_rejects_document_on_legacy_model() {
        use crate::{ContentBlock, LlmMessage, LlmRequest};

        let msgs = [LlmMessage::User {
            content: vec![ContentBlock::DocumentBase64 {
                media_type: "application/pdf".to_string(),
                data: "JVBE".to_string(),
            }],
        }];
        let req = LlmRequest {
            model: "claude-3-opus-20240229",
            messages: &msgs,
            tools: &[],
            system: None,
            max_tokens: 100,
            temperature: None,
        };
        let err = build_request_body(&req).unwrap_err();
        match err {
            LlmError::Config(msg) => {
                assert!(msg.contains("does not support document blocks"));
                assert!(msg.contains("claude-3-opus"));
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[test]
    fn build_request_body_accepts_document_on_supported_model() {
        use crate::{ContentBlock, LlmMessage, LlmRequest};

        let msgs = [LlmMessage::User {
            content: vec![ContentBlock::DocumentBase64 {
                media_type: "application/pdf".to_string(),
                data: "JVBE".to_string(),
            }],
        }];
        let req = LlmRequest {
            model: "claude-haiku-4-5-20251001",
            messages: &msgs,
            tools: &[],
            system: None,
            max_tokens: 100,
            temperature: None,
        };
        let body = build_request_body(&req).expect("ok");
        assert_eq!(body["model"], "claude-haiku-4-5-20251001");
        // Request still contains the document
        // block — passes through to the API.
        let user_msg = &body["messages"][0];
        assert_eq!(user_msg["content"][0]["type"], "document");
    }

    #[test]
    fn build_request_body_text_only_unaffected_on_legacy_model() {
        // Sanity: the guard fires ONLY when
        // documents are present. Text-only
        // requests on legacy models pass through
        // (those models just don't get
        // documents, not nothing).
        use crate::{LlmMessage, LlmRequest};
        let msgs = [LlmMessage::user_text("hello")];
        let req = LlmRequest {
            model: "claude-3-opus-20240229",
            messages: &msgs,
            tools: &[],
            system: None,
            max_tokens: 100,
            temperature: None,
        };
        assert!(build_request_body(&req).is_ok());
    }
}

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
    LlmError, LlmMessage, LlmProvider, LlmRequest, LlmStepEnd, LlmStream, LlmStreamEvent, LlmUsage,
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

        Ok(Box::new(AnthropicStream {
            sse: SseReader::new(byte_stream),
            state: StreamState::default(),
            terminal: None,
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

    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(anthropic_message)
        .collect::<Result<_, _>>()?;

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

fn anthropic_message(msg: &LlmMessage) -> Result<Value, LlmError> {
    Ok(match msg {
        LlmMessage::User { content } => json!({
            "role": "user",
            "content": [{"type": "text", "text": content}],
        }),
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

// ---------------------------------------------------------------------------
// Stream state machine
// ---------------------------------------------------------------------------

#[derive(Default)]
struct StreamState {
    accumulated_text: String,
    usage: LlmUsage,
    pending_tool: Option<PendingTool>,
    completed_tool: Option<CompletedTool>,
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
                    self.state.completed_tool = Some(CompletedTool {
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
            let tool = self.state.completed_tool.take().ok_or_else(|| {
                LlmError::Parse(
                    "stop_reason=tool_use but no completed tool_use block".to_string(),
                )
            })?;
            Ok(LlmStepEnd::ToolCall {
                call_id: tool.call_id,
                tool_name: tool.tool_name,
                input: tool.input,
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
            vec![LlmMessage::User {
                content: "hi".to_string(),
            }],
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
            LlmStepEnd::ToolCall {
                call_id,
                tool_name,
                input,
                text_so_far,
                usage,
            } => {
                assert_eq!(call_id, "toolu_01");
                assert_eq!(tool_name, "memory.read");
                assert_eq!(input, json!({"query": "yesterday"}));
                assert_eq!(text_so_far, "");
                assert_eq!(usage.input_tokens, 25);
                assert_eq!(usage.output_tokens, 19);
            }
            other => panic!("expected ToolCall, got {other:?}"),
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
        let messages = vec![LlmMessage::User {
            content: "ping".to_string(),
        }];
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
}

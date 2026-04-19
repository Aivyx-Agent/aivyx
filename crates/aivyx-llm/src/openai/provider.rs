//! The `OpenAiProvider`: concrete `LlmProvider` against the OpenAI
//! Chat Completions streaming API (`/v1/chat/completions`).
//!
//! ## Wire format
//!
//! A streaming POST returns SSE frames as `data: {json}` lines:
//!
//! 1. Each chunk has `choices[0].delta` containing either
//!    `content` (text) or `tool_calls` (function call deltas).
//! 2. `tool_calls` stream as incremental `function.arguments`
//!    strings that must be concatenated.
//! 3. `finish_reason` appears on the final choice: `"stop"` for
//!    text, `"tool_calls"` for tool invocation.
//! 4. `data: [DONE]` terminates the stream.
//! 5. Usage appears in the final chunk when
//!    `stream_options.include_usage` is set.

use async_trait::async_trait;
use futures_util::StreamExt;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::{
    LlmError, LlmMessage, LlmProvider, LlmRequest, LlmStepEnd, LlmStream,
    LlmStreamEvent, LlmUsage,
};

use crate::transport::{ByteStream, HttpTransport, ReqwestTransport};

const DEFAULT_BASE_URL: &str = "https://api.openai.com";
/// Default Ollama base URL — standard port for `ollama serve`.
pub const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434";

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

pub struct OpenAiConfig {
    pub api_key: Option<SecretString>,
    pub base_url: Option<String>,
    /// When `true`, the `stream_options.include_usage` field is
    /// included in request bodies. Cloud OpenAI supports this;
    /// some Ollama versions may reject unknown fields. Default:
    /// `true`.
    pub include_stream_usage: bool,
}

impl OpenAiConfig {
    pub fn new(api_key: impl Into<SecretString>) -> Self {
        OpenAiConfig {
            api_key: Some(api_key.into()),
            base_url: None,
            include_stream_usage: true,
        }
    }

    /// Build a config with no API key. Used for local providers
    /// like Ollama that don't require authentication.
    pub fn without_api_key() -> Self {
        OpenAiConfig {
            api_key: None,
            base_url: None,
            include_stream_usage: false,
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    pub fn with_include_stream_usage(mut self, include: bool) -> Self {
        self.include_stream_usage = include;
        self
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

pub struct OpenAiProvider {
    config: OpenAiConfig,
    transport: Box<dyn HttpTransport>,
}

impl OpenAiProvider {
    pub fn new(config: OpenAiConfig) -> Result<Self, LlmError> {
        Ok(OpenAiProvider {
            config,
            transport: Box::new(ReqwestTransport::new()?),
        })
    }

    pub fn with_transport(
        config: OpenAiConfig,
        transport: Box<dyn HttpTransport>,
    ) -> Self {
        OpenAiProvider { config, transport }
    }

    fn endpoint(&self) -> String {
        let base = self
            .config
            .base_url
            .as_deref()
            .unwrap_or(DEFAULT_BASE_URL);
        format!("{base}/v1/chat/completions")
    }

    fn base_url(&self) -> &str {
        self.config
            .base_url
            .as_deref()
            .unwrap_or(DEFAULT_BASE_URL)
    }

    /// Lightweight health check against the provider's base URL.
    ///
    /// For Ollama, a GET to `http://localhost:11434` returns the
    /// plain-text body `"Ollama is running"`. This method checks
    /// reachability and returns a human-readable diagnostic:
    ///
    /// - `Ok(())` — the server responded (any 2xx).
    /// - `Err(msg)` — actionable error string suitable for display
    ///   to the user.
    pub async fn health_check(&self) -> Result<(), String> {
        let url = self.base_url();
        match self.transport.get_text(url).await {
            Ok(_body) => Ok(()),
            Err(LlmError::Transport(e)) => {
                // Connection refused, DNS failure, timeout, etc.
                Err(format!(
                    "cannot reach {url} — is Ollama running? \
                     Start it with `ollama serve`.\n  \
                     (transport error: {e})"
                ))
            }
            Err(LlmError::Api { status, message }) => {
                Err(format!(
                    "{url} returned HTTP {status}: {message}"
                ))
            }
            Err(other) => {
                Err(format!(
                    "health check against {url} failed: {other}"
                ))
            }
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn chat_stream(
        &self,
        request: LlmRequest<'_>,
        cancellation: &CancellationToken,
    ) -> Result<Box<dyn LlmStream>, LlmError> {
        let body = build_request_body(&request, self.config.include_stream_usage)?;
        let body_bytes = serde_json::to_vec(&body)
            .map_err(|e| LlmError::Parse(format!("request serialization: {e}")))?;

        let mut headers: Vec<(&str, &str)> = vec![
            ("content-type", "application/json"),
        ];
        // Only add the Authorization header if an API key is
        // configured. Ollama ignores it, but omitting it avoids
        // sending "Bearer " with an empty secret.
        let auth_header;
        if let Some(ref api_key) = self.config.api_key {
            auth_header = format!("Bearer {}", api_key.expose_secret());
            headers.push(("authorization", auth_header.as_str()));
        }

        let endpoint = self.endpoint();
        let byte_stream = self
            .transport
            .post_sse(&endpoint, &headers, body_bytes, cancellation)
            .await?;

        Ok(Box::new(OpenAiStream {
            stream: byte_stream,
            buf: Vec::with_capacity(4096),
            exhausted: false,
            state: StreamState::default(),
            terminal: None,
        }))
    }
}

// ---------------------------------------------------------------------------
// Request-body construction
// ---------------------------------------------------------------------------

fn build_request_body(
    request: &LlmRequest<'_>,
    include_stream_usage: bool,
) -> Result<Value, LlmError> {
    if request.model.is_empty() {
        return Err(LlmError::UnknownModel(String::new()));
    }

    let mut messages: Vec<Value> = Vec::new();
    if let Some(system) = request.system {
        messages.push(json!({"role": "system", "content": system}));
    }
    for msg in request.messages {
        messages.push(openai_message(msg)?);
    }

    let mut body = json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "messages": messages,
        "stream": true,
    });

    if include_stream_usage {
        body["stream_options"] = json!({"include_usage": true});
    }

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

    if let Some(temp) = request.temperature {
        body["temperature"] = json!(temp);
    }

    Ok(body)
}

fn openai_message(msg: &LlmMessage) -> Result<Value, LlmError> {
    Ok(match msg {
        LlmMessage::User { content } => json!({
            "role": "user",
            "content": content,
        }),
        LlmMessage::Assistant { text, tool_calls } => {
            let mut msg = json!({"role": "assistant"});
            if !text.is_empty() {
                msg["content"] = Value::String(text.clone());
            }
            if !tool_calls.is_empty() {
                let calls: Vec<Value> = tool_calls
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.call_id,
                            "type": "function",
                            "function": {
                                "name": c.tool_name,
                                "arguments": c.input.to_string(),
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
// Stream state machine
// ---------------------------------------------------------------------------

#[derive(Default)]
struct StreamState {
    accumulated_text: String,
    usage: LlmUsage,
    tool_call_id: Option<String>,
    tool_name: String,
    tool_arguments: String,
    finish_reason: Option<String>,
}

struct OpenAiStream {
    stream: ByteStream,
    buf: Vec<u8>,
    exhausted: bool,
    state: StreamState,
    terminal: Option<LlmStepEnd>,
}

#[async_trait]
impl LlmStream for OpenAiStream {
    async fn next_event(&mut self) -> Result<Option<LlmStreamEvent>, LlmError> {
        loop {
            if self.terminal.is_some() {
                return Ok(None);
            }
            match self.next_data_line().await? {
                Some(line) if line == "[DONE]" => {
                    self.terminal = Some(self.build_terminal()?);
                    return Ok(None);
                }
                Some(line) => {
                    if let Some(emit) = self.handle_chunk(&line)? {
                        return Ok(Some(emit));
                    }
                }
                None => {
                    self.terminal = Some(self.build_terminal()?);
                    return Ok(None);
                }
            }
        }
    }

    async fn finish(self: Box<Self>) -> Result<LlmStepEnd, LlmError> {
        self.terminal.ok_or_else(|| {
            LlmError::StreamEnded(
                "OpenAiStream::finish called before stream drained".to_string(),
            )
        })
    }
}

impl OpenAiStream {
    async fn next_data_line(&mut self) -> Result<Option<String>, LlmError> {
        loop {
            if let Some(line) = try_extract_data_line(&mut self.buf) {
                return Ok(Some(line));
            }
            if self.exhausted {
                return Ok(None);
            }
            match self.stream.next().await {
                Some(Ok(chunk)) => self.buf.extend_from_slice(&chunk),
                Some(Err(e)) => return Err(e),
                None => self.exhausted = true,
            }
        }
    }

    fn handle_chunk(&mut self, data: &str) -> Result<Option<LlmStreamEvent>, LlmError> {
        let chunk: ChatChunk = serde_json::from_str(data)
            .map_err(|e| LlmError::Parse(format!("OpenAI chunk JSON: {e}")))?;

        if let Some(usage) = chunk.usage {
            self.state.usage.input_tokens = usage.prompt_tokens.unwrap_or(0);
            self.state.usage.output_tokens = usage.completion_tokens.unwrap_or(0);
        }

        let Some(choice) = chunk.choices.into_iter().next() else {
            return Ok(None);
        };

        if let Some(reason) = choice.finish_reason {
            self.state.finish_reason = Some(reason);
        }

        if let Some(content) = choice.delta.content {
            self.state.accumulated_text.push_str(&content);
            return Ok(Some(LlmStreamEvent::TextChunk(content)));
        }

        if let Some(tool_calls) = choice.delta.tool_calls {
            for tc in tool_calls {
                if let Some(id) = tc.id {
                    self.state.tool_call_id = Some(id);
                }
                if let Some(func) = tc.function {
                    if let Some(name) = func.name {
                        self.state.tool_name = name;
                    }
                    if let Some(args) = func.arguments {
                        self.state.tool_arguments.push_str(&args);
                    }
                }
            }
        }

        Ok(None)
    }

    fn build_terminal(&mut self) -> Result<LlmStepEnd, LlmError> {
        let usage = self.state.usage;
        let reason = self.state.finish_reason.as_deref().unwrap_or("stop");

        if reason == "tool_calls" {
            let input: Value = if self.state.tool_arguments.is_empty() {
                json!({})
            } else {
                serde_json::from_str(&self.state.tool_arguments).map_err(|e| {
                    LlmError::Parse(format!("tool arguments JSON: {e}"))
                })?
            };
            Ok(LlmStepEnd::ToolCall {
                call_id: self.state.tool_call_id.take().unwrap_or_default(),
                tool_name: std::mem::take(&mut self.state.tool_name),
                input,
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

fn try_extract_data_line(buf: &mut Vec<u8>) -> Option<String> {
    let s = std::str::from_utf8(buf).ok()?;
    for (i, line) in s.split('\n').enumerate() {
        let trimmed = line.trim();
        if let Some(data) = trimmed.strip_prefix("data:") {
            let data = data.strip_prefix(' ').unwrap_or(data);
            let result = data.to_string();
            let consumed = s.split('\n')
                .take(i + 1)
                .map(|l| l.len() + 1)
                .sum::<usize>();
            buf.drain(..consumed.min(buf.len()));
            return Some(result);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// OpenAI wire-format structs (minimal — only the fields we read)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ChatChunk {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<ChunkUsage>,
}

#[derive(Deserialize)]
struct ChunkChoice {
    delta: ChunkDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChunkDelta {
    content: Option<String>,
    tool_calls: Option<Vec<ChunkToolCall>>,
}

#[derive(Deserialize)]
struct ChunkToolCall {
    id: Option<String>,
    function: Option<ChunkFunction>,
}

#[derive(Deserialize)]
struct ChunkFunction {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct ChunkUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use crate::transport::ByteStream;
    use crate::{LlmToolCallRecord, LlmToolDescriptor};
    use futures_util::stream;

    fn stream_from(chunks: Vec<&'static str>) -> ByteStream {
        let iter = chunks
            .into_iter()
            .map(|s| Ok::<Bytes, LlmError>(Bytes::from_static(s.as_bytes())));
        Box::pin(stream::iter(iter))
    }

    struct FakeTransport {
        sse_bytes: Vec<u8>,
    }

    impl FakeTransport {
        fn new(sse: &str) -> Self {
            FakeTransport {
                sse_bytes: sse.as_bytes().to_vec(),
            }
        }
    }

    #[async_trait]
    impl HttpTransport for FakeTransport {
        async fn post_sse(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
            _body: Vec<u8>,
            _cancellation: &CancellationToken,
        ) -> Result<ByteStream, LlmError> {
            let chunk = Bytes::from(self.sse_bytes.clone());
            Ok(Box::pin(stream::once(async move { Ok(chunk) })))
        }
    }

    fn test_provider(sse: &str) -> OpenAiProvider {
        let config = OpenAiConfig::new("sk-test");
        OpenAiProvider::with_transport(config, Box::new(FakeTransport::new(sse)))
    }

    fn simple_request() -> (Vec<LlmMessage>, Vec<LlmToolDescriptor>) {
        (
            vec![LlmMessage::User {
                content: "hello".into(),
            }],
            vec![],
        )
    }

    #[tokio::test]
    async fn text_response_streams_and_finishes() {
        let sse = "\
data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}\n\n\
data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2}}\n\n\
data: [DONE]\n\n";

        let provider = test_provider(sse);
        let (msgs, tools) = simple_request();
        let req = LlmRequest {
            model: "gpt-4",
            system: None,
            messages: &msgs,
            tools: &tools,
            max_tokens: 1000,
            temperature: None,
        };
        let cancel = CancellationToken::new();
        let mut stream = provider.chat_stream(req, &cancel).await.unwrap();

        let ev1 = stream.next_event().await.unwrap().unwrap();
        assert!(matches!(ev1, LlmStreamEvent::TextChunk(ref t) if t == "Hello"));

        let ev2 = stream.next_event().await.unwrap().unwrap();
        assert!(matches!(ev2, LlmStreamEvent::TextChunk(ref t) if t == " world"));

        assert!(stream.next_event().await.unwrap().is_none());

        let end = stream.finish().await.unwrap();
        match end {
            LlmStepEnd::FinalMessage { text, usage } => {
                assert_eq!(text, "Hello world");
                assert_eq!(usage.input_tokens, 10);
                assert_eq!(usage.output_tokens, 2);
            }
            _ => panic!("expected FinalMessage"),
        }
    }

    #[tokio::test]
    async fn tool_call_response_assembles_arguments() {
        let sse = "\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"id\":\"call_abc\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"function\":{\"arguments\":\"{\\\"loc\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"function\":{\"arguments\":\"ation\\\": \\\"SF\\\"}\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":8}}\n\n\
data: [DONE]\n\n";

        let provider = test_provider(sse);
        let (msgs, tools) = simple_request();
        let req = LlmRequest {
            model: "gpt-4",
            system: None,
            messages: &msgs,
            tools: &tools,
            max_tokens: 1000,
            temperature: None,
        };
        let cancel = CancellationToken::new();
        let mut stream = provider.chat_stream(req, &cancel).await.unwrap();

        while stream.next_event().await.unwrap().is_some() {}

        let end = stream.finish().await.unwrap();
        match end {
            LlmStepEnd::ToolCall {
                call_id,
                tool_name,
                input,
                usage,
                ..
            } => {
                assert_eq!(call_id, "call_abc");
                assert_eq!(tool_name, "get_weather");
                assert_eq!(input["location"], "SF");
                assert_eq!(usage.input_tokens, 5);
                assert_eq!(usage.output_tokens, 8);
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[tokio::test]
    async fn request_body_includes_system_as_message() {
        let (msgs, _) = simple_request();
        let req = LlmRequest {
            model: "gpt-4",
            system: Some("you are helpful"),
            messages: &msgs,
            tools: &[],
            max_tokens: 100,
            temperature: None,
        };
        let body = build_request_body(&req, true).unwrap();
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "you are helpful");
        assert_eq!(messages[1]["role"], "user");
    }

    #[tokio::test]
    async fn request_body_includes_tools_as_functions() {
        let (msgs, _) = simple_request();
        let tools = vec![LlmToolDescriptor {
            name: "read_file".into(),
            description: "Read a file".into(),
            input_schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        }];
        let req = LlmRequest {
            model: "gpt-4",
            system: None,
            messages: &msgs,
            tools: &tools,
            max_tokens: 100,
            temperature: None,
        };
        let body = build_request_body(&req, true).unwrap();
        let tool_arr = body["tools"].as_array().unwrap();
        assert_eq!(tool_arr.len(), 1);
        assert_eq!(tool_arr[0]["type"], "function");
        assert_eq!(tool_arr[0]["function"]["name"], "read_file");
    }

    #[tokio::test]
    async fn tool_result_message_maps_correctly() {
        let msg = LlmMessage::ToolResult {
            call_id: "call_xyz".into(),
            content: "file contents here".into(),
            is_error: false,
        };
        let val = openai_message(&msg).unwrap();
        assert_eq!(val["role"], "tool");
        assert_eq!(val["tool_call_id"], "call_xyz");
        assert_eq!(val["content"], "file contents here");
    }

    #[tokio::test]
    async fn assistant_with_tool_calls_round_trips() {
        let msg = LlmMessage::Assistant {
            text: "Let me check".into(),
            tool_calls: vec![LlmToolCallRecord {
                call_id: "call_1".into(),
                tool_name: "search".into(),
                input: json!({"q": "test"}),
            }],
        };
        let val = openai_message(&msg).unwrap();
        assert_eq!(val["role"], "assistant");
        assert_eq!(val["content"], "Let me check");
        let tcs = val["tool_calls"].as_array().unwrap();
        assert_eq!(tcs[0]["id"], "call_1");
        assert_eq!(tcs[0]["function"]["name"], "search");
    }

    // ---- Ollama / optional API key tests --------------------------------

    #[test]
    fn without_api_key_config_has_none_key_and_no_stream_usage() {
        let cfg = OpenAiConfig::without_api_key();
        assert!(cfg.api_key.is_none());
        assert!(!cfg.include_stream_usage);
    }

    #[test]
    fn with_api_key_config_has_some_key_and_stream_usage() {
        let cfg = OpenAiConfig::new("sk-test");
        assert!(cfg.api_key.is_some());
        assert!(cfg.include_stream_usage);
    }

    #[tokio::test]
    async fn request_body_omits_stream_options_when_disabled() {
        let (msgs, _) = simple_request();
        let req = LlmRequest {
            model: "llama3.1",
            system: None,
            messages: &msgs,
            tools: &[],
            max_tokens: 2048,
            temperature: None,
        };
        let body = build_request_body(&req, false).unwrap();
        assert!(
            body.get("stream_options").is_none(),
            "stream_options must be absent when include_stream_usage is false: {body}"
        );
    }

    #[tokio::test]
    async fn request_body_includes_stream_options_when_enabled() {
        let (msgs, _) = simple_request();
        let req = LlmRequest {
            model: "gpt-4",
            system: None,
            messages: &msgs,
            tools: &[],
            max_tokens: 1000,
            temperature: None,
        };
        let body = build_request_body(&req, true).unwrap();
        assert!(
            body.get("stream_options").is_some(),
            "stream_options must be present when include_stream_usage is true: {body}"
        );
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn default_ollama_base_url_is_localhost_11434() {
        assert_eq!(DEFAULT_OLLAMA_BASE_URL, "http://localhost:11434");
    }

    // ---- Health-check tests -----------------------------------------------

    struct HealthyTransport;

    #[async_trait]
    impl HttpTransport for HealthyTransport {
        async fn post_sse(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
            _body: Vec<u8>,
            _cancellation: &CancellationToken,
        ) -> Result<ByteStream, LlmError> {
            unreachable!("post_sse should not be called during health check");
        }

        async fn get_text(&self, _url: &str) -> Result<String, LlmError> {
            Ok("Ollama is running".to_string())
        }
    }

    struct UnreachableTransport;

    #[async_trait]
    impl HttpTransport for UnreachableTransport {
        async fn post_sse(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
            _body: Vec<u8>,
            _cancellation: &CancellationToken,
        ) -> Result<ByteStream, LlmError> {
            unreachable!();
        }

        async fn get_text(&self, _url: &str) -> Result<String, LlmError> {
            Err(LlmError::Transport(
                "connection refused".to_string(),
            ))
        }
    }

    #[tokio::test]
    async fn health_check_ok_when_server_responds() {
        let cfg = OpenAiConfig::without_api_key()
            .with_base_url("http://localhost:11434");
        let provider = OpenAiProvider::with_transport(
            cfg,
            Box::new(HealthyTransport),
        );
        assert!(provider.health_check().await.is_ok());
    }

    #[tokio::test]
    async fn health_check_returns_actionable_error_on_connection_refused() {
        let cfg = OpenAiConfig::without_api_key()
            .with_base_url("http://localhost:11434");
        let provider = OpenAiProvider::with_transport(
            cfg,
            Box::new(UnreachableTransport),
        );
        let err = provider.health_check().await.unwrap_err();
        assert!(
            err.contains("ollama serve"),
            "error should mention `ollama serve`: {err}"
        );
        assert!(
            err.contains("localhost:11434"),
            "error should mention the URL: {err}"
        );
    }

    #[test]
    fn ollama_config_endpoint_uses_ollama_base_url() {
        let cfg = OpenAiConfig::without_api_key()
            .with_base_url(DEFAULT_OLLAMA_BASE_URL);
        let provider = OpenAiProvider::with_transport(
            cfg,
            Box::new(FakeTransport::new("")),
        );
        assert_eq!(
            provider.endpoint(),
            "http://localhost:11434/v1/chat/completions"
        );
    }

    #[tokio::test]
    async fn ollama_style_text_response_without_usage() {
        // Ollama often omits usage fields entirely and may not
        // send stream_options. This test verifies the provider
        // handles a response that has no usage block gracefully.
        let sse = "\
data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\n\
data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
data: [DONE]\n\n";

        let cfg = OpenAiConfig::without_api_key()
            .with_base_url("http://localhost:11434");
        let provider = OpenAiProvider::with_transport(
            cfg,
            Box::new(FakeTransport::new(sse)),
        );
        let (msgs, tools) = simple_request();
        let req = LlmRequest {
            model: "llama3.1",
            system: None,
            messages: &msgs,
            tools: &tools,
            max_tokens: 2048,
            temperature: None,
        };
        let cancel = CancellationToken::new();
        let mut stream = provider.chat_stream(req, &cancel).await.unwrap();

        let ev = stream.next_event().await.unwrap().unwrap();
        assert!(matches!(ev, LlmStreamEvent::TextChunk(ref t) if t == "Hi"));
        assert!(stream.next_event().await.unwrap().is_none());

        let end = stream.finish().await.unwrap();
        match end {
            LlmStepEnd::FinalMessage { text, usage } => {
                assert_eq!(text, "Hi");
                // Usage is zero when not provided — not an error.
                assert_eq!(usage.input_tokens, 0);
                assert_eq!(usage.output_tokens, 0);
            }
            _ => panic!("expected FinalMessage"),
        }
    }
}

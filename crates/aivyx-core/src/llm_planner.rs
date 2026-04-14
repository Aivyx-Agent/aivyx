//! LLM-backed [`TurnPlanner`].
//!
//! [`LlmPlanner`] is the Phase 2 counterpart to Phase 1's `VecPlanner`.
//! Instead of walking a pre-recorded script, it holds an
//! `Arc<dyn LlmProvider>` (from `aivyx-llm`) and drives the turn loop
//! by asking the provider for the next step on every call.
//!
//! ## What it does, in order
//!
//! 1. `begin_turn(message)` — seeds the conversation history with a
//!    single `LlmMessage::User` containing the message text.
//! 2. `next_step(...)` — builds an `LlmRequest` from the current
//!    history + system prompt + tool descriptors, calls
//!    `provider.chat_stream(...)`, drains the mid-stream
//!    `LlmStreamEvent::TextChunk` events while relaying each to
//!    `channel.stream_event(StreamEvent::Text(chunk))`, then calls
//!    `stream.finish()` to obtain the terminal `LlmStepEnd`.
//! 3. On `LlmStepEnd::FinalMessage { text, .. }` — appends an
//!    `Assistant { text, tool_calls: [] }` entry to the history and
//!    returns [`NextStep::FinalMessage`].
//! 4. On `LlmStepEnd::ToolCall { .. }` — resolves the tool by name in
//!    the registry, appends an `Assistant { text: text_so_far,
//!    tool_calls: [record] }` entry, remembers the pending `call_id`,
//!    and returns [`NextStep::ToolCall`]. If the tool name is unknown,
//!    the planner synthesizes a `tool_result` error and recursively
//!    asks the provider for another step so the LLM can recover.
//! 5. `observe_tool_outcome(tool_id, outcome)` — consumes the pending
//!    call_id, serializes the outcome into the structured `tool_result`
//!    content (see [`render_tool_result`]), and appends it to history.
//!
//! ## Conversation-history ownership
//!
//! The planner owns the `Vec<LlmMessage>` mutably across the whole turn.
//! One planner instance = one turn — the `ConcreteAgent` factory
//! produces a fresh planner per `Agent::turn` call, so concurrent turns
//! never share history.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use aivyx_llm::{
    LlmError, LlmMessage, LlmProvider, LlmRequest, LlmStepEnd, LlmStream, LlmStreamEvent,
    LlmToolCallRecord, LlmToolDescriptor,
};

use crate::planner::{NextStep, StepObservation, ToolRegistry, TurnPlanner};
use crate::{ChannelContext, Message, MessageContent, StreamEvent, ToolId, ToolOutcome};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// All the knobs the LLM planner needs at construction time. Separated
/// from [`LlmPlanner::new`] so callers can build one at config-parse
/// time and reuse it, and so new fields can land without churning the
/// constructor signature.
#[derive(Debug, Clone)]
pub struct LlmPlannerConfig {
    /// Provider-specific model id, e.g. `"claude-haiku-4-5-20251001"`.
    pub model: String,
    /// Optional system prompt. `None` means the provider's default
    /// (usually empty) is used.
    pub system_prompt: Option<String>,
    /// Max output tokens per step.
    pub max_tokens: u32,
    /// Optional sampling temperature; `None` means provider default.
    pub temperature: Option<f32>,
}

impl LlmPlannerConfig {
    pub fn new(model: impl Into<String>) -> Self {
        LlmPlannerConfig {
            model: model.into(),
            system_prompt: None,
            max_tokens: 1024,
            temperature: None,
        }
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }
}

// ---------------------------------------------------------------------------
// Planner
// ---------------------------------------------------------------------------

/// LLM-backed [`TurnPlanner`]. Built once per turn — the planner factory
/// on [`crate::ConcreteAgent`] constructs a fresh instance whose
/// conversation history starts empty.
pub struct LlmPlanner {
    provider: Arc<dyn LlmProvider>,
    registry: Arc<ToolRegistry>,
    config: LlmPlannerConfig,
    tools: Vec<LlmToolDescriptor>,
    history: Vec<LlmMessage>,
    pending_call_id: Option<String>,
}

impl LlmPlanner {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        registry: Arc<ToolRegistry>,
        config: LlmPlannerConfig,
    ) -> Self {
        let tools = registry
            .iter_tools()
            .map(|tool| LlmToolDescriptor {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                input_schema: tool.input_schema().clone(),
            })
            .collect();

        LlmPlanner {
            provider,
            registry,
            config,
            tools,
            history: Vec::new(),
            pending_call_id: None,
        }
    }

    /// Inspect the conversation history. Test-only: the planner owns
    /// the history internally, but tests need to assert on its contents
    /// after tool observations.
    pub fn history(&self) -> &[LlmMessage] {
        &self.history
    }

    /// Build one `LlmRequest` from the current history + config and
    /// drain the provider's stream, returning the terminal value.
    /// Relays every `TextChunk` to the channel as a `StreamEvent::Text`.
    async fn one_step(
        &self,
        channel: &dyn ChannelContext,
    ) -> Result<LlmStepEnd, LlmError> {
        let request = LlmRequest {
            model: self.config.model.as_str(),
            system: self.config.system_prompt.as_deref(),
            messages: &self.history,
            tools: &self.tools,
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
        };

        let cancellation = channel.cancellation_token();
        let mut stream: Box<dyn LlmStream> =
            self.provider.chat_stream(request, &cancellation).await?;

        // Race the stream's next event against cancellation. When the
        // cancel future wins, we drop the stream immediately (dropping
        // a Box<dyn LlmStream> propagates through the provider's
        // internal body stream and aborts the underlying connection)
        // and return `LlmError::Cancelled`. The turn loop's own
        // post-next_step cancellation check then takes over and emits
        // `LoopOutcome::Cancelled` / `LoopOutcome::TimedOut` as
        // appropriate. Phase 3 task 4 added this path so wall-clock
        // timeouts actually interrupt a completion mid-token.
        loop {
            let next = tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    drop(stream);
                    return Err(LlmError::Cancelled);
                }
                event = stream.next_event() => event?,
            };

            let Some(event) = next else { break };

            if let LlmStreamEvent::TextChunk(ref chunk) = event {
                // Best-effort relay: if the channel rejects the event,
                // we log it in the sense of "drop it on the floor" —
                // the LLM stream still has to be drained or we leak the
                // outbound connection. Channel errors are terminal for
                // the turn loop, not for the stream.
                let _ = channel.stream_event(StreamEvent::Text(chunk)).await;
            }
            // `Usage` events are currently ignored — usage is carried
            // on the terminal value. This matches the provider's
            // documented contract.
        }

        stream.finish().await
    }
}

#[async_trait]
impl TurnPlanner for LlmPlanner {
    async fn begin_turn(&mut self, message: &Message) {
        let content = match &message.content {
            MessageContent::Text(text) => text.clone(),
        };
        self.history.push(LlmMessage::User { content });
        self.pending_call_id = None;
    }

    async fn next_step(
        &mut self,
        _observed: &[StepObservation],
        channel: &dyn ChannelContext,
    ) -> NextStep {
        // Defensive: if `begin_turn` was never called (a non-
        // ConcreteAgent caller drove us manually), seed with an empty
        // user message rather than sending a tool-less message list —
        // Anthropic rejects zero-message requests.
        if self.history.is_empty() {
            self.history.push(LlmMessage::User {
                content: String::new(),
            });
        }

        // Loop so we can synthesize a recovery step if the LLM picks a
        // tool name we don't recognize.
        loop {
            let terminal = match self.one_step(channel).await {
                Ok(t) => t,
                Err(LlmError::Cancelled) => {
                    // Mid-stream cancellation (either an external
                    // signal or a wall-clock timeout firing on the
                    // channel's token). Return `NextStep::Stop` so the
                    // turn loop's own post-next_step cancellation
                    // re-check takes over and translates to
                    // `LoopOutcome::Cancelled` / `TimedOut`. Returning
                    // a FinalMessage here would misleadingly show up
                    // as a completed turn.
                    return NextStep::Stop;
                }
                Err(e) => {
                    // Any other provider error terminates the turn
                    // cleanly from the loop's perspective. We surface
                    // it as a FinalMessage carrying the error text so
                    // audit still sees a Completed turn. A future
                    // enhancement could plumb `AivyxError::Llm`
                    // through a new NextStep variant, but that's a
                    // bigger change.
                    return NextStep::FinalMessage(format!("LLM error: {e}"));
                }
            };

            match terminal {
                LlmStepEnd::FinalMessage { text, .. } => {
                    self.history.push(LlmMessage::Assistant {
                        text: text.clone(),
                        tool_calls: Vec::new(),
                    });
                    return NextStep::FinalMessage(text);
                }
                LlmStepEnd::ToolCall {
                    call_id,
                    tool_name,
                    input,
                    text_so_far,
                    ..
                } => {
                    let record = LlmToolCallRecord {
                        call_id: call_id.clone(),
                        tool_name: tool_name.clone(),
                        input: input.clone(),
                    };
                    self.history.push(LlmMessage::Assistant {
                        text: text_so_far,
                        tool_calls: vec![record],
                    });

                    match self.registry.find_by_name(&tool_name) {
                        Some(tool_id) => {
                            self.pending_call_id = Some(call_id);
                            return NextStep::ToolCall { tool_id, input };
                        }
                        None => {
                            // Unknown tool — synthesize an error
                            // tool_result, append it to history, and
                            // ask the provider for another step. This
                            // gives the LLM a chance to recover
                            // (pick a different tool or give up).
                            self.history.push(LlmMessage::ToolResult {
                                call_id,
                                content: json!({
                                    "error": "unknown_tool",
                                    "message": format!("tool '{tool_name}' is not registered"),
                                })
                                .to_string(),
                                is_error: true,
                            });
                            // Fall through to the loop's next iteration
                            // so we call the provider again with the
                            // updated history.
                            continue;
                        }
                    }
                }
            }
        }
    }

    async fn observe_tool_outcome(
        &mut self,
        _tool_id: ToolId,
        outcome: &ToolOutcome,
    ) {
        // `pending_call_id` is set by the most recent ToolCall return;
        // if it's None, either `begin_turn` wasn't called or the turn
        // loop invoked us out of order. Either way, synthesize a stable
        // id so the history stays well-formed.
        let call_id = self
            .pending_call_id
            .take()
            .unwrap_or_else(|| "unknown-call".to_string());

        let (content, is_error) = render_tool_result(outcome);
        self.history.push(LlmMessage::ToolResult {
            call_id,
            content,
            is_error,
        });
    }
}

// ---------------------------------------------------------------------------
// Tool-result rendering
// ---------------------------------------------------------------------------

/// Serialize a [`ToolOutcome`] into the `(content, is_error)` pair that
/// goes into an [`LlmMessage::ToolResult`].
///
/// Successful outcomes emit the tool's `output` verbatim as compact
/// JSON — whatever the tool produced, the LLM sees. Failure outcomes
/// use a stable structured envelope so future additions don't break
/// existing agents:
///
/// ```json
/// { "error": "<kind>", "message": "<detail>" }
/// ```
///
/// The `error` field is one of: `denied`, `failed`, `timed_out`,
/// `requires_escalation`. It is stable across versions; new kinds land
/// as new strings, never as renames.
fn render_tool_result(outcome: &ToolOutcome) -> (String, bool) {
    match outcome {
        ToolOutcome::Completed { output, .. } => {
            // Emit the output as-is. If the tool's output happens to be
            // `{"error": ...}` we leave that alone — that's the tool's
            // responsibility. Verification state is not propagated
            // because an unverified success is still a successful
            // return per D1, and audit is authoritative for verify.
            let content = serde_json::to_string(output)
                .unwrap_or_else(|_| "<unserializable output>".to_string());
            (content, false)
        }
        ToolOutcome::Denied { scope, .. } => {
            let envelope = json!({
                "error": "denied",
                "message": format!("scope {scope} not granted"),
            });
            (envelope.to_string(), true)
        }
        ToolOutcome::RequiresEscalation { reason } => {
            let envelope = json!({
                "error": "requires_escalation",
                "message": reason,
            });
            (envelope.to_string(), true)
        }
        ToolOutcome::Failed(err) => {
            let envelope = json!({
                "error": "failed",
                "message": err.to_string(),
            });
            (envelope.to_string(), true)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use serde_json::{json, Value};

    use aivyx_capability::{Scope, TrustTier};
    use aivyx_llm::LlmUsage;

    use crate::planner::NextStep;
    use crate::{
        AivyxError, ChannelError, ChannelPlatform, SessionId, Tool, ToolContext, ToolId,
        ToolOutcome, TurnOutcome, Verification,
    };

    // -----------------------------------------------------------------------
    // A minimal FakeLlmProvider built from a script of (events, terminal)
    // pairs — one per expected `chat_stream` call. Deliberately rebuilt
    // here rather than imported from `aivyx-llm`'s test module.
    // -----------------------------------------------------------------------

    struct FakeLlmProvider {
        script: Mutex<std::collections::VecDeque<FakeStep>>,
    }

    struct FakeStep {
        events: Vec<LlmStreamEvent>,
        terminal: LlmStepEnd,
    }

    impl FakeLlmProvider {
        fn new(steps: Vec<FakeStep>) -> Arc<Self> {
            Arc::new(FakeLlmProvider {
                script: Mutex::new(steps.into()),
            })
        }
    }

    #[async_trait]
    impl LlmProvider for FakeLlmProvider {
        async fn chat_stream(
            &self,
            _request: LlmRequest<'_>,
            _cancellation: &crate::CancellationToken,
        ) -> Result<Box<dyn LlmStream>, LlmError> {
            let step = self
                .script
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| LlmError::Config("FakeLlmProvider exhausted".to_string()))?;
            Ok(Box::new(FakeStream {
                events: step.events.into_iter(),
                terminal: Some(step.terminal),
            }))
        }
    }

    struct FakeStream {
        events: std::vec::IntoIter<LlmStreamEvent>,
        terminal: Option<LlmStepEnd>,
    }

    #[async_trait]
    impl LlmStream for FakeStream {
        async fn next_event(&mut self) -> Result<Option<LlmStreamEvent>, LlmError> {
            Ok(self.events.next())
        }
        async fn finish(self: Box<Self>) -> Result<LlmStepEnd, LlmError> {
            self.terminal
                .ok_or_else(|| LlmError::StreamEnded("double finish".to_string()))
        }
    }

    // -----------------------------------------------------------------------
    // FakeChannel records streamed text so we can assert the planner
    // relayed tokens as they arrived.
    // -----------------------------------------------------------------------

    struct RecChannel {
        session: SessionId,
        token: crate::CancellationToken,
        streamed: Mutex<Vec<String>>,
    }

    impl RecChannel {
        fn new() -> Self {
            RecChannel {
                session: SessionId::new(),
                token: crate::CancellationToken::new(),
                streamed: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl ChannelContext for RecChannel {
        fn channel_name(&self) -> &str {
            "rec"
        }
        fn platform(&self) -> ChannelPlatform {
            ChannelPlatform::Local
        }
        fn trust_tier(&self) -> TrustTier {
            TrustTier::Trusted
        }
        fn session_id(&self) -> SessionId {
            self.session
        }
        async fn stream_event(&self, event: StreamEvent<'_>) -> Result<(), ChannelError> {
            if let StreamEvent::Text(s) = event {
                self.streamed.lock().unwrap().push(s.to_string());
            }
            Ok(())
        }
        async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), ChannelError> {
            Ok(())
        }
        fn cancellation_token(&self) -> crate::CancellationToken {
            self.token.clone()
        }
    }

    // -----------------------------------------------------------------------
    // FakeTool (reused shape from agent.rs tests, trimmed).
    // -----------------------------------------------------------------------

    struct FakeTool {
        id: ToolId,
        name: &'static str,
        schema: Value,
    }

    impl FakeTool {
        fn new(name: &'static str) -> Self {
            FakeTool {
                id: ToolId::new(),
                name,
                schema: json!({"type": "object"}),
            }
        }
    }

    #[async_trait]
    impl Tool for FakeTool {
        fn id(&self) -> ToolId {
            self.id
        }
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "fake"
        }
        fn input_schema(&self) -> &Value {
            &self.schema
        }
        fn required_scope(&self, _input: &Value) -> Scope {
            Scope::parse("memory.read").unwrap()
        }
        async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
            ToolOutcome::Completed {
                output: json!({"ok": true}),
                verified: Verification::NotApplicable,
            }
        }
    }

    fn zero_usage() -> LlmUsage {
        LlmUsage::default()
    }

    // -----------------------------------------------------------------------
    // Tests proper
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn final_message_path_returns_next_step_and_appends_history() {
        let script = vec![FakeStep {
            events: vec![
                LlmStreamEvent::TextChunk("he".to_string()),
                LlmStreamEvent::TextChunk("llo".to_string()),
            ],
            terminal: LlmStepEnd::FinalMessage {
                text: "hello".to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("claude-haiku-4-5-20251001"),
        );

        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "hi"))
            .await;
        let step = planner.next_step(&[], &channel).await;
        assert!(matches!(step, NextStep::FinalMessage(ref m) if m == "hello"));

        // Streamed chunks relayed to the channel in order.
        let streamed = channel.streamed.lock().unwrap().clone();
        assert_eq!(streamed, vec!["he".to_string(), "llo".to_string()]);

        // History: User("hi") → Assistant("hello", no tool calls).
        let hist = planner.history();
        assert_eq!(hist.len(), 2);
        assert!(matches!(
            hist[0],
            LlmMessage::User { ref content } if content == "hi"
        ));
        match &hist[1] {
            LlmMessage::Assistant { text, tool_calls } => {
                assert_eq!(text, "hello");
                assert!(tool_calls.is_empty());
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_call_path_resolves_name_via_registry_and_appends_history() {
        let tool = Arc::new(FakeTool::new("memory.read"));
        let tool_id = tool.id();

        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::ToolCall {
                call_id: "toolu_01".to_string(),
                tool_name: "memory.read".to_string(),
                input: json!({"query": "yesterday"}),
                text_so_far: String::new(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![tool]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("claude-haiku-4-5-20251001"),
        );

        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "recall"))
            .await;
        let step = planner.next_step(&[], &channel).await;
        match step {
            NextStep::ToolCall {
                tool_id: returned,
                input,
            } => {
                assert_eq!(returned, tool_id);
                assert_eq!(input, json!({"query": "yesterday"}));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }

        // History has the assistant tool_use message recorded.
        let hist = planner.history();
        match &hist[1] {
            LlmMessage::Assistant { tool_calls, .. } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].call_id, "toolu_01");
                assert_eq!(tool_calls[0].tool_name, "memory.read");
            }
            other => panic!("expected Assistant at index 1, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn observe_tool_outcome_appends_success_result_to_history() {
        let tool = Arc::new(FakeTool::new("memory.read"));
        let tool_id = tool.id();

        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::ToolCall {
                call_id: "toolu_42".to_string(),
                tool_name: "memory.read".to_string(),
                input: json!({}),
                text_so_far: String::new(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![tool]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("claude-haiku-4-5-20251001"),
        );

        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "go"))
            .await;
        let _ = planner.next_step(&[], &channel).await;

        let outcome = ToolOutcome::Completed {
            output: json!({"found": 3, "items": ["a", "b", "c"]}),
            verified: Verification::NotApplicable,
        };
        planner.observe_tool_outcome(tool_id, &outcome).await;

        let last = planner.history().last().unwrap();
        match last {
            LlmMessage::ToolResult {
                call_id,
                content,
                is_error,
            } => {
                assert_eq!(call_id, "toolu_42");
                assert!(!is_error);
                // The content should round-trip back to the original output.
                let parsed: Value = serde_json::from_str(content).unwrap();
                assert_eq!(parsed, json!({"found": 3, "items": ["a", "b", "c"]}));
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn observe_tool_outcome_serializes_denied_as_structured_error() {
        let tool = Arc::new(FakeTool::new("shell.exec"));
        let tool_id = tool.id();

        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::ToolCall {
                call_id: "toolu_denied".to_string(),
                tool_name: "shell.exec".to_string(),
                input: json!({}),
                text_so_far: String::new(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![tool]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("claude-haiku-4-5-20251001"),
        );

        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "run stuff"))
            .await;
        let _ = planner.next_step(&[], &channel).await;

        let outcome = ToolOutcome::Denied {
            scope: Scope::parse("shell.exec:rm").unwrap(),
            held: aivyx_capability::CapabilitySet::from_scopes([]),
        };
        planner.observe_tool_outcome(tool_id, &outcome).await;

        let last = planner.history().last().unwrap();
        match last {
            LlmMessage::ToolResult {
                content, is_error, ..
            } => {
                assert!(*is_error);
                let parsed: Value = serde_json::from_str(content).unwrap();
                assert_eq!(parsed["error"], "denied");
                assert!(
                    parsed["message"]
                        .as_str()
                        .unwrap()
                        .contains("shell.exec:rm")
                );
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_tool_name_synthesizes_error_and_retries() {
        // Script: first chat_stream returns ToolCall with an unknown
        // name; second chat_stream returns a FinalMessage. Planner
        // should NOT surface an error — it should append the synthetic
        // error tool_result and loop internally.
        let known = Arc::new(FakeTool::new("memory.read"));
        let script = vec![
            FakeStep {
                events: vec![],
                terminal: LlmStepEnd::ToolCall {
                    call_id: "toolu_bad".to_string(),
                    tool_name: "does.not.exist".to_string(),
                    input: json!({}),
                    text_so_far: String::new(),
                    usage: zero_usage(),
                },
            },
            FakeStep {
                events: vec![],
                terminal: LlmStepEnd::FinalMessage {
                    text: "giving up".to_string(),
                    usage: zero_usage(),
                },
            },
        ];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![known]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("claude-haiku-4-5-20251001"),
        );

        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "help"))
            .await;
        let step = planner.next_step(&[], &channel).await;
        assert!(matches!(step, NextStep::FinalMessage(ref m) if m == "giving up"));

        // The history should contain the synthetic error tool_result.
        let has_unknown = planner.history().iter().any(|m| match m {
            LlmMessage::ToolResult {
                content, is_error, ..
            } => *is_error && content.contains("unknown_tool"),
            _ => false,
        });
        assert!(has_unknown, "expected a synthetic unknown_tool entry");
    }

    #[tokio::test]
    async fn provider_error_surfaces_as_final_message() {
        let script: Vec<FakeStep> = vec![]; // immediately exhausted
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("claude-haiku-4-5-20251001"),
        );

        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "hi"))
            .await;
        let step = planner.next_step(&[], &channel).await;
        match step {
            NextStep::FinalMessage(m) => assert!(m.starts_with("LLM error:")),
            other => panic!("expected FinalMessage, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Mid-stream cancellation — Phase 3 task 4. The planner's one_step
    // loop races stream events against `cancellation.cancelled()`. When
    // the channel's token flips to cancelled while the stream is still
    // yielding, the planner must drop the stream, surface
    // `LlmError::Cancelled`, and `next_step` must translate that into
    // `NextStep::Stop` (not a FinalMessage — doing so would misleadingly
    // complete the turn).
    // -----------------------------------------------------------------------

    /// Provider whose stream blocks forever on `next_event`. The only
    /// way a turn that uses it can terminate is via cancellation of the
    /// channel's token.
    struct BlockingProvider;

    #[async_trait]
    impl LlmProvider for BlockingProvider {
        async fn chat_stream(
            &self,
            _request: LlmRequest<'_>,
            _cancellation: &crate::CancellationToken,
        ) -> Result<Box<dyn LlmStream>, LlmError> {
            Ok(Box::new(BlockingStream))
        }
    }

    struct BlockingStream;

    #[async_trait]
    impl LlmStream for BlockingStream {
        async fn next_event(&mut self) -> Result<Option<LlmStreamEvent>, LlmError> {
            // Never resolves. The `tokio::select!` in `one_step` must
            // always pick the cancellation branch to let the caller
            // make progress.
            std::future::pending::<()>().await;
            unreachable!()
        }
        async fn finish(self: Box<Self>) -> Result<LlmStepEnd, LlmError> {
            // finish() shouldn't be reached on the cancel path, but if
            // it is, report it loudly so the test catches the misroute.
            Err(LlmError::StreamEnded("BlockingStream::finish reached".into()))
        }
    }

    #[tokio::test]
    async fn mid_stream_cancel_returns_next_step_stop() {
        let provider = Arc::new(BlockingProvider);
        let registry = Arc::new(ToolRegistry::new(vec![]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("claude-haiku-4-5-20251001"),
        );

        let channel = RecChannel::new();
        // Spawn a task that cancels the channel's token shortly after
        // the planner starts draining the stream. Yielding once
        // guarantees we enter `one_step` before the cancel fires.
        let token = channel.token.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            token.cancel();
        });

        planner
            .begin_turn(&Message::text(channel.session, "hi"))
            .await;
        let step = planner.next_step(&[], &channel).await;

        assert!(
            matches!(step, NextStep::Stop),
            "mid-stream cancel must surface as NextStep::Stop, got {step:?}"
        );
    }

    #[tokio::test]
    async fn failed_outcome_produces_failed_envelope() {
        // Direct unit test of render_tool_result — no planner needed.
        let outcome = ToolOutcome::Failed(AivyxError::Internal("boom".to_string()));
        let (content, is_error) = render_tool_result(&outcome);
        assert!(is_error);
        let parsed: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["error"], "failed");
        assert!(parsed["message"].as_str().unwrap().contains("boom"));
    }
}

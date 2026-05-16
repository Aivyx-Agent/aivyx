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

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use aivyx_llm::{
    ContentBlock, LlmError, LlmMessage, LlmProvider, LlmRequest, LlmStepEnd, LlmStream,
    LlmStreamEvent, LlmToolCallRecord, LlmToolDescriptor, LlmUsage,
};

use crate::planner::{NextStep, StepObservation, ToolCallRequest, ToolRegistry, TurnPlanner};
use crate::{
    ChannelContext, ContentPart, Message, MessageContent, StreamEvent, ToolId, ToolOutcome,
};

// ---------------------------------------------------------------------------
// PruneSink — callback for persisting pruned context
// ---------------------------------------------------------------------------

/// Receives a summary of pruned messages when context-window pruning
/// fires. Implementations live in the channel layer (which has access
/// to `Memory`); the core crate defines only the contract.
#[async_trait]
pub trait PruneSink: Send + Sync {
    /// Called once per pruning event with the number of messages
    /// removed and a human-readable summary of their content.
    async fn on_prune(&self, session_id: crate::SessionId, pruned_count: usize, summary: &str);
}

// ---------------------------------------------------------------------------
// ContextProvider — per-turn automatic recall hook (Phase 76)
// ---------------------------------------------------------------------------

/// Read-side sibling of [`PruneSink`]: invoked once per turn with the
/// user's message, returning an already-formatted context block to
/// prepend, or `None` for "nothing relevant — leave the turn
/// untouched."
///
/// The concrete implementation lives in the channel layer (which has
/// access to `Memory` + the embedding provider); the core crate
/// defines only the contract. Mirrors the `PruneSink` pattern.
///
/// Phase 76 deliberately does **not** re-export this trait from
/// `aivyx-core`'s `lib.rs` (unlike the older `PruneSink`): consumers
/// reach it via `aivyx_core::llm_planner::ContextProvider`. Keeping
/// `lib.rs` byte-identical protects the production-core streak; the
/// minor re-export asymmetry is the documented, intentional price.
#[async_trait]
pub trait ContextProvider: Send + Sync {
    /// Return a formatted, injection-safe context block to prepend to
    /// this turn, or `None` to leave the turn unchanged. Must never
    /// panic and must swallow its own errors into `None` (the
    /// universal no-op path — recall is best-effort, never fatal).
    ///
    /// Phase 77 — `session_id` is the turn's conversation id,
    /// passed so an implementation can persist a recall-feedback
    /// event correlated to the turn (the reflection loop pairs it
    /// against the audit chain's per-session `TurnEnded`). It does
    /// not influence what is recalled.
    async fn recall(
        &self,
        user_message: &str,
        session_id: crate::SessionId,
    ) -> Option<String>;
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// All the knobs the LLM planner needs at construction time. Separated
/// from [`LlmPlanner::new`] so callers can build one at config-parse
/// time and reuse it, and so new fields can land without churning the
/// constructor signature.
#[derive(Clone)]
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
    /// Phase 11 Task 4 — role-derived tool allowlist. When `Some`,
    /// `LlmPlanner::new` filters the registry's tool list through
    /// this set before sending the catalog to the provider. The
    /// filtered-out tools are never advertised to the model, so
    /// the model never tries to call them — **this** is the
    /// primary enforcement. The dispatch-layer check in
    /// `ConcreteAgent::run_tool_call` is belt-and-suspenders for
    /// tool calls that bypass advertisement (stale tool_use
    /// blocks on resumed conversations, non-LLM planners, etc.).
    ///
    /// `None` means "no filter — advertise every registered
    /// tool," preserving Phase 6–10 behavior for planners built
    /// without a role.
    pub tool_allowlist: Option<std::collections::BTreeSet<String>>,
    /// Phase 43 Task 2 — context window size in tokens. Used by the
    /// pruning layer to decide when to drop old history messages.
    /// Defaults per provider: 200_000 (Anthropic), 128_000 (OpenAI).
    /// `None` disables pruning entirely.
    pub context_window_tokens: Option<usize>,
    /// Phase 43 Task 4 — optional callback invoked when messages are
    /// pruned. The channel layer provides an implementation backed by
    /// `Memory::put()` to persist pruned context for later reflection.
    /// `None` means pruned messages are silently discarded.
    pub prune_sink: Option<Arc<dyn PruneSink>>,
    /// Phase 76 — optional automatic-recall hook. When `Some`,
    /// `begin_turn` calls it with the user's message and prepends
    /// any returned block to that turn's context. `None` means no
    /// auto-recall (pre-Phase-76 behavior exactly).
    pub context_provider: Option<Arc<dyn ContextProvider>>,
}

impl std::fmt::Debug for LlmPlannerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmPlannerConfig")
            .field("model", &self.model)
            .field("system_prompt", &self.system_prompt)
            .field("max_tokens", &self.max_tokens)
            .field("temperature", &self.temperature)
            .field("tool_allowlist", &self.tool_allowlist)
            .field("context_window_tokens", &self.context_window_tokens)
            .field("prune_sink", &self.prune_sink.as_ref().map(|_| ".."))
            .field(
                "context_provider",
                &self.context_provider.as_ref().map(|_| ".."),
            )
            .finish()
    }
}

impl LlmPlannerConfig {
    pub fn new(model: impl Into<String>) -> Self {
        LlmPlannerConfig {
            model: model.into(),
            system_prompt: None,
            max_tokens: 1024,
            temperature: None,
            tool_allowlist: None,
            context_window_tokens: None,
            prune_sink: None,
            context_provider: None,
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

    /// Set the context window size in tokens. When set, the planner
    /// prunes old history messages before each LLM call if the
    /// estimated token count exceeds 80% of this value.
    pub fn with_context_window(mut self, tokens: usize) -> Self {
        self.context_window_tokens = Some(tokens);
        self
    }

    /// Attach a prune sink that receives summaries of pruned messages
    /// for persistence (e.g. to memory). See [`PruneSink`].
    pub fn with_prune_sink(mut self, sink: Arc<dyn PruneSink>) -> Self {
        self.prune_sink = Some(sink);
        self
    }

    /// Attach a [`ContextProvider`] for per-turn automatic recall.
    /// `None` (the default) preserves pre-Phase-76 behavior. Mirrors
    /// [`Self::with_prune_sink`].
    pub fn with_context_provider(
        mut self,
        provider: Arc<dyn ContextProvider>,
    ) -> Self {
        self.context_provider = Some(provider);
        self
    }

    /// Attach a role-derived tool allowlist. See
    /// [`Self::tool_allowlist`] for semantics. `None` preserves
    /// legacy behavior (allow all registered tools).
    pub fn with_tool_allowlist(
        mut self,
        allowlist: Option<std::collections::BTreeSet<String>>,
    ) -> Self {
        self.tool_allowlist = allowlist;
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
    pending_call_ids: VecDeque<String>,
    /// Cumulative token usage across all LLM steps in this turn.
    accumulated_usage: crate::TokenUsage,
    /// Running count of messages pruned during this turn for context
    /// window management (Phase 43).
    pruned_message_count: usize,
}

impl LlmPlanner {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        registry: Arc<ToolRegistry>,
        config: LlmPlannerConfig,
    ) -> Self {
        // Phase 11 Task 4 — role-allowlist filter on the advertised
        // tool catalog. When `config.tool_allowlist` is `Some`,
        // tools whose name is not in the set are not collected
        // into the descriptor list, so the provider request
        // (`request.tools`) never mentions them and the model
        // therefore never emits a tool_use block against them.
        // This is the primary enforcement point for the role
        // allowlist; see the dispatch-layer check in
        // `agent.rs::run_tool_call` for the belt-and-suspenders
        // safety net.
        let tools = registry
            .iter_tools()
            .filter(|tool| {
                config
                    .tool_allowlist
                    .as_ref()
                    .is_none_or(|set| set.contains(tool.name()))
            })
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
            pending_call_ids: VecDeque::new(),
            accumulated_usage: crate::TokenUsage::default(),
            pruned_message_count: 0,
        }
    }

    /// Inspect the conversation history. Test-only: the planner owns
    /// the history internally, but tests need to assert on its contents
    /// after tool observations.
    pub fn history(&self) -> &[LlmMessage] {
        &self.history
    }

    /// Number of messages pruned from conversation history during this
    /// turn to stay within the context window budget (Phase 43).
    pub fn pruned_message_count(&self) -> usize {
        self.pruned_message_count
    }

    /// Names of tools actually advertised to the provider — i.e. the
    /// post-filter catalog after `config.tool_allowlist` is applied.
    /// Tests assert on this to confirm the planner-layer allowlist
    /// filter is the *primary* enforcement point for Phase 11 roles
    /// (the dispatch-layer gate in `ConcreteAgent::run_tool_call` is
    /// the belt-and-suspenders). Returns names in registry iteration
    /// order.
    pub fn advertised_tool_names(&self) -> Vec<&str> {
        self.tools.iter().map(|t| t.name.as_str()).collect()
    }

    /// Add a step's usage to the running total.
    fn accumulate(&mut self, usage: LlmUsage) {
        self.accumulated_usage.input_tokens += usage.input_tokens;
        self.accumulated_usage.output_tokens += usage.output_tokens;
        self.accumulated_usage.cache_creation_input_tokens +=
            usage.cache_creation_input_tokens;
        self.accumulated_usage.cache_read_input_tokens +=
            usage.cache_read_input_tokens;
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
        let mut content = match &message.content {
            MessageContent::Text(text) => vec![ContentBlock::text(text)],
            MessageContent::Image { media_type, data } => {
                vec![ContentBlock::image_from_bytes(media_type, data)]
            }
            MessageContent::Mixed(parts) => parts
                .iter()
                .map(|part| match part {
                    ContentPart::Text(text) => ContentBlock::text(text),
                    ContentPart::Image { media_type, data } => {
                        ContentBlock::image_from_bytes(media_type, data)
                    }
                })
                .collect(),
        };

        // Phase 76 — automatic recall. Embed-and-retrieve is driven
        // by the user's *text* (Q2a: latest user message only). The
        // returned block is prepended as a distinct leading text
        // block *inside the same user message* rather than as its
        // own message: a separate message would risk provider
        // role-alternation rules, and folding it into the static
        // system prompt would make per-turn recall look like a
        // standing instruction. The provider's `recall` is
        // best-effort — a `None` (no provider, embed failure, empty
        // index, all-below-floor) leaves the turn byte-identical to
        // pre-Phase-76 behavior.
        if let Some(provider) = &self.config.context_provider {
            let query_text = match &message.content {
                MessageContent::Text(text) => text.clone(),
                MessageContent::Image { .. } => String::new(),
                MessageContent::Mixed(parts) => {
                    let joined: Vec<&str> = parts
                        .iter()
                        .filter_map(|p| match p {
                            ContentPart::Text(t) => Some(t.as_str()),
                            ContentPart::Image { .. } => None,
                        })
                        .collect();
                    joined.join(" ")
                }
            };
            if !query_text.trim().is_empty() {
                if let Some(block) = provider
                    .recall(&query_text, message.session_id)
                    .await
                {
                    content.insert(0, ContentBlock::text(block));
                }
            }
        }

        self.history.push(LlmMessage::User { content });
        self.pending_call_ids.clear();
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
            self.history.push(LlmMessage::user_text(""));
        }

        // Phase 43 Task 3 — context window pruning. If the estimated
        // token count exceeds 80% of the configured context window,
        // drop the oldest messages (preserving the most-recent tail)
        // and insert a sentinel so the model knows context was lost.
        if let Some(window) = self.config.context_window_tokens {
            let budget = window * 4 / 5; // 80% threshold
            let system_tokens = aivyx_llm::estimate_system_tokens(
                self.config.system_prompt.as_deref(),
            );
            let history_tokens = aivyx_llm::estimate_tokens(&self.history);
            let total = system_tokens + history_tokens;
            if total > budget && self.history.len() > 1 {
                // Record pre-pruning token count.
                self.accumulated_usage.context_tokens_before_pruning =
                    total as u32;

                // Keep at least the last message (the most recent user
                // turn or tool result). Prune from the front until we
                // fit, or until only one message remains.
                let target = budget.saturating_sub(system_tokens);
                let mut keep_from = self.history.len() - 1;
                let mut tail_tokens = aivyx_llm::estimate_tokens(&self.history[keep_from..]);
                // Grow the tail backwards while it still fits.
                while keep_from > 0 {
                    let candidate = keep_from - 1;
                    let candidate_tokens =
                        aivyx_llm::estimate_tokens(&self.history[candidate..candidate + 1]);
                    if tail_tokens + candidate_tokens > target {
                        break;
                    }
                    tail_tokens += candidate_tokens;
                    keep_from = candidate;
                }
                let pruned_count = keep_from;
                if pruned_count > 0 {
                    // Build a summary before draining, for the prune sink.
                    if let Some(ref sink) = self.config.prune_sink {
                        let summary = summarise_pruned(&self.history[..pruned_count]);
                        let sid = channel.session_id();
                        sink.on_prune(sid, pruned_count, &summary).await;
                    }
                    self.history.drain(..pruned_count);
                    self.history.insert(
                        0,
                        LlmMessage::user_text(format!(
                            "[Earlier context pruned: {pruned_count} messages removed \
                             to fit context window]"
                        )),
                    );
                    self.pruned_message_count += pruned_count;
                }

                // Record post-pruning token count.
                let after = system_tokens
                    + aivyx_llm::estimate_tokens(&self.history);
                self.accumulated_usage.context_tokens_after_pruning =
                    after as u32;
            }
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
                LlmStepEnd::FinalMessage { text, usage } => {
                    self.accumulate(usage);
                    self.history.push(LlmMessage::Assistant {
                        text: text.clone(),
                        tool_calls: Vec::new(),
                    });
                    return NextStep::FinalMessage(text);
                }
                LlmStepEnd::ToolCalls {
                    calls,
                    text_so_far,
                    usage,
                } => {
                    self.accumulate(usage);

                    // Build assistant message with all tool call records.
                    let records: Vec<LlmToolCallRecord> = calls
                        .iter()
                        .map(|c| LlmToolCallRecord {
                            call_id: c.call_id.clone(),
                            tool_name: c.tool_name.clone(),
                            input: c.input.clone(),
                        })
                        .collect();
                    self.history.push(LlmMessage::Assistant {
                        text: text_so_far,
                        tool_calls: records,
                    });

                    // Partition calls into known (dispatchable) and
                    // unknown (immediate error). Known calls get queued
                    // for execution; unknown ones get synthetic
                    // tool_result errors appended to history now.
                    let mut batch: Vec<ToolCallRequest> = Vec::new();

                    for call in calls {
                        match self.registry.find_by_name(&call.tool_name) {
                            Some(tool_id) => {
                                self.pending_call_ids.push_back(call.call_id);
                                batch.push(ToolCallRequest {
                                    tool_id,
                                    input: call.input,
                                });
                            }
                            None => {
                                self.history.push(LlmMessage::ToolResult {
                                    call_id: call.call_id,
                                    content: json!({
                                        "error": "unknown_tool",
                                        "message": format!(
                                            "tool '{}' is not registered",
                                            call.tool_name
                                        ),
                                    })
                                    .to_string(),
                                    is_error: true,
                                });
                            }
                        }
                    }

                    if batch.is_empty() {
                        // All tools unknown — loop to retry the LLM
                        // with the error results in history.
                        continue;
                    }

                    if batch.len() == 1 {
                        // Single known tool — use the singular path.
                        let req = batch.into_iter().next().unwrap();
                        return NextStep::ToolCall {
                            tool_id: req.tool_id,
                            input: req.input,
                        };
                    }

                    // Multiple known tools — batch dispatch.
                    return NextStep::ToolCalls(batch);
                }
            }
        }
    }

    async fn observe_tool_outcome(
        &mut self,
        _tool_id: ToolId,
        outcome: &ToolOutcome,
    ) {
        // `pending_call_ids` is populated by the most recent ToolCall(s)
        // return; if empty, either `begin_turn` wasn't called or the turn
        // loop invoked us out of order. Synthesize a stable id so the
        // history stays well-formed.
        let call_id = self
            .pending_call_ids
            .pop_front()
            .unwrap_or_else(|| "unknown-call".to_string());

        let (content, is_error) = render_tool_result(outcome);
        self.history.push(LlmMessage::ToolResult {
            call_id,
            content,
            is_error,
        });
    }

    fn turn_usage(&self) -> crate::TokenUsage {
        self.accumulated_usage
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
        ToolOutcome::NotInRole { tool_name } => {
            let envelope = json!({
                "error": "not_in_role",
                "message": format!("tool {tool_name} is not in the active role's allowlist"),
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

/// Build a compact summary of pruned messages for the prune sink.
/// Truncates each message to avoid storing massive tool results
/// verbatim in memory — the point is orientation, not replay.
fn summarise_pruned(messages: &[LlmMessage]) -> String {
    use std::fmt::Write;
    let mut buf = String::new();
    for (i, msg) in messages.iter().enumerate() {
        if i > 0 {
            buf.push('\n');
        }
        match msg {
            LlmMessage::User { content } => {
                let parts: Vec<&str> = content
                    .iter()
                    .map(|b| match b {
                        ContentBlock::Text { text } => text.as_str(),
                        ContentBlock::ImageBase64 { media_type, .. } => media_type.as_str(),
                    })
                    .collect();
                let summary = parts.join(", ");
                let _ = write!(buf, "[user] {}", truncate(&summary, 200));
            }
            LlmMessage::Assistant { text, tool_calls } => {
                let _ = write!(buf, "[assistant] {}", truncate(text, 200));
                for tc in tool_calls {
                    let _ = write!(buf, "\n  tool_call: {}", tc.tool_name);
                }
            }
            LlmMessage::ToolResult {
                call_id, content, ..
            } => {
                let _ = write!(
                    buf,
                    "[tool_result call_id={call_id}] {}",
                    truncate(content, 200)
                );
            }
        }
    }
    buf
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        // Find a char boundary at or before `max`.
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_llm::ToolCallEnd;
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
            LlmMessage::User { ref content } if content == &[ContentBlock::text("hi")]
        ));
        match &hist[1] {
            LlmMessage::Assistant { text, tool_calls } => {
                assert_eq!(text, "hello");
                assert!(tool_calls.is_empty());
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    // ---- Phase 76 — ContextProvider auto-recall hook -----------

    struct FakeContextProvider {
        block: Option<String>,
        seen: std::sync::Mutex<Vec<String>>,
        seen_sessions: std::sync::Mutex<Vec<SessionId>>,
    }

    impl FakeContextProvider {
        fn new(block: Option<&str>) -> Arc<Self> {
            Arc::new(Self {
                block: block.map(str::to_string),
                seen: std::sync::Mutex::new(Vec::new()),
                seen_sessions: std::sync::Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl ContextProvider for FakeContextProvider {
        async fn recall(
            &self,
            user_message: &str,
            session_id: SessionId,
        ) -> Option<String> {
            self.seen.lock().unwrap().push(user_message.to_string());
            self.seen_sessions.lock().unwrap().push(session_id);
            self.block.clone()
        }
    }

    fn bare_planner(config: LlmPlannerConfig) -> LlmPlanner {
        LlmPlanner::new(
            FakeLlmProvider::new(vec![]),
            Arc::new(ToolRegistry::new(vec![])),
            config,
        )
    }

    #[tokio::test]
    async fn context_provider_prepends_recalled_block() {
        let provider = FakeContextProvider::new(Some(
            "## Relevant context (auto-recalled)\n- [notes] purple",
        ));
        let mut planner = bare_planner(
            LlmPlannerConfig::new("m")
                .with_context_provider(provider.clone()),
        );
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "what's my color?"))
            .await;

        // The query handed to recall is the raw user text.
        assert_eq!(
            provider.seen.lock().unwrap().clone(),
            vec!["what's my color?".to_string()]
        );
        // Phase 77 — begin_turn threads the message's session id
        // through so the impl can correlate a recall-feedback
        // event to this turn.
        assert_eq!(
            provider.seen_sessions.lock().unwrap().clone(),
            vec![channel.session]
        );
        // History user message: recalled block FIRST, then the
        // user's own text — one message, two content blocks.
        match &planner.history()[0] {
            LlmMessage::User { content } => {
                assert_eq!(content.len(), 2);
                assert_eq!(
                    content[0],
                    ContentBlock::text(
                        "## Relevant context (auto-recalled)\n\
                         - [notes] purple"
                    )
                );
                assert_eq!(
                    content[1],
                    ContentBlock::text("what's my color?")
                );
            }
            other => panic!("expected User, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn context_provider_none_leaves_turn_unchanged() {
        let provider = FakeContextProvider::new(None);
        let mut planner = bare_planner(
            LlmPlannerConfig::new("m")
                .with_context_provider(provider.clone()),
        );
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "hi there"))
            .await;
        // recall consulted, returned None → turn byte-identical.
        assert_eq!(provider.seen.lock().unwrap().len(), 1);
        assert!(matches!(
            planner.history()[0],
            LlmMessage::User { ref content }
                if content == &[ContentBlock::text("hi there")]
        ));
    }

    #[tokio::test]
    async fn no_context_provider_is_unchanged() {
        // Regression guard: the default config path must not change.
        let mut planner = bare_planner(LlmPlannerConfig::new("m"));
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "hello"))
            .await;
        assert!(matches!(
            planner.history()[0],
            LlmMessage::User { ref content }
                if content == &[ContentBlock::text("hello")]
        ));
    }

    #[tokio::test]
    async fn context_provider_skipped_for_blank_query() {
        // Whitespace-only text must not consult the provider (no
        // point embedding empty input) and must not inject a block
        // even if the provider would return one.
        let provider = FakeContextProvider::new(Some("SHOULD-NOT-APPEAR"));
        let mut planner = bare_planner(
            LlmPlannerConfig::new("m")
                .with_context_provider(provider.clone()),
        );
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "   "))
            .await;
        assert!(provider.seen.lock().unwrap().is_empty());
        assert!(matches!(
            planner.history()[0],
            LlmMessage::User { ref content }
                if content == &[ContentBlock::text("   ")]
        ));
    }

    #[tokio::test]
    async fn tool_call_path_resolves_name_via_registry_and_appends_history() {
        let tool = Arc::new(FakeTool::new("memory.read"));
        let tool_id = tool.id();

        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::ToolCalls {
                calls: vec![ToolCallEnd {
                    call_id: "toolu_01".to_string(),
                    tool_name: "memory.read".to_string(),
                    input: json!({"query": "yesterday"}),
                }],
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
            terminal: LlmStepEnd::ToolCalls {
                calls: vec![ToolCallEnd {
                    call_id: "toolu_42".to_string(),
                    tool_name: "memory.read".to_string(),
                    input: json!({}),
                }],
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
            terminal: LlmStepEnd::ToolCalls {
                calls: vec![ToolCallEnd {
                    call_id: "toolu_denied".to_string(),
                    tool_name: "shell.exec".to_string(),
                    input: json!({}),
                }],
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
            held: aivyx_capability::CapabilitySet::empty(),
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
                terminal: LlmStepEnd::ToolCalls {
                    calls: vec![ToolCallEnd {
                        call_id: "toolu_bad".to_string(),
                        tool_name: "does.not.exist".to_string(),
                        input: json!({}),
                    }],
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

    // -----------------------------------------------------------------------
    // Phase 40 — multi-tool ToolCalls produces NextStep::ToolCalls batch
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn multi_tool_calls_returns_next_step_tool_calls_batch() {
        let tool_a = Arc::new(FakeTool::new("fs.read"));
        let tool_b = Arc::new(FakeTool::new("memory.read"));
        let tool_a_id = tool_a.id();
        let tool_b_id = tool_b.id();

        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::ToolCalls {
                calls: vec![
                    ToolCallEnd {
                        call_id: "toolu_a".to_string(),
                        tool_name: "fs.read".to_string(),
                        input: json!({"path": "/x"}),
                    },
                    ToolCallEnd {
                        call_id: "toolu_b".to_string(),
                        tool_name: "memory.read".to_string(),
                        input: json!({"topic": "notes"}),
                    },
                ],
                text_so_far: String::new(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![tool_a, tool_b]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("claude-haiku-4-5-20251001"),
        );

        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "do both"))
            .await;
        let step = planner.next_step(&[], &channel).await;

        match step {
            NextStep::ToolCalls(batch) => {
                assert_eq!(batch.len(), 2);
                assert_eq!(batch[0].tool_id, tool_a_id);
                assert_eq!(batch[0].input, json!({"path": "/x"}));
                assert_eq!(batch[1].tool_id, tool_b_id);
                assert_eq!(batch[1].input, json!({"topic": "notes"}));
            }
            other => panic!("expected ToolCalls batch, got {other:?}"),
        }

        // Verify history: assistant has 2 tool_calls.
        let hist = planner.history();
        match &hist[1] {
            LlmMessage::Assistant { tool_calls, .. } => {
                assert_eq!(tool_calls.len(), 2);
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn multi_tool_with_unknown_executes_known_and_errors_unknown() {
        // 3 calls: 2 known, 1 unknown. Should return ToolCalls with the
        // 2 known tools and append a synthetic error ToolResult for the unknown.
        let tool_a = Arc::new(FakeTool::new("fs.read"));
        let tool_b = Arc::new(FakeTool::new("memory.read"));
        let tool_a_id = tool_a.id();
        let tool_b_id = tool_b.id();

        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::ToolCalls {
                calls: vec![
                    ToolCallEnd {
                        call_id: "toolu_a".to_string(),
                        tool_name: "fs.read".to_string(),
                        input: json!({}),
                    },
                    ToolCallEnd {
                        call_id: "toolu_bad".to_string(),
                        tool_name: "does.not.exist".to_string(),
                        input: json!({}),
                    },
                    ToolCallEnd {
                        call_id: "toolu_b".to_string(),
                        tool_name: "memory.read".to_string(),
                        input: json!({}),
                    },
                ],
                text_so_far: String::new(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![tool_a, tool_b]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("claude-haiku-4-5-20251001"),
        );

        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "go"))
            .await;
        let step = planner.next_step(&[], &channel).await;

        match step {
            NextStep::ToolCalls(batch) => {
                assert_eq!(batch.len(), 2);
                assert_eq!(batch[0].tool_id, tool_a_id);
                assert_eq!(batch[1].tool_id, tool_b_id);
            }
            other => panic!("expected ToolCalls batch, got {other:?}"),
        }

        // The unknown tool's error result is already in history.
        let hist = planner.history();
        let tool_result = &hist[2]; // index 0 = user, 1 = assistant, 2 = tool_result
        match tool_result {
            LlmMessage::ToolResult {
                call_id, is_error, ..
            } => {
                assert_eq!(call_id, "toolu_bad");
                assert!(is_error);
            }
            other => panic!("expected ToolResult for unknown tool, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Phase 43 Task 2 — context window config
    // -----------------------------------------------------------------------

    #[test]
    fn context_window_defaults_to_none() {
        let config = LlmPlannerConfig::new("test-model");
        assert_eq!(config.context_window_tokens, None);
    }

    #[test]
    fn context_window_builder() {
        let config = LlmPlannerConfig::new("test-model")
            .with_context_window(200_000);
        assert_eq!(config.context_window_tokens, Some(200_000));
    }

    // -----------------------------------------------------------------------
    // Phase 43 Task 3 — context window pruning
    // -----------------------------------------------------------------------

    /// Helper: build a planner with a tiny context window, pre-seed
    /// history with known messages, then call `next_step` so the
    /// pruning logic runs.
    fn make_pruning_planner(
        window_tokens: usize,
        messages: Vec<LlmMessage>,
        reply: &str,
    ) -> (LlmPlanner, RecChannel) {
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::FinalMessage {
                text: reply.to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![]));
        let config = LlmPlannerConfig::new("test")
            .with_context_window(window_tokens);
        let mut planner = LlmPlanner::new(provider, registry, config);
        planner.history = messages;
        (planner, RecChannel::new())
    }

    #[tokio::test]
    async fn pruning_skipped_when_under_budget() {
        // 5 short messages, generous context window — no pruning.
        let msgs: Vec<LlmMessage> = (0..5)
            .map(|i| LlmMessage::user_text(format!("msg{i}")))
            .collect();
        let (mut planner, ch) = make_pruning_planner(200_000, msgs, "ok");
        planner.next_step(&[], &ch).await;
        assert_eq!(planner.pruned_message_count(), 0);
        // 5 original + 1 assistant reply (no sentinel inserted).
        assert_eq!(planner.history().len(), 6);
    }

    #[tokio::test]
    async fn pruning_drops_oldest_when_over_budget() {
        // Each "x".repeat(100) message ≈ 25 tokens.
        // 10 messages ≈ 250 tokens. Set window to 200 → budget = 160.
        // Pruning should drop some messages.
        let msgs: Vec<LlmMessage> = (0..10)
            .map(|i| LlmMessage::user_text(format!("message-{i}-{}", "x".repeat(100))))
            .collect();
        let (mut planner, ch) = make_pruning_planner(200, msgs, "ok");
        planner.next_step(&[], &ch).await;
        assert!(planner.pruned_message_count() > 0);
        // First message in history should be the sentinel.
        match &planner.history()[0] {
            LlmMessage::User { content } => {
                let text = match &content[0] {
                    ContentBlock::Text { text } => text,
                    other => panic!("expected Text block, got {other:?}"),
                };
                assert!(text.contains("[Earlier context pruned:"));
            }
            other => panic!("expected User sentinel, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pruning_preserves_at_least_last_message() {
        // Extremely small window (10 tokens = 40 chars). Even a
        // single message exceeds budget, but we never prune the last
        // message. Two messages in: we should prune one and keep one
        // (plus sentinel).
        let msgs = vec![
            LlmMessage::user_text("a]".repeat(50)), // ~25 tokens
            LlmMessage::user_text("b".repeat(200)),  // ~50 tokens
        ];
        let (mut planner, ch) = make_pruning_planner(10, msgs, "ok");
        planner.next_step(&[], &ch).await;
        assert_eq!(planner.pruned_message_count(), 1);
        // History: sentinel + last-original + assistant-reply = 3.
        assert_eq!(planner.history().len(), 3);
    }

    #[tokio::test]
    async fn pruning_skipped_when_no_context_window() {
        // No context window configured → pruning never triggers.
        let msgs: Vec<LlmMessage> = (0..20)
            .map(|_| LlmMessage::user_text("x".repeat(1000)))
            .collect();
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::FinalMessage {
                text: "ok".to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![]));
        let config = LlmPlannerConfig::new("test"); // no .with_context_window()
        let mut planner = LlmPlanner::new(provider, registry, config);
        planner.history = msgs;
        let ch = RecChannel::new();
        planner.next_step(&[], &ch).await;
        assert_eq!(planner.pruned_message_count(), 0);
        // 20 original + 1 reply
        assert_eq!(planner.history().len(), 21);
    }

    #[tokio::test]
    async fn pruning_accumulates_across_steps() {
        // Two LLM calls in one turn (tool call → final). Each call
        // prunes. We verify the counter accumulates.
        let tool = Arc::new(FakeTool::new("echo"));
        let tool_id = tool.id();
        let script = vec![
            FakeStep {
                events: vec![],
                terminal: LlmStepEnd::ToolCalls {
                    calls: vec![ToolCallEnd {
                        call_id: "c1".to_string(),
                        tool_name: "echo".to_string(),
                        input: json!({}),
                    }],
                    text_so_far: String::new(),
                    usage: zero_usage(),
                },
            },
            FakeStep {
                events: vec![],
                terminal: LlmStepEnd::FinalMessage {
                    text: "done".to_string(),
                    usage: zero_usage(),
                },
            },
        ];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![tool]));
        // Tiny window ensures pruning fires on both calls.
        let config = LlmPlannerConfig::new("test")
            .with_context_window(60);
        let mut planner = LlmPlanner::new(provider, registry, config);
        // Seed with enough bulk to trigger pruning.
        for i in 0..8 {
            planner.history.push(LlmMessage::user_text(
                format!("bulk-{i}-{}", "y".repeat(80)),
            ));
        }
        let ch = RecChannel::new();
        // First call — tool call.
        let step = planner.next_step(&[], &ch).await;
        assert!(matches!(step, NextStep::ToolCall { .. }));
        let first_pruned = planner.pruned_message_count();
        assert!(first_pruned > 0, "should have pruned on first step");
        // Observe tool result, adding more content.
        planner.observe_tool_outcome(
            tool_id,
            &ToolOutcome::Completed {
                output: json!({"data": "x".repeat(100)}),
                verified: Verification::NotApplicable,
            },
        ).await;
        // Second call — final message.
        let step = planner.next_step(&[], &ch).await;
        assert!(matches!(step, NextStep::FinalMessage(_)));
        assert!(
            planner.pruned_message_count() >= first_pruned,
            "counter should accumulate"
        );
    }

    // -----------------------------------------------------------------------
    // Phase 43 Task 5 — TokenUsage pruning fields
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn turn_usage_reports_pruning_tokens() {
        // Over-budget history triggers pruning and populates the
        // before/after token fields in TokenUsage.
        let msgs: Vec<LlmMessage> = (0..10)
            .map(|i| LlmMessage::user_text(format!("msg-{i}-{}", "x".repeat(100))))
            .collect();
        let (mut planner, ch) = make_pruning_planner(200, msgs, "ok");
        planner.next_step(&[], &ch).await;

        let usage = planner.turn_usage();
        assert!(
            usage.context_tokens_before_pruning > 0,
            "before should be populated when pruning fires"
        );
        assert!(
            usage.context_tokens_after_pruning > 0,
            "after should be populated when pruning fires"
        );
        assert!(
            usage.context_tokens_after_pruning < usage.context_tokens_before_pruning,
            "after < before when messages were pruned"
        );
    }

    #[tokio::test]
    async fn turn_usage_zeroes_when_no_pruning() {
        // Under-budget — pruning doesn't fire, fields stay zero.
        let msgs = vec![LlmMessage::user_text("short")];
        let (mut planner, ch) = make_pruning_planner(200_000, msgs, "ok");
        planner.next_step(&[], &ch).await;

        let usage = planner.turn_usage();
        assert_eq!(usage.context_tokens_before_pruning, 0);
        assert_eq!(usage.context_tokens_after_pruning, 0);
    }

    // -----------------------------------------------------------------------
    // Phase 45 Task 3 — begin_turn with multimodal MessageContent
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn begin_turn_image_to_content_block() {
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::FinalMessage {
                text: "I see an image".to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![]));
        let config = LlmPlannerConfig::new("test");
        let mut planner = LlmPlanner::new(provider, registry, config);

        let session = crate::SessionId::new();
        let msg = Message::image(session, "image/png", vec![0x89, 0x50]);
        planner.begin_turn(&msg).await;

        let hist = planner.history();
        assert_eq!(hist.len(), 1);
        match &hist[0] {
            LlmMessage::User { content } => {
                assert_eq!(content.len(), 1);
                assert!(content[0].is_image());
            }
            other => panic!("expected User, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn begin_turn_mixed_content() {
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::FinalMessage {
                text: "ok".to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![]));
        let config = LlmPlannerConfig::new("test");
        let mut planner = LlmPlanner::new(provider, registry, config);

        let session = crate::SessionId::new();
        let msg = Message::text_with_image(session, "describe this", "image/jpeg", vec![0xFF]);
        planner.begin_turn(&msg).await;

        let hist = planner.history();
        assert_eq!(hist.len(), 1);
        match &hist[0] {
            LlmMessage::User { content } => {
                assert_eq!(content.len(), 2);
                assert!(matches!(&content[0], ContentBlock::Text { text } if text == "describe this"));
                assert!(content[1].is_image());
            }
            other => panic!("expected User, got {other:?}"),
        }
    }
}

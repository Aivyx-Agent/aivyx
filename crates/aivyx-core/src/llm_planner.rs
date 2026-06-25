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
    LlmStreamEvent, LlmToolCallRecord, LlmToolDescriptor, LlmUsage, ToolCallEnd,
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

/// Phase 79 — per-turn system-prompt refiner. Sibling of
/// [`ContextProvider`]: invoked in `begin_turn` with the user's
/// message, it may return a replacement system prompt for *this
/// turn only*, or `None` to leave the planner's base prompt
/// untouched (the universal byte-identical fallback path).
///
/// Used by the adaptive-Persona refiner, which selects the
/// Persona facets relevant to the turn instead of injecting the
/// whole accreted Soul every time. Like every hook in this
/// module it is **not** re-exported from `aivyx-core`'s
/// `lib.rs` — consumers reach it via
/// `aivyx_core::llm_planner::SystemPromptRefiner`. Keeping
/// `lib.rs` byte-identical protects the production-core streak;
/// the minor re-export asymmetry is the documented, intentional
/// price (same rationale as `ContextProvider`).
#[async_trait]
pub trait SystemPromptRefiner: Send + Sync {
    /// Return a replacement system prompt for this turn, or
    /// `None` to keep the planner's base prompt unchanged. Must
    /// never panic and must swallow its own errors into `None`
    /// (best-effort — refinement is never fatal).
    ///
    /// Phase 86 — `session_id` is provided so implementations
    /// that consult a per-session conversational window (the
    /// recall-window-relevance work) can locate the right
    /// recent-turns buffer. Mirrors `ContextProvider::recall`,
    /// which already carries `session_id`.
    ///
    /// Phase 117 — `base_prompt` is the planner's currently
    /// configured base system prompt. Implementations that
    /// produce a refined prompt from scratch (Phase 79's
    /// adaptive Persona refiner) ignore this argument; ones
    /// that EXTEND the base (Phase 117's relevance refiner)
    /// can compose their output as `base_prompt +
    /// addendum`. The default value behaviour for callers
    /// that don't carry the base is the empty string.
    async fn refine(
        &self,
        user_message: &str,
        session_id: crate::SessionId,
        base_prompt: &str,
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
    /// Phase 79 — optional per-turn system-prompt refiner. When
    /// `Some`, `begin_turn` calls it with the user's message and
    /// (on `Some`) swaps the system prompt for that turn. `None`
    /// means the base prompt is used unchanged (pre-Phase-79
    /// behavior exactly).
    pub system_prompt_refiner: Option<Arc<dyn SystemPromptRefiner>>,
    /// Phase 120 — threshold for the planner's tool-name fuzzy-
    /// match recovery. Float in `[0.0, 1.0]`. Defaults to
    /// [`FUZZY_TOOL_NAME_THRESHOLD`] (0.80, matches Phase 112's
    /// fuzzy-match default).
    ///
    /// Operators set this via `[providers]
    /// tool_name_auto_correct_threshold = ...` in `aivyx.toml`;
    /// the binary plumbs it through to this field at planner-
    /// construction time.
    ///
    /// `0.0` → every Unknown name matches (the planner picks the
    /// first registered tool — effectively garbage out).
    /// `1.0` → only exact-token-set matches (preserves the
    /// pre-Phase-120 unknown-tool error path).
    pub tool_name_auto_correct_threshold: f32,
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
            .field(
                "system_prompt_refiner",
                &self.system_prompt_refiner.as_ref().map(|_| ".."),
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
            system_prompt_refiner: None,
            // Phase 120 — same default as the FUZZY_TOOL_NAME_THRESHOLD
            // const used at Task 4. Operators override via TOML.
            tool_name_auto_correct_threshold: FUZZY_TOOL_NAME_THRESHOLD,
        }
    }

    /// Phase 120 — override the tool-name fuzzy-match threshold.
    /// Caller is responsible for clamping into `[0.0, 1.0]` —
    /// the config layer (`aivyx-config`) rejects out-of-range
    /// values at TOML-parse time so the planner never sees a
    /// malformed value in practice.
    pub fn with_tool_name_auto_correct_threshold(
        mut self,
        threshold: f32,
    ) -> Self {
        self.tool_name_auto_correct_threshold = threshold;
        self
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

    /// Phase 79 — attach a per-turn [`SystemPromptRefiner`].
    /// Mirrors [`Self::with_context_provider`]; `None` (the
    /// default) preserves pre-Phase-79 behavior.
    pub fn with_system_prompt_refiner(
        mut self,
        refiner: Arc<dyn SystemPromptRefiner>,
    ) -> Self {
        self.system_prompt_refiner = Some(refiner);
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
            .snapshot()
            .into_iter()
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

    /// Phase 126 — per-call dispatch helper extracted from the
    /// Phase 120 / 101 inline loop. Resolves the tool name
    /// (with fuzzy-match recovery if unknown), validates the
    /// input schema (if repair budget remains), and either
    /// returns a `ToolCallRequest` to batch or surfaces an
    /// error result into history.
    ///
    /// `extracted_from_text` threads into the returned request's
    /// audit-trail field: `None` for protocol-channel calls;
    /// `Some(wrapper_tag)` for Phase 126 text-extracted calls.
    /// The bool in the return tuple is `true` when this call
    /// emitted an `invalid_input` repair result (the caller
    /// uses it to advance the repair-rounds counter).
    fn process_one_call(
        &mut self,
        call: ToolCallEnd,
        extracted_from_text: Option<String>,
        validate_enabled: bool,
    ) -> (Option<ToolCallRequest>, bool) {
        let resolution = self.registry.find_by_name(&call.tool_name);
        let (tool_id, auto_corrected_from) = match resolution {
            Some(id) => (id, None),
            None => match fuzzy_recover_tool_name(
                &self.registry,
                &call.tool_name,
                self.config.tool_name_auto_correct_threshold,
            ) {
                Some(matched_id) => (matched_id, Some(call.tool_name.clone())),
                None => {
                    let suggestions =
                        top_n_similar_tools(&self.registry, &call.tool_name, 3);
                    let message =
                        build_unknown_tool_message(&call.tool_name, &suggestions);
                    let mut body = json!({
                        "error": "unknown_tool",
                        "message": message,
                    });
                    if !suggestions.is_empty() {
                        body["did_you_mean"] = json!(
                            suggestions
                                .iter()
                                .map(|(n, _)| n.clone())
                                .collect::<Vec<_>>()
                        );
                    }
                    self.history.push(LlmMessage::ToolResult {
                        call_id: call.call_id,
                        content: body.to_string(),
                        is_error: true,
                    });
                    return (None, false);
                }
            },
        };

        if validate_enabled {
            if let Some(tool) = self.registry.get(tool_id) {
                if let Err(summary) =
                    validate_tool_input(tool.input_schema(), &call.input)
                {
                    let schema = tool.input_schema().clone();
                    self.history.push(LlmMessage::ToolResult {
                        call_id: call.call_id,
                        content: json!({
                            "error": "invalid_input",
                            "message": summary,
                            "expected_schema": schema,
                        })
                        .to_string(),
                        is_error: true,
                    });
                    return (None, true);
                }
            }
        }
        self.pending_call_ids.push_back(call.call_id);
        (
            Some(ToolCallRequest {
                tool_id,
                input: call.input,
                auto_corrected_from,
                extracted_from_text,
            }),
            false,
        )
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
            MessageContent::Document { media_type, data } => {
                vec![ContentBlock::document_from_bytes(media_type, data)]
            }
            MessageContent::Mixed(parts) => parts
                .iter()
                .map(|part| match part {
                    ContentPart::Text(text) => ContentBlock::text(text),
                    ContentPart::Image { media_type, data } => {
                        ContentBlock::image_from_bytes(media_type, data)
                    }
                    ContentPart::Document { media_type, data } => {
                        ContentBlock::document_from_bytes(media_type, data)
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
        // Compute the user's text once — both the Phase 76
        // recall hook and the Phase 79 system-prompt refiner key
        // off it, and they are independently configured.
        let query_text = match &message.content {
            MessageContent::Text(text) => text.clone(),
            MessageContent::Image { .. } => String::new(),
            MessageContent::Document { .. } => String::new(),
            MessageContent::Mixed(parts) => {
                let joined: Vec<&str> = parts
                    .iter()
                    .filter_map(|p| match p {
                        ContentPart::Text(t) => Some(t.as_str()),
                        ContentPart::Image { .. } => None,
                        ContentPart::Document { .. } => None,
                    })
                    .collect();
                joined.join(" ")
            }
        };
        let has_query = !query_text.trim().is_empty();

        if let Some(provider) = &self.config.context_provider {
            if has_query {
                if let Some(block) = provider
                    .recall(&query_text, message.session_id)
                    .await
                {
                    content.insert(0, ContentBlock::text(block));
                }
            }
        }

        // Phase 79 — adaptive Persona. The refiner may replace
        // this turn's system prompt with one carrying only the
        // contextually-relevant Persona facets. The planner is
        // built fresh per turn (the factory constructs a new
        // instance each turn), so mutating `config.system_prompt`
        // here is naturally turn-scoped. Clone the `Arc` out
        // first to release the `&self.config` borrow before the
        // `&mut self.config` assignment. `None` (no refiner,
        // blank message, fallback) leaves the base prompt
        // byte-identical to pre-Phase-79.
        let refiner = self.config.system_prompt_refiner.clone();
        if let Some(refiner) = refiner {
            if has_query {
                // Phase 117 — pass the planner's current base
                // prompt so extending refiners (relevance
                // section) can compose without rebuilding the
                // base from scratch. Refiners that ignore the
                // arg (Phase 79 adaptive Persona) behave
                // identically to pre-Phase-117.
                let base_prompt = self
                    .config
                    .system_prompt
                    .clone()
                    .unwrap_or_default();
                if let Some(refined) = refiner
                    .refine(&query_text, message.session_id, &base_prompt)
                    .await
                {
                    self.config.system_prompt = Some(refined);
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
        // tool name we don't recognize, or (Phase 101) emits a known
        // tool with input that fails its schema. `repair_rounds`
        // bounds the latter: after two `invalid_input` repair results
        // the call dispatches as-is (PHASE_101.md Q3).
        let mut repair_rounds = 0usize;
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

                    // Phase 126 — before treating this as a final
                    // message, try to extract tool calls from the
                    // text. Some LLM providers (qwen3 via Ollama
                    // observed in Phase 124) emit `<tool_code>` /
                    // `<tool_call>` JSON in response text rather
                    // than the protocol `tool_calls` array.
                    // Extraction yields synthesized ToolCallEnds
                    // that flow through the same Phase 120/101
                    // dispatch helper as protocol-channel calls;
                    // the wrapper-tag is threaded into the per-call
                    // `extracted_from_text` audit field for
                    // forensic visibility.
                    //
                    // Phase 127 Task 7 — the extractor receives a
                    // family-hint from the provider (Ollama queries
                    // `/api/show` once per model; other providers
                    // return None via the trait default). The hint
                    // biases inner-shape priority — qwen-family
                    // models prefer Qwen3-Coder XML over JSON
                    // inside `<tool_call>`. Failure to determine
                    // the family is silent — the substrate falls
                    // back to the default permissive scan.
                    let family_hint = self
                        .provider
                        .tool_call_family_hint(&self.config.model)
                        .await;
                    let extracted = crate::textual_tool_call::extract_tool_calls_with_hint(
                        &text,
                        family_hint.as_deref(),
                    );
                    if !extracted.is_empty() {
                        // Synthesize ToolCallEnds with UUID call IDs
                        // (the protocol channel didn't issue any).
                        // Each call carries the wrapper-tag through
                        // to its eventual audit entry.
                        let synthesized: Vec<(ToolCallEnd, String)> = extracted
                            .into_iter()
                            .map(|ext| {
                                let wrapper = ext.wrapper_tag.clone();
                                let call = ToolCallEnd {
                                    call_id: format!(
                                        "extracted-{}",
                                        uuid::Uuid::new_v4()
                                    ),
                                    tool_name: ext.tool_name,
                                    input: ext.arguments,
                                    name_resolution:
                                        aivyx_llm::NameResolution::Known,
                                };
                                (call, wrapper)
                            })
                            .collect();

                        // Push the assistant message AS THE MODEL
                        // SENT IT — text contains the `<tool_code>`
                        // blocks; the synthesized records mirror the
                        // protocol-channel shape so re-feeding history
                        // on the next round (after tool execution)
                        // works the same as a normal protocol-channel
                        // tool-call turn.
                        let records: Vec<LlmToolCallRecord> = synthesized
                            .iter()
                            .map(|(c, _)| LlmToolCallRecord {
                                call_id: c.call_id.clone(),
                                tool_name: c.tool_name.clone(),
                                input: c.input.clone(),
                            })
                            .collect();
                        self.history.push(LlmMessage::Assistant {
                            text: text.clone(),
                            tool_calls: records,
                        });

                        // Process each extracted call through the
                        // same dispatch helper as protocol calls.
                        // wrapper_tag flows into the per-request
                        // `extracted_from_text` field.
                        let validate_enabled = repair_rounds < 2;
                        let mut batch: Vec<ToolCallRequest> = Vec::new();
                        let mut had_invalid_input = false;
                        for (call, wrapper_tag) in synthesized {
                            let (req_opt, invalid) = self.process_one_call(
                                call,
                                Some(wrapper_tag),
                                validate_enabled,
                            );
                            if invalid {
                                had_invalid_input = true;
                            }
                            if let Some(req) = req_opt {
                                batch.push(req);
                            }
                        }
                        if had_invalid_input {
                            repair_rounds += 1;
                        }
                        if batch.is_empty() {
                            // Every extracted call failed (unknown or
                            // invalid). Loop to retry LLM with error
                            // results in history.
                            continue;
                        }
                        if batch.len() == 1 {
                            let req = batch.into_iter().next().unwrap();
                            return NextStep::ToolCall {
                                tool_id: req.tool_id,
                                input: req.input,
                                auto_corrected_from: req.auto_corrected_from,
                                extracted_from_text: req.extracted_from_text,
                            };
                        }
                        return NextStep::ToolCalls(batch);
                    }

                    // No extractable calls — original FinalMessage path.
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
                    // Phase 101 — tracks whether this round emitted an
                    // `invalid_input` repair result, so the repair cap
                    // advances only on a genuine validation failure.
                    let mut had_invalid_input = false;
                    // Once two repair rounds are spent, validation is
                    // skipped: a known call dispatches as-is and the
                    // tool's own `execute` validation is the floor
                    // (PHASE_101.md Q3).
                    let validate_enabled = repair_rounds < 2;

                    for call in calls {
                        // Phase 120 fuzzy-recovery + Phase 101
                        // validation, refactored into a helper
                        // at Phase 126 Task 4 so the new
                        // FinalMessage extraction branch can share
                        // the same dispatch path. Protocol-channel
                        // calls always carry `extracted_from_text:
                        // None`; the helper does not synthesize a
                        // wrapper-tag for these.
                        let (req_opt, invalid) =
                            self.process_one_call(call, None, validate_enabled);
                        if invalid {
                            had_invalid_input = true;
                        }
                        if let Some(req) = req_opt {
                            batch.push(req);
                        }
                    }

                    // Phase 101 — a round that emitted an `invalid_input`
                    // result spends one of the two repair attempts.
                    if had_invalid_input {
                        repair_rounds += 1;
                    }

                    if batch.is_empty() {
                        // Every call was unknown or failed validation —
                        // loop to retry the LLM with the error results
                        // in history.
                        continue;
                    }

                    if batch.len() == 1 {
                        // Single known tool — use the singular path.
                        // Phase 120 — preserve the auto-correction flag
                        // from the per-call ToolCallRequest.
                        // Phase 126 — preserve the extraction flag too;
                        // both compose forensically in the audit chain.
                        let req = batch.into_iter().next().unwrap();
                        return NextStep::ToolCall {
                            tool_id: req.tool_id,
                            input: req.input,
                            auto_corrected_from: req.auto_corrected_from,
                            extracted_from_text: req.extracted_from_text,
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

    fn model(&self) -> &str {
        &self.config.model
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
/// The `error` field is one of: `denied`, `not_in_role`, `rate_limited`,
/// `failed`, `timed_out`, `requires_escalation`. It is stable across versions;
/// new kinds land as new strings, never as renames.
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
        ToolOutcome::RateLimited { tool_name, reason } => {
            let envelope = json!({
                "error": "rate_limited",
                "message": format!("tool {tool_name} throttled: {reason}"),
            });
            (envelope.to_string(), true)
        }
        ToolOutcome::RequiresEscalation { reason, .. } => {
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
// Tool-call input validation — Phase 101
// ---------------------------------------------------------------------------

/// Validate a tool call's `input` against the tool's declared
/// `input_schema()` (JSON Schema). Returns `Ok(())` when the input
/// satisfies the schema, or `Err(summary)` — a human-readable
/// digest of the first few violations — which the planner turns
/// into an `invalid_input` repair result.
///
/// **Fails open.** If the schema itself does not compile as valid
/// JSON Schema, the input is treated as valid. Tool schemas are
/// authored in-tree and a malformed one should never reach here;
/// failing open guarantees a quirky future schema can never brick
/// its own tool's dispatch — the worst case degrades to
/// pre-Phase-101 behavior (the tool's own `execute` validation is
/// still the floor).
/// Phase 120 Task 4 — fuzzy-match threshold for the tool-name
/// recovery path. Default `0.80` matches Phase 112's fuzzy-match
/// default (the substrate's load-bearing threshold for tokenized
/// Jaccard similarity decisions). Task 5 will make this operator-
/// configurable via `[providers] tool_name_auto_correct_threshold`.
const FUZZY_TOOL_NAME_THRESHOLD: f32 = 0.80;

/// Phase 120 Task 4 — fuzzy-match recovery for hallucinated tool
/// names. Walks `registry`'s tools, computes
/// `aivyx_core::skill_proposer::title_similarity(emitted_name,
/// tool_name)`, and returns the `ToolId` of the best match if and
/// only if its score meets or exceeds `threshold`.
///
/// Returns `None` when:
/// - The registry is empty (no tools registered for this turn).
/// - No tool's name scores at or above `threshold`.
///
/// Ties broken by registration order (the first tool to reach the
/// max score wins). In practice ties are rare since the threshold
/// gates on a meaningful similarity ceiling.
///
/// Pure function modulo the registry iteration; planner-internal.
fn fuzzy_recover_tool_name(
    registry: &crate::ToolRegistry,
    emitted_name: &str,
    threshold: f32,
) -> Option<crate::ToolId> {
    let mut best: Option<(crate::ToolId, f32)> = None;
    let snapshot = registry.snapshot();
    for tool in &snapshot {
        let score = crate::skill_proposer::title_similarity(
            emitted_name,
            tool.name(),
        );
        if score >= threshold {
            // Find the id for this tool by name (cheap — the
            // registry's `find_by_name` is the canonical lookup).
            let Some(id) = registry.find_by_name(tool.name()) else {
                continue;
            };
            match best {
                None => best = Some((id, score)),
                Some((_, b)) if score > b => best = Some((id, score)),
                _ => {} // existing best wins on tie
            }
        }
    }
    best.map(|(id, _)| id)
}

/// Phase 120 Task 6 — top-N tools ranked by Jaccard title similarity
/// against the model's emitted name. Used by the synthetic
/// `unknown_tool` error path so the model sees `available_suggestions`
/// and can retry with the right name.
///
/// Returns `Vec<(tool_name, score)>` sorted descending by score; ties
/// broken by registration order (stable). At most `n` entries; empty
/// when the registry is empty (the synthetic message falls back to
/// a neutral "no tools available" form — see [`build_unknown_tool_message`]).
///
/// Pure function modulo the registry iteration.
fn top_n_similar_tools(
    registry: &crate::ToolRegistry,
    emitted_name: &str,
    n: usize,
) -> Vec<(String, f32)> {
    let mut scored: Vec<(String, f32)> = registry
        .snapshot()
        .into_iter()
        .map(|tool| {
            let score = crate::skill_proposer::title_similarity(
                emitted_name,
                tool.name(),
            );
            (tool.name().to_string(), score)
        })
        .collect();
    // Stable sort by score descending; ties keep iteration order
    // (which matches registration order on `ToolRegistry::iter_tools`).
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(n);
    scored
}

/// Phase 120 Task 6 — operator-style synthetic message for the
/// model when the planner's fuzzy-match recovery couldn't resolve
/// the emitted name above threshold.
///
/// Wording is deliberately unambiguous about WHICH tools to retry
/// with so the model picks the right name on the next turn:
/// - With suggestions: `"tool 'X' is not registered. Did you mean
///   'Y', 'Z', 'W'?"`
/// - Empty registry: `"tool 'X' is not registered. (no tools
///   available in this role)"` — neutral fallback rather than a
///   misleading "did you mean?" with no suggestions.
fn build_unknown_tool_message(
    emitted_name: &str,
    suggestions: &[(String, f32)],
) -> String {
    if suggestions.is_empty() {
        return format!(
            "tool '{}' is not registered. (no tools available in this role)",
            emitted_name
        );
    }
    let names: Vec<String> = suggestions
        .iter()
        .map(|(n, _)| format!("'{n}'"))
        .collect();
    format!(
        "tool '{}' is not registered. Did you mean {}?",
        emitted_name,
        names.join(", "),
    )
}

fn validate_tool_input(
    schema: &serde_json::Value,
    input: &serde_json::Value,
) -> Result<(), String> {
    let validator = match jsonschema::validator_for(schema) {
        Ok(v) => v,
        Err(_) => return Ok(()), // fail open — see doc comment
    };
    // Cap the digest at the first five violations: enough for the
    // model to repair the call, short enough to keep the result
    // message compact.
    let violations: Vec<String> = validator
        .iter_errors(input)
        .take(5)
        .map(|e| e.to_string())
        .collect();
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations.join("; "))
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
                        ContentBlock::DocumentBase64 { media_type, .. } => media_type.as_str(),
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

        /// Phase 101 — a `FakeTool` carrying a real JSON Schema, for
        /// the planner validate-before-dispatch tests.
        fn with_schema(name: &'static str, schema: Value) -> Self {
            FakeTool {
                id: ToolId::new(),
                name,
                schema,
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

    // ---- Phase 79 — SystemPromptRefiner hook -------------------

    struct FakeRefiner {
        refined: Option<String>,
        seen: std::sync::Mutex<Vec<String>>,
    }

    impl FakeRefiner {
        fn new(refined: Option<&str>) -> Arc<Self> {
            Arc::new(Self {
                refined: refined.map(str::to_string),
                seen: std::sync::Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl SystemPromptRefiner for FakeRefiner {
        async fn refine(
            &self,
            user_message: &str,
            _session_id: crate::SessionId,
            _base_prompt: &str,
        ) -> Option<String> {
            self.seen.lock().unwrap().push(user_message.to_string());
            self.refined.clone()
        }
    }

    #[tokio::test]
    async fn refiner_some_swaps_system_prompt_for_the_turn() {
        let refiner = FakeRefiner::new(Some("REFINED PERSONA PROMPT"));
        let mut planner = bare_planner(
            LlmPlannerConfig::new("m")
                .with_system_prompt("BASE PROMPT")
                .with_system_prompt_refiner(refiner.clone()),
        );
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "help me ship"))
            .await;
        assert_eq!(
            refiner.seen.lock().unwrap().clone(),
            vec!["help me ship".to_string()]
        );
        assert_eq!(
            planner.config.system_prompt.as_deref(),
            Some("REFINED PERSONA PROMPT")
        );
    }

    #[tokio::test]
    async fn refiner_none_keeps_base_prompt() {
        let refiner = FakeRefiner::new(None);
        let mut planner = bare_planner(
            LlmPlannerConfig::new("m")
                .with_system_prompt("BASE PROMPT")
                .with_system_prompt_refiner(refiner.clone()),
        );
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "hi"))
            .await;
        // Consulted, returned None → base byte-identical.
        assert_eq!(refiner.seen.lock().unwrap().len(), 1);
        assert_eq!(
            planner.config.system_prompt.as_deref(),
            Some("BASE PROMPT")
        );
    }

    #[tokio::test]
    async fn no_refiner_is_unchanged() {
        // Regression guard: the default path must not change.
        let mut planner = bare_planner(
            LlmPlannerConfig::new("m").with_system_prompt("BASE"),
        );
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "hello"))
            .await;
        assert_eq!(
            planner.config.system_prompt.as_deref(),
            Some("BASE")
        );
    }

    #[tokio::test]
    async fn refiner_skipped_for_blank_query() {
        // Whitespace-only message must not consult the refiner
        // and must leave the base prompt untouched.
        let refiner = FakeRefiner::new(Some("SHOULD-NOT-APPEAR"));
        let mut planner = bare_planner(
            LlmPlannerConfig::new("m")
                .with_system_prompt("BASE")
                .with_system_prompt_refiner(refiner.clone()),
        );
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "   "))
            .await;
        assert!(refiner.seen.lock().unwrap().is_empty());
        assert_eq!(
            planner.config.system_prompt.as_deref(),
            Some("BASE")
        );
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
                    name_resolution: aivyx_llm::NameResolution::Known,
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
                auto_corrected_from: None,
                extracted_from_text: None,
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
                    name_resolution: aivyx_llm::NameResolution::Known,
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
                    name_resolution: aivyx_llm::NameResolution::Known,
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
                        name_resolution: aivyx_llm::NameResolution::Known,
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

    // ----- Phase 120 — Tool-name fuzzy recovery + auto-correction audit -----

    #[tokio::test]
    async fn phase_120_fuzzy_recovery_dispatches_close_match() {
        // qwen3.6:27b emits `fs_read`; the registered tool is `fs.read`.
        // title_similarity("fs_read", "fs.read") = 1.0 (same tokens
        // after separator normalization) → above the 0.80 threshold
        // → the planner dispatches `fs.read` and records the verbatim
        // `fs_read` as auto_corrected_from.
        let fs_read = Arc::new(FakeTool::new("fs.read"));
        let fs_read_id = fs_read.id();
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::ToolCalls {
                calls: vec![ToolCallEnd {
                    call_id: "c1".into(),
                    tool_name: "fs_read".into(),
                    input: json!({}),
                    // Provider flagged Unknown; planner takes over.
                    name_resolution: aivyx_llm::NameResolution::Unknown {
                        original: "fs_read".into(),
                    },
                }],
                text_so_far: String::new(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![fs_read]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("local-qwen"),
        );
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "read a file"))
            .await;
        let step = planner.next_step(&[], &channel).await;
        match step {
            NextStep::ToolCall {
                tool_id,
                auto_corrected_from,
                extracted_from_text: _,
                ..
            } => {
                assert_eq!(tool_id, fs_read_id);
                assert_eq!(auto_corrected_from.as_deref(), Some("fs_read"));
            }
            other => panic!(
                "expected ToolCall with auto-correction, got {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn phase_120_below_threshold_synthesizes_unknown_tool_error() {
        // `do_the_thing` vs registered `memory.read` has too few
        // shared tokens to clear 0.80 (1/4 Jaccard). Fall-through
        // path: synthetic unknown_tool error message; loop continues
        // so the model can retry.
        let memory_read = Arc::new(FakeTool::new("memory.read"));
        let script = vec![
            FakeStep {
                events: vec![],
                terminal: LlmStepEnd::ToolCalls {
                    calls: vec![ToolCallEnd {
                        call_id: "c1".into(),
                        tool_name: "do_the_thing".into(),
                        input: json!({}),
                        name_resolution: aivyx_llm::NameResolution::Unknown {
                            original: "do_the_thing".into(),
                        },
                    }],
                    text_so_far: String::new(),
                    usage: zero_usage(),
                },
            },
            FakeStep {
                events: vec![],
                terminal: LlmStepEnd::FinalMessage {
                    text: "giving up".into(),
                    usage: zero_usage(),
                },
            },
        ];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![memory_read]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("local-qwen"),
        );
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "do it"))
            .await;
        let step = planner.next_step(&[], &channel).await;
        // Loop continued past the unknown call and reached
        // FinalMessage on the next chat_stream.
        assert!(matches!(step, NextStep::FinalMessage(ref m) if m == "giving up"));
        // History carries the synthetic error.
        let has_unknown = planner.history().iter().any(|m| match m {
            LlmMessage::ToolResult {
                content, is_error, ..
            } => *is_error && content.contains("unknown_tool"),
            _ => false,
        });
        assert!(has_unknown, "below-threshold path must synthesize unknown_tool");
    }

    #[tokio::test]
    async fn phase_120_known_name_dispatches_with_no_auto_correction() {
        // The dominant case: model emits a registered name verbatim;
        // no recovery needed; auto_corrected_from is None.
        let fs_read = Arc::new(FakeTool::new("fs.read"));
        let fs_read_id = fs_read.id();
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::ToolCalls {
                calls: vec![ToolCallEnd {
                    call_id: "c1".into(),
                    tool_name: "fs.read".into(),
                    input: json!({}),
                    name_resolution: aivyx_llm::NameResolution::Known,
                }],
                text_so_far: String::new(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![fs_read]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("m"),
        );
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "read"))
            .await;
        let step = planner.next_step(&[], &channel).await;
        match step {
            NextStep::ToolCall {
                tool_id,
                auto_corrected_from,
                extracted_from_text: _,
                ..
            } => {
                assert_eq!(tool_id, fs_read_id);
                assert!(
                    auto_corrected_from.is_none(),
                    "verbatim Known dispatch must NOT report an auto-correction"
                );
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn phase_120_fuzzy_recover_picks_best_match() {
        // Threshold-pinning unit test for the pure helper. With both
        // fs.read and web.fetch registered, an emitted `fs_read`
        // resolves to fs.read (not web.fetch).
        let fs_read = Arc::new(FakeTool::new("fs.read")) as Arc<dyn Tool>;
        let web_fetch = Arc::new(FakeTool::new("web.fetch")) as Arc<dyn Tool>;
        let fs_id = fs_read.id();
        let registry = ToolRegistry::new(vec![fs_read, web_fetch]);
        let resolved = fuzzy_recover_tool_name(&registry, "fs_read", 0.80);
        assert_eq!(resolved, Some(fs_id));
    }

    #[test]
    fn phase_120_fuzzy_recover_returns_none_when_no_match_clears_threshold() {
        let memory_read =
            Arc::new(FakeTool::new("memory.read")) as Arc<dyn Tool>;
        let registry = ToolRegistry::new(vec![memory_read]);
        // do_the_thing vs memory.read → Jaccard 0/5 = 0 < 0.80.
        let resolved = fuzzy_recover_tool_name(&registry, "do_the_thing", 0.80);
        assert!(resolved.is_none());
    }

    #[test]
    fn phase_120_fuzzy_recover_returns_none_for_empty_registry() {
        let registry = ToolRegistry::new(vec![]);
        let resolved = fuzzy_recover_tool_name(&registry, "fs_read", 0.80);
        assert!(resolved.is_none());
    }

    #[test]
    fn phase_120_planner_config_default_threshold_matches_const() {
        // The LlmPlannerConfig::new default plumbs through the
        // FUZZY_TOOL_NAME_THRESHOLD const; the config layer's
        // DEFAULT_TOOL_NAME_AUTO_CORRECT_THRESHOLD is the same value.
        let config = LlmPlannerConfig::new("m");
        assert!(
            (config.tool_name_auto_correct_threshold
                - FUZZY_TOOL_NAME_THRESHOLD)
                .abs()
                < 1e-6
        );
    }

    #[test]
    fn phase_120_with_tool_name_auto_correct_threshold_overrides() {
        let config = LlmPlannerConfig::new("m")
            .with_tool_name_auto_correct_threshold(0.55);
        assert!(
            (config.tool_name_auto_correct_threshold - 0.55).abs() < 1e-6
        );
    }

    // ----- Phase 120 Task 6 — "Did you mean?" suggestions -----

    #[test]
    fn phase_120_top_n_orders_by_similarity_descending() {
        // `fs_read` against {fs.read, fs.write, web.fetch}:
        //   tokens(fs_read) = {fs, read}
        //   - fs.read  → {fs, read} → 1.0 (perfect)
        //   - fs.write → {fs, write} → 1/3
        //   - web.fetch → {web, fetch} → 0/4
        // Top-3 must rank fs.read, fs.write, web.fetch in that order.
        let fs_read = Arc::new(FakeTool::new("fs.read")) as Arc<dyn Tool>;
        let fs_write = Arc::new(FakeTool::new("fs.write")) as Arc<dyn Tool>;
        let web_fetch = Arc::new(FakeTool::new("web.fetch")) as Arc<dyn Tool>;
        let registry = ToolRegistry::new(vec![fs_read, fs_write, web_fetch]);
        let top = top_n_similar_tools(&registry, "fs_read", 3);
        assert_eq!(top.len(), 3);
        assert_eq!(top[0].0, "fs.read");
        assert_eq!(top[1].0, "fs.write");
        assert_eq!(top[2].0, "web.fetch");
        // Scores descending.
        assert!(top[0].1 > top[1].1);
        assert!(top[1].1 > top[2].1);
    }

    #[test]
    fn phase_120_top_n_caps_at_n() {
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(FakeTool::new("fs.read")),
            Arc::new(FakeTool::new("fs.write")),
            Arc::new(FakeTool::new("web.fetch")),
            Arc::new(FakeTool::new("memory.read")),
            Arc::new(FakeTool::new("memory.write")),
        ];
        let registry = ToolRegistry::new(tools);
        let top = top_n_similar_tools(&registry, "fs_read", 3);
        assert_eq!(top.len(), 3, "top-3 cap must hold under 5-tool registry");
    }

    #[test]
    fn phase_120_top_n_returns_empty_for_empty_registry() {
        let registry = ToolRegistry::new(vec![]);
        let top = top_n_similar_tools(&registry, "fs_read", 3);
        assert!(top.is_empty());
    }

    #[test]
    fn phase_120_build_unknown_message_includes_suggestions() {
        let suggestions = vec![
            ("fs.read".to_string(), 1.0),
            ("fs.write".to_string(), 0.5),
            ("memory.read".to_string(), 0.33),
        ];
        let msg = build_unknown_tool_message("fs_read", &suggestions);
        // The emitted name appears.
        assert!(msg.contains("'fs_read'"));
        // "Did you mean?" prefix appears.
        assert!(msg.contains("Did you mean"));
        // All three suggestions appear in order.
        let pos_a = msg.find("'fs.read'").unwrap();
        let pos_b = msg.find("'fs.write'").unwrap();
        let pos_c = msg.find("'memory.read'").unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[test]
    fn phase_120_build_unknown_message_falls_back_for_empty_registry() {
        // Empty suggestions → neutral fallback. The model should NOT
        // see a misleading "Did you mean ?" form.
        let msg = build_unknown_tool_message("fs_read", &[]);
        assert!(msg.contains("'fs_read'"));
        assert!(msg.contains("no tools available"));
        // Critical: no misleading "Did you mean?" with empty list.
        assert!(!msg.contains("Did you mean"));
    }

    #[tokio::test]
    async fn phase_120_below_threshold_includes_did_you_mean_in_history() {
        // End-to-end: model emits below-threshold name; planner
        // synthesizes unknown_tool ToolResult; the JSON body
        // includes a `did_you_mean` field with the ranked
        // suggestions. The model can parse that field on its next
        // turn and retry with the right name.
        let fs_read = Arc::new(FakeTool::new("fs.read"));
        let web_fetch = Arc::new(FakeTool::new("web.fetch"));
        let script = vec![
            FakeStep {
                events: vec![],
                terminal: LlmStepEnd::ToolCalls {
                    calls: vec![ToolCallEnd {
                        call_id: "c1".into(),
                        // do_the_thing is below threshold against
                        // either fs.read or web.fetch (1/4 max).
                        tool_name: "do_the_thing".into(),
                        input: json!({}),
                        name_resolution: aivyx_llm::NameResolution::Unknown {
                            original: "do_the_thing".into(),
                        },
                    }],
                    text_so_far: String::new(),
                    usage: zero_usage(),
                },
            },
            FakeStep {
                events: vec![],
                terminal: LlmStepEnd::FinalMessage {
                    text: "ok".into(),
                    usage: zero_usage(),
                },
            },
        ];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![fs_read, web_fetch]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("m"),
        );
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "do it"))
            .await;
        let _ = planner.next_step(&[], &channel).await;
        // History carries an unknown_tool ToolResult whose JSON
        // body includes a did_you_mean field.
        let body = planner
            .history()
            .iter()
            .find_map(|m| match m {
                LlmMessage::ToolResult { content, is_error, .. }
                    if *is_error && content.contains("unknown_tool") =>
                {
                    Some(content.clone())
                }
                _ => None,
            })
            .expect("expected synthetic unknown_tool entry");
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["error"], "unknown_tool");
        // did_you_mean array carries the top-3 (or fewer) names.
        let suggestions = parsed["did_you_mean"]
            .as_array()
            .expect("did_you_mean must be an array");
        assert!(!suggestions.is_empty());
        // The "Did you mean" phrasing is part of the human-readable
        // message too.
        assert!(
            parsed["message"].as_str().unwrap().contains("Did you mean"),
            "human message must include 'Did you mean': {body}"
        );
    }

    #[tokio::test]
    async fn phase_120_zero_threshold_disables_fuzzy_recovery() {
        // Operator sets the threshold to 0.0 — wait, 0.0 means
        // "every match clears", which would auto-correct EVERYTHING
        // (including unrelated names). The semantically conservative
        // disable is threshold = 1.0 (exact-match only). Test that
        // posture: with threshold = 1.0, an emitted `fs_read` (Jaccard
        // 1.0 vs `fs.read`) STILL clears (1.0 >= 1.0); but `fs_rea`
        // (Jaccard 0.5) does NOT. Pin the inclusive-bound semantics.
        let fs_read = Arc::new(FakeTool::new("fs.read"));
        let script = vec![
            FakeStep {
                events: vec![],
                terminal: LlmStepEnd::ToolCalls {
                    calls: vec![ToolCallEnd {
                        call_id: "c1".into(),
                        tool_name: "fs_rea".into(), // partial — Jaccard 0.5
                        input: json!({}),
                        name_resolution: aivyx_llm::NameResolution::Unknown {
                            original: "fs_rea".into(),
                        },
                    }],
                    text_so_far: String::new(),
                    usage: zero_usage(),
                },
            },
            FakeStep {
                events: vec![],
                terminal: LlmStepEnd::FinalMessage {
                    text: "giving up".into(),
                    usage: zero_usage(),
                },
            },
        ];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![fs_read]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("m")
                .with_tool_name_auto_correct_threshold(1.0),
        );
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "read"))
            .await;
        let step = planner.next_step(&[], &channel).await;
        // Partial match doesn't clear threshold 1.0 → unknown_tool.
        assert!(
            matches!(step, NextStep::FinalMessage(ref m) if m == "giving up")
        );
        let has_unknown = planner.history().iter().any(|m| match m {
            LlmMessage::ToolResult {
                content, is_error, ..
            } => *is_error && content.contains("unknown_tool"),
            _ => false,
        });
        assert!(
            has_unknown,
            "threshold 1.0 must fail-through for partial matches"
        );
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

    #[tokio::test]
    async fn rate_limited_outcome_produces_rate_limited_envelope() {
        // TH.2 — a throttled call renders a distinct `rate_limited` error the
        // model can adapt to, carrying the breached-limit reason.
        let outcome = ToolOutcome::RateLimited {
            tool_name: "web.fetch".to_string(),
            reason: "per-turn cap for `web.fetch` reached: 6 of 6".to_string(),
        };
        let (content, is_error) = render_tool_result(&outcome);
        assert!(is_error);
        let parsed: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["error"], "rate_limited");
        assert!(parsed["message"].as_str().unwrap().contains("web.fetch"));
        assert!(parsed["message"].as_str().unwrap().contains("per-turn cap"));
        // Forensically distinct from capability / role denials.
        let summary = crate::ToolOutcomeSummary::from(&outcome);
        assert_eq!(summary, crate::ToolOutcomeSummary::RateLimited);
        assert_ne!(summary, crate::ToolOutcomeSummary::Denied);
        assert_ne!(summary, crate::ToolOutcomeSummary::NotInRole);
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
                        name_resolution: aivyx_llm::NameResolution::Known,
                    },
                    ToolCallEnd {
                        call_id: "toolu_b".to_string(),
                        tool_name: "memory.read".to_string(),
                        input: json!({"topic": "notes"}),
                        name_resolution: aivyx_llm::NameResolution::Known,
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
                        name_resolution: aivyx_llm::NameResolution::Known,
                    },
                    ToolCallEnd {
                        call_id: "toolu_bad".to_string(),
                        tool_name: "does.not.exist".to_string(),
                        input: json!({}),
                        name_resolution: aivyx_llm::NameResolution::Known,
                    },
                    ToolCallEnd {
                        call_id: "toolu_b".to_string(),
                        tool_name: "memory.read".to_string(),
                        input: json!({}),
                        name_resolution: aivyx_llm::NameResolution::Known,
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
                        name_resolution: aivyx_llm::NameResolution::Known,
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

    // -----------------------------------------------------------------------
    // Phase 101 — tool-call input validation & repair.
    // -----------------------------------------------------------------------

    fn req_path_schema() -> Value {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        })
    }

    #[test]
    fn validate_accepts_well_formed_input() {
        assert!(validate_tool_input(
            &req_path_schema(),
            &json!({"path": "notes.txt"}),
        )
        .is_ok());
    }

    #[test]
    fn validate_rejects_missing_required_field() {
        let err = validate_tool_input(&req_path_schema(), &json!({}))
            .expect_err("missing required `path` must fail validation");
        assert!(
            err.contains("path"),
            "the summary should name the missing field: {err}"
        );
    }

    #[test]
    fn validate_rejects_wrong_typed_field() {
        let err = validate_tool_input(&req_path_schema(), &json!({"path": 123}))
            .expect_err("a non-string `path` must fail validation");
        assert!(!err.is_empty(), "the summary must not be empty");
    }

    #[test]
    fn validate_tolerates_extra_unschemad_field() {
        // Tool schemas do not set `additionalProperties: false`, so an
        // extra field the model invented is tolerated, not rejected.
        assert!(validate_tool_input(
            &req_path_schema(),
            &json!({"path": "x", "hallucinated": true}),
        )
        .is_ok());
    }

    #[test]
    fn validate_fails_open_on_a_malformed_schema() {
        // A value that is not itself a valid JSON Schema must never
        // brick dispatch — `validate_tool_input` treats it as valid.
        assert!(validate_tool_input(&json!(42), &json!({"anything": true})).is_ok());
    }

    #[tokio::test]
    async fn well_formed_call_dispatches_without_a_repair_round() {
        let tool = Arc::new(FakeTool::with_schema("fs.read", req_path_schema()));
        let tool_id = tool.id();
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::ToolCalls {
                calls: vec![ToolCallEnd {
                    call_id: "c1".to_string(),
                    tool_name: "fs.read".to_string(),
                    input: json!({"path": "ok.txt"}),
                    name_resolution: aivyx_llm::NameResolution::Known,
                }],
                text_so_far: String::new(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![tool]));
        let mut planner = LlmPlanner::new(provider, registry, LlmPlannerConfig::new("m"));
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "go"))
            .await;
        match planner.next_step(&[], &channel).await {
            NextStep::ToolCall { tool_id: got, .. } => assert_eq!(got, tool_id),
            other => panic!("expected ToolCall, got {other:?}"),
        }
        assert!(
            !planner.history().iter().any(|m| matches!(
                m,
                LlmMessage::ToolResult { content, .. } if content.contains("invalid_input")
            )),
            "a well-formed call must not produce a repair result"
        );
    }

    #[tokio::test]
    async fn malformed_call_is_repaired_then_dispatched() {
        let tool = Arc::new(FakeTool::with_schema("fs.read", req_path_schema()));
        let tool_id = tool.id();
        let script = vec![
            // Round 1 — missing the required `path` field.
            FakeStep {
                events: vec![],
                terminal: LlmStepEnd::ToolCalls {
                    calls: vec![ToolCallEnd {
                        call_id: "c1".to_string(),
                        tool_name: "fs.read".to_string(),
                        input: json!({}),
                        name_resolution: aivyx_llm::NameResolution::Known,
                    }],
                    text_so_far: String::new(),
                    usage: zero_usage(),
                },
            },
            // Round 2 — the model repairs the call.
            FakeStep {
                events: vec![],
                terminal: LlmStepEnd::ToolCalls {
                    calls: vec![ToolCallEnd {
                        call_id: "c2".to_string(),
                        tool_name: "fs.read".to_string(),
                        input: json!({"path": "fixed.txt"}),
                        name_resolution: aivyx_llm::NameResolution::Known,
                    }],
                    text_so_far: String::new(),
                    usage: zero_usage(),
                },
            },
        ];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![tool]));
        let mut planner = LlmPlanner::new(provider, registry, LlmPlannerConfig::new("m"));
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "go"))
            .await;
        match planner.next_step(&[], &channel).await {
            NextStep::ToolCall { tool_id: got, input, .. } => {
                assert_eq!(got, tool_id);
                assert_eq!(input, json!({"path": "fixed.txt"}));
            }
            other => panic!("expected the repaired ToolCall, got {other:?}"),
        }
        let repairs = planner
            .history()
            .iter()
            .filter(|m| matches!(
                m,
                LlmMessage::ToolResult { content, .. } if content.contains("invalid_input")
            ))
            .count();
        assert_eq!(repairs, 1, "exactly one repair round expected");
    }

    #[tokio::test]
    async fn two_repair_rounds_then_dispatch_as_is() {
        let tool = Arc::new(FakeTool::with_schema("fs.read", req_path_schema()));
        // Three rounds, all missing `path`. The third dispatches the
        // still-invalid call as-is — the two-repair cap disabled
        // validation (PHASE_101.md Q3).
        let bad = || LlmStepEnd::ToolCalls {
            calls: vec![ToolCallEnd {
                call_id: "c".to_string(),
                tool_name: "fs.read".to_string(),
                input: json!({}),
                name_resolution: aivyx_llm::NameResolution::Known,
            }],
            text_so_far: String::new(),
            usage: zero_usage(),
        };
        let script = vec![
            FakeStep { events: vec![], terminal: bad() },
            FakeStep { events: vec![], terminal: bad() },
            FakeStep { events: vec![], terminal: bad() },
        ];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![tool]));
        let mut planner = LlmPlanner::new(provider, registry, LlmPlannerConfig::new("m"));
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "go"))
            .await;
        let step = planner.next_step(&[], &channel).await;
        assert!(
            matches!(step, NextStep::ToolCall { .. }),
            "after the two-repair cap the call dispatches as-is, got {step:?}"
        );
        let repairs = planner
            .history()
            .iter()
            .filter(|m| matches!(
                m,
                LlmMessage::ToolResult { content, .. } if content.contains("invalid_input")
            ))
            .count();
        assert_eq!(repairs, 2, "repair attempts are capped at two");
    }

    #[tokio::test]
    async fn mixed_batch_dispatches_valid_call_and_errors_invalid_one() {
        let good = Arc::new(FakeTool::with_schema("fs.read", req_path_schema()));
        let bad_tool = Arc::new(FakeTool::with_schema("fs.write", req_path_schema()));
        let good_id = good.id();
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::ToolCalls {
                calls: vec![
                    ToolCallEnd {
                        call_id: "ok".to_string(),
                        tool_name: "fs.read".to_string(),
                        input: json!({"path": "ok.txt"}),
                        name_resolution: aivyx_llm::NameResolution::Known,
                    },
                    ToolCallEnd {
                        call_id: "bad".to_string(),
                        tool_name: "fs.write".to_string(),
                        input: json!({}),
                        name_resolution: aivyx_llm::NameResolution::Known,
                    },
                ],
                text_so_far: String::new(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![good, bad_tool]));
        let mut planner = LlmPlanner::new(provider, registry, LlmPlannerConfig::new("m"));
        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "go"))
            .await;
        // Only the valid call dispatches — the invalid one is errored,
        // leaving a single-tool batch.
        match planner.next_step(&[], &channel).await {
            NextStep::ToolCall { tool_id: got, .. } => assert_eq!(got, good_id),
            other => panic!("expected the valid ToolCall, got {other:?}"),
        }
        let repairs = planner
            .history()
            .iter()
            .filter(|m| matches!(
                m,
                LlmMessage::ToolResult { content, .. } if content.contains("invalid_input")
            ))
            .count();
        assert_eq!(repairs, 1, "the one invalid call produced one repair result");
    }

    // -----------------------------------------------------------------------
    // Phase 126 — textual-tool-call extraction from FinalMessage text
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn phase_126_tool_code_extraction_dispatches_known_call() {
        // Model returns text with a `<tool_code>` block containing a
        // real registered tool. The planner extracts, synthesizes
        // a ToolCallEnd, dispatches, and the AuditTag::ToolCall
        // carries extracted_from_text: Some("tool_code").
        let fs_read = FakeTool::new("fs.read");
        let fs_read_id = fs_read.id;
        let script = vec![FakeStep {
            events: vec![LlmStreamEvent::TextChunk(
                "I'll read the file.\n\n\
                 <tool_code>\n  \
                 {\"name\": \"fs.read\", \"arguments\": {\"path\": \"x.txt\"}}\n\
                 </tool_code>"
                    .to_string(),
            )],
            terminal: LlmStepEnd::FinalMessage {
                text: "I'll read the file.\n\n\
                       <tool_code>\n  \
                       {\"name\": \"fs.read\", \"arguments\": {\"path\": \"x.txt\"}}\n\
                       </tool_code>"
                    .to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![Arc::new(fs_read)]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("test-model"),
        );

        let channel = RecChannel::new();
        planner.begin_turn(&Message::text(channel.session, "read x.txt")).await;
        let step = planner.next_step(&[], &channel).await;
        match step {
            NextStep::ToolCall {
                tool_id,
                input,
                auto_corrected_from,
                extracted_from_text,
            } => {
                assert_eq!(tool_id, fs_read_id, "extraction resolved to fs.read");
                assert_eq!(input["path"], "x.txt");
                assert!(
                    auto_corrected_from.is_none(),
                    "tool name was exact; no fuzzy-recovery should fire"
                );
                assert_eq!(
                    extracted_from_text.as_deref(),
                    Some("tool_code"),
                    "wrapper-tag identifier threaded through to the audit field"
                );
            }
            other => panic!(
                "expected NextStep::ToolCall from extraction; got {other:?}"
            ),
        }
    }

    #[tokio::test]
    async fn phase_126_tool_call_extraction_with_tool_parameters_shape() {
        // gemma4's observed shape: `<tool_call>` wrapper with
        // `tool`/`parameters` JSON. Extraction handles both shapes
        // identically and routes through the same dispatcher.
        let memory_read = FakeTool::new("memory.read");
        let memory_read_id = memory_read.id;
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::FinalMessage {
                text: "<tool_call>\
                       {\"tool\": \"memory.read\", \"parameters\": {\"topic\": \"x\"}}\
                       </tool_call>"
                    .to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![Arc::new(memory_read)]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("test-model"),
        );

        let channel = RecChannel::new();
        planner.begin_turn(&Message::text(channel.session, "read memory")).await;
        let step = planner.next_step(&[], &channel).await;
        match step {
            NextStep::ToolCall {
                tool_id,
                extracted_from_text,
                ..
            } => {
                assert_eq!(tool_id, memory_read_id);
                assert_eq!(
                    extracted_from_text.as_deref(),
                    Some("tool_call"),
                    "wrapper-tag for <tool_call> shape correctly identified"
                );
            }
            other => panic!("expected ToolCall; got {other:?}"),
        }
    }

    #[tokio::test]
    async fn phase_126_extraction_falls_through_to_final_message_when_no_blocks()
    {
        // Plain-text response (no `<tool_code>` blocks). Extraction
        // returns empty; existing FinalMessage path runs unchanged.
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::FinalMessage {
                text: "Just regular prose without any tool-call markers.".to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("test-model"),
        );

        let channel = RecChannel::new();
        planner.begin_turn(&Message::text(channel.session, "say hi")).await;
        let step = planner.next_step(&[], &channel).await;
        assert!(matches!(
            step,
            NextStep::FinalMessage(ref m) if m.starts_with("Just regular prose")
        ));
    }

    #[tokio::test]
    async fn phase_126_extraction_composes_with_phase_120_fuzzy_recovery() {
        // gemma4 observed emitting `fs.write_file` (a hallucinated
        // alternative). With the operator's tool_name_auto_correct_
        // threshold lowered to 0.5, Phase 120 fuzzy-recovers
        // `fs.write_file` → `fs.write`. The audit entry carries
        // BOTH extracted_from_text AND auto_corrected_from.
        let fs_write = FakeTool::new("fs.write");
        let fs_write_id = fs_write.id;
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::FinalMessage {
                text: "<tool_call>\
                       {\"tool\": \"fs.write_file\", \"parameters\": \
                        {\"path\": \"x\", \"content\": \"y\"}}\
                       </tool_call>"
                    .to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![Arc::new(fs_write)]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("test-model")
                .with_tool_name_auto_correct_threshold(0.5),
        );

        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "save it"))
            .await;
        let step = planner.next_step(&[], &channel).await;
        match step {
            NextStep::ToolCall {
                tool_id,
                auto_corrected_from,
                extracted_from_text,
                ..
            } => {
                assert_eq!(tool_id, fs_write_id, "fuzzy-recovered to fs.write");
                assert_eq!(
                    auto_corrected_from.as_deref(),
                    Some("fs.write_file"),
                    "Phase 120 records the hallucinated original"
                );
                assert_eq!(
                    extracted_from_text.as_deref(),
                    Some("tool_call"),
                    "Phase 126 records the extraction wrapper-tag"
                );
            }
            other => panic!("expected ToolCall composed; got {other:?}"),
        }
    }

    #[tokio::test]
    async fn phase_126_unknown_tool_in_extracted_call_below_threshold_loops_with_error()
    {
        // Extracted tool name not registered AND no fuzzy match
        // (threshold too high). The planner records an unknown_tool
        // error in history and loops; the next-step result is
        // whatever the LLM produces on the second round. Mock
        // returns a clean final message on round 2 so the test
        // can assert the loop's outcome.
        let script = vec![
            FakeStep {
                events: vec![],
                terminal: LlmStepEnd::FinalMessage {
                    text: "<tool_code>\
                           {\"name\": \"nonexistent.tool\", \"arguments\": {}}\
                           </tool_code>"
                        .to_string(),
                    usage: zero_usage(),
                },
            },
            FakeStep {
                events: vec![],
                terminal: LlmStepEnd::FinalMessage {
                    text: "sorry, retrying without tool".to_string(),
                    usage: zero_usage(),
                },
            },
        ];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("test-model"),
        );

        let channel = RecChannel::new();
        planner.begin_turn(&Message::text(channel.session, "do thing")).await;
        let step = planner.next_step(&[], &channel).await;
        // Round 1 produced an unknown_tool error in history; the
        // planner looped to round 2 which returned a clean
        // FinalMessage. The error result must be in history.
        assert!(matches!(step, NextStep::FinalMessage(ref m) if m.contains("sorry")));
        let hist = planner.history();
        let unknown_tool_errors = hist
            .iter()
            .filter(|m| matches!(
                m,
                LlmMessage::ToolResult { content, .. } if content.contains("unknown_tool")
            ))
            .count();
        assert_eq!(
            unknown_tool_errors, 1,
            "extracted call with unknown tool surfaces an unknown_tool error in history"
        );
    }

    #[tokio::test]
    async fn phase_126_multiple_extracted_calls_dispatch_as_batch() {
        // Text contains multiple `<tool_code>` blocks. Planner
        // extracts all of them and returns ToolCalls(batch) when
        // more than one is dispatchable.
        let fs_read = FakeTool::new("fs.read");
        let memory_read = FakeTool::new("memory.read");
        let fs_read_id = fs_read.id;
        let memory_read_id = memory_read.id;
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::FinalMessage {
                text: "Doing two things:\n\
                       <tool_code>{\"name\": \"fs.read\", \"arguments\": {\"path\": \"x\"}}</tool_code>\n\
                       <tool_code>{\"name\": \"memory.read\", \"arguments\": {\"topic\": \"y\"}}</tool_code>"
                    .to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![
            Arc::new(fs_read),
            Arc::new(memory_read),
        ]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("test-model"),
        );

        let channel = RecChannel::new();
        planner.begin_turn(&Message::text(channel.session, "two tasks")).await;
        let step = planner.next_step(&[], &channel).await;
        match step {
            NextStep::ToolCalls(batch) => {
                assert_eq!(batch.len(), 2);
                // Source-order preserved.
                assert_eq!(batch[0].tool_id, fs_read_id);
                assert_eq!(batch[1].tool_id, memory_read_id);
                // Both carry the extraction marker.
                assert_eq!(
                    batch[0].extracted_from_text.as_deref(),
                    Some("tool_code")
                );
                assert_eq!(
                    batch[1].extracted_from_text.as_deref(),
                    Some("tool_code")
                );
            }
            other => panic!("expected ToolCalls(batch); got {other:?}"),
        }
    }

    #[tokio::test]
    async fn phase_126_malformed_extracted_block_drops_silently() {
        // `<tool_code>` block with malformed JSON. Extractor drops
        // it silently; planner sees zero extracted calls; falls
        // through to FinalMessage.
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::FinalMessage {
                text: "<tool_code>not valid json</tool_code>".to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("test-model"),
        );

        let channel = RecChannel::new();
        planner.begin_turn(&Message::text(channel.session, "x")).await;
        let step = planner.next_step(&[], &channel).await;
        // Malformed → no extraction → falls through to FinalMessage
        // with the original raw text.
        assert!(matches!(
            step,
            NextStep::FinalMessage(ref m) if m.contains("not valid json")
        ));
    }

    #[tokio::test]
    async fn phase_126_history_preserves_raw_text_with_tool_code_block() {
        // After extraction, the assistant message pushed to history
        // contains the raw text (including the `<tool_code>` block)
        // alongside the synthesized records. This preserves
        // context for re-feeding the model on the next round.
        let fs_read = FakeTool::new("fs.read");
        let raw_text = "<tool_code>\
                        {\"name\": \"fs.read\", \"arguments\": {\"path\": \"x\"}}\
                        </tool_code>";
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::FinalMessage {
                text: raw_text.to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![Arc::new(fs_read)]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("test-model"),
        );

        let channel = RecChannel::new();
        planner.begin_turn(&Message::text(channel.session, "x")).await;
        let _ = planner.next_step(&[], &channel).await;

        let hist = planner.history();
        // Expect: User → Assistant { text: raw_text, tool_calls: 1 record }
        let assistant_msg = hist
            .iter()
            .find_map(|m| match m {
                LlmMessage::Assistant { text, tool_calls } => Some((text, tool_calls)),
                _ => None,
            })
            .expect("history must include synthesized Assistant message");
        assert_eq!(
            assistant_msg.0, raw_text,
            "raw text including <tool_code> block preserved for context"
        );
        assert_eq!(
            assistant_msg.1.len(),
            1,
            "one synthesized tool_call record matching the extracted block"
        );
        assert_eq!(assistant_msg.1[0].tool_name, "fs.read");
        // Synthesized call_id prefix.
        assert!(
            assistant_msg.1[0].call_id.starts_with("extracted-"),
            "synthesized call_id carries the extraction prefix; got {:?}",
            assistant_msg.1[0].call_id
        );
    }

    // ====================================================
    // Phase 127 Task 7 — planner composition with the new
    // parser families. Each test confirms the end-to-end
    // path works: model emits text in format X, planner
    // extracts via the substrate, synthesizes a ToolCallEnd,
    // dispatches to the registered tool. The `extracted_
    // from_text` audit field carries the wrapper-tag through
    // to the audit chain.
    //
    // The FakeLlmProvider used here does NOT override
    // `tool_call_family_hint`, so the family hint is None
    // and the substrate uses default priority order. The
    // parsers don't ambiguously match on these inputs, so
    // hint-less extraction produces correct results.
    // ====================================================

    #[tokio::test]
    async fn phase_127_planner_extracts_qwen3_coder_xml() {
        // Qwen3.5/3.6 emits XML inside `<tool_call>` per
        // Ollama issue #14745. Phase 127 Task 2's parser
        // catches it; the planner dispatches via the same
        // Phase 120/101 helper as protocol tool calls.
        let fs_write = FakeTool::new("fs.write");
        let fs_write_id = fs_write.id;
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::FinalMessage {
                text: "<tool_call>\
                       <function=fs.write>\
                       <parameter=path>test.txt</parameter>\
                       <parameter=content>phase 127</parameter>\
                       </function>\
                       </tool_call>"
                    .to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![Arc::new(fs_write)]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("test-model"),
        );

        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "write a file"))
            .await;
        let step = planner.next_step(&[], &channel).await;
        match step {
            NextStep::ToolCall {
                tool_id,
                extracted_from_text,
                input,
                ..
            } => {
                assert_eq!(tool_id, fs_write_id);
                assert_eq!(
                    extracted_from_text.as_deref(),
                    Some("tool_call"),
                    "wrapper-tag for Qwen3-Coder XML extracted call"
                );
                assert_eq!(input["path"], "test.txt");
                assert_eq!(input["content"], "phase 127");
            }
            other => panic!("expected ToolCall; got {other:?}"),
        }
    }

    #[tokio::test]
    async fn phase_127_planner_extracts_phi4_mini_list() {
        // Phi-4-mini emits a JSON array inside the
        // `<|tool_call|>` special-token wrapper. Even a
        // single-element list goes through the JSON-list
        // path. wrapper_tag is the literal `"|tool_call|"`
        // (with bars).
        let fs_write = FakeTool::new("fs.write");
        let fs_write_id = fs_write.id;
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::FinalMessage {
                text: r#"<|tool_call|>[{"name": "fs.write", "arguments": {"path": "phi.txt"}}]<|/tool_call|>"#
                    .to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![Arc::new(fs_write)]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("test-model"),
        );

        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "x"))
            .await;
        let step = planner.next_step(&[], &channel).await;
        match step {
            NextStep::ToolCall {
                tool_id,
                extracted_from_text,
                input,
                ..
            } => {
                assert_eq!(tool_id, fs_write_id);
                assert_eq!(
                    extracted_from_text.as_deref(),
                    Some("|tool_call|"),
                    "wrapper-tag carries the bars verbatim for grep-distinctness"
                );
                assert_eq!(input["path"], "phi.txt");
            }
            other => panic!("expected ToolCall; got {other:?}"),
        }
    }

    #[tokio::test]
    async fn phase_127_planner_extracts_gemma3_python_fence() {
        // Gemma 3 emits Python-call syntax inside a
        // ```tool_code` markdown fence. Phase 127 Task 4's
        // hand-written recursive-descent parser translates
        // Python kwargs into JSON arguments.
        let fs_write = FakeTool::new("fs.write");
        let fs_write_id = fs_write.id;
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::FinalMessage {
                text: "```tool_code\nfs.write(path='gemma.txt', content='hi')\n```"
                    .to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![Arc::new(fs_write)]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("test-model"),
        );

        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "x"))
            .await;
        let step = planner.next_step(&[], &channel).await;
        match step {
            NextStep::ToolCall {
                tool_id,
                extracted_from_text,
                input,
                ..
            } => {
                assert_eq!(tool_id, fs_write_id);
                assert_eq!(
                    extracted_from_text.as_deref(),
                    Some("tool_code_fence"),
                    "wrapper-tag distinguishes the markdown fence from bare <tool_code>"
                );
                assert_eq!(input["path"], "gemma.txt");
                assert_eq!(input["content"], "hi");
            }
            other => panic!("expected ToolCall; got {other:?}"),
        }
    }

    #[tokio::test]
    async fn phase_127_planner_extracts_bare_json() {
        // Some Ollama models (qwen3:32b per issue #11662)
        // emit raw JSON with no wrapper at all. Phase 127
        // Task 5's bare-JSON fallback catches it when the
        // entire response is exactly one tool-call-shaped
        // JSON object. wrapper_tag is `"(bare)"` to
        // communicate "no wrapper detected".
        let fs_read = FakeTool::new("fs.read");
        let fs_read_id = fs_read.id;
        let script = vec![FakeStep {
            events: vec![],
            terminal: LlmStepEnd::FinalMessage {
                text: r#"{"name": "fs.read", "arguments": {"path": "x"}}"#
                    .to_string(),
                usage: zero_usage(),
            },
        }];
        let provider = FakeLlmProvider::new(script);
        let registry = Arc::new(ToolRegistry::new(vec![Arc::new(fs_read)]));
        let mut planner = LlmPlanner::new(
            provider,
            registry,
            LlmPlannerConfig::new("test-model"),
        );

        let channel = RecChannel::new();
        planner
            .begin_turn(&Message::text(channel.session, "x"))
            .await;
        let step = planner.next_step(&[], &channel).await;
        match step {
            NextStep::ToolCall {
                tool_id,
                extracted_from_text,
                input,
                ..
            } => {
                assert_eq!(tool_id, fs_read_id);
                assert_eq!(
                    extracted_from_text.as_deref(),
                    Some("(bare)"),
                    "wrapper-tag for bare-JSON is `(bare)`"
                );
                assert_eq!(input["path"], "x");
            }
            other => panic!("expected ToolCall; got {other:?}"),
        }
    }
}

//! `run_session` — the reusable REPL loop shared by the `aivyx` binary
//! and Phase 3 task 5's end-to-end integration test.
//!
//! ## Why this lives in the library
//!
//! Phase 3 task 5 needs a hermetic test that drives the full CLI stack
//! (planner → provider → audit → channel) via scripted I/O, without
//! touching the network. The binary's `main.rs` hardcodes
//! `AnthropicProvider::new(...)` + `io::stdin()` + `io::stdout()`, none
//! of which a test can intercept. Two options exist:
//!
//! 1. Add a test-only transport backdoor to the binary.
//! 2. Extract the REPL itself into a library function parameterized by
//!    `Arc<dyn LlmProvider>` + `impl BufRead` + `impl Write`, so tests
//!    call the library directly while `main.rs` stays a thin wiring
//!    layer.
//!
//! Option 2 is cleaner: it separates *composition* (what `main` decides
//! at process start — where secrets come from, which provider backs the
//! planner, which sinks I/O talks to) from *execution* (what a turn
//! actually does). The test gets to swap composition without ever
//! calling `main`.
//!
//! ## What this module owns
//!
//! - [`SessionConfig`] — the per-session knobs: model id, system prompt,
//!   max output tokens, capability set. Produced from the binary's
//!   env-var parsing or from a test fixture.
//! - [`run_session`] — the REPL loop itself. Reads user input from the
//!   `reader` one line at a time, rotates the channel's cancellation
//!   token, drives a turn through the provided agent, and keeps going
//!   until EOF. Returns a [`SessionReport`] the test can assert on.
//! - [`SessionReport`] — how many turns ran and the final outcome of
//!   the last turn. Minimal by design; audit verification goes through
//!   the `AuditBridge::writer()` handle the caller already holds.
//!
//! ## What this module deliberately does **not** own
//!
//! - **Signal handling.** The signal task in `main.rs` spawns a
//!   `tokio::signal::ctrl_c` listener that reads the channel's token
//!   slot. Unit tests don't send Unix signals, and wiring a signal
//!   listener into an integration test would be flaky. The channel's
//!   `reset_cancellation()` call on every iteration is the only loop-
//!   level piece of ctrl-C machinery, and that's here.
//! - **Secret handling.** `SessionConfig` takes plain `String`s for
//!   `model` and `system_prompt`. The API key lives inside whichever
//!   `LlmProvider` the caller supplies — `AnthropicProvider` holds a
//!   `SecretString` internally, and tests use a `FakeLlmProvider` with
//!   no secret at all. Keeping the session layer secret-free means the
//!   test path never has to mint a fake key.

use std::io::{BufRead, Write};
use std::sync::Arc;

use aivyx_capability::CapabilitySet;
use aivyx_core::{
    agent::ConcreteAgent, llm_planner::LlmPlanner, planner::ToolRegistry, Agent, AgentId,
    AuditHook, ChannelContext, LlmPlannerConfig, Message, TurnOutcome,
};
use aivyx_llm::LlmProvider;

use crate::LocalChannel;

/// Knobs the REPL needs to construct one session's planner + agent.
pub struct SessionConfig {
    pub model: String,
    pub system_prompt: String,
    pub max_tokens: u32,
    /// The agent's capability set. Defaults in the binary are broad
    /// (`memory.read`, `memory.write`) because the local CLI is the
    /// most-trusted channel on the box; tests may pick their own.
    pub capabilities: CapabilitySet,
    /// Prompt string written before each `read_line`. The binary
    /// passes `"> "`; tests usually pass `""` so captured output is
    /// easier to assert on.
    pub prompt: String,
    /// Banner line printed once at session start, before the first
    /// prompt. `None` means "no banner" — the test path uses this to
    /// keep stdout output deterministic.
    pub banner: Option<String>,
}

/// Summary of what the session did, returned after EOF.
#[derive(Debug, Clone)]
pub struct SessionReport {
    /// Number of non-empty lines the user fed in that actually ran a
    /// turn. Empty lines and whitespace-only lines are skipped and do
    /// not count.
    pub turns_run: usize,
    /// Outcome of the last turn, if any. `None` means the session
    /// never saw a non-empty input line.
    pub last_outcome: Option<TurnOutcome>,
}

/// Drive a single CLI session to completion.
///
/// The loop reads lines from `reader` (usually `io::stdin().lock()` in
/// production or a `Cursor` in tests), dispatches each non-empty line
/// as a `Message::text` through the agent, and streams the resulting
/// events to the `LocalChannel` wrapped around `writer`. Returns when
/// `reader` signals EOF (`read_line` returns `Ok(0)`).
///
/// The `provider` is an `Arc<dyn LlmProvider>` so the binary can pass
/// a live `AnthropicProvider` and the integration test can pass a
/// `FakeLlmProvider`. Both flow through the same `LlmPlanner` +
/// `ConcreteAgent` stack.
///
/// The `audit` hook is passed in rather than constructed here because
/// the test wants to inspect the chain afterwards via its own
/// `AuditBridge::writer()` handle, and the binary wants to use
/// `/dev/urandom` for the key while the test wants a deterministic one.
pub async fn run_session<R, W>(
    provider: Arc<dyn LlmProvider>,
    audit: Arc<dyn AuditHook>,
    config: SessionConfig,
    channel: LocalChannel<W>,
    mut reader: R,
) -> Result<SessionReport, String>
where
    // `R: BufRead` is intentionally *not* `Send`: the binary's
    // `io::StdinLock` is not `Send`, and `run_session` is always
    // driven from a single task (there's no internal `spawn` that
    // crosses threads with the reader), so a `Send` bound would be
    // a phantom requirement that just breaks the real caller.
    R: BufRead,
    W: Write + Send + 'static,
{
    // ---- Agent stack --------------------------------------------------
    // An empty ToolRegistry — Phase 3 is "first real channel," not
    // "first real tool." The planner's tool descriptor list is empty
    // and any FinalMessage-only chat works end-to-end.
    let registry = Arc::new(ToolRegistry::new(Vec::new()));

    // Planner factory — fresh planner per turn. Captures the provider
    // Arc, the registry Arc, and a planner config by value (cloned
    // per-turn; `LlmPlannerConfig` is small).
    let provider_for_factory = Arc::clone(&provider);
    let registry_for_factory = Arc::clone(&registry);
    let planner_config = LlmPlannerConfig::new(config.model)
        .with_system_prompt(config.system_prompt)
        .with_max_tokens(config.max_tokens);

    let agent = ConcreteAgent::new(
        AgentId::new(),
        config.capabilities,
        registry,
        audit,
        move || {
            Box::new(LlmPlanner::new(
                Arc::clone(&provider_for_factory),
                Arc::clone(&registry_for_factory),
                planner_config.clone(),
            ))
        },
    );

    // ---- Banner ------------------------------------------------------
    //
    // Printed to the same writer the channel will stream through. We
    // reach into `writer_handle()` rather than adding a separate
    // `Banner` event because the channel's `StreamEvent` vocabulary is
    // locked by D3 and the banner is not a turn event.
    if let Some(banner) = config.banner.as_deref() {
        let writer = channel.writer_handle();
        let mut guard = writer
            .lock()
            .map_err(|e| format!("writer mutex poisoned: {e}"))?;
        writeln!(&mut *guard, "{banner}")
            .map_err(|e| format!("banner write failed: {e}"))?;
        guard
            .flush()
            .map_err(|e| format!("banner flush failed: {e}"))?;
    }

    // ---- REPL --------------------------------------------------------
    let mut turns_run: usize = 0;
    let mut last_outcome: Option<TurnOutcome> = None;
    let mut line = String::new();

    loop {
        // Prompt is written directly to the channel's writer so the
        // test's captured output reflects exactly what the user would
        // have seen. In the binary case (`io::Stdout`), this is the
        // same file descriptor a bare `print!` would reach.
        if !config.prompt.is_empty() {
            let writer = channel.writer_handle();
            let mut guard = writer
                .lock()
                .map_err(|e| format!("writer mutex poisoned: {e}"))?;
            write!(&mut *guard, "{}", config.prompt)
                .map_err(|e| format!("prompt write failed: {e}"))?;
            guard
                .flush()
                .map_err(|e| format!("prompt flush failed: {e}"))?;
        }

        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                return Ok(SessionReport {
                    turns_run,
                    last_outcome,
                });
            }
            Ok(_) => {}
            Err(e) => return Err(format!("failed to read from input: {e}")),
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        // Rotate the channel's cancellation token so a turn-N cancel
        // does not pre-cancel turn N+1. Same reasoning as the binary:
        // `tokio_util::CancellationToken` is monotonic, so we swap in
        // a fresh one per turn.
        channel.reset_cancellation();

        let message = Message::text(channel.session_id(), input);
        let outcome = agent.turn(message, &channel).await;
        turns_run += 1;
        last_outcome = Some(outcome);
    }
}

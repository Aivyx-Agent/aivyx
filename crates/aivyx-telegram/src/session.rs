//! `run_telegram_session` — the Telegram analogue of
//! [`aivyx_channel::run_session`].
//!
//! ## Why a sibling function and not a shared abstraction
//!
//! `aivyx_channel::run_session` takes `channel: LocalChannel<W>` +
//! `R: BufRead` as concrete parameters. A Telegram adapter has
//! neither: there is no local writer to hand it, and inbound messages
//! arrive via a long-poll cursor, not a line-oriented reader. Two
//! shapes to resolve this were considered in PHASE_8.md Task 4:
//!
//! 1. Generalize `run_session` to take `&dyn ChannelContext` plus an
//!    abstract input source trait.
//! 2. **Write a sibling `run_telegram_session` that owns its own
//!    long-poll loop.**
//!
//! We picked (2). The local and Telegram lifecycles are different
//! enough — pulled line-by-line vs. pushed long-poll batches — that
//! shoehorning them into one trait would invent an abstraction that
//! has exactly two implementations and would need to be rethought
//! the moment a third adapter (webhook Matrix? push-driven Discord
//! gateway?) arrives. The ~100 lines of "duplicated" wiring here is
//! honest — it's the price of keeping each adapter's event-pump
//! code legible in isolation.
//!
//! ## What this function owns (vs. `run_session`)
//!
//! Same as the local path:
//!
//! - Builds the `ConcreteAgent` from [`TelegramSessionConfig`] —
//!   the Telegram-flavored analogue of `aivyx_channel::SessionConfig`.
//!   Why a separate type: `aivyx-telegram` cannot depend on
//!   `aivyx-channel` without creating a package cycle (the `aivyx`
//!   binary lives in `aivyx-channel` and will import the Telegram
//!   entry point). See [`TelegramSessionConfig`] for the exact
//!   shape; the binary converts its `SessionConfig` fields over
//!   field-by-field at the call site.
//! - Rotates the channel's cancellation token per turn (Phase 3
//!   monotonic-token fix: a cancelled turn must not poison turn N+1).
//! - Runs `agent.turn(message, &channel).await` for each inbound
//!   message.
//!
//! Deliberately omitted (compared to `run_session`):
//!
//! - **No session marker write under `KeyDomain::Sessions`.** The
//!   local session marker is a single-row "current session" record
//!   keyed on a fixed key. A Telegram bot serves many chats from one
//!   process; the analogous "per-chat session marker" would need a
//!   different schema (one row per chat_id) and a new key convention.
//!   Phase 8 defers that work — see PHASE_8.md Task 4 ship record.
//!   The `storage` field on `TelegramSessionConfig` is still carried
//!   through, so a future refinement can wire markers without
//!   changing the call-site shape.
//! - **No banner.** Telegram bots don't have a "session start"
//!   affordance the way a terminal REPL does; the first user message
//!   is the banner. A startup-ping message could be a Task 7 smoke-
//!   test concern.
//! - **No prompt string.** Same reason — Telegram's "prompt" is the
//!   user pressing Send, not a character printed by the bot.
//! - **No signal handling.** The binary still owns the `ctrl_c`
//!   listener task that cancels the `shutdown` token this function
//!   receives as a parameter; the loop only checks the token state
//!   at the top of each iteration.
//!
//! ## The long-poll cursor
//!
//! `get_updates(offset, timeout_secs)` is the Bot API's long-poll
//! entry point: the server holds the request open up to `timeout_secs`
//! seconds, and returns as soon as any updates with `update_id >=
//! offset` arrive (or an empty list on timeout). `offset` is the
//! **acknowledgment cursor** — passing `last_seen + 1` tells Telegram
//! "I've handled everything up to `last_seen`, don't send them again."
//!
//! We keep a local `i64` cursor initialized to 0. After each batch of
//! updates, we advance the cursor to `max(update_id) + 1`. A Telegram
//! server restart or offset reset does not cause replay — the cursor
//! is monotonic within the process lifetime.
//!
//! ## Chat filtering
//!
//! The current design binds one `TelegramChannel` to one `chat_id`
//! (Phase 8 Task 1's simplification). `get_updates` will return
//! updates for **every chat the bot is in**, not just the target one,
//! so we filter inbound messages by `chat_id` at the top of the loop.
//! Messages from other chats are silently dropped in Phase 8; a
//! multi-chat pump that spawns one `TelegramChannel` per chat_id and
//! routes accordingly is a Phase 9 concern.

use std::sync::Arc;
use std::time::Duration;

use aivyx_capability::CapabilitySet;
use aivyx_core::{
    agent::ConcreteAgent, llm_planner::LlmPlanner, planner::ToolRegistry, Agent, AgentId,
    AuditHook, CancellationToken, ChannelContext, LlmPlannerConfig, Message,
};
use aivyx_llm::LlmProvider;
use aivyx_storage::Storage;

use crate::telegram_channel::TelegramChannel;
use crate::transport::{ReqwestTransport, TelegramTransport};

/// Per-session knobs for the Telegram loop. Analogue of
/// [`aivyx_channel::SessionConfig`], minus the local-only `prompt`
/// and `banner` fields. Kept as a separate type (rather than an
/// import) to avoid a package-level cycle between `aivyx-telegram`
/// and `aivyx-channel` — see the `Cargo.toml` comment for details.
///
/// Field semantics are identical to the corresponding fields on
/// `SessionConfig`. A future refactor that extracts the shared shape
/// into a third crate would collapse both into one type without
/// touching any call sites.
pub struct TelegramSessionConfig {
    pub model: String,
    pub system_prompt: String,
    pub max_tokens: u32,
    pub capabilities: CapabilitySet,
    pub tools: Arc<ToolRegistry>,
    pub storage: Arc<dyn Storage>,
}

/// How long to hold each `getUpdates` request open (seconds).
///
/// Telegram's Bot API allows up to 50; we pick a conservative 25 so a
/// bot that is killed mid-poll comes back within ~half a minute. Tests
/// override this via `run_telegram_session_with_transport` so they
/// don't wait on real-world timeouts.
const LONG_POLL_TIMEOUT_SECS: u32 = 25;

/// Summary of what one Telegram session did, returned after the long-
/// poll cursor is shut down. Matches the shape of
/// [`aivyx_channel::SessionReport`] deliberately — a future refactor
/// that unifies the two session functions would fold these into one
/// type. Kept separate for now so the Phase 8 empty-diff streak on
/// `aivyx-channel` isn't touched.
#[derive(Debug, Clone)]
pub struct TelegramSessionReport {
    /// Number of inbound text messages that drove a turn to completion.
    pub turns_run: usize,
}

/// Drive a Telegram session to completion against a real bot token.
///
/// This is the production entry point for the `aivyx --channel
/// telegram` binary path. It builds a [`TelegramChannel`] over the
/// production `ReqwestTransport`, then delegates to the generic
/// [`run_telegram_session_with_transport`] that unit tests also call.
///
/// ## Parameters
///
/// - `channel_name` — human label for the channel, surfaced in audit
///   events and error messages. Defaults usefully to `"aivyx-telegram"`
///   in the binary.
/// - `token` — the Bot API token. Passed straight through to
///   `frankenstein::client_reqwest::Bot::new`; nothing in this crate
///   logs or echoes it. The caller is responsible for sourcing it
///   safely (the binary reads `AIVYX_TELEGRAM_TOKEN` from the
///   environment — see PHASE_8.md Q1 resolution).
/// - `chat_id` — the Telegram chat this session is bound to. One
///   channel per chat_id is Phase 8's simplification.
/// - `config` — reused `SessionConfig` from the local path. The
///   `banner` and `prompt` fields are ignored for Telegram (see the
///   module doc).
/// - `provider` / `audit` — same trait objects the local binary
///   hands to `run_session`. Identical contracts.
/// - `shutdown` — an external cancellation token the binary's ctrl-C
///   signal handler can cancel to tell the session to exit after the
///   current long-poll batch drains. Passed separately from the
///   channel's own cancellation token because the channel's token
///   rotates per-turn (a turn N cancel must not poison turn N+1, see
///   PHASE_8.md Q5 and `LocalChannel::reset_cancellation`), so it
///   isn't a stable shutdown signal. This token is checked at the
///   top of each loop iteration.
pub async fn run_telegram_session(
    channel_name: impl Into<String>,
    token: &str,
    chat_id: i64,
    config: TelegramSessionConfig,
    provider: Arc<dyn LlmProvider>,
    audit: Arc<dyn AuditHook>,
    shutdown: CancellationToken,
) -> Result<TelegramSessionReport, String> {
    let transport = Arc::new(ReqwestTransport::new(token));
    let channel = Arc::new(TelegramChannel::new(
        channel_name,
        chat_id,
        Arc::clone(&transport),
    ));
    run_telegram_session_with_transport(
        channel,
        config,
        provider,
        audit,
        LONG_POLL_TIMEOUT_SECS,
        shutdown,
    )
    .await
}

/// Transport-generic session driver. The production path calls this
/// with a `TelegramChannel<ReqwestTransport>`; unit tests call it with
/// a `TelegramChannel<ScriptedTransport>`. The same function body
/// drives both.
///
/// Visibility is `pub(crate)` rather than `pub` because the private
/// `TelegramTransport` trait appears in the bound — exposing this
/// publicly would leak the trait. The public surface for production
/// callers is [`run_telegram_session`] above; the test surface is
/// this function, called from within the crate's own `tests` module.
pub(crate) async fn run_telegram_session_with_transport<T>(
    channel: Arc<TelegramChannel<T>>,
    config: TelegramSessionConfig,
    provider: Arc<dyn LlmProvider>,
    audit: Arc<dyn AuditHook>,
    long_poll_timeout_secs: u32,
    shutdown: CancellationToken,
) -> Result<TelegramSessionReport, String>
where
    T: TelegramTransport + 'static,
{
    // ---- Agent stack --------------------------------------------------
    // Exactly the same shape as `run_session`: a fresh `LlmPlanner`
    // per turn, captured by the factory closure. Keeping this in lock-
    // step with the local path means a planner-state bug that shows
    // up locally also shows up through Telegram (and vice versa),
    // which is the invariant we want.
    let registry = config.tools;
    let _storage = config.storage; // Kept alive; not used for markers this phase.

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

    // ---- Long-poll loop ----------------------------------------------
    //
    // `offset` is the "give me everything with update_id >= offset"
    // cursor. 0 on first iteration is the Bot API's "send me everything
    // you've got buffered for this bot" sentinel; subsequent iterations
    // advance to `max(update_id) + 1`.
    let mut offset: i64 = 0;
    let mut turns_run: usize = 0;
    let target_chat = channel.chat_id();
    let transport = channel.transport();

    loop {
        // Check both the process-wide shutdown signal AND the
        // channel's per-turn token *before* the long-poll call so a
        // ctrl-C received between turns exits immediately rather than
        // stalling up to `long_poll_timeout_secs` seconds waiting for
        // the Bot API to return an empty batch.
        //
        // Why two tokens: the per-turn token rotates on every turn
        // (see `reset_cancellation` above) so that turn N's cancel
        // doesn't poison turn N+1 — that's the Phase 3 monotonic-
        // token fix. A process-wide shutdown signal needs to survive
        // that rotation, so it's a separate token the binary's
        // ctrl-C handler holds and cancels. The session unit test
        // at `tests::run_telegram_session_drives_two_scripted_turns`
        // cancels the per-turn token externally (simulating a ctrl-C
        // that happened to land between turns); the binary uses the
        // `shutdown` parameter proper.
        if shutdown.is_cancelled() || channel.cancellation_token().is_cancelled() {
            return Ok(TelegramSessionReport { turns_run });
        }

        let updates = match transport
            .get_updates(offset, long_poll_timeout_secs)
            .await
        {
            Ok(batch) => batch,
            Err(e) => {
                // Platform errors at the poll layer are non-fatal:
                // Bot API 5xx, transient network flakiness, rate-
                // limit 429s. Log to stderr and back off briefly so
                // we don't hot-loop a broken network. Same shape as
                // the session-marker error handling in `run_session`:
                // a degraded transport should never take down the
                // bot's ability to serve later messages.
                eprintln!("aivyx-telegram: get_updates failed ({e}); backing off 1s");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        if updates.is_empty() {
            // Empty batch — Bot API long-poll timed out with no new
            // messages. Loop immediately to re-arm.
            continue;
        }

        for msg in updates {
            // Advance the cursor regardless of whether we handle the
            // message, so a malformed message from a chat we're not
            // targeting doesn't cause the same update to be redelivered
            // next poll.
            offset = offset.max(msg.update_id + 1);

            if msg.chat_id != target_chat {
                // Multi-chat pumping is a Phase 9 concern; for now,
                // drop anything that isn't for this channel's chat.
                continue;
            }

            // Rotate cancellation per turn — identical rationale to
            // `LocalChannel::reset_cancellation` in the local path.
            // `tokio_util::CancellationToken` is monotonic, so a
            // previously-cancelled turn would poison turn N+1 if we
            // didn't swap the slot.
            channel.reset_cancellation();

            let message = Message::text(channel.session_id(), &msg.text);
            let _outcome = agent.turn(message, channel.as_ref()).await;
            turns_run += 1;
        }
    }
}

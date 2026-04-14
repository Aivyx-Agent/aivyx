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

use std::collections::VecDeque;
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
use crate::transport::{IncomingMessage, ReqwestTransport, TelegramTransport, TransportError};

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

/// How long each `scan_for_cancel` `getUpdates` call holds the connection
/// open, in seconds. The Phase 8 Q8 design note called out ~2 seconds
/// as the sweet spot: long enough that a user typing `/cancel` during
/// a 30-second turn has multiple scan iterations to land on, short
/// enough that a normal fast turn doesn't pay a noticeable wait cost
/// on the losing select arm when the turn finishes quickly.
///
/// Tests override this via `run_telegram_session_with_transport_ex`.
const SCAN_FOR_CANCEL_TIMEOUT_SECS: u32 = 2;

/// Result of one `scan_for_cancel` probe. The session loop consumes
/// this to advance its shared `offset` cursor and to reshuffle any
/// updates the scan saw into the next turn's pending queue.
///
/// **Why the scan returns messages the turn loop has to re-queue, not
/// the whole next-turn decision.** The scan is a transport-layer
/// helper; the turn-loop logic of "what counts as a cancel" and "what
/// to do next" stays in `run_telegram_session_with_transport`. The
/// scan's only job is to answer: "did a `/cancel` land, and what
/// other target-chat messages arrived in the same batch?"
///
/// **Design note on queueing vs. redelivery (PHASE_8.md:1376–1380
/// open question).** We pick queueing: messages that arrive *alongside*
/// `/cancel` in the same scan batch are appended to the session
/// loop's pending queue in `update_id` order and drive subsequent
/// turns. The rejected alternative was "drop them, let Telegram
/// redeliver next poll" — simpler, but worse UX. A user who types
/// "do X" then immediately "/cancel" shouldn't lose the X; a user
/// who types "/cancel" then immediately "do Y" shouldn't lose the Y.
/// Queueing preserves both.
#[derive(Debug)]
enum ScanResult {
    /// The scan batch did not contain a `/cancel` from the target chat.
    /// `queued` is any target-chat messages the scan *did* see (the
    /// main loop prepends these to its pending queue; they're
    /// non-cancel messages that the main loop can process after the
    /// current turn finishes). `max_update_id` is the highest
    /// `update_id` the scan observed from *any* chat, which the main
    /// loop uses to advance the offset cursor past this batch so
    /// Telegram doesn't redeliver it on the next `get_updates` call.
    NoCancel {
        max_update_id: Option<i64>,
        queued: Vec<IncomingMessage>,
    },
    /// The scan batch contained a `/cancel` from the target chat. The
    /// session loop cancels the channel's per-turn token so the
    /// in-flight `agent.turn` resolves as `Cancelled`. `queued` is any
    /// target-chat messages in the same batch with `update_id` other
    /// than `cancel_update_id`, in batch order — they are re-queued
    /// for subsequent turns per the queueing-over-redelivery design.
    /// `cancel_update_id` is the highest `update_id` between `/cancel`
    /// itself and any observed queued-message update_ids, used to
    /// advance the offset cursor.
    FoundCancel {
        cancel_update_id: i64,
        queued: Vec<IncomingMessage>,
    },
}

/// Probe the Bot API for a short window, looking for a `/cancel`
/// command from `target_chat`. This is the "scanning arm" of the
/// `tokio::select!` inside the session loop's per-turn block — the
/// other arm is `agent.turn(...)` itself.
///
/// **Cancellation-safety invariant the caller relies on.** When the
/// turn arm wins the `select!`, this future is dropped mid-`await` on
/// `get_updates`. For the production `ReqwestTransport`, dropping the
/// reqwest future cancels the in-flight HTTP request *before* the Bot
/// API's server-side cursor advances — so the next main-loop
/// `get_updates(offset, ...)` with the same `offset` reproduces the
/// same batch (or a superset). For the test `ScriptedTransport`,
/// `get_updates` eagerly drains its internal queue, which is a test-
/// artifact that does *not* model production precisely; the tests
/// compensate by driving the scan to completion and reading the
/// returned `ScanResult` rather than relying on select-drop.
///
/// **`/cancel` detection.** A message counts as a cancel iff its
/// `text.trim() == "/cancel"` (case-sensitive, no arguments). Bot
/// Mention forms like `/cancel@MyBotName` are a Phase 9+ refinement —
/// they require reading the bot's `getMe` username, which this seam
/// doesn't carry. A future task can thread the username through
/// `TelegramSessionConfig` if the simpler form proves insufficient.
///
/// **Non-target-chat messages.** A scan batch can return messages
/// for *any* chat this bot is in (Bot API behavior). Anything that
/// isn't for `target_chat` is silently dropped here, mirroring the
/// main-loop filter. Its `update_id` still contributes to
/// `max_update_id` so the cursor advances past it.
async fn scan_for_cancel<T: TelegramTransport + ?Sized>(
    transport: &T,
    offset: i64,
    target_chat: i64,
    scan_timeout_secs: u32,
) -> Result<ScanResult, TransportError> {
    let batch = transport.get_updates(offset, scan_timeout_secs).await?;

    let mut max_update_id: Option<i64> = None;
    let mut queued: Vec<IncomingMessage> = Vec::new();
    let mut cancel_update_id: Option<i64> = None;

    for msg in batch {
        max_update_id = Some(max_update_id.map_or(msg.update_id, |m| m.max(msg.update_id)));

        if msg.chat_id != target_chat {
            continue;
        }

        if msg.text.trim() == "/cancel" {
            // Remember only the *first* /cancel in the batch. A batch
            // with multiple cancels is a pathological case (the user
            // mashed the command), and one cancel is enough to fire
            // the branch. Subsequent cancels are dropped (they'd
            // cancel an already-cancelled turn).
            if cancel_update_id.is_none() {
                cancel_update_id = Some(msg.update_id);
            }
            // Do NOT queue the /cancel message itself — it is a
            // control signal, not a prompt. A user shouldn't see the
            // bot respond to "/cancel" as if it were a question.
        } else {
            queued.push(msg);
        }
    }

    if let Some(cancel_id) = cancel_update_id {
        // The cancel_update_id returned to the caller is the *cursor
        // advance target*: the highest update_id the scan observed,
        // whether that's the cancel itself, a queued normal message,
        // or a non-target-chat message. The main loop will advance
        // `offset` to `cancel_update_id + 1`.
        let advance_to = max_update_id.unwrap_or(cancel_id).max(cancel_id);
        Ok(ScanResult::FoundCancel {
            cancel_update_id: advance_to,
            queued,
        })
    } else {
        Ok(ScanResult::NoCancel {
            max_update_id,
            queued,
        })
    }
}

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
    //
    // `pending` is a per-target-chat queue of messages waiting to be
    // turned into agent turns. It is usually refilled from the main
    // `get_updates` call at the top of each outer iteration, but the
    // `/cancel` scan arm can also push messages it saw *during* a turn
    // onto this queue (at the front, if a cancel arrived and pre-cancel
    // messages need to run before the current turn's replacement; at
    // the back, otherwise). This is the queueing-over-redelivery
    // choice documented on `ScanResult`.
    let mut offset: i64 = 0;
    let mut turns_run: usize = 0;
    let mut pending: VecDeque<IncomingMessage> = VecDeque::new();
    let target_chat = channel.chat_id();
    let transport = channel.transport();

    loop {
        // Process-wide shutdown signal is checked every outer iteration:
        // a ctrl-C received between turns (or between long-polls) must
        // exit immediately rather than stalling up to
        // `long_poll_timeout_secs` seconds waiting for the Bot API to
        // return an empty batch.
        if shutdown.is_cancelled() {
            return Ok(TelegramSessionReport { turns_run });
        }

        // If there's nothing pending, refill from a fresh long-poll.
        // When the scan arm has queued messages from a mid-turn batch,
        // we skip this — we'd prefer to drain the scan-provided queue
        // first before paying for another round-trip.
        if pending.is_empty() {
            // Channel-token fast path: the per-turn token is rotated
            // at the top of each turn (see `reset_cancellation` below)
            // so it is a turn-internal signal, not an inter-turn one.
            // Between turns the token is free to carry the previous
            // turn's cancelled state — we must only check it when
            // we're about to *block* on a long-poll, so that a ctrl-C
            // equivalent that happened to land between a turn and its
            // long-poll can short-circuit the wait. The
            // `tests::run_telegram_session_drives_two_scripted_turns`
            // test relies on this: it cancels the channel token
            // externally (simulating a shutdown that lands between
            // turns) and expects the session to exit on the next
            // pre-poll check.
            //
            // Before Phase 9 Task 1 this check lived above the
            // `pending.is_empty()` branch and fired on *every* outer
            // iteration, which was fine when the old loop also did
            // all per-turn work inside the same outer iteration. With
            // Task 1's `pending: VecDeque` carrying across outer
            // iterations, that placement was subtly wrong: it would
            // observe the still-cancelled per-turn token *between*
            // turns of one long-poll batch and exit before turn N+1
            // got a chance to rotate the slot. Moving it here — only
            // on the pre-long-poll path — restores the Phase 8 Task 5
            // "cancelled turn 1, then turn 2 runs normally" invariant.
            if channel.cancellation_token().is_cancelled() {
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
                    // limit 429s. Log to stderr and back off briefly
                    // so we don't hot-loop a broken network. Same
                    // shape as the session-marker error handling in
                    // `run_session`: a degraded transport should
                    // never take down the bot's ability to serve
                    // later messages.
                    eprintln!("aivyx-telegram: get_updates failed ({e}); backing off 1s");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };

            if updates.is_empty() {
                // Empty batch — Bot API long-poll timed out with no
                // new messages. Loop immediately to re-arm.
                continue;
            }

            for msg in updates {
                // Advance the cursor regardless of whether we handle
                // the message, so a malformed message from a chat
                // we're not targeting doesn't cause the same update
                // to be redelivered next poll.
                offset = offset.max(msg.update_id + 1);

                if msg.chat_id != target_chat {
                    // Multi-chat pumping is a Phase 9 concern; for
                    // now, drop anything that isn't for this
                    // channel's chat.
                    continue;
                }

                // A stray top-of-loop `/cancel` with no turn running
                // is a no-op — there's nothing to cancel. We drop it
                // rather than queueing it (a user shouldn't see the
                // bot respond to "/cancel" as if it were a question;
                // same rationale as the scan arm).
                if msg.text.trim() == "/cancel" {
                    continue;
                }

                pending.push_back(msg);
            }

            if pending.is_empty() {
                // Entire batch was non-target-chat noise or /cancels
                // with no turn to cancel. Skip straight to the next
                // long-poll without trying to run a turn.
                continue;
            }
        }

        // Dequeue the next message and run a turn for it, racing
        // `scan_for_cancel` against the turn to watch for an in-band
        // `/cancel`. The scan arm never completes a turn itself — its
        // only jobs are (a) advancing `offset` past any batch it sees
        // and (b) cancelling the channel's per-turn token when it
        // observes `/cancel`, which then lets the biased turn arm win
        // the next select iteration with `TurnOutcome::Cancelled`.
        let msg = pending.pop_front().expect("pending is non-empty here");

        // Rotate cancellation per turn — identical rationale to
        // `LocalChannel::reset_cancellation` in the local path.
        // `tokio_util::CancellationToken` is monotonic, so a
        // previously-cancelled turn would poison turn N+1 if we
        // didn't swap the slot.
        channel.reset_cancellation();

        let message = Message::text(channel.session_id(), &msg.text);
        let turn_fut = agent.turn(message, channel.as_ref());
        tokio::pin!(turn_fut);

        let _outcome = loop {
            tokio::select! {
                // Biased — the turn arm is checked first each poll.
                // If the turn has already resolved (common case on a
                // fast turn), we never even arm the scan and the
                // `scan_for_cancel` future is constructed and dropped
                // synchronously, paying no network round-trip.
                biased;

                outcome = &mut turn_fut => {
                    // Turn completed (or was cancelled by a previous
                    // scan-arm cancellation). Exit the per-turn select
                    // loop with the outcome. Any `ScanResult` the scan
                    // arm may have *also* seen on this iteration is
                    // discarded by the drop here — which is fine,
                    // because that batch either (a) hasn't been
                    // fetched yet (scan arm still awaiting) or
                    // (b) was fetched, and scripted-transport-drains-
                    // eagerly corner cases aside, the main loop's
                    // next `get_updates(offset, ...)` will re-fetch
                    // the same window on production `ReqwestTransport`.
                    break outcome;
                }

                scan = scan_for_cancel(
                    transport.as_ref(),
                    offset,
                    target_chat,
                    SCAN_FOR_CANCEL_TIMEOUT_SECS,
                ) => {
                    match scan {
                        Ok(ScanResult::NoCancel { max_update_id, queued }) => {
                            // No cancel this scan window; advance the
                            // cursor past whatever we observed and
                            // push any queued target-chat messages to
                            // the *back* of `pending` so the current
                            // turn finishes first, then those queued
                            // messages drive subsequent turns in
                            // arrival order.
                            if let Some(m) = max_update_id {
                                offset = offset.max(m + 1);
                            }
                            for q in queued {
                                pending.push_back(q);
                            }
                            // Loop back to arm another scan against
                            // the still-in-flight turn.
                        }
                        Ok(ScanResult::FoundCancel { cancel_update_id, queued }) => {
                            // Cancel! Advance the cursor past the
                            // cancel (and any same-batch queued
                            // messages). Prepend the queued messages
                            // to `pending` so they run *before* any
                            // messages the user types after the
                            // cancelled turn's finalize — preserving
                            // arrival order from the user's point of
                            // view.
                            offset = offset.max(cancel_update_id + 1);
                            for q in queued.into_iter().rev() {
                                pending.push_front(q);
                            }
                            // Fire the per-turn cancel. The planner's
                            // own `tokio::select!` against the
                            // channel's cancellation token (see
                            // `llm_planner.rs:176`) will win the next
                            // scheduling step and `turn_fut` will
                            // resolve as `TurnOutcome::Cancelled`,
                            // which the biased branch above then
                            // catches on the next loop iteration.
                            channel.cancellation_token().cancel();
                        }
                        Err(e) => {
                            // A scan-layer transport failure is not
                            // fatal — the turn is still running and
                            // should be allowed to finish (the user
                            // didn't ask for a cancel as far as we
                            // know). Back off briefly to avoid hot-
                            // spinning on a consistently broken scan
                            // and loop to re-arm.
                            eprintln!(
                                "aivyx-telegram: scan_for_cancel failed ({e}); dropping scan this round"
                            );
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            }
        };
        turns_run += 1;
    }
}

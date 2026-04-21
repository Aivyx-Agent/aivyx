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
use std::time::{SystemTime, UNIX_EPOCH};

use aivyx_capability::CapabilitySet;
use aivyx_core::{
    agent::ConcreteAgent, llm_planner::LlmPlanner, planner::ToolRegistry, Agent, AgentId,
    AuditHook, ChannelContext, LlmPlannerConfig, Message, TurnOutcome,
};
use aivyx_llm::LlmProvider;
use aivyx_storage::{KeyDomain, Storage};

use crate::LocalChannel;

/// Fixed redb key used for the single-row "current session" marker.
///
/// Phase 5 task 4 persistence scope (Q1 option 1): session metadata only.
/// A second process start reads this key under `KeyDomain::Sessions` to
/// detect "I've been here before." The value layout is 40 bytes:
///
/// ```text
///   [0..16]   session_id  (Uuid bytes — *current* process's session)
///   [16..24]  opened_at_secs   u64 big-endian, seconds since UNIX_EPOCH
///   [24..32]  last_turn_index  u64 big-endian, REPL turn counter (0 at open)
///   [32..40]  last_turn_at_secs u64 big-endian, 0 before the first turn
/// ```
///
/// Not serde: this record has exactly four fields and will never grow
/// within Phase 5 (Q4: schema bumps happen by HKDF salt rotation, not by
/// in-place migration), so a hand-rolled fixed layout avoids pulling
/// `serde_json` into the channel crate's prod deps.
const SESSION_MARKER_KEY: &[u8] = b"current";
const SESSION_MARKER_LEN: usize = 40;

/// Knobs the REPL needs to construct one session's planner + agent.
pub struct SessionConfig {
    pub model: String,
    pub system_prompt: String,
    pub max_tokens: u32,
    /// The agent's capability set. Defaults in the binary are broad
    /// (`memory.read`, `memory.write`) because the local CLI is the
    /// most-trusted channel on the box; tests may pick their own.
    pub capabilities: CapabilitySet,
    /// The tool registry for this session. Phase 4 task 4 moved this
    /// out of `run_session` (where it was hardcoded to an empty
    /// registry) so the binary can register real tools at startup
    /// while the chat-only integration test keeps passing an empty
    /// one. Shared as `Arc` because the planner factory closure
    /// clones it per-turn and `ConcreteAgent` holds its own handle.
    pub tools: Arc<ToolRegistry>,
    /// The encrypted storage handle for this session. Phase 5 task 4
    /// added the field; `run_session` writes a small session-metadata
    /// record under `KeyDomain::Sessions` at open and after each turn
    /// (see [`SESSION_MARKER_KEY`]). Shared as `Arc<dyn Storage>` to
    /// match the `AuditHook` pattern from Phase 2 — the binary owns
    /// the one-per-process `RedbStorage` handle, tests inject a
    /// throwaway `RedbStorage` against a tempdir, and both flow
    /// through the same trait object.
    pub storage: Arc<dyn Storage>,
    /// Prompt string written before each `read_line`. The binary
    /// passes `"> "`; tests usually pass `""` so captured output is
    /// easier to assert on.
    pub prompt: String,
    /// Banner line printed once at session start, before the first
    /// prompt. `None` means "no banner" — the test path uses this to
    /// keep stdout output deterministic.
    pub banner: Option<String>,
    /// Phase 11 Task 4 — role-derived tool allowlist. `None` means
    /// "allow every registered tool" (legacy Phase 6–10 behavior).
    /// `Some(set)` filters the advertised catalog at the planner
    /// layer and the dispatch gate at the agent layer — see
    /// `LlmPlannerConfig::tool_allowlist` and
    /// `ConcreteAgent::with_tool_allowlist` for the two enforcement
    /// points.
    pub tool_allowlist: Option<std::collections::BTreeSet<String>>,
    /// Phase 11 Task 4 — role-derived memory-topic prefix. `None`
    /// means "no prefix" (legacy behavior). A `Some` value is
    /// prepended by the dispatch layer to every `memory.*` tool
    /// call's `topic` input before the tool sees it. Invisible to
    /// the model by design.
    pub memory_topic_prefix: Option<String>,
    /// Phase 30 — runtime role overrides. When `Some`, the planner
    /// factory reads this on each turn construction to pick up
    /// prompt appendix and allowlist mutations set by `role.update`.
    pub role_overrides: Option<crate::role_overrides::SharedRoleOverrides>,
    /// Phase 43 — context window size in tokens for pruning. When
    /// `Some`, the planner prunes old history when estimated tokens
    /// exceed 80% of this value. `None` disables pruning.
    pub context_window_tokens: Option<usize>,
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
    // Registry comes from the caller. The binary registers the Phase 4
    // filesystem tools here; the Phase 3 chat-only regression test
    // passes an empty registry so its assertions stay stable.
    let registry = config.tools;
    let storage = config.storage;

    // Planner factory — fresh planner per turn. Captures the provider
    // Arc, the registry Arc, and a planner config by value (cloned
    // per-turn; `LlmPlannerConfig` is small).
    let provider_for_factory = Arc::clone(&provider);
    let registry_for_factory = Arc::clone(&registry);
    let mut planner_config = LlmPlannerConfig::new(config.model)
        .with_system_prompt(config.system_prompt)
        .with_max_tokens(config.max_tokens)
        .with_tool_allowlist(config.tool_allowlist.clone());
    if let Some(cw) = config.context_window_tokens {
        planner_config = planner_config.with_context_window(cw);
    }
    let role_overrides_for_factory = config.role_overrides.clone();

    let agent = ConcreteAgent::new(
        AgentId::new(),
        config.capabilities,
        registry,
        audit,
        move || {
            let mut cfg = planner_config.clone();
            if let Some(ref shared) = role_overrides_for_factory {
                if let Ok(overrides) = shared.read() {
                    if !overrides.is_empty() {
                        crate::role_overrides::apply_to_planner_config(
                            &overrides,
                            &mut cfg,
                        );
                    }
                }
            }
            Box::new(LlmPlanner::new(
                Arc::clone(&provider_for_factory),
                Arc::clone(&registry_for_factory),
                cfg,
            ))
        },
    )
    .with_tool_allowlist(config.tool_allowlist)
    .with_memory_topic_prefix(config.memory_topic_prefix);

    // ---- Session marker (Phase 5 task 4) -----------------------------
    //
    // Ask the store whether a previous process already wrote a marker
    // under `KeyDomain::Sessions` / `SESSION_MARKER_KEY`. If one exists,
    // surface a one-line "resuming" message to stderr — the point of
    // the phase is proving the round-trip works across process
    // boundaries, and this is the smallest observable that demonstrates
    // it without touching the planner's conversation state (which is
    // Phase 6 memory territory).
    //
    // Storage errors are *not* fatal: if the store rejects the read or
    // the value decodes funny, we log and keep going. The REPL is the
    // user's primary surface; a degraded persistence layer should
    // never cost them the ability to talk to the agent.
    let session_id = channel.session_id();
    let sessions = storage.domain(KeyDomain::Sessions);
    match sessions.get(SESSION_MARKER_KEY).await {
        Ok(Some(bytes)) => match decode_session_marker(&bytes) {
            Some(prior) => {
                eprintln!(
                    "aivyx: resuming — prior session {} opened {}s ago, last turn index {}",
                    prior.session_uuid_hex(),
                    now_secs().saturating_sub(prior.opened_at_secs),
                    prior.last_turn_index,
                );
            }
            None => {
                eprintln!(
                    "aivyx: session marker present but unparseable ({} bytes); starting fresh",
                    bytes.len()
                );
            }
        },
        Ok(None) => {
            // First run against this store. Silent — the banner is
            // enough UX for "new session starting."
        }
        Err(e) => {
            eprintln!("aivyx: session marker read failed ({e}); starting fresh");
        }
    }

    let opened_at_secs = now_secs();
    write_session_marker(
        &sessions,
        &SessionMarker {
            session_uuid: *session_id.0.as_bytes(),
            opened_at_secs,
            last_turn_index: 0,
            last_turn_at_secs: 0,
        },
    )
    .await;

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

        // Update the session marker with the fresh turn count and the
        // current wall-clock timestamp. Same non-fatal-on-error shape
        // as the open-time write above: a storage hiccup should not
        // take down the REPL between the user's turns.
        write_session_marker(
            &sessions,
            &SessionMarker {
                session_uuid: *session_id.0.as_bytes(),
                opened_at_secs,
                last_turn_index: turns_run as u64,
                last_turn_at_secs: now_secs(),
            },
        )
        .await;
    }
}

// ---------------------------------------------------------------------------
// Session marker — hand-rolled fixed-layout encode/decode for the
// 40-byte metadata record at `KeyDomain::Sessions` / `SESSION_MARKER_KEY`.
//
// Kept private to this module because the encoding is an internal
// detail of how `run_session` uses storage, not part of the public API
// of the channel crate. A future phase (probably Phase 6 memory) will
// replace this with a richer schema keyed on `SessionId` bytes; at that
// point we'll bump the HKDF salt and start the "aivyx-v2-storage" era
// rather than try to migrate in place.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct SessionMarker {
    session_uuid: [u8; 16],
    opened_at_secs: u64,
    last_turn_index: u64,
    last_turn_at_secs: u64,
}

impl SessionMarker {
    fn encode(&self) -> [u8; SESSION_MARKER_LEN] {
        let mut out = [0u8; SESSION_MARKER_LEN];
        out[0..16].copy_from_slice(&self.session_uuid);
        out[16..24].copy_from_slice(&self.opened_at_secs.to_be_bytes());
        out[24..32].copy_from_slice(&self.last_turn_index.to_be_bytes());
        out[32..40].copy_from_slice(&self.last_turn_at_secs.to_be_bytes());
        out
    }

    fn session_uuid_hex(&self) -> String {
        let mut s = String::with_capacity(32);
        for byte in self.session_uuid.iter() {
            s.push_str(&format!("{byte:02x}"));
        }
        s
    }
}

fn decode_session_marker(bytes: &[u8]) -> Option<SessionMarker> {
    if bytes.len() != SESSION_MARKER_LEN {
        return None;
    }
    let mut session_uuid = [0u8; 16];
    session_uuid.copy_from_slice(&bytes[0..16]);
    let opened_at_secs = u64::from_be_bytes(bytes[16..24].try_into().ok()?);
    let last_turn_index = u64::from_be_bytes(bytes[24..32].try_into().ok()?);
    let last_turn_at_secs = u64::from_be_bytes(bytes[32..40].try_into().ok()?);
    Some(SessionMarker {
        session_uuid,
        opened_at_secs,
        last_turn_index,
        last_turn_at_secs,
    })
}

async fn write_session_marker(sessions: &aivyx_storage::DomainHandle, marker: &SessionMarker) {
    let encoded = marker.encode();
    if let Err(e) = sessions.put(SESSION_MARKER_KEY, &encoded).await {
        eprintln!("aivyx: session marker write failed ({e}); continuing");
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_marker() -> SessionMarker {
        SessionMarker {
            session_uuid: [
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54,
                0x32, 0x10,
            ],
            opened_at_secs: 0x1122_3344_5566_7788,
            last_turn_index: 42,
            last_turn_at_secs: 0x0011_2233_4455_6677,
        }
    }

    #[test]
    fn session_marker_round_trips_through_encode_decode() {
        let marker = sample_marker();
        let bytes = marker.encode();
        assert_eq!(bytes.len(), SESSION_MARKER_LEN);

        let decoded = decode_session_marker(&bytes).expect("well-formed bytes must decode");
        assert_eq!(decoded.session_uuid, marker.session_uuid);
        assert_eq!(decoded.opened_at_secs, marker.opened_at_secs);
        assert_eq!(decoded.last_turn_index, marker.last_turn_index);
        assert_eq!(decoded.last_turn_at_secs, marker.last_turn_at_secs);
    }

    #[test]
    fn session_marker_encode_uses_big_endian_layout() {
        // Pin the exact byte layout: a future refactor that flips
        // endian-ness would silently corrupt existing stores on disk
        // if this test didn't anchor the layout explicitly.
        let marker = sample_marker();
        let bytes = marker.encode();

        assert_eq!(&bytes[0..16], &marker.session_uuid);
        assert_eq!(&bytes[16..24], &marker.opened_at_secs.to_be_bytes());
        assert_eq!(&bytes[24..32], &marker.last_turn_index.to_be_bytes());
        assert_eq!(&bytes[32..40], &marker.last_turn_at_secs.to_be_bytes());
    }

    #[test]
    fn decode_session_marker_rejects_wrong_length() {
        assert!(decode_session_marker(&[]).is_none());
        assert!(decode_session_marker(&[0u8; SESSION_MARKER_LEN - 1]).is_none());
        assert!(decode_session_marker(&[0u8; SESSION_MARKER_LEN + 1]).is_none());
    }

    #[test]
    fn session_uuid_hex_is_lowercase_zero_padded_32_chars() {
        let marker = sample_marker();
        let hex = marker.session_uuid_hex();
        assert_eq!(hex.len(), 32);
        assert_eq!(hex, "0123456789abcdeffedcba9876543210");
    }
}

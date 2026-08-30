# Turn-outcome correction across frontends — design

**Status:** Approved, ready for planning.

## Motivation

The final whole-branch review of `agent-turn-quality` (POLISH_WAVES.md
sub-project 4) found that the turn loop's post-processing — Task 3's
final-message floor, Candor's existing claim-check, and Task 5's new
identifier-fidelity check — never reaches most frontends. `LlmPlanner::
one_step` (`crates/aivyx-core/src/llm_planner.rs`) relays every model
`TextChunk` live, as it streams from the provider, **before** the step
even finishes — let alone before the turn loop's `LoopOutcome::Completed`
arm runs its post-processing on the fully-assembled `final_message`. By
the time a floor substitution or an honesty annotation exists, any bad
text it was meant to catch has often already streamed to the operator in
real time.

Grounded directly against the real current code (not assumed): every one
of the 6 consuming surfaces (Studio, TUI, headless, Telegram, Discord,
Slack) receives the turn's authoritative `outcome: String` (`"completed:
{final_message}"` for a normal completion — with every annotation baked
in — or a fixed reason string for every other `TurnOutcome` variant, via
`format_outcome` in `daemon_server.rs`), but 5 of the 6 discard it after
already building their displayed text from raw `StreamEventPayload`
events:

- **Studio** (`aivyx-web/src/main.rs`) — `DaemonEnvelope::TurnComplete { .. }`
  ignores `outcome` entirely (already partially patched by the prior
  branch's Fix 3, which handles only the empty-stream case — this design
  generalizes and replaces that patch).
- **TUI** (`aivyx-tui/src/model.rs`) — `Msg::TurnFinished { events, .. }`
  discards its own `outcome: String` field.
- **Telegram / Discord / Slack** (`*_daemon_frontend.rs`) — each does
  `let (events, _outcome) = session.submit_input(...).await?;`, an
  explicit, named discard.
- **headless** (`aivyx-cli/.../headless.rs`) — **already correct**: it
  `eprintln!`s the full `outcome` string unconditionally, regardless of
  what streamed to stdout. No change needed here; confirmed by direct
  reading, not assumed.

## Scope

**In:** Studio, TUI, Telegram, Discord, Slack — wire `outcome` through and
surface it when it diverges from what already displayed.
**Out:** headless (already correct). Any change to `LlmPlanner::one_step`'s
live-streaming mechanism itself — a genuinely two-choice fork was
presented (buffer the final step vs. correct after the fact) and buffering
was explicitly declined: it trades away true token-by-token live typing
for the last chunk of every turn, for a much larger, riskier change to
code shared by every channel. This design corrects after the fact instead.

## Architecture

### Shared core: two pure functions in `aivyx-ipc/src/protocol.rs`

This file is already the wasm-clean shared home for turn-rendering logic
(`StreamEventPayload::render_for_cli`), used by both the daemon and the
Dioxus web client — the natural place for logic every consuming surface
needs, including Studio.

```rust
/// Concatenate just the `Text` chunks from a turn's events, in order —
/// the text a surface has displayed (or would display) as the
/// assistant's answer. Ignores Status/ToolCall/ApprovalGate events.
pub fn concat_text_events(events: &[StreamEventPayload]) -> String {
    let mut out = String::new();
    for event in events {
        if let StreamEventPayload::Text { text } = event {
            out.push_str(text);
        }
    }
    out
}

/// Compare what a surface already displayed/reconstructed for a turn
/// (`displayed`, from `concat_text_events` or an equivalent
/// live-accumulated buffer) against the turn's own authoritative
/// `outcome` string (as sent on `TurnComplete`/`Msg::TurnFinished` —
/// `"completed: {final_message}"` for a normal completion, a fixed
/// reason string for every other `TurnOutcome` variant). Returns
/// `Some(line)` to show when they diverge in a way the operator should
/// see; `None` when nothing needs correcting.
pub fn turn_outcome_correction(displayed: &str, outcome: &str) -> Option<String> {
    let displayed = displayed.trim();
    match outcome.strip_prefix("completed: ") {
        Some(final_message) => {
            let final_message = final_message.trim();
            if displayed == final_message {
                None
            } else if displayed.is_empty() {
                if final_message.is_empty() {
                    Some("(no reply)".to_string())
                } else {
                    Some(final_message.to_string())
                }
            } else {
                Some(format!("⚠ corrected: {final_message}"))
            }
        }
        None => Some(outcome.to_string()),
    }
}
```

Both are pure, `#[cfg(test)]`-testable in isolation, no I/O, no async.

### Per-surface wiring

**Studio** (`crates/aivyx-web/src/main.rs`) — replaces the prior branch's
ad hoc `DaemonEnvelope::TurnComplete` handling (which only covered the
empty-stream case) with the shared helper, net *simplifying* the existing
code:

```rust
DaemonEnvelope::TurnComplete { outcome, .. } => {
    let text = streaming();
    if !text.is_empty() {
        transcript.write().push(ChatLine::assistant(text.clone()));
    }
    if let Some(note) = aivyx_ipc::protocol::turn_outcome_correction(&text, &outcome) {
        transcript.write().push(ChatLine::system(note));
    }
    streaming.set(String::new());
}
```

**TUI** (`crates/aivyx-tui/src/model.rs`) — `Msg::TurnFinished { events,
outcome }` already carries both fields; compute `displayed` from `events`
before the existing Text-coalescing loop consumes it by value, then push
a `ChatLine::new(LineKind::System, note)` when the helper returns `Some`.

**Telegram / Discord / Slack** (`crates/aivyx-channel/src/
{telegram,discord,slack}_daemon_frontend.rs`) — stop discarding `outcome`;
after building `buf` via the existing `render_events_for_*` call, compute
`concat_text_events(&events)` and append the correction line to `buf`
(with a separating newline if `buf` is non-empty) before sending. These
three surfaces batch-send once at `TurnComplete` rather than streaming
live to the platform, so there is no "already displayed" constraint here
— the correction always lands in the same message as everything else.

### Why "correct after," not a retroactive edit

Telegram/Discord/Slack technically *could* avoid ever sending the bad text
in the first place, since nothing has gone out yet when the correction is
computed. Deliberately not doing that: using one uniform algorithm (always
append, never conditionally rewrite) across all 5 surfaces keeps the
behavior predictable — an operator who uses both Studio and Telegram sees
the same shape of correction in both places — and keeps `render_events_for_*`
(and its existing test coverage) untouched.

## Testing

- `concat_text_events` and `turn_outcome_correction`: unit tests in
  `aivyx-ipc/src/protocol.rs` covering every branch — matching completed
  text (no correction), empty-both (`"(no reply)"`), empty-displayed
  with real final text (plain text, no "corrected" framing since nothing
  was shown to correct), displayed-and-differs (`"⚠ corrected: ..."`),
  and every non-completed outcome variant's fixed reason string (always
  shown regardless of `displayed`).
- Studio: `cargo check -p aivyx-web` / `cargo clippy -p aivyx-web
  --all-targets -- -D warnings` (both confirmed to work natively, no
  wasm32 needed), a rebuilt+committed `dist/` bundle via the rustup
  wasm32 toolchain (`cargo install dioxus-cli --version 0.6.3 --locked`
  if the installed `dx` has drifted — this workspace has already hit
  and fixed this exact drift once; the fix is on record).
- TUI: real behavioral tests exercising `Msg::TurnFinished` with a
  streamed-text/outcome pair that matches (no extra line) and one that
  diverges (extra `LineKind::System` line present), mirroring the
  existing `TurnFinished` test patterns in `model.rs`.
- Telegram/Discord/Slack: real behavioral tests on the render+append
  path (mirroring the existing `render_events_for_*` test patterns —
  `slack_daemon_frontend.rs`, `discord_daemon_frontend.rs`,
  `telegram_daemon_frontend.rs` each already have a test module).
- Full sweep before merge: `cargo clippy --workspace --exclude
  aivyx-desktop --all-targets -- -D warnings` and `cargo test
  --workspace --exclude aivyx-desktop` (aivyx-desktop excluded per this
  sandbox's own established, documented environment gap — no system
  webkit2gtk libs), zero warnings/failures.

## Out of scope

- `LlmPlanner::one_step`'s live-streaming mechanism — explicitly declined
  in favor of this design during brainstorming.
- headless (`aivyx-cli/.../headless.rs`) — already correct, confirmed by
  direct reading of its unconditional `eprintln!("aivyx --headless:
  {outcome}")`.
- Any change to `render_events_for_telegram/discord/slack`'s own
  rendering of tool-call/status lines — only the trailing text portion
  is touched, via a separate append step.
- Deduplicating the case where a partial answer streamed before a
  timeout/loop-stop/escalation — the non-completed branch always shows
  the outcome's reason text in addition to whatever partial text already
  displayed, which can look slightly redundant but is strictly more
  informative than the status quo (nothing explaining why the turn ended
  the way it did).

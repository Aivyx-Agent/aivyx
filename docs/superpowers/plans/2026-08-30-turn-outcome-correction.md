# Turn-Outcome Correction Across Frontends Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every frontend (Studio, TUI, Telegram, Discord, Slack) surface the turn loop's authoritative `outcome` string when it diverges from the raw `StreamEventPayload::Text` chunks the surface already displayed — closing the gap where floor/Candor/identifier-fidelity annotations never reach the operator on surfaces that only render streamed events.

**Architecture:** Two new pure functions in `aivyx-ipc/src/protocol.rs` (`concat_text_events`, `turn_outcome_correction`) are the shared core; each of the 5 in-scope surfaces gets a small, surface-specific wiring change that calls them. `headless` needs no change (already correct — confirmed by direct reading).

**Tech Stack:** Rust workspace (`cargo test --workspace --exclude aivyx-desktop`, `cargo clippy --workspace --exclude aivyx-desktop --all-targets -- -D warnings`), Dioxus (Studio), ratatui (TUI, tested via its pure `update()` function).

## Global Constraints

- `aivyx-desktop` is excluded from all workspace-wide commands in this plan — it needs system `webkit2gtk-4.1`/`javascriptcore` libraries not installed in this sandbox; this is this workspace's own established, documented practice, not a shortcut invented here.
- `LlmPlanner::one_step`'s live-streaming mechanism (`crates/aivyx-core/src/llm_planner.rs`) is explicitly out of scope — do not touch it. This plan corrects after the fact; it does not buffer or delay streaming.
- `headless` (`crates/aivyx-cli/src/bin/aivyx_modules/headless.rs`) is explicitly out of scope — confirmed already correct (it `eprintln!`s the full `outcome` string unconditionally). Do not modify it.
- Every new function is pure (no I/O, no async) and independently unit-tested.
- The Studio `dist/` bundle is a tracked (not gitignored) directory; any `aivyx-web` source change in this plan ships with a rebuilt, committed bundle in the same commit, per this workspace's established convention.

---

### Task 1: Shared core — `concat_text_events` and `turn_outcome_correction`

**Files:**
- Modify: `crates/aivyx-ipc/src/protocol.rs` (add two functions after `impl StreamEventPayload`'s closing brace at line 2488, plus tests in the existing `#[cfg(test)] mod tests` block near the `render_for_cli` tests, line ~4686)

**Interfaces:**
- Produces: `pub fn concat_text_events(events: &[StreamEventPayload]) -> String`, `pub fn turn_outcome_correction(displayed: &str, outcome: &str) -> Option<String>` — both used by every later task in this plan.

- [ ] **Step 1: Write the failing tests**

Add to `crates/aivyx-ipc/src/protocol.rs`'s existing `#[cfg(test)] mod tests` block, immediately after the `render_for_cli_approval_gate_without_scope` test (around line 4746, right before the `// ---- Phase 45 — IpcAttachment ----` section comment):

```rust
    // ---- concat_text_events / turn_outcome_correction (turn-outcome
    // correction follow-up to POLISH_WAVES.md sub-project 4) ----

    #[test]
    fn concat_text_events_joins_only_text_chunks() {
        let events = vec![
            StreamEventPayload::Status {
                status: "thinking".into(),
            },
            StreamEventPayload::Text { text: "Hi".into() },
            StreamEventPayload::ToolCallStarted {
                tool_id: "id".into(),
                tool_name: "web_search".into(),
                input: serde_json::json!({}),
            },
            StreamEventPayload::Text {
                text: " there".into(),
            },
        ];
        assert_eq!(concat_text_events(&events), "Hi there");
    }

    #[test]
    fn concat_text_events_empty_for_no_text_events() {
        let events = vec![StreamEventPayload::Status {
            status: "thinking".into(),
        }];
        assert_eq!(concat_text_events(&events), "");
    }

    #[test]
    fn turn_outcome_correction_none_when_text_matches_final_message() {
        assert_eq!(
            turn_outcome_correction("Your home airport is Jandakot.", "completed: Your home airport is Jandakot."),
            None
        );
    }

    #[test]
    fn turn_outcome_correction_no_reply_when_both_empty() {
        assert_eq!(
            turn_outcome_correction("", "completed: "),
            Some("(no reply)".to_string())
        );
    }

    #[test]
    fn turn_outcome_correction_shows_final_message_when_nothing_displayed() {
        assert_eq!(
            turn_outcome_correction("", "completed: I wasn't able to produce a usable reply this turn — please try again."),
            Some("I wasn't able to produce a usable reply this turn — please try again.".to_string())
        );
    }

    #[test]
    fn turn_outcome_correction_flags_a_correction_when_displayed_and_final_differ() {
        let leaked = "{\"path\": \"airports.csv\"}";
        let corrected = "completed: I wasn't able to produce a usable reply this turn — please try again.";
        assert_eq!(
            turn_outcome_correction(leaked, corrected),
            Some("⚠ corrected: I wasn't able to produce a usable reply this turn — please try again.".to_string())
        );
    }

    #[test]
    fn turn_outcome_correction_always_surfaces_non_completed_outcomes() {
        assert_eq!(
            turn_outcome_correction("partial answer", "timed out"),
            Some("timed out".to_string())
        );
        assert_eq!(
            turn_outcome_correction("", "stopped: 3 repeated identical tool calls"),
            Some("stopped: 3 repeated identical tool calls".to_string())
        );
        assert_eq!(
            turn_outcome_correction("some text", "escalated: shell.exec needs approval"),
            Some("escalated: shell.exec needs approval".to_string())
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p aivyx-ipc concat_text_events -- --exact` and `cargo test -p aivyx-ipc turn_outcome_correction`
Expected: FAIL with "cannot find function" for both.

- [ ] **Step 3: Implement both functions**

Add to `crates/aivyx-ipc/src/protocol.rs`, immediately after `impl StreamEventPayload { ... }`'s closing `}` (line 2488):

```rust

/// Concatenate just the `Text` chunks from a turn's events, in order —
/// the text a surface has displayed (or would display) as the
/// assistant's answer. Ignores `Status`/`ToolCall*`/`ApprovalGate`
/// events. Pure.
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
/// (`displayed`, from [`concat_text_events`] or an equivalent
/// live-accumulated buffer) against the turn's own authoritative
/// `outcome` string (as sent on `TurnComplete`/`Msg::TurnFinished` —
/// `"completed: {final_message}"` for a normal completion, a fixed
/// reason string for every other `TurnOutcome` variant — see
/// `format_outcome` in `aivyx-channel`'s `daemon_server.rs`). Returns
/// `Some(line)` to show when they diverge in a way the operator should
/// see; `None` when nothing needs correcting. The turn loop's own
/// post-processing (a final-message floor, Candor's claim-check,
/// an identifier-fidelity check) only ever touches `outcome`'s
/// `final_message` — never the raw streamed text — so this is the seam
/// a surface uses to catch up. Pure.
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

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p aivyx-ipc concat_text_events turn_outcome_correction`
Expected: all 8 new tests PASS.

- [ ] **Step 5: Full-crate check and commit**

Run: `cargo clippy -p aivyx-ipc --all-targets -- -D warnings && cargo test -p aivyx-ipc`
Expected: clean, all tests pass.

```bash
git add crates/aivyx-ipc/src/protocol.rs
git commit -m "feat(ipc): add concat_text_events + turn_outcome_correction helpers

Shared, wasm-clean core for the turn-outcome-correction follow-up to
POLISH_WAVES.md sub-project 4's final review. Every consuming surface
(Studio, TUI, Telegram, Discord, Slack) receives the turn's own
authoritative outcome string but most discard it after already
displaying raw streamed Text events, so any post-processing the turn
loop does (a reply floor, Candor's claim-check, the identifier-
fidelity check) never reaches the operator. These two pure functions
are the shared comparison logic later tasks wire into each surface.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: Studio wiring

**Files:**
- Modify: `crates/aivyx-web/src/main.rs:19-27` (import), `:8115-8143` (`DaemonEnvelope::TurnComplete` handler)

**Interfaces:**
- Consumes: `aivyx_ipc::protocol::turn_outcome_correction` (Task 1).

- [ ] **Step 1: Add the import**

In `crates/aivyx-web/src/main.rs`, find the `use aivyx_ipc::protocol::{ ... };` block (lines 19-27):

```rust
use aivyx_ipc::protocol::{
    AuditEntrySummary, DaemonEnvelope, DocEntry, DocFile, EffectivePersonaSummary, FrontendMessage,
    GalleryImage, McpServerStatusView, MemoryEntrySummary, MemoryGraphNode, NotificationHistoryEntry,
    NotifyTargetView, PersonaDeltaSummary,
    PersonaProposalResolution,
    PersonaProposalSummary, PersonaSeedWire, ProfileDraftWire, ProfileSummary, QueryPayload,
    QueryResponsePayload, ScheduleView, SeedSkillWire, SessionSummary, SettingsSnapshot,
    SkillAuthorOp, SkillView, StreamEventPayload, ToolCatalogEntry, VoiceSettingsSnapshot,
};
```

Add `turn_outcome_correction` to the list (alphabetical placement not required by this codebase's existing list — append at the end of the last line before the closing brace):

```rust
use aivyx_ipc::protocol::{
    AuditEntrySummary, DaemonEnvelope, DocEntry, DocFile, EffectivePersonaSummary, FrontendMessage,
    GalleryImage, McpServerStatusView, MemoryEntrySummary, MemoryGraphNode, NotificationHistoryEntry,
    NotifyTargetView, PersonaDeltaSummary,
    PersonaProposalResolution,
    PersonaProposalSummary, PersonaSeedWire, ProfileDraftWire, ProfileSummary, QueryPayload,
    QueryResponsePayload, ScheduleView, SeedSkillWire, SessionSummary, SettingsSnapshot,
    SkillAuthorOp, SkillView, StreamEventPayload, ToolCatalogEntry, VoiceSettingsSnapshot,
    turn_outcome_correction,
};
```

- [ ] **Step 2: Replace the `TurnComplete` handler**

Find (lines 8115-8143):

```rust
                DaemonEnvelope::TurnComplete { outcome, .. } => {
                    let text = streaming();
                    if !text.is_empty() {
                        transcript.write().push(ChatLine::assistant(text));
                    } else if let Some(rest) = outcome.strip_prefix("completed: ") {
                        if rest.trim().is_empty() {
                            // POLISH_WAVES.md sub-project 4, item C —
                            // render something instead of a silent void
                            // when a turn completes with no streamed
                            // text AND no real final_message either.
                            transcript.write().push(ChatLine::system("(no reply)".to_string()));
                        } else {
                            // Streamed Text events didn't carry the
                            // final message for some reason, but the
                            // turn's own outcome has real content
                            // (e.g. the turn-loop's own reply floor) —
                            // show it rather than a flat placeholder.
                            transcript.write().push(ChatLine::assistant(rest.to_string()));
                        }
                    } else {
                        // Final-review fix (POLISH_WAVES.md sub-project
                        // 4) — a non-completed outcome (timed out,
                        // cancelled, looping, escalated, failed) was
                        // being overwritten with a flat "(no reply)",
                        // discarding the real reason. Show it instead.
                        transcript.write().push(ChatLine::system(outcome));
                    }
                    streaming.set(String::new());
                }
```

Replace with (this consolidates the prior branch's ad hoc logic into the shared, tested helper — net simpler, same behavior plus the new "streamed-but-differs" case the prior code didn't cover):

```rust
                DaemonEnvelope::TurnComplete { outcome, .. } => {
                    let text = streaming();
                    if !text.is_empty() {
                        transcript.write().push(ChatLine::assistant(text.clone()));
                    }
                    if let Some(note) = turn_outcome_correction(&text, &outcome) {
                        transcript.write().push(ChatLine::system(note));
                    }
                    streaming.set(String::new());
                }
```

- [ ] **Step 3: Compile-check and clippy (native, no wasm32 target needed)**

Run: `cargo check -p aivyx-web && cargo clippy -p aivyx-web --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 4: Rebuild the wasm bundle**

```bash
export PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH"
cd crates/aivyx-web && dx bundle --release --platform web
```

If `dx` errors with a version-incompatibility message, run `cargo install dioxus-cli --version 0.6.3 --locked --force` first (the `--locked` flag is load-bearing — this exact drift has already happened once in this workspace and is on record, not a new problem to diagnose).

Copy `target/dx/aivyx-web/release/web/public/` over `crates/aivyx-web/dist/` (replace the directory contents), delete any `*.br` files the build produces (`find crates/aivyx-web/dist -name '*.br' -delete` — this repo's committed `dist/` has never included brotli sidecars), and confirm with `git ls-tree -r main -- crates/aivyx-web/dist | grep -c '\.br$'` (must print `0`) before committing.

Run: `cd /home/julian/Projects/Rust/aivyx && git status --porcelain crates/aivyx-web/dist/`
Expected: shows the rebuilt bundle files as modified.

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-web/src/main.rs crates/aivyx-web/dist/
git commit -m "feat(web): wire turn_outcome_correction into Studio's TurnComplete

Turn-outcome-correction follow-up to POLISH_WAVES.md sub-project 4's
final review. Replaces the prior branch's ad hoc empty-stream handling
with the shared, tested helper — also now catches the case where
streamed text is non-empty but differs from the turn's own outcome
(a Candor/identifier-fidelity annotation, or a floored final message,
appended after streaming already started).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 3: TUI wiring

**Files:**
- Modify: `crates/aivyx-tui/src/model.rs:17` (import), `:549-584` (`Msg::TurnFinished` handler)
- Test: `crates/aivyx-tui/src/model.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `aivyx_channel::daemon_ipc::{concat_text_events, turn_outcome_correction}` (re-exported from Task 1's `aivyx-ipc` additions via `aivyx-channel`'s existing `pub use aivyx_ipc::protocol::*;` in `daemon_ipc.rs`).

- [ ] **Step 1: Add the import**

In `crates/aivyx-tui/src/model.rs`, find:

```rust
use aivyx_channel::daemon_ipc::{AuditEntrySummary, StreamEventPayload};
```

Replace with:

```rust
use aivyx_channel::daemon_ipc::{
    concat_text_events, turn_outcome_correction, AuditEntrySummary, StreamEventPayload,
};
```

- [ ] **Step 2: Write the failing tests**

Add to `crates/aivyx-tui/src/model.rs`'s existing `#[cfg(test)] mod tests` block, immediately after the existing `turn_finished_coalesces_token_level_text_events` test (search for it to find the exact spot — add right after its closing `}`):

```rust
    #[test]
    fn turn_finished_appends_correction_when_outcome_differs() {
        let mut s = typed(AppState::new(), "q");
        s = update(s, Msg::Submit);
        s = update(
            s,
            Msg::TurnFinished {
                events: vec![StreamEventPayload::Text {
                    text: "{\"path\": \"airports.csv\"}".into(),
                }],
                outcome: "completed: I wasn't able to produce a usable reply this turn — please try again.".into(),
            },
        );
        let last = s.history.last().unwrap();
        assert_eq!(last.kind, LineKind::System);
        assert!(
            last.text.contains("corrected"),
            "expected a correction line, got: {}",
            last.text
        );
    }

    #[test]
    fn turn_finished_no_correction_line_when_outcome_matches() {
        let mut s = typed(AppState::new(), "q");
        s = update(s, Msg::Submit);
        let len_before = s.history.len();
        s = update(
            s,
            Msg::TurnFinished {
                events: vec![StreamEventPayload::Text {
                    text: "an answer".into(),
                }],
                outcome: "completed: an answer".into(),
            },
        );
        assert_eq!(s.history.last().unwrap().kind, LineKind::Agent);
        assert_eq!(
            s.history.len(),
            len_before + 1,
            "outcome matches displayed text — no extra correction line should be appended"
        );
    }

    #[test]
    fn turn_finished_surfaces_non_completed_outcome() {
        let mut s = typed(AppState::new(), "q");
        s = update(s, Msg::Submit);
        s = update(
            s,
            Msg::TurnFinished {
                events: vec![],
                outcome: "stopped: 3 repeated identical tool calls".into(),
            },
        );
        let last = s.history.last().unwrap();
        assert_eq!(last.kind, LineKind::System);
        assert_eq!(last.text, "stopped: 3 repeated identical tool calls");
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p aivyx-tui turn_finished_appends_correction_when_outcome_differs -- --exact`
Expected: FAIL (no correction line appended — the mechanism doesn't exist yet).

- [ ] **Step 4: Wire the correction into `Msg::TurnFinished`**

Find (lines 549-584):

```rust
        Msg::TurnFinished { events, .. } => {
            // Coalesce consecutive Text events first: the daemon
            // streams token-level chunks ("Hi", " there", "!"), and
            // rendering each as its own ChatLine put one word per
            // line (Vitrine §12).
            let mut merged: Vec<StreamEventPayload> = Vec::with_capacity(events.len());
            for event in events {
                if let StreamEventPayload::Text { text } = &event {
                    if let Some(StreamEventPayload::Text { text: prev }) = merged.last_mut() {
                        prev.push_str(text);
                        continue;
                    }
                }
                merged.push(event);
            }
            for event in &merged {
                for line in lines_from_event(event) {
                    state.push_line(line);
                }
                if let StreamEventPayload::ApprovalGate {
                    mission_id,
                    gate_id,
                    reason,
                    scope,
                } = event
                {
                    state.gate = Some(PendingGate {
                        mission_id: mission_id.clone(),
                        gate_id: gate_id.clone(),
                        reason: reason.clone(),
                        scope: scope.clone(),
                    });
                }
            }
            state.status.working = false;
        }
```

Replace with:

```rust
        Msg::TurnFinished { events, outcome } => {
            // Turn-outcome-correction follow-up (POLISH_WAVES.md
            // sub-project 4) — compute what these events would display
            // BEFORE the coalescing loop below consumes `events` by
            // value, so it can be compared against the turn's own
            // authoritative outcome.
            let displayed = concat_text_events(&events);

            // Coalesce consecutive Text events first: the daemon
            // streams token-level chunks ("Hi", " there", "!"), and
            // rendering each as its own ChatLine put one word per
            // line (Vitrine §12).
            let mut merged: Vec<StreamEventPayload> = Vec::with_capacity(events.len());
            for event in events {
                if let StreamEventPayload::Text { text } = &event {
                    if let Some(StreamEventPayload::Text { text: prev }) = merged.last_mut() {
                        prev.push_str(text);
                        continue;
                    }
                }
                merged.push(event);
            }
            for event in &merged {
                for line in lines_from_event(event) {
                    state.push_line(line);
                }
                if let StreamEventPayload::ApprovalGate {
                    mission_id,
                    gate_id,
                    reason,
                    scope,
                } = event
                {
                    state.gate = Some(PendingGate {
                        mission_id: mission_id.clone(),
                        gate_id: gate_id.clone(),
                        reason: reason.clone(),
                        scope: scope.clone(),
                    });
                }
            }
            if let Some(note) = turn_outcome_correction(&displayed, &outcome) {
                state.push_line(ChatLine::new(LineKind::System, note));
            }
            state.status.working = false;
        }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p aivyx-tui turn_finished`
Expected: all `turn_finished_*` tests PASS, including the 3 new ones and the pre-existing `turn_finished_clears_working_and_appends` / `turn_finished_coalesces_token_level_text_events` / `new_content_repins_to_bottom` (their `events` text already matches their `outcome` string exactly, so `turn_outcome_correction` returns `None` for them and no extra line is appended — they remain valid unmodified).

- [ ] **Step 6: Full-crate check and commit**

Run: `cargo clippy -p aivyx-tui --all-targets -- -D warnings && cargo test -p aivyx-tui`
Expected: clean, all tests pass.

```bash
git add crates/aivyx-tui/src/model.rs
git commit -m "feat(tui): wire turn_outcome_correction into Msg::TurnFinished

Turn-outcome-correction follow-up to POLISH_WAVES.md sub-project 4's
final review. Msg::TurnFinished already carried its own outcome field;
it was discarded. Now appends a System-kind correction line when the
streamed text diverges from the turn's authoritative outcome.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 4: Telegram, Discord, and Slack wiring

**Files:**
- Modify: `crates/aivyx-channel/src/telegram_daemon_frontend.rs:537-552`
- Modify: `crates/aivyx-channel/src/discord_daemon_frontend.rs:514-515`
- Modify: `crates/aivyx-channel/src/slack_daemon_frontend.rs:540-541`
- Test: each file's own existing `#[cfg(test)] mod tests` block

**Interfaces:**
- Consumes: `crate::daemon_ipc::{concat_text_events, turn_outcome_correction}` (Task 1, re-exported via `aivyx-channel`'s `daemon_ipc` module — already the exact re-export path these three files use for `StreamEventPayload`).
- Produces: `fn build_slack_reply(events: &[StreamEventPayload], outcome: &str) -> String`, `fn build_discord_reply(...)`, `fn build_telegram_reply(...)` — one per file, same shape, not shared across files (matches this codebase's own existing convention of near-identical, deliberately independent per-adapter functions — see `render_events_for_slack`'s own doc comment: "byte-identical to render_events_for_discord... the three SemiTrusted adapters all produce the same in-message UX deliberately").

**Important — avoiding a double "(no reply)":** `render_events_for_slack`/`_discord`/`_telegram` each already fall back to `"(no reply)"` internally when the rendered buffer is empty (`if buf.trim().is_empty() { "(no reply)".to_string() } else { buf }`). `turn_outcome_correction` can ALSO return `Some("(no reply)".to_string())` for the same empty-turn case. Appending unconditionally would print `"(no reply)\n(no reply)"`. Each new `build_*_reply` function below guards against this one specific double-up while still appending every other kind of correction normally.

- [ ] **Step 1: Write the failing tests for Slack**

Add to `crates/aivyx-channel/src/slack_daemon_frontend.rs`'s existing `#[cfg(test)] mod tests` block, immediately after the test that asserts `render_events_for_slack(&[]) == "(no reply)"` (search for that assertion to find the spot):

```rust
    #[test]
    fn build_slack_reply_appends_correction_when_outcome_differs() {
        let events = vec![StreamEventPayload::Text {
            text: "{\"path\": \"airports.csv\"}".into(),
        }];
        let out = build_slack_reply(
            &events,
            "completed: I wasn't able to produce a usable reply this turn — please try again.",
        );
        assert!(out.contains("{\"path\": \"airports.csv\"}"), "{out}");
        assert!(out.contains("corrected"), "{out}");
    }

    #[test]
    fn build_slack_reply_no_correction_when_outcome_matches() {
        let events = vec![StreamEventPayload::Text {
            text: "an answer".into(),
        }];
        let out = build_slack_reply(&events, "completed: an answer");
        assert_eq!(out, "an answer");
    }

    #[test]
    fn build_slack_reply_does_not_double_up_no_reply() {
        let out = build_slack_reply(&[], "completed: ");
        assert_eq!(out, "(no reply)", "must not print (no reply) twice: {out}");
    }

    #[test]
    fn build_slack_reply_surfaces_non_completed_outcome() {
        let out = build_slack_reply(&[], "timed out");
        assert!(out.contains("timed out"), "{out}");
    }
```

- [ ] **Step 2: Run the Slack test to verify it fails**

Run: `cargo test -p aivyx-channel build_slack_reply_appends_correction_when_outcome_differs -- --exact`
Expected: FAIL with "cannot find function `build_slack_reply`".

- [ ] **Step 3: Implement `build_slack_reply` and wire it in**

In `crates/aivyx-channel/src/slack_daemon_frontend.rs`, add this function immediately after `render_events_for_slack`'s closing `}` (the function ending in `if buf.trim().is_empty() { "(no reply)".to_string() } else { buf } }`):

```rust

/// Build the outbound Slack reply text: the rendered event journal
/// (tool-call/status lines + streamed text), plus a correction line
/// when the turn's own outcome diverges from what the events alone
/// would show (a reply floor, a Candor/identifier-fidelity annotation,
/// or a non-completed outcome's reason). Skips the correction only
/// when it would exactly duplicate `render_events_for_slack`'s own
/// empty-events "(no reply)" fallback.
fn build_slack_reply(events: &[StreamEventPayload], outcome: &str) -> String {
    let displayed = crate::daemon_ipc::concat_text_events(events);
    let mut buf = render_events_for_slack(events);
    if let Some(note) = crate::daemon_ipc::turn_outcome_correction(&displayed, outcome) {
        if !(note == "(no reply)" && buf.trim() == "(no reply)") {
            if !buf.is_empty() && !buf.ends_with('\n') {
                buf.push('\n');
            }
            buf.push_str(&note);
        }
    }
    buf
}
```

Find the call site (search for `let (events, _outcome) = session.submit_input(msg.text).await?;` in this file):

```rust
        let (events, _outcome) = session.submit_input(msg.text).await?;
        let buf = render_events_for_slack(&events);
```

Replace with:

```rust
        let (events, outcome) = session.submit_input(msg.text).await?;
        let buf = build_slack_reply(&events, &outcome);
```

- [ ] **Step 4: Run the Slack tests to verify they pass**

Run: `cargo test -p aivyx-channel build_slack_reply`
Expected: all 4 new tests PASS.

- [ ] **Step 5: Repeat for Discord**

Add the same 4 tests to `crates/aivyx-channel/src/discord_daemon_frontend.rs`'s existing `#[cfg(test)] mod tests` block (replace every `slack`/`Slack` with `discord`/`Discord` in the test names and the function name):

```rust
    #[test]
    fn build_discord_reply_appends_correction_when_outcome_differs() {
        let events = vec![StreamEventPayload::Text {
            text: "{\"path\": \"airports.csv\"}".into(),
        }];
        let out = build_discord_reply(
            &events,
            "completed: I wasn't able to produce a usable reply this turn — please try again.",
        );
        assert!(out.contains("{\"path\": \"airports.csv\"}"), "{out}");
        assert!(out.contains("corrected"), "{out}");
    }

    #[test]
    fn build_discord_reply_no_correction_when_outcome_matches() {
        let events = vec![StreamEventPayload::Text {
            text: "an answer".into(),
        }];
        let out = build_discord_reply(&events, "completed: an answer");
        assert_eq!(out, "an answer");
    }

    #[test]
    fn build_discord_reply_does_not_double_up_no_reply() {
        let out = build_discord_reply(&[], "completed: ");
        assert_eq!(out, "(no reply)", "must not print (no reply) twice: {out}");
    }

    #[test]
    fn build_discord_reply_surfaces_non_completed_outcome() {
        let out = build_discord_reply(&[], "timed out");
        assert!(out.contains("timed out"), "{out}");
    }
```

Add, immediately after `render_events_for_discord`'s closing `}`:

```rust

/// Build the outbound Discord reply text: the rendered event journal
/// (tool-call/status lines + streamed text), plus a correction line
/// when the turn's own outcome diverges from what the events alone
/// would show (a reply floor, a Candor/identifier-fidelity annotation,
/// or a non-completed outcome's reason). Skips the correction only
/// when it would exactly duplicate `render_events_for_discord`'s own
/// empty-events "(no reply)" fallback.
fn build_discord_reply(events: &[StreamEventPayload], outcome: &str) -> String {
    let displayed = crate::daemon_ipc::concat_text_events(events);
    let mut buf = render_events_for_discord(events);
    if let Some(note) = crate::daemon_ipc::turn_outcome_correction(&displayed, outcome) {
        if !(note == "(no reply)" && buf.trim() == "(no reply)") {
            if !buf.is_empty() && !buf.ends_with('\n') {
                buf.push('\n');
            }
            buf.push_str(&note);
        }
    }
    buf
}
```

Find (search for `let (events, _outcome) = session.submit_input(msg.text).await?;` in this file):

```rust
        let (events, _outcome) = session.submit_input(msg.text).await?;
        let buf = render_events_for_discord(&events);
```

Replace with:

```rust
        let (events, outcome) = session.submit_input(msg.text).await?;
        let buf = build_discord_reply(&events, &outcome);
```

Run: `cargo test -p aivyx-channel build_discord_reply`
Expected: all 4 new tests PASS.

- [ ] **Step 6: Repeat for Telegram**

Add the same 4 tests (renamed to `telegram`/`Telegram`) to `crates/aivyx-channel/src/telegram_daemon_frontend.rs`'s existing `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn build_telegram_reply_appends_correction_when_outcome_differs() {
        let events = vec![StreamEventPayload::Text {
            text: "{\"path\": \"airports.csv\"}".into(),
        }];
        let out = build_telegram_reply(
            &events,
            "completed: I wasn't able to produce a usable reply this turn — please try again.",
        );
        assert!(out.contains("{\"path\": \"airports.csv\"}"), "{out}");
        assert!(out.contains("corrected"), "{out}");
    }

    #[test]
    fn build_telegram_reply_no_correction_when_outcome_matches() {
        let events = vec![StreamEventPayload::Text {
            text: "an answer".into(),
        }];
        let out = build_telegram_reply(&events, "completed: an answer");
        assert_eq!(out, "an answer");
    }

    #[test]
    fn build_telegram_reply_does_not_double_up_no_reply() {
        let out = build_telegram_reply(&[], "completed: ");
        assert_eq!(out, "(no reply)", "must not print (no reply) twice: {out}");
    }

    #[test]
    fn build_telegram_reply_surfaces_non_completed_outcome() {
        let out = build_telegram_reply(&[], "timed out");
        assert!(out.contains("timed out"), "{out}");
    }
```

Add, immediately after `render_events_for_telegram`'s closing `}`:

```rust

/// Build the outbound Telegram reply text: the rendered event journal
/// (tool-call/status lines + streamed text), plus a correction line
/// when the turn's own outcome diverges from what the events alone
/// would show (a reply floor, a Candor/identifier-fidelity annotation,
/// or a non-completed outcome's reason). Skips the correction only
/// when it would exactly duplicate `render_events_for_telegram`'s own
/// empty-events "(no reply)" fallback.
fn build_telegram_reply(events: &[StreamEventPayload], outcome: &str) -> String {
    let displayed = crate::daemon_ipc::concat_text_events(events);
    let mut buf = render_events_for_telegram(events);
    if let Some(note) = crate::daemon_ipc::turn_outcome_correction(&displayed, outcome) {
        if !(note == "(no reply)" && buf.trim() == "(no reply)") {
            if !buf.is_empty() && !buf.ends_with('\n') {
                buf.push('\n');
            }
            buf.push_str(&note);
        }
    }
    buf
}
```

Find (this file's call site has an if/else for image attachments — search for `let (events, _outcome) = if let Some(ref img) = msg.image {`):

```rust
        let (events, _outcome) = if let Some(ref img) = msg.image {
            use base64::Engine;
            let encoder = base64::engine::general_purpose::STANDARD;
            let att = crate::daemon_ipc::IpcAttachment {
                media_type: img.media_type.clone(),
                data_base64: encoder.encode(&img.data),
                filename: None,
            };
            session
                .submit_input_with_attachments(msg.text, vec![att])
                .await?
        } else {
            session.submit_input(msg.text).await?
        };

        let buf = render_events_for_telegram(&events);
```

Replace with:

```rust
        let (events, outcome) = if let Some(ref img) = msg.image {
            use base64::Engine;
            let encoder = base64::engine::general_purpose::STANDARD;
            let att = crate::daemon_ipc::IpcAttachment {
                media_type: img.media_type.clone(),
                data_base64: encoder.encode(&img.data),
                filename: None,
            };
            session
                .submit_input_with_attachments(msg.text, vec![att])
                .await?
        } else {
            session.submit_input(msg.text).await?
        };

        let buf = build_telegram_reply(&events, &outcome);
```

Run: `cargo test -p aivyx-channel build_telegram_reply`
Expected: all 4 new tests PASS.

- [ ] **Step 7: Full-crate check and commit**

Run: `cargo clippy -p aivyx-channel --all-targets -- -D warnings && cargo test -p aivyx-channel`
Expected: clean, all tests pass (the pre-existing `render_events_for_slack`/`_discord`/`_telegram` tests are untouched and still call those functions directly, unaffected by the new wrapper functions).

```bash
git add crates/aivyx-channel/src/slack_daemon_frontend.rs crates/aivyx-channel/src/discord_daemon_frontend.rs crates/aivyx-channel/src/telegram_daemon_frontend.rs
git commit -m "feat(channel): wire turn_outcome_correction into Telegram/Discord/Slack

Turn-outcome-correction follow-up to POLISH_WAVES.md sub-project 4's
final review. All three adapters batch-send one message at
TurnComplete rather than streaming live, so unlike Studio/TUI there's
no 'already displayed' constraint — but each was discarding its own
outcome field (\`let (events, _outcome) = ...\`) and reconstructing the
sent message purely from raw StreamEventPayload events. A small
build_*_reply wrapper per adapter (matching this codebase's existing
per-adapter-duplication convention) now appends a correction line when
the turn's authoritative outcome diverges from the event replay,
guarding against duplicating each adapter's own existing empty-events
'(no reply)' fallback.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

---

### Task 5: Daemon-backed CLI REPL wiring

Added during this plan's final review: the final review's own surface enumeration was incomplete. `run_daemon_session_connected` (the default `aivyx` interactive chat entry point — `crates/aivyx-cli/src/bin/aivyx.rs:9558`) already captures `outcome` (not `_outcome`) from `session.submit_input(...)`, but only stores it into the returned `SessionReport`, never writes it to the terminal. Same class of gap as the TUI (Task 3), same fix shape.

**Files:**
- Modify: `crates/aivyx-channel/src/daemon_session.rs:18` (import), `:132-139` (event-rendering block in `run_daemon_session_inner`)

**Interfaces:**
- Consumes: `crate::daemon_ipc::{concat_text_events, turn_outcome_correction}` (Task 1, re-exported via `aivyx-channel`'s `daemon_ipc` module).

- [ ] **Step 1: Add the import**

Find:

```rust
use crate::daemon_ipc::{FrontendType, StreamEventPayload};
```

Replace with:

```rust
use crate::daemon_ipc::{concat_text_events, turn_outcome_correction, FrontendType, StreamEventPayload};
```

- [ ] **Step 2: Wire the correction into the REPL loop**

Find, inside `run_daemon_session_inner`'s `loop { ... }` body (search for `let (events, outcome) = session.submit_input(input.to_string())`):

```rust
        let (events, outcome) = session.submit_input(input.to_string())
            .await
            .map_err(|e| e.to_string())?;

        for event in &events {
            let rendered = event.render_for_cli();
            write!(writer, "{rendered}").map_err(|e| format!("render write: {e}"))?;
        }
        writer.flush().map_err(|e| format!("render flush: {e}"))?;

        for event in &events {
            if let StreamEventPayload::ApprovalGate {
```

Replace with:

```rust
        let (events, outcome) = session.submit_input(input.to_string())
            .await
            .map_err(|e| e.to_string())?;

        for event in &events {
            let rendered = event.render_for_cli();
            write!(writer, "{rendered}").map_err(|e| format!("render write: {e}"))?;
        }
        writer.flush().map_err(|e| format!("render flush: {e}"))?;

        // Turn-outcome-correction follow-up (POLISH_WAVES.md
        // sub-project 4) — show the turn's own authoritative outcome
        // when it diverges from what the streamed events alone
        // rendered (a reply floor, a Candor/identifier-fidelity
        // annotation, or a non-completed outcome's reason). `outcome`
        // was already captured here (used below for the session
        // report) but never written to the terminal.
        let displayed = concat_text_events(&events);
        if let Some(note) = turn_outcome_correction(&displayed, &outcome) {
            writeln!(writer, "{note}").map_err(|e| format!("outcome-correction write: {e}"))?;
            writer.flush().map_err(|e| format!("outcome-correction flush: {e}"))?;
        }

        for event in &events {
            if let StreamEventPayload::ApprovalGate {
```

(Leave the rest of the loop — the approval-gate handling, `turns_run += 1;`, `last_outcome_str = Some(outcome);` — exactly as-is; `outcome` is a `String` that gets moved into `last_outcome_str` at the end of the loop body, and this step only reads it by reference via `&outcome`, so no ownership conflict.)

- [ ] **Step 3: Compile-check**

Run: `cargo check -p aivyx-channel && cargo clippy -p aivyx-channel --all-targets -- -D warnings`
Expected: clean.

No new automated test for this task: `run_daemon_session_inner` takes a concrete `DaemonSession` (a real daemon connection, not a trait), so it cannot be unit-tested without a live daemon — matching this file's own existing convention (no `#[cfg(test)] mod tests` block exists in `daemon_session.rs` at all). Verification is compile + clippy plus the code-inspection argument above (this is the identical 3-line pattern already proven correct and tested at the shared-helper level in Task 1, and identical in shape to Task 3's TUI wiring).

- [ ] **Step 4: Full-crate check and commit**

Run: `cargo test -p aivyx-channel`
Expected: all pre-existing tests still pass (this change adds no new test surface, so this just confirms no regression).

```bash
git add crates/aivyx-channel/src/daemon_session.rs
git commit -m "feat(channel): wire turn_outcome_correction into the daemon-backed CLI REPL

Found during this plan's own final review — the surface enumeration
in the design spec missed the default 'aivyx' interactive chat entry
point (run_daemon_session_connected). It already captured outcome
(used only for the session report) but never wrote it to the
terminal. Same 3-line pattern as the TUI's Task 3 fix.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Final Verification

After all 5 tasks:

```bash
cargo clippy --workspace --exclude aivyx-desktop --all-targets -- -D warnings
cargo test --workspace --exclude aivyx-desktop
```

Expected: zero warnings, zero failures.

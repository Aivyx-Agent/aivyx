# Team-Command Sender Allowlist Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the gap where any sender in a connected Telegram/Discord/Slack chat can issue `/team ...` commands (status, approve, reject, pause, resume, abort, run) with a new, deny-by-default, per-channel-type sender allowlist enforced client-side in each channel-adapter process.

**Architecture:** A new `team_command_allowed_senders` config field per channel (native-typed per platform's own real sender-id type), checked at the very top of each channel's existing `handle_*_incoming_command` precedence-chain function — before either the `/team run`-specific logic or the generic `/team ...` dispatch it wraps ever run. A shared, pure `sender_allowed` helper backs the check on all three channels.

**Tech Stack:** Rust, the existing `aivyx-config`/`aivyx-channel`/`aivyx-cli` machinery this whole initiative has already extended three times.

## Global Constraints

- **Deny by default.** Empty or absent `team_command_allowed_senders` denies every sender — no `/team` command of any kind succeeds until the operator explicitly populates the list.
- **Gates the whole `/team` surface uniformly** — `status`/`approve`/`reject`/`pause`/`resume`/`abort`/`run` all require allowlist membership. One check, one mental model.
- **`gate_command.rs` (the separate `/approve`/`/reject` parser for the old single-agent mission system) is out of scope and must not be touched.** Re-verified during planning: it remains a distinct call, in each channel's own loop, running *after* `handle_*_incoming_command` returns `ForwardToChatTurn` — never folded into the precedence-chain function this plan modifies.
- **Enforcement is client-side**, in each channel-adapter process — not a daemon round-trip. The sender id being checked is authenticated by the platform's own API before the channel-adapter ever constructs an `IncomingMessage`.
- **Denial always produces a clear `✗`-prefixed reply, never silence.**
- **The ordering-invariant test must be right on the first attempt.** Every task that wires this into a channel must include the exact test that would fail if the check were misplaced, *and* a step directing the implementer to verify this by mutation (temporarily move the check, confirm the test fails with the expected symptom, restore, confirm it passes) before considering the task done — not leave that verification to review time. This is a hard-won lesson from this exact codebase: Piece C's own analogous test needed two full review rounds because its first version only exercised an extracted helper's internals, never the real call-site order.

---

## File Structure

- **Modify** `crates/aivyx-config/src/lib.rs` — `RawTelegram`/`TelegramConfig` gain `team_command_allowed_senders: Vec<i64>`; `RawDiscord`/`DiscordConfig` gain `Vec<u64>`; `RawSlack`/`SlackConfig` gain `Vec<String>`. Mirrors `team_run_channel`'s own exact pattern.
- **Modify** `crates/aivyx-channel/src/team_command.rs` — new `pub fn sender_allowed<T: PartialEq>(allowed_senders: &[T], sender_id: &T) -> bool`, shared by all three channels (the semantics — including deny-on-empty, which `slice::contains` already gives for free — are identical regardless of each platform's own sender-id type).
- **Modify** `crates/aivyx-channel/src/telegram_daemon_frontend.rs` / `discord_daemon_frontend.rs` / `slack_daemon_frontend.rs` — each channel's `handle_*_incoming_command` gains the check at its top; the per-chat loop function threads the new config value and the per-message sender id through.
- **Modify** `crates/aivyx-cli/src/bin/aivyx.rs` — thread the new config field from each loaded `TelegramConfig`/`DiscordConfig`/`SlackConfig` into the three `run_*_daemon_multi_session` call sites.

---

### Task 1: `team_command_allowed_senders` config field (all 3 channels)

**Files:**
- Modify: `crates/aivyx-config/src/lib.rs` — `RawTelegram` (~line 4580), `RawDiscord` (~line 4600), `RawSlack` (~line 4620, exact lines will have shifted slightly, locate by the existing `team_run_channel`/`team_trigger_rate_limit` fields already there); `TelegramConfig` (~line 1577), `DiscordConfig`, `SlackConfig`; the three conversion sites (~line 6012, 6053, 6095)

**Interfaces:**
- Produces: `TelegramConfig.team_command_allowed_senders: Vec<i64>`, `DiscordConfig.team_command_allowed_senders: Vec<u64>`, `SlackConfig.team_command_allowed_senders: Vec<String>`. `#[serde(default)]` on the raw TOML fields (absent → empty `Vec`, matching the deny-by-default global constraint since an empty `Vec` denies every sender via `slice::contains`). Consumed by Tasks 3-5.

- [ ] **Step 1: Write the failing tests**

Add to `crates/aivyx-config/src/tests.rs` (the file Piece C's own equivalent tests landed in — confirm this is still the real test module by checking for `team_run_channel_defaults_to_false_and_unset_rate_limit`-style tests before writing; if the real current test module has a different name, use that instead of guessing):

```rust
#[test]
fn telegram_team_command_allowed_senders_defaults_to_empty() {
    let toml = r#"
        [telegram]
        token = "t"
    "#;
    let cfg = load_with_toml(toml, "telegram-allowlist-default");
    let tg = cfg.telegram.expect("telegram section present");
    assert!(tg.team_command_allowed_senders.is_empty());
}

#[test]
fn telegram_team_command_allowed_senders_round_trips_from_toml() {
    let toml = r#"
        [telegram]
        token = "t"
        team_command_allowed_senders = [123456789, 987654321]
    "#;
    let cfg = load_with_toml(toml, "telegram-allowlist-round-trip");
    let tg = cfg.telegram.expect("telegram section present");
    assert_eq!(tg.team_command_allowed_senders, vec![123456789, 987654321]);
}

#[test]
fn discord_and_slack_team_command_allowed_senders_round_trip_from_toml() {
    let toml = r#"
        [discord]
        token = "t"
        team_command_allowed_senders = [111111111, 222222222]

        [slack]
        bot_token = "b"
        app_token = "a"
        team_command_allowed_senders = ["U012ABCDEF", "U098ZYXWVU"]
    "#;
    let cfg = load_with_toml(toml, "discord-slack-allowlist-round-trip");
    let discord = cfg.discord.expect("discord section present");
    assert_eq!(
        discord.team_command_allowed_senders,
        vec![111111111u64, 222222222u64]
    );
    let slack = cfg.slack.expect("slack section present");
    assert_eq!(
        slack.team_command_allowed_senders,
        vec!["U012ABCDEF".to_string(), "U098ZYXWVU".to_string()]
    );
}
```

If this crate's real test module uses a different loader function name than `load_with_toml(body, tag) -> AivyxConfig` (check an existing nearby test in the same file first and copy its exact loader call), use that exact function instead — do not invent a new one.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p aivyx-config team_command_allowed_senders -- --test-threads=1`
Expected: FAIL to compile — the field doesn't exist yet on any of the three config structs.

- [ ] **Step 3: Add the raw TOML fields**

In `crates/aivyx-config/src/lib.rs`, add to `RawTelegram`:

```rust
    /// Team-Command Sender Allowlist (2026-08-23) — the Telegram user
    /// ids allowed to issue any `/team ...` command (status/approve/
    /// reject/pause/resume/abort/run) from this channel. Empty/absent:
    /// no sender is authorized — deny by default, closing a real gap
    /// (previously, any sender in a connected chat could act).
    #[serde(default)]
    team_command_allowed_senders: Vec<i64>,
```

Add the identical field (same doc comment, `Vec<u64>`) to `RawDiscord`, and (same doc comment, `Vec<String>`) to `RawSlack`.

- [ ] **Step 4: Add the public config fields**

Add to `TelegramConfig`:

```rust
    /// Team-Command Sender Allowlist (2026-08-23) — the Telegram user
    /// ids allowed to issue any `/team ...` command from this channel.
    /// Empty: no sender is authorized (deny by default).
    pub team_command_allowed_senders: Vec<i64>,
```

Add the identical field (same doc comment, `Vec<u64>`) to `DiscordConfig`, and (same doc comment, `Vec<String>`) to `SlackConfig`.

- [ ] **Step 5: Wire the conversion sites**

In the function building `Some(TelegramConfig { token: telegram_token, chat_filter: telegram_chat_filter, team_run_channel: ..., team_trigger_rate_limit: ... })` (~line 6012), add one more field:

```rust
        let telegram = if telegram_token.is_some() || telegram_chat_filter.is_some() {
            Some(TelegramConfig {
                token: telegram_token,
                chat_filter: telegram_chat_filter,
                team_run_channel: toml.telegram.team_run_channel,
                team_trigger_rate_limit: toml.telegram.team_trigger_rate_limit,
                team_command_allowed_senders: toml.telegram.team_command_allowed_senders.clone(),
            })
        } else {
            None
        };
```

Add the identical one-line addition (`team_command_allowed_senders: toml.discord.team_command_allowed_senders.clone(),` / `toml.slack....clone(),`) to the Discord (~line 6053) and Slack (~line 6095) conversion sites — read them first to match each site's exact current field-list style before editing.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p aivyx-config team_command_allowed_senders -- --test-threads=1`
Expected: PASS, all 3 new tests.

- [ ] **Step 7: Run the crate's full suite**

Run: `cargo test -p aivyx-config -- --test-threads=1`
Expected: PASS, no regressions (baseline: 409 tests — confirm the exact current count first if it's shifted since, then expect baseline + 3).

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-config/src/lib.rs crates/aivyx-config/src/tests.rs
git commit -m "feat(config): add team_command_allowed_senders per-channel config (Sender Allowlist Task 1)"
```

---

### Task 2: `sender_allowed` shared pure helper

**Files:**
- Modify: `crates/aivyx-channel/src/team_command.rs`

**Interfaces:**
- Produces: `pub fn sender_allowed<T: PartialEq>(allowed_senders: &[T], sender_id: &T) -> bool`. Consumed by Tasks 3-5.

- [ ] **Step 1: Write the failing tests**

Add to `team_command.rs`'s existing `mod tests`:

```rust
#[test]
fn sender_allowed_denies_when_list_is_empty() {
    let allowed: Vec<i64> = vec![];
    assert!(!sender_allowed(&allowed, &123));
}

#[test]
fn sender_allowed_denies_a_sender_not_in_the_list() {
    let allowed = vec![123i64, 456];
    assert!(!sender_allowed(&allowed, &999));
}

#[test]
fn sender_allowed_allows_a_sender_in_the_list() {
    let allowed = vec![123i64, 456];
    assert!(sender_allowed(&allowed, &123));
    assert!(sender_allowed(&allowed, &456));
}

#[test]
fn sender_allowed_works_for_string_sender_ids_too() {
    // Slack's own sender-id type — confirms the generic bound covers
    // every platform's real id type, not just integers.
    let allowed = vec!["U012ABCDEF".to_string(), "U098ZYXWVU".to_string()];
    assert!(sender_allowed(&allowed, &"U012ABCDEF".to_string()));
    assert!(!sender_allowed(&allowed, &"U999NOTALLOWED".to_string()));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p aivyx-channel --lib team_command:: -- --test-threads=1`
Expected: FAIL to compile — `sender_allowed` doesn't exist yet.

- [ ] **Step 3: Add the function**

Add to `team_command.rs`, above the `#[cfg(test)]` block:

```rust
/// Team-Command Sender Allowlist (2026-08-23) — pure, deny-by-default
/// sender authorization for the whole `/team ...` command surface.
/// Shared across all three channels since the semantics are identical
/// regardless of each platform's own native sender-id type (Telegram
/// `i64`, Discord `u64`, Slack `String`) — including the deny-on-empty
/// behavior, which `slice::contains` already gives for free on an empty
/// slice, so an operator who never configures an allowlist denies every
/// sender rather than allowing everyone (the gap this whole feature
/// exists to close).
pub fn sender_allowed<T: PartialEq>(allowed_senders: &[T], sender_id: &T) -> bool {
    allowed_senders.contains(sender_id)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p aivyx-channel --lib team_command:: -- --test-threads=1`
Expected: PASS, all tests (existing + 4 new).

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-channel/src/team_command.rs
git commit -m "feat(channel): add shared sender_allowed pure helper (Sender Allowlist Task 2)"
```

---

### Task 3: Wire the sender allowlist into Telegram

**Files:**
- Modify: `crates/aivyx-channel/src/telegram_daemon_frontend.rs` — `handle_telegram_incoming_command` (~line 361), `run_telegram_daemon_chat_task` (~line 394), `run_telegram_daemon_multi_session` (its own signature and the site inside it that spawns `run_telegram_daemon_chat_task`)
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` — the Telegram config extraction (~line 9682, the `let team_run_channel = tg.team_run_channel;` site) and the `run_telegram_daemon_multi_session(...)` call (~line 9729)

**Interfaces:**
- Consumes: Task 1's `TelegramConfig.team_command_allowed_senders: Vec<i64>`, Task 2's `team_command::sender_allowed`.

- [ ] **Step 1: Read the current real code before editing**

Read `crates/aivyx-channel/src/telegram_daemon_frontend.rs`'s current `handle_telegram_incoming_command`, `run_telegram_daemon_chat_task`, and `run_telegram_daemon_multi_session` in full, and `crates/aivyx-cli/src/bin/aivyx.rs`'s real current lines around `let team_run_channel = tg.team_run_channel;` and the `run_telegram_daemon_multi_session(...)` call — this task's code below assumes the shapes confirmed during this plan's own research; adjust variable names to match the real current code, not this plan's paraphrase, if they differ (unlikely, since only doc/design-doc commits have landed since, but confirm).

- [ ] **Step 2: Write the failing tests first**

Add to `telegram_daemon_frontend.rs`'s existing `mod tests`:

```rust
#[tokio::test]
async fn unauthorized_sender_is_denied_before_team_run_recognition() {
    // This is the test that proves the ordering: the sender check must
    // run BEFORE handle_telegram_team_run_message's own call, or an
    // unauthorized sender's /team run would still reach the confirm-
    // first flow (team_run_channel: true below would otherwise let it
    // succeed). Piece C's own analogous ordering test needed two review
    // rounds because an earlier version only proved a narrower helper's
    // internals, never the real call-site order — this test is written
    // to avoid that exact failure mode by asserting on the SPECIFIC
    // confirm-prompt text that would appear if the check were bypassed.
    let mut pending_trigger = None;
    let mut trigger_history = Vec::new();
    let outcome = handle_telegram_incoming_command(
        "/team run close the books",
        std::path::Path::new("/nonexistent/unused.sock"),
        &mut pending_trigger,
        &mut trigger_history,
        true, // team_run_channel -- would otherwise let this succeed
        None, // team_trigger_rate_limit
        999,  // sender_id -- NOT in the allowlist
        &[123i64, 456],
    )
    .await;
    match outcome {
        TelegramIncomingOutcome::Reply(text) => {
            assert!(
                text.contains("not authorized to issue /team commands"),
                "expected the sender-denial reply, got: {text}"
            );
            assert!(
                !text.contains("Reply yes/no"),
                "got the confirm-first prompt instead of the sender-denial \
                 reply -- this means the sender-allowlist check is being \
                 bypassed by /team run's own recognition, the exact bug \
                 this test exists to catch: {text}"
            );
        }
        TelegramIncomingOutcome::ForwardToChatTurn => {
            panic!("expected a denial reply, not a forward to chat turn")
        }
    }
    assert!(
        pending_trigger.is_none(),
        "an unauthorized /team run must not set a pending trigger"
    );
}

#[tokio::test]
async fn unauthorized_sender_is_denied_for_the_generic_team_surface_too() {
    // Confirms the check gates the WHOLE /team surface, not just Run --
    // /team status is a Piece B command with no team_run_channel
    // involvement at all.
    let mut pending_trigger = None;
    let mut trigger_history = Vec::new();
    let outcome = handle_telegram_incoming_command(
        "/team status",
        std::path::Path::new("/nonexistent/unused.sock"),
        &mut pending_trigger,
        &mut trigger_history,
        false,
        None,
        999,
        &[123i64, 456],
    )
    .await;
    match outcome {
        TelegramIncomingOutcome::Reply(text) => {
            assert!(text.contains("not authorized to issue /team commands"));
        }
        TelegramIncomingOutcome::ForwardToChatTurn => {
            panic!("expected a denial reply, not a forward to chat turn")
        }
    }
}

#[tokio::test]
async fn authorized_sender_reaches_dispatch_not_the_denial() {
    // Confirms the check doesn't false-positive-deny a legitimate
    // sender. socket_path points nowhere, so team_dispatch::dispatch
    // itself will fail (daemon unreachable) -- the point is the reply
    // is THAT failure, not the sender-denial message, proving the
    // authorized sender got past this check and reached real dispatch.
    let mut pending_trigger = None;
    let mut trigger_history = Vec::new();
    let outcome = handle_telegram_incoming_command(
        "/team status",
        std::path::Path::new("/nonexistent/unused.sock"),
        &mut pending_trigger,
        &mut trigger_history,
        false,
        None,
        123, // sender_id -- IS in the allowlist
        &[123i64, 456],
    )
    .await;
    match outcome {
        TelegramIncomingOutcome::Reply(text) => {
            assert!(!text.contains("not authorized to issue /team commands"));
        }
        TelegramIncomingOutcome::ForwardToChatTurn => {
            panic!("expected a Reply (dispatch attempted), not ForwardToChatTurn")
        }
    }
}

#[tokio::test]
async fn non_team_text_is_unaffected_regardless_of_sender() {
    let mut pending_trigger = None;
    let mut trigger_history = Vec::new();
    let outcome = handle_telegram_incoming_command(
        "hello, just chatting",
        std::path::Path::new("/nonexistent/unused.sock"),
        &mut pending_trigger,
        &mut trigger_history,
        false,
        None,
        999, // unauthorized sender -- must not matter here
        &[123i64, 456],
    )
    .await;
    assert_eq!(outcome, TelegramIncomingOutcome::ForwardToChatTurn);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p aivyx-channel --lib telegram_daemon_frontend:: -- --test-threads=1`
Expected: FAIL to compile — `handle_telegram_incoming_command` doesn't accept `sender_id`/`allowed_senders` parameters yet.

- [ ] **Step 4: Add the check and thread the new parameters**

In `crates/aivyx-channel/src/telegram_daemon_frontend.rs`, update `handle_telegram_incoming_command`'s signature and add the check as its first statement:

```rust
async fn handle_telegram_incoming_command(
    text: &str,
    socket_path: &Path,
    pending_trigger: &mut Option<PendingTrigger>,
    trigger_history: &mut Vec<Instant>,
    team_run_channel: bool,
    team_trigger_rate_limit: Option<u32>,
    sender_id: i64,
    allowed_senders: &[i64],
) -> TelegramIncomingOutcome {
    // Team-Command Sender Allowlist (2026-08-23) — must run before
    // BOTH handle_telegram_team_run_message (or an unauthorized /team
    // run would still reach the confirm-first flow) and the generic
    // team_command::parse dispatch below. Checked only when the text
    // actually parses as a /team command at all -- ordinary chat text
    // from an unauthorized sender is completely unaffected.
    if team_command::parse(text).is_some() && !sender_allowed(allowed_senders, &sender_id) {
        return TelegramIncomingOutcome::Reply(
            "✗ you are not authorized to issue /team commands.".to_string(),
        );
    }

    match handle_telegram_team_run_message(
        text,
        socket_path,
        pending_trigger,
        trigger_history,
        team_run_channel,
        team_trigger_rate_limit,
    )
    .await
    {
        TelegramChatOutcome::Reply(reply) => return TelegramIncomingOutcome::Reply(reply),
        TelegramChatOutcome::NotHandled => {}
    }

    if let Some(team_cmd) = team_command::parse(text) {
        let reply = team_dispatch::dispatch(socket_path, team_cmd).await;
        return TelegramIncomingOutcome::Reply(reply);
    }

    TelegramIncomingOutcome::ForwardToChatTurn
}
```

Add `use crate::team_command::sender_allowed;` to this file's imports if `team_command::sender_allowed` isn't already reachable via an existing `use crate::team_command::{self, TeamCommand};`-style import (adjust to `use crate::team_command::{self, sender_allowed, TeamCommand};` if that's the existing form).

Update `run_telegram_daemon_chat_task`'s signature to accept the new config value, and its call to `handle_telegram_incoming_command` to pass both new arguments:

```rust
#[allow(clippy::too_many_arguments)]
async fn run_telegram_daemon_chat_task(
    transport: Arc<ReqwestTransport>,
    chat_id: i64,
    socket_path: PathBuf,
    role: Option<String>,
    mut mailbox: tokio::sync::mpsc::Receiver<IncomingMessage>,
    shutdown: CancellationToken,
    team_run_channel: bool,
    team_trigger_rate_limit: Option<u32>,
    team_command_allowed_senders: Vec<i64>,
) -> Result<(), DaemonError> {
```

(`#[allow(clippy::too_many_arguments)]` is already present — nine parameters now, consistent with this crate's own established precedent for this exact attribute on these per-chat task functions.) Inside the loop, update the call:

```rust
        match handle_telegram_incoming_command(
            msg.text.trim(),
            &socket_path,
            &mut pending_trigger,
            &mut trigger_history,
            team_run_channel,
            team_trigger_rate_limit,
            msg.user_id,
            &team_command_allowed_senders,
        )
        .await
```

Update `run_telegram_daemon_multi_session`'s own signature to accept and forward `team_command_allowed_senders: Vec<i64>` (add as a new trailing parameter), and find where it spawns `run_telegram_daemon_chat_task` (a `tokio::spawn` inside its own per-chat loop) — add `team_command_allowed_senders.clone(),` to that call (clone since multiple chat tasks may be spawned from the same multi-session loop, each needing its own copy).

- [ ] **Step 5: Thread the config value from `aivyx.rs`**

In `crates/aivyx-cli/src/bin/aivyx.rs`, near the existing `let team_run_channel = tg.team_run_channel;` / `let team_trigger_rate_limit = tg.team_trigger_rate_limit;` lines, add:

```rust
            let team_command_allowed_senders = tg.team_command_allowed_senders.clone();
```

Update the `run_telegram_daemon_multi_session(...)` call to pass it as a new trailing argument:

```rust
                match run_telegram_daemon_multi_session(
                    transport,
                    chat_filter,
                    sp.clone(),
                    Some(active_role_name.clone()),
                    shutdown.clone(),
                    team_run_channel,
                    team_trigger_rate_limit,
                    team_command_allowed_senders,
                )
                .await
```

(Keep every existing argument in its current position — this shows only where the one new trailing argument goes; read the real current call site first and confirm the exact existing argument list before editing.)

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p aivyx-channel --lib telegram_daemon_frontend:: -- --test-threads=1`
Expected: PASS, all 4 new tests plus existing ones.

- [ ] **Step 7: Verify the ordering test genuinely discriminates, by mutation**

Temporarily move the sender-allowlist check (the `if team_command::parse(text).is_some() && !sender_allowed(...)` block) from the top of `handle_telegram_incoming_command` to immediately *after* the `handle_telegram_team_run_message` call (i.e., simulate the bug this test exists to catch — the check no longer precedes `/team run`'s own recognition). Run:

Run: `cargo test -p aivyx-channel --lib telegram_daemon_frontend::tests::unauthorized_sender_is_denied_before_team_run_recognition -- --test-threads=1`
Expected: FAIL — the test should fail with the assertion message about getting the confirm-first prompt instead of the denial (since `team_run_channel: true` in the test means an unauthorized `/team run` would now succeed in reaching the confirm stage before the misplaced check ever runs).

Then restore the check to its correct position (before `handle_telegram_team_run_message`) and re-run:

Run: `cargo test -p aivyx-channel --lib telegram_daemon_frontend:: -- --test-threads=1`
Expected: PASS, all tests — confirming the test suite is back to green with the check in its correct position. Report the exact output of both runs (the failure and the recovery) in your task report — this is the specific verification this plan's own Global Constraints require before considering this task done.

- [ ] **Step 8: Run the crate's full suite and confirm builds**

Run: `cargo build -p aivyx-channel` and `cargo build -p aivyx-cli` (both must succeed), then `cargo test -p aivyx-channel --lib -- --test-threads=1` (baseline before this task: 1273 tests — report the exact new count).

- [ ] **Step 9: Commit**

```bash
git add crates/aivyx-channel/src/telegram_daemon_frontend.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat(channel): wire sender allowlist into Telegram /team commands (Sender Allowlist Task 3)"
```

---

### Task 4: Wire the sender allowlist into Discord

**Files:**
- Modify: `crates/aivyx-channel/src/discord_daemon_frontend.rs` — `handle_discord_incoming_command`, `run_discord_daemon_channel_task`, `run_discord_daemon_multi_session`
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` — the Discord config extraction (~line 9796, the `let team_run_channel = dc.team_run_channel;` site) and the `run_discord_daemon_multi_session(...)` call (~line 9841)

**Interfaces:**
- Consumes: Task 1's `DiscordConfig.team_command_allowed_senders: Vec<u64>`, Task 2's `team_command::sender_allowed`.

- [ ] **Step 1: Read the current real code before editing**

Same caution as Task 3 Step 1, applied to `discord_daemon_frontend.rs` and the `dc`/Discord section of `aivyx.rs`.

- [ ] **Step 2: Write the failing tests first**

Add to `discord_daemon_frontend.rs`'s existing `mod tests` — identical shape to Task 3 Step 2's four tests, adapted only for Discord's own types (`u64` sender ids, `DiscordIncomingOutcome` instead of `TelegramIncomingOutcome`, `handle_discord_incoming_command`):

```rust
#[tokio::test]
async fn unauthorized_sender_is_denied_before_team_run_recognition() {
    let mut pending_trigger = None;
    let mut trigger_history = Vec::new();
    let outcome = handle_discord_incoming_command(
        "/team run close the books",
        std::path::Path::new("/nonexistent/unused.sock"),
        &mut pending_trigger,
        &mut trigger_history,
        true,
        None,
        999u64,
        &[123u64, 456],
    )
    .await;
    match outcome {
        DiscordIncomingOutcome::Reply(text) => {
            assert!(
                text.contains("not authorized to issue /team commands"),
                "expected the sender-denial reply, got: {text}"
            );
            assert!(
                !text.contains("Reply yes/no"),
                "got the confirm-first prompt instead of the sender-denial \
                 reply -- this means the sender-allowlist check is being \
                 bypassed by /team run's own recognition, the exact bug \
                 this test exists to catch: {text}"
            );
        }
        DiscordIncomingOutcome::ForwardToChatTurn => {
            panic!("expected a denial reply, not a forward to chat turn")
        }
    }
    assert!(
        pending_trigger.is_none(),
        "an unauthorized /team run must not set a pending trigger"
    );
}

#[tokio::test]
async fn unauthorized_sender_is_denied_for_the_generic_team_surface_too() {
    let mut pending_trigger = None;
    let mut trigger_history = Vec::new();
    let outcome = handle_discord_incoming_command(
        "/team status",
        std::path::Path::new("/nonexistent/unused.sock"),
        &mut pending_trigger,
        &mut trigger_history,
        false,
        None,
        999u64,
        &[123u64, 456],
    )
    .await;
    match outcome {
        DiscordIncomingOutcome::Reply(text) => {
            assert!(text.contains("not authorized to issue /team commands"));
        }
        DiscordIncomingOutcome::ForwardToChatTurn => {
            panic!("expected a denial reply, not a forward to chat turn")
        }
    }
}

#[tokio::test]
async fn authorized_sender_reaches_dispatch_not_the_denial() {
    let mut pending_trigger = None;
    let mut trigger_history = Vec::new();
    let outcome = handle_discord_incoming_command(
        "/team status",
        std::path::Path::new("/nonexistent/unused.sock"),
        &mut pending_trigger,
        &mut trigger_history,
        false,
        None,
        123u64,
        &[123u64, 456],
    )
    .await;
    match outcome {
        DiscordIncomingOutcome::Reply(text) => {
            assert!(!text.contains("not authorized to issue /team commands"));
        }
        DiscordIncomingOutcome::ForwardToChatTurn => {
            panic!("expected a Reply (dispatch attempted), not ForwardToChatTurn")
        }
    }
}

#[tokio::test]
async fn non_team_text_is_unaffected_regardless_of_sender() {
    let mut pending_trigger = None;
    let mut trigger_history = Vec::new();
    let outcome = handle_discord_incoming_command(
        "hello, just chatting",
        std::path::Path::new("/nonexistent/unused.sock"),
        &mut pending_trigger,
        &mut trigger_history,
        false,
        None,
        999u64,
        &[123u64, 456],
    )
    .await;
    assert_eq!(outcome, DiscordIncomingOutcome::ForwardToChatTurn);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p aivyx-channel --lib discord_daemon_frontend:: -- --test-threads=1`
Expected: FAIL to compile.

- [ ] **Step 4: Add the check and thread the new parameters**

Same shape as Task 3 Step 4, applied to `handle_discord_incoming_command` (add `sender_id: u64, allowed_senders: &[u64]` parameters, add the identical check as the first statement — same denial string), `run_discord_daemon_channel_task` (add `team_command_allowed_senders: Vec<u64>` parameter, pass `msg.author_id` and `&team_command_allowed_senders` into the `handle_discord_incoming_command` call), and `run_discord_daemon_multi_session` (add and forward the new parameter to its own `tokio::spawn` of `run_discord_daemon_channel_task`).

- [ ] **Step 5: Thread the config value from `aivyx.rs`**

Same shape as Task 3 Step 5, applied to the Discord config extraction (`let team_command_allowed_senders = dc.team_command_allowed_senders.clone();`) and the `run_discord_daemon_multi_session(...)` call site (~line 9841), appending the new trailing argument.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p aivyx-channel --lib discord_daemon_frontend:: -- --test-threads=1`
Expected: PASS, all 4 new tests plus existing ones.

- [ ] **Step 7: Verify the ordering test genuinely discriminates, by mutation**

Same procedure as Task 3 Step 7, applied to Discord's own `unauthorized_sender_is_denied_before_team_run_recognition` test and `handle_discord_incoming_command`. Report both run outputs (failure, then recovery) in your task report.

- [ ] **Step 8: Run the crate's full suite and confirm builds**

Run: `cargo build -p aivyx-channel` and `cargo build -p aivyx-cli` (both must succeed), then `cargo test -p aivyx-channel --lib -- --test-threads=1` (report the exact count vs. Task 3's own final count).

- [ ] **Step 9: Commit**

```bash
git add crates/aivyx-channel/src/discord_daemon_frontend.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat(channel): wire sender allowlist into Discord /team commands (Sender Allowlist Task 4)"
```

---

### Task 5: Wire the sender allowlist into Slack

**Files:**
- Modify: `crates/aivyx-channel/src/slack_daemon_frontend.rs` — `handle_slack_incoming_command`, `run_slack_daemon_partition_task`, `run_slack_daemon_multi_session`
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` — the Slack config extraction (~line 9918, the `let team_run_channel = sc.team_run_channel;` site) and the `run_slack_daemon_multi_session(...)` call (~line 9968)

**Interfaces:**
- Consumes: Task 1's `SlackConfig.team_command_allowed_senders: Vec<String>`, Task 2's `team_command::sender_allowed`.

- [ ] **Step 1: Read the current real code before editing**

Same caution as Tasks 3-4 Step 1, applied to `slack_daemon_frontend.rs` and the `sc`/Slack section of `aivyx.rs`.

- [ ] **Step 2: Write the failing tests first**

Add to `slack_daemon_frontend.rs`'s existing `mod tests` — identical shape to Tasks 3-4's own tests, adapted for Slack's `String` sender ids and `SlackIncomingOutcome`/`handle_slack_incoming_command`:

```rust
#[tokio::test]
async fn unauthorized_sender_is_denied_before_team_run_recognition() {
    let mut pending_trigger = None;
    let mut trigger_history = Vec::new();
    let outcome = handle_slack_incoming_command(
        "/team run close the books",
        std::path::Path::new("/nonexistent/unused.sock"),
        &mut pending_trigger,
        &mut trigger_history,
        true,
        None,
        "U999NOTALLOWED".to_string(),
        &["U123ALLOWED".to_string(), "U456ALLOWED".to_string()],
    )
    .await;
    match outcome {
        SlackIncomingOutcome::Reply(text) => {
            assert!(
                text.contains("not authorized to issue /team commands"),
                "expected the sender-denial reply, got: {text}"
            );
            assert!(
                !text.contains("Reply yes/no"),
                "got the confirm-first prompt instead of the sender-denial \
                 reply -- this means the sender-allowlist check is being \
                 bypassed by /team run's own recognition, the exact bug \
                 this test exists to catch: {text}"
            );
        }
        SlackIncomingOutcome::ForwardToChatTurn => {
            panic!("expected a denial reply, not a forward to chat turn")
        }
    }
    assert!(
        pending_trigger.is_none(),
        "an unauthorized /team run must not set a pending trigger"
    );
}

#[tokio::test]
async fn unauthorized_sender_is_denied_for_the_generic_team_surface_too() {
    let mut pending_trigger = None;
    let mut trigger_history = Vec::new();
    let outcome = handle_slack_incoming_command(
        "/team status",
        std::path::Path::new("/nonexistent/unused.sock"),
        &mut pending_trigger,
        &mut trigger_history,
        false,
        None,
        "U999NOTALLOWED".to_string(),
        &["U123ALLOWED".to_string(), "U456ALLOWED".to_string()],
    )
    .await;
    match outcome {
        SlackIncomingOutcome::Reply(text) => {
            assert!(text.contains("not authorized to issue /team commands"));
        }
        SlackIncomingOutcome::ForwardToChatTurn => {
            panic!("expected a denial reply, not a forward to chat turn")
        }
    }
}

#[tokio::test]
async fn authorized_sender_reaches_dispatch_not_the_denial() {
    let mut pending_trigger = None;
    let mut trigger_history = Vec::new();
    let outcome = handle_slack_incoming_command(
        "/team status",
        std::path::Path::new("/nonexistent/unused.sock"),
        &mut pending_trigger,
        &mut trigger_history,
        false,
        None,
        "U123ALLOWED".to_string(),
        &["U123ALLOWED".to_string(), "U456ALLOWED".to_string()],
    )
    .await;
    match outcome {
        SlackIncomingOutcome::Reply(text) => {
            assert!(!text.contains("not authorized to issue /team commands"));
        }
        SlackIncomingOutcome::ForwardToChatTurn => {
            panic!("expected a Reply (dispatch attempted), not ForwardToChatTurn")
        }
    }
}

#[tokio::test]
async fn non_team_text_is_unaffected_regardless_of_sender() {
    let mut pending_trigger = None;
    let mut trigger_history = Vec::new();
    let outcome = handle_slack_incoming_command(
        "hello, just chatting",
        std::path::Path::new("/nonexistent/unused.sock"),
        &mut pending_trigger,
        &mut trigger_history,
        false,
        None,
        "U999NOTALLOWED".to_string(),
        &["U123ALLOWED".to_string(), "U456ALLOWED".to_string()],
    )
    .await;
    assert_eq!(outcome, SlackIncomingOutcome::ForwardToChatTurn);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p aivyx-channel --lib slack_daemon_frontend:: -- --test-threads=1`
Expected: FAIL to compile.

- [ ] **Step 4: Add the check and thread the new parameters**

Same shape as Tasks 3-4 Step 4, applied to `handle_slack_incoming_command` (add `sender_id: String, allowed_senders: &[String]` parameters — note `sender_id` is owned `String` here, not `&str`, matching `msg.user_id`'s own real type and avoiding a lifetime parameter this function doesn't otherwise need; the check becomes `if team_command::parse(text).is_some() && !sender_allowed(allowed_senders, &sender_id)`), `run_slack_daemon_partition_task` (add `team_command_allowed_senders: Vec<String>` parameter, pass `msg.user_id.clone()` and `&team_command_allowed_senders` into the call — `msg.user_id.clone()` since `msg` is also used later in the same loop iteration for `msg.channel_id.clone()`), and `run_slack_daemon_multi_session` (add and forward the new parameter to its own spawn of `run_slack_daemon_partition_task`).

- [ ] **Step 5: Thread the config value from `aivyx.rs`**

Same shape as Tasks 3-4 Step 5, applied to the Slack config extraction (`let team_command_allowed_senders = sc.team_command_allowed_senders.clone();`) and the `run_slack_daemon_multi_session(...)` call site (~line 9968), appending the new trailing argument.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p aivyx-channel --lib slack_daemon_frontend:: -- --test-threads=1`
Expected: PASS, all 4 new tests plus existing ones.

- [ ] **Step 7: Verify the ordering test genuinely discriminates, by mutation**

Same procedure as Tasks 3-4 Step 7, applied to Slack's own `unauthorized_sender_is_denied_before_team_run_recognition` test and `handle_slack_incoming_command`. Report both run outputs (failure, then recovery) in your task report.

- [ ] **Step 8: Run the crate's full suite and confirm builds**

Run: `cargo build -p aivyx-channel` and `cargo build -p aivyx-cli` (both must succeed), then `cargo test -p aivyx-channel --lib -- --test-threads=1` (report the exact final count).

- [ ] **Step 9: Commit**

```bash
git add crates/aivyx-channel/src/slack_daemon_frontend.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat(channel): wire sender allowlist into Slack /team commands (Sender Allowlist Task 5)"
```

---

## Final Verification

After all 5 tasks:

1. `cargo build -p aivyx-capability -p aivyx-config -p aivyx-ipc -p aivyx-audit -p aivyx-channel -p aivyx-cli` — clean build across every touched crate (do not attempt `cargo build --workspace`; `aivyx-desktop` fails on a pre-existing, unrelated missing system library in this sandboxed environment).
2. `cargo test -p aivyx-config -p aivyx-channel --lib -- --test-threads=1` — full suite passes across both touched crates, no regressions.
3. `cargo clippy -p aivyx-channel --all-targets -- -D warnings` — check for new warnings beyond the one pre-existing, out-of-scope `trigger.rs:223` finding.
4. Grep for any remaining `TODO`/`unimplemented!()` introduced by this plan's files — should be none.
5. Grep the whole diff for `gate_command` to confirm `gate_command.rs` itself, and every call site of `gate_command::parse`, are genuinely untouched by this plan (the Global Constraint this whole plan is built to honor).
6. Manually trace the full authorization path end-to-end by reading (not running) the final diff, for at least one channel: an operator with an empty `team_command_allowed_senders` (the default) — any sender's `/team status`, `/team approve ...`, or `/team run <goal>` all hit the new check at the top of `handle_*_incoming_command`, `sender_allowed` returns `false` (empty slice), and the reply is the denial message — never reaching `handle_*_team_run_message` or `team_dispatch::dispatch`.

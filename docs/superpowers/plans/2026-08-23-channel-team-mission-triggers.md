# Channel-Triggered New Mission Starts (Piece C) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an operator opt a channel (Telegram, Discord, or Slack) into starting new Nonagon team missions from chat via `/team run <goal>`, gated by a new, narrow, server-enforced capability (`team.run.channel`) that is off by default and never reachable by the model or a channel message alone.

**Architecture:** A new capability base (`team.run.channel`) that no trust tier grants by default; a new per-channel operator opt-in config bool the **daemon itself** reads and enforces (not the channel-adapter alone) via a new identity-declaring one-shot IPC call; a client-side (channel-adapter) confirm-first state machine and rate limiter, since those are UX/abuse-prevention, not the security boundary; and a `TeamCommand::Run` variant wired into the existing Piece B parser/frontends.

**Tech Stack:** Rust, tokio, the existing `aivyx-capability`/`aivyx-config`/`aivyx-ipc`/`aivyx-audit`/`aivyx-channel` machinery Pieces A and B already extended.

## Global Constraints

- **No pack/config selection anywhere in this path.** `/team run <goal>` always calls `start_from_goal_for_channel_trigger(goal, None, ...)` — `config` is hard-coded `None` in every call site this plan adds. Re-verified against the current `TeamMissionService::start_from_goal`/`start_from_goal_for_schedule` signatures (both take `config: Option<TeamConfig>`): this plan's new sibling method takes NO config parameter at all, structurally preventing a channel or the model from ever naming a pack — closing off the same union-vs-intersection risk class Piece A's Critical finding (`bind_lead_scopes`) exposed, without needing to fix that root cause.
- **The new capability (`team.run.channel`) is authorized server-side, by the daemon, not by trusting the channel-adapter process.** Re-verified: the *existing* one-shot IPC path Piece B's other six `/team ...` commands use (`FrontendMessage::Query`, sent via `daemon_client::send_query`) carries **no session/frontend identity at all** — it skips `StartSession` entirely, so the daemon has no way to know which channel a bare `Query` came from. This plan's new `/team run` command therefore uses a *different*, identity-declaring one-shot connection: connect → `StartSession { frontend_type: Some(<real platform>) }` → wait `SessionStarted` → send the new `RunTeamMissionChannel` request. The daemon's own `handle_connection` loop already binds `channel = Some(channel_factory(ft))` from that `frontend_type` (pre-existing code, confirmed unchanged), giving the new match arm a real, un-spoofable (by the channel-adapter alone — see below) `channel.platform()` to check against a new daemon-side-loaded config bool.
  - **Trust boundary note (accepted, not fixed here):** the daemon trusts whatever `FrontendType` a Unix-socket connection declares in `StartSession` — same trust boundary every existing channel command already relies on (a compromised or arbitrary local process connecting to the daemon's own socket could claim to be Telegram). This plan does not change or worsen that pre-existing boundary; it only ensures `/team run` actually reaches a real server-side check instead of none at all.
- **`team.run.channel` is not part of `CEILING_TRUSTED`, `CEILING_SEMITRUSTED`, or `CEILING_UNTRUSTED`.** Re-verified against the real `aivyx-capability` ceiling-construction code: `CEILING_KERNEL` is built by unconditionally including *every* `KNOWN_BASES` entry (a pre-existing, unrelated-to-this-plan pattern — Kernel is the fully-trusted local-operator tier and never receives an inbound channel connection, so this is accepted as harmless, not fought). The three tiers that actually govern remote channels (`SemiTrusted`, which Telegram/Discord/Slack all hardcode) and the escalated `Trusted` tier never grant it. Actual enforcement is the new daemon-side per-channel config check (above), not `CapabilitySet::grants()` — this scope exists in `KNOWN_BASES` purely for audit-trail/drift-guard consistency with every other capability surface in this codebase, matching its own established convention that every gated action names a real, known scope.
- **Rate limiting is enforced client-side (in the channel-adapter process), not by the daemon.** Deliberate, and different from the capability check above: rate-limiting exists to stop an *already-authorized* channel's own chat users from spamming `/team run`, not to stop an unauthorized channel from acting at all — a UX/abuse-prevention concern, not the security boundary. The operator's own channel-adapter process is trusted software they deployed (same trust level as its config file), so no server round-trip is needed to enforce it.
- **Confirm-first state is client-side, per-chat, in-memory, non-persistent.** No existing pending-confirmation precedent was found anywhere in this codebase (re-verified via repo-wide grep) — this plan introduces the pattern fresh, kept deliberately simple (a single `Option<PendingTrigger>` local to each chat's own daemon-frontend loop, a fixed 5-minute TTL, no cross-restart durability, matching the design's own "short-lived... in-memory or store-backed — implementation-plan decision" framing).
- **All 3 channels in the same pass** (Telegram, Discord, Slack) — matching Piece B's own approved scope decision, still applicable here.

---

## File Structure

- **Modify** `crates/aivyx-capability/src/lib.rs` — add `"team.run.channel"` to `KNOWN_BASES`.
- **Modify** `crates/aivyx-config/src/lib.rs` — `RawTelegram`/`RawDiscord`/`RawSlack` and `TelegramConfig`/`DiscordConfig`/`SlackConfig` each gain `team_run_channel: bool` + `team_trigger_rate_limit: Option<u32>`.
- **Modify** `crates/aivyx-ipc/src/protocol.rs` — new `FrontendMessage::RunTeamMissionChannel`, `DaemonMessage::TeamMissionChannelStarted`, `DaemonEnvelope::TeamMissionChannelStarted`.
- **Modify** `crates/aivyx-audit/src/lib.rs` — new `AuditEvent::TeamMissionChannelTriggered`.
- **Modify** `crates/aivyx-channel/src/team_mission_driver.rs` — new `register_mission_for_channel_trigger` / `start_from_goal_for_channel_trigger` (additive siblings, mirroring Piece A's `register_mission_for_schedule` / `start_from_goal_for_schedule`).
- **Modify** `crates/aivyx-channel/src/daemon_server.rs` — new `ChannelTriggerAuthz` struct + `ConnectionContext` field + new `FrontendMessage::RunTeamMissionChannel` match arm (the real enforcement point).
- **Modify** `crates/aivyx-channel/src/daemon_client.rs` — new one-shot `run_team_mission_channel` free function (identity-declaring, unlike the other six).
- **Modify** `crates/aivyx-channel/src/team_command.rs` — new `TeamCommand::Run { goal: String }` variant + parse support.
- **Create** `crates/aivyx-channel/src/team_trigger_state.rs` — `PendingTrigger`, `parse_confirm_reply`, `check_and_record_trigger` (pure, testable client-side state helpers).
- **Modify** `crates/aivyx-channel/src/team_dispatch.rs` — defensive fallback arm for `TeamCommand::Run` (should never actually be reached — see Task 10).
- **Modify** `crates/aivyx-channel/src/telegram_daemon_frontend.rs` / `discord_daemon_frontend.rs` / `slack_daemon_frontend.rs` — the confirm-first + rate-limit wiring, one per channel.
- **Modify** `crates/aivyx-cli/src/bin/aivyx.rs` — thread the two new `TelegramConfig`/`DiscordConfig`/`SlackConfig` fields into the three `run_*_daemon_multi_session` call sites.

---

### Task 1: `team.run.channel` capability base

**Files:**
- Modify: `crates/aivyx-capability/src/lib.rs:35-...` (the `KNOWN_BASES` array — insert alongside the existing `"team.delegate"` / `"team.message"` / `"team.run"` entries)

**Interfaces:**
- Produces: `"team.run.channel"` as a valid `Scope::parse`-able base string, present in `KNOWN_BASES`, absent from `CEILING_TRUSTED`/`CEILING_SEMITRUSTED`/`CEILING_UNTRUSTED` (present only in `CEILING_KERNEL`, which unconditionally includes every `KNOWN_BASES` entry — pre-existing, unrelated behavior).

- [ ] **Step 1: Write the failing test**

Add to `crates/aivyx-capability/src/lib.rs`'s existing `mod tests` (the same module the `team.run` tests at line ~1992 live in):

```rust
#[test]
fn team_run_channel_is_known_but_granted_by_no_real_tier() {
    let s = |x: &str| Scope::parse(x).unwrap();
    assert!(KNOWN_BASES.contains(&"team.run.channel"));
    assert!(!TrustTier::SemiTrusted
        .default_ceiling()
        .grants(&s("team.run.channel")));
    assert!(!TrustTier::Trusted
        .default_ceiling()
        .grants(&s("team.run.channel")));
    assert!(!TrustTier::Untrusted
        .default_ceiling()
        .grants(&s("team.run.channel")));
    // Kernel grants every KNOWN_BASES entry unconditionally — pre-existing,
    // unrelated behavior this base does not special-case around.
    assert!(TrustTier::Kernel
        .default_ceiling()
        .grants(&s("team.run.channel")));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p aivyx-capability team_run_channel_is_known_but_granted_by_no_real_tier -- --test-threads=1`
Expected: FAIL — `Scope::parse("team.run.channel")` returns `None` (`.unwrap()` panics), since the base doesn't exist yet.

- [ ] **Step 3: Add the base**

In `crates/aivyx-capability/src/lib.rs`, find the `KNOWN_BASES` array's team section (around line 461-474, the `"team.delegate"` / `"team.message"` / `"team.run"` entries) and add a new entry immediately after `"team.run"`:

```rust
    "team.run",
    // team.run.channel — Chapter (Piece C, 2026-08-23). Narrow,
    // channel-only sibling to team.run: starts a new Nonagon team
    // mission from a chat command (`/team run <goal>`), never from
    // the model. Deliberately absent from every real trust-tier
    // ceiling (Trusted included) — this base is never granted via
    // the normal CapabilitySet/TrustTier intersection at all.
    // Authorization is a bespoke, daemon-side, per-channel-type
    // config check (see `daemon_server.rs`'s `ChannelTriggerAuthz`),
    // since the trust-tier ceiling mechanism has no per-channel-type
    // granularity to hang this on (every channel type hardcodes the
    // same SemiTrusted tier). Present in KNOWN_BASES purely for
    // audit-trail/drift-guard consistency with every other gated
    // capability surface in this codebase.
    "team.run.channel",
```

Do **not** add `"team.run.channel"` to `CEILING_TRUSTED`, `CEILING_SEMITRUSTED`, or `CEILING_UNTRUSTED`'s own `CapabilitySet::from_scopes([...])` construction — leave those three unchanged.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p aivyx-capability team_run_channel_is_known_but_granted_by_no_real_tier -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Run the crate's full suite (drift-guard tests touch `KNOWN_BASES`)**

Run: `cargo test -p aivyx-capability -- --test-threads=1`
Expected: PASS, no regressions — in particular the existing `every_known_base_is_documented`-style drift-guard tests (search the crate's own test module if a failure appears here; they assert every `KNOWN_BASES` entry has a matching doc reference, so if one fails, add `team.run.channel` to whatever list it's checking against, following that test's own existing pattern for the other `team.*` bases).

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-capability/src/lib.rs
git commit -m "feat(capability): add team.run.channel base, granted by no real trust tier (Piece C Task 1)"
```

---

### Task 2: Per-channel `team_run_channel` + `team_trigger_rate_limit` config

**Files:**
- Modify: `crates/aivyx-config/src/lib.rs` — `RawTelegram` (~line 4556), `RawDiscord` (~line 4568), `RawSlack` (~line 4579); `TelegramConfig` (~line 1577), `DiscordConfig` (~line 1590), `SlackConfig` (~line 1626); the conversion site building each `Some(TelegramConfig {...})` etc. (~line 5956 for Telegram; Discord/Slack conversions follow immediately after in the same function)

**Interfaces:**
- Produces: `TelegramConfig.team_run_channel: bool`, `TelegramConfig.team_trigger_rate_limit: Option<u32>` (and the identical two fields on `DiscordConfig`/`SlackConfig`). `None`/absent in TOML → `team_run_channel: false`, `team_trigger_rate_limit: None` (unlimited). Consumed by Task 6 (daemon-side authorization) and Tasks 11-13 (channel-adapter-side rate limiting + fail-fast UX).

- [ ] **Step 1: Write the failing tests**

Add to `crates/aivyx-config/src/lib.rs`'s existing test module (search for an existing Telegram-config-loading test, e.g. one asserting `chat_filter` round-trips from TOML, and place these alongside it):

```rust
#[test]
fn telegram_team_run_channel_defaults_to_false_and_unset_rate_limit() {
    let toml = r#"
        [telegram]
        token = "t"
    "#;
    let cfg = load_config_from_str(toml).expect("load");
    let tg = cfg.telegram.expect("telegram section present");
    assert!(!tg.team_run_channel);
    assert_eq!(tg.team_trigger_rate_limit, None);
}

#[test]
fn telegram_team_run_channel_and_rate_limit_round_trip_from_toml() {
    let toml = r#"
        [telegram]
        token = "t"
        team_run_channel = true
        team_trigger_rate_limit = 5
    "#;
    let cfg = load_config_from_str(toml).expect("load");
    let tg = cfg.telegram.expect("telegram section present");
    assert!(tg.team_run_channel);
    assert_eq!(tg.team_trigger_rate_limit, Some(5));
}

#[test]
fn discord_and_slack_team_run_channel_round_trip_from_toml() {
    let toml = r#"
        [discord]
        token = "t"
        team_run_channel = true
        team_trigger_rate_limit = 3

        [slack]
        bot_token = "b"
        app_token = "a"
        team_run_channel = true
        team_trigger_rate_limit = 7
    "#;
    let cfg = load_config_from_str(toml).expect("load");
    let discord = cfg.discord.expect("discord section present");
    assert!(discord.team_run_channel);
    assert_eq!(discord.team_trigger_rate_limit, Some(3));
    let slack = cfg.slack.expect("slack section present");
    assert!(slack.team_run_channel);
    assert_eq!(slack.team_trigger_rate_limit, Some(7));
}
```

If this crate's existing tests use a different loader entry-point name than `load_config_from_str` (check an existing nearby Telegram-section test in the same file first and copy its exact loader call), use that exact function instead — do not invent a new one.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p aivyx-config team_run_channel -- --test-threads=1`
Expected: FAIL to compile — `team_run_channel`/`team_trigger_rate_limit` are not fields on `TelegramConfig`/`DiscordConfig`/`SlackConfig` yet.

- [ ] **Step 3: Add the raw TOML fields**

In `crates/aivyx-config/src/lib.rs`, modify `RawTelegram`:

```rust
#[derive(Debug, Default, Deserialize)]
struct RawTelegram {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    chat_id: Option<i64>,
    /// Piece C (2026-08-23) — operator opt-in for `/team run <goal>`
    /// from this channel. Absent/false: the command is recognized but
    /// always replies with a capability-denial message, both client-
    /// side (fail-fast UX) and server-side (the real enforcement).
    #[serde(default)]
    team_run_channel: bool,
    /// Piece C — max `/team run` confirmations accepted per rolling
    /// hour from this channel (client-side enforced). `None` =
    /// unlimited.
    #[serde(default)]
    team_trigger_rate_limit: Option<u32>,
}
```

Apply the identical two fields (same doc comments, same `#[serde(default)]`) to `RawDiscord` and `RawSlack`.

- [ ] **Step 4: Add the public config fields**

Modify `TelegramConfig`:

```rust
pub struct TelegramConfig {
    pub token: Option<SourcedSecret>,
    pub chat_filter: Option<Sourced<i64>>,
    /// Piece C (2026-08-23) — operator opt-in for `/team run <goal>`
    /// from this channel. No env-var override (TOML-only, matching
    /// how narrow this knob is) so it stays a plain `bool`, not
    /// `Sourced`-wrapped like `chat_filter`.
    pub team_run_channel: bool,
    /// Piece C — max `/team run` confirmations per rolling hour from
    /// this channel. `None` = unlimited.
    pub team_trigger_rate_limit: Option<u32>,
}
```

Apply the identical two fields (same doc comments) to `DiscordConfig` and `SlackConfig`.

- [ ] **Step 5: Wire the conversion site**

In the function that builds `Some(TelegramConfig { token: telegram_token, chat_filter: telegram_chat_filter })` (~line 5956), change to:

```rust
        let telegram = if telegram_token.is_some() || telegram_chat_filter.is_some() {
            Some(TelegramConfig {
                token: telegram_token,
                chat_filter: telegram_chat_filter,
                team_run_channel: toml.telegram.team_run_channel,
                team_trigger_rate_limit: toml.telegram.team_trigger_rate_limit,
            })
        } else {
            None
        };
```

Apply the identical two added lines (`team_run_channel: toml.discord.team_run_channel, team_trigger_rate_limit: toml.discord.team_trigger_rate_limit,` and the Slack equivalent) to the Discord and Slack conversion sites immediately following in the same function — read them first to match each site's exact existing field-list style before editing.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p aivyx-config team_run_channel -- --test-threads=1`
Expected: PASS, all 3 new tests.

- [ ] **Step 7: Run the crate's full suite**

Run: `cargo test -p aivyx-config -- --test-threads=1`
Expected: PASS, no regressions (in particular, no other code constructs `TelegramConfig`/`DiscordConfig`/`SlackConfig` via struct-literal elsewhere that would now fail to compile from the two new required fields — if the build fails elsewhere, grep the whole workspace for `TelegramConfig {`/`DiscordConfig {`/`SlackConfig {` struct-literal construction sites outside this file and add the two new fields there too, following this task's own field values).

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-config/src/lib.rs
git commit -m "feat(config): add per-channel team_run_channel + team_trigger_rate_limit knobs (Piece C Task 2)"
```

---

### Task 3: New IPC protocol messages

**Files:**
- Modify: `crates/aivyx-ipc/src/protocol.rs` — `FrontendMessage` enum (~line 1773, alongside `Query`), `DaemonMessage` enum (~line 2083), `DaemonEnvelope` enum (~line 2516)

**Interfaces:**
- Produces: `FrontendMessage::RunTeamMissionChannel { goal: String }`, `DaemonMessage::TeamMissionChannelStarted { mission_id: String }`, `DaemonEnvelope::TeamMissionChannelStarted { mission_id: String }` (byte-compatible with each other over the wire — both variants must serialize identically since `DaemonEnvelope` decodes whatever `DaemonMessage` wrote). Consumed by Task 6 (daemon-side handler, writes `DaemonMessage`) and Task 7 (client-side one-shot function, reads `DaemonEnvelope`).

- [ ] **Step 1: Write the failing test**

Add to `crates/aivyx-ipc/src/protocol.rs`'s existing test module (find the existing round-trip serialization tests for `FrontendMessage::Query`/`DaemonEnvelope::QueryResponse` and place this alongside them, matching their exact style):

```rust
#[test]
fn run_team_mission_channel_round_trips_through_frontend_message() {
    let msg = FrontendMessage::RunTeamMissionChannel {
        goal: "close the books".to_string(),
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    let back: FrontendMessage = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(msg, back);
}

#[test]
fn team_mission_channel_started_round_trips_daemon_message_to_envelope() {
    // DaemonMessage (what the daemon writes) must decode as the
    // matching DaemonEnvelope variant (what the client reads) — the
    // same cross-type compatibility every other daemon->client
    // message in this protocol already relies on.
    let msg = DaemonMessage::TeamMissionChannelStarted {
        mission_id: "m-1".to_string(),
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    let envelope: DaemonEnvelope = serde_json::from_str(&json).expect("deserialize as envelope");
    assert_eq!(
        envelope,
        DaemonEnvelope::TeamMissionChannelStarted {
            mission_id: "m-1".to_string()
        }
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p aivyx-ipc run_team_mission_channel team_mission_channel_started -- --test-threads=1`
Expected: FAIL to compile — none of the three new variants exist yet.

- [ ] **Step 3: Add `FrontendMessage::RunTeamMissionChannel`**

In `crates/aivyx-ipc/src/protocol.rs`, immediately after the `Query { id: String, payload: QueryPayload }` variant (~line 1776) in the `FrontendMessage` enum, add:

```rust
    /// Piece C (2026-08-23) — start a new Nonagon team mission from a
    /// channel's native `/team run <goal>` command. Unlike `Query`,
    /// this is only ever sent over a connection that has already sent
    /// `StartSession` with a real `frontend_type` — the daemon's own
    /// authorization check (Chapter — `ChannelTriggerAuthz` in
    /// `daemon_server.rs`) depends on knowing which channel is asking,
    /// which the anonymous one-shot `Query` path cannot provide.
    /// Responds with [`DaemonMessage::TeamMissionChannelStarted`] or
    /// `Error` (capability-denied, no team-mission service configured,
    /// or a decomposition/start failure).
    RunTeamMissionChannel {
        goal: String,
    },
```

- [ ] **Step 4: Add `DaemonMessage::TeamMissionChannelStarted`**

In the same file's `DaemonMessage` enum (~line 2083), add a new variant (place it near `GateResolved`/`TeamMissionUpdated`, matching the enum's own existing team-mission-adjacent grouping):

```rust
    /// Piece C — response to [`FrontendMessage::RunTeamMissionChannel`]
    /// on success. The new mission's id; the drive runs in the
    /// background (poll via the existing `QueryPayload::
    /// TeamMissionStatus`, same as every other team-mission start
    /// path).
    TeamMissionChannelStarted {
        mission_id: String,
    },
```

- [ ] **Step 5: Add `DaemonEnvelope::TeamMissionChannelStarted`**

In the same file's `DaemonEnvelope` enum (~line 2516), add the identical variant (same field, same name — this enum "mirrors" `DaemonMessage`'s variants per its own doc comment):

```rust
    /// Piece C — mirrors `DaemonMessage::TeamMissionChannelStarted`.
    TeamMissionChannelStarted {
        mission_id: String,
    },
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p aivyx-ipc run_team_mission_channel team_mission_channel_started -- --test-threads=1`
Expected: PASS.

- [ ] **Step 7: Run the crate's full suite**

Run: `cargo test -p aivyx-ipc -- --test-threads=1`
Expected: PASS, no regressions.

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-ipc/src/protocol.rs
git commit -m "feat(ipc): add RunTeamMissionChannel protocol messages (Piece C Task 3)"
```

---

### Task 4: New audit event

**Files:**
- Modify: `crates/aivyx-audit/src/lib.rs` — `AuditEvent` enum (~line 68, alongside `TeamMission`/`Trigger` at ~line 459-468)

**Interfaces:**
- Produces: `AuditEvent::TeamMissionChannelTriggered { platform: String, goal: String, mission_id: String }`. Consumed by Task 6 (daemon-side handler logs it on a successful channel-triggered mission start).

- [ ] **Step 1: Write the failing test**

Add to `crates/aivyx-audit/src/lib.rs`'s existing test module (find an existing round-trip/serialize test for `AuditEvent::TeamMission` or `AuditEvent::Trigger` and mirror its exact style):

```rust
#[test]
fn team_mission_channel_triggered_round_trips() {
    let event = AuditEvent::TeamMissionChannelTriggered {
        platform: "telegram".to_string(),
        goal: "close the books".to_string(),
        mission_id: "m-1".to_string(),
    };
    let json = serde_json::to_string(&event).expect("serialize");
    let back: AuditEvent = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(event, back);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p aivyx-audit team_mission_channel_triggered_round_trips -- --test-threads=1`
Expected: FAIL to compile — the variant doesn't exist.

- [ ] **Step 3: Add the variant**

In `crates/aivyx-audit/src/lib.rs`'s `AuditEvent` enum, immediately after the existing `Trigger { trigger_kind: TriggerKindSummary }` variant (~line 465-468), add:

```rust
    /// Piece C (2026-08-23) — a channel's native `/team run <goal>`
    /// command successfully started a new team mission. Distinct from
    /// `Trigger` (that variant is specifically for *refused* headless
    /// trigger runs) and from `TeamMission` (that variant is a gate
    /// event on an already-running mission) — this is the one-time
    /// "a channel started a brand-new mission" audit record.
    TeamMissionChannelTriggered {
        /// The originating channel platform (`"telegram"` /
        /// `"discord"` / `"slack"`), lower-case, matching
        /// `daemon_server.rs`'s own platform-tag convention.
        platform: String,
        goal: String,
        mission_id: String,
    },
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p aivyx-audit team_mission_channel_triggered_round_trips -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Run the crate's full suite**

Run: `cargo test -p aivyx-audit -- --test-threads=1`
Expected: PASS, no regressions. If a drift-guard/exhaustiveness test in this crate fails (some audit crates assert every `AuditEvent` variant appears in a docs table or a `match` elsewhere), follow that test's own existing pattern for the other `Team*`/`Trigger` variants to fix it.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-audit/src/lib.rs
git commit -m "feat(audit): add TeamMissionChannelTriggered event (Piece C Task 4)"
```

---

### Task 5: `TeamMissionService::start_from_goal_for_channel_trigger`

**Files:**
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs` — `register_mission` (~line 497) and `start_from_goal`/`start_from_goal_for_schedule` (~line 1107-1150)

**Interfaces:**
- Consumes: nothing from earlier tasks in this crate (mirrors Piece A's own already-shipped `register_mission_for_schedule`/`start_from_goal_for_schedule` shape exactly, substituting a generic `trigger_tag: &str` for `schedule_id: &str`).
- Produces: `pub async fn register_mission_for_channel_trigger(shared: &SharedMissionState, plan: MissionPlan, id: impl Into<String>, config: Option<TeamConfig>, trigger_tag: &str) -> Result<String, MissionDriverError>` and `pub async fn start_from_goal_for_channel_trigger(&self, goal: &str, config: Option<TeamConfig>, trigger_tag: &str) -> Result<String, MissionDriverError>` on `TeamMissionService`. Consumed by Task 6.

- [ ] **Step 1: Write the failing tests**

Find this file's existing tests for `register_mission_for_schedule`/`start_from_goal_for_schedule` (they exist per Piece A — search `mod tests` for `schedule` in the test names) and add these alongside them, mirroring their exact fixture setup:

```rust
#[tokio::test]
async fn register_mission_for_channel_trigger_tags_triggered_by() {
    let shared = test_shared_mission_state(); // use this file's own existing test fixture helper — copy its exact name/signature from the register_mission_for_schedule test above
    let plan = test_plan(); // likewise — reuse this file's existing plan-fixture helper
    let id = register_mission_for_channel_trigger(
        &shared,
        plan,
        "m-channel-1",
        None,
        "channel:telegram",
    )
    .await
    .expect("register");
    let record = shared.get(&id).await.expect("get").expect("present");
    assert_eq!(record.triggered_by.as_deref(), Some("channel:telegram"));
}
```

Read the actual current test module first to get the exact fixture helper names (`test_shared_mission_state`/`test_plan` above are placeholders for whatever this file's own `register_mission_for_schedule` test already uses — copy those real names verbatim, do not invent new fixtures).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p aivyx-channel register_mission_for_channel_trigger -- --test-threads=1`
Expected: FAIL to compile — the function doesn't exist yet.

- [ ] **Step 3: Add `register_mission_for_channel_trigger`**

Immediately after the existing `register_mission_for_schedule` function (~line 536), add:

```rust
/// Chapter (Piece C) — like [`register_mission`], but tags the
/// resulting record with the channel that started it. A separate
/// function rather than generalizing `register_mission_for_schedule`'s
/// own `schedule_id` parameter, for the same reason that function gave
/// for not touching `register_mission` itself: avoid disturbing an
/// already-shipped, tested call site.
pub async fn register_mission_for_channel_trigger(
    shared: &SharedMissionState,
    plan: MissionPlan,
    id: impl Into<String>,
    config: Option<TeamConfig>,
    trigger_tag: &str,
) -> Result<String, MissionDriverError> {
    let id = id.into();
    plan.validate()?;
    let goal = plan.goal.clone();
    shared
        .put(
            TeamMissionRecord::new(&id, goal, plan)
                .with_config(config)
                .with_triggered_by(trigger_tag),
        )
        .await?;
    Ok(id)
}
```

- [ ] **Step 4: Add `start_from_goal_for_channel_trigger`**

Immediately after the existing `start_from_goal_for_schedule` method (~line 1150), add:

```rust
    /// Chapter (Piece C) — like [`start_from_goal`], but the resulting
    /// mission is tagged with the channel that started it
    /// (`triggered_by`). `config` is always `None` from every real
    /// call site in this codebase (see the Piece C plan's own Global
    /// Constraints) — the parameter is kept for shape-parity with
    /// `start_from_goal_for_schedule` and to avoid a signature that
    /// silently forecloses a future, deliberately-designed pack-
    /// selection feature, not because any caller passes `Some`.
    pub async fn start_from_goal_for_channel_trigger(
        &self,
        goal: &str,
        config: Option<TeamConfig>,
        trigger_tag: &str,
    ) -> Result<String, MissionDriverError> {
        let cancel = aivyx_core::CancellationToken::new();
        let plan = aivyx_team::decompose_goal(
            self.deps.provider.as_ref(),
            &self.deps.model,
            goal,
            config.as_ref().unwrap_or(&self.config),
            &cancel,
            true,
        )
        .await?;
        let id = register_mission_for_channel_trigger(
            &self.state,
            plan,
            uuid::Uuid::new_v4().to_string(),
            config,
            trigger_tag,
        )
        .await?;
        self.spawn_drive(id.clone());
        Ok(id)
    }
```

This mirrors `start_from_goal_for_schedule`'s real, complete tail exactly (verified directly, not reconstructed): `self.spawn_drive(id.clone())` — **not awaited**, matching "the drive runs in the background" — is the real call, not `drive_registered(...).await` (a different method entirely, meant to be spawned by *its own* caller per its own doc comment, not called+awaited inline).

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p aivyx-channel register_mission_for_channel_trigger -- --test-threads=1`
Expected: PASS.

- [ ] **Step 6: Run the crate's full suite**

Run: `cargo test -p aivyx-channel --lib -- --test-threads=1`
Expected: PASS, no regressions.

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-channel/src/team_mission_driver.rs
git commit -m "feat(team): add register/start_from_goal_for_channel_trigger siblings (Piece C Task 5)"
```

---

### Task 6: Daemon-side authorization + handler (the real enforcement point)

**Files:**
- Modify: `crates/aivyx-channel/src/daemon_server.rs` — `ConnectionContext` struct (~line 1575), its construction site(s), and `handle_connection`'s big `match` (~line 2346, alongside `CancelTurn`/`ResolveGate`)

**Interfaces:**
- Consumes: Task 1's `"team.run.channel"` scope (documentation-only reference, not functionally checked here), Task 2's `TelegramConfig.team_run_channel`/`DiscordConfig.team_run_channel`/`SlackConfig.team_run_channel`, Task 3's `FrontendMessage::RunTeamMissionChannel`/`DaemonMessage::TeamMissionChannelStarted`, Task 4's `AuditEvent::TeamMissionChannelTriggered`, Task 5's `TeamMissionService::start_from_goal_for_channel_trigger`.
- Produces: `pub struct ChannelTriggerAuthz { pub telegram: bool, pub discord: bool, pub slack: bool }`, `pub fn channel_trigger_authorized(...)`, `pub fn channel_trigger_tag(...)`, and `async fn handle_run_team_mission_channel(svc: Option<&TeamMissionService>, authz: &ChannelTriggerAuthz, platform: Option<ChannelPlatform>, audit_log: Option<&PersistentAuditLog>, goal: String) -> DaemonMessage` — all in `daemon_server.rs`. Consumed by Tasks 11-13 only indirectly (via the wire protocol, not by importing these directly — they're daemon-internal).

**Testability note (why this task extracts a helper function):** re-verified during planning that `handle_connection` (the function this match arm lives in) has **no existing test precedent anywhere in this file** — every `UnixListener::bind` call in `daemon_server.rs` is real production code (`run_daemon`'s own socket setup), not a test fixture; unlike Task 7's `daemon_client.rs`, which already has a fake-daemon-over-`UnixListener` test pattern to mirror, there is nothing equivalent on the *server* side to reuse or extend cheaply. Rather than leave the actual security-critical decision (deny vs. allow vs. start) untested — the exact class of gap this whole piece exists to close, per this session's own established caution about "a claimed check that isn't really enforced" — this task extracts the decision logic into `handle_run_team_mission_channel`, a plain `async fn` callable directly from a test with no `UnixStream`/`ConnectionContext` needed at all. The match arm inside `handle_connection` itself stays a thin, untested wrapper (read the message → call the helper → write the response), the same shape every other arm in this match already has.

- [ ] **Step 1: Write the failing tests**

Add to `crates/aivyx-channel/src/daemon_server.rs`'s existing test module:

```rust
#[test]
fn channel_trigger_authorized_checks_the_right_platform_flag() {
    let authz = ChannelTriggerAuthz {
        telegram: true,
        discord: false,
        slack: false,
    };
    assert!(channel_trigger_authorized(
        &authz,
        Some(aivyx_core::ChannelPlatform::Telegram)
    ));
    assert!(!channel_trigger_authorized(
        &authz,
        Some(aivyx_core::ChannelPlatform::Discord)
    ));
    assert!(!channel_trigger_authorized(
        &authz,
        Some(aivyx_core::ChannelPlatform::Slack)
    ));
}

#[test]
fn channel_trigger_authorized_denies_unknown_or_absent_platform() {
    let authz = ChannelTriggerAuthz {
        telegram: true,
        discord: true,
        slack: true,
    };
    // No StartSession yet, or a platform this feature was never
    // designed for (Local/Rest/Voice/...) — always denied, never
    // fail-open.
    assert!(!channel_trigger_authorized(&authz, None));
    assert!(!channel_trigger_authorized(
        &authz,
        Some(aivyx_core::ChannelPlatform::Local)
    ));
}

#[test]
fn channel_trigger_tag_names_the_platform() {
    assert_eq!(
        channel_trigger_tag(Some(aivyx_core::ChannelPlatform::Telegram)),
        "channel:telegram"
    );
    assert_eq!(
        channel_trigger_tag(Some(aivyx_core::ChannelPlatform::Discord)),
        "channel:discord"
    );
    assert_eq!(
        channel_trigger_tag(Some(aivyx_core::ChannelPlatform::Slack)),
        "channel:slack"
    );
    assert_eq!(channel_trigger_tag(None), "channel:unknown");
}

#[tokio::test]
async fn handle_run_team_mission_channel_denies_before_ever_checking_the_service() {
    // Deliberately checks authorization BEFORE service-presence: an
    // unauthorized channel gets denied even if the daemon has no
    // TeamMissionService at all — `svc: None` here proves the
    // authorization branch never touches `svc`, so this test needs no
    // TeamMissionService fixture (there is no existing lightweight one
    // in this file to build from).
    let authz = ChannelTriggerAuthz::default(); // all false
    let resp = handle_run_team_mission_channel(
        None,
        &authz,
        Some(aivyx_core::ChannelPlatform::Telegram),
        None,
        "close the books".to_string(),
    )
    .await;
    match resp {
        DaemonMessage::Error { code, .. } => assert_eq!(code, "team_run_channel_denied"),
        other => panic!("expected Error(team_run_channel_denied), got {other:?}"),
    }
}

#[tokio::test]
async fn handle_run_team_mission_channel_reports_no_service_when_authorized_but_absent() {
    let authz = ChannelTriggerAuthz {
        telegram: true,
        discord: false,
        slack: false,
    };
    let resp = handle_run_team_mission_channel(
        None,
        &authz,
        Some(aivyx_core::ChannelPlatform::Telegram),
        None,
        "close the books".to_string(),
    )
    .await;
    match resp {
        DaemonMessage::Error { code, .. } => assert_eq!(code, "no_team_missions"),
        other => panic!("expected Error(no_team_missions), got {other:?}"),
    }
}
```

Note what this task's tests deliberately do **not** cover: the success path (`authorized: true` + a real `Some(&TeamMissionService)`) needs a live `TeamMissionService`, which needs a real or scripted LLM provider to actually decompose a goal — the same class of dependency `start_from_goal`/`start_from_goal_for_schedule` already carry in this file's own (pre-existing, Piece-A-era) tests. This task does not attempt to build that fixture from scratch; the success path is verified by this plan's own Final Verification end-to-end trace instead, matching this lineage's accepted pattern for code that genuinely needs a live LLM to exercise fully.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p aivyx-channel channel_trigger_authorized channel_trigger_tag handle_run_team_mission_channel -- --test-threads=1`
Expected: FAIL to compile — none of `ChannelTriggerAuthz`/`channel_trigger_authorized`/`channel_trigger_tag`/`handle_run_team_mission_channel` exist yet.

- [ ] **Step 3: Add the struct, pure helpers, and the extracted decision function**

Add near the top of `daemon_server.rs` (alongside other small daemon-wide types, or immediately before `ConnectionContext`):

```rust
/// Piece C (2026-08-23) — the daemon's own, independently-loaded
/// per-channel-type authorization for `/team run <goal>`. Built once
/// at daemon startup from the same `aivyx.toml` every process reads
/// (see the construction site below) — deliberately *not* trusting
/// anything the connecting channel-adapter process claims about its
/// own authorization, since that process is a separate, potentially
/// stale or misconfigured copy of the same config.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChannelTriggerAuthz {
    pub telegram: bool,
    pub discord: bool,
    pub slack: bool,
}

/// Pure: does `authz` grant `platform` the right to start a new team
/// mission via `/team run`? `None` (no `StartSession` yet) or any
/// platform this feature doesn't recognize (Local/Rest/Voice/Email/
/// Matrix) is always denied — fail-closed, never fail-open on an
/// unrecognized or absent identity.
pub fn channel_trigger_authorized(
    authz: &ChannelTriggerAuthz,
    platform: Option<aivyx_core::ChannelPlatform>,
) -> bool {
    match platform {
        Some(aivyx_core::ChannelPlatform::Telegram) => authz.telegram,
        Some(aivyx_core::ChannelPlatform::Discord) => authz.discord,
        Some(aivyx_core::ChannelPlatform::Slack) => authz.slack,
        _ => false,
    }
}

/// Pure: the `triggered_by` tag a channel-started mission's record
/// carries, and the `platform` field the audit event logs.
pub fn channel_trigger_tag(platform: Option<aivyx_core::ChannelPlatform>) -> String {
    match platform {
        Some(aivyx_core::ChannelPlatform::Telegram) => "channel:telegram".to_string(),
        Some(aivyx_core::ChannelPlatform::Discord) => "channel:discord".to_string(),
        Some(aivyx_core::ChannelPlatform::Slack) => "channel:slack".to_string(),
        _ => "channel:unknown".to_string(),
    }
}

/// Piece C — the real authorization + start decision for
/// `FrontendMessage::RunTeamMissionChannel`, extracted from the raw
/// wire-protocol read/write glue in `handle_connection` specifically
/// so it's directly testable without a live `UnixStream`/
/// `ConnectionContext` (no precedent for that exists anywhere in this
/// file — see this task's own "Testability note" above). Checks
/// authorization *before* service-presence, deliberately: whether a
/// team-mission service even exists is irrelevant to an unauthorized
/// caller, and checking the cheaper, more restrictive gate first keeps
/// both branches independently testable with no service fixture
/// needed for the deny path.
async fn handle_run_team_mission_channel(
    svc: Option<&crate::team_mission_driver::TeamMissionService>,
    authz: &ChannelTriggerAuthz,
    platform: Option<aivyx_core::ChannelPlatform>,
    audit_log: Option<&PersistentAuditLog>,
    goal: String,
) -> DaemonMessage {
    if !channel_trigger_authorized(authz, platform) {
        return DaemonMessage::Error {
            code: "team_run_channel_denied".into(),
            message: "this channel is not authorized to start team missions (operator \
                      opt-in required via team_run_channel in aivyx.toml)"
                .into(),
        };
    }
    let Some(svc) = svc else {
        return DaemonMessage::Error {
            code: "no_team_missions".into(),
            message: "daemon has no team-mission service configured".into(),
        };
    };
    let tag = channel_trigger_tag(platform);
    match svc.start_from_goal_for_channel_trigger(&goal, None, &tag).await {
        Ok(mission_id) => {
            if let Some(log) = audit_log {
                use aivyx_audit::AuditWriter;
                if let Err(e) = log.append(aivyx_audit::AuditEvent::TeamMissionChannelTriggered {
                    platform: tag.clone(),
                    goal: goal.clone(),
                    mission_id: mission_id.clone(),
                }) {
                    eprintln!("aivyx daemon: failed to audit channel team trigger: {e}");
                }
            }
            DaemonMessage::TeamMissionChannelStarted { mission_id }
        }
        Err(e) => DaemonMessage::Error {
            code: "team_run_channel_failed".into(),
            message: e.to_string(),
        },
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p aivyx-channel channel_trigger_authorized channel_trigger_tag handle_run_team_mission_channel -- --test-threads=1`
Expected: PASS, all 5 tests.

- [ ] **Step 5: Thread `ChannelTriggerAuthz` into `ConnectionContext`**

Add a new field to the `ConnectionContext` struct (~line 1575, alongside `team_missions`/`gate_policy`):

```rust
    /// Piece C — per-channel-type authorization for `/team run`.
    channel_trigger_authz: ChannelTriggerAuthz,
```

Find every site that constructs a `ConnectionContext { ... }` literal (search the file for `ConnectionContext {`) and add `channel_trigger_authz,` to each (a bound local variable of the same name — see Step 6 for where it's built once and passed down). In test-fixture construction sites within this file's own `mod tests` (if any build a bare `ConnectionContext` directly rather than through `run_daemon`), pass `ChannelTriggerAuthz::default()` (all `false` — matches the struct's `#[derive(Default)]`).

- [ ] **Step 6: Build `ChannelTriggerAuthz` once at daemon startup**

Find `run_daemon`'s own construction of the pieces that feed `ConnectionContext` (search for where `team_missions: Option<crate::team_mission_driver::TeamMissionService>` or `loop_config: Option<aivyx_config::LoopConfig>` get built from the loaded `AivyxConfig`, and add this alongside them, before whichever loop/closure constructs each per-connection `ConnectionContext`):

```rust
    let channel_trigger_authz = ChannelTriggerAuthz {
        telegram: config
            .telegram
            .as_ref()
            .map(|t| t.team_run_channel)
            .unwrap_or(false),
        discord: config
            .discord
            .as_ref()
            .map(|d| d.team_run_channel)
            .unwrap_or(false),
        slack: config
            .slack
            .as_ref()
            .map(|s| s.team_run_channel)
            .unwrap_or(false),
    };
```

(`config` here stands for whatever the real local variable name is for the loaded `AivyxConfig`/`Settings` at that point in `run_daemon` — read the surrounding code first to use its real name, not a placeholder.) Ensure `channel_trigger_authz` is `Copy` (already derived) so it can be included directly in each per-connection `ConnectionContext { ..., channel_trigger_authz, ... }` literal without needing an `Arc`.

- [ ] **Step 7: Add the thin `RunTeamMissionChannel` match arm**

In `handle_connection`'s big match (immediately after the existing `FrontendMessage::ResolveGate { ... } => { ... }` arm, ~line 2362-2420+), add:

```rust
                        FrontendMessage::RunTeamMissionChannel { goal } => {
                            let platform = channel.as_ref().map(|c| c.platform());
                            let resp = handle_run_team_mission_channel(
                                team_missions.as_ref(),
                                &channel_trigger_authz,
                                platform,
                                audit_log.as_deref(),
                                goal,
                            )
                            .await;
                            let frame = encode_frame(&resp)?;
                            writer.write_all(&frame).await?;
                        }
```

Before finalizing this step, read the surrounding `ResolveGate` arm's own exact variable names in scope at this point (`writer`, `channel`, `audit_log`, `team_missions`) and confirm each matches — this plan's earlier research read `ConnectionContext`'s field list and the `ResolveGate` arm's body directly, but the implementer must re-confirm the exact bound-variable names at this specific point in the function (they're destructured from `ConnectionContext` near the top of `handle_connection`, not necessarily named identically to the struct's own field names). In particular confirm `team_missions`'s exact type at this point (`Option<TeamMissionService>` vs. `Option<&TeamMissionService>` vs. `Option<Arc<TeamMissionService>>`) and adjust `.as_ref()` / `.as_deref()` accordingly so `handle_run_team_mission_channel`'s first argument type-checks.

- [ ] **Step 8: Run the crate's full suite**

Run: `cargo test -p aivyx-channel --lib -- --test-threads=1`
Expected: PASS, no regressions. The match arm itself (Step 7) is intentionally *not* separately tested beyond this — it is now three lines of read/dispatch/write glue with no branching of its own, and the actual decision logic it calls (`handle_run_team_mission_channel`) is already directly tested in Step 4.

- [ ] **Step 9: Commit**

```bash
git add crates/aivyx-channel/src/daemon_server.rs
git commit -m "feat(channel): daemon-side team.run.channel authorization + handler (Piece C Task 6)"
```

---

### Task 7: `daemon_client::run_team_mission_channel` (identity-declaring one-shot call)

**Files:**
- Modify: `crates/aivyx-channel/src/daemon_client.rs` — new free function, placed alongside the existing six one-shot `team_mission_*` functions (~line 1216-1347)

**Interfaces:**
- Consumes: Task 3's `FrontendMessage::RunTeamMissionChannel`/`DaemonEnvelope::{SessionStarted, TeamMissionChannelStarted, Error, RecoveryNotice, DaemonReady}` (all pre-existing except the new `TeamMissionChannelStarted`), Task 6's daemon-side handler (end-to-end tested here).
- Produces: `pub async fn run_team_mission_channel(socket_path: &Path, frontend_type: aivyx_ipc::protocol::FrontendType, goal: String) -> Result<String, DaemonError>`. Consumed by Tasks 11-13.

- [ ] **Step 1: Write the failing tests**

Add to `crates/aivyx-channel/src/daemon_client.rs`'s existing `mod tests` (mirroring the file's own `connect_skips_recovery_notice_in_the_handshake` / `send_query_skips_recovery_notice_before_the_response` fake-daemon fixture pattern exactly):

```rust
#[tokio::test]
async fn run_team_mission_channel_does_the_start_session_handshake_then_sends_the_request() {
    let sock = std::env::temp_dir()
        .join(format!("aivyx-runteam-{}.sock", uuid::Uuid::new_v4()));
    let listener = UnixListener::bind(&sock).expect("bind fake daemon");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let ready = encode_frame(&DaemonEnvelope::DaemonReady {
            version: "0.1".into(),
        })
        .expect("encode ready");
        stream.write_all(&ready).await.expect("write ready");

        // Read the client's StartSession, then reply SessionStarted.
        let mut tmp = [0u8; 2048];
        let _ = stream.read(&mut tmp).await;
        let started = encode_frame(&DaemonEnvelope::SessionStarted {
            session_id: "sess-1".into(),
        })
        .expect("encode started");
        stream.write_all(&started).await.expect("write started");

        // Read the client's RunTeamMissionChannel, then reply success.
        let _ = stream.read(&mut tmp).await;
        let resp = encode_frame(&DaemonEnvelope::TeamMissionChannelStarted {
            mission_id: "m-1".into(),
        })
        .expect("encode resp");
        stream.write_all(&resp).await.expect("write resp");
        let _ = stream.read(&mut tmp).await;
    });

    let mission_id = run_team_mission_channel(
        &sock,
        FrontendType::Telegram,
        "close the books".to_string(),
    )
    .await
    .expect("run_team_mission_channel must succeed");
    assert_eq!(mission_id, "m-1");

    let _ = server.await;
    let _ = std::fs::remove_file(&sock);
}

#[tokio::test]
async fn run_team_mission_channel_surfaces_a_capability_denial_error() {
    let sock = std::env::temp_dir()
        .join(format!("aivyx-runteam-denied-{}.sock", uuid::Uuid::new_v4()));
    let listener = UnixListener::bind(&sock).expect("bind fake daemon");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let ready = encode_frame(&DaemonEnvelope::DaemonReady {
            version: "0.1".into(),
        })
        .expect("encode ready");
        stream.write_all(&ready).await.expect("write ready");
        let mut tmp = [0u8; 2048];
        let _ = stream.read(&mut tmp).await;
        let started = encode_frame(&DaemonEnvelope::SessionStarted {
            session_id: "sess-1".into(),
        })
        .expect("encode started");
        stream.write_all(&started).await.expect("write started");
        let _ = stream.read(&mut tmp).await;
        let err = encode_frame(&DaemonEnvelope::Error {
            code: "team_run_channel_denied".into(),
            message: "this channel is not authorized".into(),
        })
        .expect("encode err");
        stream.write_all(&err).await.expect("write err");
        let _ = stream.read(&mut tmp).await;
    });

    let result = run_team_mission_channel(
        &sock,
        FrontendType::Discord,
        "close the books".to_string(),
    )
    .await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("team_run_channel_denied"));

    let _ = server.await;
    let _ = std::fs::remove_file(&sock);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p aivyx-channel run_team_mission_channel -- --test-threads=1`
Expected: FAIL to compile — the function doesn't exist yet.

- [ ] **Step 3: Add the function**

Add near the existing six one-shot `team_mission_*` functions (~line 1216):

```rust
/// Piece C (2026-08-23) — start a new team mission from a channel's
/// `/team run <goal>` command. Unlike the other one-shot
/// `team_mission_*` functions above (which use the anonymous `Query`
/// path via `send_query`), this function does its own `StartSession`
/// handshake first, declaring `frontend_type` — the daemon's
/// authorization check needs to know which real channel is asking,
/// which the anonymous `Query` path cannot provide (see the Piece C
/// plan's Global Constraints for the full rationale).
pub async fn run_team_mission_channel(
    socket_path: &Path,
    frontend_type: FrontendType,
    goal: String,
) -> Result<String, DaemonError> {
    let stream = UnixStream::connect(socket_path).await?;
    let (mut reader, mut writer) = stream.into_split();
    let mut buf = Vec::with_capacity(4096);

    read_more(&mut reader, &mut buf).await?;
    match decode_frame::<DaemonEnvelope>(&buf) {
        Ok((DaemonEnvelope::DaemonReady { .. }, consumed)) => buf.drain(..consumed),
        Ok((other, _)) => {
            return Err(DaemonError::Protocol(format!(
                "expected DaemonReady, got {other:?}"
            )))
        }
        Err(e) => return Err(e.into()),
    };

    let start = FrontendMessage::StartSession {
        role: None,
        frontend_type: Some(frontend_type),
    };
    let frame = encode_frame(&start)?;
    writer.write_all(&frame).await?;

    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((DaemonEnvelope::SessionStarted { .. }, consumed)) => {
                buf.drain(..consumed);
                break;
            }
            Ok((DaemonEnvelope::RecoveryNotice { .. }, consumed)) => {
                buf.drain(..consumed);
            }
            Ok((other, _)) => {
                return Err(DaemonError::Protocol(format!(
                    "expected SessionStarted, got {other:?}"
                )))
            }
            Err(FrameError::IncompleteBuf) => read_more(&mut reader, &mut buf).await?,
            Err(e) => return Err(e.into()),
        }
    }

    let req = FrontendMessage::RunTeamMissionChannel { goal };
    let frame = encode_frame(&req)?;
    writer.write_all(&frame).await?;

    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((DaemonEnvelope::TeamMissionChannelStarted { mission_id }, _)) => {
                return Ok(mission_id)
            }
            Ok((DaemonEnvelope::Error { code, message }, _)) => {
                return Err(DaemonError::Protocol(format!("{code}: {message}")))
            }
            Ok((other, consumed)) => {
                buf.drain(..consumed);
                return Err(DaemonError::Protocol(format!(
                    "expected TeamMissionChannelStarted, got {other:?}"
                )));
            }
            Err(FrameError::IncompleteBuf) => read_more(&mut reader, &mut buf).await?,
            Err(e) => return Err(e.into()),
        }
    }
}
```

Confirm `FrameError` is already imported/in-scope in this file (it's used by the existing `resolve_gate` method) — if not, add `use crate::daemon_ipc::FrameError;` (or whatever its real import path is, matching how the existing code in this file already imports it).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p aivyx-channel run_team_mission_channel -- --test-threads=1`
Expected: PASS, both tests.

- [ ] **Step 5: Run the crate's full suite**

Run: `cargo test -p aivyx-channel --lib -- --test-threads=1`
Expected: PASS, no regressions.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-channel/src/daemon_client.rs
git commit -m "feat(channel): add identity-declaring run_team_mission_channel one-shot call (Piece C Task 7)"
```

---

### Task 8: `TeamCommand::Run` parser support

**Files:**
- Modify: `crates/aivyx-channel/src/team_command.rs`

**Interfaces:**
- Produces: `TeamCommand::Run { goal: String }` as a new enum variant; `parse` recognizes `/team run <goal...>` (multi-word goal, unlike every other variant's single-token ids).

- [ ] **Step 1: Write the failing tests**

Add to `team_command.rs`'s existing `mod tests`:

```rust
#[test]
fn parse_run_canonical_single_word_goal() {
    assert_eq!(
        parse("/team run close-the-books"),
        Some(TeamCommand::Run {
            goal: "close-the-books".to_string()
        })
    );
}

#[test]
fn parse_run_joins_multi_word_goal() {
    assert_eq!(
        parse("/team run close the books"),
        Some(TeamCommand::Run {
            goal: "close the books".to_string()
        })
    );
}

#[test]
fn parse_run_collapses_internal_multiple_spaces() {
    // split_whitespace collapses runs — matches this file's own
    // existing parse_accepts_multiple_spaces_between_tokens precedent.
    assert_eq!(
        parse("/team run close   the   books"),
        Some(TeamCommand::Run {
            goal: "close the books".to_string()
        })
    );
}

#[test]
fn parse_run_with_no_goal_is_usage() {
    assert_eq!(parse("/team run"), Some(TeamCommand::Usage));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p aivyx-channel --lib team_command:: -- --test-threads=1`
Expected: FAIL to compile — `TeamCommand::Run` doesn't exist yet.

- [ ] **Step 3: Add the variant and parse support**

Add to the `TeamCommand` enum:

```rust
    /// `/team run <goal>` — Piece C. `goal` is everything after `run`,
    /// re-joined with single spaces (multi-word goals are the norm;
    /// unlike every other variant's single-token ids).
    Run { goal: String },
```

In `parse`, add a check for `run` **before** the existing fixed-arity `match parts.as_slice() { ... }` block (since `run`'s goal has variable length, it can't be expressed as a slice pattern the way every other subcommand is):

```rust
pub fn parse(text: &str) -> Option<TeamCommand> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.first() != Some(&"/team") {
        return None;
    }
    if parts.get(1) == Some(&"run") {
        return if parts.len() >= 3 {
            Some(TeamCommand::Run {
                goal: parts[2..].join(" "),
            })
        } else {
            Some(TeamCommand::Usage)
        };
    }
    match parts.as_slice() {
        // ... existing arms unchanged ...
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p aivyx-channel --lib team_command:: -- --test-threads=1`
Expected: PASS, all tests (existing + new 4).

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-channel/src/team_command.rs
git commit -m "feat(channel): add TeamCommand::Run parser support for /team run <goal> (Piece C Task 8)"
```

---

### Task 9: `team_trigger_state.rs` — confirm-first + rate-limit helpers

**Files:**
- Create: `crates/aivyx-channel/src/team_trigger_state.rs`
- Modify: `crates/aivyx-channel/src/lib.rs` (register the module, immediately after the `pub mod team_dispatch;` line Piece B added)

**Interfaces:**
- Produces: `pub struct PendingTrigger { pub goal: String, pub created_at: Instant }`, `pub const PENDING_TRIGGER_TTL: Duration`, `impl PendingTrigger { pub fn new(goal: impl Into<String>) -> Self; pub fn is_expired(&self, now: Instant) -> bool }`, `pub enum ConfirmReply { Yes, No }`, `pub fn parse_confirm_reply(text: &str) -> Option<ConfirmReply>`, `pub fn check_and_record_trigger(history: &mut Vec<Instant>, limit: u32, now: Instant) -> bool`. Consumed by Tasks 11-13.

- [ ] **Step 1: Write the failing tests**

Create `crates/aivyx-channel/src/team_trigger_state.rs` with the test module (the implementation goes in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn pending_trigger_is_not_expired_immediately() {
        let p = PendingTrigger::new("close the books");
        assert!(!p.is_expired(Instant::now()));
    }

    #[test]
    fn pending_trigger_expires_after_ttl() {
        let mut p = PendingTrigger::new("close the books");
        p.created_at = Instant::now() - PENDING_TRIGGER_TTL - Duration::from_secs(1);
        assert!(p.is_expired(Instant::now()));
    }

    #[test]
    fn pending_trigger_not_yet_expired_just_under_ttl() {
        let mut p = PendingTrigger::new("close the books");
        p.created_at = Instant::now() - PENDING_TRIGGER_TTL + Duration::from_secs(1);
        assert!(!p.is_expired(Instant::now()));
    }

    #[test]
    fn parse_confirm_reply_recognizes_yes_and_no_case_insensitively() {
        assert_eq!(parse_confirm_reply("yes"), Some(ConfirmReply::Yes));
        assert_eq!(parse_confirm_reply("Yes"), Some(ConfirmReply::Yes));
        assert_eq!(parse_confirm_reply("YES"), Some(ConfirmReply::Yes));
        assert_eq!(parse_confirm_reply("  yes  "), Some(ConfirmReply::Yes));
        assert_eq!(parse_confirm_reply("no"), Some(ConfirmReply::No));
        assert_eq!(parse_confirm_reply("No"), Some(ConfirmReply::No));
    }

    #[test]
    fn parse_confirm_reply_rejects_anything_else() {
        assert_eq!(parse_confirm_reply("yeah"), None);
        assert_eq!(parse_confirm_reply("nope"), None);
        assert_eq!(parse_confirm_reply(""), None);
        assert_eq!(parse_confirm_reply("/team status"), None);
    }

    #[test]
    fn check_and_record_trigger_allows_under_the_limit() {
        let mut history = Vec::new();
        let now = Instant::now();
        assert!(check_and_record_trigger(&mut history, 3, now));
        assert!(check_and_record_trigger(&mut history, 3, now));
        assert!(check_and_record_trigger(&mut history, 3, now));
        assert_eq!(history.len(), 3);
    }

    #[test]
    fn check_and_record_trigger_denies_at_the_limit() {
        let mut history = Vec::new();
        let now = Instant::now();
        for _ in 0..3 {
            assert!(check_and_record_trigger(&mut history, 3, now));
        }
        assert!(!check_and_record_trigger(&mut history, 3, now));
        assert_eq!(history.len(), 3, "a denied attempt is not recorded");
    }

    #[test]
    fn check_and_record_trigger_forgets_attempts_older_than_the_window() {
        let mut history = vec![Instant::now() - Duration::from_secs(3601)];
        let now = Instant::now();
        // The one stale entry ages out, so this new attempt is allowed
        // even at a limit of 1.
        assert!(check_and_record_trigger(&mut history, 1, now));
        assert_eq!(history.len(), 1, "the stale entry was pruned, not kept");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p aivyx-channel --lib team_trigger_state:: -- --test-threads=1`
Expected: FAIL to compile — none of the types/functions exist yet (and the module isn't registered).

- [ ] **Step 3: Write the implementation**

Add above the test module in the same file:

```rust
//! Piece C (2026-08-23) — client-side (channel-adapter) state for
//! `/team run <goal>`'s confirm-first flow and per-channel rate
//! limiting. Deliberately client-side, not daemon-side — see the
//! Piece C plan's own Global Constraints for why (rate-limiting and
//! confirm-tracking are UX/abuse-prevention, not the security
//! boundary; the actual authorization check lives in
//! `daemon_server.rs`, enforced independently of any of this state).
//!
//! No confirm-first/pending-state precedent existed anywhere in this
//! codebase before this module (re-verified via a repo-wide grep
//! during planning) — this is a fresh, deliberately minimal pattern:
//! one `Option<PendingTrigger>` and one `Vec<Instant>` per chat,
//! held locally in each channel's own per-chat daemon-frontend loop
//! (no cross-restart persistence, no shared/global state).

use std::time::{Duration, Instant};

/// How long a `/team run` confirmation prompt stays valid before it
/// must be re-asked. Matches the design's own "e.g. 5 minutes."
pub const PENDING_TRIGGER_TTL: Duration = Duration::from_secs(300);

/// An outstanding "start '<goal>' on the default team? Reply
/// yes/no." prompt for one chat, awaiting resolution.
#[derive(Debug, Clone)]
pub struct PendingTrigger {
    pub goal: String,
    pub created_at: Instant,
}

impl PendingTrigger {
    pub fn new(goal: impl Into<String>) -> Self {
        PendingTrigger {
            goal: goal.into(),
            created_at: Instant::now(),
        }
    }

    /// Whether this prompt is too old to honor a late "yes"/"no" for.
    pub fn is_expired(&self, now: Instant) -> bool {
        now.duration_since(self.created_at) >= PENDING_TRIGGER_TTL
    }
}

/// A bare "yes" / "no" reply, recognized only while a
/// [`PendingTrigger`] is outstanding for that chat — never as a
/// general-purpose command, since that would swallow ordinary
/// conversation. Case-insensitive, whitespace-trimmed, exact match
/// only (no "yeah"/"yep"/"sure" fuzziness, matching this codebase's
/// established preference for unambiguous, exact command parsing
/// over natural-language guessing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmReply {
    Yes,
    No,
}

pub fn parse_confirm_reply(text: &str) -> Option<ConfirmReply> {
    match text.trim().to_ascii_lowercase().as_str() {
        "yes" => Some(ConfirmReply::Yes),
        "no" => Some(ConfirmReply::No),
        _ => None,
    }
}

/// A rolling-hour sliding-window rate limiter. `history` holds the
/// timestamp of every *allowed* attempt; entries older than one hour
/// are pruned before checking. Returns `true` (and records `now`) iff
/// `history.len() < limit` after pruning; returns `false` (and does
/// not record) otherwise — a denied attempt never counts against a
/// future one.
pub fn check_and_record_trigger(history: &mut Vec<Instant>, limit: u32, now: Instant) -> bool {
    const WINDOW: Duration = Duration::from_secs(3600);
    history.retain(|&t| now.duration_since(t) < WINDOW);
    if history.len() as u32 >= limit {
        false
    } else {
        history.push(now);
        true
    }
}
```

- [ ] **Step 4: Register the module**

In `crates/aivyx-channel/src/lib.rs`, immediately after `pub mod team_dispatch;` (added by Piece B), add:

```rust
pub mod team_trigger_state;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p aivyx-channel --lib team_trigger_state:: -- --test-threads=1`
Expected: PASS, all 9 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-channel/src/team_trigger_state.rs crates/aivyx-channel/src/lib.rs
git commit -m "feat(channel): add client-side confirm-first + rate-limit state (Piece C Task 9)"
```

---

### Task 10: `team_dispatch::dispatch`'s defensive `TeamCommand::Run` arm

**Files:**
- Modify: `crates/aivyx-channel/src/team_dispatch.rs`

**Interfaces:**
- Consumes: Task 8's `TeamCommand::Run { goal }` variant (`dispatch`'s `match` must be exhaustive over all `TeamCommand` variants now that `Run` exists).

- [ ] **Step 1: Write the failing test**

Add to `team_dispatch.rs`'s existing `mod tests`:

```rust
#[tokio::test]
async fn dispatch_run_is_never_reached_in_normal_flow_and_fails_closed_if_it_is() {
    // TeamCommand::Run's real flow (confirm-first, rate-limit, the
    // identity-declaring daemon_client::run_team_mission_channel one-
    // shot call) is handled directly by each channel's own daemon-
    // frontend loop, BEFORE team_command::parse's result ever reaches
    // this function — see Tasks 11-13. This arm exists only so
    // dispatch()'s match stays exhaustive; if it's ever hit anyway
    // (a wiring bug), it must fail closed with a clear message, never
    // silently start a mission or panic.
    let reply = dispatch(
        std::path::Path::new("/nonexistent/unused.sock"),
        TeamCommand::Run {
            goal: "close the books".to_string(),
        },
    )
    .await;
    assert!(reply.starts_with('✗'));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p aivyx-channel --lib team_dispatch:: -- --test-threads=1`
Expected: FAIL to compile — `dispatch`'s `match` is not exhaustive over `TeamCommand` once `Run` exists (Task 8 added the variant).

- [ ] **Step 3: Add the arm**

In `team_dispatch.rs`'s `dispatch` function, add a new match arm (order doesn't matter; place it last for readability):

```rust
        TeamCommand::Run { .. } => {
            "✗ /team run requires confirmation and must go through the channel's own \
             confirm-first flow — this should never be dispatched directly."
                .to_string()
        }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p aivyx-channel --lib team_dispatch:: -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Run the crate's full suite**

Run: `cargo test -p aivyx-channel --lib -- --test-threads=1`
Expected: PASS, no regressions.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-channel/src/team_dispatch.rs
git commit -m "feat(channel): add fail-closed defensive TeamCommand::Run arm to dispatch (Piece C Task 10)"
```

---

### Task 11: Wire `/team run` confirm-first + rate-limit into Telegram

**Files:**
- Modify: `crates/aivyx-channel/src/telegram_daemon_frontend.rs` (imports, `run_telegram_daemon_chat_task`'s signature + body, `run_telegram_daemon_multi_session`'s signature + its own call site inside this file if it spawns the chat task with new params)
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` (thread `team_run_channel`/`team_trigger_rate_limit` from the loaded `TelegramConfig` into the `run_telegram_daemon_multi_session(...)` call, ~line 9721)

**Interfaces:**
- Consumes: Task 2's `TelegramConfig.team_run_channel`/`team_trigger_rate_limit`, Task 7's `daemon_client::run_team_mission_channel`, Task 8's `TeamCommand::Run`, Task 9's `PendingTrigger`/`parse_confirm_reply`/`check_and_record_trigger`.

- [ ] **Step 1: Read the current real signatures before editing**

Read `crates/aivyx-channel/src/telegram_daemon_frontend.rs`'s current `run_telegram_daemon_multi_session` and `run_telegram_daemon_chat_task` signatures in full (they may have shifted slightly since this plan's own research), and `crates/aivyx-cli/src/bin/aivyx.rs`'s call site around line 9721 and the `tg`/`chat_filter` local variables just above it. This task's code below assumes the shapes this plan's research found; adjust variable names to match the real current code, not this plan's paraphrase, if they differ.

- [ ] **Step 2: Thread the two new params from `aivyx.rs` down to `run_telegram_daemon_multi_session`**

In `crates/aivyx-cli/src/bin/aivyx.rs`, near the existing `let chat_filter: Option<i64> = tg.chat_filter.map(|c| c.value);` (~line 9681), add:

```rust
            let team_run_channel = tg.team_run_channel;
            let team_trigger_rate_limit = tg.team_trigger_rate_limit;
```

Update the `run_telegram_daemon_multi_session(...)` call (~line 9721) to pass them:

```rust
                match run_telegram_daemon_multi_session(
                    transport,
                    chat_filter,
                    sp.clone(),
                    Some(active_role_name.clone()),
                    shutdown.clone(),
                    team_run_channel,
                    team_trigger_rate_limit,
                )
                .await
```

In `crates/aivyx-channel/src/telegram_daemon_frontend.rs`, update `run_telegram_daemon_multi_session`'s signature to accept and forward the two new parameters:

```rust
pub async fn run_telegram_daemon_multi_session(
    transport: Arc<ReqwestTransport>,
    chat_filter: Option<i64>,
    socket_path: PathBuf,
    role: Option<String>,
    shutdown: CancellationToken,
    team_run_channel: bool,
    team_trigger_rate_limit: Option<u32>,
) -> Result<(), DaemonError> {
```

Find where this function spawns `run_telegram_daemon_chat_task` (a `tokio::spawn` inside its own loop, per-chat) and add `team_run_channel, team_trigger_rate_limit,` to that call, matching whatever other per-chat params (`socket_path.clone()`, `role.clone()`, etc.) it already forwards.

- [ ] **Step 3: Add the imports**

In `telegram_daemon_frontend.rs`, immediately after the existing `use crate::team_dispatch;` (added by Piece B), add:

```rust
use crate::daemon_client;
use crate::team_trigger_state::{
    check_and_record_trigger, parse_confirm_reply, ConfirmReply, PendingTrigger,
};
use std::time::Instant;
```

(`crate::daemon_client` may already be imported for `DaemonSession` — if `use crate::daemon_client::DaemonSession;` already exists, change it to `use crate::daemon_client::{self, DaemonSession};` instead of adding a duplicate `use` line.)

- [ ] **Step 4: Add local per-chat state and the new command handling**

In `run_telegram_daemon_chat_task`'s signature, add the two new parameters:

```rust
async fn run_telegram_daemon_chat_task(
    transport: Arc<ReqwestTransport>,
    chat_id: i64,
    socket_path: PathBuf,
    role: Option<String>,
    mut mailbox: tokio::sync::mpsc::Receiver<IncomingMessage>,
    shutdown: CancellationToken,
    team_run_channel: bool,
    team_trigger_rate_limit: Option<u32>,
) -> Result<(), DaemonError> {
```

Immediately after `let mut session = DaemonSession::connect(...).await?;` and before the `loop {`, add:

```rust
    let mut pending_trigger: Option<PendingTrigger> = None;
    let mut trigger_history: Vec<Instant> = Vec::new();
```

Immediately after the existing block that handles `team_command::parse(msg.text.trim())` (Piece B's own addition — the `if let Some(team_cmd) = team_command::parse(...) { ... continue; }` block) and **before** the fallthrough to image-forwarding/`session.submit_input`, add:

```rust
        // Piece C — a pending confirm-first prompt takes priority over
        // everything else (including a stray gate_command/team_command
        // match, though "yes"/"no" never collide with either's own
        // `/`-prefixed syntax).
        if let Some(pending) = pending_trigger.take() {
            let now = Instant::now();
            if pending.is_expired(now) {
                // Fall through: an expired trigger doesn't consume this
                // message at all, and a stale yes/no gets the same
                // "expired" reply as a fresh, still-pending one below —
                // simplest to just re-check confirm_reply against it.
            }
            match parse_confirm_reply(msg.text.trim()) {
                Some(ConfirmReply::Yes) if pending.is_expired(now) => {
                    transport
                        .send_message(OutgoingMessage {
                            chat_id,
                            text: "✗ that request expired, ask again.".to_string(),
                        })
                        .await
                        .map_err(|e| {
                            DaemonError::Internal(format!("send_message to chat {chat_id}: {e}"))
                        })?;
                    continue;
                }
                Some(ConfirmReply::Yes) => {
                    let reply = match daemon_client::run_team_mission_channel(
                        &socket_path,
                        aivyx_ipc::protocol::FrontendType::Telegram,
                        pending.goal.clone(),
                    )
                    .await
                    {
                        Ok(mission_id) => {
                            format!("✓ Started mission {mission_id}.")
                        }
                        Err(e) => format!("✗ Could not start the mission: {e}"),
                    };
                    transport
                        .send_message(OutgoingMessage { chat_id, text: reply })
                        .await
                        .map_err(|e| {
                            DaemonError::Internal(format!("send_message to chat {chat_id}: {e}"))
                        })?;
                    continue;
                }
                Some(ConfirmReply::No) => {
                    transport
                        .send_message(OutgoingMessage {
                            chat_id,
                            text: "Cancelled.".to_string(),
                        })
                        .await
                        .map_err(|e| {
                            DaemonError::Internal(format!("send_message to chat {chat_id}: {e}"))
                        })?;
                    continue;
                }
                None => {
                    // Not a yes/no reply — put the pending trigger back
                    // (unless it just expired above) and fall through to
                    // the normal command/chat-turn handling below.
                    if !pending.is_expired(now) {
                        pending_trigger = Some(pending);
                    }
                }
            }
        }

        if let Some(TeamCommand::Run { goal }) = team_command::parse(msg.text.trim()) {
            if !team_run_channel {
                transport
                    .send_message(OutgoingMessage {
                        chat_id,
                        text: "✗ this channel is not authorized to start team missions."
                            .to_string(),
                    })
                    .await
                    .map_err(|e| {
                        DaemonError::Internal(format!("send_message to chat {chat_id}: {e}"))
                    })?;
                continue;
            }
            let allowed = match team_trigger_rate_limit {
                Some(limit) => {
                    check_and_record_trigger(&mut trigger_history, limit, Instant::now())
                }
                None => true,
            };
            if !allowed {
                let limit = team_trigger_rate_limit.unwrap_or(0);
                transport
                    .send_message(OutgoingMessage {
                        chat_id,
                        text: format!(
                            "✗ too many mission-start requests (max {limit} per hour), \
                             try again later."
                        ),
                    })
                    .await
                    .map_err(|e| {
                        DaemonError::Internal(format!("send_message to chat {chat_id}: {e}"))
                    })?;
                continue;
            }
            pending_trigger = Some(PendingTrigger::new(goal.clone()));
            transport
                .send_message(OutgoingMessage {
                    chat_id,
                    text: format!("Start '{goal}' on the default team? Reply yes/no."),
                })
                .await
                .map_err(|e| {
                    DaemonError::Internal(format!("send_message to chat {chat_id}: {e}"))
                })?;
            continue;
        }
```

Note this second block calls `team_command::parse` a **second** time (once inside the existing Piece B block above it, once here) — this is intentional and matches this file's own existing pattern of sequential `if let Some(...) = parser::parse(...)` checks (`gate_command::parse` then `team_command::parse` are already two separate calls on the same text today); a future cleanup could hoist a single `let team_cmd = team_command::parse(...)` above both checks, but that's out of this task's scope (it would touch Piece B's own already-shipped block, which this task deliberately leaves untouched apart from inserting before/after it).

- [ ] **Step 5: Update `TeamCommand` import**

Confirm `use crate::team_command;` (Piece B's own import) is already present; the new code above uses `TeamCommand::Run` directly, so also ensure `crate::team_command::TeamCommand` is in scope — add `use crate::team_command::{self, TeamCommand};` if the file currently only imports the module (`use crate::team_command;`) and not the type.

- [ ] **Step 6: Run the crate's full suite**

Run: `cargo build -p aivyx-channel` then `cargo test -p aivyx-channel --lib -- --test-threads=1`
Expected: clean build, all tests pass (this task adds no new tests of its own — same rationale as Piece B's own wiring tasks: this is glue over already-tested pure/dispatch primitives, with no existing per-file harness for the async loop itself).

Also run: `cargo build -p aivyx-cli` to confirm the `aivyx.rs` call-site change compiles.

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-channel/src/telegram_daemon_frontend.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat(channel): wire /team run confirm-first + rate-limit into Telegram (Piece C Task 11)"
```

---

### Task 12: Wire `/team run` confirm-first + rate-limit into Discord

**Files:**
- Modify: `crates/aivyx-channel/src/discord_daemon_frontend.rs`
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` (the `run_discord_daemon_multi_session(...)` call site, ~line 9830)

**Interfaces:**
- Consumes: the same Task 2/7/8/9 interfaces as Task 11, applied to Discord's own `run_discord_daemon_multi_session`/`run_discord_daemon_channel_task` and `OutgoingMessage { channel_id, text }` shape (a bare `channel_id: u64` variable, not Telegram's `chat_id` or Slack's `msg.channel_id.clone()`).

- [ ] **Step 1: Read the current real signatures before editing**

Read `discord_daemon_frontend.rs`'s current `run_discord_daemon_multi_session`/`run_discord_daemon_channel_task` signatures and the `aivyx.rs` call site (~line 9830) and its own local `DiscordConfig` variable in full — this task's code below assumes the shapes this plan's own research found; adjust variable names to match the real current code, not this plan's paraphrase, if they differ.

- [ ] **Step 2: Thread the two new params from `aivyx.rs` down to `run_discord_daemon_multi_session`**

In `crates/aivyx-cli/src/bin/aivyx.rs`, near wherever the Discord `token`/`application_id` locals are extracted from the loaded `DiscordConfig` (the equivalent of Telegram's `tg` variable), add:

```rust
            let team_run_channel = discord_cfg.team_run_channel;
            let team_trigger_rate_limit = discord_cfg.team_trigger_rate_limit;
```

(`discord_cfg` stands for whatever the real local variable name is for the loaded `DiscordConfig` at that point — read the surrounding code first and use its real name.) Update the `run_discord_daemon_multi_session(...)` call (~line 9830) to pass both:

```rust
                match aivyx_channel::discord_daemon_frontend::run_discord_daemon_multi_session(
                    transport,
                    sp.clone(),
                    Some(active_role_name.clone()),
                    shutdown.clone(),
                    team_run_channel,
                    team_trigger_rate_limit,
                )
                .await
```

(Keep every existing argument in its current position and order — the snippet above shows where the two new trailing arguments go, not a replacement for arguments this plan's research didn't have full visibility into; read the real current call site first and append the two new arguments at its actual end.)

In `crates/aivyx-channel/src/discord_daemon_frontend.rs`, update `run_discord_daemon_multi_session`'s signature to accept and forward the two new parameters:

```rust
pub async fn run_discord_daemon_multi_session(
    transport: Arc<TwilightTransport>,
    socket_path: PathBuf,
    role: Option<String>,
    shutdown: CancellationToken,
    team_run_channel: bool,
    team_trigger_rate_limit: Option<u32>,
) -> Result<(), DaemonError> {
```

(Read the real current signature first — it may have parameters beyond these four; add the two new ones at the end, keep every existing parameter unchanged.) Find where this function spawns `run_discord_daemon_channel_task` (a `tokio::spawn` inside its own loop, per-channel) and add `team_run_channel, team_trigger_rate_limit,` to that call, matching whatever other per-channel params it already forwards.

- [ ] **Step 3: Add the imports**

In `discord_daemon_frontend.rs`, immediately after the existing `use crate::team_dispatch;` (added by Piece B), add:

```rust
use crate::daemon_client;
use crate::team_trigger_state::{
    check_and_record_trigger, parse_confirm_reply, ConfirmReply, PendingTrigger,
};
use std::time::Instant;
```

(`crate::daemon_client` may already be imported for `DaemonSession` — if `use crate::daemon_client::DaemonSession;` already exists, change it to `use crate::daemon_client::{self, DaemonSession};` instead of adding a duplicate `use` line.)

- [ ] **Step 4: Add local per-chat state and the new command handling**

In `run_discord_daemon_channel_task`'s signature, add the two new parameters:

```rust
async fn run_discord_daemon_channel_task(
    transport: Arc<TwilightTransport>,
    channel_id: u64,
    socket_path: PathBuf,
    role: Option<String>,
    mut mailbox: tokio::sync::mpsc::Receiver<IncomingMessage>,
    shutdown: CancellationToken,
    team_run_channel: bool,
    team_trigger_rate_limit: Option<u32>,
) -> Result<(), DaemonError> {
```

Immediately after `let mut session = DaemonSession::connect(...).await?;` and before the `loop {`, add:

```rust
    let mut pending_trigger: Option<PendingTrigger> = None;
    let mut trigger_history: Vec<Instant> = Vec::new();
```

Immediately after the existing block that handles `team_command::parse(msg.text.trim())` (Piece B's own addition) and **before** the fallthrough to `session.submit_input`, add:

```rust
        // Piece C — a pending confirm-first prompt takes priority over
        // everything else.
        if let Some(pending) = pending_trigger.take() {
            let now = Instant::now();
            match parse_confirm_reply(msg.text.trim()) {
                Some(ConfirmReply::Yes) if pending.is_expired(now) => {
                    transport
                        .send_message(OutgoingMessage {
                            channel_id,
                            text: "✗ that request expired, ask again.".to_string(),
                        })
                        .await
                        .map_err(|e| {
                            DaemonError::Internal(format!(
                                "send_message to channel {channel_id}: {e}"
                            ))
                        })?;
                    continue;
                }
                Some(ConfirmReply::Yes) => {
                    let reply = match daemon_client::run_team_mission_channel(
                        &socket_path,
                        aivyx_ipc::protocol::FrontendType::Discord,
                        pending.goal.clone(),
                    )
                    .await
                    {
                        Ok(mission_id) => format!("✓ Started mission {mission_id}."),
                        Err(e) => format!("✗ Could not start the mission: {e}"),
                    };
                    transport
                        .send_message(OutgoingMessage { channel_id, text: reply })
                        .await
                        .map_err(|e| {
                            DaemonError::Internal(format!(
                                "send_message to channel {channel_id}: {e}"
                            ))
                        })?;
                    continue;
                }
                Some(ConfirmReply::No) => {
                    transport
                        .send_message(OutgoingMessage {
                            channel_id,
                            text: "Cancelled.".to_string(),
                        })
                        .await
                        .map_err(|e| {
                            DaemonError::Internal(format!(
                                "send_message to channel {channel_id}: {e}"
                            ))
                        })?;
                    continue;
                }
                None => {
                    if !pending.is_expired(now) {
                        pending_trigger = Some(pending);
                    }
                }
            }
        }

        if let Some(TeamCommand::Run { goal }) = team_command::parse(msg.text.trim()) {
            if !team_run_channel {
                transport
                    .send_message(OutgoingMessage {
                        channel_id,
                        text: "✗ this channel is not authorized to start team missions."
                            .to_string(),
                    })
                    .await
                    .map_err(|e| {
                        DaemonError::Internal(format!(
                            "send_message to channel {channel_id}: {e}"
                        ))
                    })?;
                continue;
            }
            let allowed = match team_trigger_rate_limit {
                Some(limit) => {
                    check_and_record_trigger(&mut trigger_history, limit, Instant::now())
                }
                None => true,
            };
            if !allowed {
                let limit = team_trigger_rate_limit.unwrap_or(0);
                transport
                    .send_message(OutgoingMessage {
                        channel_id,
                        text: format!(
                            "✗ too many mission-start requests (max {limit} per hour), \
                             try again later."
                        ),
                    })
                    .await
                    .map_err(|e| {
                        DaemonError::Internal(format!(
                            "send_message to channel {channel_id}: {e}"
                        ))
                    })?;
                continue;
            }
            pending_trigger = Some(PendingTrigger::new(goal.clone()));
            transport
                .send_message(OutgoingMessage {
                    channel_id,
                    text: format!("Start '{goal}' on the default team? Reply yes/no."),
                })
                .await
                .map_err(|e| {
                    DaemonError::Internal(format!("send_message to channel {channel_id}: {e}"))
                })?;
            continue;
        }
```

Insert both blocks at the identical point Task 11 used: immediately after Discord's own existing Piece B `gate_command`/`team_command` block, before the fallthrough to `session.submit_input`.

- [ ] **Step 5: Update `TeamCommand` import**

Confirm `use crate::team_command;` (Piece B's own import) is already present; the new code above uses `TeamCommand::Run` directly, so also ensure `crate::team_command::TeamCommand` is in scope — add `use crate::team_command::{self, TeamCommand};` if the file currently only imports the module.

- [ ] **Step 6: Run the crate's full suite**

Run: `cargo build -p aivyx-channel` then `cargo test -p aivyx-channel --lib -- --test-threads=1`
Expected: clean build, all tests pass, no new tests (same rationale as Task 11 Step 6 — thin glue over already-tested primitives, no existing per-file harness for the async loop itself).

Also run: `cargo build -p aivyx-cli` to confirm the `aivyx.rs` call-site change compiles.

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-channel/src/discord_daemon_frontend.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat(channel): wire /team run confirm-first + rate-limit into Discord (Piece C Task 12)"
```

---

### Task 13: Wire `/team run` confirm-first + rate-limit into Slack

**Files:**
- Modify: `crates/aivyx-channel/src/slack_daemon_frontend.rs`
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` (the `run_slack_daemon_multi_session(...)` call site, ~line 9953)

**Interfaces:**
- Consumes: the same Task 2/7/8/9 interfaces as Tasks 11-12, applied to Slack's own `run_slack_daemon_multi_session`/`run_slack_daemon_partition_task` and `OutgoingMessage { channel_id: msg.channel_id.clone(), text }` shape.

- [ ] **Step 1: Read the current real signatures before editing**

Read `slack_daemon_frontend.rs`'s current `run_slack_daemon_multi_session`/`run_slack_daemon_partition_task` signatures and the `aivyx.rs` call site (~line 9953) and its own local `SlackConfig` variable in full — this task's code below assumes the shapes this plan's own research found; adjust variable names to match the real current code, not this plan's paraphrase, if they differ.

- [ ] **Step 2: Thread the two new params from `aivyx.rs` down to `run_slack_daemon_multi_session`**

In `crates/aivyx-cli/src/bin/aivyx.rs`, near wherever the Slack `bot_token`/`app_token`/`team_id` locals are extracted from the loaded `SlackConfig`, add:

```rust
                        let team_run_channel = slack_cfg.team_run_channel;
                        let team_trigger_rate_limit = slack_cfg.team_trigger_rate_limit;
```

(`slack_cfg` stands for whatever the real local variable name is for the loaded `SlackConfig` at that point — read the surrounding code first and use its real name.) Update the `run_slack_daemon_multi_session(...)` call (~line 9953) to pass both:

```rust
                        match aivyx_channel::slack_daemon_frontend::run_slack_daemon_multi_session(
                            transport,
                            sp.clone(),
                            Some(active_role_name.clone()),
                            shutdown.clone(),
                            team_run_channel,
                            team_trigger_rate_limit,
                        )
                        .await
```

(Keep every existing argument in its current position and order — the snippet above shows where the two new trailing arguments go, not a replacement for arguments this plan's research didn't have full visibility into; read the real current call site first and append the two new arguments at its actual end.)

In `crates/aivyx-channel/src/slack_daemon_frontend.rs`, update `run_slack_daemon_multi_session`'s signature to accept and forward the two new parameters:

```rust
pub async fn run_slack_daemon_multi_session(
    transport: Arc<SlackMorphismTransport>,
    socket_path: PathBuf,
    role: Option<String>,
    shutdown: CancellationToken,
    team_run_channel: bool,
    team_trigger_rate_limit: Option<u32>,
) -> Result<(), DaemonError> {
```

(Read the real current signature first — it may have parameters beyond these four; add the two new ones at the end, keep every existing parameter unchanged.) Find where this function spawns `run_slack_daemon_partition_task` (a `tokio::spawn` inside its own loop, per-partition) and add `team_run_channel, team_trigger_rate_limit,` to that call, matching whatever other per-partition params it already forwards.

- [ ] **Step 3: Add the imports**

In `slack_daemon_frontend.rs`, immediately after the existing `use crate::team_dispatch;` (added by Piece B), add:

```rust
use crate::daemon_client;
use crate::team_trigger_state::{
    check_and_record_trigger, parse_confirm_reply, ConfirmReply, PendingTrigger,
};
use std::time::Instant;
```

(`crate::daemon_client` may already be imported for `DaemonSession` — if `use crate::daemon_client::DaemonSession;` already exists, change it to `use crate::daemon_client::{self, DaemonSession};` instead of adding a duplicate `use` line.)

- [ ] **Step 4: Add local per-chat state and the new command handling**

In `run_slack_daemon_partition_task`'s signature, add the two new parameters:

```rust
async fn run_slack_daemon_partition_task(
    transport: Arc<SlackMorphismTransport>,
    partition: String,
    socket_path: PathBuf,
    role: Option<String>,
    mut mailbox: tokio::sync::mpsc::Receiver<IncomingMessage>,
    shutdown: CancellationToken,
    team_run_channel: bool,
    team_trigger_rate_limit: Option<u32>,
) -> Result<(), DaemonError> {
```

Immediately after `let mut session = DaemonSession::connect(...).await?;` and before the `loop {`, add:

```rust
    let mut pending_trigger: Option<PendingTrigger> = None;
    let mut trigger_history: Vec<Instant> = Vec::new();
```

Immediately after the existing block that handles `team_command::parse(msg.text.trim())` (Piece B's own addition) and **before** the fallthrough to `session.submit_input`, add — note Slack's own existing `gate_command` block builds its `OutgoingMessage` with `channel_id: msg.channel_id.clone()` (read from the mailbox message currently being handled, a `String`, unlike Telegram's loop-scoped `chat_id: i64` or Discord's loop-scoped `channel_id: u64`); every `OutgoingMessage` literal below matches that same field-value expression:

```rust
        // Piece C — a pending confirm-first prompt takes priority over
        // everything else.
        if let Some(pending) = pending_trigger.take() {
            let now = Instant::now();
            match parse_confirm_reply(msg.text.trim()) {
                Some(ConfirmReply::Yes) if pending.is_expired(now) => {
                    transport
                        .send_message(OutgoingMessage {
                            channel_id: msg.channel_id.clone(),
                            text: "✗ that request expired, ask again.".to_string(),
                        })
                        .await
                        .map_err(|e| {
                            DaemonError::Internal(format!(
                                "send_message to partition {partition}: {e}"
                            ))
                        })?;
                    continue;
                }
                Some(ConfirmReply::Yes) => {
                    let reply = match daemon_client::run_team_mission_channel(
                        &socket_path,
                        aivyx_ipc::protocol::FrontendType::Slack,
                        pending.goal.clone(),
                    )
                    .await
                    {
                        Ok(mission_id) => format!("✓ Started mission {mission_id}."),
                        Err(e) => format!("✗ Could not start the mission: {e}"),
                    };
                    transport
                        .send_message(OutgoingMessage {
                            channel_id: msg.channel_id.clone(),
                            text: reply,
                        })
                        .await
                        .map_err(|e| {
                            DaemonError::Internal(format!(
                                "send_message to partition {partition}: {e}"
                            ))
                        })?;
                    continue;
                }
                Some(ConfirmReply::No) => {
                    transport
                        .send_message(OutgoingMessage {
                            channel_id: msg.channel_id.clone(),
                            text: "Cancelled.".to_string(),
                        })
                        .await
                        .map_err(|e| {
                            DaemonError::Internal(format!(
                                "send_message to partition {partition}: {e}"
                            ))
                        })?;
                    continue;
                }
                None => {
                    if !pending.is_expired(now) {
                        pending_trigger = Some(pending);
                    }
                }
            }
        }

        if let Some(TeamCommand::Run { goal }) = team_command::parse(msg.text.trim()) {
            if !team_run_channel {
                transport
                    .send_message(OutgoingMessage {
                        channel_id: msg.channel_id.clone(),
                        text: "✗ this channel is not authorized to start team missions."
                            .to_string(),
                    })
                    .await
                    .map_err(|e| {
                        DaemonError::Internal(format!(
                            "send_message to partition {partition}: {e}"
                        ))
                    })?;
                continue;
            }
            let allowed = match team_trigger_rate_limit {
                Some(limit) => {
                    check_and_record_trigger(&mut trigger_history, limit, Instant::now())
                }
                None => true,
            };
            if !allowed {
                let limit = team_trigger_rate_limit.unwrap_or(0);
                transport
                    .send_message(OutgoingMessage {
                        channel_id: msg.channel_id.clone(),
                        text: format!(
                            "✗ too many mission-start requests (max {limit} per hour), \
                             try again later."
                        ),
                    })
                    .await
                    .map_err(|e| {
                        DaemonError::Internal(format!(
                            "send_message to partition {partition}: {e}"
                        ))
                    })?;
                continue;
            }
            pending_trigger = Some(PendingTrigger::new(goal.clone()));
            transport
                .send_message(OutgoingMessage {
                    channel_id: msg.channel_id.clone(),
                    text: format!("Start '{goal}' on the default team? Reply yes/no."),
                })
                .await
                .map_err(|e| {
                    DaemonError::Internal(format!("send_message to partition {partition}: {e}"))
                })?;
            continue;
        }
```

Insert both blocks at the identical point Tasks 11-12 used: immediately after Slack's own existing Piece B `gate_command`/`team_command` block, before the fallthrough to `session.submit_input`.

- [ ] **Step 5: Update `TeamCommand` import**

Confirm `use crate::team_command;` (Piece B's own import) is already present; the new code above uses `TeamCommand::Run` directly, so also ensure `crate::team_command::TeamCommand` is in scope — add `use crate::team_command::{self, TeamCommand};` if the file currently only imports the module.

- [ ] **Step 6: Run the crate's full suite**

Run: `cargo build -p aivyx-channel` then `cargo test -p aivyx-channel --lib -- --test-threads=1`
Expected: clean build, all tests pass, no new tests (same rationale as Tasks 11-12 Step 6 — thin glue over already-tested primitives, no existing per-file harness for the async loop itself).

Also run: `cargo build -p aivyx-cli` to confirm the `aivyx.rs` call-site change compiles.

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-channel/src/slack_daemon_frontend.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat(channel): wire /team run confirm-first + rate-limit into Slack (Piece C Task 13)"
```

---

## Final Verification

After all 13 tasks:

1. `cargo build --workspace` — clean build across every crate this plan touched (`aivyx-capability`, `aivyx-config`, `aivyx-ipc`, `aivyx-audit`, `aivyx-channel`, `aivyx-cli`).
2. `cargo test -p aivyx-capability -p aivyx-config -p aivyx-ipc -p aivyx-audit -p aivyx-channel --lib -- --test-threads=1` — full suite across every touched crate passes.
3. `cargo clippy -p aivyx-channel --all-targets -- -D warnings` — check for new warnings beyond the one pre-existing, out-of-scope `trigger.rs:223` finding (confirmed predating this and every other branch in this lineage).
4. Grep for any remaining `TODO`/`unimplemented!()` introduced by this plan's files — should be none.
5. **Manually trace the full authorization path end-to-end by reading (not running) the final diff**, confirming the security property this whole plan exists to deliver: an operator with `team_run_channel = false` (the default) in `[telegram]`/`[discord]`/`[slack]` — even if a malicious or buggy channel-adapter process somehow sent `RunTeamMissionChannel` anyway — hits `channel_trigger_authorized` returning `false` inside the **daemon's own** `handle_connection`, independent of anything the channel-adapter claims about itself, and gets `DaemonMessage::Error { code: "team_run_channel_denied", ... }` back — never a started mission.
6. Confirm no path in the new code ever passes `Some(config)` to `start_from_goal_for_channel_trigger` — grep the whole diff for `start_from_goal_for_channel_trigger(` and confirm every call site's second argument is literally `None`.

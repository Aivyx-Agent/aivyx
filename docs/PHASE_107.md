# Phase 107 — Discord Channel Adapter (`aivyx-discord`)

The third Chapter D item and the first to genuinely grow the
substrate. Phase 105 added a JSONL emitter over existing
data; Phase 106 added a curated docs catalog and a thin CLI
surface. Phase 107 adds a **third channel adapter** —
`aivyx-discord` — at full parity with the Phase 8/9
`aivyx-telegram` precedent: a new workspace crate with its
own `ChannelContext` implementation, a private transport
trait + scripted-double, a `run_discord_session` sibling of
the existing `run_session` / `run_telegram_session`, and
binary wiring through the existing
`ChannelKind::<Discord>` dispatch shape.

The headline outcome: an operator with a Discord bot token
runs `aivyx --channel discord` and gets the same full agent
experience they get on Telegram today — Profile + Persona +
mission gates + `/approve` / `/reject` text commands + the
HMAC audit chain — but inside Discord.

## Why this, why now

- **Chapter D's third item by the easy-wins-first
  ordering.** Phase 105 (read-only export) and Phase 106
  (docs + CLI emit) were both substrate-friendly; Phase 107
  is the first item that genuinely lifts new code into the
  workspace. Doing it here, with two adapter precedents
  already in tree (Local from Phase 0 and Telegram from
  Phase 8/9) and the channel-SDK contract (`docs/ADAPTER_PATTERN.md`,
  `docs/CHANNEL_SDK.md`) already shipped, is the right
  moment.
- **It closes the Hermes-comparison channel-breadth gap.**
  Hermes ships six channels out of the box (Telegram,
  Discord, Slack, WhatsApp, Signal, CLI); Aivyx ships three
  (Local, Telegram, Web UI). Phase 107 adds Discord; Phase
  108 adds Slack. Two more adapters move Aivyx to five —
  not parity, but operator-meaningful breadth.
- **The substrate is genuinely ready for it.** Phase 19
  shipped daemon-frontend wiring for Telegram; Phase 48
  shipped the Channel SDK contract; the `ChannelContext`
  trait has been stable since Phase 8. Adding adapter #3 is
  the precise scenario `docs/ADAPTER_PATTERN.md` was
  written to support.
- **Full parity with `aivyx-telegram` chosen at sign-off
  (Q2).** Foundation-only was the recommended option;
  operator chose full parity instead. That makes Phase 107
  a deliberate multi-session phase rather than a single-
  commit ship.

## Scope (Q-block sign-off)

- **Q1 — Library:** (a) **twilight-rs.** The modular thin-
  layer SDK. Mirrors the Phase 8 `frankenstein vs teloxide`
  decision: pick the wrapper that exposes raw protocol
  methods, not the framework that owns its own event loop.
  `twilight-gateway` for the WebSocket, `twilight-http` for
  REST, `twilight-model` for types.
- **Q2 — Scope:** (c) **Full parity with `aivyx-telegram`.**
  DMs *and* channel messages, slash-command-shape paths,
  multi-session-partition support across guilds / channels,
  full scripted-test coverage. Realistically multi-session;
  the open doc explicitly sub-tasks the work so the phase
  can pause/resume.
- **Q3 — Approve UX:** (a) **Text commands matching
  Telegram.** `/approve` and `/reject` typed as regular
  messages — same regex as Phase 21. Zero new substrate;
  existing mission-gate plumbing flows through unchanged.
  Discord shows the slash-autocomplete affordance but the
  message lands as text.

## What will *not* hold (predicted streak breaks)

- **`aivyx-core/src/lib.rs` — likely to break.** Phase 107
  is the first since Phase 100 that introduces a new
  workspace crate with a public `ChannelContext` impl.
  Whether `aivyx-core/src/lib.rs` survives byte-identical
  depends on whether any new variant lifts into core (e.g.,
  a `ChannelPlatform::Discord` enum variant — the Telegram
  precedent shows `ChannelPlatform::Telegram` lives in
  `aivyx-core`). The honest prediction: this streak
  **breaks** at the variant-add commit. Streak: **0 after
  Phase 107**, re-establishing from there.
- **Zero new workspace deps — breaks.** Phase 107 is the
  deliberate one-time twilight-rs adoption. Four new direct
  workspace deps (`twilight-gateway`,
  `twilight-http`, `twilight-model`, optional
  `twilight-cache-inmemory`) plus transitives. This is the
  cost of an SDK-anchored adapter; the alternative
  (hand-roll Gateway) was scoped out at Q1.
- **A4 amendment addendum.** Workspace crate count moves
  12 → 13. Matches the Phase 49 / 24 / 22 pattern of
  amending A4 (Workspace Layout) when the crate set grows
  in a load-bearing way. The amendment lands inside the
  phase, not as a separate `docs(amendment): …` commit, per
  the precedent.

## What *should* hold

- **DESIGN.md** — should hold. New adapter is exactly the
  shape the existing substrate is designed for; no
  technical-contract decision should need amending.
  `ChannelContext`, `TrustTier::SemiTrusted` ceiling,
  audit-chain `ChannelPlatform` enum, mission state machine
  — all already accommodate the third adapter.
  Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to fifty-four**.
- **PRODUCT.md** — should hold. P5 (Multi-Channel) is
  already delivered; Phase 107 widens the channel set
  inside the same commitment envelope.
  Hash at entry:
  `9f0a515c9076544866aa955d6835763ee15beb0d009b4c335f5710b6d9ba61d3`.
  Prediction: streak **extends to seven** (was 6).
- **Test count** — large positive. The Telegram precedent
  carries 3163 lines of tests. Discord parity targets
  similar coverage. Rough prediction: **+50 to +100** tests.

## Tasks

This phase is structured into six sub-tasks so the work can
pause/resume across sessions. Each sub-task ends in a clean
commit; the phase exits only after all six land plus the
exit commit + backfill.

### Task 1 — Open (this commit)

`docs/PHASE_107.md` + `docs/ROADMAP.md` Chapter D entry flip
(scheduled → Active) + per-phase `## Phase 107` section +
`docs/README.md` status row.

### Task 2 — Crate skeleton + workspace wiring + A4 addendum

- New `crates/aivyx-discord/` workspace member with
  `Cargo.toml` (twilight-rs dep set + the shared workspace
  deps mirroring `aivyx-telegram`) and `src/lib.rs` (module
  declarations only).
- Module skeletons: `discord_channel.rs`, `transport.rs`,
  `session.rs`, `tests.rs`. Each lands as a one-line stub
  so the workspace compiles before any logic exists.
- Workspace `Cargo.toml` gains the `aivyx-discord` member.
- DESIGN.md A4 addendum: crate count 12 → 13, known-bases
  list reviewed for any new scope (`channel.discord:*` is
  the most likely candidate; the Telegram precedent didn't
  earn a dedicated scope-base, and Phase 107 follows that
  rule unless a concrete need surfaces).
- ChannelPlatform variant lift: `ChannelPlatform::Discord`
  added to `aivyx-core/src/lib.rs`. This is the predicted
  lib.rs streak break.
- `[discord]` TOML section added to `aivyx-config` with
  fields `token` (bot token), `application_id` (for slash
  command registration; can stay `None` under Q3a).

### Task 3 — Transport trait + scripted double

- `pub(crate) trait DiscordTransport` with the
  two-to-four-method surface the channel actually calls:
  `next_message(timeout) -> Result<IncomingMessage, …>` for
  Gateway reads and `send_message(channel_id, text) ->
  Result<(), …>` for REST writes.
- Production impl: `TwilightTransport` wraps
  `twilight_gateway::Shard` + `twilight_http::Client`. The
  Gateway state machine (identify, heartbeat, sequence
  tracking, resume) lives inside twilight; we wire it via
  the standard `Shard::next_event` loop.
- Scripted double: `ScriptedTransport` lives in
  `src/tests.rs`. Captures outgoing sends; replays queued
  inbound messages with `tokio::time::sleep` on drain so the
  outer loop doesn't hot-spin.
- `IncomingMessage` and `OutgoingMessage` types local to
  the crate (mirrors the `aivyx-telegram` `transport.rs`
  shape so the sibling pattern reads cleanly).

### Task 4 — `DiscordChannel` ChannelContext impl

- `DiscordChannel` struct holding a `Box<dyn
  DiscordTransport>`, an outgoing-buffer `Mutex<Vec<String>>`
  (mirrors Telegram's pattern; never holds the lock across
  `.await`), and a per-turn cancellation token.
- `impl ChannelContext for DiscordChannel`:
  - `trust_tier()` → `TrustTier::SemiTrusted` (matches the
    `docs/ADAPTER_PATTERN.md` tier-selection guidance).
  - `platform()` → `ChannelPlatform::Discord`.
  - `session_partition()` → `Some(channel_id.to_string())`
    so DMs and channel messages partition cleanly (matches
    Telegram's `chat_id`-based partitioning from Phase 8).
  - `stream_event` / `emit_text` flush via
    `send_message`.
  - `cancellation_token()` exposes the per-turn token.
  - `reset_cancellation()` rotates it between turns.

### Task 5 — `run_discord_session` driver + binary wiring

- `run_discord_session` sibling of `run_telegram_session`:
  the planner / agent / audit construction is identical
  copy (~50 lines per the adapter pattern doc); the outer
  loop is fresh Discord-shaped (a `Shard::next_event` loop
  per chat partition rather than a long-poll cursor).
- `run_discord_session_with_transport` inner for tests.
- Binary wiring: `ChannelKind::Discord` variant in
  `aivyx-channel/src/bin/aivyx.rs`'s channel dispatch +
  `--channel discord` CLI flag parsing + the
  `daemon-or-in-process` fallback path mirroring Phase 19
  Telegram-over-daemon.
- Daemon-frontend support: `FrontendType::Discord`
  variant + `discord_daemon_frontend.rs` (mirrors the
  Phase 19 `telegram_daemon_frontend.rs` pattern).
- Multi-partition support: one shard for the bot, the
  session driver demultiplexes inbound events by
  `channel_id` and dispatches each to its own turn with
  the right partition.
- `/approve` and `/reject` text-command handling: regex
  match inbound text against the Phase 21 mission-gate
  commands, route to the existing
  `MissionGateResolveCommand` plumbing.

### Task 6 — Scripted e2e tests

- `discord_session_smoke_e2e`: one-turn round-trip
  through the scripted transport. Captures the outgoing
  send; asserts the inbound message reached the agent.
- `discord_two_chats_persistent_e2e`: two channel
  partitions, one redb store, asserts memory partitioning
  via `session_partition()` matches the Telegram
  precedent.
- `discord_approve_command_resolves_gate_e2e`: agent
  escalates → `Gate` lands as outgoing message; operator
  scripts a `/approve` reply; turn resumes. Mirrors the
  Telegram gate test.
- `discord_cancellation_token_rotates_between_turns`: the
  per-turn token shape Phase 18 / 19 introduced for
  daemon-mode-over-IPC.
- Real-protocol smoke test **deferred to the Channel
  Activation Milestone** per the
  `docs/ADAPTER_PATTERN.md` checklist (item 7). The
  scripted suite covers enough to ship; the real-bot
  pass runs once per milestone against the full adapter
  matrix.

### Task 7 — Docs sweep

- `docs/ADAPTER_PATTERN.md` updated with the Discord data
  point per the pattern doc's own item-9 instruction
  ("update this document if any step felt wrong"). Item-9
  is the spot where two-data-point patterns become
  three-data-point patterns.
- `docs/CHANNEL_SDK.md` Discord row added to whatever
  adapter-set table the doc carries.
- `docs/INSTALL.md` first-run checklist gains a
  `--channel discord` setup-walkthrough note.
- `examples/aivyx.toml` gains a `[discord]` section
  alongside the existing `[telegram]` block.
- Channel Activation Milestone entry in ROADMAP gains the
  Discord adapter as a deferred real-protocol smoke item.

### Task 8 — Exit

PHASE_107 prediction-vs-reality + ROADMAP freeze + README
flip + hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — SDK:** (a) **twilight-rs.** Thin protocol wrapper
  matching the Phase 8 frankenstein-vs-teloxide decision.
  Adds ~4 direct workspace deps; the Gateway state machine
  (identify, heartbeat, sequence, resume) lives inside
  twilight, surfaced as a stream of typed events.
- **Q2 — Scope:** (c) **Full parity with `aivyx-telegram`.**
  Multi-session phase with explicit sub-task structure
  above. Foundation-only was the easier-wins option; full
  parity is the operator-chosen scope.
- **Q3 — Approve UX:** (a) **Text commands matching
  Telegram.** Zero new substrate. Slash-command registration
  + interaction handling stays a named deferral for a
  follow-on phase if Discord-native UX surfaces as a
  meaningful operator-experience gap.

## Exit criteria

- [ ] `docs/PHASE_107.md` + ROADMAP Chapter D Phase 107
  entry flip + docs/README status row — Task 1 (this
  commit).
- [ ] `crates/aivyx-discord/` workspace member + module
  skeletons + workspace `Cargo.toml` wiring + DESIGN.md
  A4 addendum + `ChannelPlatform::Discord` variant in
  `aivyx-core` + `[discord]` TOML section — Task 2.
- [ ] `DiscordTransport` trait + `TwilightTransport`
  production impl + `ScriptedTransport` test double —
  Task 3.
- [ ] `DiscordChannel` ChannelContext impl — Task 4.
- [ ] `run_discord_session` + `_with_transport` inner +
  binary wiring + `--channel discord` flag +
  daemon-frontend variant — Task 5.
- [ ] Scripted e2e suite (smoke, two-partition,
  gate-resolve, cancellation) — Task 6.
- [ ] Docs sweep (ADAPTER_PATTERN.md, CHANNEL_SDK.md,
  INSTALL.md, examples/aivyx.toml) — Task 7.
- [ ] All three Q-block questions resolved with operator
  sign-off pre-Task 2 (recorded above).
- [ ] DESIGN.md streak extends to fifty-four.
- [ ] PRODUCT.md streak extends to seven.
- [ ] **`aivyx-core/src/lib.rs` streak deliberately breaks**
  at the `ChannelPlatform::Discord` variant-add (Task 2).
  Re-establishes from 0.
- [ ] **Zero-new-deps streak deliberately breaks** at the
  twilight-rs adoption (Task 2). New direct deps:
  `twilight-gateway`, `twilight-http`, `twilight-model`,
  optional `twilight-cache-inmemory`.
- [ ] A4 amendment addendum filed (12 → 13 crates).
- [ ] Test count delta positive — predicted `+50` to
  `+100`.
- [ ] Zero clippy warnings.
- [ ] Real-protocol smoke test deferred to the Channel
  Activation Milestone (per `docs/ADAPTER_PATTERN.md`
  checklist item 7).

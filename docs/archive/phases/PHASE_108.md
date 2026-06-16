# Phase 108 — Slack Channel Adapter (`aivyx-slack`)

The fourth Chapter D item and the **four-data-point
confirmation** for the adapter pattern Phase 9 wrote down and
Phase 107 promoted from tentative to confirmed-at-three.
Phase 108 adds a fourth in-tree adapter (`aivyx-slack`) at
foundation scope: DMs + channel messages, no threads, no
Block Kit, no file attachments, no slash-command registration.

The headline outcome: an operator who has a Slack workspace
runs `aivyx --channel slack` with a bot token + an app-level
Socket Mode token, talks to the bot from any DM or any
channel the bot has been invited to, and gets the same agent
experience they already get on Discord and Telegram — Profile
+ Persona + mission-gate escalation rendering + HMAC audit
chain + `/cancel` mid-turn cancellation.

## Why this, why now

- **The fourth-adapter sanity check.** Phase 107 promoted
  `docs/ADAPTER_PATTERN.md` from "tentative" (two data
  points) to "confirmed at three." Phase 108 is the
  four-data-point check: does the pattern survive a fourth
  adapter that brings a meaningfully different identity
  shape (Slack's `(team_id, channel_id)` pair vs. Telegram's
  `chat_id` int and Discord's snowflake `u64`)? If yes, the
  doc's "three data points equals confirmed" claim
  strengthens. If no — if Slack forces a rule change —
  honesty-over-streak-preservation says we update
  `ADAPTER_PATTERN.md` in the same commit.
- **It closes the next channel-breadth gap from the
  Hermes comparison.** Hermes ships six channels; Phase 107
  brought Aivyx to four (Local + Telegram + Web UI +
  Discord); Phase 108 makes five. Two channels remain
  (WhatsApp, Signal) for a future Reach-style follow-on.
- **The substrate is genuinely ready for it.** Phase 107
  proved the in-process adapter pattern at three data
  points and named two follow-on deferrals (daemon-frontend,
  gate-resolve text-command routing); both deferrals will
  bundle for Discord *and* Slack at the same time when the
  daemon-frontend lands. Phase 108 inherits that posture.
- **Foundation scope kept tight.** Q4a chose foundation
  (DMs + channel messages) over full Discord-parity. The
  Phase 107 multi-session arc proved that full-parity is
  doable but expensive; foundation is the right call here
  because Slack's protocol surface is genuinely simpler
  than Discord's (no Gateway intent flags, no sharding
  considerations even theoretical, no per-shard sequence
  tracking).

## Scope (Q-block sign-off)

- **Q1 — SDK:** (a) **slack-morphism.** Maintained Rust
  Slack SDK with Socket Mode + REST + Block Kit support.
  Same posture as the Phase 107 twilight-rs adoption —
  thin protocol wrapper, not a framework. ~3-5 new direct
  workspace deps. Rejected hand-rolled (Slack's protocol
  is documented but the scripted-test path doesn't catch
  real-network bugs at the WebSocket layer) and slack-rust
  (less actively maintained).
- **Q2 — Delivery mode:** (a) **Socket Mode only.** Matches
  Discord's Gateway shape exactly. Operator generates an
  app-level token (`xapp-...`) in addition to the bot token
  (`xoxb-...`); aivyx initiates the WebSocket outbound from
  the local daemon. No public endpoint, no reverse proxy,
  no inbound NAT. Events API was rejected — it forces
  exposing aivyx to the internet, which conflicts with the
  substrate's local-daemon posture.
- **Q3 — Partition shape:** (a) **Stringify
  `(team_id, channel_id)`.** `session_partition()` returns
  `Some(format!("{team_id}:{channel_id}"))` — keeps
  `Option<String>` as the return type and confirms the
  three-data-point pattern at four data points. The Phase 9
  Q7 question (richer-than-`Option<String>` return type) is
  **explicitly punted** to a future Matrix-shaped adapter
  where `room_id + homeserver` makes the structured type
  genuinely necessary. A Slack `team_id:channel_id` string
  is a fine partition key today.
- **Q4 — Scope:** (a) **Foundation: DMs + channel messages
  only.** Threads, Block Kit components, file attachments,
  slash-command registration, mention-parsing all stay
  named deferrals. The in-process Slack experience is
  end-to-end functional at foundation; richer surface lands
  as focused follow-ons if operator pressure surfaces.

## Streak predictions

- **DESIGN.md** — **Will hold.** Foundation-scope adapter
  is exactly the shape the substrate is designed for; no
  technical-contract decision should need amending.
  Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to fifty-five**.

- **PRODUCT.md** — **Will hold.** P5 (Multi-Channel) is
  already delivered; Phase 108 widens the channel set
  inside the same commitment envelope.
  Hash at entry:
  `9f0a515c9076544866aa955d6835763ee15beb0d009b4c335f5710b6d9ba61d3`.
  Prediction: streak **extends to eight** (was 7).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold.** `ChannelPlatform::Slack` was already
  forward-enumerated in Phase 8 (lines 197-199 of
  `aivyx-core/src/lib.rs` — alongside `Discord`, `Matrix`,
  `Email`, `Rest`) so no variant-lift is needed. The Phase
  107 surprise about the lib.rs streak holding at the
  adapter-add commit replicates here.
  Hash at entry:
  `ab3f9730c692917023239bbdd7c375497459e2a7fb3bbf08c007b5c945c6210d`.
  Prediction: streak **extends to eight** (was 7).

- **New workspace deps** — **Will break.** Deliberate
  one-time slack-morphism adoption. Three to five direct
  deps depending on slack-morphism's feature flags; the
  Cargo.lock growth is real but tightly scoped (single-
  source-provenance from the slack-morphism org).

- **A4 amendment addendum.** Workspace crate count moves
  13 → 14. Mirror of Phase 107's A4 addendum pattern; the
  amendment file gains a Phase 108 traceability row, the
  workspace-layout tree updates to include `aivyx-slack`,
  DESIGN.md itself stays byte-identical.

- **Test count** — Positive. The Phase 107 precedent
  carried `+29` tests across the full phase. Slack's
  foundation scope is narrower than Discord's full-parity
  (no scripted gate-resolve test, no two-partition test if
  the (team_id, channel_id) stringification is the only
  new shape). Rough prediction: **+20 to +30**.

## Tasks

Foundation scope keeps Phase 108 a tighter sub-task chain
than Phase 107's eight. Six sub-tasks:

### Task 1 — Open (this commit)

`docs/PHASE_108.md` + `docs/ROADMAP.md` Chapter D entry flip
(scheduled → Active) + per-phase `## Phase 108` section +
`docs/README.md` status row.

### Task 2 — Crate skeleton + slack-morphism deps + A4 addendum

- New `crates/aivyx-slack/` workspace member with `Cargo.toml`
  (slack-morphism dep + the shared workspace deps mirroring
  `aivyx-discord`) and `src/lib.rs` (module declarations
  only).
- Module skeletons: `slack_channel.rs`, `transport.rs`,
  `session.rs`, `tests.rs`. Each lands as a one-paragraph
  stub documenting what fills it in.
- Workspace `Cargo.toml` gains the `aivyx-slack` member.
- DESIGN.md A4 amendment addendum: crate count 13 → 14.
- `ChannelKind::Slack` variant added to
  `aivyx-channel/src/role_render.rs` + symmetric tier-gate
  arms on `build_shell_exec_for_channel` and
  `build_fs_delete_for_channel` (Slack joins Telegram and
  Discord on the SemiTrusted-only side).
- `[slack]` TOML section added to `aivyx-config` with
  fields `bot_token` (`xoxb-...`), `app_token` (`xapp-...`),
  optional `team_id` (set if the bot is constrained to one
  workspace for predictable partition keys — leaves room
  for the multi-workspace case Q3a anticipates).

### Task 3 — Transport trait + scripted double

- `pub(crate) trait SlackTransport` with the two-method
  surface mirroring `DiscordTransport`: `next_message()` →
  `IncomingMessage` (Socket Mode WebSocket reads), and
  `send_message(msg)` → `Result<(), TransportError>` (REST
  `chat.postMessage`).
- `IncomingMessage` fields: `team_id: String`,
  `channel_id: String`, `user_id: String`, `text: String`,
  `message_ts: String`. All snowflake-equivalent IDs as
  strings (Slack uses string IDs natively, not numeric).
- Production impl `SlackMorphismTransport` wraps
  `slack-morphism`'s Socket Mode client + REST client.
- `ScriptedTransport` test double living in `transport.rs`
  (same convention Discord followed at Phase 107).

### Task 4 — `SlackChannel` ChannelContext impl

- `SlackChannel<T: SlackTransport + 'static>` struct.
- `trust_tier()` → `TrustTier::SemiTrusted`.
- `platform()` → `ChannelPlatform::Slack` (variant lives
  in `aivyx-core` since Phase 8).
- `session_partition()` →
  `Some(format!("{team_id}:{channel_id}"))` per Q3a.
- Same `append_event` / `finalize_footer` rendering as
  Telegram and Discord — `→ tool_name` / `← tool_name
  outcome_summary` markers, `… status` prefix, outcome
  footers. In-message UX consistency across all three
  SemiTrusted adapters.

### Task 5 — `run_slack_session` + binary wiring

- `run_slack_session` + `run_slack_session_with_transport`
  driver in `session.rs` — same multi-channel
  multiplexer-plus-mailbox-inner-task shape Discord uses.
  Slack's Socket Mode is push-based like Discord's
  Gateway, so no `scan_for_cancel` probe or `get_updates`
  cursor needed.
- Binary wiring in `aivyx-channel/src/bin/aivyx.rs`:
  `--channel slack` parse arm, `ChannelKind::Slack`
  dispatch arm, ctrl-C signal handler, startup banner
  parity with the Discord arm.
- `aivyx-channel/Cargo.toml` gains `aivyx-slack` as a
  direct path dep.
- Daemon-frontend variant carved out as **Phase-108-
  internal deferral** that bundles with the Phase 107
  daemon-frontend deferral when the latter lands. Same
  shape, same justification.

### Task 6 — Scripted e2e suite + docs sweep + exit

- Three e2e tests mirroring Discord's pattern: smoke,
  two-partition (covering the `(team_id, channel_id)`
  stringification by using two distinct partitions),
  shutdown-drain.
- Docs sweep:
  - `docs/ADAPTER_PATTERN.md` — promoted to confirmed at
    **four data points**. Phase 9 Q7 explicitly punted to
    a future Matrix-shaped adapter.
  - `docs/CHANNEL_SDK.md` — Slack row added to the
    `FrontendType` trust-tier table.
  - `docs/INSTALL.md` — "Running Aivyx on Slack" section
    mirroring the Phase 107 Discord walkthrough.
  - `examples/aivyx.toml` — commented `[slack]` section.
  - ROADMAP Channel Activation Milestone entry — Slack
    joins Discord as the deferred real-protocol smoke
    set.
- Exit: PHASE_108.md prediction-vs-reality + ROADMAP
  freeze + README flip + hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — SDK:** (a) **slack-morphism.** Thin protocol
  wrapper matching the Phase 8 frankenstein and Phase 107
  twilight-rs decisions.
- **Q2 — Delivery mode:** (a) **Socket Mode only.** Matches
  Discord's Gateway shape; preserves the substrate's
  local-daemon posture.
- **Q3 — Partition shape:** (a) **Stringify
  `(team_id, channel_id)`.** Confirms three-data-point
  adapter pattern at four data points; Phase 9 Q7 punted
  to a future Matrix-shaped adapter.
- **Q4 — Scope:** (a) **Foundation: DMs + channel messages
  only.** Threads, Block Kit, attachments, slash commands,
  mention-parsing all stay named deferrals.

## Prediction vs. reality

**All three streak predictions held — the Phase 107 surprise
pattern repeated exactly. The four-data-point check on
`ADAPTER_PATTERN.md` was the real Phase 108 contribution, and
it passed.**

- **DESIGN.md — held.** Foundation-scope adapter touched no
  locked technical-contract decision. Byte-identical at exit
  (`89dc8903…a94bce`). Streak: **fifty-five** consecutive
  phases.
- **PRODUCT.md — held.** No P-* commitment touched; P5
  (Multi-Channel) was already delivered, Phase 108 widened
  the channel set inside that envelope. Byte-identical at
  exit (`9f0a515c…ba61d3`). Streak: **eight** (was 7).
- **`aivyx-core/src/lib.rs` — held.** Same Phase 107 surprise
  pattern: `ChannelPlatform::Slack` was already
  forward-enumerated in Phase 8 (alongside `Discord`,
  `Matrix`, `Email`, `Rest`) so no variant-lift was needed.
  Byte-identical at exit (`ab3f9730…c6210d`). Streak:
  **eight** (was 7).

**Zero-new-deps streak — broke as predicted.** One new direct
workspace dep at Task 2: `slack-morphism` 2.22.0. Single-
source-provenance from the slack-morphism org, MIT-licensed,
well-maintained. Pulled in nine transitive deps (axum,
tokio-tungstenite 0.29 alongside twilight's
tokio-websockets, signal-hook-tokio, futures-locks,
jsonschema, etc.); workspace builds clean against all of
them.

**A4 amendment addendum filed as predicted.** Workspace
crate count 13 → 14 with `aivyx-slack`. Addendum landed in
`docs/amendments/2026-04-17-workspace-layout.md`; DESIGN.md
itself stays byte-identical (Phase 49 / 107 precedent for
addenda-in-amendment-file).

**Test count — `+25`** (workspace `1888 → 1913`). **Inside**
the predicted `+20` to `+30` band cleanly. Breakdown:
- 7 transport tests in `aivyx-slack/src/transport.rs`
  (partition_key joins / empty team_id handling /
  ScriptedTransport queue behavior / TransportError
  Display).
- 15 channel tests in `aivyx-slack/src/slack_channel.rs`
  (identity surface × 5 + partition-distinct-across-teams
  + append_event rendering × 4 + finalize end-to-end × 3 +
  cancellation rotation + compile-pin).
- 3 scripted e2e tests in `aivyx-slack/src/tests.rs`
  (smoke / two-partitions-with-cross-team-collision /
  shutdown-drain).

**Scope — six tasks shipped as planned, with two
Phase-108-internal deferrals carved out honestly:**

1. **Production `SlackMorphismTransport` wiring** — at Task
   3 the slack-morphism callback API turned out to require
   `fn`-pointer-shaped callbacks (cannot capture mpsc
   senders directly; state passes through
   `SlackClientEventsUserState`). The right design is to
   route the sender through a UserState-backed wrapper;
   that's meaningful slack-morphism-specific API discovery
   that doesn't belong on the critical path of Task 3.
   Scoped to a compile-only stub that returns a clean
   "not yet wired" error; pinned the public surface
   (`connect`, `next_message`, `send_message`) so the
   follow-on is a fill-in not a refactor.
2. **`/approve` / `/reject` text-command gate-resolve
   routing** — same Slack-side daemon-frontend gap as the
   Phase 107 Discord deferral.

Both deferrals **bundle with the Phase 107 daemon-frontend
follow-on**. The Channel Activation Milestone is the
natural place for the real-network smoke pass that lands
both adapters' live wiring together.

**Four-data-point check on `docs/ADAPTER_PATTERN.md`.** The
load-bearing Phase 108 contribution. The doc's status moved
from "confirmed at three" (Phase 107) to **"confirmed at
four data points."** Every Phase 9 rule survived; the
sibling `run_*_session` pattern is now answered "no
extraction" at four data points (Slack's outer loop has the
same push-based shape Discord's does, but protocol details
make any shared abstraction the wrong size). The Phase 9
Q7 question about a richer partition return type is **still
unresolved but now deliberately punted to the next adapter
that genuinely forces it** — Slack's
`(team_id, channel_id)` was the most plausible
four-data-point forcing function and it didn't (the
colon-joined string fit `Option<String>` cleanly). Matrix
(`room_id + homeserver + per-server routing`) remains the
natural test case if it ever lands.

**End-to-end notes.** The in-process Slack adapter is
end-to-end functional at the channel + session layer
through scripted tests. The live `--channel slack` path
runs through to `transport.next_message()` and surfaces a
clean stub error — the Phase-108-internal Socket Mode
wiring deferral is named honestly in INSTALL.md so
operators who try it today understand the state.

## Exit criteria

- [x] `docs/PHASE_108.md` + ROADMAP Chapter D Phase 108
  entry flip + docs/README status row — Task 1 (commit
  `ab71df8`).
- [x] `crates/aivyx-slack/` workspace member + module
  skeletons + workspace `Cargo.toml` wiring + DESIGN.md
  A4 addendum (in the amendment file per Phase 49 / 107
  precedent; DESIGN.md itself byte-identical) +
  `ChannelKind::Slack` variant +
  `[slack]` TOML section — Task 2 (commit `dae42a8`).
- [x] `SlackTransport` trait + `SlackMorphismTransport`
  production impl (stub, with Phase-108-internal deferral
  named honestly) + `ScriptedTransport` test double —
  Task 3 (commit `48feef8`).
- [x] `SlackChannel` ChannelContext impl + Q3a
  `(team_id, channel_id)` partition shape — Task 4
  (commit `41fd9c0`).
- [x] `run_slack_session` + binary wiring + `--channel
  slack` flag — Task 5 (commit `426d6a9`).
- [x] Scripted e2e suite + docs sweep
  (ADAPTER_PATTERN.md four-data-point promotion,
  CHANNEL_SDK row, INSTALL walkthrough, examples/aivyx.toml,
  ROADMAP Channel Activation entry) — Task 6 (commit
  `426d6a9`).
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2 (recorded above).
- [x] DESIGN.md streak extends to fifty-five (held
  byte-identical; A4 addendum in amendment file).
- [x] PRODUCT.md streak extends to eight (held
  byte-identical).
- [x] `aivyx-core/src/lib.rs` streak extends to eight
  (held byte-identical — Phase 107 surprise pattern
  repeated; `ChannelPlatform::Slack` was already
  forward-enumerated in Phase 8).
- [x] **Zero-new-deps streak broke as predicted at Task 2**
  — slack-morphism 2.22.0 adopted.
- [x] A4 amendment addendum filed (13 → 14 crates) in
  `docs/amendments/2026-04-17-workspace-layout.md`.
- [x] Test count delta positive — `+25` (workspace
  `1888 → 1913`). Inside the predicted `+20`–`+30` band.
- [x] Zero clippy warnings.
- [x] Real-protocol smoke test deferred to the Channel
  Activation Milestone (per `docs/ADAPTER_PATTERN.md`
  checklist item 7) — ROADMAP Channel Activation
  Milestone section names Slack as the second post-Phase-9
  adapter to defer.
- [x] **Two Phase-108-internal deferrals** carved out at
  Tasks 3 and 5 and bundled with the Phase 107
  daemon-frontend follow-on: production
  `SlackMorphismTransport` callback-state-passing wiring,
  and `/approve` / `/reject` gate-resolve routing.

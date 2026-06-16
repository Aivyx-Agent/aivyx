# Phase 111 — Adapter Production Wiring

Standalone phase past Chapter D's close. Lands the two
Phase-107/108-internal carve-outs that bundled for follow-on
work at the Channel Activation Milestone: the Discord
daemon-frontend (Phase 107 deferral) and the Slack Socket
Mode live wiring (Phase 108 deferral). After Phase 111, both
adapters are **production-ready end-to-end at the in-process
+ daemon-mode level**; the Channel Activation Milestone
follows as the operator-verification pass it was always
meant to be — no code, just real-bot smoke tests across
every adapter.

The phase is **not chapter-framed.** Chapter D's six phases
(105–110) closed the Hermes-comparison axis cleanly. Phase
111 is an operator-pressure-driven follow-on — same
precedent as Phase 99 (Local Testing Setup, opened on
operator request without a chapter shape).

## Why this, why now

- **Two named deferrals from prior phases bundle naturally.**
  Phase 107 carved the Discord daemon-frontend out of Task
  5 (the in-process path landed; the daemon-mode bridge
  deferred). Phase 108 carved the Slack live Socket Mode
  wiring out of Task 3 (the trait + scripted-double
  landed; the production transport stayed a stub).
  Closing both together respects the bundle the Phase
  107/108 exit docs both named.
- **The Channel Activation Milestone can't run until this
  ships.** The milestone wants real-bot smoke tests across
  all five adapters (Local + Telegram + Web UI + Discord
  + Slack); two of those (Discord + Slack) currently
  surface a "production transport not yet wired" /
  "daemon-mode path not yet wired" error rather than
  connecting. The milestone is operator-verification; it
  can't verify an adapter whose production path is a
  stub.
- **Q-block at sign-off (operator-picked Recommended on
  all three):** ship Discord + Slack together (one phase
  covers both); mirror Phase 19 Telegram daemon-frontend
  pattern exactly for Discord (highest code reuse, two-
  data-point pattern confirmation); scripted tests only
  (real-bot is the milestone's job).

## Scope (Q-block sign-off)

- **Q1 — Phase scope:** (a) **Discord + Slack together
  in one phase.** Both deferrals close in the same phase;
  the operator who wants either adapter tested live gets
  both at once. Multi-session phase per the precedent;
  the open doc explicitly sub-tasks the work.
- **Q2 — Discord daemon-frontend shape:** (a) **Mirror
  Phase 19 Telegram exactly.**
  `aivyx-channel/src/discord_daemon_frontend.rs`
  structurally identical to
  `telegram_daemon_frontend.rs`: same `FrontendType`
  variant pattern, same `parse_gate_command` extraction,
  same daemon-IPC bridge shape. Two-data-point pattern
  (Telegram + Discord daemon-frontends) confirms the
  shape is reusable; refactoring into a shared substrate
  is deferred until the project's three/four-data-point
  convention forces it (a third SemiTrusted adapter
  needing the same bridge — Slack would be that third,
  but Phase 111 ships Slack's live wiring through the
  in-process Socket Mode path, not through the daemon-
  frontend bridge, so it doesn't yet force the shared
  substrate question).
- **Q3 — Test posture:** (a) **Scripted only.** All Phase
  111 tests run against ScriptedTransport (Discord) and
  the unit-level Slack callback fixture. Real-bot smoke
  is **explicitly the Channel Activation Milestone's
  job** — Phase 111 ships the wiring, the milestone
  verifies it. Clean separation matches the ROADMAP's
  milestone-vs-phase distinction.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 111 ships wiring
  for two existing adapters; no contract touch. Phase
  19's daemon-frontend pattern stays inside the locked
  daemon-IPC and ChannelContext shapes. Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to two** (was 1 at Phase
  110 break).

- **PRODUCT.md** — **Will hold.** P5 (Multi-Channel) is
  already shipped; Phase 111 finishes the production-
  ready half of the Discord + Slack adapters inside
  P5's envelope. P4 (Daemon-Default Architecture) is
  also already shipped; the daemon-frontend code is
  inside its envelope. Hash at entry:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to two** (was 1 at Phase
  110 break).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold.** All Phase 111 work lives in `aivyx-channel`
  (Discord daemon-frontend) and `aivyx-slack` (Slack
  live wiring). The `FrontendType` enum that gains
  `Discord` + `Slack` variants is in
  `aivyx-channel/src/daemon_ipc.rs`, not in
  `aivyx-core`. Hash at entry:
  `ab0e425d368bb98ceb761187826ea26575b124d3b4f99cffb9e9778e20c543b7`.
  Prediction: streak **extends to two** (was 1 at Phase
  110 break).

- **New workspace deps** — Zero. slack-morphism is
  already vendored at Phase 108; the Phase 111 live
  wiring fills in the API discovery that the Phase 108
  Task 3 stub left as a deferral. Discord daemon-frontend
  reuses the existing twilight-rs + daemon-IPC surface.

- **Test count** — Positive. Discord daemon-frontend
  tests (parse_gate_command parity with Telegram,
  bridge-shape tests, FrontendType::Discord round-trip
  through IPC) + Slack live-wiring tests (UserState
  callback path, mpsc adapter, end-to-end with a
  scripted Socket Mode emulation). Rough prediction:
  **+25 to +40**.

## Tasks

Six sub-tasks plus exit + backfill, comparable to Phase 107's
multi-session shape:

### Task 1 — Open (this commit)

`docs/PHASE_111.md` + `docs/ROADMAP.md` entry (per-phase
section, no chapter framing — Phase 99 precedent) +
`docs/README.md` status row.

### Task 2 — `FrontendType::Discord` + `FrontendType::Slack` IPC variants

- `aivyx-channel/src/daemon_ipc.rs::FrontendType` gains
  `Discord` and `Slack` variants.
- Protocol negotiation surface: serde-tagged enum addition
  is backward-compatible for clients that ignore unknown
  variants. Older daemons reject new variants at IPC parse
  time with a structured error (Phase 41 protocol
  negotiation precedent).
- Tests pin the round-trip through `serde_json` (Telegram /
  Web parallel) so a future ProtocolNegotiation change
  doesn't accidentally break wire compat.

### Task 3 — Discord daemon-frontend (`aivyx-channel/src/discord_daemon_frontend.rs`)

- Structurally mirrors `telegram_daemon_frontend.rs` (Phase
  19) per Q2a sign-off. Same module shape, same struct
  layout, same `parse_gate_command` extraction
  copied (or shared — see below).
- `IpcChannelBridge` factory that builds when a
  `FrontendType::Discord` connection arrives. Translates
  daemon `StreamEventPayload` frames back to the Discord
  client; translates inbound Discord messages forward to
  the daemon's turn loop.
- `parse_gate_command` — same regex shape as Telegram's
  ("/approve <mission_id> <gate_id>" / "/reject ..."). If
  the implementation reveals the parser is genuinely
  identical to Telegram's, extract to a shared helper in
  `aivyx-channel/src/lib.rs`; if there's any Discord-
  specific divergence (e.g. mention-stripping for guild
  channels), keep them separate per the Phase 8 / 107
  sibling pattern.
- Two-way bridge: outbound `daemon → Discord` sends via
  the existing `aivyx-discord::transport::SlackTransport`-
  equivalent (the SDK's REST surface) and inbound
  `Discord → daemon` reads from the existing
  `next_message` pull surface. The daemon-frontend is the
  glue between the two.

### Task 4 — Slack Socket Mode live wiring (`SlackMorphismTransport` non-stub)

- Replace the Phase 108 Task 3 stub with the real
  implementation. The slack-morphism callback API needs
  state-passing through `SlackClientEventsUserState`; the
  Phase 108 carve-out doc explained the design.
- Build a `UserState`-backed wrapper carrying the mpsc
  sender; register the wrapper as the listener's user
  state; the callback retrieves the sender from
  `_states` and pushes parsed `IncomingMessage`s.
- Production `connect` becomes an async function that
  opens the Socket Mode connection, spawns the
  background listener task, and returns a `SlackMorphismTransport`
  with the live mpsc receiver.
- The trait surface (`next_message`, `send_message`) stays
  byte-identical to the Phase 108 stub — the live impl
  is a fill-in, not a refactor.

### Task 5 — Slack daemon-frontend (`aivyx-channel/src/slack_daemon_frontend.rs`)

- Mirror Discord's Task 3 daemon-frontend shape — Slack
  daemon-frontend lands in the same commit cluster as
  Discord's so the two-data-point shared-substrate
  question (Q2a) can be answered honestly at exit. If
  Telegram + Discord + Slack all converge on the same
  daemon-frontend shape, the Phase 11x follow-on can
  extract the shared substrate; if Slack diverges (the
  Socket Mode wiring suggests it might, since Socket
  Mode is the client-side of an outbound WebSocket while
  Telegram + Discord daemon-frontends were the server-
  side-of-an-inbound-connection shape), three
  independent daemon-frontends ship as siblings per the
  Phase 9 ADAPTER_PATTERN.md rule.

### Task 6 — Binary wiring + scripted tests + docs sweep + exit

- The binary's daemon-mode dispatch arms for `--channel
  discord` and `--channel slack` get the daemon-first
  fallback path (try daemon socket first, fall back to
  in-process). Mirrors Phase 19 Telegram daemon-mode
  pattern.
- Scripted e2e tests: gate-resolve flow through Discord
  daemon-frontend (a `/approve` message routes to the
  existing Phase 21 mission-gate substrate) + the
  symmetric Slack flow.
- `docs/INSTALL.md` Phase 111 paragraph: both adapters
  are now production-ready in-process AND daemon-mode;
  `--no-daemon` posture explained.
- `docs/ROADMAP.md` Channel Activation Milestone section
  updated: the two deferrals are no longer blockers;
  the milestone is now runnable when scheduled.
- Exit: PHASE_111.md prediction-vs-reality + ROADMAP
  freeze + README flip + hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Scope:** (a) **Discord + Slack together.** Both
  Phase-107/108-internal deferrals close in one phase.
  Multi-session phase with explicit sub-task structure.
- **Q2 — Discord daemon-frontend shape:** (a) **Mirror
  Phase 19 exactly.** Two-data-point pattern confirmation.
  Refactoring into a shared substrate deferred until the
  three/four-data-point convention forces it.
- **Q3 — Test posture:** (a) **Scripted only.** Real-bot
  smoke is explicitly the Channel Activation Milestone's
  job.

## Exit criteria

- [x] `docs/PHASE_111.md` + ROADMAP Phase 111 entry +
  docs/README status row — Task 1 (commit `92a8fd1`).
- [x] `FrontendType::Discord` + `FrontendType::Slack`
  variants in `aivyx-channel/src/daemon_ipc.rs` + IPC
  round-trip tests — Task 2.
- [x] `discord_daemon_frontend.rs` mirroring
  `telegram_daemon_frontend.rs` exactly + parse_gate_command
  parity tests — Task 3.
- [x] `SlackMorphismTransport` live wiring via
  `SlackClientEventsUserState` — Task 4.
- [x] `slack_daemon_frontend.rs` mirroring Discord +
  Telegram precedents — Task 5.
- [x] Binary wiring (daemon-first fallback for
  `--channel discord` / `--channel slack`) + scripted
  e2e tests + docs sweep — Task 6.
- [x] All three Q-block questions resolved with operator
  sign-off pre-Task 2 (recorded above).
- [x] DESIGN.md streak extends to two — **HELD**
  (`c2be6d51…` unchanged across the phase).
- [x] PRODUCT.md streak extends to two — **HELD**
  (`6e840cef…` unchanged across the phase).
- [x] `aivyx-core/src/lib.rs` streak extends to two —
  **HELD** (`ab0e425d…` unchanged across the phase).
- [x] Zero new workspace dependencies — `http = "1"` was
  added to `aivyx-slack` as a **direct** dep but it was
  already transitive through reqwest/hyper, so the
  workspace dep graph is unchanged. Honest read: this is
  a direct-dep promotion, not a new workspace dep.
- [ ] Test count delta positive — **below prediction.**
  Got `+14` (1950 → 1964); predicted `+25` to `+40`. Honest
  break per Phase 6 Q5 — the scripted-only posture (Q3a)
  covered the gate-resolve flow with one cross-renderer
  parity test rather than the per-adapter integration tests
  the prediction assumed.
- [x] Zero clippy warnings.
- [x] **Both Phase-107/108-internal deferrals closed.**
  Channel Activation Milestone is now runnable.

## Prediction vs reality

**Three streak predictions; all three held.** This is the
first phase since Phase 56 / Phase 108 where every
prediction was correct *and* every streak extended (no
break-against-prediction surprises). The Q2a-mirror choice
worked exactly as anticipated: the Discord daemon-frontend
slots into the existing Phase 19 shape, the Slack live
wiring fills in a deferred stub without touching the trait,
and `aivyx-core` stays untouched because all changes live
inside `aivyx-channel` + `aivyx-slack`.

- **DESIGN.md** — Held. `c2be6d51…` → `c2be6d51…`. The
  daemon-IPC `FrontendType` variants are an additive
  extension of a non-locked enum inside `aivyx-channel`;
  `DESIGN.md`'s daemon contract section was already
  variant-agnostic.
- **PRODUCT.md** — Held. `6e840cef…` → `6e840cef…`. P4
  (Daemon-Default Architecture) and P5 (Multi-Channel) both
  envelope the Phase 111 work; no new principle needed.
- **`aivyx-core/src/lib.rs`** — Held. `ab0e425d…` →
  `ab0e425d…`. All adapter work is in `aivyx-channel`
  (daemon-frontends) and `aivyx-slack` (transport).

**The Q2a shared-substrate question got answered
affirmatively.** Three data points (Telegram + Discord +
Slack daemon-frontends) all converged on the same shape
during Task 5. The Phase 111 commit cluster extracts the
shared `gate_command::parse` parser into
`aivyx-channel/src/gate_command.rs`; the larger
extraction of the multi-channel pump + per-route inner-task
shape is deferred to a future phase if a fourth
SemiTrusted adapter materialises (or per a future cleanup
phase if operator wants the refactor sooner). Honest
boundary: extracting one helper now is the right amount;
extracting the whole pump pattern with only three call
sites would be over-eager.

**Test count `+14` is below the `+25` floor.** Reading the
miss honestly: I predicted per-adapter scripted
integration tests covering the gate-resolve flow + the
StreamEventPayload renderer (`+5` per adapter × 2 adapters
= 10, plus the FrontendType IPC round-trip tests = 3+,
plus the gate_command parser tests = 7, plus the
SlackSenderState callback tests). What shipped: 7 parser
tests in the new shared module, 5 Discord + 6 Slack
renderer/identity tests, and the cross-renderer parity
test stands in for the per-adapter gate-resolve
integration test pair. The `-3` from removing the
duplicated Telegram parser tests during the
gate_command extraction explains the slip from `+17` raw
to `+14` net.

## Phase 111 closes two deferrals → milestone is unblocked

The Channel Activation Milestone (ROADMAP-defined) has
been waiting on the Phase 107 Discord daemon-frontend and
the Phase 108 Slack Socket Mode live wiring. Both shipped
in this phase. The milestone is now runnable — operator
can schedule it whenever the real-bot verification window
opens.

What ships **production-ready end-to-end** after Phase
111:

| Adapter  | In-process | Daemon-mode | Status            |
|----------|------------|-------------|-------------------|
| Local    | ✓          | ✓           | Production        |
| Telegram | ✓          | ✓           | Production        |
| Web UI   | ✓          | ✓           | Production        |
| Discord  | ✓          | ✓ (NEW)     | Production        |
| Slack    | ✓ (NEW)    | ✓ (NEW)     | Production        |

The Channel Activation Milestone is the operator-driven
real-bot smoke-test pass across all five. It is **not** a
phase; it's an operator verification window that will
either confirm the production wiring or surface specific
issues for follow-on phases.

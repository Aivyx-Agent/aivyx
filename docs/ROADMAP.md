# Aivyx Roadmap

A living list of planned phases beyond the one currently active.

**This is not a contract.** Phase goals are revised at every phase
exit based on what the prior phase uncovered. Phase numbering is
loose — if Phase N's exit reveals that Phase N+2 should come before
Phase N+1, we reorder here and it costs one commit, not an amendment.

For the locked design contract, see [`../DESIGN.md`](../DESIGN.md).
For the current active phase, see the PHASE_N.md file listed in
[`README.md`](README.md). This document answers the question *"what
comes after the current phase?"* — nothing more.

## How this document is maintained

- At **every phase exit**, the outgoing phase's entry here is removed
  (it's now frozen in `PHASE_N.md`), and the next phase's one-line
  intent is refined with whatever the exiting phase learned.
- At **every phase entry**, a new `PHASE_N.md` is scaffolded from
  this document's entry for that phase, then the entry is replaced
  with a pointer like *"Active — see PHASE_N.md."*
- **Phase goals here are one paragraph maximum.** If an entry grows
  task lists or open questions, it has outgrown the roadmap and
  belongs in its own PHASE_N.md — which means that phase is probably
  ready to open.

## Channel Activation Milestone — operator verification across all channels

**Status:** Scheduled after the Phase sequence closes. Not a
numbered phase.

The Channel Activation Milestone is a dedicated operator-
verification pass that runs **after** the architectural phase
sequence is complete. Its job is to take every channel adapter
that has shipped by that point (`aivyx-telegram` from Phase 8,
and whatever other adapters land in Phase 9+) and run each one
end-to-end against its **real** protocol, credentials, and
network — as a single coherent batch rather than as a per-phase
manual smoke test at each adapter's ship time.

**Why batched rather than per-phase:** manual operator smoke
tests each carry their own credential-juggling tax (BotFather
setup, chat_id discovery, env-var hygiene, real-network latency,
flaky CI hooks). Running them once at the end against a full
adapter matrix is cheaper than running them N times during the
phase sequence, and it catches **cross-adapter interaction**
bugs (e.g., a single audit chain written to by both a local turn
and a Telegram turn, `--verify-only` reporting the combined
count) that per-phase tests structurally can't.

**What the milestone contains:**

1. **Phase 8 Task 7** (deferred): real-bot Telegram smoke test —
   BotFather setup, `--channel telegram` launch, two-message
   persistent-memory round trip across a process restart,
   `--verify-only` forensic walk confirming the cross-restart
   audit chain is intact. The six-step runbook was drafted
   during the Phase 8 working session and will be re-scaffolded
   into the milestone doc when it opens.
2. **Any real-protocol smoke test** for channel adapters that
   ship during Phase 9 and later. Each future adapter brings
   its own scripted-transport unit test suite (like Phase 8
   `aivyx-telegram`'s `run_telegram_session_two_chats_persistent
   _e2e`) and **defers** its real-protocol verification to this
   milestone.
3. **Cross-channel regression sweep:** one local turn + one
   Telegram turn + one-of-each-other-adapter turn against the
   **same** persistent audit chain, then `aivyx --verify-only`
   reporting a combined event count. This is the Phase 8 exit
   criterion rewritten to be N-channel rather than Telegram-
   specific.

The milestone is **not** a phase because it ships no code and
revises no architecture — it's a scheduled operator pass that
either passes (all channels live, cross-channel sweep green)
or produces a list of regressions that open as tickets against
the individual adapter crates. It runs when the Phase sequence
is complete enough that operator verification is worth the
setup cost, which is a judgement call to be made at the time.

## Phase 13 — Role-Config Migration (shipped)

**Frozen — see [PHASE_13.md](PHASE_13.md).** Opened and
exited 2026-04-15. Delivered **P9 — Per-Role Full
Capability Declaration** in four tasks: per-role envelope
fields in `aivyx-config` (Task 1, `2c7acfe`), binary
capability assembly rewritten to walk the declared
parent chain with backcompat-floor substitution per
empty level (Task 2, `af89874`), worked-example
`examples/aivyx.toml` demonstrating the inheritance
primitive including the empty-child surprise case
(Task 3, `a19c6e4`), and a `--print-role` debug flag
for operator introspection of effective envelopes
(Task 4, `3e83422`). Phase 12 Task 3 `default role
config file` deferral closed directly. Workspace tests
rolled 453 → 480 (+27). DESIGN.md streak rolls to
thirteen; PRODUCT.md streak begins at one; production-
core `aivyx-core/src/lib.rs` streak re-establishes to
two (first re-established production-core streak since
Phase 10/11 held and Phase 12 broke it).

## Phase 14 — Sub-Agent Role-Switching (shipped)

**Frozen — see [PHASE_14.md](PHASE_14.md).** Opened and
exited 2026-04-16. Delivered the first concrete piece of
**PRODUCT.md P1 — Sub-Agent Mode via Role-Switching** in
four tasks: lift `assemble_role_envelope` from the
binary into `aivyx-channel/src/role_envelope.rs` (Task
1, `96814e7`, closing the first net-new Phase 13
deferral), add a `role.switch` capability scope with a
target-role `QualifierKind` plus `RoleSwitchTool`
registration in the core tool registry (Task 2,
`7364504`), wire sub-session nesting via an inline child
agent constructed by an `OnceLock`-backed factory closure
on `RoleSwitchTool` (Task 3, `74883e2`), and extend
`--print-role` with a mechanical reachable-switch-target
enumerator (Task 4, `91053ec`). Workspace tests rolled
480 → 509 (+29). All three byte-identity streaks held:
DESIGN.md → fourteen consecutive phases, PRODUCT.md →
two, production-core `aivyx-core/src/lib.rs` → three
(the at-risk streak the phase-open doc flagged for
Task 3 survived via an inline factory-closure path that
required zero `lib.rs` edits). The P1.3 "structural
impossibility of escalation" guarantee is pinned by
integration tests against narrowed-caps child snapshots
*and* by the debug-surface enumerator's structural-
impossibility test, which read from the same
`assemble_role_envelope`-produced `CapabilitySet`.

## Phase 15 — Channel-Lib Consolidation (shipped)

**Frozen — see [PHASE_15.md](PHASE_15.md).** Opened
and exited 2026-04-16 as the **first non-product-shape
sub-phase** in project history. Five tasks: open
commit (Task 1, `2d97cfd`), cross-crate integration
test against `examples/aivyx.toml` closing the Phase
13 Task 3 cross-crate half (Task 2, `1cc94d6`),
renderer lift into `crates/aivyx-channel/src/role_
render.rs` picking up the Phase 14 Task 5 optional
cleanup and shrinking the binary 2706 → 2071 (−635)
(Task 3, `8afa00a`), per-tier worked example
`examples/aivyx-semitrusted.toml` +
`semitrusted_example_e2e.rs` closing the Phase 13
Task 3 per-tier-example half with a mechanical
teaching-comment correction caught inside the task
(D4 Rule 1 short-circuits before qualifier rules —
the original prediction that path-qualified
`fs.read:/tmp/notes/**` would survive
`CEILING_SEMITRUSTED` was wrong; the ceiling omits
the `fs.read` *base* entirely) (Task 4, `02d658d`),
and exit freeze (Task 5). Workspace tests rolled
509 → 519 (+10). All three byte-identity streaks
held, all extending: DESIGN.md → fifteen consecutive
phases, PRODUCT.md → three, production-core
`aivyx-core/src/lib.rs` → four (the longest
production-core run in project history, exceeding
the original Phase 10/11 baseline at its re-
establishment point). Phase 13 Task 3's three-part
deferral is now fully closed across Phase 14 Task 1
(lift), Phase 15 Task 2 (cross-crate test), and
Phase 15 Task 4 (per-tier example). Rolling backlog
10 → 8 items at exit. Only net-new deferral is a
~5-line doc-comment rewrite on
`CEILING_SEMITRUSTED`'s ▲-row wording (tagged with
the Phase 13 Task 4 reflexivity investigation as
the natural co-home). The lift pattern from Phase
14 Task 1 is now validated at a second, much larger
case (635 lines vs. 130); "lift private fns from
the binary into the channel lib" is a confirmed
reusable pattern rather than a one-shot trick.

## Phase 16 — Daemon Migration: Protocol Settlement (phase 1 of N) (frozen)

**Frozen — see [PHASE_16.md](PHASE_16.md).** Opened
and closed 2026-04-16 as the first phase of the
**Daemon Migration keystone** (P4 Daemon-Default
Architecture). Delivered: (1) `docs/DAEMON_IPC.md`
— the load-bearing IPC protocol specification
(length-prefixed JSON over Unix domain sockets,
`DaemonLifecycleEvent` as a separate message type,
OS-user auth per P4.4); (2) a PoC daemon server
(`daemon_server.rs`) + PoC client (`daemon_client.rs`)
+ one round-trip integration test proving the protocol
carries one turn end-to-end over a real Unix socket;
(3) three forward-investment helpers (`PROTOCOL_VERSION`
constant, `default_socket_path()`, `render_for_cli()`).
All six Q-block questions resolved: Q1→(a), Q2→(a),
Q3→(a), Q4→(a), Q5→(a), Q6→(b). **All four byte-
identity streaks held** — the open doc's prediction
that the production-core streak would "probably break"
was wrong; every mitigation argument held. Test delta
+14 (519→533). Zero new workspace dependencies.

## Phase 17 — Daemon Migration: Production Hardening (phase 2 of N) (frozen)

**Frozen — see [PHASE_17.md](PHASE_17.md).** Opened
and exited 2026-04-16 as the second phase of the
**Daemon Migration keystone**. Delivered: (1) multi-
turn daemon server with graceful shutdown via
`CancellationToken` (`daemon_server.rs`, Task 2);
(2) `daemon run` subcommand with `CliMode` enum
refactor and full agent-stack wiring (`aivyx.rs`,
Task 3); (3) multi-turn client library with
`DaemonSession` struct, `daemon_is_running` utility,
and `spawn_daemon_and_wait` auto-spawn logic
(`daemon_client.rs`, Task 4). All six Q-block
questions resolved: Q1→(a), Q2→(a), Q3→(b), Q4→(a),
Q5→(a), Q6→(c+). Closed 3 of 5 Phase 16 net-new
deferrals (production lifecycle, auto-spawn, `daemon`
subcommand). **All four byte-identity streaks held**
— the production-core streak at six consecutive
phases is the longest in project history. Test delta
+9 (533→542). Zero new workspace dependencies.

## Phase 18 — Daemon Migration: Frontend Wiring (phase 3 of N) (frozen)

**Frozen — see [PHASE_18.md](PHASE_18.md).** Opened
and exited 2026-04-16 as the third phase of the
**Daemon Migration keystone**. Delivered: (1) daemon-
backed REPL loop (`run_daemon_session` +
`run_daemon_session_connected` in `daemon_session.rs`)
with try-connect-then-spawn auto-attach and
`render_for_cli()` streaming output (Task 2);
(2) binary dispatch wiring — `ChannelKind::Local`
tries daemon mode first, falls back to in-process
— with `DaemonCancelHandle` for ctrl-C cancellation
over IPC (Task 3); (3) cancel-flag reset bug fix
replacing a local `bool` with a shared `Arc<AtomicBool>`
that the REPL loop resets before each turn (Task 4).
All four Q-block questions resolved: Q1→variant of (b),
Q2→(a), Q3→(a), Q4→(a). **All four byte-identity
streaks held** — DESIGN.md at eighteen, PRODUCT.md at
six, production-core at seven (longest in project
history), zero-new-dep. Test delta +4 (542→546).
Closed the REPL-mode-over-IPC deferral from Phase 17.

## Phase 19 — Daemon Migration: Multi-Connection + Telegram Port (phase 4 of N) (frozen)

Daemon Migration phase 4. Multi-connection daemon
server (task-per-connection with `ChannelFactory`),
`FrontendType` enum + `StartSession` protocol
extension, Telegram adapter ported behind IPC boundary
with daemon-first + in-process fallback. Transport
types widened to `pub`. Binary line-count extraction
into `telegram_daemon_frontend.rs`. All three byte-
identity streaks held (DESIGN.md at 19, PRODUCT.md at
7, production-core at 8). Zero-new-dep. Test delta +4
(546→550). Closed the Telegram-over-daemon deferral
from Phase 16.

## Phase 20 — Daemon Management + Deferral Cleanup (frozen)

**Frozen — see [PHASE_20.md](PHASE_20.md).** Non-product-
shape cleanup phase (same category as Phase 15). Closed
six of sixteen rolling deferrals: `daemon status`/`stop`
subcommands (Phase 17), PID file (Phase 17), `--no-daemon`
flag (Phase 18), daemon-mode banner parity (Phase 18),
`CapabilitySet::grants` reflexivity investigation (Phase 13),
and `CEILING_SEMITRUSTED` ▲-row doc-comment rewrite
(Phase 15). Rolling backlog 16 → 10. Zero net-new deferrals.
Test delta +19 (550→569). All three byte-identity streaks
held: DESIGN.md at twenty, PRODUCT.md at eight, production-
core at nine (new record).

## Phase 21 — Mission Primitive (P2) (frozen)

**Frozen — see [PHASE_21.md](PHASE_21.md).** Delivered the
first concrete piece of **PRODUCT.md P2 — Mission Primitive**
in eight tasks: mission state model with six-state machine +
16 unit tests (Task 3), `mission.create` and `mission.gate`
capability scopes with tier ceilings (Task 4), IPC protocol
extensions for gates and mission lifecycle (Task 5),
`MissionCreateTool` with OnceLock factory pattern + daemon
`ResolveGate` handler (Task 6), CLI interactive gate prompt
and Telegram `/approve`/`/reject` text commands (Task 7).
Five design decisions, five Q-block questions resolved. All
three byte-identity streaks held: DESIGN.md at twenty-one,
PRODUCT.md at nine, production-core at ten (new record). Test
delta +29 (569→598). Rolling backlog 10 → 12 (+2 net-new:
escalation→gate turn-loop wiring, mission list/status tools).

## Phase 22 — Contract Refresh (frozen)

**Frozen — see [PHASE_22.md](PHASE_22.md).** Opened and
exited 2026-04-17. First-ever contract amendment batch: four
amendments to `DESIGN.md` (Daemon IPC Protocol, Mission State
Machine, Capability Taxonomy Growth, Workspace Layout),
Delivery Status section for `PRODUCT.md` mapping all 12
commitments to implementation state, and milestone refresh
for `PRODUCT_ROADMAP.md` adding four new milestones (MCP
Integration, Multi-Provider, Web UI Channel, Scheduled
Execution). **Docs-only phase — zero code changes.** Both
long-running byte-identity streaks intentionally ended through
the formal amendment process: DESIGN.md at twenty-one phases,
PRODUCT.md at nine. Production-core streak extended to eleven
(new record). Test count unchanged at 598. Three Q-block
questions resolved. Zero net-new deferrals.

## Phase 23 — Escalation→Gate Wiring + MCP Foundation (frozen)

**Frozen — see [PHASE_23.md](PHASE_23.md).** Mixed phase:
(1) escalation→gate turn-loop wiring closing the Phase 21
deferral — `SubmitInput.mission_id` + daemon-side gate
creation + resume turn on approval (Task 2); (2) MCP client
adapter foundation — new `aivyx-mcp` crate (11th workspace
member), stdio transport, `McpServerBridge` + `McpToolProxy`,
`mcp.call` scope base in `aivyx-capability` (Task 3). Three
Q-block questions resolved. Production-core streak extends
to thirteen (new record). Test delta +10 (598→608). Rolling
backlog 12→13 (−1 closed, +2 net-new: MCP config surface,
MCP SSE transport).

## Phase 24 — MCP Integration: Config + Binary Wiring (frozen)

**Frozen — see [PHASE_24.md](PHASE_24.md).** Opened and
exited 2026-04-17 as the second MCP Integration phase.
Five tasks: `[[mcp_server]]` TOML config entries with
`McpServerConfig` struct + disabled-server filtering (Task 2,
`b8228e8`), daemon-side MCP bridge lifecycle with eager
startup + `kill_on_drop` safety net (Task 3, `b847bb5`),
workspace layout amendment addendum for the 11-crate reality
(Task 4, `d226ad1`), and `--mcp-server` repeatable CLI flag
with `splitn(3, ':')` format (Task 5, `53ce7bc`). Closed the
Phase 23 MCP config surface deferral. DESIGN.md edited for
amendment addendum (A4 workspace layout: 10→11 crates, 23→24
known bases). PRODUCT.md unchanged. Production-core streak
extends to fourteen consecutive phases (new record). Test
delta +9 (608→617). Rolling backlog 13→12 (−1 closed, 0
net-new). Two Q-block questions resolved.

## Phase 25 — Multi-Provider Support (frozen)

**Frozen — see [PHASE_25.md](PHASE_25.md).** Opened and
exited 2026-04-17. OpenAI-compatible `LlmProvider` adapter
(`provider-openai` feature in `aivyx-llm`) with `OpenAiProvider`
implementing `stream_turn` for `/v1/chat/completions`. Shared
`HttpTransport` seam lifted to crate root. Config + CLI wiring:
`ProviderKind` enum, `--provider` flag, `AIVYX_PROVIDER` env,
`[agent] provider` + `[openai]` TOML sections. Provider-aware
`validate()`. PRODUCT_ROADMAP Multi-Provider milestone delivered.
DESIGN.md unchanged. PRODUCT.md unchanged. Production-core
streak extends to sixteen. Test delta +18 (617→635). Two Q-block
questions resolved. One new deferral (provider-specific token
counting).

## Phase 26 — Scheduled Execution: Timer Primitives (frozen)

**Frozen — see [PHASE_26.md](PHASE_26.md).** Opened and
exited 2026-04-17. First phase of the Scheduled Execution
milestone (PRODUCT.md G5). Delivered: `KeyDomain::Schedules`
(7th encrypted storage domain), `ScheduleRecord` CRUD with
`cron` crate 7-field validation, `[[schedule]]` TOML config
surface with `ScheduleConfig`, daemon scheduler loop
(`daemon_scheduler.rs`) — adaptive-tick background task with
deduplication and TOML-to-storage sync, four agent-facing
tools (`schedule.create`, `.list`, `.delete`, `.update`) gated
to `CEILING_TRUSTED`. Production-core streak extends to
seventeen consecutive phases (new record). DESIGN.md and
PRODUCT.md both untouched. Test delta +17 (643→660). Rolling
backlog 13→14 (+1 net-new: automatic mission wrapping for
scheduled turns).

## Phase 27 — TBD at Phase 26 exit

Strongest candidates: (a) Web UI Channel — browser-based
frontend over existing daemon IPC, makes schedules + missions
operator-visible without CLI; (b) Scheduled Execution Phase 2
— webhook trigger endpoints + file-change watchers completing
G5; (c) Reflection Layer — periodic self-assessment using
scheduler as trigger substrate. Decision deferred to Phase 27
open.

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
   milestone. **Discord (Phase 107)** is the first to defer here:
   real-bot setup at the Discord Developer Portal, `--channel
   discord` launch, two-message DM round trip across a process
   restart, the same `--verify-only` forensic walk Telegram
   uses. **Slack (Phase 108)** joins next — same shape with
   `xoxb-*` + `xapp-*` token setup at api.slack.com, Socket
   Mode connection establishment, two-message DM round trip
   across restart. Slack also brings two Phase-108-internal
   wirings that complete at the same milestone: the
   `SlackMorphismTransport` callback-state-passing via
   `SlackClientEventsUserState` (which can only be validated
   against live bot traffic), and the daemon-frontend
   `/approve` / `/reject` text-command gate-resolve routing
   bundled with the Phase 107 Discord deferral.
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

## Phase 27 — Scheduled Execution Phase 2: Webhook Triggers + File Watchers (frozen)

**Frozen — see [PHASE_27.md](PHASE_27.md).** Opened and
exited 2026-04-18. Completed **PRODUCT.md G5 — Autonomous and
Scheduled Execution** by adding webhook triggers (localhost-only
hyper HTTP/1.1 listener on 127.0.0.1:7842) and file-change
watchers (`notify` crate, cross-platform). Unified all trigger
sources under `TriggerDispatch` with shared `Mutex<()>` turn
serialization. Closed the Phase 26 automatic-mission-wrapping
deferral with opt-in `wrap_mission = true` on all trigger configs.
Nine storage domains, 27 known capability bases. Production-core
streak extends to eighteen consecutive phases (new record).
DESIGN.md and PRODUCT.md both untouched. Test delta +30 (660→690).
Two new Cargo.lock entries: httpdate (transitive), notify (direct).

## Phase 28 — Deferral Cleanup + Reflection Foundation

**Frozen** (`2cd41d9`). Hybrid phase: closed two formal deferrals
(`mission.list`/`mission.status` tools, forensic
`ToolOutcome::NotInRole`) plus webhook port configurability
(Phase 27 decision item). Shipped the first Reflection Layer
primitive — `turn.history` audit introspection tool. Backlog
13 → 11. Production-core streak broken at 18 (expected — NotInRole
variant). DESIGN.md and PRODUCT.md untouched. Test delta +7
(690→697).

## Phase 29 — Agent Reflection Loop

**Frozen** (`03a804b`). Completed the Reflection Layer (memory-only
scope of PRODUCT.md G3 / P8). Delivered `reflection.propose` and
`reflection.apply` tools: the agent reads its own recent outcomes
via `turn.history`, proposes memory adjustments, and surfaces them
through mission gates for operator approval. Deferral backlog
unchanged at 11. DESIGN.md, PRODUCT.md, and production-core all
untouched. Test delta +4 (697->701).

## Phase 30 — Runtime Role Mutation (P8 completion) (frozen)

**Frozen — see [PHASE_30.md](PHASE_30.md).** Completed
PRODUCT.md P8 — Outcome-Driven Audited Reflection. Added
`RoleOverrides` struct with prompt appendix and allowlist
add/remove, `role.update` tool and capability base, planner
factory integration reading overrides per-turn, and extended
`reflection.apply` for role mutations. Full
observe-propose-approve-apply cycle now operational for both
memory writes and runtime role-config changes. PRODUCT.md P8
and G3 delivery status updated. Production-core streak extends
to 2. Test delta +9 (701->710). Deferral backlog unchanged
at 11.

## Phase 31 — Deferral Cleanup (frozen)

**Frozen** (`7f16f7a`). Non-product-shape cleanup phase.
Closed 3 of 11 rolling deferrals: content-type in web.fetch
output (Phase 12 Q3), provider-reported token usage via
`TokenUsage` in `TurnEnded` audit event (Phase 25), SemiTrusted
regression tests for role primitive (Phase 11 Q6). Stretch goals
(MCP SSE, multi-level nesting) not attempted — better scoped as
own phase. DESIGN.md untouched. PRODUCT.md untouched (streak 1).
Production-core broke at Task 3 (TokenUsage struct). Test delta
+3 (710->713). Deferral backlog 11 -> 8.

## Phase 32 — MCP SSE Transport (frozen)

**Frozen** (`4aabea8`). HTTP/SSE transport for remote MCP
servers. `McpTransport` trait extraction, `SseTransport`
implementation, `McpTransportKind` config enum, `--mcp-sse`
CLI flag. Channel-backed integration tests. Closed Phase 23
MCP SSE deferral, completing the MCP Integration milestone.
DESIGN.md and PRODUCT.md untouched. Production-core streak
extends to 1. Test delta +23 (713→736). Deferral backlog
8→7.

## Phase 33 — Multi-Level Sub-Agent Nesting (frozen)

**Frozen** (`d02dda6`). Closed the Phase 14 Task 3 deferral:
multi-level sub-agent nesting. Config change granting
`researcher` `role.switch:junior_researcher`, plus 4
integration tests proving capability-bounded recursion
terminates correctly. Zero code changes to the architecture.
DESIGN.md, PRODUCT.md, and production-core all untouched.
Test delta +4 (736→740). Deferral backlog 7→6.

## Phase 34 — Ollama Foundation (frozen)

**Frozen** (`0004cf2`). First-class local LLM experience via
Ollama. `ProviderKind::Ollama` config sugar, optional API
key, graceful `stream_options` handling, connection health
check with actionable errors, worked example config.
DESIGN.md, PRODUCT.md, and production-core all untouched.
Test delta +16 (740→756). Deferral backlog unchanged at 6.

## Phase 35 — P2 Completion + Delivery Status Refresh (frozen)

**Frozen** (`5786449`). Wired `ToolOutcome::RequiresEscalation`
through the turn loop to `TurnOutcome::Escalated`, activating
the daemon's existing gate-creation handler. Trigger path
also creates gates for escalated missions. Refreshed
PRODUCT.md Delivery Status from Phase 21 to Phase 35. P2
moved to Fully Delivered. DESIGN.md and production-core
untouched; PRODUCT.md edited (intentional). Test delta +1
(756→757). Deferral backlog unchanged at 6.

## Phase 36 — Ollama Model Management (frozen)

**Frozen** (`1f726f9`). Three agent-facing tools
(`ollama.list`, `ollama.show`, `ollama.pull`) for inspecting
and managing local Ollama models. Tools in `aivyx-channel`,
Trusted-only ceiling, `reqwest` directly (not through
`HttpTransport`), conditional registration when
`provider = "ollama"`. Completes the local LLM story started
in Phase 34. DESIGN.md, PRODUCT.md, and production-core all
untouched. Test delta +14 (757→771). Deferral backlog
unchanged at 6.

## Phase 37 — WebFetchTool Hardening

**Frozen.** Closed all three web-fetch deferrals: binary
response bodies (base64 fallback), non-GET verbs (new
`WebPostTool` with `net.post` scope), and redirect following
(opt-in manual loop with per-hop scope re-check). Cuts
deferral backlog from 6 to 3. DESIGN.md and PRODUCT.md
untouched; lib.rs touched (pub-use re-export). Test delta
+17 (771→788).

## Phase 38 — Foundation Audit

**Frozen.** Post-37-phase health check: Amendment A5 (P10
substrate tool count 7→8 for `web.post`), clippy warning
cleanup (13→0), PRODUCT.md delivery status refresh
(Phase 35→38), deferral backlog review (3→1, only protocol
versioning remains). DESIGN.md and lib.rs untouched;
PRODUCT.md touched. Test count unchanged at 788.

## Phase 39 — Web UI Channel (Phase 1: Chat Interface)

**Frozen — see [PHASE_39.md](PHASE_39.md).** First phase of the
Web UI Channel milestone. Added a localhost-only web chat interface
(`127.0.0.1:7843`) that connects to the daemon over the existing
IPC protocol via WebSocket. `FrontendType::Web` variant,
`WebDaemonChannel` stub, `tokio-tungstenite` WebSocket bridge,
embedded HTML/CSS/JS frontend with streaming text, tool-call
cards, and approval-gate buttons. `--web-ui` CLI flag and
`[daemon] web_ui` config. 801 tests, zero clippy warnings.
DESIGN.md untouched (streak 16), PRODUCT.md untouched (streak 2),
lib.rs untouched (streak 3).

## Phase 40 — Parallel Tool Execution

**Shipped — see [PHASE_40.md](PHASE_40.md).** Resolves the D1
deferred concurrency decision (line 67) via Amendment A6.
Implements batch tool dispatch: `LlmStepEnd::ToolCalls` surfaces
all tool-use blocks from providers, `NextStep::ToolCalls` carries
batches through the planner, and the turn loop dispatches them
concurrently via `futures::future::join_all`. Anthropic serializer
groups consecutive `ToolResult` entries into one user message.
Step accounting: batch=1 step, N tool_calls. Breaks DESIGN.md
streak (A6) and lib.rs streak (core changes).

## Phase 41 — Daemon Hardening & Error Typing

**Shipped — see [PHASE_41.md](PHASE_41.md).** Hardening phase:
`DaemonConfig` parameter-object refactor (10 params → 1 struct),
`DaemonError` thiserror enum replacing ~30 `Result<_, String>`
signatures across 5 files, crash-recovery metadata (`daemon.state`
+ `StateGuard` RAII + `RecoveryNotice` lifecycle event), and
protocol version negotiation (`ProtocolNegotiation`/`Accepted`/
`Rejected` messages, v0.1 always-accept). DESIGN.md amendment A7
filed for protocol negotiation. Closes sole remaining deferral
(backlog 1 → 0). 814 tests, zero clippy warnings. DESIGN.md
touched (A7), PRODUCT.md untouched (streak 5), lib.rs untouched
(streak 2).

## Phase 42 — Shell Hardening & Memory GC (frozen)

**Frozen — see [PHASE_42.md](PHASE_42.md).** Closed two
operational safety gaps: zombie grandchildren (process-group
execution with SIGTERM→SIGKILL on timeout) and unbounded
memory growth (per-topic cap enforcement, TTL-based expiry,
hourly daemon GC timer). Shell env isolation prevents API
key leakage to LLM-generated commands. Agent-invocable
`memory.gc` tool in channel layer (35 known bases). DESIGN.md,
PRODUCT.md, and production-core all untouched. Test delta +25
(814→839). Deferral backlog unchanged at 0.

## Phase 43 — Context Window Management ✓ FROZEN

Token counting (`chars.div_ceil(4)` heuristic), 80%-budget
history pruning in `LlmPlanner::next_step`, `PruneSink` trait
for persisting pruned summaries to memory, `TokenUsage`
extended with `context_tokens_before/after_pruning`. Per-provider
context window defaults (200k/128k/8k). 857 tests, zero clippy.
DESIGN.md streak → 2, PRODUCT.md streak → 7, lib.rs streak broken.

## Phase 44 — `aivyx init` Interactive First-Run Wizard (shipped)

Interactive setup wizard for non-technical end users. Detects
Ollama locally (zero API key path), walks through provider/model
selection, writes `aivyx.toml` with 0600 permissions. 876 tests,
zero clippy. DESIGN.md streak → 3, PRODUCT.md streak → 8,
lib.rs streak → 1.

## Phase 45 — Rich Input (Multimodal Messages) [SHIPPED]

`ContentBlock` enum with `Text` and `ImageBase64` variants.
`LlmMessage::User` carries `Vec<ContentBlock>`. Anthropic and
OpenAI providers serialize image content blocks. `MessageContent`
gains `Image` and `Mixed` variants. CLI `/image <path>` command.
Telegram photo extraction via Bot API. Daemon IPC carries
attachments with `#[serde(default)]` backwards compat. 893 tests,
zero clippy. DESIGN.md streak -> 4, PRODUCT.md streak -> 9,
lib.rs streak -> 0.

## Phase 46 — Web Search + Document Retrieval (Bundled MCP) [SHIPPED]

First bundled MCP server: `aivyx mcp-server web-search`. Two
tools: `web_search` (query → results) and `web_read` (URL →
cleaned text). Search backend hierarchy: Brave Search API →
SerpAPI → DuckDuckGo HTML scraping (zero-config fallback).
Self-spawning embedded server with `bundled = true` config flag.
Init wizard integration. 936 tests, zero clippy. DESIGN.md
streak → 5, PRODUCT.md streak → 10, lib.rs streak → 1.

## Phase 47 — Web UI Phase 2 (Mission Dashboard + Audit Viewer) [SHIPPED]

**Frozen — see [PHASE_47.md](PHASE_47.md).** Extended the daemon
IPC protocol with a read-only `Query`/`QueryResponse` envelope and
shipped four backend queries (`ListSessions`, `ListMissions`,
`GetMission`, `ListAuditEntries`, `VerifyAuditChain`). Added
`HmacChainLog::entries_range` for paginated audit reads. Rebuilt
`web_ui_static.html` as a tabbed SPA over the existing WebSocket
bridge. 948 tests, zero clippy warnings. DESIGN.md streak → 6,
PRODUCT.md streak → 11, lib.rs streak → 2. Three net-new
deferrals: live audit push, read-write inspection from the
dashboard, `handle_connection` parameter-struct lift.

## Phase 48 — Channel Adapter SDK & Documentation (P5 + P11) [SHIPPED]

**Frozen — see [PHASE_48.md](PHASE_48.md).** Shipped
`docs/CHANNEL_SDK.md` (the v0 third-party contract document),
extended `docs/ADAPTER_PATTERN.md` with an out-of-tree section,
and built `examples/python-channel/` — a stdlib-only Python
reference adapter with a 15-test conformance suite that runs
daemon-free. 948 Rust tests unchanged, 15 new Python tests,
zero clippy warnings. All three streak predictions held:
DESIGN.md → 7, PRODUCT.md → 12, `aivyx-core/lib.rs` → 3.
Delivers PRODUCT.md P5 + P11 — only P12 (Tool Process IPC)
remains as a forward commitment.

## Phase 55 — MCP Server Sandbox Layer [SHIPPED]

**Frozen — see [PHASE_55.md](PHASE_55.md).** First post-Chapter-A
phase. Ported the Phase 52 generic command-wrapper sandbox from
`[[tool_process]]` to `[[mcp_server]]`, closing THREAT_MODEL.md
§5.2. New `aivyx-mcp::SandboxConfig` parallel to
`aivyx-tool::SandboxConfig` (no new dep edges between adapter
crates). `StdioTransport::start` gained an `Option<&SandboxConfig>`
parameter; `McpServerBridge::start_with_sandbox` is the new
entry point. `[mcp_server.sandbox]` TOML schema parallel to
`[tool_process.sandbox]`. Loader rejects sandbox declared on
SSE transport (no local child to wrap). 992 Rust tests (+8),
zero clippy. All three streak predictions correct (DESIGN.md
→ 2, PRODUCT.md → 6, lib.rs → 4). Demonstrates the post-
Chapter-A posture: operator-feedback-shaped trigger +
proven-pattern port + small task list.

## Phase 56 — Profile + Persona Amendments (P13 + P14)

**Scheduled** — docs-only phase, same shape as Phase 22 (the
contract-refresh precedent). Files two PRODUCT.md amendments:
**P13 — Assistant Profile** (operator-declared static
identity layer: name, operator_profile, communication_style,
primary_use_cases, preferences, constraints) and **P14 —
Persona** (reflection-written dynamic identity layer:
seed_pointer to Profile, learned_context,
communication_adaptations, character_traits,
relationship_milestones, delta_log). Updates the PRODUCT.md
pitch from "personal autonomous agent platform" to the
operator's vision statement
("self-learning, self-improving AI-personal assistant with
a user-defined Profile and Persona based on the end-user
use-case"). Adds Forward milestones to PRODUCT_ROADMAP.md.
Adds Phase 57–60 entries to ROADMAP.md (this commit
provides the entries themselves). Zero code changes. Expected
streak break: PRODUCT.md streak ends by amendment (same
precedent as Phase 22). DESIGN.md and lib.rs streaks should
hold.

## Phase 57 — Profile Foundation [SHIPPED]

**Frozen — see [PHASE_57.md](PHASE_57.md).** First code
phase of the Profile + Persona arc. Delivered the Profile
substrate per PRODUCT.md P13:

- `aivyx-config::Profile` struct with six P13-commit-5
  fields (`assistant_name`, `operator_profile`,
  `communication_style`, `primary_use_cases`,
  `behavioral_preferences`, `behavioral_constraints`).
- `[profile]` TOML table per Q1(a) (plain-text-
  inspectable, in `aivyx.toml` alongside roles).
- `Profile::default()` synthesizing Q5(b) fallback
  (`assistant_name = DEFAULT_ASSISTANT_NAME`, all else
  empty) — every pre-Phase-57 `aivyx.toml` keeps working
  unchanged.
- `aivyx-channel::assemble_session_prompt` helper composing
  Profile + role envelope into a labeled system prompt per
  Q3(c) (*"## About this assistant"* + *"## Active role:
  <name>"* layout), with passthrough behavior for legacy
  configs where no operator content is declared.
- Wiring through both the parent session-build path
  (`run_async`'s `system_prompt` local) and the role-switch
  child factory (`profile_for_factory` capture), so
  sub-sessions inherit the same Profile section as their
  parent.
- `aivyx init` extended with three opt-in Profile prompts
  per Q4(c) (assistant name, primary use case,
  communication style); `render_toml` emits `[profile]`
  only when the operator customized at least one field.
- Startup-banner `profile` row surfacing assistant_name
  provenance and a count of operator-declared extras.

All six Q-block questions resolved with the recommended
defaults. Tests +14 across aivyx-config and aivyx-channel
(992 → 1006). Zero clippy warnings. All three streak
predictions correct: DESIGN.md → 4, PRODUCT.md → 1, lib.rs
→ 6 (longest run since the Phase 51 deliberate break at 6).
Phase 58 (Inspection) is next.

## Phase 58 — Profile Inspection (closes P13) [SHIPPED]

**Frozen — see [PHASE_58.md](PHASE_58.md).** Operator-facing
inspection and edit surface that closes the Assistant Profile
milestone. After Phase 58, **P1–P13 are all fully shipped**;
only P14 (Persona, Phases 59–60) remains forward.

- `aivyx profile show` (CLI) — reads `aivyx.toml` via the
  existing config loader path, renders the resolved Profile
  in labeled banner-style format per Q3(a). Works whether
  the daemon is running or not.
- `aivyx profile edit` (CLI) — opens the `[profile]`
  section in `$EDITOR` against a tempfile, merges back via
  `toml_edit` surgical update per Q2(a) preserving every
  other section and every comment in `aivyx.toml`. Prints
  a `aivyx daemon stop && aivyx` restart reminder on save
  per Q5(a) load-time-only semantics.
- `CliMode::Profile(ProfileSubcommand)` nested enum per
  Q1(a); five new parser tests cover happy paths and
  error cases.
- Web UI Profile pane via new `Query::GetProfile` IPC
  envelope + `ProfileSummary` wire-shape per Q4(a)
  read-only. Mirrors the show CLI output with an
  injection-status banner.
- PRODUCT.md Delivery Status refreshed (P13 → Fully
  Delivered, Task 5 streak-break).

`toml_edit = "0.22"` is the first new workspace crate
added since Phase 27's `notify`. Tests +17 across the
phase (1006 → 1023), zero clippy warnings. All three
streak predictions correct: DESIGN.md → 5 (untouched),
PRODUCT.md → broke at 2 (Task 5, intentional), lib.rs
→ 7 (untouched). Phase 59 (Persona Foundation) is next.

## Phase 59 — Persona Foundation [SHIPPED]

**Frozen — see [PHASE_59.md](PHASE_59.md).** First code phase
of the Persona half of the Profile + Persona arc. Delivered
the Persona substrate per PRODUCT.md P14 across six Q-block
resolutions:

- **Q1(a) — one field-edit per delta.** Fine-grained
  `PersonaDelta` records with `PersonaDeltaCategory` (10
  variants: 6 Profile-mirror + 4 Persona-specific) and
  `PersonaDeltaOp` (SetScalar / AppendList / RemoveList).
  `(category, op)` pair validated at append time.
- **Q2(a) — parallel HMAC chain.** New `KeyDomain::Persona`
  (10th storage domain). `PersonaChainLog` in-memory
  primitive + `PersistentPersonaLog` storage wrapper.
  Distinct `PERSONA_GENESIS_SEED` from the audit chain so a
  chain-confusion attack (swapping entries between chains)
  is structurally rejected.
- **Q3(c) — Profile-mirror + Persona-specific.** Operator
  can refine Profile fields OR add Persona-specific
  content (LearnedContext, CommunicationAdaptations,
  CharacterTraits, RelationshipMilestones).
- **Q4(a) — extend reflection.propose.** Schema gains a
  `persona_deltas` array; `required_scope` escalates to
  `persona.propose` when the array is non-empty.
  `ProposalRecord` round-trips deltas through the mission
  description.
- **Q5(a) — shared-state hot-reload (deferred to Phase 60).**
  `SharedEffectivePersona = Arc<RwLock<EffectivePersona>>`
  with `apply_delta_to_shared` helper. `reflection.apply`
  writes both the persistent chain and the shared state.
  Per-turn planner-factory re-call (full hot-reload UX)
  defers to Phase 60 alongside the operator-facing
  dashboards. Phase 59 ships snapshot-at-session-build
  freshness — operators who approve mid-session see effect
  on next daemon startup (Profile-shaped semantics).
- **Q6(a) — three labeled sections.**
  `assemble_session_prompt` gains an
  `Option<&EffectivePersona>` parameter; renders
  "## How I have learned to communicate" between Profile
  and Active role when Persona is non-empty.

New `persona.propose` capability scope in
`aivyx-capability::KNOWN_BASES` + `CEILING_TRUSTED`. Three
direct deps added on aivyx-channel (`hmac`, `sha2`,
`serde_jcs` — already transitive). Tests +29 across the
phase (1023 → 1052). Zero clippy warnings. All three
streak predictions correct: DESIGN.md → 6 (untouched),
PRODUCT.md → 1 (untouched), lib.rs → 7 (untouched —
the "at risk" prediction resolved cleanly because the
shared-state plumbing went through aivyx-channel + the
existing planner factory pattern without touching core).
Phase 60 (Persona Visualization) is next.

## Phase 60 — Persona Visualization (closes P14 + ledger) [SHIPPED]

**Frozen — see [PHASE_60.md](PHASE_60.md).** Closes the
Profile + Persona forward arc and the entire PRODUCT.md
forward-commitment ledger. Delivered across five Q-block
resolutions:

- **Q1(c) — nested CLI enum.** New `CliMode::Persona(PersonaSubcommand)`
  with `Show` / `List` / `Revert { target_delta_id }`
  variants. `aivyx persona show` prints the effective
  state; `list` prints the chain; `revert` operator-
  initiated undo.
- **Q2(a) — direct closure capture.** Per-turn planner-
  factory refresh threaded through three sites (parent
  run_session, daemon-run path, role-switch child factory)
  via captured Arc<RwLock<EffectivePersona>> clones reading
  per-turn. Closes the Phase 59 Q5(a) hot-reload deferral.
- **Q3(a) — Web UI click-to-revert.** New Persona tab
  rendering the effective state + delta timeline with
  per-entry Revert buttons. `FrontendMessage::RevertPersonaDelta`
  via the existing WebSocket bridge;
  `DaemonMessage::PersonaRevertResolved` reply.
- **Q4(a) — Revert as a chain op.** New
  `PersonaDeltaOp::Revert { target_delta_id }` variant.
  Folder consults the chain and applies the inverse of
  the target's op; revert-of-revert restores the original
  effect (recursive resolution, termination guaranteed by
  strictly-decreasing index).
- **Q5(a) — operator-only reverts.** Agent cannot propose
  Revert via `reflection.propose`; reverts come from the
  CLI / Web UI surfaces and are auto-approved (no gate
  prompt since the operator is the proposer).

Other surface: `Query::GetEffectivePersona` +
`Query::ListPersonaDeltas` IPC envelopes;
`EffectivePersonaSummary` + `PersonaDeltaSummary` wire
types; `recompute_shared_from_entries` (replaces the Phase
59 single-delta `apply_delta_to_shared` because Revert
needs chain context); 19 new tests (7 revert folder + 4
render + 8 parser); zero clippy warnings. **Tests
1052 → 1071.** All three streak predictions correct:
DESIGN.md → 7 (untouched), PRODUCT.md → broke at 2 (Task 7
Delivery Status refresh, intentional), lib.rs → 8
(untouched).

**Identity export/import** is the one deferred piece — P14
permits it and Phase 60 flagged it optional. Re-binding the
HMAC chain on import is the load-bearing question; lands as
a focused future micro-phase if operator pressure surfaces.

After Phase 60 the project sits at every PRODUCT.md
commitment (P1–P14) fully delivered, identity layer fully
shaped, and the forward-commitment ledger closed. Future
numbered phases land in response to operator feedback or as
amendment-introduced commitments.

## Phase 61 — Distribution: Release Pipeline (Pipeline Ready, Publication Held)

**Frozen — see [PHASE_61.md](PHASE_61.md).** First phase past
the closed forward-commitment ledger. Operator-feedback-shaped
work on the post-Phase-60 substrate-ergonomics axis, opening
the **Distribution Milestone** (phase 1 of N).

Delivered across six engineering tasks + one mid-phase fixup:

- **Task 2 — `aivyx --version` / `-V` CLI flag.** New
  `CliMode::Version` variant in the hand-rolled parser; prints
  `aivyx <CARGO_PKG_VERSION>` and exits 0 without touching the
  config loader, storage layer, or daemon socket. Three parser
  tests (long, short, extra-args rejection).
- **Task 3 — `cargo-dist` initialization.** Q1(a) sign-off:
  cargo-dist v0.31.0 (binary name `dist`) over hand-rolled CI.
  `dist init` generates `dist-workspace.toml` + `.github/
  workflows/release.yml`. Two cleanups on top of the bare
  output: target matrix trimmed to Q2(a) musl-static + macOS
  only (no gnu variants, no `x86_64-pc-windows-msvc`); and
  `aivyx-tool` marked `[package.metadata.dist] dist = false`
  to exclude the `fs_read_subprocess_fixture` test binary
  from release artifacts.
- **Task 4 — CI gates.** Three workflow files for defense in
  depth: `quality-gate.yml` (reusable `workflow_call`,
  `cargo clippy --workspace --all-targets -- -D warnings` +
  `cargo test --workspace`), `ci.yml` (calls quality-gate on
  every push to main and every PR), and a `plan-jobs =
  ["./quality-gate"]` entry in `dist-workspace.toml` that
  wires the same gate into the release pipeline's `plan` job
  — every release-pipeline artifact transitively depends on
  the gate.
- **Task 5 — README rewrite.** Status table refreshed
  (Phase 54 → Phase 61 numbers); Five-minute setup quickstart
  framing prepared for prebuilt binaries.
- **Task 6 — `docs/INSTALL.md`.** New file. Full install
  matrix: supported targets, build-from-source path, where
  files land, first-run checklist, uninstall.
- **Task 5/6 fixup — pipeline-ready framing.** Mid-phase
  the operator pivoted to a "VPS-private-first,
  GitHub-later" posture. Tasks 5 and 6 had landed a
  "v0.1.0 published" framing that was no longer true; the
  fixup reverted README's Five-minute setup to the
  build-from-source primary path and reframed `docs/
  INSTALL.md` with a "Current install state" disclaimer +
  shell-installer-as-forward-looking.

**Task 7 deferred indefinitely.** Phase 61's stated goal at
open was to cut `v0.1.0` on GitHub Releases. Under the
VPS-first pivot, the operator-side publication step (create
public repo, push history, tag `v0.1.0`) is held until
public hosting is configured. The release substrate is
in place and fires the first time a `v*.*.*` tag is pushed
to a public GitHub remote — a focused micro-phase
(numbered later) ships the actual `v0.1.0` when ready.

All three streak predictions correct: DESIGN.md → 8,
PRODUCT.md → 1 (recovered after Phase 60's intentional
break), `aivyx-core/src/lib.rs` → 9 (new record, beating
Phase 60's 8). Test count +3 (1071 → 1074) — Task 2's
three `--version` parser tests. Zero clippy warnings. One
net-new deferral (Task 7 — v0.1.0 publication).

## Phase 62 — Reach: Agent-Initiated Outbound Notifications

**Frozen — see [PHASE_62.md](PHASE_62.md).** Second phase past
the closed forward-commitment ledger and the first phase of the
new **Reach Milestone**. Gives the agent a `notify.send`
infrastructure tool that pushes a message to an
operator-configured `[[notify_target]]` (Telegram chat or
generic webhook URL). Transforms Aivyx from purely reactive
("I talk to it") to proactive ("it can wake my phone").

Delivered across seven engineering tasks:

- **Task 2 — `notify.send` capability scope** in
  `aivyx-capability`: new base in KNOWN_BASES with
  `<target_name>` qualifier (matches `role.switch:<name>`
  precedent), included in CEILING_TRUSTED only. Wildcard form
  `notify.send:*` rejected at parse time alongside
  `role.switch:*` — same Rule 2 rationale.
- **Task 3 — `[[notify_target]]` TOML config surface** in
  `aivyx-config`: `NotifyTargetConfig` + `NotifyTargetKind`
  enum making invalid kind/field combinations unrepresentable
  at the runtime layer; load-time validations for unique
  names, non-empty fields, kind-required fields, and
  http(s)-scheme URLs.
- **Task 4 — `NotifyDispatcher` + `NotifyBackend` trait** in
  `aivyx-channel`: async trait with `send(message, subject)
  -> Result<(), NotifyError>` and a `kind()` discriminator;
  five-variant `NotifyError` (Transport / Auth / Rejected /
  Timeout / UnknownTarget) classified for agent retry
  guidance; name-keyed registry with `register` /
  `dispatch` / `list_targets`.
- **Task 5 — Telegram outbound backend** wraps the existing
  `aivyx-telegram` `ReqwestTransport`. Per-target backend
  shares the transport across multiple Telegram targets;
  `chat_id` parses from config string to `i64` at
  construction; negative group chat_ids supported; heuristic
  status-code classification mapping `TransportError::Platform`
  strings to the right `NotifyError` variant.
- **Task 6 — Generic webhook backend** posts JSON
  `{source, target, subject?, message, timestamp}` per
  Q5(a). Trait abstraction (`WebhookSender`) so tests don't
  need a real HTTP server; production `ReqwestWebhookSender`
  uses a 5s timeout. `map_http_status` pure-function tested
  across all status ranges.
- **Task 7 — `NotifySendTool`** with OnceLock factory pattern
  matching `MissionCreateTool` / `RoleSwitchTool`. Input
  schema `{target, message, subject?}` per Q3(b). Output
  per Q4(a): `Completed` with `success: bool` so the agent
  reads structured retry data without `ToolOutcome::Failed`
  escalating up the turn loop. `required_scope` builds
  `notify.send:<target>` from input.
- **Task 8 — daemon wiring + worked example.**
  `build_notify_dispatcher` factory walks the config and
  constructs the right backend per target. Binary's
  session-build path builds a dedicated `ReqwestTransport`
  when Telegram targets exist, constructs the dispatcher,
  registers `NotifySendTool` with it. `examples/aivyx.toml`
  gains a documented `[[notify_target]]` section (commented
  out by default).

Streak predictions all correct: DESIGN.md → 9 (extended);
PRODUCT.md → 2 (extended); `aivyx-core/src/lib.rs` → 10 (new
record, beating Phase 61's 9). Tests +65 (1074 → 1139) —
underestimated at open (predicted ~+20); the six concurrent
new surfaces each carried ~10 tests. Zero clippy warnings.
Zero new workspace deps.

**Scope adjustment at exit:** the `assemble_session_prompt`
extension originally planned for Task 8 (the agent learns
target names from the system prompt) deferred to a follow-on
phase. The tool is fully functional without it — the agent
learns from the input schema; the operator's
`[profile] communication_style` or role `system_prompt` can
name specific targets when needed.

## Phase 63 — Reach Phase 2: Trigger-Config Notify Sugar

**Frozen — see [PHASE_63.md](PHASE_63.md).** Second phase of
the Reach Milestone. Closes the Phase 62-deferred trigger-
config sugar alternative: each `[[schedule]]`, `[[webhook]]`,
`[[file_watch]]` entry accepts an optional
`notify_target = "..."` field; when the trigger fires and the
turn completes, the daemon auto-dispatches the agent's final
response to the named target. No agent involvement, no
system-prompt instruction. The "scheduled summary lands on my
phone" use case now Just Works.

Delivered across six engineering tasks (Tasks 1, 2, 3+4+5
combined, 6, 7):

- **Task 2 — Config field + cross-validation.** All three
  Raw* trigger types gain `notify_target: Option<String>`;
  mirror to public ScheduleConfig / WebhookConfig /
  FileWatchConfig. New `validate_trigger_notify_targets`
  helper called after notify_targets is built but before
  AivyxConfig assembly. Two failure modes: unknown-target,
  and missing-capability (walks the trigger's role parent
  chain, accumulates declared scopes into a CapabilitySet,
  intersects with the trust ceiling, checks
  `notify.send:<target>` is granted). Nine new tests
  including the SemiTrusted-ceiling-drops-notify regression
  and inheritance-via-parent-role.
- **Task 3 — TriggerDispatch auto-notify hook.** New
  `with_notify_dispatcher` builder; `fire()` signature
  extends with `notify_target: Option<&str>`. After the turn
  completes, if both are Some, dispatch the agent's response
  with subject `<kind>: <trigger-id>`. `render_notify_body`
  pure function maps each TurnOutcome variant to a body
  string (Completed → final_message; Failed → "Turn failed:
  <reason>"; Escalated → "Turn escalated: <reason>";
  TimedOut + Cancelled → marker strings). Empty Completed
  body skips per Q2(a). Six render-body unit tests.
- **Task 3 plumbing.** All three Record types
  (ScheduleRecord, WebhookRecord, FileWatchRecord) gain
  `notify_target` with `#[serde(default)]` for backward
  compat with pre-Phase-63 encrypted records. The three
  fire() call sites + WatchState + config-to-record paths
  all propagate the field. DaemonConfig gains
  `notify_dispatcher: Option<Arc<NotifyDispatcher>>`; the
  binary shares one Arc between `NotifySendTool` (Phase 62)
  and `DaemonConfig` for the trigger path.
- **Task 6 — Worked example.** New section in
  `examples/aivyx.toml` documents the schedule + notify_target
  pattern with operator rationale.

Streak predictions all correct: DESIGN.md → 10, PRODUCT.md → 3,
`aivyx-core/src/lib.rs` → 11 (new record, beating Phase 62's
10). Tests +15 (1139 → 1154), inside the predicted +15–20
range. Zero clippy warnings. Zero new workspace deps.

**Scope adjustment at exit:** Q1(a) sign-off chose a new
`AuditEventKind::AutoNotifyDispatched` variant for forensic
search. Implementation revealed `TriggerDispatch` doesn't hold
an audit-hook reference today; wiring one in touches multiple
plumbing layers and is materially larger than the notify hook
itself. Deferred to a focused follow-on phase. Auto-notify is
eprintln-logged matching existing trigger.rs patterns (mission
state changes aren't audit-logged either). Per-kind
dispatcher-recording integration tests similarly folded into
the `render_notify_body` unit coverage.

## Phase 64 — Identity Export (Persona Phase 3)

**Frozen — see [PHASE_64.md](PHASE_64.md).** Closes the oldest
open deferral: Phase 60's "identity export/import" item. The
phase ships **export only** per the implementation-time scope
adjustment; import lands in Phase 65 with focused destructive-
write design attention.

Delivered across five engineering tasks (Tasks 1, 2+4, 3+6, 7,
8):

- **Task 2+4 — Format substrate.** New module
  `crates/aivyx-channel/src/identity_export.rs` with:
    * `IdentityExport` top-level bundle (schema_version,
      exported_at, source_host, profile, persona).
    * `ProfileExport` — plain-values projection of
      `aivyx_config::Profile` dropping `Sourced<...>` metadata.
    * `PersonaExport` — `Vec<DeltaExport>` (MAC-stripped) +
      `effective_at_export` snapshot.
    * `DeltaExport { seq, delta }` — preserves seq for
      parse-time gap detection.
    * `build()` pure assembler + `parse_and_validate()` with
      five failure modes (Json, SchemaVersion, NonMonotonicSeq,
      InvalidDelta, EffectiveMismatch).
  Drive-by: `EffectivePersona` gains `Serialize` + `Deserialize`
  derives. 9 new tests.

- **Task 3+6 — CLI + IPC wiring.** New IPC envelope
  `QueryPayload::ExportPersonaChain` with single-shot response
  capped at 100,000 entries. Daemon handler reads
  `persona_log.entries()` + the shared persona state. Client
  wrapper `export_persona_chain` mirrors the
  `list_persona_deltas` pattern. New CLI module
  `aivyx_modules/identity.rs` with `run_identity_export(path)`
  — fetches Persona via IPC, loads Profile via aivyx-config
  directly, writes 0600 JSON. New `CliMode::Identity(...)` +
  `IdentitySubcommand::Export` variant. `aivyx identity import`
  recognized but routes to a descriptive Phase-65 deferral
  error. 6 parser tests + 2 module tests.

- **Task 7 — Operator docs.** New "Moving Aivyx to a new
  machine" section in `docs/INSTALL.md` documents the export
  flow, the JSON shape, the HMAC re-bind design (Q1(a)), and
  the Phase 65 deferral.

Streak predictions all correct: DESIGN.md → 11, PRODUCT.md → 4,
`aivyx-core/src/lib.rs` → 12 (new record, longest production-
core run in project history; beats Phase 63's 11). Tests +17
(1154 → 1171). Zero clippy warnings. Zero new workspace deps.

**Scope adjustment at exit:** Task 5 (`aivyx identity import
<path>`) deferred to Phase 65 per the implementation-time
sign-off on Option B (future-proofing argument). The import
side carries substantial substrate of its own — new IPC
envelopes for destructive writes, conflict resolution, atomic
replay, `--force` flag, integration tests for each failure
mode. Splitting the work yields cleaner phases and focused
design attention for the locked-in IPC semantics. Phase 65
opens next with the dedicated scope.

## Phase 65 — Identity Import (Persona Phase 4)

**Frozen — see [PHASE_65.md](PHASE_65.md).** Closes the Phase
60 identity-deferral end to end. `aivyx identity import
<path>` replays an exported bundle onto the local persona
chain, re-signing each delta against the target host's HMAC
key. Together with Phase 64's export, the operator now has a
full multi-host identity transfer path.

Delivered across three engineering commits (Open, Tasks 2–6
combined, Tasks 7+8 combined, Exit):

- **IPC envelope pair.** `FrontendMessage::ImportPersonaChain
  { id, deltas, effective_at_export, force }` →
  `DaemonMessage::PersonaImportResolved { id, ok, success,
  error }` with `PersonaImportSuccess { deltas_imported,
  final_chain_seq }` per Q4(a). `DaemonEnvelope` gains the
  response variant for client-side decode.
- **Daemon handler.** `resolve_persona_import` runs the full
  flow: server-side re-validation, conflict check (Q3(a)),
  optional force-wipe, per-delta replay through the existing
  `append` path (re-signs against local key per Phase 60
  Q1(a)), runtime refresh via `recompute_shared_from_entries`.
  Best-effort atomicity per Q1(a): daemon crash mid-import
  leaves chain partial; operator re-imports.
- **`PersistentPersonaLog::clear`** — new method wipes both
  persisted rows and the in-memory chain. HMAC key preserved
  so subsequent appends produce a fresh chain at seq 0.
- **Client wrapper + CLI handler.** `import_persona_chain`
  mirrors `revert_persona_delta`'s shape. CLI handler reads
  file, locally validates via Phase 64's
  `parse_and_validate`, forwards to daemon, prints
  per-Q4(a) summary + Q2(a) Profile-hand-edit reminder.
- **Parser update.** `aivyx identity import <path>
  [--force]` replaces the Phase 64 deferral message;
  trailing `--force` accepted, double-force and unknown
  args rejected.
- **Docs.** `docs/INSTALL.md` "Moving Aivyx to a new
  machine" updated with the import command, the conflict-
  resolution explanation, and the Q2(a) Profile note.

Streak predictions all correct: DESIGN.md → 12, PRODUCT.md →
5, `aivyx-core/src/lib.rs` → 13 (new record — longest
production-core run in project history; beats Phase 64's 12).
Tests +5 (1171 → 1176), under the predicted +10–15 range
because Phase 65 leaned heavily on the Phase 64 substrate
(parse_and_validate from Phase 64, append from Phase 59,
recompute helper from Phase 60). Zero clippy warnings. Zero
new workspace deps.

**Identity export + import are now both shipped.** The Phase
60 deferral closes entirely. Future Persona work is operator-
feedback-shaped (merge-strategy imports, encrypted export
format, multi-source merge, schema migrations, real atomic
import).

## Phase 66 — Starter Profile Templates

**Frozen — see [PHASE_66.md](PHASE_66.md).** Closes the third
post-Phase-60 codebase-review direction (after Distribution
and Reach). Ships `aivyx init --template <name>` so a fresh
operator gets from "downloaded the binary" to "useful agent"
without writing aivyx.toml from scratch. Three starter
templates cover the common archetypes: `coder` (software
engineering), `researcher` (research + synthesis), `personal`
(personal task management + briefings).

Delivered across three engineering commits (Open, Tasks 2–8
combined, Tasks 9+10):

- **Template registry substrate.** New
  `init_templates.rs` module with `Template { name,
  description, source, toml_content }`, `bundled_templates()`
  via `include_str!`, `user_templates()` reading from
  `$XDG_DATA_HOME/aivyx/templates/` or
  `~/.local/share/aivyx/templates/`, `list_templates()`
  unioning both with user-dir winning on collision,
  `load_template(name)` with descriptive miss error, and
  `parse_description` reading the `# description: …` marker
  from leading TOML comments.
- **CLI flags.** `CliMode::Init` becomes `Init(InitMode)`
  with `Interactive`, `InteractiveFromTemplate { name }`,
  `ListTemplates` variants. Parser accepts `aivyx init`,
  `aivyx init --template <name>`, `aivyx init --template`
  (no name → list), `aivyx init --list-templates`.
- **Wizard pre-fill.** New `TemplateDefaults` extracts
  provider, model, fs_root, storage_path, assistant_name,
  primary_use_case, communication_style from a template's
  TOML via `toml_edit`. Each wizard prompt's default
  switches to the template-sourced value when present.
  `render_with_template` splices wizard answers into the
  template's `DocumentMut` and serializes — comments, role
  declarations, MCP server blocks, commented-out sections
  all survive.
- **Three starter templates** in `examples/templates/`. Each
  is a complete `aivyx.toml` with archetype-appropriate
  Profile content, role envelope, and MCP/notify defaults
  (commented-out where appropriate).
- **20 unit tests** (14 registry + 6 parser) including
  per-template TOML validity checks.
- **`docs/TEMPLATES.md`** as the operator-facing reference.
  `INSTALL.md` First-run checklist + README quickstart
  updated to point at the fast-path.

Streak predictions all correct: DESIGN.md → 13, PRODUCT.md →
6, `aivyx-core/src/lib.rs` → 14 (new record, longest
production-core run in project history — beats Phase 65's
13). Tests +18 (1176 → 1194), slightly under the predicted
+20–30 range. Zero clippy warnings. Zero new workspace deps.

**Q-block sign-off ambition held cleanly.** The operator
chose the more ambitious options across all four questions
at design time (hybrid location, three templates, pre-fill
wizard, both discovery modes). All four sign-offs delivered
as designed; no scope adjustments at implementation time.
Phase 66 substantial-scope-but-clean shape.

## Phase 67 — Auto-Notify Audit Event

**Frozen — see [PHASE_67.md](PHASE_67.md).** Closes the Phase
63 deferred Q1(a) sign-off: shipping
`AuditEvent::AutoNotifyDispatched` as a new variant of the
audit chain enum + wiring `TriggerDispatch` with the audit
hook needed to emit it.

After Phase 67, every trigger-fired auto-notify (delivered,
skipped-empty-response, or failed) lands as an entry in the
persistent audit chain alongside `TurnStarted` / `TurnEnded`.
Operators can answer "why didn't my morning briefing arrive?"
from the audit chain alone — eprintln moves from primary
evidence to debugging supplement.

Delivered across four engineering commits (Open, Tasks 2–4
combined, Tasks 5+6, Exit):

- **Audit variant in `aivyx-audit`.** New
  `AuditEvent::AutoNotifyDispatched { session_id,
  trigger_kind, trigger_id, target_name, outcome,
  dispatched_at_unix_ms }` + supporting
  `TriggerKindSummary` (Cron / Webhook / FileWatch) +
  `AutoNotifyOutcomeSummary` (Delivered /
  SkippedEmptyResponse / Failed { error_kind,
  error_message }) enums. All `#[serde(tag = "kind")]` so
  existing chain readers (Web UI Audit tab, `aivyx
  --verify-only`) parse the new variant without per-reader
  changes.
- **Plumbing.** `TriggerDispatch` gains an
  `audit_log: Option<Arc<PersistentAuditLog>>` field +
  `with_audit_log` builder. `run_daemon` wires
  `DaemonConfig::audit_log` (Phase 47 field) through to the
  dispatch instance.
- **Emission.** New `emit_auto_notify_audit` method on
  `TriggerDispatch`; the three auto-notify branches
  (skip-empty / dispatch-Ok / dispatch-Err) all converge on
  it. Append failures are eprintln-logged per Q3(a) — the
  notify already happened or didn't; audit failure shouldn't
  conflate the outcome.
- **Conversion helpers.** `From<TriggerSource>` for
  `TriggerKindSummary`; `outcome_from_notify_error` maps
  the five `NotifyError` variants to the
  `AutoNotifyOutcomeSummary::Failed` shape using the same
  `error_kind` labels (`transport` / `auth` / `rejected` /
  `timeout` / `unknown_target`) the `notify.send` tool
  emits — forensic searches grep across both
  agent-initiated and daemon-initiated notify failures
  uniformly.
- **Docs.** New "Debugging missing notifications" section
  in `docs/INSTALL.md` walks operators through the audit
  chain + the three outcome shapes + `session_id`
  correlation back to `TurnStarted` / `TurnEnded`.

Streak predictions all correct: DESIGN.md → 14, PRODUCT.md →
7, `aivyx-core/src/lib.rs` → 15 (new record, longest
production-core run in project history — beats Phase 66's 14).
Tests +9 (1194 → 1203), slight undershoot of the predicted
+10–15 (3 audit + 6 conversion-helper unit tests; full
end-to-end fire integration deferred to dogfooding). Zero
clippy warnings. Zero new workspace deps.

**Implementation-time scope adjustment held cleanly.** Q1
sign-off chose rich event with turn_id; investigation
revealed `TurnOutcome` doesn't carry `turn_id` (lifting it
would touch 120 match sites). Solved at design time with
`session_id` correlation — already minted per trigger fire
and recorded on `TurnStarted` audit entries. Same forensic
payoff; no upstream refactor. `turn_id` correlation recorded
as a dedicated future-phase deferral.

## Phase 68 — Email SMTP Notify Backend (Reach Phase 3)

**Frozen — see [PHASE_68.md](PHASE_68.md).** Third notify
backend after Phase 62's Telegram + webhook. Closes the
largest remaining adoption-shape gap on the Reach axis:
every operator has email; most don't run Telegram bots.
After Phase 68 the supported notify kinds are `telegram`,
`webhook`, and `email`.

Delivered across three engineering commits (Open, Tasks 2–5
combined, Exit):

- **Config in `aivyx-config`.** `EmailConfig` + `TlsMode`
  enum + `NotifyTargetKind::Email { to }` variant.
  `[email]` section parses via a `RawEmail` shape;
  `build_email_config` validates required-when-present
  semantics + the security rules (tls_mode = "none"
  rejected per Q4; default port from tls_mode; address
  `@` checks; email target without `[email]` section is a
  load-time error naming the offending target).
- **`notify_email.rs` in `aivyx-channel`.** `EmailSender`
  trait + `LettreEmailSender` production impl (built once
  per deployment, shared via `Arc`) + `NotifyEmailBackend`
  wrapping the shared sender with per-target from/to.
  `map_lettre_error` classifies lettre errors into the
  five-variant `NotifyError` taxonomy `notify.send`
  already uses. Auth mechanisms declared explicitly as
  PLAIN + LOGIN.
- **Dispatcher + binary wiring.** `build_notify_dispatcher`
  gains an `EmailDispatchContext` parameter; binary builds
  the `LettreEmailSender` once if any email target exists.
- **Docs + worked example.** `examples/aivyx.toml` gains
  commented `[email]` + email `[[notify_target]]` blocks
  with provider-specific setup (Gmail / Fastmail /
  ProtonMail Bridge / SES / self-hosted). `docs/INSTALL.md`
  "Email notifications" section walks operators through
  quick-setup, the TLS-mandatory rule, and the
  no-OAuth2-yet caveat.

Streak predictions all correct: DESIGN.md → 15, PRODUCT.md →
8, `aivyx-core/src/lib.rs` → 16 (new record, longest
production-core run in project history — beats Phase 67's
15). Tests +19 (1203 → 1222). Zero clippy warnings.

**New workspace dep:** `lettre` with `default-features =
false` + explicit rustls features. First net-new workspace
dep since Phase 58's `toml_edit` (six phases ago). Verified
zero openssl pulls; TLS stays uniformly rustls across
reqwest + lettre.

After Phase 68 the Reach Milestone covers the three most
common operator channels (Telegram for chat-style, webhook
for tooling-style, email for everyone else). Remaining
Reach-axis deferrals (Web UI desktop notify, OS-level
notify, default-target sugar, retry semantics, multi-target
dispatch, conditional notify) are operator-feedback-shaped
follow-ups; the substrate is extensible.

## Phase 69 — Web UI Desktop Notifications (Reach Phase 4)

**Frozen — see [PHASE_69.md](PHASE_69.md).** Fourth notify
backend after Phase 62's Telegram + webhook and Phase 68's
email. Closes the focused-at-the-laptop case: operators who
already keep the Web UI tab open at `127.0.0.1:7843` get
OS-level desktop notifications + an in-page toast banner with
no API keys, no SMTP setup, no bot tokens.

Delivered across nine engineering commits (Open, Tasks 2–10
each as their own commit, Exit):

- **Config in `aivyx-config`.** `NotifyTargetKind::WebUi`
  unit variant (no per-target fields per Q3(a)). Loader
  accepts `kind = "web-ui"`; unknown-kind error message
  lists `web-ui` in the supported-kinds suggestion.
- **IPC envelope.** `DaemonMessage::DesktopNotification {
  title, body }` + matching `DaemonEnvelope` variant per
  Q4(a). Distinct from `StreamEvent` (per-session) because
  broadcast events deserve their own variant.
- **`notify_webui.rs` in `aivyx-channel`.** `WebUiBroadcaster`
  wraps a `tokio::sync::broadcast::Sender<DesktopNotificationFrame>`;
  `NotifyWebUiBackend` implements `NotifyBackend::send` by
  pushing onto the broadcaster. Per Q1(a), zero subscribers
  yields `Ok(())` — fire-and-forget broadcast model. Audit
  chain (Phase 67) still records every dispatch.
- **WS handler subscription.** Each browser WS connection
  subscribes a fresh broadcast receiver; a third concurrent
  loop relays each frame onto the WS as
  `DaemonEnvelope::DesktopNotification` JSON.
- **Dispatcher + binary wiring.** `build_notify_dispatcher`
  gains the broadcaster as a fourth parameter; binary
  constructs one `Arc<WebUiBroadcaster>` at startup whenever
  the Web UI is enabled and Arc-shares it between the
  dispatcher (push side) and `DaemonConfig` (subscribe side).
- **Web UI JS.** Handler for `{type: "DesktopNotification"}`
  triggers `new Notification(title, {body})` (browser API)
  AND renders a stackable in-page toast banner per Q2 (both
  UX modes). One-time "Enable notifications" prompt on page
  load when `Notification.permission === "default"`.
- **Docs + worked example.** `examples/aivyx.toml` gains a
  commented `kind = "web-ui"` block; `docs/INSTALL.md`
  "Web UI desktop notifications" subsection covers the
  two-step enable, the browser-tab-must-be-open caveat, and
  the pair-with-email-for-persistence guidance.

Streak predictions all correct: DESIGN.md → 16, PRODUCT.md →
9, `aivyx-core/src/lib.rs` → 17 (new record, longest
production-core run in project history — beats Phase 68's
16). Tests +12 (1222 → 1234). Zero clippy warnings.
Zero new workspace deps (`tokio::sync::broadcast` was
already available via the existing tokio dep).

After Phase 69 the Reach Milestone covers all four
operator-shape categories: chat-style (Telegram), tooling-
style (webhook), inbox-style (email), focused-at-the-laptop
(Web UI desktop). Remaining Reach-axis deferrals are smaller
operator-feedback shapes: WebPush / service-worker
notifications for closed-tab delivery (real engineering —
needs VAPID + service-worker registration), default-target
sugar, per-target rate limits, retry semantics, multi-target
dispatch, conditional notify, notification urgency / sound /
icons.

## Phase 70 — Reflection Auto-Loop (P14 Self-Learning Closure)

**Frozen — see [PHASE_70.md](PHASE_70.md).** Closes the
long-held self-learning half of **P14 Persona**. Phase 29
(frozen) shipped the reflection substrate — `ReflectionProposeTool`
+ `ReflectionApplyTool` — but the agent's persona-delta
proposals went through a synchronous mission-gate flow that
required the operator to be present at the agent's terminal
at the moment of proposal. Phase 70 adds the **asynchronous
review surface**: agent calls `reflection.propose` →
proposals land as Pending rows in a new encrypted proposal
chain → operator reviews on their own schedule via the Web UI
Proposals pane or `aivyx persona proposals` CLI →
approval/rejection appends to the persona chain (or audit
trail) accordingly.

Delivered across ten engineering commits (Open, Tasks 2–10,
Exit):

- **Config in `aivyx-config`.** `[[reflection_schedule]]`
  section parses + validates (cron non-empty, lookback bounds
  60s–30 days, name uniqueness across both schedule and
  reflection-schedule namespaces, role_override existence).
- **Storage in `aivyx-storage`.** `KeyDomain::PersonaProposals`
  variant with HKDF info bytes `persona-proposals`, table
  name `aivyx_persona_proposals_v1`, integrated into the
  fixed-size subkey array and the all-variants tripwire test.
- **`persona_proposal.rs` in `aivyx-channel`.** HMAC-chained
  proposal log mirroring `PersistentPersonaLog`. Status as
  state machine derived from chain entries (Pending → Approved
  / Rejected / Superseded); each transition appends a new
  signed row rather than mutating, preserving the audit story.
  Distinct genesis seed (`aivyx-proposal-genesis-v1`) from
  the persona chain so chain-confusion attacks are
  structurally rejected at MAC verification (Q4(a)).
- **IPC envelopes.** Two new `QueryPayload` variants
  (`ListPersonaProposals`, `GetPersonaProposal`), the wire-
  format `PersonaProposalSummary`, `FrontendMessage::ResolvePersonaProposal`
  with the three-variant `PersonaProposalResolution` tagged
  enum (`Approve | ApproveWithEdit { edited_op } | Reject {
  reason }` per Q3(a)), and the matching
  `DaemonMessage::PersonaProposalResolved` response with
  `PersonaProposalResolveSuccess { proposal_status,
  applied_seq }`.
- **Daemon-side resolution.** `resolve_persona_proposal` in
  `daemon_server.rs`: Approve / ApproveWithEdit validate the
  applied op, append to the persona chain, then record an
  Approved row bound to the resulting seq; Reject just
  records Rejected. Shared persona state is recomputed on
  approve. `handle_query` gains arms for both proposal-list
  queries with status-filter parsing.
- **Web UI Proposals pane.** New tab with filter chips
  (`pending | approved | rejected | all`), per-proposal
  cards showing category / id / source-reflection-session /
  status badge / proposed op / agent reason, three actions
  per Pending card (Approve verbatim / inline JSON editor +
  Save & Approve for edit-on-approve / Reject with optional
  reason). Approved cards with operator-edited applied_op
  render both ops for audit visibility.
- **CLI subcommands.** `aivyx persona proposals list
  [--status STATUS]` (default `pending`), `show <id>`,
  `approve <id>`, `reject <id> [--reason TEXT]`. Pretty-
  prints proposals with all status-specific fields.
- **Agent-write integration.** `ReflectionProposeTool` gains
  a proposal-log setter; when set (binary's startup path
  wires it in), every agent-supplied `ProposedPersonaDelta`
  is appended to the proposal chain as a Pending row in
  addition to the existing mission-gate flow.

Streak predictions all correct: DESIGN.md → 17, PRODUCT.md →
10, `aivyx-core/src/lib.rs` → 18 (new project record, beats
Phase 69's 17). Tests +41 (1234 → 1275), exceeding the
+25-35 prediction. Zero clippy warnings. Zero new workspace
deps.

**Scope note — cron auto-firing deferred.** The
`[[reflection_schedule]]` config section parses + validates
end-to-end, and the proposal substrate accepts agent-
generated deltas via `reflection.propose` today; the
dedicated scheduler-loop that fires reflection turns on the
configured cron is a deferred follow-up. Operators who want
auto-reflection today wire a regular `[[schedule]]` entry
with a reflection-flavored prompt; the proposals land in the
same chain and surface in the same Web UI / CLI panes either
way.

## Phase 71 — Reflection Scheduler Loop (closes Phase 70 deferral)

**Frozen — see [PHASE_71.md](PHASE_71.md).** Closes the
cron-auto-firing deferral carried at Phase 70 exit. After
Phase 71 the self-learning loop is genuinely autonomous: at
each configured cron boundary the daemon synthesizes recent
turn outcomes from the audit chain, fires a reflection turn
carrying the canonical reflection prompt + outcome summary
block, and any persona deltas the agent proposes land in the
Phase 70 proposal chain for asynchronous operator review.

Delivered across four engineering commits (Open, Tasks 2-4,
Task 5, Tasks 7+Exit):

- **New `reflection_scheduler.rs` module in `aivyx-channel`.**
  `run_reflection_scheduler` async loop with adaptive
  sleep-until-earliest-fire (cap 60s); per-schedule in-memory
  last-fired-at; cron parsing reuses the existing `cron` crate
  from Phase 26. `fire_reflection` summarizes recent outcomes,
  formats the prompt block, calls `TriggerDispatch::fire(...)`.
- **Canonical reflection prompt.** Hardcoded
  `REFLECTION_SYSTEM_PROMPT` constant (Q2(a)) with
  conservative behavioral framing: propose only on ≥3-turn
  pattern recurrence, prefer narrower categories, every
  proposal lands Pending until operator approves. A smoke
  test guards the constraint set from accidental gutting.
- **Audit walker + LRU cache.** `OutcomeSummary` struct
  + `summarize_recent_outcomes_from_entries` pairs
  TurnStarted/TurnEnded by turn_id, filters by lookback
  window, sorts most-recent-first; in-flight + un-paired
  entries skipped. `OutcomeSummaryCache` is a bounded
  VecDeque-backed LRU keyed by
  `(lookback_secs, audit_chain_len)` per Q1(c).
- **TriggerSource + TriggerKindSummary variants.** New
  `Reflection` variant in both runtime + audit enums so
  forensic searches distinguish self-learning reflection
  turns from operator-declared crons.
- **Binary wire-up.** `DaemonConfig` gains
  `reflection_schedules: Vec<ReflectionScheduleConfig>`. When
  non-empty AND an audit log is available, the daemon spawns
  the scheduler task alongside the existing scheduler /
  webhook listener / file watcher. Startup banner prints one
  line per registered schedule. Graceful degradation: when
  schedules exist but no audit log is available, a clear
  diagnostic prints and the scheduler is not spawned.
- **Docs.** `examples/aivyx.toml` and `docs/INSTALL.md` flip
  from the Phase 70 "deferred-polish" caveat to a concrete
  setup walkthrough.

Streak predictions all correct: DESIGN.md → 18, PRODUCT.md →
11, `aivyx-core/src/lib.rs` → 19 (new project record, beats
Phase 70's 18). Tests +14 (1275 → 1289), one below the
+15-25 prediction floor — the binary wire-up didn't add
net-new tests because the spawn pattern was already
exercised by the existing scheduler e2e. Zero clippy
warnings. Zero new workspace deps.

**Scope note — Q3(a) role_override is recorded but not
runtime-honored.** The schedule config validates that
`role_override` references an existing role and the scheduler
logs the override for forensic attribution, but the per-fire
runtime role swap is a deferred polish: v1 runs the
reflection turn under the daemon's active role. Operators
who want a dedicated reflection envelope today declare a
`[[role]]` and run the daemon under it via the existing
role-switching path.

After Phase 71 the self-learning half of P14 is genuinely
autonomous end-to-end.

## Phase 72 — Reach Polish: Multi-Target, Default, Conditional

**Frozen — see [PHASE_72.md](PHASE_72.md).** Closes three
Tier-1 operator-feedback shapes carried from the Reach
Milestone (Phases 62-69) in one phase:

- **Multi-target dispatch.** A single trigger fans out to N
  notify targets in one fire. `notify_targets: Vec<String>`
  on trigger configs; the singular `notify_target` stays as
  a backwards-compat alias. Dispatch uses
  `futures_util::future::join_all` for concurrent fan-out
  (Q4(a)); each per-target backend outcome is audited
  independently as a separate `AutoNotifyDispatched` entry,
  so one target's transport failure doesn't block the
  others.
- **Default-target sugar.** `default = true` on one
  `[[notify_target]]` block marks it as the global default;
  triggers that omit `notify_targets` fall through to it at
  config-load time so runtime dispatch never has to resolve
  defaults again. Loader rejects multiple defaults with a
  clear "phone, desktop" multi-name error.
- **Conditional notify.** New `NotifyWhen` enum
  (`Always | OnFailed | OnCompletedNonEmpty`) gates dispatch
  by turn outcome. A gate-skipped dispatch records the new
  `AutoNotifyOutcomeSummary::SkippedByCondition { condition }`
  audit variant so forensic searches can answer "why didn't
  this fire?" definitively.

Schedule / Webhook / FileWatch storage records gain
`notify_targets` + `notify_when` fields (serde-defaulted for
backwards compatibility); the config-to-record bridges copy
both through. All four `dispatch.fire(...)` callers updated
to the new signature.

Streak predictions all correct: DESIGN.md → 19, PRODUCT.md →
12, `aivyx-core/src/lib.rs` → **20** (new project record +
**two-decade milestone**, beating Phase 71's 19). Tests +13
(1289 → 1302), below the +20-30 prediction floor — honest
miss called out in the phase doc: fan-out integration tests
require heavier scaffolding (mocked dispatcher + audit log +
spawned futures) than fit cleanly in the phase. Zero clippy
warnings. Zero new workspace deps.

Tier-2 polish (per-target retry semantics, per-target rate
limits, Web UI notification history pane) defers to a focused
follow-up — different cluster of concerns.

## Phase 73 — Reach Tier-2 Polish: Retry, Rate Limit, History

**Frozen — see [PHASE_73.md](PHASE_73.md).** Closes the
Tier-2 polish backlog Phase 72 explicitly deferred. Three
items shipped in one phase:

- **Per-target retry.** Flat fields `retry_count` +
  `retry_backoff_ms_start` on `NotifyTargetConfig` per
  Q1(b). On transient failures (`Transport`, `Timeout`, or
  `Rejected` with HTTP status ≥ 500 per Q2(b)), the
  dispatcher retries up to `retry_count` times with
  exponential backoff. `Auth`, `UnknownTarget`, and
  `Rejected` < 500 never retry. Capped at 10 retries by the
  loader (footgun guard).
- **Per-target rate limit.** In-memory sliding-window token
  bucket per target via the new `RateLimitRegistry` per
  Q3(a). Both `rate_limit_max` + `rate_limit_window_secs`
  must be set together. Exhausted bucket records the new
  `AutoNotifyOutcomeSummary::SkippedByRateLimit { limit,
  window_secs }` audit variant. Daemon-lifetime state;
  restart resets the bucket.
- **Notification history pane.** New
  `QueryPayload::ListNotificationHistory { from_seq, limit,
  target_filter }` walks the audit chain for
  `AutoNotifyDispatched` events with server-side
  pagination (cap 500, matches the audit pane). Web UI
  Notifications tab renders a 4-column grid with
  colour-coded outcome badges; auto-populated per-target
  chips filter the view. CLI parity: `aivyx notify history
  [--target NAME] [--limit N]`.

`NotificationHistoryEntry` is the flat wire shape; the
daemon's `render_notify_outcome_for_history` helper renders
each `AutoNotifyOutcomeSummary` variant into stable lowercase
`outcome_kind` + variant-specific `outcome_detail` strings.

Streak predictions all correct: DESIGN.md → 20, PRODUCT.md →
13, `aivyx-core/src/lib.rs` → **21** (new project record,
beating Phase 72's 20). Tests +31 (1302 → 1333), comfortably
inside the +25-35 prediction. Zero clippy warnings. Zero new
workspace deps.

After Phase 73 the Reach Milestone polish backlog is closed
end-to-end. Remaining notify-shaped deferrals (WebPush,
Slack-flavored webhooks, XOAUTH2, persisted rate-limit
buckets) are operator-feedback-gated and ship if/when real
pressure surfaces.

## Phase 74 — Memory Polish: Search, Retention, LRU, Web UI Pane

**Frozen — see [PHASE_74.md](PHASE_74.md).** Completes the
self-learning triad — Persona (P14), reflection (Phases
70-71), and now a first-class memory surface. Four items:

- **Keyword search.** `Memory::search` (case-insensitive
  substring across topics + bodies) + the `memory.search`
  agent tool (cross-topic wildcard scope) + `SearchMemory`
  IPC + `aivyx memory search` CLI + Web UI search bar. No
  embedding dep per Q1(a) — semantic retrieval defers to a
  future RAG arc.
- **Per-topic retention.** `[[memory.retention]]` config
  blocks with topic-glob patterns + `forever` |
  `retention_days = N` policies (Q2(a)). The hourly GC pass
  applies the first matching rule; unmatched topics fall
  through to the global `ttl_secs`. New
  `Memory::gc_expired_with_rules` + `RetentionMatcher`
  boundary type keep the memory crate config-agnostic.
- **LRU eviction.** `MemoryEntry::last_read_at_secs`
  (serde-defaulted), stamped by `get_recent` on both substrate
  impls; `evict_oldest_unread` ranks `(last_read ASC, seq
  ASC)` per Q3(a). Replaces FIFO-on-write — a recalled old
  note now survives a never-read younger one.
- **Operator surfaces.** Web UI Memory pane (two-column
  browse + search + per-topic confirm-gated Evict per Q4(a))
  + `aivyx memory list/show/search/evict` CLI parity. New
  IPC: ListMemoryTopics / GetMemoryTopicEntries /
  SearchMemory queries + EvictMemoryTopic frontend message.

Streak predictions all correct: DESIGN.md → 21, PRODUCT.md →
14, `aivyx-core/src/lib.rs` → **22** (new project record,
beating Phase 73's 21). Tests +45 (1333 → 1378), above the
+30-40 prediction. Zero clippy warnings. Zero new workspace
deps.

After Phase 74 the self-learning triad is complete. Likely
follow-ups (semantic RAG, fuzzy match, edit-content Web UI,
per-topic eviction-strategy override) are operator-feedback-
gated.

## Phase 75 — Semantic RAG Memory Arc

**Frozen — see [PHASE_75.md](PHASE_75.md).** Picks up the
Phase 74 deferred "semantic RAG" follow-up: embedding-ranked
memory retrieval layered on top of keyword search, off by
default. Eight tasks:

- **`KeyDomain::MemoryVectors`** — a new encrypted, HKDF-
  isolated storage domain for vectors (precedent: Phases
  21/26/27/56/70). Entry bodies and vectors are never
  decryptable with the same subkey.
- **`EmbeddingProvider` trait + OpenAI-compatible HTTP impl** —
  reuses the existing `aivyx-llm` `reqwest` transport via a
  new non-streaming `post_json` seam. **Zero new workspace
  deps** (Q1(a)). `EmbeddingError` taxonomy mirrors the
  notify-error classification.
- **`[embedding]` config** — `base_url` / `model` / `api_key`
  / `dimensions`. Absent section → semantic disabled, keyword
  unchanged. API key resolves env > TOML > encrypted store
  (same two-phase pattern as the anthropic/openai keys).
- **Vector store + cosine** — `Memory` gains `put_vector` /
  `load_all_vectors` / `semantic_search`; an in-memory flat
  index rebuilt at open; hand-rolled cosine (no linalg dep).
  `forget` + `evict_oldest_unread` drop vectors; orphan
  vectors are skipped at query time (vector store may lag
  entry GC).
- **Write-time embed + lazy backfill (Q2(a))** — an
  `EmbeddingHook` seam keeps `aivyx-memory` free of an
  `aivyx-llm` dep; `MemoryWriteTool` embeds inline (non-fatal)
  and a bounded hourly backfill (reusing the GC timer) indexes
  the back-catalog and re-embeds stale-dimension vectors.
- **`mode` flag + keyword fallback (Q4(a))** — `memory.search`
  gains `mode = keyword|semantic` (default keyword, no
  behavior change); semantic transparently falls back to
  keyword — flagged — when no provider, embed failure, or an
  empty index. Threaded through the agent tool, `SearchMemory`
  IPC (serde-defaulted for round-trip back-compat), CLI
  `--semantic`, and a Web UI toggle.

Streak predictions all correct: DESIGN.md → **22**,
PRODUCT.md → **15**, `aivyx-core/src/lib.rs` → **23** (new
project record, beating Phase 74's 22). Tests +46 (1378 →
1424), inside the +35-50 prediction. Zero clippy warnings.
Zero new workspace deps. Privacy is the operator's `base_url`
choice — cloud or fully on-device.

Likely follow-ups (ANN index, explicit `aivyx memory reembed`,
hybrid keyword+semantic fusion, query-embedding cache) are
operator-feedback-gated.

## Phase 76 — Automatic Semantic Recall (RAG context injection)

**Frozen — see [PHASE_76.md](PHASE_76.md).** Closes the RAG arc
Phase 75 set up: the agent no longer only recalls when it calls
`memory.search` — every turn it embeds the user's message and
auto-injects the most relevant past memories. Pure integration
on the Phase 75 substrate, zero new deps.

- **`ContextProvider` planner hook (Q1a).** Read-side sibling
  of `PruneSink`; `LlmPlannerConfig::with_context_provider` +
  a `begin_turn` invocation that prepends the recalled block
  as a distinct leading text block in the user message (not
  the static system prompt; avoids provider role-alternation).
  Deliberately **not** re-exported from `aivyx-core/src/lib.rs`
  (reachable via `aivyx_core::llm_planner::ContextProvider`) —
  this is what protected the core streak.
- **`rag_top_k` (5) + `rag_min_similarity` (0.20)** on
  `[embedding]`; the floor is what stops naive-RAG noise.
- **`SemanticMemoryContext`** embeds the latest user message
  (Q2a), `semantic_search_scored` top-K, drops sub-floor hits
  (Q3a), formats an injection-safe reference-only block;
  silent no-op on embed-fail / empty index / all-below-floor —
  recall never errors a turn.
- Wired into all three planner factories (local-CLI, daemon,
  child-agent — sub-agents recall too).
- **Visible marker (Q4b) — streak-forced deviation:** a
  `ContextRecall` `AuditTag` variant would have broken the
  core streak (the enum lives in the streak file), so the
  marker is the established stderr-breadcrumb convention
  (`aivyx recall: injected N memories […]`); the recalled
  content is independently visible as the in-turn block. No
  separate Web UI indicator. The streak discipline had teeth
  this phase — a late cost was paid in scope, not in the
  contract.

Streak all three correct: DESIGN.md → 23, PRODUCT.md → 16,
`aivyx-core/src/lib.rs` → **24** (new record, beats Phase 75's
23). Tests +15 (1424 → 1439) — **below** the +25-40 prediction
(Task 5 wiring-only; Task 6 collapsed by the streak deviation;
no Web UI smoke). Honest miss, documented. Zero clippy
warnings. Zero new workspace deps.

Likely follow-ups (conversational-window query, heuristic
recall gate, token-budget context sizing, query-embedding
cache, recall-usage feedback into reflection) are
operator-feedback-gated.

## Phase 77 — Recall → Reflection Feedback Loop

**Frozen — see [PHASE_77.md](PHASE_77.md).** Closes the loop
Phase 76 opened: recall stops being a bigger cache and starts
*teaching* the system. Pure integration on the 75/76/70-71
substrate, zero new deps.

- **`KeyDomain::RecallEvents`** — a dedicated encrypted domain
  for the signal (the Phase 76 audit-streak lesson applied by
  design: route *around* `AuditTag`, not through it).
- **`PersistentRecallLog`** — `ts_be||uuid` keys (time-ordered
  scan, collision-free appends), `events_since` lookback +
  independent `gc_older_than` clamp.
- **Capture (Q1a/Q2a):** `ContextProvider::recall()` gains a
  `SessionId` (llm_planner.rs only — `lib.rs` byte-identical);
  `SemanticMemoryContext` appends a session-correlated
  `RecallEvent` per injected recall, strictly best-effort.
- **Structural correlator (Q1a):** pure, no LLM — matches a
  recall to its turn and signs it from the audit chain's
  existing `OutcomeSummary` (completed-no-followup → +;
  failed / quick-comeback → −; escalated/cancelled → 0).
- **Both actuators (Q3c):** memory-retention self-tuning
  (helpful entries kept LRU-warm via a targeted
  `promote_recall_helpful` — *no new eviction primitive*) and
  operator-gated **Pending** Persona proposals (deterministic,
  deduped, never auto-applied — Phase 70 P14 authority rule).
- **Piggybacked (Q4a):** the existing cron reflection pass
  runs correlate → retention → proposals → recall-log GC over
  the same lookback window. Zero new scheduler; whole pass is
  a no-op when the substrate is absent.

Streak all three correct: DESIGN.md → 24, PRODUCT.md → 17,
`aivyx-core/src/lib.rs` → **25** (new record, beats Phase
76's 24). Tests +22 (1439 → 1461) — **below** the +35-55
prediction (the actuators deliberately reused existing
machinery rather than adding primitives, so each is lean;
honest miss, documented). Zero clippy warnings. Zero new
workspace deps.

Likely follow-ups (`[recall_feedback]` tuning knob,
LLM-judged recall usefulness, cross-session pattern learning)
are operator-feedback-gated.

## Phase 78 — Learning Observability & Trust Surface

**Frozen — see [PHASE_78.md](PHASE_78.md).** Makes the closed
Phase 75–77 self-learning loop *legible*: an autonomous system
the operator can't see is one they can't trust. A read-only
view of what the assistant has learned and why, with full
parity, reusing existing machinery — zero new behaviour, zero
new storage, zero new deps.

- **`recall_insights` (Q1a):** `recall_feedback` refactored to
  `correlate_detailed` (tally + per-recall detail; `correlate`
  delegates — all 15 Phase 77 tests still green, one matching
  pass so the surface can never disagree with the loop).
  `build_digest` (recall counts, promoted/aging, top
  helpful/unhelpful) + `build_provenance` (each `recall-fb:`
  proposal traced to its contributing recalls/turns,
  reconstructed on-query — no schema/chain migration).
- **`GetLearningInsights` IPC + handler:** computed on-query
  from the live recall log + audit `OutcomeSummary` + the
  proposal chain (the same builders the reflection loop uses).
  No recall substrate → an empty digest, a valid "nothing
  learned yet" answer, not an error.
- **Full parity (Q2a):** `aivyx learning [--window <secs>]`
  CLI + a read-only Web UI **Learning** tab; approve/reject
  stays in the existing Proposals surface.

Streak all three correct: DESIGN.md → 25, PRODUCT.md → 18,
`aivyx-core/src/lib.rs` → **26** (new record, beats Phase
77's 25) — the surface lives entirely in `aivyx-channel`,
reusing existing types; `lib.rs` byte-identical, the
streak-shaped-architecture discipline continued. Tests +12
(1461 → 1473) — **below** even the deliberately-lowered
+18-30 prediction (third consecutive miss; pure derivation +
reused round-trip harness + thin compositional handler).
Honest, documented; calibration tightened. Zero clippy
warnings. Zero new workspace deps.

Likely follow-ups (per-memory-entry drill-down, history
beyond the recall-log window, Web UI live refresh, actionable
insights) are operator-feedback-gated.

## Phase 79 — Adaptive Persona (contextual "Soul" selection)

**Frozen — see [PHASE_79.md](PHASE_79.md).** The accreted
Persona was dumped whole into every system prompt, unbounded
and turn-blind. Phase 79 makes the Soul *adaptive*: per-turn
semantic selection of the relevant facets, reusing the Phase
76 begin_turn seam and the Phase 78 trust surface. Zero new
deps.

- **`SystemPromptRefiner` hook (Q1a):** sibling of Phase 76's
  `ContextProvider` in `llm_planner.rs` (not `lib.rs` —
  reachable via `aivyx_core::llm_planner::`); `begin_turn`
  swaps the turn's system prompt on `Some`. The planner is
  per-turn so the swap is naturally turn-scoped.
- **Core invariant (Q2a), structurally enforced:**
  `reduce_persona` copies scalar identity +
  `behavioral_constraints` through *unconditionally*; `keep`
  only ever touches the six soft list categories. No caller
  can drop identity or guardrails — proven end-to-end.
- **`PersonaContextRefiner` (Q3a):** embed the message,
  cosine-rank facets, top-K above a floor, re-assemble via the
  *unchanged* `assemble_session_prompt`. Below a size
  threshold / no embedding / embed failure → `None` →
  byte-identical full Persona. The feature is invisible until
  the Soul is large enough to need bounding.
- **Legible (Q4a):** per-turn `aivyx persona: injected N/M
  facets` breadcrumb + a `persona_selection` field on the
  Phase 78 `GetLearningInsights` surface (CLI + Web UI).

Streak all three correct: DESIGN.md → 26, PRODUCT.md → 19,
`aivyx-core/src/lib.rs` → **27** (new record, beats Phase
78's 26) — every line lives in `aivyx-channel` /
`llm_planner.rs`, reusing existing types; `lib.rs`
byte-identical, the streak-shaped-architecture discipline
continued. Tests +14 (1473 → 1487) — just under the
calibrated +15-25 (4th consecutive small miss; the band has
converged — reuse-heavy phases land ~+10-15, this had genuine
new selection logic so topped that band at +14). No
deviations: every planned surface shipped as scoped. Zero
clippy warnings. Zero new workspace deps.

Likely follow-ups (`[persona]` tuning block, Persona
consolidation/supersession/decay, behavioural Persona,
conversational-window selection) are operator-feedback-gated.

## Phase 80 — Proactive Surfacing (the assistant brings things to you)

**Frozen — see [PHASE_80.md](PHASE_80.md).** For 79 phases the
assistant only ever acted when prompted. Phase 80 is the
capstone of the 75–79 arc: on its existing reflection cadence
it notices a concrete, high-confidence reason to reach out and
**surfaces it unprompted** — the single biggest step from "a
tool you query" to "an assistant that brings things to you."
An unprompted *outbound* message is the highest-trust-stakes
action, so it ships off by default, hard-capped, and fully
explainable. Zero new deps.

- **Piggyback the reflection cron (Q1a):** no new scheduler;
  the reflection pass also runs the detector, `RecallFeedback`
  wiring precedent. No `[proactive]` / disabled / no schedule
  → complete no-op (pre-Phase-80 behaviour).
- **Structural gate, no extra LLM (Q2a):** surfaces only on a
  concrete reason in three conservative classes — `TtlExpiry`
  (entry near TTL eviction), `RecallCluster` (Phase-77 net
  helpfulness strongly positive), `DueReminder` (`@due:` time
  arrived). The Phase 77 no-self-judgement ethos applied to
  the highest-stakes action; pure, per-class + empty tested.
- **Reuse the notify dispatcher (Q3a):** every send is a
  normal auto-notify — same `AutoNotifyDispatched` audit
  event, same notify history, Phase 73's per-target
  rate-limit.
- **Default-off, capped, explainable (Q4a):** opt-in
  `[proactive]` in `aivyx-config`; an HKDF-isolated
  `KeyDomain::ProactiveLog` never-nag dedup store
  (`was_surfaced` + GC clamp); a deterministic hard
  `max_per_window` cap on top of the rate-limit; per-cycle
  breadcrumb + a `proactive` field on the Phase 78
  `GetLearningInsights` surface (CLI + Web UI).

Streak all three correct: DESIGN.md → **27**, PRODUCT.md →
**20**, `aivyx-core/src/lib.rs` → **28** (new record, beats
Phase 79's 27) — the proactive pass is a structural,
no-LLM/no-turn composition through the existing dispatcher, so
it produced the *existing* audit event with no new `AuditTag`;
every line lives in `aivyx-channel` / `aivyx-config` /
`aivyx-storage`; `lib.rs` byte-identical, the
streak-shaped-architecture discipline continued. Tests **+20**
(1487 → 1507) — **in band** (predicted ~+14-22, top), the
first in-band landing after four consecutive small misses: a
new KeyDomain + config section + a heavily-tested pure
detector + an integration pass carried the genuine new
surface the converged calibration anticipated. No deviations:
every planned surface shipped as scoped. Zero clippy warnings.
Zero new workspace deps.

Likely follow-ups (standalone `[[proactive_schedule]]`,
LLM-composed proactive prose, conversational/interactive
proactive, additional signal classes) are
operator-feedback-gated.

## Phase 81 — Persona Lifecycle (the Soul that refines itself)

**Frozen — see [PHASE_81.md](PHASE_81.md).** For 80 phases
the Persona only ever grew. Phase 81 closes the open half of
the identity arc (the Phase 79 deferral, made consequential by
Phase 80): on the existing reflection cadence the assistant
*proposes* consolidation of near-duplicate facets and decay of
long-unreinforced ones — the "self-improving" half of the
identity layer. Zero new deps.

- **Piggyback the reflection cron (Q1a):** `PersonaLifecycleDeps`
  threaded into `run_reflection_scheduler` exactly like Phase
  77/80; no new scheduler. No section / disabled / no schedule
  → complete no-op.
- **Propose-only, operator-gated (Q2a):** the pass files
  normal Pending `PersonaProposal`s (`RemoveList` ops) and
  **never resolves** them — the Phase 77/79 no-self-mutation
  ethos applied to the highest-stakes layer; `Revert` makes
  every approved action reversible. Consolidation removes the
  shorter near-duplicates and keeps the longest *existing*
  member, so no `AppendList` is needed and each removal is
  independently safe (a deliberate simplification of the plan's
  "RemoveList×N + AppendList" recipe — see prediction vs.
  reality).
- **Structural + embedding, no LLM (Q3a):** `soft_facets_of`
  is the core-protection choke point (six soft lists only —
  scalars + `behavioral_constraints` can never become an
  action, the Phase 79 invariant extended, proven by test);
  consolidation reuses the Phase 79 cosine seam, decay is
  age-based; deterministic, no new persistence.
- **Default-off, legible (Q4a):** opt-in `[persona_lifecycle]`
  in `aivyx-config`; deterministic proposal ids dedup across
  cycles against the chain in *any* status (never nag); a
  per-cycle breadcrumb + a `persona_lifecycle` field on the
  Phase 78 `GetLearningInsights` surface (CLI + Web UI).

Streak all three correct: DESIGN.md → **28**, PRODUCT.md →
**21**, `aivyx-core/src/lib.rs` → **29** (new record, beats
Phase 80's 28) — lifecycle actions are existing-shape
proposals through the existing proposal/persona chains, so no
new `KeyDomain` and no new `AuditTag`; every line lives in
`aivyx-channel` / `aivyx-config`; `lib.rs` byte-identical, the
streak-shaped-architecture discipline continued. Tests **+17**
(1507 → 1524) — **in band** (predicted ~+16-22), the second
consecutive in-band landing; slightly lighter than Phase 80's
+20 exactly as predicted (no new KeyDomain), the
embedding-clustering detector + core-protection invariant +
config carrying the genuine new surface. One scoped
simplification (consolidation needs no `AppendList`), recorded
honestly. Zero clippy warnings. Zero new workspace deps.

Likely follow-ups (helpfulness-driven decay,
contradiction-based supersession, standalone
`[[persona_lifecycle_schedule]]`, facet-scoped one-click
revert) are operator-feedback-gated.

## Phase 82 — Persistent Helpfulness Ledger (durable, longitudinal self-learning)

**Frozen — see [PHASE_82.md](PHASE_82.md).** For 81 phases
the "did recalling this topic help" signal was ephemeral
(Phase 77 recomputed a tally each window and discarded it) —
the common cause behind Phase 81's age-only decay, Phase 77's
deferred cross-session patterns, and Phase 78's deferred
longitudinal history. Phase 82 makes it durable: a persisted,
per-topic, time-decayed EWMA folded in on the existing
reflection cadence. Zero new deps.

- **Zero-config (Q4a):** the Phase 77 / `RecallEvents`
  precedent — a passive internal signal, auto-initialised, no
  `[…]` block. New HKDF-isolated `KeyDomain::HelpfulnessLedger`
  (15th variant, the Phase 80 checklist). It changes no
  behaviour on its own (folded *after* the recall-feedback
  actuators, byte-identical).
- **Recency-weighted EWMA (Q1a/Q2a):** per topic
  `{ ewma_score, samples, last_update_secs }`; each cycle the
  stored score is decayed by `0.5^(dt/half_life)` (~60d) then
  the window net added. A topic that stopped helping fades on
  its own; per-topic grain survives memory eviction. Self-
  pruning (decayed-to-epsilon + stale → dropped) so growth
  mirrors the signal's own decay.
- **Fold-in on the existing pass (Q… cadence):** the Phase 77
  `run_recall_feedback_pass` aggregates `tally.ranked()` to a
  per-topic net and folds + prunes — no new scheduler, no new
  pass, threaded via the existing `RecallFeedbackDeps`.
- **Surface-only this phase (Q3a):** the read-only
  `GetLearningInsights` gains the decayed accumulated
  top-helpful/unhelpful view + sample counts (the Phase 78
  longitudinal deferral), rendered in the CLI + Web UI. It
  does **not** rewire Phase 81 decay — the topic→category
  mapping is the explicit next phase.

Streak all three correct: DESIGN.md → **29**, PRODUCT.md →
**22**, `aivyx-core/src/lib.rs` → **30** (new record, beats
Phase 81's 29) — the new store is a `KeyDomain` in
`aivyx-storage`; the ledger, fold-in, and surface live in
`aivyx-channel`; no new `AuditTag`, `lib.rs` byte-identical.
Tests **+10** (1524 → 1534) — a **MISS below the predicted
~+18-24 band** (first miss after two in-band landings): the
prediction over-weighted "has a new KeyDomain ≈ Phase 80's
+20" and under-weighted that this phase is *zero-config*
(no config-validation tests — Phase 80 had ~6) and
*surface-only* (no detector/behaviour test breadth). The
honest recalibration: a new KeyDomain alone is ~+7; it is the
config + detector + behaviour breadth that drives test count,
not the storage variant. Every planned surface still shipped;
this is a calibration miss, not a scope miss. One unplanned
**test-only** fix (a pre-existing `pid-nanos` shared-store
path collision in `storage_persistence_e2e`, exposed by the
extra KeyDomain shifting redb-open timing under loaded
parallel runs — fixed with a uuid suffix; no production
change, no new dep). Zero clippy warnings. Zero new workspace
deps.

Likely follow-ups (helpfulness-driven Persona decay — needs a
topic→category mapping; operator-tunable half-life/retention;
topic canonicalization) are operator-feedback-gated.

## Phase 83 — Cross-Session Pattern Learning (the durable co-occurrence ledger)

**Frozen — see [PHASE_83.md](PHASE_83.md).** Phase 77's
headline deferral, unblocked by the Phase 82 durable-ledger
model. Per-topic helpfulness is shallow; the relationships
*between* topics — which travel together and jointly help —
are where cross-session structure lives. Phase 83 adds a
persistent, time-decayed **co-occurrence ledger** folded in on
the existing reflection cadence. Zero new deps.

- **Co-occurrence pairs (Q1a):** unordered `{A,B}` recalled in
  the same `RecallEvent`; the Phase 77 uniform per-event
  signal makes "this co-recalled set landed in a helpful
  turn" directly observable. Sequential/n-ary defer.
- **Durable derived store (Q2a):** a new HKDF-isolated
  `KeyDomain::CooccurrenceLedger` (16th), key = a
  collision-safe **length-prefixed** canonical pair (so
  `{A,B}=={B,A}` yet `("a|","b")≠("a","|b")`), value = the
  Phase 82 EWMA `{score,samples,last_update}`. The raw recall
  log GC's at 30 days, so the durable derived store is the
  only genuine cross-session path. Exactly the Phase 82
  model.
- **Surface-only (Q3a):** detect + persist + show on the
  Phase 78 surface; change no behaviour. Consumption is the
  explicit next phase — the Phase 82 substrate-then-
  consumption discipline.
- **Zero-config, bounded (Q4a):** the Phase 77/82
  passive-signal precedent; the O(n²) blowup bounded by
  folding only pairs among the **top-8 highest-scoring
  distinct topics** per event; EWMA-decay + self-prune;
  surfaced on `GetLearningInsights` (CLI + Web UI) +
  breadcrumb; constants not config.

Streak all three correct: DESIGN.md → **30**, PRODUCT.md →
**23**, `aivyx-core/src/lib.rs` → **31** (new record, beats
Phase 82's 30) — the new store is a `KeyDomain` in
`aivyx-storage`; the ledger, fold-in, and surface live in
`aivyx-channel`; no new `AuditTag`, `lib.rs` byte-identical.
Tests **+10** (1534 → 1544) — **a small miss vs the
phase-specific ~+12-16 refinement**, but squarely in the
Phase-82-recalibrated ≈ +8-12 band: the cross-session pair
detector landed at *exactly* Phase 82's +10, not above it.
The empirical lesson (refined again): a zero-config,
surface-only, one-new-KeyDomain phase is ≈ +10 regardless of
whether the store is per-topic or per-pair — the test surface
is dominated by the fixed scaffolding (KeyDomain isolation ×2,
ledger CRUD/decay/prune ×~6, integration ×1, render ×1), not
by detector complexity. Every planned surface shipped. One
in-scope clippy resolution: the `LearningInsights` protocol
payload accretes one read-only field per learning phase, so
`#[allow(clippy::large_enum_variant)]` on the
`DaemonMessage`/`DaemonEnvelope` envelopes with justification
(consistent with the codebase's existing
`#[allow(too_many_arguments)]` practice). Zero clippy
warnings. Zero new workspace deps.

Likely follow-ups (helpfulness-driven Persona decay; pattern
*consumption* — cluster-aware co-recall, pattern-driven
proposals; sequential/temporal patterns; n-ary clusters;
operator-tunable top-K/half-life) are operator-feedback-gated.

## Phase 84 — Cluster-Aware Co-Recall (the first consumption phase)

**Frozen — see [PHASE_84.md](PHASE_84.md).** Phases 82–83
built durable learning substrate surface-only; Phase 84 is the
first phase that *acts* on it, consuming the freshest piece
(the Phase 83 co-occurrence ledger). Auto-recall (Phase 76)
surfaces only literal keyword/semantic matches; now, when a
topic is recalled, its durable affined siblings the query
missed are also surfaced — recall becomes associative. Zero
new deps.

- **Bounded sibling injection (Q1a):** after the base recall,
  `siblings_of` (a new one-scan per-topic ledger query) yields
  the strongest affined siblings; the most-recent memory under
  each new sibling topic is injected. Re-rank-only was
  rejected — it cannot surface a sibling the query never
  retrieved.
- **Opt-in (Q2a):** `[recall_cluster]` in `aivyx-config`, off
  by default — the first phase that changes hot-path context,
  so the Phase 80/81 behaviour-change discipline.
- **Self-policing (Q3a):** a serde-safe `cluster` marker on
  `RecallHit`; the Phase 83 co-occurrence fold *excludes*
  marked hits (the ledger never learns from its own expansion
  — no runaway self-reinforcement), while the Phase 77/82
  helpfulness signal still measures them (a bad expansion
  self-penalises and the driving affinity decays).
- **Budget-neutral + legible (Q4a):** siblings share the
  existing `rag_top_k` budget (displace the weakest primary
  hits — zero context/token growth); hard `max_siblings` cap
  + `min_affinity` floor; a per-turn breadcrumb + a Phase 78
  `cluster_recall` surface (CLI + Web UI), the persona-
  selection shared-handle pattern.

Streak all three correct: DESIGN.md → **31**, PRODUCT.md →
**24**, `aivyx-core/src/lib.rs` → **32** (new record, beats
Phase 83's 31) — the recall provider is the existing
`ContextProvider` (the Phase 76/79 `llm_planner.rs` seam, not
`lib.rs`); config/query/marker/expansion/surface all in
`aivyx-channel` / `aivyx-config`; no new `AuditTag`,
byte-identical `lib.rs`. Tests **+10** (1544 → 1554) — **a
second consecutive miss vs the ~+16-22 prediction**; it
landed at the same flat ≈ +10 as Phases 82/83. The recurring,
now-confirmed calibration law: realized test count is driven
by **new standalone pure modules each carrying a per-branch
unit suite** (a detector ≈ +7, a new `KeyDomain` ≈ +2, a
config section ≈ +5-6) — *not* by config-presence or
hot-path-behaviour breadth. Phase 84 added **no** new detector
module and **no** new `KeyDomain` (it reused `memory_recall` +
the Phase 83 ledger, integration-testing the new behaviour),
so it sits at the floor ≈ +10 regardless of being a
config+behaviour phase. Future predictions: count new
unit-tested pure modules, not surface area. A calibration
miss, not a scope miss — every planned surface shipped. Two
in-scope clippy resolutions, recorded honestly:
`#[allow(large_enum_variant)]` was already in place (Phase
83); `render_insights` crossed `too_many_arguments` (one
display param per learning phase) → justified
`#[allow(too_many_arguments)]`, the Phase 81 fire_reflection
precedent. Zero new clippy warnings. Zero new workspace deps.

Likely follow-ups (pattern-driven Persona proposals; affinity
re-ranking of existing candidates; sequential/temporal
patterns; operator-tunable affinity policy) are
operator-feedback-gated.

## Phase 85 — Helpfulness-Driven Persona Decay (the self-improving Soul, completed)

**Frozen — see [PHASE_85.md](PHASE_85.md).** The symmetric
consumption to Phase 84 and the long-deferred Phase 81+82
capstone: Phase 81 decay was age-only; Phase 85 wires the
durable Phase 82 helpfulness ledger so the Soul retires
identity that demonstrably *stopped helping* and protects old
identity that *still helps*. Zero new deps.

- **Provenance-only association (Q1a):** the topic is
  recovered structurally from `proposal_id ==
  "recall-fb:{topic}"` (it survives onto the
  `PersonaDelta`); only recall-feedback-derived facets are
  helpfulness-gated, reflection-authored facets stay age-only
  (no fuzzy embedding driving identity removal). The
  long-flagged "blocker" turned out to be exact.
- **Symmetric gate (Q2a):** sustained-negative triggers decay
  before the age horizon; sustained-positive protects an
  age-old facet from age-decay. Propose-only + `Revert` +
  operator-gated + core-protected — every Phase 81 safety
  property carries over.
- **Conservative evidence (Q3a):** decayed EWMA at/below
  `decay_unhelpful_threshold` **and** `samples >=
  decay_min_samples`; never on thin evidence; recency is
  inherent in the ledger half-life.
- **Cohesive + graceful (Q4a):** two knobs on the existing
  `[persona_lifecycle]` block (gated by `signal_decay`); no
  helpfulness ledger → pure age-only, byte-identical to Phase
  81; the decay `reason` cites the evidence; the existing
  Phase 78 lifecycle surface renders it unchanged (no new IPC
  field).

Streak all three correct: DESIGN.md → **32**, PRODUCT.md →
**25**, `aivyx-core/src/lib.rs` → **33** (new record, beats
Phase 84's 32) — decay actions remain existing-shape
proposals through the existing chains; detector/pass/config/
surface all in `aivyx-channel` / `aivyx-config`; no new
`AuditTag`, byte-identical `lib.rs`. Tests **+5** (1554 →
1559) — below the ~+8-12 nominal but exactly the "slightly
under, no new config *section*" case the open doc explicitly
hedged: the now-converged calibration law (count tracks new
unit-tested pure modules) gains a sharper sub-rule —
**config *knobs on an existing block* ≈ +1, extension-only
detector/pass changes ride existing test files (≈ +3 units +
1 integration)**, so a phase with no new module, no
`KeyDomain`, *and* no new config section floors at ≈ +5, not
+10. First prediction in the recent run to call the
under-shoot direction correctly (the hedge held). A
*calibration* refinement, not a *scope* miss — every planned
surface (provenance recovery, the symmetric gate, the
evidence floor, graceful no-ledger fallback, evidence-cited
reason) shipped exactly as scoped. Zero clippy warnings. Zero
new workspace deps. With no ledger, decay is byte-identical
to Phase 81 (asserted).

Likely follow-ups (helpfulness-driven *consolidation*;
helpfulness decay for reflection-authored facets via fuzzy
embedding; pattern-driven Persona proposals; topic
canonicalization) are operator-feedback-gated.

## Phase 86 — Conversational-Window Relevance (sharpening the whole stack's input, completed)

**Frozen — see [PHASE_86.md](PHASE_86.md).** The twice-deferred
(Phase 76 *and* Phase 79) input-quality gap: for 85 phases the
assistant judged relevance off *one line*. Phase 86 gives both
relevance consumers — auto-recall (76) and adaptive Persona
selection (79) — a recent conversational window: a small,
recency-ordered slice of the last few `(user, assistant)`
turns concatenated into the same single embed they already
make (current message last so it dominates). Sharper input
under the entire self-learning stack with zero new deps and
opt-in defaults.

- **Daemon-scoped session-keyed buffer (Q1a):** a new
  `conversation_window` module in `aivyx-channel` —
  `Arc<RwLock<HashMap<SessionId, ConversationWindow>>>` shared
  at daemon startup (the Phase 82/84 shared-handle precedent),
  written by the daemon turn loop on each
  `TurnOutcome::Completed`. Ephemeral by design (no new
  `KeyDomain`); a restart starts fresh.
- **Single-vector composition (Q2a):** the last
  `recall_window_turns - 1` prior turns (oldest → newest,
  role-labelled) followed by the current message, char-budgeted
  with oldest-first eviction; current message is never
  truncated. One embed call, unchanged ranking math.
- **Both consumers (Q3a):** auto-recall *and* Persona
  selection — same seam, same knob; the deferral came from
  both, fixing one without the other was incoherent.
- **Opt-in, byte-identical by default (Q4a):**
  `[embedding].recall_window_turns` (default `1` = exactly the
  latest single message); below the floor the embedded query
  is bit-for-bit pre-Phase-86. The trait-extension ripple
  (adding `session_id` to `SystemPromptRefiner::refine`) lives
  in `llm_planner.rs`, *not* the byte-identical `lib.rs`. A
  positive cascade: the daemon's `Message::session_id` is now
  stable across turns (was fresh per turn), which both makes
  the buffer key load-bearing and corrects Phase 77 recall
  correlation.

Streak all three correct: DESIGN.md → **33**, PRODUCT.md →
**26**, `aivyx-core/src/lib.rs` → **34** (new record, beats
Phase 85's 33) — the trait edit lives in `llm_planner.rs`, the
new module + handle thread through `aivyx-channel`, and the
config knob is a field on the existing `[embedding]` block.
Test count delta within the converged calibration band — one
new pure module + a knob on an existing config section + two
provider integration tests on each side (the assemble-engaged
and fallback-matrix pairs). Zero clippy warnings. Zero new
workspace deps. With `recall_window_turns = 1` (the default),
both consumers are byte-identical to pre-Phase-86 (asserted on
both providers via recording-provider matrix tests).

Likely follow-ups (token-budget context sizing, embed-each-
and-pool windows, persisted windows, heuristic recall gate)
are operator-feedback-gated.

## Phase 87 — Pattern-Driven Persona Proposals (the self-improving Soul, the second consumption, completed)

**Frozen — see [PHASE_87.md](PHASE_87.md).** Closes the
deliberate Phase 85 deferral. After 84 (recall acts on the
Phase 83 co-occurrence ledger) and 85 (decay acts on the
Phase 82 helpfulness ledger), the visible asymmetry was that
the co-occurrence ledger fed only *recall*, not the *Soul*.
Phase 87 closes the symmetric arc: durable consistently-
co-occurring pairs of *helpful* topics propose a new
`learned_context` facet through the existing Phase 70 chain —
same propose-only + edit-then-approve + Revert + core-
protected flow, just driven by the second durable signal.

- **Conservative double-gate (Q1a):** pair affinity AND both
  endpoints helpful — a pattern made of topics that
  individually hurt is never proposed. Mirrors Phase 85's
  evidence-floor discipline; same `min_topic_helpfulness`
  knob defaults to `0.0` (non-negative).
- **LLM-summarized facets (Q2b):** the existing reflection
  LLM phrases each surviving pair into a one-sentence
  `learned_context` facet. The operator is still the final
  filter (edit-then-approve / reject); a per-candidate LLM
  hiccup skips that pair, a cycle-wide outage records
  `llm_unavailable = true` on the Phase 78 surface so a quiet
  cycle stays distinguishable from a broken one.
- **Reflection cron + cap + dual dedup (Q3a):** the
  established "act on durable learning" cadence (Phase 77 /
  82 / 83 / 85); idempotent — a pair already in the proposal
  chain (any status) is never re-filed; per-cycle filings
  bounded by `max_proposals_per_cycle`.
- **Opt-in by default (Q4a):** new `[persona_consolidation]`
  config block, `enabled = false` default; with no block the
  pass never runs (byte-identical to pre-Phase-87). The Phase
  80/81/84 actuator posture.

Streak all three correct: DESIGN.md → **34**, PRODUCT.md →
**27**, `aivyx-core/src/lib.rs` → **35** (new project
record, beats Phase 86's 34) — the new pass + config block +
Phase 78 surface stat all live in `aivyx-channel` /
`aivyx-config` / `bin/aivyx`; proposals land through the
existing `PersistentPersonaProposalLog::append` API (no new
chain operation); no new `AuditTag`. Test count delta within
the predicted `+11-15` band (`+15` exactly — config section
+6, pure module +8, integration +1). Zero clippy warnings.
Zero new workspace deps.

Likely follow-ups (**pattern-driven Persona *decay* —
shipped in Phase 88**, n-ary cluster proposals, pattern-
driven supersession, operator-tunable LLM prompt, topic
canonicalization) are operator-feedback-gated.

## Phase 88 — Pattern-Driven Persona Decay (the decay-side of the Phase 87 arc, completed)

**Frozen — see [PHASE_88.md](PHASE_88.md).** Closes the
deliberate Phase 87 deferral with the decay half of the
symmetric arc. After Phase 87 made the Phase 83 co-occurrence
ledger drive Persona *construction*, Phase 88 makes the
**same ledger** drive Persona *decay*: a `consolidate-pair:`
facet whose underlying pair has demonstrably weakened
(decayed Phase 83 affinity below the new
`decay_pair_below_affinity` floor) is decay-proposed — the
relationship that justified the identity no longer holds.
Symmetrically, a still-durable pair **protects** its facet
from age-decay (the relationship still applies, so the
identity still applies). After Phase 88, every durable
learning signal is consumed on both sides of the Soul
lifecycle.

- **Single-signal decay (Q1a/Q2a):** decayed affinity below
  the floor is sufficient. Endpoint helpfulness is not
  double-consulted; conditioning would leave drifted-but-
  warm pair facets in place forever — the very case Phase
  88 is meant to handle.
- **Symmetric protection (Q3a):** pair at/above the floor
  protects from age-decay. Reuses the Phase 85 OR-protection
  machinery on a parallel provenance arm.
- **Same `[persona_lifecycle]` block (Q4a):** new
  `decay_pair_below_affinity` knob (default `1.0` — mirrors
  Phase 87's `min_affinity` so the construction and decay
  floors coincide by default; operators tune below for
  hysteresis). Gated by the existing `signal_decay`.

Streak all three correct: DESIGN.md → **35**, PRODUCT.md →
**28**, `aivyx-core/src/lib.rs` → **36** (new project
record, beats Phase 87's 35) — detector extension + config
knob + fold-site read all live in `aivyx-channel` /
`aivyx-config`; the decay proposals land through the existing
`PersistentPersonaProposalLog::append` API (no new chain
operation, no new `AuditTag` — the Phase 85 precedent).
Test count delta `+6` (`+1` config, `+4` detector, `+1`
integration) — inside the predicted `+3-7` band. Zero clippy
warnings. Zero new workspace deps. With no co-occurrence
ledger the pair arm sits out entirely → byte-identical to
Phase 85 (asserted on the existing Phase 85 test, which
runs with `cooccurrence_ledger: None`).

## Phase 89 — Topic Canonicalization (sharper learning through sharper bookkeeping, completed)

**Frozen — see [PHASE_89.md](PHASE_89.md).** Closes the
longest-standing learning-stack deferral — the Phase 82
deferral carried forward six times through Phases 83-88. For
88 phases every topic-keyed accumulator (the Phase 7 memory,
the Phase 77 recall log, the Phase 82 helpfulness ledger,
the Phase 83 co-occurrence ledger, the Phase 87 consolidate-
pair proposal IDs) keyed by the operator's typed topic string
verbatim — so `deploy`, `Deploy`, `deploys`, `deploying` were
four distinct topics across every signal, and the value that
should add up across them was silently fragmented. Phase 89
is the first phase since the act-on-durable-learning arc
closed that **sharpens existing signals** rather than adding
a new capability — the natural infrastructure pause before
the next big surface.

The fix is the smallest possible substrate change: an opt-in
canonicalization seam at the `Memory` trait's topic-string
boundary. Every downstream consumer inherits clean signal
through the existing pipeline — no per-layer plumbing.

- **Hand-rolled English stemmer (Q1a):** lowercase + trim +
  whitespace fold + one suffix-strip rule with min-length
  guards (`ies → y`, `ing` / `ed` / hissing-`es` / `s`).
  The `es` rule fires only when the stem ends in a hissing
  sound (`sh` / `ch` / `s` / `x` / `z`) — the real English
  plural rule — so `boxes → box` but `roles` falls through
  to `s` rule → `role`. Idempotent. Zero new deps.
- **Write-side only (Q2a):** no migration. Existing
  fragmented signal decays out via the Phase 82/83 ~60-day
  half-life + the Phase 77 ~30-day recall-log retention;
  the past converges to clean within ~quarter without
  intervention; MAC-signed Persona chain entries stay
  untouched.
- **`Memory::put` boundary (Q3a):** a single wrapper-
  delegate (`CanonicalizingMemory`) canonicalizes at every
  topic-keyed trait entry point (`put` / `get_recent` /
  `forget` / `gc_topic` / `evict_oldest_unread` /
  `put_vector` / `promote_recall_helpful`). One
  canonicalization site per method; prefix matching,
  text-search queries, and topic-less methods pass through
  unchanged.
- **Opt-in (Q4a):** `[memory].canonicalize_topics: bool`,
  default `false`. Matches the 88-phase
  behaviour-change-is-opt-in discipline; with the flag off
  the memory layer is byte-identical to pre-Phase-89.

Streak all three correct: DESIGN.md → **36**, PRODUCT.md →
**29**, `aivyx-core/src/lib.rs` → **37** (new project record,
beats Phase 88's 36) — the canonicalization function +
wrapper live in `aivyx-memory`, not `aivyx-core`; the config
knob + binary wiring touch no production-core code. Test
count delta `+20` workspace (`+3` config, `+13` pure module,
`+4` wrapper) — comfortably above the predicted `+6-10`
band; the rich rule coverage in Task 3 (each suffix rule +
the hissing-sound guard + idempotency + non-ASCII pass-
through + path-like-topic + short-string guards) all earned
their own test. Zero clippy warnings. Zero new workspace
deps. With `canonicalize_topics = false` (the default), the
memory layer is byte-identical to pre-Phase-89 (asserted on
the `without_wrapper_variants_stay_distinct_baseline` test).

Likely follow-ups (operator-tunable `[[topic_alias]]`
mappings, topic-by-topic exception list, one-time migration
of existing fragmented data, non-ASCII / Unicode stemming)
are operator-feedback-gated.

## Phase 90 — Heuristic Recall Gate (the third move in the input-quality arc, completed)

**Frozen — see [PHASE_90.md](PHASE_90.md).** Closes the
longest-running recall-side deferral (Phase 76, carried
forward 14 phases through 77-89). For 89 phases both
auto-recall (Phase 76) and adaptive Persona selection
(Phase 79) fired on **every** conversational turn —
including turns where the user message is a single-token
acknowledgment (`ok` / `thanks` / `yes` / `cool`) that
cannot meaningfully steer recall or facet selection.

Phase 90 closes this with the smallest possible gate: a
length-based heuristic at the top of both relevance hooks
that skips the embed (and everything downstream) when the
trimmed user message is shorter than `recall_gate_min_chars`
Unicode characters. Both consumers share the same gate
under one opt-in knob, exactly as Phase 86's window work
shipped both consumers under one switch.

The third move in the input-quality arc:
- **Phase 86** — sharpened *what* gets embedded (windows)
- **Phase 89** — sharpened *how* signals key (canonical)
- **Phase 90** — sharpens *when* recall fires at all

- **Length-based gate (Q1a):** trimmed Unicode-char count
  strictly-less-than `recall_gate_min_chars` → short-
  circuit. Simple, deterministic, language-agnostic.
- **Opt-in (Q2a):** `[embedding].recall_gate_min_chars`,
  default `0` (gate disabled = byte-identical to
  pre-Phase-90). Matches the 89-phase
  behaviour-change-is-opt-in discipline.
- **Both consumers (Q3a):** `SemanticMemoryContext::recall`
  AND `PersonaContextRefiner::refine` short-circuit on the
  same gate under the same shared knob. Symmetric Phase 86
  design.
- **Skip embed entirely (Q4a):** the gated turn pays zero
  embed cost (not just memory-walk or ranking) — the
  cheapest possible noise-turn path; uses the existing
  best-effort `None` fallback contract both providers
  already honoured.

Streak all three correct: DESIGN.md → **37**, PRODUCT.md →
**30**, `aivyx-core/src/lib.rs` → **38** (new project
record, beats Phase 89's 37) — the gate function +
provider short-circuits live in `aivyx-channel`, not
`aivyx-core`; the config knob is a new field on the
existing `EmbeddingConfig`; zero new `AuditTag`. Test count
delta **`+15`** workspace (`+3` config, `+6` pure helper,
`+6` provider integration) — over the predicted `+6-10`
band; the recording-provider matrix on both providers
earned its own coverage. Zero clippy warnings. Zero new
workspace deps. With `recall_gate_min_chars = 0` (the
default), both providers are byte-identical to pre-Phase-90.

Likely follow-ups (pattern-based stoplist, LLM-judged gate,
adaptive thresholds from operator message-length
distribution, token-budget context sizing) are
operator-feedback-gated.

## Phase 91 — LLM-Judged Recall Usefulness (the missing half of the learning loop, completed)

**Frozen — see [PHASE_91.md](PHASE_91.md).** Closes the
longest-running feedback-side deferral — the Phase 77
deferral carried forward 14 phases through 78-90. For 90
phases the recall-feedback signal has been STRUCTURAL: every
recall in a successfully-completed turn inherits `+1`
helpfulness, every recall in a failed turn inherits `-1`. The
proxy works but operates at turn-level granularity. Phase 91
adds an opt-in LLM-judged per-recall classification
alongside the structural proxy — a 3-way verdict
(`Used` / `Irrelevant` / `Hurt`) recorded on each recall
hit, finer than turn-level.

After the input-quality arc (86 windows, 89 canonical, 90
gate), Phase 91 is the symmetric **feedback-quality** move
that completes the learning-loop picture:

|                                                | Sharpens               |
|------------------------------------------------|------------------------|
| Phase 86 — Conversational window               | *What* gets embedded   |
| Phase 89 — Topic canonicalization              | *How* signals key      |
| Phase 90 — Heuristic recall gate               | *When* recall fires    |
| **Phase 91** — LLM-judged recall usefulness    | **Whether** it helped  |

- **Reflection cron, batched (Q1a):** one LLM call per
  cron tick judging every unjudged recall in the lookback
  window (up to `max_recalls_per_cycle`); the remainder
  rolls to the next cycle. Bounded cost.
- **3-way structured (Q2a):** the LLM produces `Used` /
  `Irrelevant` / `Hurt` per recall — easy to aggregate to
  the existing `+1 / 0 / -1` helpfulness shape, deterministic
  decode, trivially mockable.
- **Augment, NOT replace (Q3a):** the new
  `judgment: Option<RecallJudgment>` field on `RecallHit` is
  captured but no existing accumulator (Phase 82 helpfulness
  ledger, Phase 83 co-occurrence ledger, Phase 85/88 decay,
  Phase 87 proposals) consumes it in v1. Every existing
  behaviour stays byte-identical to pre-Phase-91. A future
  phase reads the new signal once it's validated in
  production.
- **Opt-in (Q4a):** `[recall_judgment].enabled`, default
  `false`. The LLM call has real cost; the operator opts
  into paying it.

Streak all three correct: DESIGN.md → **38**, PRODUCT.md →
**31**, `aivyx-core/src/lib.rs` → **39** (new project
record, beats Phase 90's 38) — the new trait + adapter +
pass + stat + IPC field all live in `aivyx-channel`; the
config block + provenance tracking in `aivyx-config`; no
new `AuditTag` (the judgment is recorded on the existing
recall log, not on the audit chain). Test count delta
**`+14`** workspace (`+5` config, `+8` module + wire-compat,
`+1` reflection-scheduler integration) — inside the
predicted `+9-15` band. Zero clippy warnings. Zero new
workspace deps.

**v1 simplification.** The judge classifies based on the
recall's `(topic, body)` content + a weak context hint —
not the model's actual response text (which isn't in the
audit chain today). The Q3a augment posture means even this
weaker v1 judgment changes nothing; future phases enrich
the input via audit-chain extension or per-turn capture.

Likely follow-ups (actuator-side switch from structural
proxy to the new judgment signal, per-recall LLM critique,
adaptive batch size, multi-model ensembling, response-text
recovery via audit-chain extension) are operator-feedback-
gated.

## Phase 92 — Pattern-Driven Supersession (the longest-running Persona-actuator deferral, closed)

**Frozen — see [PHASE_92.md](PHASE_92.md).** Closes the
Phase 70 proposal-supersession deferral — 22 phases old,
deferred again at Phase 87 and Phase 88. After Phase 87
(pattern-driven construction) and Phase 88 (pattern-driven
decay), the Soul actuator handled a shifting co-occurrence
pair `(A, B) → (A, C)` as TWO independent operator
decisions. Phase 92 introduces opt-in **shared-endpoint
supersession**: when an existing applied `consolidate-pair:`
facet's pair decays while a new pair sharing one endpoint
strengthens, the consolidation pass files the `RemoveList`
+ `AppendList` proposals **linked by metadata** so the
operator-facing surface presents them as a single
supersession decision.

- **Shared-endpoint detection (Q1a):** old `(A, B)`
  decayed below the Phase 88 floor AND new `(A, C)`
  sharing one endpoint above the Phase 87 construction
  floor with both endpoints helpful. Conservative;
  deterministic; only fires on clear "replacement"
  relationships.
- **Reuse Phase 87's `PairPhraser` (Q2a):** structural
  detection is pure; the new facet's prose comes from
  the same LLM seam Phase 87 already pays. Per-candidate
  phrasing failure → skip that supersession; the facet
  stays via the standard Phase 87/88 flow.
- **Linked, not atomic (Q3a):** the new optional
  `supersedes_proposal_id: Option<String>` field on
  `ProposedPersonaDelta` carries the cross-link. The
  `AppendList`-side proposal points at the
  `RemoveList`-side proposal_id; the `RemoveList`-side
  points back. Each half remains independently
  `Revert`-able; no new proposal kind; no chain-schema
  migration. Wire-compat via the Phase 84 / Phase 91
  `#[serde(default, skip_serializing_if = "...")]`
  precedent.
- **Opt-in (Q4a):** new
  `enable_supersession: bool` knob on the existing
  `[persona_consolidation]` block, default `false`.
  Operators running Phase 87 consolidation enable it with
  one extra key; with the knob off the Phase 87 / Phase 88
  proposal flow is byte-identical to pre-Phase-92.

Streak all three correct: DESIGN.md → **39**, PRODUCT.md →
**32**, `aivyx-core/src/lib.rs` → **40** (new project
record, beats Phase 91's 39) — detector + field + pass
integration all in `aivyx-channel`; config knob in
`aivyx-config`; no new `AuditTag`. Test count delta
**`+10`** workspace (`+3` config, `+6` field + detector,
`+1` integration) — squarely inside the predicted `+6-10`
band. Zero clippy warnings. Zero new workspace deps.

Likely follow-ups (Web UI visual grouping of linked
supersession proposals, atomic chain-level supersession
primitive via a new `Supersede` proposal kind or
`Compound` proposals, n-ary cluster supersession,
semantic-similarity supersession for synonym pairs that
share zero literal endpoints) are operator-feedback-
gated.

## Phase 93 — Recall-Feedback Switches to LLM-Judgment Signal (closing the Phase 91 deferral)

**Frozen — see [PHASE_93.md](PHASE_93.md).** Closes the
Phase 91 deferral named verbatim in the Phase 92 open doc:
"actuator-side switch from structural proxy to the new
judgment signal." Phase 91 added the per-hit
`judgment: Option<RecallJudgment>` field on `RecallHit`
but explicitly scoped the change as **v1 augment, not
replace** — the field was recorded into the audit chain
and the Phase 78 insights surface, but no runtime
accumulator consumed it. Phase 93 wires the consumer:
`correlate_detailed` — the single source of truth that
drives memory promotion (Phase 77 Task 6) and Persona
proposal filing (Phase 77 Task 7) — now reads the per-hit
verdict where present and falls back to the existing
turn-level structural proxy where absent.

- **Augment, not replace (Q1a):** per-hit `Some(judgment)`
  overrides the turn-level structural signal for that hit;
  `None` hits keep using the structural proxy. Smooth
  migration — judgments take effect incrementally as the
  Phase 91 cron processes hits; the loop never loses
  signal while the cron catches up. Matches the Phase 91
  field doc's own framing.
- **Symmetric ±WEIGHT mapping (Q2a):**
  `Used → +WEIGHT`, `Hurt → -WEIGHT`,
  `Irrelevant → 0` (dead weight; neither rewards nor
  punishes). Same magnitude as the existing structural
  mapping — single source of magnitude.
- **Consumer-side knob (Q3a):** new
  `[recall_feedback].use_judgment_signal: bool`, default
  `false`. Matches the Phase 87 / 88 / 91 / 92 actuator
  opt-in pattern. The knob lives on the consumer
  (`[recall_feedback]`), separate from the Phase 91
  producer-side `[recall_judgment]`, so the two configs
  remain independently reason-aboutable. Turning the
  judge on alone keeps Phase 91 in visibility-only mode
  (the v1 default posture); turning both on closes the
  self-improving loop end-to-end.
- **Single-fixture integration test (Q4a):** one recall
  with three hits — `Used` / `Hurt` / un-judged — on a
  +WEIGHT structural turn. With the knob on: Used
  promoted, Hurt NOT promoted (judgment overrides
  positive structural), un-judged promoted via fallback.

Streak all three correct: DESIGN.md → **40**, PRODUCT.md
→ **33**, `aivyx-core/src/lib.rs` → **41** (new project
record, beats Phase 92's 40) — augmentation in
`aivyx-channel`; config knob in `aivyx-config`; the
`HelpfulnessTally` shape unchanged so downstream
actuators are byte-identical. Test count delta **`+11`**
workspace (`+4` config knob, `+5` augmentation +
judgment-signal mapping, `+1` reflection-cron integration,
`+1` surface banner) — one over the predicted `+6-10`
band, accounted for by the wire-compat surface field
earning its own test alongside the field plumbing. Zero
clippy warnings. Zero new workspace deps.

Mid-phase scope correction: the open commit
(`9f875c6`) scoped Phase 93 around calibrating a per-
domain `min_score` recall-gate threshold. On reading the
codebase that knob doesn't exist — Phase 90's gate is a
single global `recall_gate_min_chars` input-length check,
not a per-domain post-recall score filter. The
re-scope commit (`27d8c41`) pivoted to the loop closure
that actually exists in the codebase (the Phase 91
deferral). Phase ritual followed: new Q-block resolved
pre-Task 2.

Likely follow-ups (per-domain or per-topic weights for
the verdict→signal mapping; "replace" mode that drops
the structural fallback; asymmetric Hurt penalty; sum
mode that stacks both signals — see PHASE_93.md
deferrals list) are operator-feedback-gated.

## Phase 94 — Web UI Grouping for Linked Supersession Proposals (Phase 92's first deferral, closed)

**Frozen — see [PHASE_94.md](PHASE_94.md).** Closes the
Phase 92 deferral that headlined that phase's "likely
follow-ups" list: *"Web UI visual grouping of linked
supersession proposals."* Phase 92 shipped the structured
`supersedes_proposal_id` linkage on `ProposedPersonaDelta`
but rendered it only via `reason` text on each half.
Phase 94 makes the linkage operator-visible at a glance
in both surfaces: the `aivyx persona proposals` CLI shows
linked pairs with `└─ supersedes:` indicators; the Web UI
Proposals tab renders the pair as one card with a primary
"Approve both" action plus a `⋮ Split` menu for partial
actions.

- **Pure structural pass (Q1a):** new
  `group_supersession_pairs` helper in `aivyx-channel`
  consumes the flat `Vec<PersonaProposal>` the IPC already
  returns and produces `Vec<ProposalRendering>`. No IPC
  contract change for the grouping itself (the
  `supersedes_proposal_id` field was added to
  `PersonaProposalSummary` with full wire-compat via
  `#[serde(default, skip_serializing_if = "Option::is_none")]`).
  Both surfaces call the same algorithm — Rust helper on
  the CLI side, line-for-line JS port on the Web UI side.
- **Generic via small trait (`GroupableProposal`):** the
  helper takes any type implementing `id()`,
  `supersedes_proposal_id()`, and `op_kind()` accessors;
  impls exist for both the typed `PersonaProposal`
  (daemon-side use) and the wire `PersonaProposalSummary`
  (CLI use). One source of truth, no algorithm
  duplication.
- **Defensive degradation:** self-reference, dangling
  partner id, asymmetric link (A→B without B→A), and
  same-op pair (two `AppendList`s or two `RemoveList`s)
  all degrade to standalone rendering rather than panic.
  The grouping invariant requires exactly one
  `RemoveList` + one `AppendList` with mutual
  cross-reference.
- **One-click ergonomics + Phase 92 guarantee preserved
  (Q2a):** the Web UI's primary action fires two
  sequential `ResolvePersonaProposal` IPC calls
  (RemoveList first, then AppendList). Phase 92's
  explicit `each half independently Revert-able`
  guarantee covers the half-approved-on-failure case
  without needing a transactional chain primitive; the
  `⋮ Split` menu exposes the partial-action paths the
  operator may need.

Streak all three correct: DESIGN.md → **41**, PRODUCT.md
→ **34**, `aivyx-core/src/lib.rs` → **42** (new project
record, beats Phase 93's 41) — grouping helper +
generic trait + CLI render in `aivyx-channel`; Web UI
changes in the embedded static HTML asset; IPC field
added with full wire-compat. Test count delta `+11`
workspace (`+8` pure helper covering Q4a mixed fixture
plus six dedicated edge cases, `+3` CLI rendering;
the Web UI JS is a port of the Rust helper covered by
those tests + manual browser testing). **One over the
predicted `+6-10` band** — accounted for by the helper
earning extra edge-case coverage (self-ref / asymmetric /
same-op / dangling / position-determinism / empty all
worth a test apiece). Zero clippy warnings. Zero new
workspace deps.

Likely follow-ups (atomic
`ResolveSupersessionGroup` IPC primitive that wraps the
two-call dance transactionally; backend-side grouping
enrichment via a `linked_with: Vec<String>` field on
`ProposalSummary`; drag-to-merge / drag-to-split UI
affordances; n-ary group rendering once Phase 83's n-ary
cluster supersession deferral closes — see PHASE_94.md
deferrals list) are operator-feedback-gated.

## Phase 95 — Reflection Cadence Learning (Skip-When-Idle) (Phase 71's reflection-cadence deferral, closed)

**Frozen — see [PHASE_95.md](PHASE_95.md).** Closes the
Phase 71 deferral that's been carried 24 phases:
*"reflection cadence learning."* The reflection cron has
been firing unconditionally on its `cron` schedule since
Phase 71, paying LLM cost for Phase 87 phrasing + Phase 91
judgment + Phase 92 supersession passes even on idle days
where there's nothing actionable to find. Phase 95 closes
the deferral with the simplest leverage shape: **skip-when-
idle**. The scheduler reads audit-chain growth since the
last *fired* cycle for that schedule; if growth is below
the operator-configured threshold AND `skip_when_idle =
true`, the cycle is skipped entirely (no LLM calls; just a
log line + a counter bump). The operator's cron remains
the **upper bound** on firing rate — cadence learning is
monotonic-slower-only, never faster.

- **Audit-growth signal (Q1a):** the scheduler reads
  `audit_log.len()` delta since the last fired cycle for
  the same schedule. Simple, deterministic, no LLM cost,
  no new substrate. The first cycle after a daemon boot
  fires unconditionally (no prior baseline to compare).
- **Skip-when-idle binary (Q2a):** either fire or skip;
  monotonic-slower-only. The boundary uses `>=` so
  `min_audit_entries_to_fire = 1` means "any new entry
  fires," not "any entry beyond the first."
- **Per-schedule knob (Q3a):** two new optional fields on
  the existing `[[reflection_schedule]]` block.
  `skip_when_idle: bool` (default `false`) +
  `min_audit_entries_to_fire: u32` (default `1`). Wire-
  compat via the established `Option<T>` +
  `#[serde(default)]` pattern. Different schedules may
  carry different idleness tolerances — a daily housekeeping
  schedule wants a high threshold; an hourly responsive
  schedule wants a low one.
- **Observable (Q4a):** daemon log on skip
  (`aivyx reflection: schedule "X" — skipped (audit-growth
  K below threshold M)`) + `aivyx learning` surface
  block (`Reflection cadence (Phase 95):` with per-
  schedule `K fired, S skipped` counts). Real-time + aggregate.

Streak all three correct: DESIGN.md → **42**, PRODUCT.md →
**35**, `aivyx-core/src/lib.rs` → **43** (new project
record, beats Phase 94's 42) — helper + state + integration
all in `aivyx-channel/src/reflection_scheduler.rs`; config
knobs in `aivyx-config`; surface in
`bin/aivyx_modules/learning.rs`; IPC field added with full
wire-compat. Test count delta `+12` workspace (`+3` config
knobs covering defaults / explicit values / staged
posture / zero-threshold rejection, `+7` helper covering
boundary + defended-zero + stat default + serde wire-
compat, `+1` multi-cycle integration over the helpers, `+1`
surface rendering with empty / all-zero / mixed cases) —
**two over** the predicted `+6-10` band, accounted for by
the helper + stat earning more individual unit tests than
the calibration law anticipated for "new pure module with
multiple boundary cases." Zero clippy warnings (one
`large_enum_variant` allow added to `QueryResponsePayload`
since the addition tipped a long-running additive-fields
variant past clippy's threshold). Zero new workspace deps.

Likely follow-ups (backoff-multiplier mode; adaptive
interval; time-of-day pattern learning; persisted cadence
stat across daemon restarts; LLM-based signal-density
classifier; per-pass skip granularity — see PHASE_95.md
deferrals list) are operator-feedback-gated.

## Phase 96 — ANN Index for Semantic Memory Search (Phase 75's ANN-index deferral, closed)

**Frozen — see [PHASE_96.md](PHASE_96.md).** Closes the
Phase 75 deferral that's been carried 20 phases: the
approximate-nearest-neighbor index for semantic memory
search. The brute-force `rank_by_cosine` over the full
`vector_index: Vec<(String, u64, Vec<f32>)>` has been
the only path since Phase 75; it scales linearly with
memory size. Phase 96 adds opt-in **IVF-style
clustering** that scales `O(N) → O(√N)` at query time.

- **Hand-rolled IVF (Q1a):** vectors partition into
  `K ≈ √N` clusters at build time via deterministic
  spaced-sampling seeds + one-pass nearest-centroid
  assignment. ~150 lines in
  `aivyx-memory::ann_index`. Preserves the project's
  zero-new-deps streak.
- **In-memory + rebuild-on-demand (Q2a):** the
  `AnnIndex` lives in a `tokio::sync::Mutex<Option<...>>`
  alongside the flat `vector_index`; rebuilt on the first
  ANN query after boot AND when the
  `writes_since_ann_build: AtomicU32` counter crosses
  the operator-configured threshold. No new schema, no
  serialization, no incremental-update complexity.
- **ANN narrows → brute-force re-ranks (Q3a):** the ANN
  returns a candidate pool of `4 * limit` candidates
  searched from `default_top_clusters(K) = max(2, K/4)`
  clusters; the existing brute-force ordering rule then
  re-ranks within and returns the top-K. The exact-
  cosine guarantee on the final ordering is preserved
  within the candidate set.
- **Two consumer-side knobs (Q4a):**
  `[embedding].ann_index: bool` (default `false`) +
  `ann_rebuild_threshold: u32` (default `100`).
  Validation: `ann_rebuild_threshold >= 1` when armed
  (zero would force a rebuild every recall). With the
  knob off, recall is byte-identical to pre-Phase-96.

Streak all three correct: DESIGN.md → **43**, PRODUCT.md
→ **36**, `aivyx-core/src/lib.rs` → **44** (new project
record, beats Phase 95's 43) — the ANN data structure +
build + query + RedbMemory integration all in
`aivyx-memory`; the dispatch in
`aivyx-channel::memory_recall`; the config knobs in
`aivyx-config`. No `aivyx-core` touch; no new
`AuditTag`; no new `KeyDomain`. Test count delta `+22`
workspace (`+4` config knobs, `+15` pure ANN module
with comprehensive boundary coverage, `+3` RedbMemory
integration — small-N brute-equivalence, large-N
top-1 recall, stale-counter increment/reset) —
**squarely inside the predicted +15-25 band**. Zero
clippy warnings. Zero new workspace deps.

Likely follow-ups (HNSW-quality recall for very large
stores; iterative k-means refinement; persisted index
across daemon restarts; incremental updates; topic-aware
centroid seeding; ANN for the `aivyx memory search`
operator path — see PHASE_96.md deferrals list) are
operator-feedback-gated.

## Phase 97 — Token-Budget Context Sizing (Phase 76's + Phase 86's longest-running content deferral, closed)

**Frozen — see [PHASE_97.md](PHASE_97.md).** Closes the
twice-deferred token-budget item carried 21 phases
(Phase 76) and 11 phases (Phase 86). Auto-recall,
adaptive Persona selection, and the conversational
window have all capped injection by **entry count** —
a proxy for token cost, not the cost itself. A single
4 KB memory body silently displaced multiple shorter
ones from the same `rag_top_k` budget; a grown Persona
facet ate turn after turn of input. Phase 97 adds an
opt-in token budget that caps both recall + Persona
injection paths AFTER their existing rank-and-filter
steps: the lowest-ranked items drop until the running
estimate fits.

- **Hand-rolled `chars/4` estimator (Q1a):** preserves
  the project's zero-new-deps streak; ~20 lines of code
  in `aivyx-channel::token_budget`. Accuracy ~±20% for
  English — adequate for budget enforcement at any
  realistic operator threshold. Unicode `chars()`-
  counted, not bytes.
- **Combined recall + Persona scope (Q2a):** one knob
  caps both. The existing `rag_top_k` and Persona
  K-facet caps become soft hints; the token budget is
  the hard cap. One coherent operator-facing number.
- **Drop-lowest-ranked eviction (Q3a):** the first item
  whose addition would exceed the budget AND every item
  after it are dropped. No mid-item truncation. The
  caller pre-ranked, so the dropped tail is by
  construction the lowest-priority subset.
- **Single opt-in knob (Q4a):** new
  `[embedding].recall_token_budget: u32` (default `0`
  = disabled). Matches the established Phase 87 / 88 /
  91 / 92 / 93 / 95 / 96 actuator opt-in pattern. With
  the knob at `0`, the recall + Persona paths are
  byte-identical to pre-Phase-97.

The protected Persona core (behavioral constraints +
identity scalars) is ALWAYS present regardless of
budget — the budget only trims soft-facet selection.
The recall breadcrumb + Phase 78 learning surface +
Phase 84 cluster stat + Phase 77 recall_log all see
the post-budget set so observers match what's actually
injected. Edge case: if every recall hit falls out of
the budget, auto-recall returns no block (planner falls
back to the base prompt).

Streak all three correct: DESIGN.md → **44**, PRODUCT.md
→ **37**, `aivyx-core/src/lib.rs` → **45** (new project
record, beats Phase 96's 44) — `token_budget` module +
recall integration + Persona integration all in
`aivyx-channel`; config knob in `aivyx-config`. Test
count delta `+21` workspace (`+3` config knob, `+13`
pure module with comprehensive coverage of both
`estimate_tokens` and `apply_token_budget` boundary
cases, `+3` recall integration including the "every hit
fell out → None" path, `+2` Persona integration
including the "protected core always present" pin) —
**over** the predicted `+10-15` band, accounted for by
the pure helper module earning ~12 individual tests
instead of the calibration law's expected 5-7. Zero
clippy warnings (one trivial `into_iter` cleanup). Zero
new workspace deps.

Likely follow-ups (exact-tokenizer integration; per-
category budgets; auto-derive from model context window;
conversational-window budget; mid-item truncation
strategy; surface line for dropped-by-budget count —
see PHASE_97.md deferrals list) are operator-feedback-
gated.

## Phase 98 — Hybrid Keyword+Semantic Recall Fusion (Phase 75's hybrid-fusion deferral, closed)

**Frozen — see [PHASE_98.md](PHASE_98.md).** Closes
Phase 75's 23-phase-old hybrid-fusion deferral. The
recall pipeline has ranked by cosine similarity over
embeddings since Phase 75 — strong on semantic
relationships but weak on rare-term recall (acronyms,
proper nouns, code identifiers, project codenames). The
keyword search tool (Phase 74) handles those exact-
match cases but operates as a separate manual path.

Phase 98 fuses the two paths via **Reciprocal Rank
Fusion (RRF)**. With `[embedding].recall_hybrid = true`,
auto-recall runs both the semantic ranker AND the
existing `Memory::search` substring search at recall
time; the two rankings fuse via RRF before feeding the
downstream pipeline (cluster expansion, token budget,
etc.).

- **Reciprocal Rank Fusion (Q1a):** industry-standard
  fusion. `score = Σ 1 / (k + rank + 1)` with `k = 60`
  (Cormack et al.'s standard value). Rank-based —
  cosine scores and substring hit counts don't need
  normalization because RRF doesn't read scores. ~30
  lines of pure code in
  `aivyx-channel::recall_fusion`.
- **Reuse Phase 74's substring search (Q2a):**
  `Memory::search` is what the operator-facing search
  already uses. No new tokenization; no corpus stats;
  no BM25-style state to maintain. RRF is rank-based,
  so the simple substring path gives RRF the ranks it
  needs.
- **Same query text both sides (Q3a):** Phase 86's
  conversational window (if engaged) or the bare
  current message — whatever the semantic side
  embeds. Single source of truth; consistent ranking
  targets.
- **Single opt-in knob (Q4a):**
  `[embedding].recall_hybrid: bool` (default `false`).
  Matches the established actuator opt-in pattern.

Trade-off documented: the `rag_min_similarity` floor is
**skipped on the hybrid path** because RRF scores
aren't on the cosine scale. The `rag_top_k` cap still
limits the fused output; items only one ranker surfaces
get small RRF scores that get pushed out by stronger
items. A separate `rag_hybrid_min_rrf` knob is a
documented deferral.

Streak all three correct: DESIGN.md → **45**, PRODUCT.md
→ **38**, `aivyx-core/src/lib.rs` → **46** (new project
record, beats Phase 97's 45) — `recall_fusion` module +
recall integration all in `aivyx-channel`; config knob
in `aivyx-config`. Test count delta `+16` workspace
(`+3` config knob, `+10` pure RRF helper with the full
boundary set including the core "item in both rankings
outranks single-ranker top hits" claim, `+3` recall
integration including the rare-term case the deferral
named) — slightly over the predicted `+10-15` band,
accounted for by the RRF module's comprehensive
boundary coverage. Zero clippy warnings (two trivial
cleanups). Zero new workspace deps.

Likely follow-ups (rag_hybrid_min_rrf floor knob; BM25-
style keyword scoring; tokenization-aware substring
matching; operator-tunable recall_hybrid_k; surface
line for fused stats; shared embedding cache between
rankers — see PHASE_98.md deferrals list) are
operator-feedback-gated.

## Phase 99 — Local Testing Setup (operator-requested)

**Frozen — see [PHASE_99.md](PHASE_99.md).** The first
operator-feedback infrastructure phase. Ninety-nine phases
of substrate shipped with the test pyramid resting entirely
on `cargo test`; what the project never had was a one-command
way to build the `aivyx` binary and drive the real agent
stack against a real LLM backend locally. Phase 99 built
that loop: a `scripts/dev-run.sh` launcher (interactive
session against a fully local Ollama backend, all state under
a gitignored `.dev-run/`) and a `scripts/dev-verify.sh`
scripted verification pass (audit chain, daemon lifecycle,
memory/fs tool probes — the tool paths an interactive chat
test cannot cover). Shell tooling only — no crate touched,
no workspace test added (`cargo test` stays at 1746); all
three contract streaks held byte-identical: DESIGN.md → 46,
PRODUCT.md → 39, `aivyx-core/src/lib.rs` → 47 (new project
record, beats Phase 98's 46). Opened under an explicit
**local-build posture**: builds stay local while repo
infrastructure (CI, remote runners, publication) is still
being decided — that work belongs to the Distribution
milestone, not here. Phase 99 is the prerequisite for the
Channel Activation Milestone, which consumes this harness
for operator verification across all channels.

## Chapter B — Tooling (Phases 100+)

Phase 99 established the local operator loop; the
`dev-verify.sh` pass it shipped immediately surfaced the
agent's **tool surface** as the next area worth focused
work — both the breadth of what the agent can do and the
reliability with which it does it. Chapter B is the tooling
arc. It expands the first-party tool surface, then hardens
the substrate around it: how reliably models invoke tools,
how the operator observes tool usage, and how third parties
author new tools. Each item lands as its own focused phase
in the small-scope, Q-block-signed-off rhythm Chapter A
established. The chapter opens with the most concrete gap —
capability scopes declared in `aivyx-capability` that have
no tool behind them.

**Expected phases:**

- **Phase 100 — Tool-Surface Gap Closure.** Shipped — see
  below and [PHASE_100.md](PHASE_100.md).
- **Phase 101 — Tool-Call Input Validation & Repair.**
  Shipped — see below and [PHASE_101.md](PHASE_101.md). The
  reliability item: the planner validates a known tool's
  call input against the tool's schema before dispatch and
  loops the model to repair a malformed call.
- **Phase 102 — Tool Observability (`aivyx tools`).**
  Shipped — see below and [PHASE_102.md](PHASE_102.md). A
  read-only subcommand that lists every registered tool
  and annotates each with audit-derived call/outcome
  stats, with a `--window` filter.
- **Phase 103 — External Tool Ergonomics (`aivyx tool init`).**
  Shipped — see below and [PHASE_103.md](PHASE_103.md). The
  closing Chapter B item: an `aivyx tool init <path>`
  subcommand that scaffolds a runnable Rust tool-process
  starter (Cargo.toml, src/main.rs handshake + invocation
  loop, README, conformance test, `[[tool_process]]`
  snippet) so a third-party tool author edits one function
  body rather than copying `examples/python-tool/` and
  porting it to Rust by hand.

**Chapter B is complete.** All four expected phases shipped
(100, 101, 102, 103); subsequent tool-layer work is
operator-feedback-gated, in the project's established
post-Chapter posture. The next chapter — **Chapter C —
Operator Onboarding** — opens at Phase 104.

## Chapter C — Operator Onboarding (Phases 104+)

Chapter B closed the tool layer in good shape; the next axis
of work is the *fresh operator's* experience. From the
moment they decide to try Aivyx to the moment their first
turn returns a useful answer, every step is operator-
visible and every paper-cut compounds. Chapter C is the
arc that closes those paper-cuts: init wizard polish,
provider-default refresh, default-model recommendations,
docs landing, and — when the held public-hosting decision
resolves — the Phase 61 `v0.1.0` publication that turns
"build from source" into "install the binary." Each item
lands as its own focused phase in the small-scope,
Q-block-signed-off rhythm Chapters A and B established.
The chapter opens with the first thing every new operator
touches: the `aivyx init` wizard itself.

**Expected phases (subject to revision at each exit):**

- **Phase 104 — `aivyx init` Polish.** Shipped — see
  below and [PHASE_104.md](PHASE_104.md). Refreshed
  stale provider defaults, named a concrete starter
  model on the empty-Ollama path, and added a verify-
  before-write step against `GET /v1/models` so a broken
  config never lands on disk.
- **Phase 105+ candidates** at chapter open were docs
  landing rewrite, the held Phase 61 `v0.1.0` publication
  once the hosting decision lands, and operator-feedback
  follow-ons from Phase 104. The post-Phase-104 review
  surfaced a separate axis — a comparison pass against
  the Hermes Agent (Nous Research) named five concrete
  out-of-the-box surface gaps (channel breadth, tool
  breadth, MCP server breadth, skills auto-creation,
  trajectory logging) that warranted their own arc.
  Those five land as **Chapter D — Substrate Breadth**
  below. Chapter C's onboarding remit (docs landing,
  `v0.1.0` publication) stays paused, not retired —
  whichever thread surfaces first opens the next
  Chapter C phase.

**Chapter C status — open with one phase shipped.**
Phase 104 closed the wizard-side first-touch paper-cuts;
the docs-landing and `v0.1.0`-publication threads named
at chapter open are paused. **Chapter D — Substrate
Breadth** takes the immediate post-Phase-104 sequence.

## Chapter D — Substrate Breadth (Phases 105–110+)

Phase 104 closed with a comparison pass between Aivyx and
the **Hermes Agent** (Nous Research, MIT, Python — the
closest public reference for a personal-AI-agent
substrate). The comparison found the Aivyx substrate sound
but its *out-of-the-box surface* narrower than the
reference along five axes: a smaller channel-adapter set
(Local / Telegram / Web UI vs. Hermes's six), a smaller
first-party tool count (ten vs. forty-plus), a single
bundled MCP server (the Phase 46 `web-search` vs. an
opt-in matrix), no agent-drafted skill substrate, and no
trajectory-export path for the audit chain even though
every event is already structured for it. Chapter D is the
arc that closes those gaps. The five items land
easy-wins-first so the substrate-design items (Amendment
A12 for the tool count, reflection extension for skills)
arrive after the lighter items build momentum and real
operator pressure tightens the exact scope.

**Expected phases (subject to revision at each exit):**

- **Phase 105 — Trajectory Logging (`aivyx audit export`).**
  Shipped — see below and [PHASE_105.md](PHASE_105.md).
  Lowest-risk Chapter D item. The HMAC audit chain already
  carried the structured per-turn / per-tool-call rows a
  trajectory exporter needs; Phase 105 wired
  `PersistentAuditLog::entries_range` into a JSONL emitter
  with `--from <seq>` and `--limit <N>` filters. All three
  byte-identity streaks held → DESIGN.md 52, PRODUCT.md 5,
  `aivyx-core/src/lib.rs` 5.

- **Phase 106 — MCP Server Breadth (Curated Recipes).**
  Shipped — see below and [PHASE_106.md](PHASE_106.md). Q1a
  at sign-off chose **recipes-only** scope: zero new
  bundled-server code paths. Shipped `docs/MCP_RECIPES.md`
  cataloguing 12 well-supported MCP servers (filesystem,
  github, gitlab, sqlite, postgres, time, fetch,
  brave-search, slack, memory, puppeteer, everything) with
  paste-able `[[mcp_server]]` + inline `[mcp_server.sandbox]`
  blocks per Q3a + env vars + capability-scope notes. Plus a
  new `aivyx mcp recipes [<name>]` CLI surface per Q2a. All
  three byte-identity streaks held → DESIGN.md 53,
  PRODUCT.md 6, `aivyx-core/src/lib.rs` 6. New bundled-
  server code stays a deferral pending operator pressure.

- **Phase 107 — Discord Channel Adapter (`aivyx-discord`).**
  Shipped — see below and [PHASE_107.md](PHASE_107.md). Full
  parity with `aivyx-telegram` at the in-process layer.
  `aivyx --channel discord` runs an end-to-end Discord bot
  against twilight-rs (twilight-gateway + twilight-http +
  twilight-model). Two streak-prediction surprises in the
  operator's favor: `aivyx-core/src/lib.rs` held
  byte-identical (Phase 8 forward-enumerated
  `ChannelPlatform::Discord`); DESIGN.md held (A4 addendum
  in the amendment file per Phase 49 precedent). Test count
  delta `+29` (workspace `1859 → 1888`), below the predicted
  `+50`–`+100` because Discord's push-based Gateway
  simplified the substrate the scripted suite covers. Two
  Phase-107-internal deferrals named honestly: Discord
  daemon-frontend (`FrontendType::Discord` +
  `discord_daemon_frontend.rs` mirroring Phase 19) and
  `/approve` / `/reject` text-command gate-resolve routing
  — both land alongside in a focused follow-on.

- **Phase 108 — Slack Channel Adapter (`aivyx-slack`).**
  Shipped — see below and [PHASE_108.md](PHASE_108.md).
  Fourth in-tree channel adapter, four-data-point
  confirmation for `docs/ADAPTER_PATTERN.md` (was
  confirmed-at-three at Phase 107). slack-morphism 2.22 SDK,
  Socket Mode only, `(team_id, channel_id)` colon-joined
  partition key. **All three byte-identity streak
  predictions held** — `ChannelPlatform::Slack` was
  already in `aivyx-core` since Phase 8 (same Phase 107
  surprise pattern). Zero-new-deps broke as predicted at
  the slack-morphism adoption. Workspace tests `+25` →
  1913 (inside the predicted `+20`–`+30` band). Two
  Phase-108-internal deferrals bundled with the Phase 107
  daemon-frontend follow-on: production
  `SlackMorphismTransport` callback-state-passing wiring
  (slack-morphism's `SlackClientEventsUserState` API
  surface needs proper UserState-backed design), and
  `/approve` / `/reject` gate-resolve routing.

- **Phase 109 — Tool Breadth (Amendment A12).** Shipped —
  see below and [PHASE_109.md](PHASE_109.md). Q1a chose
  **`git.status` + `git.diff`** sharing one new `git.read`
  scope base (qualified by repo path) as the headline two
  tools. Q2a also closed **`net.dns`** as a bonus closure
  of one of Phase 100's eight audit-deferred toolless
  scopes. A12 takes P10 from 10 → 13 substrate tools.
  All three byte-identity streaks reset together — DESIGN.md
  + PRODUCT.md broke at A12 as predicted; **`aivyx-core/src/lib.rs`
  broke too, against the open-doc prediction** (tools
  live in aivyx-core/src/tools/ per the Phase 4 substrate
  convention, not aivyx-channel; honest break per the
  Phase 6 Q5 convention). First triple-reset since Phase
  56's P13/P14 amendment double-break. Zero new workspace
  deps. Workspace tests `+25` → 1938 (inside the predicted
  `+15`–`+25` band at the upper edge).

- **Phase 110 — Skills Auto-Creation (Reflection Staging).**
  Shipped — see below and [PHASE_110.md](PHASE_110.md). The
  last Chapter D item — closes the sixth and final
  Hermes-comparison gap. LearnedSkill as the 11th
  PersonaDeltaCategory variant reusing the entire Phase
  59/60/70 Persona substrate; three new scope bases
  (skills.propose, skills.list, skills.invoke) in
  CEILING_TRUSTED; two substrate tools (skills.list +
  skills.invoke) gated by their own bases; `## Learned
  skills` system-prompt section between Persona and active
  role. **Second consecutive triple-streak-reset** —
  DESIGN.md broke at the new D4 skills section; PRODUCT.md
  broke against the prediction (P8 delivery-status refresh);
  `aivyx-core/src/lib.rs` broke at the new tool re-exports.
  Workspace tests +12 → 1950 (below the predicted +25-+40
  band; substrate-extension work was largely covered by the
  existing profile_prompt / persona test surface). Zero new
  workspace deps. Agent-side auto-proposer heuristic deferred
  to a follow-on phase.

Chapter D ends when each item has shipped or been
explicitly retired. The post-110 ledger picks up Chapter
C's paused threads (docs landing, `v0.1.0` publication)
or opens against operator feedback as it arises. Phase
ordering inside Chapter D is "easy wins first" by design;
inversion at any phase exit costs one ROADMAP commit, not
an amendment.

## Phase 111 — Adapter Production Wiring (operator-requested follow-on)

**Frozen — see [PHASE_111.md](PHASE_111.md).** Standalone
phase past Chapter D's close (no chapter framing, matching
the Phase 99 precedent for operator-requested follow-on
work). Landed the two Phase-107/108-internal carve-outs:
- **Discord daemon-frontend** (Phase 107 Task 5 carve-out)
  shipped mirroring Phase 19's Telegram-over-daemon pattern.
- **Slack Socket Mode live wiring** (Phase 108 Task 3
  carve-out) shipped replacing the stub
  `SlackMorphismTransport` with the real implementation
  via `SlackClientEventsUserState` callback-state-passing.

After Phase 111, all five adapters (Local, Telegram, Web
UI, Discord, Slack) are production-ready end-to-end at the
in-process AND daemon-mode levels. The **Channel
Activation Milestone** is now unblocked and runnable as
the operator-verification pass it was always meant to be —
no code, just real-bot smoke tests across every adapter.

**Exit outcomes:**
- All three streak predictions held — first phase since
  Phase 56/108 with every prediction correct and every
  streak extending (DESIGN.md, PRODUCT.md,
  aivyx-core/src/lib.rs all reach streak=2).
- Q2a shared-substrate question answered affirmatively at
  three data points: extracted `gate_command::parse` into
  `aivyx-channel/src/gate_command.rs`; broader pump-shape
  extraction deferred until a fourth adapter forces it.
- Test count delta `+14` (1950 → 1964), below the `+25`
  to `+40` prediction — honest break per Phase 6 Q5
  attributable to scripted-only test posture (Q3a)
  collapsing per-adapter integration tests into a single
  cross-renderer parity assertion.
- Zero new workspace deps; `http = "1"` direct-promoted in
  `aivyx-slack` for callback signature legibility but it
  was already transitive.

## Chapter E — Self-Improvement Loop Deepening (Phases 114+)

After Chapter D closed the Hermes-comparison gaps and
Phase 111-113 cleared the operator-surface deferral
ledger, Chapter E opens against the project-vision
critical path: deepening the **self-improvement loop**
beyond the skill-specific framework Phase 112 shipped.
Phase 112's auto-proposer drafts new skills from complex
turns; Chapter E generalizes the loop to the full
self-improvement surface — every PersonaDeltaCategory,
tool-selection learning, recall-quality self-tuning,
self-correction on failed turns.

Chapter ordering follows the "easy wins first" discipline
established for Chapter D: the most natural extension of
the freshly-warm Phase 112 substrate ships first. The
operator picks subsequent direction at each phase exit
based on observed value vs the next-axis options.

**Expected phases (subject to revision at each exit):**

- **Phase 114 — Persona Auto-Proposer Generalization.**
  Active — see below and [PHASE_114.md](PHASE_114.md).
  Extends the Phase 112 auto-proposer from `LearnedSkill`
  to every PersonaDeltaCategory variant. Per-category
  TOML config (operator-picked over uniform-single); same
  four-signal heuristic gate; reuses existing scope bases
  (no KNOWN_BASES growth).

Subsequent Chapter E phases will be picked from the
remaining self-improvement axes (tool/skill selection
learning from outcomes; self-correction loop on failed
turns; outcome-driven Profile/Role refinement) based on
operator pressure and observed value from Phase 114.

## Phase 119 — Phase 118 Apply-Side Closeout + Tool-Relevance Dump (Operator-Value Polish)

**Active — see [PHASE_119.md](PHASE_119.md).** Audit-
informed phase. Closes the manual-edit gap left by
Phase 118 and the deferred `aivyx tool-relevance dump`
CLI from Phase 116. Three new operator-side CLI
commands ship together:
- `aivyx profile apply-hint <id>` — applies an
  approved `ProfileHint` to `aivyx.toml`'s `[profile]`
  section atomically.
- `aivyx role import <id>` — adds a new
  `[roles.<name>]` section from an approved
  `RoleDefinitionSuggestion` (with parent inheritance
  honored; refuses overwrite without `--force`).
- `aivyx tool-relevance dump` — renders the encrypted
  Phase 116 relevance ledger as a human-readable table
  per keyword-key.

**Q-block (all Recommended):**
- Q1a — Both apply-helpers + tool-relevance dump
  (Recommended). Three CLI commands ship in one phase.
- Q2a — Separate approve + apply commands
  (Recommended). The existing approve command stays
  category-agnostic; the new apply commands are
  independent operator gestures.
- Q3a — New `ProfileHintApplied` + `RoleDraftImported`
  audit-event variants (Recommended). Wire-compat
  serde defaults; the audit chain records the
  operator's act-on-approval gesture distinctly from
  the approval itself.

**Streak predictions:** DESIGN.md HOLD → 10; PRODUCT.md
HOLD → 10; `aivyx-core/src/lib.rs` HOLD → 3 (honest
70/30; if daemon-side wiring needs new `AuditTag`
variant for the bridge, the 30% case fires).

Test count: predicted `+25 to +45`. Zero new workspace
deps anticipated (TBD on `toml_edit` crate presence).

After Phase 119, every named operator-value deferral
from Phases 112-118 is closed. The Channel Activation
Milestone becomes the highest-information-value
direction for Phase 120.

## Phase 118 — Outcome-Driven Profile/Role Refinement (Chapter E #4 — closer)

**Frozen — see [PHASE_118.md](PHASE_118.md).** The last
named Chapter E axis. Adds `ProfileHint` and
`RoleDefinitionSuggestion` variants to
`PersonaDeltaCategory`, extends the Phase 114 auto-
proposer pipeline (heuristic + LLM-judge + routing) to
fire for both, forces always-staged routing for the two
new categories regardless of confidence, and surfaces
approved drafts through the existing `aivyx persona
proposals` CLI for operator copy into `aivyx.toml`.

**Q-block (one non-Recommended):**
- Q1c — **Both Profile attributes AND new Role
  definitions** (non-Recommended; picked over the
  Profile-attribute-only Recommended). Larger surface;
  closes Chapter E #4 in one phase.
- Q2a — Always-staged for operator approval
  (Recommended). Preserves P13 (Profile-operator-owned)
  and P9 (Role-config operator-curated). Hard-coded
  routing override, not operator policy.
- Q3a — Extend Phase 114 auto-proposer with new
  categories (Recommended). Maximum substrate reuse.

**Streak outcomes:** all three predictions correct.
- DESIGN.md: HELD as predicted. Streak → 9.
- PRODUCT.md: HELD as predicted. Streak → 9.
- `aivyx-core/src/lib.rs`: HELD as predicted (60/40 hold
  case fired). No new AuditTag needed; new draft types
  fit inside existing `pub mod skill_proposer`
  boundary. Streak resets 1 → 2.

Test count: 2188 → 2248 (+60). **Above** the predicted
`+25 to +50` band — honest scope reporting (Phase 6 Q5):
Q1c's broader scope (both Profile attributes AND Role
suggestions) plus Task 5's per-category integration
(load-bearing across both runtime and aivyx-config TOML
surfaces) produced more test surface than anticipated
at sign-off. Each test pins one concrete behavior.

Zero new workspace deps. Zero clippy warnings.

**Chapter E closes with Phase 118.** All four named
axes shipped (Phases 114-118). Post-Phase-118 deferral
ledger is empty. The next phase opens against the
Channel Activation Milestone OR a new thematic Chapter F
shaped by operator pressure.

## Phase 117 — Phase 116 Deferral Closeout (live-prompt pipe + per-skill tracking)

**Frozen — see [PHASE_117.md](PHASE_117.md).** Standalone
phase that closes both Phase-116-internal deferrals
together per Q1b sign-off. After Phase 117, the Phase 116
relevance substrate reaches the LLM in live turns AND
records per-skill outcomes accurately — the operator value
Phase 116 aimed at lands in full.

**Q-block (one non-Recommended):**
- Q1b — **Bundle both deferrals together** (non-
  Recommended; picked over the focused single-deferral
  option). Larger scope; cleaner ledger + prompt surface
  immediately.

Phase 113's Operator-Surface Polish precedent: batching
small named deferrals into one cleanup phase. Phase 117 is
NOT a substrate-novelty phase; ships focused integration
work taking the Phase 116 substrate and plumbing it into
live use.

**Streak outcomes:**
- DESIGN.md: HELD as predicted. Streak → 8.
- PRODUCT.md: HELD as predicted. Streak → 8.
- `aivyx-core/src/lib.rs`: BROKE against predicted hold
  (70/30 risk acknowledged at sign-off; the 30% case
  fired at the new `AuditTag::SkillInvocation` variant).
  Streak resets 1 → 1.

Test count: 2174 → 2188 (+14). **Below** the predicted
+30 to +60 band — honest scope reduction in Task 2
(reusing existing `SystemPromptRefiner` trait rather
than introducing a new `DynamicSystemPromptBuilder`
trait + ConcreteAgent surface) cut test surface
significantly. The substrate end state is the same; the
simpler path got there with fewer test artifacts.
Phase 6 Q5 honesty.

Zero new workspace deps. Zero clippy warnings.

After Phase 117, the last named Chapter E axis (outcome-
driven Profile/Role refinement) is the only Chapter E
direction remaining. The post-Phase-117 deferral ledger
is empty — both Phase 116 named deferrals closed in
this phase.

## Phase 116 — Tool/Skill Selection Learning from Outcomes (Chapter E #3)

**Frozen — see [PHASE_116.md](PHASE_116.md).** Third phase
of Chapter E. The agent's tool/skill selection has been
pure LLM intuition since Phase 0; Phase 116 builds a
relevance ledger that tracks per-tool/per-skill success/
failure outcomes per keyword-extracted turn pattern, and
surfaces accumulated signal in the next turn's system
prompt as a passive augmentation. The LLM still picks;
just better informed.

**Q-block (all three Recommended) — conservative shape:**
- Q1a — **Keyword-set from user input** (Recommended).
  Cheap deterministic; zero LLM cost per turn. Phase 95
  precedent for cheap-deterministic-signal substrate.
- Q2a — **System-prompt section** (Recommended). Passive
  augmentation; no tool-call substrate change.
- Q3a — **Tools AND skills together** (Recommended).
  Symmetric coverage.

**Streak outcomes — two positive surprises:**
- DESIGN.md: HELD byte-identical against predicted break.
  Streak → 7. The ledger lives in aivyx-channel like every
  other learning ledger (Phase 78 / 82 / 83); none of
  those broke D, and this one didn't either.
- PRODUCT.md: HELD byte-identical against predicted break.
  Streak → 7. P8 envelope absorbed "outcome-tracked
  selection hint" cleanly without amendment.
- `aivyx-core/src/lib.rs`: BROKE as predicted at the new
  `pub mod relevance` line. Streak resets 4 → 1.

Test count: 2129 → 2174 (**+45**), inside the predicted
`+40 to +70` band. Zero new workspace deps. Zero clippy
warnings.

**Two Phase-116-internal deferrals named:**
- Live-prompt augmentation pipe. Renderer is ready;
  integration awaits a per-turn prompt-reassembly
  substrate change (the planner today builds the system
  prompt once at session-construction).
- Per-skill outcome tracking. Skills currently record as
  `skills.invoke` itself; per-skill granularity needs a
  side-channel capture path that bypasses audit
  input-hash protection.

After Phase 116, one named Chapter E axis remains
(outcome-driven Profile/Role refinement). The next
Chapter E phase opens against that axis OR closes the
Phase-116-internal deferrals — operator-pressure-shaped at
the next phase exit.

## Phase 115 — Self-Correction Loop on Failed Turns (Chapter E #2)

**Frozen — see [PHASE_115.md](PHASE_115.md).** Second
phase of Chapter E. Closes the symmetric negative-
feedback half of Phase 114's auto-proposer. The agent
observes failed turns (Failed / Cancelled / TimedOut /
Escalated) and proposes targeted Persona refinements to
prevent recurrence. Same pipeline, broader trigger —
Q3a sign-off (extend Phase 114 substrate).

**Q-block (one non-Recommended, two Recommended):**
- Q1c — **All non-Completed outcomes** (non-
  Recommended). Maximum-signal posture; per-failure-
  type enable flags + judge confidence threshold +
  per-category enables gate noise.
- Q2a — **Let judge pick from full surface**
  (Recommended). Same polymorphic surface as Phase 114;
  per-category enables apply uniformly to failure-
  driven proposals.
- Q3a — **Extend Phase 114 substrate** (Recommended).
  Same orchestration; new `from_failed_turns` config
  knob; new `source` field on audit-event with
  serde-skip-if-none for backward compatibility.

**Streak outcomes** — all three predictions held; fourth
all-hold result in a row (Phase 112-115). Longest
streak-hold run in project history; substrate is
genuinely mature.
- DESIGN.md: HELD as predicted (no D-section touch).
  Streak → 6.
- PRODUCT.md: HELD as predicted (P8 envelope).
  Streak → 6.
- `aivyx-core/src/lib.rs`: HELD as predicted (extensions
  inside the existing `skill_proposer` boundary).
  Streak → 4.

Test count: 2105 → 2129 (**+24**), inside the predicted
`+20` to `+35` band. Zero new workspace deps. Zero
clippy warnings.

Backward-compatible audit chain: `AuditEvent::
SkillAutoProposal` gained a `source` field with
`#[serde(default, skip_serializing_if = "Option::is_none")]`;
pre-Phase-115 entries verify byte-identically. Phase 92's
`supersedes_proposal_id` pattern.

One Phase-115-internal deferral named: operator-resolve
escalation (`/reject`) integration would require hooking
the persona-proposal-resolve site separately from the
turn-finalize hook — held unless operator pressure
surfaces.

After Phase 115, the agent self-learns AND self-corrects
across the same 11-category PersonaDelta surface. Chapter
E's remaining named axes (tool/skill selection learning;
outcome-driven Profile/Role refinement) ship operator-
pressure-shaped at the next phase exit.

## Phase 114 — Persona Auto-Proposer Generalization (Chapter E opener)

**Frozen — see [PHASE_114.md](PHASE_114.md).** First
phase of Chapter E. Generalizes the Phase 112 skill auto-
proposer from `LearnedSkill` to the full 11-category
`PersonaDeltaCategory` surface. After Phase 114, the
inline-at-turn-boundary auto-proposer can draft
BehavioralPreferences, LearnedContext,
CommunicationAdaptations, and the other 7 categories in
addition to skills — every Persona axis self-learns end-
to-end.

**Q-block at sign-off (one Recommended, two non-
Recommended):**
- Q1b — **Per-category TOML config block** (non-
  Recommended; operator picked over uniform-single).
  More flexible; more substrate.
- Q2a — **Reuse the four Phase 112 signals**
  (Recommended). Same heuristic gate; judge handles
  per-category specifics.
- Q3a — **Reuse existing scope bases** (Recommended).
  `skills.propose` for `LearnedSkill`; `persona.propose`
  for the other 10 categories. No new KNOWN_BASES entry;
  A3 stays at 49.

**Streak outcomes** — all three predictions held;
third all-hold result in a row (Phase 112 first, 113
second, 114 third — substrate momentum at its highest
point of the project).
- DESIGN.md: HELD as predicted (no D-section touch).
  Streak → 5.
- PRODUCT.md: HELD as predicted (P8 envelope).
  Streak → 5.
- `aivyx-core/src/lib.rs`: HELD as predicted (no new
  `pub mod`). Streak → 3.

Test count: 2068 → 2105 (**+37**), inside the predicted
`+30` to `+50` band. Zero new workspace deps. Zero
clippy warnings.

Backward-compatible audit chain: `AuditEvent::
SkillAutoProposal` gained an optional `category` field
with `#[serde(default, skip_serializing_if =
"Option::is_none")]`; existing chains verify
byte-identically. Phase 92's `supersedes_proposal_id`
precedent.

## Phase 113 — Operator-Surface Polish (deferral cleanup)

**Frozen — see [PHASE_113.md](PHASE_113.md).** Standalone
phase past Chapter D's close, mirroring the Phase 20
(Daemon Management + Deferral Cleanup) precedent: batch
five small named operator-surface deferrals into one
focused phase rather than carrying them across more
substrate work.

Closes the accumulated operator-surface deferrals from
Chapter D and Phase 112:
- Phase 112 TOML `[skills.auto_propose]` config loader
  in `aivyx-config`.
- Phase 112 daemon auto-construction of
  `SkillAutoProposerContext` from the loaded TOML (makes
  the auto-proposer actually enable-able via config
  rather than only via source-edit).
- Phase 112 operator-inspection flags: `aivyx persona
  list --auto-only` / `--manual-only` filter the persona
  chain by `pd-auto-*` `delta_id` prefix; `aivyx audit
  export --event-type SkillAutoProposal` filter
  on the existing audit-export JSONL emitter.
- Phase 110-deferred A3 amendment addendum: `KNOWN_BASES`
  inventory catches up from 43 (Phase 54 refresh) to ~47
  (add `git.read` from Phase 109 + `skills.propose` +
  `skills.list` + `skills.invoke` from Phase 110).
- Phase 112 INSTALL.md paragraph covering the
  `[skills.auto_propose]` config + revert escape hatch.

**Q-block at sign-off:** Q1a — **bundle A3 into Phase
113** (operator-picked Recommended).

**Streak outcomes** — all three predictions held; first
all-hold result since Phase 56 / Phase 108:
- DESIGN.md: HELD against predicted break. The A3
  addendum lives in `docs/amendments/`, not DESIGN.md
  proper. Streak → 4.
- PRODUCT.md: HELD as predicted (no contract touch).
  Streak → 4.
- `aivyx-core/src/lib.rs`: HELD as predicted (work
  lives in `aivyx-config`, `aivyx-channel/bin`, docs
  tree only). Streak → 2.

Test count: 2044 → 2068 (**+24**), inside the predicted
`+15` to `+25` band. Zero new workspace deps. Zero
clippy warnings.

KNOWN_BASES catch-up landed at +6 (43 → 49), not the
open doc's ~+4 estimate. Phase 113 surfaced two
post-Phase-54 entries the open doc missed:
`persona.propose` (Phase 59) and `notify.send`
(Phase 62). The A3 addendum traces all six.

After Phase 113, every named Chapter D + Phase 112
operator-surface deferral is closed; the Channel
Activation Milestone is fully ready to run.

## Phase 112 — Skill Auto-Proposer (Phase 110's named follow-on)

**Frozen — see [PHASE_112.md](PHASE_112.md).** Shipped the
agent-side auto-proposer that closes the last named Chapter
D follow-on (Phase 110's deferral) and the project-vision
critical-path piece. After Phase 112, the agent self-learns
at the skill layer end-to-end: complex turns fire the
auto-proposer in a detached background task; high-confidence
verdicts auto-accept into the LearnedSkill chain; below-
threshold verdicts stage for operator review through the
existing `aivyx persona proposals approve` surface.

**Streak outcomes** — 2 of 3 streaks held (1 positive
surprise), 1 broke as predicted:
- DESIGN.md: HELD (predicted to break). The substrate
  slotted into the existing D4 surface without a new
  section. Streak → 3.
- PRODUCT.md: HELD as predicted. P8 covers inline-fired
  reflection identically to cron-fired. Streak → 3.
- `aivyx-core/src/lib.rs`: BROKE as predicted with the new
  `pub mod skill_proposer`. Streak resets to 1.

Test count: 1950 → 2044 (**+94**, way past the predicted
`+30` to `+50` band). Zero new workspace deps; the LLM
substrate (Phase 25 / Phase 91), tokio, audit log, and
persona chain were all already vendored. Zero clippy
warnings.

**Two Phase-112-internal deferrals** — small operator-
surface pieces shipped as a focused follow-on:
- TOML `[skills.auto_propose]` config-section loader in
  `aivyx-config` (the struct exists in `aivyx-channel`
  with `Default` impl; promotion follows the Phase 91
  `RecallJudgmentConfig` precedent).
- `aivyx persona list --auto-only` / `--manual-only` and
  `aivyx audit export --event-type` filter flags. Data is
  already audit-logged; these are operator-convenience.

**The Channel Activation Milestone is still unblocked**
(Phase 111 closed the adapter-wiring carve-outs) and the
self-learning loop is now closed end-to-end at the
substrate level. The operator can run the milestone +
exercise the auto-proposer in the same session whenever
the verification window opens.

 Standalone
phase past Chapter D's close (matching the Phase 111
precedent for closing Chapter-D-internal deferrals as
follow-on phases). Closes the last named Chapter D
deferral: Phase 110's *"agent-side auto-proposer heuristic
(fire reflection-cron-style after complex turns, draft
skill proposals automatically)."*

After Phase 112, the agent self-learns at the skill layer:
complex turns trigger an LLM-judged proposal, high-
confidence proposals auto-accept into the LearnedSkill
chain, and the next turn's system prompt already carries
the new skill. The propose-approve-render-invoke substrate
shipped at Phase 110 becomes a propose-judge-accept-
render-invoke loop where "judge" replaces the operator's
manual approval for the steady-state high-confidence
case.

**Q-block (three of four non-Recommended; operator picked
the more autonomous shape):**
- Q1b — heuristic + LLM-judge (Recommended).
- Q2b — **inline at turn boundary** (non-Recommended;
  picked over cron-fired). Background-task spawned
  post-`finalize` keeps critical-path latency at zero.
- Q3b — **threshold-gated auto-accept** (non-Recommended;
  picked over always-staged). Operator-configurable TOML
  threshold defaulting `0.85`.
- Q4b — **title fuzzy-match + LLM semantic check** (non-
  Recommended; picked over title-only). LLM check
  piggybacks on the Q1b judge call.

This is the **project-vision critical path** — the
operator's stated vision is "Self-Learning, Self-Improving
AI Personal Assistant," and Phase 112 turns the Phase 110
operator-driven framework into actual self-learning. All
five adapters inherit the auto-proposer for free (it
hooks the turn-finalize event, not any specific adapter).
Zero new workspace deps; LLM substrate already in place
(Phase 25 multi-provider, Phase 87 phrasing, Phase 91
judgment, Phase 92 supersession).

DESIGN.md streak predicted to break (D4 skills section
extended); PRODUCT.md predicted to extend to three (P8
envelope); `aivyx-core/src/lib.rs` predicted to break
(new `skill_proposer` module).

## Phase 110 — Skills Auto-Creation (Reflection Staging) (Chapter D)

**Frozen — see [PHASE_110.md](PHASE_110.md).** The sixth and
final Chapter D item — the substrate-design-heavy piece the
ROADMAP flagged with "highest amendment risk." Extends the
existing reflection layer (Phase 29 propose / apply, Phase
30 role-mutation, Phase 59 Persona propose, Phase 60 Persona
revert, Phase 70 proposal review surface) with a **skills
primitive**: procedural patterns the agent drafts after
complex turns, staged as Persona-style deltas the operator
approves via the existing persona-proposal surface, then
rendered into the agent's system prompt alongside Persona
content on every subsequent turn. Mid-ground between
Hermes's autonomous skill creation and Aivyx's current
per-action reflection propose / apply; stays inside P8's
"outcome-driven audited reflection" envelope. Q1a chose
**11th PersonaDeltaCategory variant `LearnedSkill`** reusing
the entire Persona substrate. Q2b chose **new
`skills.propose` scope base** for per-category granularity
(operator pick over the Q2a "extend persona.propose"
recommendation). Q3c chose **both render-in-prompt AND
callable `skills.list` / `skills.invoke` tool surface**
(operator pick over the Q3a "render-only" recommendation).
Q4a kept **foundation scope** — propose + approve + render
+ tool-surface; agent-side auto-proposer heuristic
deferred. DESIGN.md break predicted at the new scope base;
`aivyx-core/src/lib.rs` break predicted at the two new
tool re-exports; PRODUCT.md predicted to hold (P8 envelope
unchanged). All three streaks already reset at Phase
109's triple-break, so Phase 110 lands its predicted breaks
without compounding break-on-break framing.

## Phase 109 — Tool Breadth + Amendment A12 (Chapter D)

**Frozen — see [PHASE_109.md](PHASE_109.md).** The fifth
Chapter D item — Hermes-comparison tool-breadth gap closure.
Three new substrate tools across A12 + one Phase-100 audit
closure: `git.status` and `git.diff` sharing a new
`git.read` capability scope (qualified by repo path; shells
out to system `git` binary; no Rust deps), plus `net.dns`
closing one of Phase 100's eight declared-but-toolless
scopes (`tokio::net::lookup_host`; uses the existing
`net.dns` scope base from Phase 0; no amendment needed for
the scope itself). A12 takes P10's substrate tool count
from 10 → 13. Q3a chose **mirror A11 exactly** — short
amendment file structurally identical to A11 (which itself
mirrored A5 from Phase 37). DESIGN.md and PRODUCT.md
streaks deliberately break at A12 (same pattern as the
A11 / A5 amendment commits); `aivyx-core/src/lib.rs`
streak extends (tools land in `aivyx-channel`, not core).
Zero new workspace deps. The other seven Phase-100-audited
toolless scopes (`shell.spawn`, `audit.read`,
`config.read`, `config.write`, etc.) stay deferred per
Phase 100's audit conclusions.

## Phase 108 — Slack Channel Adapter (`aivyx-slack`) (Chapter D)

**Frozen — see [PHASE_108.md](PHASE_108.md).** The fourth
Chapter D item and the four-data-point confirmation for the
adapter pattern Phase 9 wrote down (then Phase 107 promoted
to confirmed-at-three). Adds a new workspace crate
(`aivyx-slack`) at foundation scope — DMs + channel
messages only; threads, Block Kit, attachments, slash
commands all stay named deferrals. The headline outcome: an
operator with a Slack workspace runs `aivyx --channel slack`
with a bot token + an app-level Socket Mode token, talks to
the bot from any DM or invited channel, and gets the same
agent experience they already get on Discord and Telegram.
Q1a chose **slack-morphism** as the SDK (thin protocol
wrapper, matches Phase 107's twilight-rs decision). Q2a
chose **Socket Mode only** (WebSocket initiated outbound
from aivyx, no public endpoint — matches Discord's Gateway
shape and the substrate's local-daemon posture). Q3a chose
`format!("{team_id}:{channel_id}")` as the partition key —
**confirms three-data-point `Option<String>` partition
shape at four data points**, explicitly punts the Phase 9
Q7 richer-type question to a future Matrix-shaped adapter
where `room_id + homeserver` makes the structured type
genuinely necessary. Q4a chose **foundation scope** over
full Phase-107-style parity — Slack's protocol surface is
simpler than Discord's so foundation is genuinely the right
call here. One streak break predicted: zero-new-deps
(slack-morphism adoption); two surprises expected to repeat
the Phase 107 pattern (DESIGN.md held byte-identical via
A4-addendum-in-amendment-file pattern; `aivyx-core/src/lib.rs`
held byte-identical because `ChannelPlatform::Slack` was
already forward-enumerated in Phase 8). Daemon-frontend
variant carved out as Phase-108-internal deferral that
bundles with the Phase 107 daemon-frontend follow-on.

## Phase 107 — Discord Channel Adapter (`aivyx-discord`) (Chapter D)

**Frozen — see [PHASE_107.md](PHASE_107.md).** The third
Chapter D item and the first to genuinely grow the substrate.
Phases 105 and 106 were lower-risk reader / docs work; Phase
107 added a new workspace crate (`aivyx-discord`) at full
parity with the Phase 8/9 `aivyx-telegram` adapter at the
in-process layer — a `ChannelContext` impl, a private
`DiscordTransport` trait + scripted double, a
`run_discord_session` sibling of `run_session` /
`run_telegram_session`, and binary wiring through
`ChannelKind::Discord`. twilight-rs 0.16 (Q1a — thin
protocol wrapper matching the Phase 8 frankenstein-vs-
teloxide decision; three new direct workspace deps:
`twilight-gateway`, `twilight-http`, `twilight-model`).
Full-parity scope (Q2c). Text commands for `/approve` /
`/reject` matching Telegram (Q3a). Two streak-prediction
surprises in the operator's favor: **`aivyx-core/src/lib.rs`
held byte-identical** (Phase 8 had already
forward-enumerated `ChannelPlatform::Discord` alongside
Slack / Matrix / Email / Rest; no variant-lift needed) and
**DESIGN.md held byte-identical** (A4 addendum landed in
the amendment file per the Phase 49 precedent). Streaks at
exit: DESIGN.md → 54, PRODUCT.md → 7, `aivyx-core/src/lib.rs`
→ 7. Workspace tests `+29` → 1888 (below the predicted
`+50`–`+100` band — Discord's push-based Gateway simplified
the substrate the scripted suite covers; the Telegram
precedent's larger test count is substrate-discovery work
Phase 107 didn't need to redo). Two Phase-107-internal
deferrals named honestly at exit: **Discord daemon-frontend**
(`FrontendType::Discord` + `discord_daemon_frontend.rs`
mirroring Phase 19's Telegram-over-daemon path), and
**`/approve` / `/reject` gate-resolve routing** (which lives
in the daemon-frontend half of the Telegram precedent). The
in-process Discord adapter is end-to-end functional today;
daemon-mode is a deployment optimization that lands as a
focused follow-on commit. A4 amendment addendum filed
(12 → 13 crates). Real-protocol smoke test deferred to the
Channel Activation Milestone per the
`docs/ADAPTER_PATTERN.md` checklist.

## Phase 106 — MCP Server Breadth (Curated Recipes) (Chapter D)

**Frozen — see [PHASE_106.md](PHASE_106.md).** The second
Chapter D item. Phase 46 shipped the first bundled MCP server
(`aivyx mcp-server web-search`); Phase 24 / 32 shipped the
external `[[mcp_server]]` TOML surface for plugging in any
MCP-compatible binary; Phase 55 shipped the sandbox layer
that wraps each spawn. What was missing was the **catalog** —
an operator's discovery story for "which MCP servers should I
actually enable, and what do their blocks look like with the
right sandbox config?" Phase 106 shipped that catalog as
`docs/MCP_RECIPES.md` (12 worked recipes: filesystem, github,
gitlab, sqlite, postgres, time, fetch, brave-search, slack,
memory, puppeteer, everything) plus a new `aivyx mcp recipes
[<name>]` CLI subcommand for in-shell discovery. Q1a chose
recipes-only scope (no new bundled-server code paths); Q2a
chose the doc + CLI surface (mirrors Phase 103's
`aivyx tool init`); Q3a chose inline `[mcp_server.sandbox]`
per recipe so a copy-paste produces a sandboxed config out of
the gate (Phase 55 substrate-default posture). All three
streak predictions held: DESIGN.md → 53, PRODUCT.md → 6,
`aivyx-core/src/lib.rs` → 6. Zero new workspace deps;
workspace tests `+19` → 1862 (above the predicted `+8`–`+12`
band by seven; over-shoot called out in the
prediction-vs-reality section — pinning every recipe's
`[[mcp_server]]` + `[mcp_server.sandbox]` blocks as twin
shape tests names the Q3a sandbox-by-default contract more
clearly than a single combined test would). New
bundled-server code stays a Chapter D deferral pending
operator pressure.

## Phase 105 — Trajectory Logging (`aivyx audit export`) (Chapter D opener)

**Frozen — see [PHASE_105.md](PHASE_105.md).** Chapter D's
opener and the easiest-wins-first item of the Hermes-
comparison-driven arc. A read-only offline subcommand that
emits the HMAC audit chain as JSONL on stdout. Each line is
the full `SignedEntry` projection (`seq +
appended_at_ms + prev_mac + mac + event` — Q1a), enough that
downstream tooling can re-verify the HMAC against a
separately-supplied genesis seed. Filters in v1 are
sequence-based only (`--from <seq>` + `--limit <N>` — Q2a)
mapping directly to `PersistentAuditLog::entries_range`,
which Phase 47 already shipped for the Web UI paginated
viewer. Source path is offline-only via cold-start storage
open (Q3a) — same code path as `aivyx --verify-only`,
requires the passphrase, no new IPC variant. All three
streak predictions held: DESIGN.md → 52, PRODUCT.md → 5,
`aivyx-core/src/lib.rs` → 5. Zero new workspace deps;
workspace tests `+18` → 1843 (above the predicted `+6`–`+12`
band by six; over-shoot in parse-test coverage of error
paths + per-helper coverage in `audit_export.rs`). New
`docs/AUDIT_EXPORT.md` reference doc carries the JSONL shape,
worked `jq` examples, and the re-verify-downstream procedure.
Three named deferrals at exit: time-range filters
(`--since` / `--until`), correlation filters
(`--session` / `--mission`), and daemon-mode export over
the Phase 47 `Query` envelope.

## Phase 104 — `aivyx init` Polish (Chapter C opener)

**Frozen — see [PHASE_104.md](PHASE_104.md).** Chapter C's
opener. After 60-plus phases of substrate work, the init
wizard's first-touch UX had accumulated three concrete
paper-cuts: stale Anthropic/OpenAI default model strings
(`claude-sonnet-4-20250514` was a year out of date; `gpt-4o`
was no longer the current flagship), no-models guidance on
the Ollama path that said `Run "ollama pull <model>" first.`
with no concrete recommendation, and a write-then-find-out-
later validation gap where a typo'd key or non-existent
model only surfaced on first turn (after the operator had
already committed to a passphrase and opened the daemon).
Phase 104 closed all three: refreshed the defaults to
current generation (`claude-sonnet-4-6` / `gpt-4.1` per Q3a),
added a single hardcoded `Try: ollama pull llama3.2:3b`
suggestion on the empty-models path per Q4a, and added a
`verify-before-write` step that hits `GET /v1/models` with
the supplied key, confirms the chosen model is in the
returned list, and re-prompts on failure with a three-retry
cap and a `Write anyway?` escape hatch (Q1a/Q2a). Additive
on the operator-facing surface; new `aivyx-llm::verify`
module gated on existing provider features. All three
streak predictions held: DESIGN.md → 51 (one past the
Phase 103 half-hundred milestone), PRODUCT.md → 4,
`aivyx-core/src/lib.rs` → 4. Zero new workspace deps;
workspace tests `+17` → 1825 (above the predicted `+8`–`+12`
band; over-shoot in `verify.rs`'s exhaustive parse+classify
coverage for the auth-vs-model-not-found classifier the
wizard's retry loop branches on).

## Phase 100 — Tool-Surface Gap Closure (Chapter B opener)

**Frozen — see [PHASE_100.md](PHASE_100.md).** Chapter B's
opener. `fs.delete` and `fs.metadata` are D4-original
substrate scope bases (Phase 0, in Amendment A3's inventory)
that have never had a first-party tool — the agent can read
and write a file but cannot delete or stat one. Closing the
gap is gated by P10: Amendment A5 locked the substrate tool
list at exactly eight, so Phase 100 is an **amendment
phase** — it files Amendment A11 extending P10 to ten tools
(the A5 pattern that took the count seven → eight when Phase
37 added `web.post`), then ships `fs.delete` and
`fs.metadata` behind it. Directory listing folds into
`fs.metadata` or earns a new `fs.list` scope (Q1). The other
eight declared-but-toolless scopes (`shell.spawn`,
`net.dns`, `audit.read`, `config.read`, `config.write`,
`display.window_close`, `memory.gc`, `mission.gate`) are
audited and each ruled "tool later" or "deliberately
reserved." Streaks at exit: PRODUCT.md broke at 39
(Amendment A11) and `aivyx-core/src/lib.rs` broke at 47
(new tool re-exports); DESIGN.md held → 47 (Q1 kept
directory listing inside `fs.metadata`). Workspace tests
`+32` → 1778.

## Phase 101 — Tool-Call Input Validation & Repair (Chapter B)

**Frozen — see [PHASE_101.md](PHASE_101.md).** Chapter B's
reliability item. Two `dev-verify` runs (Phases 99, 100)
caught local models emitting tool calls with malformed
arguments. The planner already loops the model to retry an
**unknown tool name**; a *known* tool called with bad input
is dispatched blind and fails ad-hoc inside the tool. Phase
101 makes the two halves symmetric: a **validate-before-
dispatch** step checks a known call's input against the
tool's `input_schema()` (the JSON Schema every `Tool`
already exposes), and on a mismatch appends a structured
`invalid_input` result echoing the expected schema, looping
the model to **repair** the call. A two-repair cap then
dispatches as-is (the tool's own `execute` validation is the
floor). Validation uses the `jsonschema` crate — the first
net-new workspace dependency since Phase 27, a deliberate
Q2 choice of spec-correct validation over a hand-rolled
checker. Additive: a well-formed call leaves the planner
byte-identical to pre-Phase-101. Streaks at exit: all three
held — DESIGN.md → 48, PRODUCT.md and `aivyx-core/src/lib.rs`
each re-establish to 1 after their Phase 100 breaks.
Workspace tests `+9` → 1787.

## Phase 102 — Tool Observability (`aivyx tools`) (Chapter B)

**Frozen — see [PHASE_102.md](PHASE_102.md).** Chapter B's
observability item. Phase 100 widened the tool surface and
Phase 101 made tool calls more reliable; neither gave the
operator a way to *see* the tool layer. Phase 102 adds
`aivyx tools` — a read-only subcommand, sibling of `aivyx
memory` / `aivyx learning`, that lists every registered
tool (name, description, capability base) and annotates
each with audit-derived call statistics: total calls, the
outcome breakdown (completed / failed / denied / …), and
timing, with a `--window <secs>` filter. The data already
exists — every tool call is an `AuditEvent::ToolCall` in
the chain, keyed by the stable `scope_used.base()`. A new
daemon IPC `GetToolStats` query carries it; the variant is
additive and backward-compatible under the Phase 41
protocol handshake. All `aivyx-channel`; `aivyx-core`
untouched. Streaks at exit: all three held — DESIGN.md → 49,
PRODUCT.md → 2, `aivyx-core/src/lib.rs` → 2. Zero new deps;
workspace tests `+12` → 1799.

## Phase 103 — External Tool Ergonomics (`aivyx tool init`) (Chapter B)

**Frozen — see [PHASE_103.md](PHASE_103.md).** Chapter B's
closing item. Three Chapter B phases shipped the *operator's*
tool experience (Phase 100 surface, Phase 101 reliability,
Phase 102 observability); Phase 103 closes the chapter on the
*third-party tool author's* experience. `aivyx tool init
<path>` writes a runnable Rust tool-process project: a
`Cargo.toml` depending on the existing `aivyx-tool` crate
(which already re-exports the wire types and framing — no new
SDK helper to add), a `src/main.rs` with the handshake +
invocation main loop and a handler stub the author replaces,
a README, a conformance test, and a `[[tool_process]]`
snippet to paste into `aivyx.toml`. Rust over Python at
operator choice (Q2): the existing `examples/python-tool/`
already covers stdlib-Python, and the missing scaffold is for
authors who want the `wire.rs` enums' type-safety and the
`cargo` toolchain. Additive — a new CLI subcommand, no
daemon IPC, no existing path altered. Streaks at exit: all
three held — DESIGN.md → 50 (project milestone, fifty
consecutive phases), PRODUCT.md → 3, `aivyx-core/src/lib.rs`
→ 3. Zero new deps; workspace tests `+9` → 1808.

**Chapter B closes here.** Phase 100 widened the tool surface
(`fs.delete`, `fs.metadata`, A11 amending P10 to ten tools);
Phase 101 added planner validate-before-dispatch + repair so
malformed tool calls round-trip back to the model for fixing;
Phase 102 added `aivyx tools`, the read-only observability
view; Phase 103 added `aivyx tool init`, the scaffolder for
third-party tool authors. All four expected items shipped;
no further pre-named Chapter B phases remain.

## Chapter A — Foundation Closeout (Phases 50–54) [COMPLETE]

After Phase 49 closed the PRODUCT.md forward-commitment ledger,
the project pivoted from "deliver remaining commitments" to
"close out Phase 0–49 loose ends." Chapter A was the
finish-the-job arc: pay down the deferral backlog, close the
P12 deferred half, harden the tool-process surface with
container isolation, and exit with a documentation sweep.

| Phase | Status | Headline |
|---|---|---|
| 50 | ✓ shipped | P12 closeout — `run_tool_as_subprocess<T: Tool>` + `p12_equivalence.rs` |
| 51 | ✓ shipped | Cleanup — typed `AivyxError`, `ConnectionContext`, TOML passphrase |
| 52 | ✓ shipped | Sandbox layer — generic command wrapper for `[[tool_process]]` |
| 53 | skipped | Audit log rotation — no pressure to date |
| 54 | ✓ shipped | Final documentation sweep |

**Chapter A retrospective.** The arc ended with the project at
*every PRODUCT.md commitment honored, every visible loose end
closed, every doc in sync with the substrate*. Five phases held
the discipline of small scopes + Q-block sign-off + honest
streak predictions. The deliberate break (lib.rs streak at
Phase 51 Q1) was the right call: D6's `AivyxError` shape had
been wrong for 50 phases; honoring it was overdue. No
regression-induced rollbacks; no contract amendments that
required reshaping shipped surface. The substrate held. Future
work is operator-feedback-driven and amendment-driven rather
than speculative — the next numbered phase opens in response
to a specific need.

## Phase 54 — Final Documentation Sweep (Chapter A Closer) [SHIPPED]

**Frozen — see [PHASE_54.md](PHASE_54.md).** Closed Chapter A
with a docs catch-up. Root README rewritten (179 lines, current
state + five-minute setup). PRODUCT_ROADMAP Delivered section
refreshed with all 12 PRODUCT.md commitments grouped by category
+ Chapter A entries. DAEMON_IPC Phase 47 Query/QueryResponse
addendum. A3 amendment addendum brings scope-base count from
"24" to the current 43 with a full enumeration grouped by tier.
Cross-doc consistency spot-check found no drift. DESIGN.md
streak broke at 4 (A3 addendum, predicted); PRODUCT.md and
lib.rs streaks held at 4 and 2. No code changes.

## Phase 53 — (skipped)

Audit log rotation/compaction was scheduled here. Skipped at
Phase 52 exit: audit chain growth is bounded by tool-call
frequency × uptime; even at 10k turns/year the chain stays well
under 1M rows. If chain-size pressure surfaces during real
operation, a future cleanup phase will absorb it.

## Phase 52 — Tool Process Sandbox Layer [SHIPPED]

**Frozen — see [PHASE_52.md](PHASE_52.md).** Added a generic
command-wrapper sandbox layer to `[[tool_process]]`.
`[tool_process.sandbox] { wrapper, args }` is prepended to the
spawn — `wrapper wrapper_args... command command_args...`.
Aivyx supplies the policy slot; the operator supplies the
policy (bubblewrap, firejail, docker, sandbox-exec). Closes
the Phase 49 sandboxing deferral and narrows THREAT_MODEL.md
§5.6. New `aivyx-tool::SandboxConfig` + matching `aivyx-config`
type + binary wiring. Integration test uses POSIX `env` as a
universal no-op wrapper so the suite runs anywhere `cargo test`
runs. `docs/TOOL_SDK.md` §9 documents three worked examples
(bwrap / firejail / docker). 984 tests, zero clippy. All three
streak predictions correct (DESIGN.md → 4, PRODUCT.md → 3,
lib.rs → 1).

## Phase 51 — Cleanup: Error Typing + ConnectionContext + Passphrase Path [SHIPPED]

**Frozen — see [PHASE_51.md](PHASE_51.md).** Mechanical
Chapter A cleanup phase. Closed three independent items:

1. **`AivyxError::{Storage,Crypto}` typed nested errors.** The
   two TODOs in `aivyx-core/lib.rs:689,693` that have sat in
   production code since Phase 1. D6 prescribed `#[from]
   StorageError` / `CryptoError`; Phase 51 finally wires them.
   `StorageError` and `CryptoError` gained `Clone` derives;
   `aivyx-core` gained intra-workspace deps on both.
2. **`handle_connection` → `ConnectionContext` lift.** Removes
   the Phase 47 Task 4 `#[allow(clippy::too_many_arguments)]`
   shortcut; same shape as Phase 41 `DaemonConfig`.
3. **`AIVYX_PASSPHRASE` TOML/env inconsistency.** New
   `PassphraseSource::FromConfig(SecretString)` makes the
   `[aivyx] passphrase` TOML field actually drive Argon2id
   derivation. Phase 47 visual-pass footgun closed.

979 Rust tests (+6), zero clippy. lib.rs streak broke at 6
(deliberate, Q1 honored D6 after 50 phases of stub). DESIGN.md
streak → 3, PRODUCT.md streak → 2.

## Phase 50 — P12 Closeout: First-Party In-Process Protocol Unification [SHIPPED]

**Frozen — see [PHASE_50.md](PHASE_50.md).** Wired the two Phase
49 deferred bridge stubs: `ToolEvent` frames now relay onto the
channel (`Status`/`OutputChunk` → `StreamEvent`), and
cancellation is targeted via `CancelInvocation { call_id }`
using a caller-supplied id. Added `run_tool_as_subprocess<T:
Tool>` harness in `aivyx-tool::harness` — a generic function
that wraps any `aivyx_core::Tool` impl as a tool-process binary
with a synthesized `ToolContext` (channel relays events back as
`ToolEvent` frames; audit is null per the one-row-per-call
invariant). The canonical proof of P12's "extractable without
rewriting" clause is `tests/p12_equivalence.rs`, which drives
the same `FsReadTool` invocation through in-process `execute()`
and through the harness-wrapped subprocess via
`ToolProcessBridge` + `ToolProxy`, then asserts byte-identical
`ToolOutcome::Completed`. PRODUCT.md P12 moved to **Fully
Delivered**. 973 tests, zero clippy. lib.rs streak held at 6;
DESIGN.md held at 2; PRODUCT.md broke at delivery-status
refresh (honest break, predicted).

## Phase 49 — Tool Process IPC Foundation (P12) [SHIPPED]

**Frozen — see [PHASE_49.md](PHASE_49.md).** Delivered the
last `PRODUCT.md` forward commitment. New 12th workspace crate
`aivyx-tool` shipping `ToolProcessBridge` (spawns + handshakes a
child over length-prefixed JSON on stdin/stdout, `kill_on_drop`
safety net), `ToolProxy` (implements `aivyx_core::Tool` by
delegating to the bridge), `[[tool_process]]` TOML config with
operator scope-narrowing overrides, `docs/TOOL_SDK.md` as the
v0 third-party contract, and `examples/python-tool/` — a stdlib-
only Python wordcount reference with a 9-test conformance suite
driving the real subprocess. Foundation phase: first-party
in-process protocol unification deferred to a future phase per
Q5. **All twelve PRODUCT.md commitments now shipped — the
forward-commitment ledger is closed.** DESIGN.md A4 addendum
filed (11 → 12 crates); PRODUCT.md Delivery Status refreshed
(both streak predictions broken deliberately, lib.rs streak
held at 4). 966 Rust tests + 24 Python conformance tests, zero
clippy.

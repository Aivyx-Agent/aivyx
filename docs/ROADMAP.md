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

Likely follow-ups (helpfulness-driven Persona decay;
pattern-driven Persona proposals; affinity re-ranking of
existing candidates; sequential/temporal patterns;
operator-tunable affinity policy) are operator-feedback-gated.

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

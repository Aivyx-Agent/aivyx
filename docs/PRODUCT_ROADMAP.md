## Aivyx Product Roadmap

A living list of **product-shape milestones** derived from the
forward commitments in [`../PRODUCT.md`](../PRODUCT.md). This is
the product analogue of [`ROADMAP.md`](ROADMAP.md): that file
tracks technical phases (Phase 13, Phase 14, …); this file tracks
product-shape work (Daemon Migration, Role-Config Migration,
Reflection Layer, …) that may span multiple phases or fold into
one.

**This is not a contract.** Milestone shapes are revised whenever
a phase exit teaches us something new. Sequencing is loose — if
the Reflection milestone turns out to need the Role-Config
milestone first, we reorder here and it costs one commit, not an
amendment. The locked product contract is `../PRODUCT.md`; this
document only answers *"what comes after the current phase, in
product terms?"*.

For the locked product contract, see [`../PRODUCT.md`](../PRODUCT.md).
For the locked design contract, see [`../DESIGN.md`](../DESIGN.md).
For numbered technical phases, see [`ROADMAP.md`](ROADMAP.md).

## How this document is maintained

- **One paragraph per milestone, maximum.** If a milestone grows
  task lists or open questions, it has outgrown the roadmap and
  belongs in its own `PHASE_N.md` — which means that milestone
  is probably ready to open as a phase (or a sequence of phases).
- **Milestones are named, not numbered.** A milestone may map to
  one phase, several phases, or share a phase with another
  milestone. The `ROADMAP.md` numbering is the source of truth
  for sequence; this document is the source of truth for
  *product-shape intent*.
- **At every phase exit**, any milestone that the exiting phase
  advanced is updated here with whatever was learned, and any
  milestone that landed entirely is moved to the "Delivered"
  section at the bottom (with a pointer to the phase that
  shipped it).
- **At every phase entry**, the relevant milestone entries here
  inform the new `PHASE_N.md` scaffold but are not copied into
  it — the phase doc carries the task list, the milestone entry
  carries the intent.

## Milestone — Daemon Migration (architecturally complete)

**Forward commitment:** [`PRODUCT.md` P4](../PRODUCT.md). **Keystone:** unlocks
P5, P12, P2, P1. **Status:** Architecturally complete after
five dedicated phases (16–20). Remaining daemon work is
incremental (crash recovery, in-flight turn replay) rather
than architectural.

The single biggest reshape on the product roadmap. Today's binary
runs as a one-shot per-channel process; the daemon milestone moves
state ownership (redb, audit chain, role registry, capability
ceiling) into a long-lived background process and reshapes every
existing channel adapter into a thin frontend that auto-attaches
to the running daemon over a local IPC socket. The migration spans
multiple dedicated phases. The shape of the IPC protocol is the
load-bearing design decision this milestone had to settle first,
because P5 (channel SDK) and P12 (tool IPC) both consume it.

**Phase 16 (Protocol Settlement, 2026-04-16):** settled the IPC
protocol shape — length-prefixed JSON frames over Unix domain
sockets, `DaemonLifecycleEvent` as a separate message type from
`StreamEvent`, OS-user auth per P4.4. Landed a PoC daemon server +
client + one round-trip integration test proving the protocol
carries one turn end-to-end. All byte-identity streaks held; six
architectural questions resolved with streak-preserving options.
See [`docs/PHASE_16.md`](PHASE_16.md).

**Phase 17 (Production Hardening, 2026-04-16):** converted
the PoC into a production-ready daemon substrate: multi-turn
session server with graceful shutdown via `CancellationToken`,
`daemon run` subcommand with full agent-stack wiring (CliMode
enum refactor), and a multi-turn client library (`DaemonSession`)
with `spawn_daemon_and_wait` auto-spawn logic. Closed 3 of 5
Phase 16 net-new deferrals. All byte-identity streaks held;
production-core streak at six consecutive phases (longest in
project history). See [`docs/PHASE_17.md`](PHASE_17.md).

**Phase 18 (Frontend Wiring, 2026-04-16):** wired the default
`aivyx` invocation to auto-spawn a daemon, connect via
`DaemonSession`, and run a REPL loop rendering `StreamEventPayload`s
via `render_for_cli()`. Daemon-first dispatch with in-process
fallback. `DaemonCancelHandle` for ctrl-C cancellation over IPC
(first ctrl-C sends `CancelTurn`, second exits). Discovered and
fixed a cancel-flag reset bug between turns. All byte-identity
streaks held; production-core at seven consecutive phases (longest
in project history). Closed the REPL-mode-over-IPC deferral.
See [`docs/PHASE_18.md`](PHASE_18.md).

**Phase 19 (Multi-Connection + Telegram Port, 2026-04-16):**
upgraded the daemon from single-connection to multi-connection
(task-per-connection with `ChannelFactory` dispatching on
`FrontendType`). Ported the Telegram adapter behind the IPC
boundary — `aivyx --channel telegram` now auto-attaches to the
daemon the same way the local CLI does. Transport types widened
to `pub` for binary access; binary line-count managed via
extraction to `telegram_daemon_frontend.rs`. All byte-identity
streaks held; production-core at eight consecutive phases.
Closed the Telegram-over-daemon deferral from Phase 16.
See [`docs/PHASE_19.md`](PHASE_19.md).

**Phase 20 (Daemon Management + Deferral Cleanup, 2026-04-16):**
non-product-shape cleanup phase closing six daemon-management
and capability-system deferrals accumulated during the migration:
`daemon status`/`stop` subcommands with `FrontendMessage::Shutdown`
IPC, PID file with `Drop` guard, `--no-daemon` flag for in-process-
only mode, daemon-mode banner parity (no IPC change needed —
metadata already in scope on the frontend side), `CapabilitySet::
grants` reflexivity investigation (reflexive for all practical
scopes), and `CEILING_SEMITRUSTED` doc-comment rewrite. Rolling
backlog 16 → 10. All byte-identity streaks held; production-core
at nine consecutive phases (new record). See
[`docs/PHASE_20.md`](PHASE_20.md).

**Phase 21 (Mission Primitive, 2026-04-17):** delivered the first
concrete piece of **P2 — Mission Primitive** as a product-shape
phase. Mission state model with six-state machine (`Created →
Running → GatePending → Completed | Failed | Cancelled`) persisted
to redb under `KeyDomain::Missions`. Two new capability bases
(`mission.create`, `mission.gate`) with tier ceilings. IPC protocol
extended with `ApprovalGate`, `ResolveGate`, `MissionCreated`,
`MissionStateChanged`, `GateResolved`. `MissionCreateTool` with
OnceLock factory pattern (preserving production-core streak).
Daemon `ResolveGate` handler wired end-to-end. CLI interactive
gate prompt (`Approve? [y/N]:`) and Telegram `/approve`/`/reject`
text commands. Five design decisions, five Q-block questions
resolved. All byte-identity streaks held; production-core at ten
consecutive phases (new record). Test delta +29 (569→598). See
[`docs/PHASE_21.md`](PHASE_21.md).

**Next:** Escalation→gate turn-loop wiring (daemon-side
orchestration between `TurnOutcome::Escalated` and
`mission::add_gate`), completing the full approval-gate lifecycle.
All primitives in place; the missing piece is the daemon's
turn-boundary gate creation logic.

## Milestone — Role-Config Migration (shipped in Phase 13)

**Forward commitment:** [`PRODUCT.md` P9](../PRODUCT.md). **Keystone:** unlocks
P1 (sub-agents), P2 (missions).

Phase 11 introduced the role primitive but left every capability
grant inside `crates/aivyx-channel/src/bin/aivyx.rs` at the
operator level. This milestone migrated those grants out of the
binary and into a per-role config file that declares **the
complete capability envelope** of each role: tool allowlist,
scope set with qualifiers, trust ceiling, memory topic prefix,
and the parent role it inherits from per **P7**.

**Landed shape (Phase 13, 2026-04-15):** per-role
`[[role]]` table-array entries in a single TOML file (not
one file per role), with `capability_scopes` parsed
directly via `Scope::parse` at config-load time and
`parent_role` as explicit single-inheritance (no implicit
`default` parenting). Attenuation is enforced against
declared sets only, walking up through empty ancestors;
an empty child's envelope triggers a one-step-deep
backcompat-floor substitution at runtime. A
worked-example `examples/aivyx.toml` demonstrates four
roles (`default`, `coder`, `researcher`,
`junior_researcher`) including the deliberate empty-
child surprise case, and a `--print-role <name>` debug
flag lets operators inspect the effective envelope of
any role without side effects. See
[`docs/PHASE_13.md`](PHASE_13.md) for the full phase
record.

Once the binary no longer hard-codes role bodies, two derived
capabilities become mechanical to add: a primary agent can
switch into a child role mid-session under an attenuated
envelope (the substrate **P1** needs), and a long-running
mission can be tied to a specific role's envelope independent
of which channel kicked it off (the substrate **P2** needs).
Both are now unblocked. Two non-blocking follow-ups are
recorded in Phase 13's deferrals (lift
`assemble_role_envelope` from the binary into
`aivyx-channel/src/lib.rs` for cross-crate integration
tests; ship per-tier worked examples for SemiTrusted and
Untrusted channels) — neither requires a dedicated sub-
phase and both can be picked up reactively whenever a
future phase needs them.

## Milestone — Mission Primitive ✓

**Delivered across Phases 21, 23, 28, 35.** **Forward commitment:**
[`PRODUCT.md` P2](../PRODUCT.md).

Phase 21 introduced the mission state machine (six states:
`Created → Running → GatePending → Completed | Failed |
Cancelled`) with HMAC-bounded persistence under
`KeyDomain::Missions`, two new capability bases
(`mission.create`, `mission.gate`), IPC protocol extensions
(`ApprovalGate`, `ResolveGate`, `MissionCreated`,
`MissionStateChanged`, `GateResolved`), `MissionCreateTool` with
the `OnceLock` factory pattern, daemon `ResolveGate` handler,
CLI interactive gate prompt, and Telegram `/approve` / `/reject`
text commands. Phase 23 added the escalation → gate turn-loop
wiring. Phase 28 added `mission.list` / `mission.status`
read-only inspection tools. Phase 35 wired
`ToolOutcome::RequiresEscalation` through the turn loop to
`TurnOutcome::Escalated` for the trigger path. P2 fully
delivered.

## Milestone — Sub-Agent Role-Switching (shipped in Phase 14)

**Forward commitment:** [`PRODUCT.md` P1](../PRODUCT.md). **Couples to:**
Role-Config Migration.

Once the Role-Config Migration lands, sub-agents become a
relatively small additional primitive: a turn-loop instruction
that switches the active role from parent to child for the
remainder of a sub-task and switches back when the sub-task
completes. The capability envelope of the child role is
mechanically the intersection of the parent's envelope and
whatever the child role's config declares — the type system
enforces attenuation per **P7**, so escalation is structurally
impossible. The milestone is expected to be small (one phase,
possibly shared) but it has to land *after* role-config
migration and *before* the mission primitive, because a
mission's approval gate may want to spawn a constrained
sub-task as part of how it checkpoints progress.

**Landed shape (Phase 14, 2026-04-16):** a `role.switch`
capability scope with a target-role `QualifierKind` in
`aivyx-capability` (parses as either bare `role.switch` or
`role.switch:<target>`; rejected `role.switch:*` because the
unqualified form is already the wildcard), a `RoleSwitchTool`
registered in the core tool registry, and an inline sub-
session dispatcher implemented as an `OnceLock`-backed factory
closure on the tool. The factory is constructed in
`crates/aivyx-channel/src/bin/aivyx.rs`'s startup path and
captures the provider, audit hook, tool registry, role table,
backcompat floor, and model — *without* extending
`ToolContext`'s shape, which keeps `aivyx-core/src/lib.rs`
byte-identical and the production-core streak intact at three
phases. The factory closure has exactly one `CapabilitySet`-
producing call: `assemble_role_envelope(&target_role, &roles,
&backcompat_floor)`. There is no second route. **P1.3
"structural impossibility of escalation" is a documentation
property pinned by integration tests** rather than a
type-system property in the strict sense — the documentation
property is stronger in practice because the call site is
auditable and the tests verify the invariant directly. The
sub-session is one level deep (the child cannot itself invoke
`role.switch` unless its own role declares the scope, which
no role in `examples/aivyx.toml` does); multi-level nesting
is the single net-new Phase 14 deferral, with no urgency
because the no-op-by-default failure mode is already correct.
**P1.4 "each turn tagged by role active at turn-start" is
satisfied** through distinct `TurnId` values across the
parent and child `TurnStarted` audit events — no dedicated
audit tag was added (the `aivyx-audit` chain walker can
reconstruct the boundary from the role-name transition). The
clean-slate child conversation history (Q2) and per-role
memory-topic prefix (Q3) were both pinned at implementation
time. The `--print-role` debug flag gained a mechanical
"reachable role.switch targets" enumerator that reads from
the same `assemble_role_envelope`-produced `CapabilitySet`
the production dispatcher reads from, so the operator's
debug-time view of sub-session reachability is guaranteed to
agree with the runtime's dispatch-time view by construction.
See [`docs/PHASE_14.md`](PHASE_14.md) for the full phase
record.

The Mission Primitive is now the next keystone that couples
to this milestone — its approval-gate's "spawn a constrained
sub-task" pattern can compose directly against `role.switch`
without needing additional primitive work. The one remaining
sub-phase candidate for P1 itself is multi-level nesting,
which is recorded as a Phase 14 deferral and gated on a
concrete recursive-role-switching use case rather than a
forward-commitment requirement.

## Milestone — Reflection Layer ✓ (shipped across Phases 28–30)

**Forward commitments:** [`PRODUCT.md` P8](../PRODUCT.md), [`PRODUCT.md` G3](../PRODUCT.md).
**Couples to:** Outcome history exposure, runtime role mutation.

The product contract's commitment to *outcome-driven audited
reflection* is the most ambitious forward item: the agent
observes its own success/failure outcomes via the existing audit
chain and uses that history to evolve its memory and (within
guardrails) its own role config. The milestone has two halves
that may sequence as separate phases. The first half is purely
substrate: expose the audit chain as an introspection surface
the agent can read from (today it's only readable by
`--verify-only`). The second half is the reflection loop itself:
a periodic or operator-triggered pass where the agent reads
recent outcomes, proposes memory edits and role-config edits,
and surfaces them through an approval gate (likely the same
approval primitive the Mission Milestone introduces). The
reflection loop must itself be auditable — every proposed
edit, every approval/rejection, every applied change is an
audit event under the operator's chain.

**Phase 21 note (2026-04-17):** The mission gate primitive
now provides the approval-gate substrate the reflection loop's
second half needs. Gate rendering (CLI prompt, Telegram
`/approve`/`/reject`) is operational. The escalation→gate
turn-loop wiring (Phase 22 Task 8) will complete the daemon-
side orchestration, at which point the reflection loop can
compose directly against the existing gate machinery for
surfacing proposed self-modifications to the operator.

**Phase 28 (Reflection Foundation, 2026-04-18):** frozen.
First Reflection Layer primitive — `turn.history` audit
introspection tool giving the agent read access to its own
recent turn outcomes. Combined with deferral cleanup
(`mission.list`/`mission.status` tools, webhook port config,
forensic `ToolOutcome::NotInRole`). See
[`docs/PHASE_28.md`](PHASE_28.md).

**Phase 29 (Agent Reflection Loop, 2026-04-18):** frozen.
Completes the Reflection Layer — `reflection.propose` and
`reflection.apply` tools for the full observe-propose-approve-apply
cycle. Memory-only scope for this phase; runtime role-config
mutation deferred. See [`docs/PHASE_29.md`](PHASE_29.md).

**Phase 30 (Runtime Role Mutation, 2026-04-18):** frozen.
Completes P8 — `RoleOverrides` struct with prompt appendix and
allowlist add/remove, `role.update` tool and capability base,
planner factory integration reading overrides per-turn, and
extended `reflection.apply` for role mutations. Full
observe-propose-approve-apply cycle now operational for both
memory writes and runtime role-config changes. See
[`docs/PHASE_30.md`](PHASE_30.md).

**Status: G3 / P8 — Reflection Layer is now fully delivered**
across Phases 28–30.

## Milestone — Channel SDK Surface ✓ (shipped in Phase 48)

**Forward commitment:** [`PRODUCT.md` P5](../PRODUCT.md). **Couples to:**
Daemon Migration.

The product contract commits to an open first-party SDK for
building channel adapters. Today the in-tree adapters
(`LocalChannel`, `aivyx-telegram`) directly depend on
`aivyx-core` and re-implement `ChannelContext` from scratch each
time. This milestone extracts the adapter surface into a
documented, versioned, third-party-consumable form — almost
certainly as a separate crate plus a written contract document
in `docs/`. The shape of the surface is largely fixed by what
the daemon IPC protocol turns out to look like, so the milestone
is expected to land *immediately after* the Daemon Migration's
first phase and may share a phase with it if the daemon work
turns out to be lighter than expected.

## Milestone — Tool Process IPC ✓ (shipped in Phases 49–50, hardened in 52)

**Forward commitment:** [`PRODUCT.md` P12](../PRODUCT.md). **Couples to:**
Daemon Migration, Channel SDK Surface.

The product contract commits to running tools as separate
processes that speak an IPC protocol matching the channel
adapter protocol. Today's tools are in-process trait
implementations. This milestone introduces the tool-process
IPC contract — likely the same wire format as the channel SDK,
since both are "external code that talks to the daemon under a
declared capability envelope." The substrate tools (the seven
in **P10**) move into the daemon process itself or into a
trusted in-process registry; the third-party tools become
separate processes the daemon spawns under a specific role's
envelope. The load-bearing decision is whether to fold the
tool IPC into the channel SDK protocol (one wire format, two
client roles) or to keep them separate (two protocols, easier
to evolve independently). Deferred to milestone open.

## Milestone — SDK Documentation Surface ✓ (shipped in Phases 48–49)

**Forward commitment:** [`PRODUCT.md` P11](../PRODUCT.md).

A docs-and-examples phase after the in-tree SDK has stabilized.
The product contract commits to publishing the SDK as a
versioned interface with worked examples and an integration
test surface a third-party developer can run against their own
adapter. This milestone is the lightest on the roadmap: it
ships no new substrate and revises no architecture, it just
takes the in-tree adapter and tool surfaces (after the Channel
SDK and Tool Process IPC milestones have landed) and dresses
them as a publishable contract. Expected to be one phase, late
in the sequence.

## Milestone — MCP Integration ✓

**Forward commitment candidate:** not yet locked (requires
product-shape review). **Couples to:** P11 (SDK Contract),
P12 (Tool Process IPC), Daemon Migration.

The `Tool` trait's shape (`name`, `description`,
`input_schema`, `required_scope`, `execute`) maps near-1:1
to MCP's tool interface. This milestone adds an MCP client
adapter that bridges external MCP servers into Aivyx's tool
registry. Each MCP tool gets a declared scope in the
capability system, audit logging as a standard tool call,
and role allowlisting through the existing config surface.
The load-bearing design decision — bridge category vs P12
process model — was resolved in Phase 23 in favor of bridge
(in-process `McpToolProxy` delegating over stdio JSON-RPC).

**Phase 23 (MCP Foundation, 2026-04-17):** new `aivyx-mcp`
crate (11th workspace member), stdio transport,
`McpServerBridge` + `McpToolProxy`, `mcp.call` scope base
in `aivyx-capability`. 8 tests. Programmatic API only.

**Phase 24 (Config + Binary Wiring, 2026-04-17):**
`[[mcp_server]]` TOML config entries, daemon-side bridge
lifecycle (eager startup, tool registration, `kill_on_drop`
shutdown), workspace layout amendment addendum, `--mcp-server`
CLI flag. Closed the MCP config surface deferral. 9 tests.

**Phase 32 (MCP SSE Transport, 2026-04-19):**
`McpTransport` trait abstraction, `StdioTransport` extraction,
`SseTransport` over HTTP+SSE with background event reader,
`McpTransportKind` config enum, `--mcp-sse` CLI flag. Closes the
Phase 23 SSE deferral. 23 new tests (9 parser, 8 config/CLI,
6 integration). Milestone fully delivered.

## Milestone — Multi-Provider Support ✓

**Delivered in Phase 25** (2026-04-17). **Coupled to:**
`LlmProvider` trait (Phase 1).

The `LlmProvider` trait was already provider-agnostic. Phase 25
added an OpenAI-compatible adapter covering GPT-4, Ollama, and
any OpenAI-API-compatible endpoint. Capability differences
between providers (tool-calling wire format) are handled
internally by the adapter — no core type changes were needed.
Delivered in 1 phase as predicted.

## Milestone — Web UI Channel ✓ (shipped across Phases 39, 47)

**Forward commitment candidate:** not yet locked. **Couples
to:** P4 (Daemon), P5 (Channel SDK).

A `127.0.0.1`-only web interface that connects to the daemon
over the existing IPC protocol. The daemon architecture makes
this cheap — the frontend is a thin client rendering
`StreamEventPayload` events. First phase: minimal chat UI
with the same rendering as the CLI REPL. Second phase: mission
management, gate resolution buttons, audit inspection surface.
The `FrontendType` enum already has the extension point
(`FrontendType::Web`).

**Phase 39 (Chat Interface, 2026-04-20):** delivered Phase 1
of the Web UI Channel. `FrontendType::Web` variant,
`WebDaemonChannel` stub, `tokio-tungstenite` WebSocket bridge
(TCP peek routing, no hyper in WS path), embedded HTML/CSS/JS
frontend with streaming text, collapsible tool-call cards,
approval-gate Approve/Deny buttons, cancel button.
`--web-ui` CLI flag and `[daemon] web_ui` / `web_ui_port`
TOML config. 801 tests (+13), zero clippy warnings. All three
byte-level streaks intact (DESIGN.md 16, PRODUCT.md 2, lib.rs 3).

## Milestone — Scheduled Execution ✓ (shipped across Phases 26–27)

**Forward commitment:** [`PRODUCT.md` G5](../PRODUCT.md).
**Couples to:** Daemon Migration, Mission Primitive.

G5 commits to autonomous and scheduled execution. The daemon
substrate exists and missions survive restarts. This milestone
adds cron-like timer primitives, webhook trigger endpoints
(localhost-only per P6), and file-change watchers. Each
trigger creates a daemon turn attributed to the operator's
identity.

**Phase 26 (Timer Primitives, 2026-04-17):** delivered cron
scheduler — `KeyDomain::Schedules`, `ScheduleRecord` CRUD,
`[[schedule]]` TOML config, daemon scheduler loop with
adaptive-tick and dedup, four agent tools (`schedule.create`,
`.list`, `.delete`, `.update`) gated to `CEILING_TRUSTED`.
660 tests. All byte-identity streaks held. See
[`docs/PHASE_26.md`](PHASE_26.md).

**Phase 27 (Webhook Triggers + File Watchers, 2026-04-18):**
completed G5. Webhook HTTP listener (localhost-only, hyper,
127.0.0.1:7842), file-change watcher (`notify` crate,
cross-platform, per-watch debounce), `TriggerDispatch`
unification with shared turn-lock, opt-in `wrap_mission`
on all trigger configs closing the Phase 26 deferral.
Nine storage domains. +30 tests (660→690). See
[`docs/PHASE_27.md`](PHASE_27.md).

**Status: G5 — Autonomous and Scheduled Execution is now
fully delivered** across Phases 26–27.

## Milestone — Assistant Profile ✓ (delivered across Phases 57–58)

**Forward commitment:** PRODUCT.md P13 (amendment A9, Phase 56).
**Couples to:** Per-Role Envelope (P9), `aivyx init` wizard,
system-prompt assembly.
**Status:** Fully delivered. P13 closed at Phase 58 exit
(2026-05-12).

The operator's stated vision (post-Phase-55) reframes Aivyx
from "personal autonomous agent platform" into a
*self-learning, self-improving AI-personal assistant with a
user-defined **Profile** and **Persona** based on the
end-user use-case*. The **Profile** is the operator-declared,
mostly-static layer: who this specific assistant is for,
what use cases it serves, how the operator wants it to
communicate, and the high-level constraints that flavor every
turn. It is distinct from the *role envelope* (which gates
capabilities) — Profile shapes the assistant's *identity*
where roles shape its *authority*.

The expected fields (subject to Phase 57 design):

- `name` — what the operator calls this assistant
- `operator_profile` — short paragraph about the operator
  (role, expertise, primary work context). Drives
  domain-specific language and assumed background knowledge
- `communication_style` — terse / detailed, formal /
  casual, with-source-citations / synthesized, etc.
- `primary_use_cases` — the 1–3 use-case archetypes the
  assistant is being shaped around (e.g. "Rust systems
  programming," "personal-finance analysis," "research
  synthesis")
- `preferences` — non-capability behavioral preferences
  ("prefer integration tests over mocks", "always cite
  sources when summarizing")
- `constraints` — non-capability behavioral constraints
  ("never autonomously commit code", "always confirm
  destructive shell commands")

**Substrate coupling.** Profile injects into the system
prompt at turn start (assembled alongside the role-derived
envelope description). The per-role inheritance tree from
P9 is unchanged — Profile is a peer concept that flavors
the assistant's voice and judgment across every role.
`aivyx init` extends with use-case prompts that populate
Profile fields.

**Expected phases:**

- **Phase 57 (Foundation, shipped 2026-05-12):** delivered
  the Profile substrate. `aivyx-config::Profile` struct
  with six P13-commit-5 fields; `[profile]` TOML table
  parsed at config-load time per Q1(a); `Profile::default()`
  synthesizing the Q5(b) `assistant_name = "Aivyx"` fallback
  for legacy configs; `aivyx-channel::assemble_session_prompt`
  helper composing Profile + role envelope into a labeled
  system prompt per Q3(c); wiring through both the parent
  session-build path and the role-switch child factory so
  sub-sessions inherit the same Profile section; `aivyx
  init` extended with three opt-in Profile prompts per
  Q4(c) (assistant name, primary use case, communication
  style); startup-banner row surfacing Profile provenance
  to the operator. Tests +14 across aivyx-config and
  aivyx-channel (992 → 1006). All three streak predictions
  correct: DESIGN.md → 4, PRODUCT.md → 1, lib.rs → 6
  (longest run since Phase 51's deliberate break at 6).
- **Phase 58 (Inspection, shipped 2026-05-12):** delivered
  the operator-facing CLI + Web UI surface and closed the
  P13 milestone. `aivyx profile show` reads `aivyx.toml`
  via the existing config loader path and renders the
  resolved Profile in labeled banner-style format per
  Q3(a). `aivyx profile edit` opens the `[profile]`
  section in `$EDITOR` against a tempfile and merges the
  result back via `toml_edit` surgical update per Q2(a)
  — preserves comments, whitespace, and every other
  section in `aivyx.toml`. New `CliMode::Profile(ProfileSubcommand)`
  nested enum per Q1(a). Web UI Profile pane shows live
  daemon state via a new `Query::GetProfile` IPC envelope
  + `ProfileSummary` wire-shape per Q4(a) read-only.
  Reload semantics: load-time-only per Q5(a) — edit
  prints a `aivyx daemon stop && aivyx` restart reminder
  on save. PRODUCT.md Delivery Status refreshed: P13 →
  Fully Delivered (Task 5 streak-breaker). `toml_edit =
  "0.22"` is the first new workspace crate added since
  Phase 27's `notify`. Tests +17 across aivyx-channel
  bin (parser + render + merge) and lib (IPC handler
  conversion), 1006 → 1023. All three streak predictions
  correct: DESIGN.md → 5, PRODUCT.md → broke at 2
  (Task 5, intentional), lib.rs → 7.

## Milestone — Persona ✓ (delivered across Phases 59–60)

**Forward commitment:** PRODUCT.md P14 (amendment A10, Phase 56).
**Couples to:** Profile (P13), Reflection Layer (P8), Memory
persistence (G3).
**Status:** Fully delivered. P14 closed at Phase 60 exit
(2026-05-12). After Phase 60, **P1–P14 are all fully shipped**;
the PRODUCT.md forward-commitment ledger closes.

**Persona** (operator's framing: "Soul") is the dynamic
counterpart to Profile — the evolving character-layer that
emerges from accumulated reflection-approved deltas over the
assistant's lifetime. Where Profile is operator-declared and
mostly-static, Persona is *reflection-written* and
*operator-gated*: the assistant proposes refinements to its
own learned voice via `reflection.propose`, the operator
approves them through the existing mission-gate machinery,
and the deltas accumulate into the Persona surface that
flavors future turns.

**Why "Persona" not "Soul" in contract docs.** The operator's
vision uses "Soul" as the evocative term; contract docs and
code use "Persona" for the same concept. Both refer to the
same evolving identity layer — Persona is the
contract-document spelling.

The expected fields (subject to Phase 59 design):

- `seed_pointer` — Profile name this Persona is grown from
  (Profile is the static seed; Persona is what it becomes)
- `learned_context` — accumulated facts about the operator
  and their domain that the assistant has internalized
  (proposed by reflection, gated by operator)
- `communication_adaptations` — refinements to
  Profile.communication_style learned over time (e.g.
  "operator prefers conclusion-first paragraphs", "operator
  finds three-bullet lists optimal")
- `character_traits` — emergent voice properties (e.g.
  "leans toward conservative recommendations on
  irreversible operations", "preempts ambiguity with
  clarifying questions")
- `relationship_milestones` — operator-significant events
  the assistant references for continuity (the human
  equivalent of "remember when we…")
- `delta_log` — the append-only HMAC-chained history of
  every approved Persona delta (audit-verifiable like the
  rest of the chain)

**Substrate coupling.** Persona delta proposals extend
`reflection.propose` with a new delta category; gate
threading reuses the Phase 21 / 28–30 machinery; the
effective-identity assembly at turn start composes
Profile + Persona deltas into the system-prompt voice
layer.

**Expected phases:**

- **Phase 59 (Foundation, shipped 2026-05-12):** delivered
  the Persona substrate per Q1–Q6 at sign-off. New
  `KeyDomain::Persona` (10th storage domain) parallel to
  `KeyDomain::Audit`; `PersonaDelta` struct with 10
  `PersonaDeltaCategory` variants (6 Profile-mirror +
  4 Persona-specific) and `PersonaDeltaOp` (SetScalar /
  AppendList / RemoveList); `PersonaChainLog` HMAC-SHA256
  chain primitive with a distinct genesis seed (chain-
  confusion attacks structurally rejected);
  `PersistentPersonaLog` storage wrapper (one redb row per
  signed entry, keyed by big-endian seq);
  `EffectivePersona` replay state +
  `compute_effective_persona` pure folder;
  `SharedEffectivePersona = Arc<RwLock<EffectivePersona>>`
  with `apply_delta_to_shared` helper; new
  `persona.propose` capability scope (KNOWN_BASES + 
  CEILING_TRUSTED); `reflection.propose` schema extended
  with `persona_deltas` array and `required_scope`
  escalates to `persona.propose` when the array is
  non-empty; `reflection.apply` writes approved deltas
  to the chain AND mutates the shared runtime state;
  `assemble_session_prompt` extended with an
  `Option<&EffectivePersona>` parameter and a new
  "## How I have learned to communicate" labeled
  section; binary opens the chain at startup, replays
  into shared state, registers on the apply tool, and
  composes the system prompt from Profile + Persona +
  Role for both parent and role-switch child sessions.
  Tests +29 (1023 → 1052 across the phase). All three
  streak predictions correct: DESIGN.md → 6, PRODUCT.md
  → 1, lib.rs → 7. **Per-turn freshness deferred to
  Phase 60** — Phase 59 ships snapshot-at-session-build;
  the planner-factory per-turn re-call lands alongside
  the Web UI / revert / CLI surfaces.
- **Phase 60 (Visualization, shipped 2026-05-12):** closes
  the milestone and the entire forward-commitment ledger.
  `PersonaDeltaOp::Revert { target_delta_id }` variant +
  inverse-apply folder for revert semantics including
  revert-of-revert (P14 commit 4); per-turn planner-factory
  refresh (closes Phase 59 Q5(a) deferral — approved deltas
  take effect on next turn without restart);
  `Query::GetEffectivePersona` + `Query::ListPersonaDeltas`
  IPC envelopes + `FrontendMessage::RevertPersonaDelta`;
  `aivyx persona show / list / revert` CLI subcommands
  (daemon-IPC-backed per Q3(a) at sign-off); Web UI Persona
  pane with effective-state rendering and click-to-revert.
  Reverts are operator-only per Q5(a), auto-approved (the
  operator is the proposer). Per Q4(a), revert appends a
  delta to the append-only chain rather than mutating in
  place. Identity export/import deferred to a future
  micro-phase if pressure surfaces — **fully closed:** export
  shipped in Phase 64 (2026-05-14), import shipped in Phase
  65 (2026-05-14). Operators can now transfer Profile +
  Persona between hosts via `aivyx identity export <path>` +
  `aivyx identity import <path> [--force]`. Tests +19
  (1052 → 1071).
  All three streak predictions correct: DESIGN.md → 7,
  PRODUCT.md → broke at 2 (Task 7 Delivery Status refresh,
  intentional), lib.rs → 8. After Phase 60, **P1–P14 are
  all fully shipped**. The PRODUCT.md forward-commitment
  ledger closes here.

## Milestone — Distribution (in progress, Pipeline Ready)

**Forward commitment:** none — operator-feedback-shaped
substrate-ergonomics work post-ledger-closure.
**Couples to:** `aivyx init` (Phase 44 wizard), the operator
identity layer (P13 + P14).
**Status:** Phase 1 of N delivered as "Pipeline Ready,
Publication Held." First published release pending public
hosting; future micro-phases extend reach.

After Phase 60 closed the forward-commitment ledger, the
codebase review surfaced **distribution** as the largest
adoption-shape gap: end users had to run `cargo run --release
--bin aivyx` from source because there were no prebuilt
binaries on any platform. This milestone closes that gap
incrementally — release pipeline first, then publication,
then platform expansion (Windows, Homebrew, Docker, signing),
then the long-tail polish (signed third-party tool
registries, automated update channels). Each item lands as a
focused phase rather than one monolithic distribution effort.

**Expected sub-phases / micro-phases:**

- **Phase 61 (Release Pipeline, shipped 2026-05-13 as
  "Pipeline Ready").** First phase of the milestone. Wired
  the release substrate: `aivyx --version` flag, cargo-dist
  config (`dist-workspace.toml`), four-target matrix (Linux
  x86_64/aarch64 musl + macOS x86_64/aarch64), three
  workflow files (`ci.yml`, `quality-gate.yml`, dist's
  `release.yml`), and the `plan-jobs = ["./quality-gate"]`
  wiring that gates every release-pipeline artifact behind
  `cargo clippy --workspace --all-targets -- -D warnings` +
  `cargo test --workspace`. README's "Five-minute setup"
  refresh prepared the install-script framing; mid-phase
  the operator pivoted to a VPS-private-first posture and
  the Task 5/6 fixup reframed the docs to honest
  "build-from-source primary, shell-installer when
  published." Tests +3 (1071 → 1074). All three streak
  predictions correct: DESIGN.md → 8, PRODUCT.md → 1,
  lib.rs → 9 (new record).
- **v0.1.0 publication (deferred to future micro-phase).**
  Operator-side: create public GitHub repo, push history,
  tag `v0.1.0`. The pipeline fires automatically and
  publishes prebuilt binaries + shell installer. Re-opens
  when public hosting goes live.
- **Native Windows port (future).** Daemon IPC NamedPipe
  replacement for the Unix-socket-only substrate;
  platform-conditional spawn for `shell.exec`; parallel CI
  matrix. Most architecturally heavy item on the milestone.
- **Package manager presence (future).** Homebrew tap;
  optionally crates.io publishing (12-crate namespace
  check + versioning policy); optionally Linux distro
  packages (AUR / `.deb` / `.rpm`).
- **Container distribution (future).** Docker image with
  the daemon as the entrypoint.
- **macOS signing + notarization (future, gated on Apple
  Developer account).** Removes the Gatekeeper friction at
  first launch.

## Milestone — Reach (in progress, agent-initiated outbound)

**Forward commitment:** none — operator-feedback-shaped work
on the "what does the agent reach OUT to" axis.
**Couples to:** `aivyx-telegram` channel adapter (Phase 8),
operator identity layer (P13 + P14), schedules / webhooks /
file-watchers (Phases 26–27).
**Status:** Phase 1 of N delivered as "agent-driven notify.send
+ Telegram + webhook." Future sub-phases extend the reach
surface (trigger-config sugar, new channels, system-prompt
enumeration).

The post-Phase-60 codebase review surfaced **reach** as the
next-largest adoption-shape gap after distribution: schedules,
webhooks, and file-watchers fire turns, but the agent's output
stops at the audit log — there's no path from "agent has
something to say" to "operator's phone buzzes." This milestone
gives the agent the substrate to *initiate* contact, not just
respond to it. Closing the gap is the inflection point between
"thing I talk to" and "thing that talks to me."

**Expected sub-phases / micro-phases:**

- **Phase 62 (Agent-Initiated Outbound Notifications, shipped
  2026-05-13).** First phase of the milestone. Shipped the
  `notify.send` infrastructure tool, the `[[notify_target]]`
  config surface, and two backends (Telegram + generic
  webhook). Substrate: `notify.send` capability scope
  (CEILING_TRUSTED only) with `<target_name>` qualifier,
  `NotifyDispatcher` + `NotifyBackend` trait + 5-variant
  `NotifyError` classification, `build_notify_dispatcher`
  factory consuming the loaded config. Behavior: agent calls
  `notify.send {target, message, subject?}`; on success the
  tool returns `Completed` with `{success: true,
  delivered_at}`; on delivery failure the tool returns
  `Completed` with `{success: false, error_kind,
  error_message}` so the agent gets structured retry
  guidance. Daemon wired end-to-end. Tests +65 (1074 → 1139).
  All three streak predictions correct: DESIGN.md → 9,
  PRODUCT.md → 2, lib.rs → 10 (new record). Zero new
  workspace deps.

- **System-prompt enumeration (deferred from Phase 62 Task 8).**
  The `assemble_session_prompt` extension that surfaces
  reachable target names to the agent. Originally planned for
  Phase 62 Task 8 but deferred at implementation time —
  threading the dispatcher through six call sites with proper
  per-role capability checks was scope-creepy for one phase.
  Lands as a follow-on micro-phase when adoption surfaces
  friction.

- **Phase 63 (Trigger-Config Notify Sugar, shipped
  2026-05-13).** Closes the Phase 62-deferred trigger-config
  alternative. `[[schedule]]`, `[[webhook]]`,
  `[[file_watch]]` entries each accept an optional
  `notify_target = "..."`. When the trigger fires, the daemon
  auto-dispatches the agent's final response to the named
  target after the turn completes — operator-correct subject
  `<kind>: <trigger-id>`, no agent involvement, no
  system-prompt instruction. Config-load-time capability
  validation (Q5(a)) catches role/target mismatches at startup
  rather than at 9am the next morning. Three Record types
  gain `notify_target` with `#[serde(default)]` for backward
  compat. Tests +15. All three streak predictions correct:
  DESIGN.md → 10, PRODUCT.md → 3, lib.rs → 11 (new record).
  **Q1(a) audit-event variant scope-adjusted at exit:**
  `AuditEventKind::AutoNotifyDispatched` deferred to a
  follow-on phase because `TriggerDispatch` doesn't hold an
  audit-hook reference today and wiring one in is materially
  larger than the notify hook itself. Auto-notify is
  eprintln-logged matching the existing trigger.rs patterns.

- **Email SMTP outbound (future).** Adds a `kind = "email"`
  backend. New dep (`lettre` or similar) plus SMTP config
  surface (host, port, auth). One worked example covering
  the common case (Fastmail, ProtonMail, Gmail app password).

- **Web UI desktop notifications (future).** Adds a
  `kind = "web-ui"` backend that pushes a WebSocket
  notification to the running Web UI session, surfaced via
  the browser's `Notification` API. Needs WebPush
  registration or aggressive polling.

- **Slack-flavored webhook payload (future, gated on use).**
  Slack incoming webhooks expect `{text: ...}` not
  `{message: ...}` — a real shape mismatch. Lands as a
  separate `kind = "slack-webhook"` if Slack-using
  operators surface.

- **Agent-initiated proactive Telegram session (future,
  speculative).** Today's Telegram adapter reacts to inbound
  messages. A future evolution would let the agent open a
  session-of-its-own-initiative — e.g. on a schedule-fired
  turn that wants to start a multi-turn conversation, not
  just push a single notification. Architecturally heavier;
  not on the current sub-phase ladder.

## Sequencing notes (revised at Phase 56 sign-off, 2026-05-12)

Through Phase 55, every PRODUCT.md commitment (P1–P12) is
delivered. Chapter A (Phases 50–54) closed Foundation
Closeout. Phase 55 demonstrated the post-Chapter-A posture
by porting the Phase 52 sandbox pattern to `[[mcp_server]]`
in response to THREAT_MODEL §5.2.

**Forward arc — Profile + Persona (Phases 56–60).** The
operator's stated vision (self-learning, self-improving
AI-personal assistant with a user-defined Profile and
Persona) reframes the project's next phase block. Phase 56
files the P13 + P14 amendments and updates PRODUCT.md (docs
only, no code). Phases 57–58 deliver the Profile half.
Phases 59–60 deliver the Persona half. After Phase 60, the
project will have a full operator-declared + agent-learned
identity layer composed against the existing
substrate.

For the per-phase narrative including Chapter A, see
[`ROADMAP.md`](ROADMAP.md). For the current implementation
state of each P1–P12 commitment, see the Delivery Status
section in [`../PRODUCT.md`](../PRODUCT.md).

### Other forward work (operator-feedback-shaped)

Beyond the Profile + Persona arc, future phases continue to
work against:

- **Hardening & operator-facing polish** — items that have not
  yet shown enough pressure to be milestones. Examples:
  container-sandbox profile presets, audit log rotation if
  chain size becomes load-bearing, additional channel adapters
  (Matrix / Signal / Discord), additional tool-process
  examples, signed third-party tool registries.
- **Strategic conversations** — anything that would amend
  DESIGN.md or PRODUCT.md goes through the amendment process
  before opening a phase. Seven amendments stand as the
  precedent (Phase 56 will file the eighth and ninth).
- **Operator feedback loops** — at this point the project
  benefits more from real-world operator use than from
  speculative forward work outside the Profile + Persona arc.

## Delivered

Every product-shape milestone shipped across the 54-phase arc.
Grouped by commitment for traceability.

### `PRODUCT.md` commitments

- **P1 — Sub-Agent Role-Switching** — Phases 14, 33. Inline
  sub-session nesting via an `OnceLock`-backed `RoleSwitchTool`
  factory; multi-level nesting closed in Phase 33. Structural
  impossibility of escalation pinned by integration tests +
  `--print-role` reachable-targets enumerator.

- **P2 — Mission Primitive** — Phases 21, 23, 28, 35. Six-state
  machine, `mission.create` / `mission.gate` capability bases,
  IPC extensions, CLI + Telegram gate prompts, escalation → gate
  turn-loop wiring, `mission.list` / `mission.status` inspection.

- **P3 — Goals and Non-Goals** — all 7 goal commitments (G1–G7)
  shipped, see "Goal commitments" subsection below.

- **P4 — Daemon-Default Architecture** — Phases 16–20. Protocol
  settlement, production hardening, REPL wiring, multi-connection
  + Telegram port, daemon management.

- **P5 — Open First-Party Channel Surface** — Phase 48. v0
  `CHANNEL_SDK.md` + `examples/python-channel/` + 15-test
  conformance suite.

- **P6 — OS-Level Operator Identity** — always true by
  construction. IPC socket mode 0600 owned by operator UID.

- **P7 — Single-Inheritance Role Tree** — Phases 11, 13. Strict
  attenuation along every dimension, validated at config load.

- **P8 — Outcome-Driven Audited Reflection** — Phases 28–30.
  Audit introspection (`turn.history`), reflection loop
  (`reflection.propose` / `reflection.apply`), runtime role
  mutation (`role.update` + planner factory integration).

- **P9 — Per-Role Full Capability Declaration** — Phase 13.
  `capability_scopes` parsed via `Scope::parse` at config load.

- **P10 — Substrate-Only Core, Eight Tools Forever** — true by
  contract. `web.post` added in Phase 37, A5 amendment Phase 38.

- **P11 — SDK Contract** — Phases 48 (channel half) + 49 (tool
  half). `docs/CHANNEL_SDK.md` + `docs/TOOL_SDK.md` v0 contracts.

- **P12 — Tools as Separate Processes Over Daemon IPC** —
  Phase 49 foundation + Phase 50 closeout + Phase 52 sandbox
  hardening. `aivyx-tool` crate, `ToolProcessBridge`,
  `ToolProxy`, `run_tool_as_subprocess<T: Tool>` proven
  equivalent in `p12_equivalence.rs`, optional
  `[tool_process.sandbox]` wrapper layer.

### Goal commitments (P3)

- **G1 — Web interaction** — `web.fetch` (Phase 12), `web.post`
  (Phase 37), binary body + redirects (Phase 37). Rich web
  interaction (page rendering) not pursued — `web_search` and
  `web_read` bundled MCP tools cover the operator-useful subset.
- **G2 — Code interaction** — `fs.read`, `fs.write`,
  `shell.exec` operational with role gating; shell hardened
  in Phase 42.
- **G3 — Memory Reflection** — Phases 28–30. Memory substrate,
  reflection loop, runtime role mutation.
- **G4 — Sub-agent orchestration** — Phases 14, 33. Multi-level
  role-switching with capability attenuation.
- **G5 — Autonomous and scheduled execution** — Phases 26–27.
  Cron schedules, webhooks, file watchers, trigger unification,
  mission wrapping.
- **G6 — Local execution, privacy non-negotiable** — Phase 34
  (Ollama first-class with health check). No hosted control
  plane at any point.
- **G7 — Third-party tool SDK** — Phases 48, 49. v0 channel SDK
  + v0 tool SDK + MCP integration (Phases 23–24, 32) as a
  parallel adapter.

### Cross-cutting initiatives

- **Daemon Migration** — Phases 16–20 (architecturally
  complete). Five-phase keystone that reshaped the binary
  lifecycle and unlocked P5, P12, P2, P1.
- **Role-Config Migration** — Phase 13. Per-role envelope,
  single-inheritance.
- **MCP Integration** — Phases 23–24, 32. `aivyx-mcp` crate,
  stdio + SSE transports, daemon-side bridge lifecycle.
- **Multi-Provider Support** — Phases 25, 34. Anthropic,
  OpenAI-compatible, Ollama-first-class.
- **Web UI Channel** — Phases 39, 47. Localhost-only chat
  surface + mission/audit/sessions inspection panes.
- **Bundled MCP Web Search** — Phase 46. `aivyx mcp-server
  web-search` with Brave / SerpAPI / DuckDuckGo backend chain.
- **Rich Input (Multimodal)** — Phase 45. `ContentBlock`,
  image input across Anthropic / OpenAI / Telegram / Web UI.
- **`aivyx init` Wizard** — Phase 44. First-run interactive
  setup, Ollama auto-detection.

### Chapter A — Foundation Closeout (Phases 50–54)

The arc that brought every loose end from Phases 0–49 to a
close.

- **Phase 50 — P12 closeout.** Wired the two Phase 49 deferred
  bridge stubs (`ToolEvent` channel relay, per-call cancellation).
  Added `run_tool_as_subprocess<T: Tool>` harness. Proved
  "extractable without rewriting" with `p12_equivalence.rs`.
- **Phase 51 — Cleanup.** Closed three pre-existing items:
  `AivyxError::{Storage,Crypto}` typed nested errors (D6
  honored after 50 phases), `handle_connection` parameter-struct
  lift, `AIVYX_PASSPHRASE` TOML/env footgun (TOML path now
  drives derivation).
- **Phase 52 — Sandbox layer.** Generic command-wrapper
  sandbox for `[[tool_process]]`. Operator supplies the policy
  (bubblewrap / firejail / Docker / sandbox-exec). Narrowed
  THREAT_MODEL.md §5.6.
- **Phase 53 — (skipped).** Audit log rotation deferred
  indefinitely; chain growth is bounded by tool-call frequency
  × uptime.
- **Phase 54 — Final docs sweep.** Brought the docs back in
  sync with the substrate. Root README rewrite, this
  PRODUCT_ROADMAP refresh, DAEMON_IPC Phase 47 addendum, A3
  scope-base count addendum, walkthrough refresh, cross-doc
  consistency spot-check.

After Phase 54 the project sits at: 984 Rust tests + 24 Python
conformance tests passing, zero clippy warnings, 12 workspace
crates, 43 capability scope bases, 9 encrypted storage domains,
7 contract amendments filed, 4 deferrals carried forward (none
load-bearing).

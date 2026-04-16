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

## Milestone — Daemon Migration

**Forward commitment:** [`PRODUCT.md` P4](../PRODUCT.md). **Keystone:** unlocks
P5, P12, P2, P1.

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

## Milestone — Mission Primitive

**Forward commitment:** [`PRODUCT.md` P2](../PRODUCT.md). **Couples to:**
Daemon Migration, Role-Config Migration.

The product contract distinguishes *bounded tasks* (single-turn,
success defined by completion) from *open-ended missions*
(multi-turn, success defined by an approval gate the operator
issues somewhere along the way). Today's foundation only has
bounded tasks; the mission milestone introduces the long-running
work item as a first-class primitive — one that survives across
process restarts (which means it sits on top of the Daemon
Migration), runs under a specific role's envelope (which means
it sits on top of the Role-Config Migration), and emits
operator-visible approval gates as `StreamEvent`s the channel
adapter renders distinctively. The shape of the approval gate
itself (button? text command? structured tool call?) is the
load-bearing question and is deliberately deferred to milestone
open.

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

## Milestone — Reflection Layer

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

## Milestone — Channel SDK Surface

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

## Milestone — Tool Process IPC

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

## Milestone — SDK Documentation Surface

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

## Sequencing notes (subject to revision)

- **Daemon Migration is the first keystone.** Almost everything
  else couples to it. The first product-shape phase after
  Phase 13 is most likely a Daemon Migration phase.
- **Role-Config Migration is the second keystone.** It can run
  in parallel with the Daemon Migration only if the two
  milestones don't both touch the same files — likely they will,
  so they probably sequence rather than parallelize.
- **Reflection is the most ambitious milestone and the most
  likely to slip.** It depends on outcome history exposure plus
  the approval-gate primitive plus runtime role mutation, all
  of which are themselves forward commitments. Expect Reflection
  to land last and to take more than one phase.
- **The SDK Documentation milestone is deliberately last.** No
  point publishing a contract until the in-tree adopters have
  shaken it out.

## Delivered

- **Role-Config Migration** — shipped in Phase 13
  (2026-04-15, exit commit `25a09de`). Per-role
  capability envelope in a single TOML file with
  single-inheritance, declared-set attenuation, worked
  example, and `--print-role` debug flag. Delivered
  **PRODUCT.md P9 — Per-Role Full Capability
  Declaration** in full. (Backfilled into this section
  by Phase 14's exit freeze; the Phase 13 freeze did
  not populate the Delivered section, recorded as a
  Phase 13 oversight rather than a Phase 14 scope
  expansion.)
- **Sub-Agent Role-Switching** — shipped in Phase 14
  (2026-04-16, exit commit TBD-backfilled). Inline
  sub-session nesting via an `OnceLock`-backed
  `RoleSwitchTool` factory closure, one level deep,
  with structural-impossibility-of-escalation pinned
  by integration tests against narrowed-caps child
  snapshots and by the `--print-role` reachable-
  targets enumerator reading from the same envelope
  source as the production dispatcher. Delivered the
  in-process portion of **PRODUCT.md P1 — Sub-Agent
  Mode via Role-Switching**. Multi-level nesting is
  the single net-new Phase 14 deferral, low-urgency.

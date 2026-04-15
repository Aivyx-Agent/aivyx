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
to the running daemon over a local IPC socket. The migration is
expected to span one or more dedicated phases — the first phase
likely lands the daemon process and a single auto-attaching
frontend (probably `LocalChannel`), with subsequent phases
porting `aivyx-telegram` and any other shipped adapters. The
shape of the IPC protocol is the load-bearing design decision
this milestone has to settle, because P5 (channel SDK) and P12
(tool IPC) both consume it.

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

## Milestone — Sub-Agent Role-Switching

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

(Empty until the first product-shape milestone ships.)

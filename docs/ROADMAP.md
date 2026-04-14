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

## Phase 5 — Encrypted Storage

**Status:** Active — see [`PHASE_5.md`](PHASE_5.md).

## Phase 6 — Memory as Tool

Implement `aivyx-memory` with `memory.read`, `memory.write`, and
`memory.forget` as real tools. First agent that recalls across
turns via the lazy-recall contract from D1 — memory is *a tool
the agent chooses to call*, not ambient context injected at turn
start.

This closes the loop on D1's core commitment: every memory
access is an explicit, scope-checked, audited tool call. If this
phase works, Aivyx has structurally prevented the "hidden memory
injection" class of bug by construction. Phase 6 is also the
deadline for re-evaluating Phase 4's Q1: by then `aivyx-core`
will hold filesystem tools *and* memory tools, and the choice
between "keep piling into `aivyx-core::tools`" and "spin up an
`aivyx-tools` umbrella crate" can be made against two concrete
data points instead of one.

## Phase 7+ — Ecosystem

Unplanned and intentionally so. Remote channels (Telegram, Discord,
Slack, Matrix, Email), desktop GUI, federation, multi-agent — none of
it is scoped until Phases 1–6 establish what the agent actually needs
from the outside world. The lesson from the archived codebase is that
ecosystem work started too early and shaped the core in ways that
later became drift markers. This time the core gets to stabilize
first.

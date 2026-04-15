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

## Phase 13 — Role-Config Migration (first product-shape keystone)

**Active — see [PHASE_13.md](PHASE_13.md).** Opened 2026-04-15.
Phase 13 is the first phase written under the dual-contract
regime (DESIGN.md + PRODUCT.md, both LOCKED), and delivers the
first numbered Product Commitment directly: **P9 — Per-Role
Full Capability Declaration**. The goal is to move every
capability grant currently inlined in
`crates/aivyx-channel/src/bin/aivyx.rs:907–934` into per-role
config — each role declaring its complete envelope
(`capability_scopes`, `trust_ceiling`, `parent_role` per
**P7**'s single-inheritance rule). Phase 13 is a
config-substrate phase, not a capability-layer rewrite; the
existing capability layer gets *consumed* by the new config
fields rather than modified. Ships a worked-example
`examples/aivyx.toml` in the same phase to close the Phase 12
Task 3 deferral and prove the inheritance primitive against a
non-trivial multi-level role tree. Task count: 5, same cadence
as Phase 11 and 12.

## Phase 14 — shape TBD at Phase 13 exit

The Phase 11 + 12 + 13 triplet will have validated three
distinct phase shapes: product phase, product phase again,
config-substrate phase. Phase 14's shape is decided at Phase
13 exit informed by what Phase 13 uncovered. Candidates from
`PRODUCT_ROADMAP.md`: **Daemon Migration keystone start**
(the largest forward reshape, now unblocked because Role-
Config Migration has landed first), **Mission Primitive**
(couples to both Daemon Migration and Role-Config
Migration, smaller if only the approval-gate `StreamEvent`
lands), or **Sub-Agent Role-Switching** (smallest of the
three, consumes Role-Config Migration directly). The backlog
still carries seven items after Phase 13 closes the
`default role config file` deferral, so a consolidation
sub-phase is also possible if the foundation backlog grows
sharper design pressure during Phase 13 than a keystone
does.

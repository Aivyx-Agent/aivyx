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

## Phase 14 — Sub-Agent Role-Switching (first P1 delivery)

**Active — see [PHASE_14.md](PHASE_14.md).** Opened
2026-04-15. Phase 14 is the first phase to deliver on
**PRODUCT.md P1 — Sub-Agent Mode via Role-Switching**. It
consumes the per-role capability envelope substrate Phase
13 shipped (`assemble_role_envelope`, the worked example,
`--print-role`) by wiring a real second caller into it:
the turn loop. Task 1 lifts `assemble_role_envelope` from
the `aivyx-channel` binary into its lib (closing the first
net-new Phase 13 deferral). Task 2 adds a `role.switch`
capability scope with a `target-role` qualifier and
registers a `role.switch` tool. Task 3 integrates the tool
with the session layer via **sub-session nesting** — the
child opens, runs to completion under its own envelope,
and stack-pops back to the parent, respecting the
`Agent::turn(&self, ...)` immutability invariant and the
P1.3 "structural impossibility of escalation" rule. Task 4
is the working-session slot; Task 5 is exit freeze with
optional cleanup lifting `render_role_envelope` + helpers.
The production-core `aivyx-core/src/lib.rs` byte-identity
streak is aspirationally preserved (would extend to three
consecutive phases); the fallback path if inline sub-
session nesting hits a re-entrancy wall is one additive
`TurnOutcome::SwitchRoleRequested` variant, same shape as
Phase 12 Task 1's streak break.

## Phase 15 — shape TBD at Phase 14 exit

Phase 14's clean exit and whatever it uncovers will shape
Phase 15. Candidates from `PRODUCT_ROADMAP.md` remain
**Daemon Migration keystone start** (unblocked by Phase
13, still the largest forward reshape), **Mission
Primitive** (approval-gate half without durability, or
full with daemon coupling), or a **consolidation sub-
phase** if the foundation backlog grows sharper design
pressure during Phase 14 than a keystone does. Phase 14's
P1 delivery makes Sub-Agent Role-Switching a resolved
milestone in the PRODUCT_ROADMAP — the Mission Primitive
is the next keystone that couples to it.

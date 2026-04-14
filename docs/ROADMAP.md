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

## Phase 7 — Hardening (audit persistence first)

**Leaning:** Hardening over Ecosystem, based on what Phase 6 uncovered.

Phase 6 shipped memory-as-tool cleanly (`serde_json` encoding,
per-topic monotonic sequence numbers, a `RedbMemory` substrate that
seeds its counter from on-disk state at reopen, three `Tool` impls
that derive `memory.<op>:topic:<topic>` scopes from their input, and
a `memory_tool_e2e.rs` integration test that proves recall crosses a
process boundary). The asymmetry it surfaced is the one that decides
Phase 7: **memory now survives restarts, but the audit of how memory
was written does not**. `HmacChainLog` still resets on every process
start, so every `AuditEvent::MemoryAccess` tag Phase 6 just made
load-bearing for D1's "memory is a tool" commitment is ephemeral —
an attacker who can crash the process once can truncate the chain.
That's the strongest hardening case the project has had so far, and
it's the thing to fix first in Phase 7.

Concrete Phase 7 candidates, ordered by Phase 6's evidence:

1. **Audit persistence via `KeyDomain::Audit`.** Load-bearing.
   Requires deciding where the HMAC chain key comes from (derive
   from passphrase? separate key in `KeyDomain::Secrets`?), what
   "chain start" means across restarts (one chain forever? one
   chain per session? merkle-linked session chains?), and how to
   verify the chain from a cold start. This is real design work,
   not just plumbing — the right shape is its own PHASE_7.md with
   open questions at entry.
2. **Interactive passphrase prompting.** ~20 lines of `rpassword`
   once audit persistence decides how the chain key is sourced
   (the two decisions touch the same surface). Still env-var-only
   today.
3. **Memory GC / TTL / size caps.** Phase 6 shipped an unbounded
   substrate and explicitly deferred eviction. If Phase 7 is doing
   hardening work, this is the right place for `memory.forget`-
   driven compaction and a size-cap tripwire.
4. **Filesystem permission hardening (`chmod 0600` on the store
   and its salt sidecar).** Small, mechanical, but a real
   disclosure hazard on a shared Unix box. Half an hour of work.

The **Ecosystem** framing (remote channels — Telegram, Discord,
Slack, Matrix, Email; desktop GUI; federation; multi-agent) remains
the longer-term destination. The lesson from the archived codebase
is that ecosystem work started too early and shaped the core in
ways that later became drift markers, so this time we want the core
to be *stable* before we go there. Phase 6 leaves the core in
exactly the shape Phase 7 hardening needs — session-level
persistence, memory-level persistence, and an audit surface that is
*almost* persistent — and Phase 7 is the phase that closes the gap.

Ecosystem is likely Phase 8 or 9, not Phase 7. The decision lands
firmly at Phase 7 entry.

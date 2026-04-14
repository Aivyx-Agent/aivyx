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

## Phase 6 — Memory as Tool

**Status:** Active — see [`PHASE_6.md`](PHASE_6.md).

## Phase 7 — (undecided: Hardening vs. Ecosystem)

Deliberately ambiguous until Phase 6 closes. Phase 5 left a short
hardening list that will be load-bearing the first time Aivyx is
used somewhere other than a developer laptop: **audit persistence**
(wire `HmacChainLog` into `KeyDomain::Audit` so the chain survives a
restart — right now a crash erases every audit entry), **interactive
passphrase prompting** (the binary still only reads
`AIVYX_PASSPHRASE`, which is fine for CI and painful for humans),
**memory GC** (size caps / TTL on `KeyDomain::Memory`, once Phase 6
proves the substrate), and **filesystem permission hardening** on
the store sidecar files (no explicit `chmod 600` today). None of
these are hard, but together they're the difference between "works"
and "safe to hand to a non-author."

The alternative framing is **Ecosystem** — remote channels
(Telegram, Discord, Slack, Matrix, Email), desktop GUI, federation,
multi-agent. That was the original Phase 7+ placeholder. The lesson
from the archived codebase is that ecosystem work started too early
and shaped the core in ways that later became drift markers, so
this time we want the core to be *stable* before we go there. The
Phase 6 exit is the moment we decide which direction Phase 7 takes,
based on concrete evidence: if Phase 6 surfaced a painful gap in
audit/GC/passphrase UX, hardening comes first; if Phase 6 runs
cleanly and the obvious next question is "how do I talk to this
agent from my phone," ecosystem comes first.

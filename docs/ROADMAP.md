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

**Status:** Active — see [`PHASE_7.md`](PHASE_7.md).

## Phase 8 — Ecosystem (remote channels)

Deliberately ambiguous until Phase 7 closes. With the core fully
hardened (persistent audit, interactive passphrase, memory size
caps, filesystem permission mode), Phase 8 is the first phase where
ecosystem work — remote channels (Telegram, Discord, Slack, Matrix,
Email), desktop GUI, federation, multi-agent — becomes a responsible
target rather than a shortcut past unfinished plumbing. The D2
`ChannelContext` trait and the trust-tier ladder (`Local` → `Trusted`
→ `Untrusted`) have been waiting for their second concrete adapter
since Phase 3 shipped `LocalChannel`; Phase 8 is where that second
adapter finally lands.

The first concrete Phase 8 candidate is probably **Telegram**,
chosen because it's the simplest credible non-local channel
(long-poll or webhook, one auth token, small message model) and
because its trust-tier story is unambiguous — a Telegram bot is
`Untrusted` by default and the capability attenuation falls
naturally out of D4's existing tier table. Matrix is a more
principled choice but has a larger protocol surface; Discord and
Slack have the best UX but need OAuth flows that Phase 8 shouldn't
be the one to invent. Phase 8's entry will finalize this decision
based on which adapter exercises the `ChannelContext` trait most
completely.

What Phase 7's hardening earns Phase 8: a core that can be handed
to a network-facing adapter without the adapter inheriting any of
the "works on a developer laptop" assumptions. Persistent audit
means a bad message over an untrusted channel is still in the log
tomorrow. Memory size caps mean an attacker who floods the agent
with requests to "remember X" can't unbounded-allocate. Interactive
passphrase means the adapter can't be the one to handle the
master key. These are all "the core had to get this right before
the adapter could exist" items, and Phase 7 is the last phase that
gets to fix them without also having a running bot to migrate.

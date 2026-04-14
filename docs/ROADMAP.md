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

## Phase 8 — Ecosystem: Telegram adapter

**Leaning:** Telegram as the first non-local channel, on the back
of Phase 7's hardening work.

Phase 7 shipped exactly the core surface a network-facing adapter
needs: **persistent audit** (a bad message over an untrusted
channel is still in the log tomorrow — `verify_from_disk` can
reconstruct the full history across any restart), **memory size
caps** (an attacker who floods the bot with "remember X" hits the
per-topic tripwire instead of unbounded-allocating),
**`chmod 0600` on the store file** (a shared Unix host can't read
another aivyx user's session state), and **interactive passphrase
prompting** (so the adapter process itself can't be the one that
decides how to get the master key — it has to route through a
human or env var at startup, not a network handler). All four of
those are things "the core had to get right before the adapter
could exist," and Phase 7 is the phase that finished fixing them.

The D2 `ChannelContext` trait and the trust-tier ladder
(`Local` → `Trusted` → `Untrusted`) have been waiting for their
second concrete adapter since Phase 3 shipped `LocalChannel`.
Phase 8 is where that second adapter finally lands, and where the
trust-tier ladder's second rung gets exercised end-to-end.

**Why Telegram first over Matrix / Discord / Slack:** Telegram is
the simplest credible non-local channel (long-poll or webhook, one
auth token, small message model) and its trust-tier story is
unambiguous — a Telegram bot is `ChannelTrustTier::Untrusted` by
default and D4's capability attenuation falls naturally out of the
existing tier table. Matrix is a more principled choice but has a
larger protocol surface (federation, device verification, encrypted
rooms) that would pull focus from the adapter pattern work.
Discord and Slack have better UX but need OAuth flows that Phase 8
shouldn't be the one to invent. Matrix / Discord / Slack adapters
are explicit **Phase 9+** candidates, built on whatever shape
Phase 8 hammers out for the *first* real `ChannelContext` impl.

What Phase 8 specifically has to figure out (these are the open
questions that will become PHASE_8.md's entry-time Q list):

1. **Where does the Telegram bot token live?** Env var
   (`AIVYX_TELEGRAM_TOKEN`), a row under `KeyDomain::Secrets`, or
   a flag passed at startup? Each has a different recovery story
   if the token leaks.
2. **Per-chat session identity.** One aivyx store per bot, or one
   store shared across all chats with `session:<chat_id>`
   qualifiers on memory/audit events? This is the first phase
   where "multi-user, one process" is a real shape, and it
   determines whether the Phase 7 "session-scoped memory
   qualifiers" deferral lands in Phase 8 or slips again.
3. **Scope attenuation at the channel boundary.** An `Untrusted`
   channel must narrow the capability set before the turn loop
   sees the request. Where does the narrowing live — at the
   adapter, at the `ChannelContext` trait, or at a new
   `TrustTierPolicy` helper? D4's tier table specifies the
   *ratios* but not the *mechanism*.
4. **Long-poll vs webhook.** Long-poll is simpler for dev boxes
   and CI; webhook is the real-world deployment shape. Start
   with long-poll, or design for webhook from day one?
5. **Turn cancellation across the network.** Phase 3's wall-clock
   cancellation assumes `stdin` as the cancel signal. A Telegram
   turn doesn't have `stdin`; what maps to `ctrl-C`?

Full task breakdown, entry criteria, and open-question resolutions
live in PHASE_8.md when that scaffolds at Phase 8 open.

## Phase 9+ — second-adapter surface + whatever Phase 8 uncovers

Deliberately ambiguous until Phase 8 closes. Candidates:
**Matrix adapter** (the federation story, encrypted rooms),
**desktop GUI** (the other end of the trust-tier ladder —
`Trusted` instead of `Untrusted`), and any **`aivyx-config`**
work that Phase 8's multi-source secrets pulls forward. The
pattern from Phases 5 → 6 → 7 is that each phase's exit reveals
the sharp edges on the *next* phase's placeholder; Phase 9's
entry will refine this list based on what the Telegram adapter
taught us about the trust-tier and channel-context contracts.

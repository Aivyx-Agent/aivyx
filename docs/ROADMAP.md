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

## Phase 9 — refinements on the second-adapter pattern

**Status:** Planned — Phase 8 closed 2026-04-14. Entry doc will
scaffold at Phase 9 open from this roadmap entry.

Phase 8 shipped `aivyx-telegram` as the second concrete
`ChannelContext`, proving that the D2 trait and the D4 trust-tier
ladder generalize beyond `LocalChannel`. Phase 9's job is to
consolidate what Phase 8 learned about the **adapter seam** — the
set of design choices Phase 8 made under time pressure that are
worth revisiting now that there's a second data point — and to
exercise the pattern on at least one additional axis. The goal is
**not** "ship a third adapter" as a headline feature; the goal is
"make sure the second-adapter pattern is the *right* pattern
before a third one commits to it."

**What Phase 8 taught us about the adapter seam** (refined here
rather than in PHASE_8.md to keep the phase doc frozen):

- **Sibling `run_*_session` functions, not a shared trait.** Phase 8
  Task 4 considered generalizing `aivyx_channel::run_session` to
  take `&dyn ChannelContext` + an abstract input-source trait so
  local + Telegram could share one function. We rejected that for
  Phase 8 — the local and Telegram lifecycles differ enough
  (pulled line-by-line vs. pushed long-poll batches) that
  shoehorning them into one trait would invent an abstraction with
  two implementations. The sibling pattern ships ~100 lines of
  "duplicated" wiring per adapter and stays legible. **Phase 9
  question:** does the pattern still hold with a third adapter,
  or does the extraction finally earn its keep?
- **Per-channel transport ownership.** `TelegramChannel` owns its
  `Arc<dyn TelegramTransport>` and the session driver reaches
  through the channel to get it, rather than the transport living
  at the binary level. This lets the scripted-transport unit tests
  in `src/tests.rs` drive the full session function without any
  HTTP stack, which was the single most valuable test-architecture
  decision of Phase 8. **Phase 9 question:** bake this pattern
  into documentation / `cargo generate`-style scaffolding for a
  third adapter, or let the next adapter re-derive it?
- **`session_partition()` as the per-channel identity hook.** The
  new `ChannelContext::session_partition() -> Option<String>`
  method (Phase 8 Task 2, non-breaking default impl returning
  `None`) threads per-chat identity through the tool layer via
  `aivyx_memory::tools::namespaced_topic`. The partition boundary
  is at the *tool*, not the *channel* or the *turn loop* —
  `aivyx-core` stayed untouched through the whole change, which
  is the signal that Option B was the right choice. **Phase 9
  question:** does any other tool family (beyond memory) need
  partition-awareness, and if so, is `namespaced_topic` the right
  reusable primitive?
- **Trust tier: `SemiTrusted`, not `Untrusted`.** The original
  ROADMAP / PHASE_8 draft said Telegram was `Untrusted`. The
  Phase 8 Task 1 correction — a Telegram chat is an authenticated
  human on a remote channel, not an anonymous internet source —
  is worth promoting to a D4 clarification in Phase 9 if the
  distinction matters for a future adapter. **Phase 9 question:**
  does the D4 trust-tier ladder need a fourth rung, or does
  `SemiTrusted` cover the full "authenticated remote human"
  space?
- **Two open deferrals from Phase 8 that are implicitly Phase 9's
  inheritance:** (a) `/cancel` in-band over Telegram, with a full
  task sketch in PHASE_8.md Task 5's "Phase 9 task sketch"
  subsection, and (b) multi-chat pumping (one aivyx process
  serving multiple Telegram chats concurrently, instead of Phase
  8's one-chat-per-channel simplification). Both need to be
  weighed against any new third-adapter work at Phase 9 entry.

**Candidate Phase 9 deliverables** (weighed at Phase 9 entry, not
committed here):

1. **`/cancel` mid-turn over Telegram** (Phase 8 Q8 deferral) —
   scan-poll `get_updates` alongside `agent.turn` with
   `tokio::select!`, full task sketch in PHASE_8.md Task 5.
2. **Multi-chat pumping** for `aivyx --channel telegram` — one
   process, N chats, N channels, shared store and audit chain.
3. **Matrix adapter** as the third `ChannelContext` — the
   federation / encrypted-rooms story, with deliberate pressure
   on the sibling `run_*_session` pattern to see whether a
   third data point justifies extraction. Matrix was
   explicitly Phase 9+ in the original Phase 8 non-goals.
4. **Desktop GUI** — the other end of the trust-tier ladder
   (`TrustTier::Trusted` via local IPC / socket, no network).
   A different shape entirely from Telegram, which is why it's
   the interesting stress test for the adapter pattern.
5. **`aivyx-config`** — a unified config-source layer (env vars,
   TOML file, `KeyDomain::Secrets`) that Phase 8's env-var
   sprawl (`ANTHROPIC_API_KEY`, `AIVYX_PASSPHRASE`,
   `AIVYX_TELEGRAM_TOKEN`, `AIVYX_TELEGRAM_CHAT_ID`,
   `AIVYX_STORAGE_PATH`, `AIVYX_MEMORY_MAX_PER_TOPIC`) implicitly
   argues for.

Phase 9 entry will pick which of these to commit to. The pattern
from Phases 5 → 6 → 7 → 8 is that each phase's exit reveals the
sharp edges on the *next* phase's placeholder, and Phase 9's
entry scaffold will refine this list based on whatever we notice
in the ~week between Phase 8 close and Phase 9 open.

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

## Phase 10+ — open

Deliberately ambiguous until Phase 9 closes. The pattern of
one-paragraph placeholders refined at each phase exit holds
here: whatever Phase 9 teaches us about the adapter pattern
(or fails to teach us) will shape the Phase 10 entry.

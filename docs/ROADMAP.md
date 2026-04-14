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

## Phase 10 — Tool-layer refinement + memory substrate

**Status:** Active — see [`PHASE_10.md`](PHASE_10.md).

The last pure-foundation phase before the codebase pivots from
foundation to product. Three deferrals that have been rolling
through phase journals close in this phase, and the tool-trait
inherits one additive refinement that every future product-phase
tool (`shell.exec`, `edit.patch`, `web.fetch`) will want on day
one. Specifically: (1) cross-topic `memory.read` — the Phase 6
Q3 deferral — lands as a `RedbMemory::scan_topics()` primitive
behind a new `memory.read:topic:*:session:<session>` wildcard
scope that is **not** in any default tier ceiling, so cross-topic
access stays a deliberate opt-in; (2) the `Tool` trait gains an
`input_schema()` default-`None` method with a hand-rolled 80-line
validator in the turn loop (no new dependency), so tool input
JSON is validated before `required_scope` runs; (3) `StreamEvent
::ToolCallStarted` gains a `tool: &str` field so consumers
(renderers, audit bridge) can show the user *which* tool is
running. Tasks 2 and 3 are an **intentional, conscious break**
of the production-core-byte-identity streak held since Phase 8
Task 2's `c3883be` — recorded in the phase journal as the first
additive trait refinement inside D3's contract. Phase 11 is the
first product phase.

## Phase 11 — Role system + shell execution (first product phase)

**Status:** Planned. Deliberately sketched at one-paragraph
depth until Phase 10 closes.

The pivot from foundation to product. Introduces a `Role`
concept — a named bundle of `(system_prompt, tool_allowlist,
memory_topic_prefix)` — so users can "employ" Aivyx as a coder,
PA, researcher, writer, etc. First concrete role: `coder`. Ships
a `shell.exec` tool at `TrustTier::Trusted` only (never offered
to Telegram or any `SemiTrusted` adapter), scoped as
`shell.exec:cwd:<path>` with path-prefix attenuation. Role
definitions live in `aivyx-config` (already provenance-tracked).
`memory.*` tools auto-prefix topics with the active role name
so personas don't bleed memory into each other. The intentional
test of Phase 10's tool-trait refinements: `shell.exec` is the
first tool that ships with a non-`None` `input_schema()` from
day one, so the tool-layer investment pays off immediately.

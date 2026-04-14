# Phase 6 — Memory as Tool

**Status:** Active (opened 2026-04-14)
**Predecessor:** [PHASE_5.md](PHASE_5.md) (exit commit `6dab2a7`, frozen at `fdc7770`)
**Contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all
LOCKED — unchanged since `e0d6437`, **five phases running**)

This document is the **working journal** for Phase 6. It will churn.
At phase exit it is frozen under the same convention as
[`PHASE_5.md`](PHASE_5.md) — no edits except through commits tagged
`docs(phase-6):`.

## Goal

Implement **`aivyx-memory`** against D1's core commitment: *memory is a
tool the agent chooses to call, not ambient context injected at turn
start*. Ship `memory.read`, `memory.write`, and `memory.forget` as real
`Tool` impls that the turn loop invokes the same way it invokes
`fs.read` and `fs.write`, persisted under `KeyDomain::Memory` in the
encrypted store that Phase 5 stood up. By phase exit, the `aivyx` CLI
can hold a conversation, restart, and have the agent **recall** what
it learned — not because the process stayed alive, and not because a
prompt hook silently dumped the last N turns into the system message,
but because the planner chose to emit a `memory.read` tool call and the
registry routed it through scope check + audit + storage.

This is the phase that **closes the loop on D1**. Once memory is a
tool, Aivyx has structurally prevented the "hidden memory injection"
class of bug by construction — there is no ambient substrate the
planner can lean on that doesn't go through a capability-checked,
audited path.

## Non-goals

- **Not a RAG system.** No embeddings, no vector search, no semantic
  retrieval. A memory "entry" is a small key/value blob under
  `KeyDomain::Memory` with a topic tag and a timestamp. The planner
  queries by topic (or by a tight prefix glob) and gets back recent
  entries. If we later want embeddings, they go in `aivyx-memory` as
  a Phase 7+ enhancement — the D1 contract is about the *shape of
  access*, not retrieval sophistication.
- **Not multi-session memory sharing.** The memory substrate is
  single-session for this phase. Cross-session recall ("what did I
  tell the other agent yesterday") is a capability expansion that
  needs its own scope attenuation story and belongs later.
- **Not a memory GC policy.** `memory.forget` is a tool the agent can
  call; automatic eviction (size caps, TTL) is deferred. If the store
  grows unbounded during development, that's a discipline problem,
  not a substrate problem.
- **Not a new crate split.** `aivyx-memory` already exists as a
  Phase 0 stub (D8-locked). This phase fills it in. No amendment
  needed on that front.
- **Not a re-litigation of Phase 4's Q1.** Phase 4 parked
  `FsReadTool` / `FsWriteTool` in `aivyx-core::tools::fs` with an
  explicit "revisit when there are two concrete tool families."
  Phase 6 *is* that second family, but the tools live in
  `aivyx-memory` per D8 — the re-eval is one of Phase 6's open
  questions (Q4), not a foregone conclusion to move fs into a shared
  umbrella.
- **Not interactive passphrase prompting.** Still deferred from
  Phase 5. The binary continues to read `AIVYX_PASSPHRASE`. If this
  phase needs to demo a fresh-install flow, it uses the env var.
- **Not persistent audit.** Audit is still HMAC-chained in memory
  per Phase 2's `aivyx-audit`. Wiring audit into `KeyDomain::Audit`
  is deferred to Phase 7+ as flagged in PHASE_5.md's "decisions
  deferred" list.

## Entry criteria (all met from Phase 5 exit)

- [x] Encrypted storage ships and `KeyDomain::Memory` is reachable
      via `storage.domain(KeyDomain::Memory)` (Phase 5 task 2,
      `772c39e`).
- [x] `SessionConfig.storage: Arc<dyn Storage>` is threaded through
      `run_session` and the binary (Phase 5 task 4, `dd175de`) —
      memory tools will clone this same handle, no new plumbing.
- [x] Persistence round-trip is end-to-end tested (Phase 5 task 5,
      `6dab2a7`) — a scripted write-then-reopen-and-read works
      through the same seams Phase 6 will exercise.
- [x] `FsReadTool` / `FsWriteTool` exist as the reference tool
      implementation pattern (Phase 4) — Phase 6 copies the shape.
- [x] Capability system has `memory.read` / `memory.write` /
      `memory.forget` defined per D4 scope taxonomy; `CapabilitySet`
      accepts them via `Scope::parse`.
- [x] `DESIGN.md` is still at its `e0d6437` baseline — five-phase
      empty-diff streak on entry.

## Refinements queued from Phase 5

The threads Phase 5 left behind, sorted by "likely to bite Phase 6"
vs "safe to punt."

- **D3's `Tool::required_scope(&self)` → `required_scope(&self,
  input: &Value)` refinement.** Flagged in DESIGN.md line ~1038 as
  "caught during D4 sanity check against D1 scenarios." The
  motivating example is literally `memory.read` needing
  `memory.read:session:<id>` when the input filter names a session,
  or bare `memory.read` when it doesn't. Phase 6 is the phase that
  forces this decision — it is the first concrete tool whose
  required scope depends on its input. **Decision point:** if the
  in-hand `required_scope(&self) -> Scope` signature suffices
  (because Phase 6 ships only coarse `memory.read` with no session
  qualifier), hold the line. If it doesn't, draft the first-ever
  `docs/amendments/` entry to refine D3 — ending the five-phase
  streak, but for the right reason.
- **Interactive passphrase prompt.** Still deferred. Phase 6 will
  punt again unless demoing the binary becomes painful.
- **Audit to `KeyDomain::Audit`.** Still deferred. Memory tools
  generate audit events that land in the in-memory
  `HmacChainLog` — same ephemeral fate as every prior phase.
- **`SessionMarker` growing a `turns_since_last_compaction` counter.**
  Floated during Phase 5 task 4 and dropped — it's the kind of
  field that only earns its place once something actually compacts.
  Phase 6 might be that something (memory writes per turn) or might
  not. Re-evaluate at task 4 when we see what the tool-invocation
  path actually touches.

## Draft task breakdown

This is a *draft*. Phase 5's breakdown survived intact from entry to
exit, but Phase 2's did not — tasks are allowed to reorder and
re-scope as we learn.

1. **Design the `Memory` trait and entry shape.** Define the minimum
   substrate surface: `put`, `get_recent(topic, limit)`, `forget`.
   Entry = `MemoryEntry { topic: String, body: String, created_at:
   u64 }`, serialized as bincode or postcard (whichever is already
   in the workspace) under a key shaped like `topic_bytes || 0x00
   || seq_be`. The trait is async + `Send + Sync`, consistent with
   `Storage` and `LlmProvider`. Unit tests against an in-memory
   fake (`InMemoryMemory`) so tool tests can avoid paying the
   redb/crypto cost. This is also where Q1 (entry encoding) must
   resolve.

2. **Ship `RedbMemory` — the redb-backed impl.** Wraps the
   `DomainHandle` for `KeyDomain::Memory`. Topic→entries mapping
   is key-prefix scan: `put` writes `topic || 0x00 || seq_be`;
   `get_recent` range-scans the topic prefix via a new
   `DomainHandle::scan_prefix` method (or whatever Phase 5 left
   on the handle — check before adding). 10+ unit tests covering:
   round-trip, ordering by sequence, topic isolation (writes to
   topic A don't appear in reads of topic B), `forget` deletes
   only matching entries, missing-topic returns empty. No
   integration with `run_session` yet — this task lands the
   substrate, not the tools.

3. **`MemoryReadTool` / `MemoryWriteTool` / `MemoryForgetTool`.**
   Three `Tool` impls in `aivyx-memory::tools` that wrap an
   `Arc<dyn Memory>` and produce the right `Scope` per D4. Input
   schemas hand-written as `serde_json::Value` (same decision
   Phase 4 made for fs tools — still cheap to hand-write at three
   tools, revisit if Phase 7 adds a fourth). Each tool's
   `invoke` goes through the usual `ToolContext` path. Unit tests
   against `InMemoryMemory`: scope-checked happy path, scope
   denial, malformed input, empty result.

4. **Wire into the `aivyx` binary.** In
   `crates/aivyx-channel/src/bin/aivyx.rs`, construct a
   `RedbMemory` from the already-open `Arc<dyn Storage>`, wrap it
   in `Arc<dyn Memory>`, and register the three tools in the
   `ToolRegistry` alongside `fs.read` / `fs.write`. The capability
   set grows by three scopes. Default grant is the whole memory
   family for a `Trusted` CLI channel — D4 tier table says this
   is fine for Trusted (line ~728 of DESIGN.md). No change to
   `SessionConfig` itself; memory is just more tools in the
   registry. Bring-up test: live-LLM smoke where the user says
   "remember that my favorite color is purple" and a later turn
   asks "what is my favorite color?"

5. **Scripted memory round-trip integration test.** New file
   `crates/aivyx-channel/tests/memory_tool_e2e.rs`. Same shape as
   Phase 4 task 5's `fs_tool_e2e.rs` and Phase 5 task 5's
   `storage_persistence_e2e.rs`: `ScriptedProvider` drives two
   scripted LLM steps in one turn (tool call → tool result →
   final message), the second turn's planner script emits a
   `memory.read` call that returns the prior write. Assertions:
   the tool was invoked, the audit chain has
   `ToolCalled`/`ToolCompleted` pairs for both memory ops, and —
   the Phase 5 trick — a **second** session opened against the
   same scratch store can still read the entry (proving the
   write hit `KeyDomain::Memory`, not just process-local cache).

6. **Phase 6 exit.** Freeze `PHASE_6.md`, update `README.md` and
   `ROADMAP.md` for Phase 7, refine the Phase 7 entry with
   whatever Phase 6 taught us. This is the first exit where the
   "what comes next" question is open — Phase 7 has been a
   placeholder since Phase 0. Resolving it is part of the exit.

## Open questions

### Q1. How is a `MemoryEntry` encoded in the store?

**Status:** open at phase entry. Must resolve before task 1.

The entry needs a serialization format for the body of each
`KeyDomain::Memory` value. Options:

1. **`serde_json`.** Already in the workspace (Phase 4's tools use
   it for input schemas, Phase 5's audit chain uses it for MAC
   coverage). Zero new deps. Human-readable if someone drops down
   to `redb` directly for debugging. Slightly larger on disk.
2. **`bincode` or `postcard`.** Compact, fast. New dep.
3. **Hand-rolled packed layout** (topic_len_be || topic || body_len_be
   || body || timestamp_be). No dep, no serde overhead, but every
   schema change is a migration.

**Leaning:** (1) for this phase. We already pay the serde_json
cost, the store is encrypted so "human-readable" matters only for
test asserts, and "no new deps" is the Phase 5 pattern. Revisit if
Phase 7 needs bulk scans.

### Q2. What does `get_recent(topic, limit)` mean — recency by seq or by wall clock?

Two semantics, different failure modes:

1. **By insertion sequence.** Monotonic per-topic counter, newest
   entries are highest seq. Simple, deterministic, survives clock
   skew. Range-scan from high to low with `limit` as the cutoff.
2. **By `created_at` timestamp.** Real wall clock. Allows
   out-of-order inserts (e.g., imported from a backup) to sort
   correctly, but vulnerable to `SystemTime` non-monotonicity.

**Leaning:** (1) because "agent recall" is about "what did I say
most recently in this conversation," which is naturally an
insertion order. Store both; sort by seq. If a Phase 7+ import
tool arrives, it can re-seq on import.

### Q3. Does `memory.read` without a topic filter return everything?

Three sub-options:

1. **Require a topic argument.** Simplest. Every recall must name
   what it's recalling. Good for capability attenuation (each
   topic can in principle become its own scope qualifier later).
   Forces the LLM to guess topics, which may not work.
2. **Optional topic; no topic = recent across all topics.** Matches
   how humans ask ("what have we been talking about"). Harder to
   audit — a single `memory.read` call can surface arbitrary
   state.
3. **Reserved special topic `"*"`.** Explicit opt-in to the
   "everything" query, so it's still visible in audit. Middle
   ground.

**Leaning:** (3). The audit chain records the input JSON, so an
explicit `"*"` makes the "read everything" calls grep-able later.

### Q4. Does `FsReadTool` move from `aivyx-core::tools::fs` to `aivyx-memory` (or a shared `aivyx-core::tools` → umbrella crate)?

Phase 4's exit explicitly left this decision to Phase 6 because
Phase 6 is the first time we'd have two concrete tool families.

1. **Leave fs where it is.** Phase 6 puts memory tools in
   `aivyx-memory` per D8, fs stays in `aivyx-core::tools::fs`.
   Zero amendment. Slight asymmetry — "why does fs live with the
   turn loop but memory lives on its own?" has the answer
   "because D8 gave memory its own crate and D8 doesn't give fs
   one, that's all."
2. **Move fs to `aivyx-memory`.** Wrong on naming.
3. **New `aivyx-tools` umbrella crate.** First D8 amendment.
   Requires moving fs code mid-phase, which expands Phase 6's
   blast radius.

**Leaning:** (1). "Evidence-driven amendments" was the Phase 4
discipline, and two concrete tools is still thin evidence. If
Phase 7 adds a third concrete family (shell? network?), *that's*
the phase to bundle the move.

### Q5. Does `Tool::required_scope` grow an `input: &Value` parameter?

**Status:** open at phase entry. Must resolve before task 3.

See DESIGN.md line ~1038's pending refinement. The scope system's
most precise story is `memory.read:session:<id>` when the input
names a session — but *today's* `required_scope(&self) -> Scope`
signature can't express that, because the scope doesn't see the
input.

Three paths forward:

1. **Ship Phase 6 with coarse `memory.read`, no session qualifier.**
   Keep the D3 signature. The scope system still works, it's just
   less precise than DESIGN.md sketched. Document the delta in the
   exit record.
2. **Amend D3.** First-ever `docs/amendments/` entry, ending the
   five-phase empty-diff streak. Change
   `required_scope(&self) -> Scope` to
   `required_scope(&self, input: &Value) -> Scope`, update every
   existing `Tool` impl (both fs tools + three memory tools),
   update the registry's check path to pass `input` through.
3. **Side-channel.** Add a second method
   `refine_scope(&self, base: Scope, input: &Value) -> Scope` that
   defaults to `base`. Doesn't require an amendment (additive,
   default-implemented), but is strictly worse than (2) — two
   methods where one would do.

**Leaning:** (1) for the first cut. If the integration test
(task 5) proves coarse `memory.read` is enough to demonstrate
"agent recalls across turns," ship it and re-queue the D3
amendment to Phase 7 when a concrete attack scenario forces it.
If coarse turns out to be painful during task 3, jump straight
to (2) — (3) is not the right shape. **The streak is worth
holding onto but it's not worth lying for.**

### Q6. Does Phase 6 need to touch `SessionMarker` at all?

Phase 5 left the marker at 40 bytes (session_uuid || opened_at ||
last_turn_index || last_turn_at). Memory writes are per-tool-call,
not per-turn, so the marker doesn't need to track them directly —
the memory substrate is its own domain. **Leaning:** no marker
changes. Revisit only if task 4's binary needs to display "you
have N memories" at reopen, which is a UX nicety, not a
correctness requirement.

## Exit criteria (draft — revised as work lands)

- [ ] `aivyx-memory::Memory` trait and `RedbMemory` impl exist
      with unit-test coverage of put, get_recent, forget, topic
      isolation, and empty-topic cases.
- [ ] `MemoryReadTool` / `MemoryWriteTool` / `MemoryForgetTool`
      implement `Tool`, declare the right scopes, and pass unit
      tests including scope-denial and malformed-input paths.
- [ ] The `aivyx` binary registers all three memory tools
      alongside the fs tools at session start.
- [ ] A scripted integration test
      (`crates/aivyx-channel/tests/memory_tool_e2e.rs`) drives a
      two-turn recall round-trip *and* reopens the store in a
      second session to verify disk persistence (the Phase 5
      pattern applied to the memory substrate).
- [ ] `cargo test --workspace` green.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `DESIGN.md` still unchanged (streak to 6) **or** a single
      amendment file under `docs/amendments/` documents the Q5
      refinement, referenced inline from the `DESIGN.md` section
      it supersedes. Either outcome is acceptable — honesty over
      streak preservation.
- [ ] Q1 (entry encoding), Q3 (topic-filter semantics), Q4 (fs
      relocation), and Q5 (required_scope signature) resolved and
      noted under "Decisions made during Phase 6 that aren't in
      DESIGN.md" in the freeze doc, regardless of which option
      won.
- [ ] At least one Phase 5 queued refinement either landed or
      explicitly re-queued to Phase 7 with a reason.
- [ ] Phase 7 roadmap entry refined with whatever Phase 6
      uncovered — in particular, the first concrete opinion on
      whether "Ecosystem" (remote channels) or "Hardening"
      (audit persistence, interactive passphrase, GC) is the
      right next step.

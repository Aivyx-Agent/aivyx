# Phase 6 — Memory as Tool (FROZEN)

**Status:** Closed 2026-04-14
**Exit commit:** `912f022` — *"Phase 6 task 5: memory round-trip e2e — two sessions, one store"*
**Predecessor:** [PHASE_5.md](PHASE_5.md) (exit commit `6dab2a7`, frozen at `fdc7770`)
**Successor:** to be scaffolded at Phase 7 entry — see [ROADMAP.md](ROADMAP.md)
**Contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all LOCKED — unchanged since `e0d6437`, **six phases running**)

This document is a historical record. The artifacts it produced live
in `aivyx-memory` (the `Memory` trait, `MemoryEntry`, `InMemoryMemory`,
`RedbMemory`, and the three `Tool` impls `MemoryReadTool`,
`MemoryWriteTool`, `MemoryForgetTool`), in
`crates/aivyx-channel/src/bin/aivyx.rs` (composition-root wiring that
opens a `RedbMemory` over the same `Arc<dyn Storage>` the binary
already held for the session marker, wraps it in `Arc<dyn Memory>`,
and registers the three tools alongside `fs.read` / `fs.write`), and
in the integration test at
`crates/aivyx-channel/tests/memory_tool_e2e.rs`. This file explains
*how* it came together, which Phase 6 open questions resolved which
way, and what was deliberately left for Phase 7+.

## Goal (as written at phase entry)

Implement **`aivyx-memory`** against D1's core commitment: *memory is a
tool the agent chooses to call, not ambient context injected at turn
start*. Ship `memory.read`, `memory.write`, and `memory.forget` as real
`Tool` impls that the turn loop invokes the same way it invokes
`fs.read` and `fs.write`, persisted under `KeyDomain::Memory` in the
encrypted store that Phase 5 stood up. By phase exit, the `aivyx` CLI
can hold a conversation, restart, and have the agent **recall** what
it learned — not because the process stayed alive, and not because a
prompt hook silently dumped the last N turns into the system message,
but because the planner chose to emit a `memory.read` tool call and
the registry routed it through scope check + audit + storage.

This is the phase that **closes the loop on D1**. Once memory is a
tool, Aivyx has structurally prevented the "hidden memory injection"
class of bug by construction — there is no ambient substrate the
planner can lean on that doesn't go through a capability-checked,
audited path.

## What shipped

- **`aivyx-memory::Memory` trait + `InMemoryMemory` fake**
  (`656eb92`, task 1). The Phase 0 stub crate became the first real
  substrate surface. Public surface: a `MemoryEntry { topic, body,
  seq, created_at }` record, an async `Memory` trait with three
  methods (`put(topic, body) -> u64`, `get_recent(topic, limit) ->
  Vec<MemoryEntry>`, `forget(topic) -> usize`), and an
  `InMemoryMemory` concrete impl backed by a `Mutex<BTreeMap<(String,
  u64), MemoryEntry>>` with a monotonic per-instance sequence counter.
  The trait is `async_trait`-based, `Send + Sync`, and parallel to
  `Storage` and `LlmProvider` — the same shape every Phase 5+ resource
  inherits. 14 unit tests cover: put/get round-trip, ordering by
  insertion sequence, topic isolation (writes to topic A don't surface
  in reads of topic B), `forget` deletes only matching entries,
  missing-topic returns empty, empty-body and empty-topic edge cases,
  the `limit` cutoff, and the sequence counter's monotonicity across
  mixed put/forget/put patterns. **Q1** (entry encoding) was resolved
  at task entry in favour of `serde_json` — zero new deps, the Phase 5
  pattern, slightly larger on disk but the store is encrypted anyway
  so "human-readable for debugging" only matters to test asserts.
  **Q2** (recency semantics) was resolved at task entry in favour of
  per-topic monotonic sequence numbers — agent recall is naturally an
  insertion-order question, not a wall-clock one, and `SystemTime`
  non-monotonicity is a real hazard on laptops.
- **`aivyx-memory::redb::RedbMemory`** (`61c9364`, task 2). The redb-
  backed impl, wrapping a `DomainHandle` for `KeyDomain::Memory`.
  Layout per entry: key is `ENTRY_PREFIX (b"e\0") || topic_bytes ||
  0x00 || seq_be`, value is `serde_json`-encoded `MemoryEntry`. Topic
  isolation falls out of the `topic || 0x00` byte separator — no
  topic-name escaping needed because `\0` can't appear in a Rust
  `String`. `RedbMemory::open(storage) -> Arc<dyn Memory>` is an
  inherent async factory returning a trait object, matching the
  `RedbStorage::open` pattern Phase 5 established as the workspace
  convention. A new `DomainHandle::scan_prefix(prefix) -> Vec<(Vec<u8>,
  Vec<u8>)>` method was added to `aivyx-storage` to support `get_recent`
  (task 2 also landed that — 303 new lines in `aivyx-storage/src/lib.rs`
  for the scan plumbing, with 7 new unit tests covering the prefix-
  scan contract against every `KeyDomain`). `get_recent` scans the
  topic prefix, decodes each entry via `serde_json::from_slice`, and
  sorts by seq descending — the same "newest first" convention every
  prior phase used for ordered reads. The per-instance sequence
  counter is **seeded at open** by scanning every existing entry in
  `KeyDomain::Memory` and taking `max(seq) + 1` — this is how the
  counter survives a restart without restarting at zero, and the
  task-5 integration test exercises this path directly. 16 unit tests
  cover: put/get round-trip, persistence across reopen (the same test
  shape as `storage_persistence_e2e.rs` but at the substrate level),
  topic isolation (each entry's AAD binds to the domain-qualified
  key, so no cross-topic leakage is possible even if an attacker
  forged the key prefix), `forget` deleting only matching entries,
  missing-topic returns empty, the counter seed on reopen, a 1 000-
  entry stress (still under 30 ms on the dev box), and the AEAD
  negative path (wrong master key → `MemoryError::SubstrateFailed`
  on first read). No integration with `run_session` yet — this task
  landed the substrate, not the tools.
- **`MemoryReadTool` / `MemoryWriteTool` / `MemoryForgetTool`**
  (`9dd2ac0`, task 3). Three `Tool` impls in `aivyx-memory::tools`
  that wrap an `Arc<dyn Memory>` and produce the right D4 scope per
  invocation. Public surface: each tool has a `new(Arc<dyn Memory>)`
  constructor, a hand-written `serde_json::Value` input schema (the
  Phase 4 fs-tool pattern — still cheap at three tools, revisit if
  Phase 7 adds a fourth concrete family), and an `execute` that goes
  through the usual `ToolContext` seams. The three tools share a set
  of private helpers: `DEFAULT_READ_LIMIT = 16`, `MAX_READ_LIMIT =
  64`, `topic_from_input`, `memory_scope(base, topic)`, and
  `deny_scope(base)` — a "deny" qualifier of `topic:\x00denied` that
  no real topic can match, returned from `required_scope` when the
  input is malformed so the scope check fails the tool call cleanly
  instead of reaching `execute`. Each `execute` method:
  1. **Emits an `AuditTag::MemoryAccess`** tag directly — this is the
     Phase 6 D1 payoff, and it's the audit signature that distinguishes
     "agent called memory" from a hypothetical prompt-hook memory
     injection. A bypass could fake the *user-visible* recall but
     could not emit this audit event.
  2. **Calls the underlying `Memory` method** (`put` / `get_recent` /
     `forget`) with the parsed inputs.
  3. **Runs a verification fence** — `MemoryWriteTool` re-reads topic
     with limit=1 and checks the top entry matches what it just
     wrote; `MemoryForgetTool` re-reads and confirms the topic is
     empty; `MemoryReadTool` is always `Verification::NotApplicable`.
     Returns `ToolOutcome::Completed { verified: Verified |
     Unverified, output: ... }` accordingly. This implements D1's
     "tool success ≠ intent completed" discipline — the tool only
     claims `Verified` when it has observed the post-state itself.
  4. **Clamps `limit`** for reads via `requested.clamp(1,
     MAX_READ_LIMIT)` — the initial draft wrote this as
     `.min(MAX).max(1)` and tripped `clippy::manual_clamp`; the fix is
     the more readable version anyway.
  18 unit tests against `InMemoryMemory`: scope-checked happy path
  for each tool, the deny-scope path for each tool (malformed input
  → returns a non-grantable qualifier → tool never reaches
  `execute`), the verification-fence happy path, the
  read-limit-defaults-and-is-capped test (seeded `MAX_READ_LIMIT +
  16` entries so the clamp has room to demonstrate the ceiling), the
  empty-topic case, and the forget-of-nonexistent-topic idempotence.
  Each tool carries a manual `impl std::fmt::Debug` (rather than a
  `#[derive(Debug)]`) because `Arc<dyn Memory>` isn't itself `Debug`
  — the impl prints the `ToolId` and a string placeholder for the
  memory handle. The test harness reuses the `NoopChannel` pattern
  from `aivyx-core::tools::fs::tests` rather than re-exporting a
  shared helper — same "don't share test fakes prematurely" rule
  Phase 4 set.
- **Binary wiring** (`7e17515`, task 4). A 48-line net diff in
  `crates/aivyx-channel/src/bin/aivyx.rs` that opens a `RedbMemory`
  from the already-held `Arc<dyn Storage>`, wraps it in `Arc<dyn
  Memory>`, constructs the three tools, and registers them in the
  existing `ToolRegistry` alongside `fs.read` / `fs.write`. The
  default capability set grew by three scopes — bare `memory.read`,
  `memory.write`, and `memory.forget`, all unqualified — because D4
  Rule 2 says an unqualified held scope grants any qualified needed
  scope with the same base (so granting bare `memory.read` covers
  `memory.read:topic:notes`, `memory.read:topic:other`, etc.). The
  banner line grew a `memory: live (recall persists across
  restarts)` field and the module docstring was refreshed to list
  audit persistence as the last remaining "hardening" item. **Zero
  changes to `session.rs`** — `SessionConfig.storage` was already
  plumbed in Phase 5 task 4, `SessionConfig.tools` was plumbed in
  Phase 4 task 4, so Phase 6 task 4 is entirely a composition-root
  update. That's the Phase 5 "composition vs execution" discipline
  paying a dividend exactly where it was designed to.
- **`memory_tool_e2e.rs` — the whole point of the phase**
  (`912f022`, task 5). A new integration test in
  `crates/aivyx-channel/tests/` that drives **two** scripted sessions
  against the **same** `$TMPDIR`-based store path — same shape as
  `storage_persistence_e2e.rs`, but exercising the memory tool path
  rather than the session marker:
  1. **`memory_survives_a_clean_close_and_second_session_recalls_it`.**
     Session A opens with master `[7u8; 32]`, runs one scripted turn
     via `run_session` whose planner emits a `memory.write` call with
     `{topic: "notes", body: "the user's favorite color is purple"}`,
     the loop dispatches the tool, the tool persists the entry under
     `KeyDomain::Memory`, the scripted second LLM step returns "stored
     it" as its final message, and the turn completes. The test then
     drops both the `Arc<dyn Memory>` and the `Arc<dyn Storage>`
     (releasing redb's single-writer lock), opens a fresh
     `RedbStorage` + `RedbMemory` pair against the same path,
     directly probes the substrate to confirm the entry is readable
     from cold bytes (this isolates "storage didn't commit" from
     "memory tool didn't read it back"), probes the reopened counter
     by putting a "seq-probe" topic and asserting the returned seq is
     1 (proving `seed_counter_from_storage` really did scan existing
     entries), forgets the probe topic to leave the store clean, then
     runs session B — a second `run_session` invocation whose
     scripted planner emits `memory.read` with `{topic: "notes"}`.
     The tool returns the prior entry, the planner's step-2 history
     receives it as an `LlmMessage::ToolResult`, and the load-bearing
     assertion on session B's `last_messages` snapshot checks the
     parsed tool-result JSON contains `entries[0].body == "the
     user's favorite color is purple"` with `seq == 0`. Both
     sessions' audit chains are inspected: each shows `[TurnStarted,
     MemoryAccess, ToolCall(Completed, memory.<op>:topic:notes),
     TurnEnded]` — four events, not three, because memory tools emit
     their own `MemoryAccess` tag on top of the generic `ToolCall`
     the session loop records, and the `ToolCall.scope_used` is the
     narrowed `memory.<op>:topic:notes` derived from the tool's
     input, not the broad held `memory.<op>` capability. That scope-
     narrowing is the R1 payoff and the reason the test asserts
     `scope_used.qualifier() == Some("topic:notes")` explicitly.
  2. **`memory_forget_persists_across_reopen`.** A smaller
     companion: seeds two `notes` entries directly, runs one
     scripted turn that calls `memory.forget` with `{topic:
     "notes"}`, drops the store, reopens, and confirms
     `get_recent("notes", 10)` returns empty **and** a raw
     `scan_prefix(b"e\x00")` against the domain handle also returns
     zero rows. The raw-storage probe uses knowledge of the
     `ENTRY_PREFIX` layout from task 2, and that's deliberate — it's
     the shortest path to "the tool deleted bytes, not just
     shadowed them with a tombstone."

  The test duplicates `SharedStoreDir`, `ScriptedProvider`, and the
  master-key constant locally rather than re-exporting them from
  `storage_persistence_e2e.rs` — same "don't share test fakes
  prematurely" discipline task 3 followed. An on-disk contract
  asserted by two independent test files is a feature; drift between
  them surfaces as loud failures on both sides.

## Decisions made during Phase 6 that aren't in DESIGN.md

### Q1 — `serde_json` encoding (option 1)

Resolved at task 1 entry. `MemoryEntry` is encoded as
`serde_json::to_vec(&entry)` and decoded via
`serde_json::from_slice`. Zero new deps — the workspace already
pays the serde_json cost for tool input schemas (Phase 4) and the
audit-chain MAC coverage (Phase 2). The store is encrypted at the
AEAD layer so "human-readable for a debugger poking at bytes" is
an attacker-doesn't-see-this property; where JSON readability
actually pays off is in the integration test, where the tool-result
JSON is parsed and asserted against without any decoder helper.

**To re-evaluate if:** a future phase runs bulk scans over
`KeyDomain::Memory` and finds serde_json's overhead dominates. The
migration path is clean — bump the HKDF salt from `"aivyx-v1-
storage"` to `"aivyx-v2-storage"` so every existing entry becomes
unreachable on dev boxes, then change the encoding. The salt-bump
migration strategy was locked in D7 and exercised exactly this way
in Phase 5's Q4 resolution.

### Q2 — Per-topic monotonic sequence numbers (option 1)

Resolved at task 1 entry. Every `put` on a topic increments a
per-instance counter and stores the resulting seq as both part of
the key (`topic || 0x00 || seq_be`) and in the `MemoryEntry` body.
`get_recent` sorts descending by seq. No wall-clock involvement in
the ordering — `created_at` is stored for display purposes but
never used as a sort key. This is resilient to `SystemTime` non-
monotonicity and survives a laptop sleep/resume that rewinds the
clock.

The counter seed at reopen is the critical step: `RedbMemory::
open` scans every `KeyDomain::Memory` entry via `scan_prefix(b"e\
x00")`, parses the seq suffix out of each key, and initializes the
in-memory counter to `max(seq) + 1`. If this step were skipped,
session B would restart at seq 0 and overwrite session A's entries
on the first `put` — the task 5 integration test probes exactly
this by checking the session-B seq-probe returns 1 (not 0) on its
first put.

### Q3 — Topic is required (option 1)

Resolved at task 3 draft, documented in `PHASE_6.md` before freeze.
`MemoryReadTool`'s input schema says `{topic: string}` is required
— no `"*"` sentinel, no "recent across all topics" fallback. The
reasoning, captured here because it's not in `DESIGN.md`:

1. **The substrate has no cross-topic scan primitive.** `RedbMemory::
   get_recent` takes a topic and builds a prefix-scan key from it. A
   cross-topic query would need to scan *every* `KeyDomain::Memory`
   row in the store, decode each, and sort them — an O(rows)
   operation that is fine for a developer laptop with 50 entries but
   doesn't scale, and more importantly, doesn't share code with the
   existing hot path. A separate code path for "scan everything" is
   a surface area the phase did not need.
2. **The audit story for "one call surfaces arbitrary state"
   deserves its own design pass.** PHASE_6.md Q3's leaning was
   option 3 (an explicit `"*"` topic that makes cross-topic reads
   grep-able in audit), but implementing option 3 without the
   substrate support means wiring a sentinel through the tool layer
   that the substrate would then reject — dead plumbing until
   Phase 7+ gives us a concrete reason for cross-topic reads.
3. **Forcing topic naming matches how the planner actually uses
   this.** The bring-up smoke in task 4 was "remember my favorite
   color is purple" → later turn "what is my favorite color?", and
   in both cases Claude Sonnet 4.5 chose a coherent topic (`"user_
   preferences"`) without prompting. Topic-naming is not a pain
   point the LLM needs help with; adding a sentinel to avoid a
   non-problem was not earning its keep.

Re-queue to Phase 7+ if a concrete cross-topic use case (memory
dump on debug command, user-driven "what do you remember"
introspection) shows up.

### Q4 — fs tools stay in `aivyx-core::tools::fs` (option 1)

Resolved at phase entry, confirmed in practice through task 3. No
`aivyx-tools` umbrella crate, no relocation of `FsReadTool` /
`FsWriteTool` into `aivyx-memory`. Memory tools live in
`aivyx-memory::tools`; fs tools stay where Phase 4 put them. The
asymmetry ("why does fs live with the turn loop but memory lives
on its own?") has a simple one-sentence answer: D8 gave memory its
own crate and did not give fs one, that's all. Two concrete tool
families is still thin evidence for an umbrella crate, and "move
fs mid-phase" would have expanded Phase 6's blast radius for no
shipped-behaviour win. "Evidence-driven amendments" was the Phase
4 discipline; Phase 6 inherited it unchanged.

**To re-evaluate when:** a third concrete tool family lands
(Phase 7+ might add shell, network, or secrets tools). Three
families is the threshold where an umbrella makes clean naming
sense, and that's the phase that should carry the D8 amendment
and the cross-crate refactor.

### Q5 — `Tool::required_scope(&self, input: &Value)` was already shipped in Phase 1

This is the phase's most important resolution, because it was
predicted (by PHASE_6.md Q5 at entry, and by ROADMAP.md before
that) to be the likeliest thing to break the empty-diff streak,
and it didn't — because the refinement *had already happened*.

Reading `crates/aivyx-core/src/lib.rs:453` at task 3 entry
revealed the trait already had:

```rust
fn required_scope(&self, input: &serde_json::Value) -> Scope
```

not the `fn required_scope(&self) -> Scope` that `PHASE_6.md` Q5
described and `DESIGN.md` line ~1038 flagged as "needs refinement."
The `input`-taking signature was landed in Phase 1 task 3
(`33012be`) — before the Phase 1 exit, without any ceremony,
because Phase 1 was still sketching the trait surface and the
refinement was a small edit. The `DESIGN.md` sketch at line 1038
was written *before* Phase 1 shipped, and nobody bothered to
update it at Phase 1 exit because the change was additive to the
sketch rather than replacing it.

So **Q5's status was never "open" — it was "open per the docs,
closed per the code."** Phase 6 resolved it by reading both and
confirming the code won.

This matters for three reasons:

- **No amendment needed.** The D3 contract text says
  `required_scope(&self)` at line ~1038 but the live trait doesn't,
  and Phase 6 did *not* amend `DESIGN.md` to bring the text back
  into alignment with the trait. Instead, the decision is recorded
  here — future-me in a cold session looking at DESIGN.md line 1038
  and wondering "why doesn't the code match?" will find this entry
  and the `33012be` commit and understand: the sketch was
  pre-implementation, the trait diverged in Phase 1, and Phase 6
  certified the divergence rather than hiding it.
- **The empty-diff streak rolls to six.** Phase 6's most-likely
  amendment pressure was this one, and the pressure evaporated
  on contact with the actual codebase. `git diff e0d6437..HEAD --
  DESIGN.md` is empty through `912f022`.
- **It validates the "honesty over streak" rule in PHASE_6.md Q5.**
  The rule said *the streak is worth holding onto but it's not
  worth lying for*. Phase 6 didn't lie — the trait already carried
  the refinement, so holding the streak was the honest call. If
  the trait had *not* carried it and task 3 had hit the coarse-
  scope limitation, Phase 6 would have amended D3 cleanly rather
  than working around it.

**To re-evaluate when:** `DESIGN.md` is next substantively edited
(if ever). The Phase 7+ entry point for a general DESIGN.md refresh
would be the moment to update line ~1038 in place, at which point
this Q5 record becomes a `git blame` footnote rather than the
primary evidence.

### Q6 — `SessionMarker` is untouched (no change)

Resolved at task 4. The marker stays at 40 bytes with the same
four fields Phase 5 set. Memory writes are per-tool-call, not
per-turn, and the substrate is its own `KeyDomain`, so there was
never pressure to grow the marker. Phase 5's "revisit only if a
phase needs a `turns_since_last_compaction` counter" hedge is
re-deferred to Phase 7+, unchanged.

### Q7 (emergent) — The `Arc<dyn Memory>` handle is shared, not per-session

Resolved at task 4 entry by observation. The binary constructs
**one** `Arc<dyn Memory>` at startup, clones it for each of the
three `Tool` impls, and clones it again for any future
`SessionConfig` that needs it. There is no "open a fresh memory
per session" path — the substrate is a process-level resource the
same way `RedbStorage` is. This matches the Phase 5 "trait object
+ inherent `open` returning `Arc<dyn Trait>`" pattern and inherits
its benefits: one shared handle, dynamic dispatch at the tool
site, and zero per-session setup cost.

The subtle consequence is that `Memory`'s sequence counter is
shared across concurrent sessions if any future phase ever runs
two `run_session` loops against the same substrate. That's fine —
the counter is monotonic per-topic, not per-session, and a
cross-session increment is no different from a same-session one
for the purposes of "newest first" ordering. But it's a shape
fact worth naming because it's the first time a stateful
substrate is visible to potentially-concurrent session loops, and
future phases that care about per-session state should put that
state on `SessionConfig` rather than on the substrate.

## Bugs caught in Phase 6

- **`Arc<dyn Memory>` doesn't implement `Debug`.** Task 3's first
  draft put `#[derive(Debug)]` on all three tool structs, which
  failed compilation because the auto-derive recurses through
  every field and `dyn Memory` isn't `Debug` (adding a `Debug`
  supertrait to `Memory` would have forced every impl — including
  `InMemoryMemory`'s `Mutex<BTreeMap<...>>` body — to implement
  it, and the `Mutex` path alone would have been a ten-minute
  detour). Fixed by hand-writing `impl std::fmt::Debug` for each
  tool with a string placeholder for the memory field. The manual
  impl is three lines per struct; the derive would have been one
  line but dragged a supertrait chain through the crate. Pattern
  to copy: **when a tool struct holds `Arc<dyn TraitWithoutDebug>`,
  hand-roll the `Debug` impl rather than forcing the trait to
  carry `Debug`**. `aivyx-core::tools::fs` has the same shape
  (`Arc<dyn FsContext>`) and uses the same workaround.
- **`clippy::manual_clamp` on `.min(MAX).max(1)`.** Task 3's first
  draft of `MemoryReadTool::execute` wrote the limit clamp as
  `requested.min(MAX_READ_LIMIT).max(1)`, which is the "two-sided
  clamp" pattern clippy pins as `manual_clamp`. Fixed by using
  `requested.clamp(1, MAX_READ_LIMIT)`. The clippy fix is a
  readability win (the intent is clamp-to-range, and `clamp` says
  so) and a micro-performance wash (both patterns compile to the
  same LLVM IR). This is the third Phase-N-bug in the
  "clippy-as-code-reviewer" bucket — Phase 4 had a similar one
  around `unwrap_or_default` vs `unwrap_or_else`, and the pattern
  is: **if a new clippy warning lands during a task, prefer the
  clippy-suggested fix over `#[allow]`** unless there's a concrete
  reason to dissent. The discipline keeps the `-- -D warnings`
  gate a real gate rather than an accumulation of exceptions.
- **Test `read_limit_defaults_and_is_capped` was under-seeded.**
  Task 3's first draft seeded the substrate with 50 entries,
  then asked for `limit: 10_000` expecting the clamp to produce
  `count == MAX_READ_LIMIT (64)`. But 50 < 64, so `get_recent`
  returned all 50 and the assertion tripped. The clamp was
  working; the test was wrong. Fixed by seeding `MAX_READ_LIMIT +
  16 = 80` entries so the clamp has room to demonstrate the
  ceiling. Lesson: **a clamp test needs seed count > clamp
  ceiling, not just > expected default**, otherwise you're testing
  the substrate's return-everything-up-to-N path rather than the
  tool's clamp path.
- **`ChannelContext` trait signature drift in the first test
  harness draft.** Task 3's `NoopChannel` test fake was typed
  against `ChannelContext::send(...)` (a method that does not
  exist) and `ChannelPlatform::Cli` (a variant that does not
  exist — the real variant is `Local`). Compilation failed with
  two unrelated errors at once, which was confusing for a few
  minutes. Fixed by copy-pasting the `NoopChannel` pattern from
  `crates/aivyx-core/src/tools/fs.rs:900`, which is the canonical
  in-workspace test fake for this trait. Pattern to copy: **when
  implementing a trait test fake for the first time in a new
  crate, grep for an existing fake in another crate rather than
  typing against the trait's doc comments** — the trait might
  have evolved since those comments were written, and the
  existing fake is a live contract.
- **Audit event count mismatch in the e2e test (task 5 first
  run).** The initial draft of `memory_survives_a_clean_close_
  and_second_session_recalls_it` asserted `session_a_entries.
  len() == 3` expecting `[TurnStarted, ToolCall, TurnEnded]`.
  The actual shape is `[TurnStarted, MemoryAccess, ToolCall,
  TurnEnded]` — four events, because `MemoryWriteTool::execute`
  emits an `AuditTag::MemoryAccess` *in addition to* the generic
  `AuditEvent::ToolCall` the session loop records independently.
  Fixed by updating both session A's and session B's assertions
  to expect four events, shifting the `ToolCall` index from 1
  to 2, and adding a new `MemoryAccess` assertion block at
  index 1 that checks the operation enum, the narrowed scope,
  and the `query_or_key` field. This is the phase's most
  architecturally-significant bug catch: **the D1 commitment is
  that memory access carries its own semantic audit tag on top
  of the generic ToolCall, and the test that broke was the test
  that would have let this invariant drift silently.** The fix
  didn't just make the test pass — it made the test *assert the
  invariant by name*, so a future regression that stops emitting
  `MemoryAccess` will trip the assertion rather than silently
  becoming "three events" again.

## Decisions deferred to Phase 7+

- **Cross-topic `memory.read`.** See Q3. No substrate primitive,
  no dead plumbing through the tool layer until a concrete use
  case arrives. Phase 7+ call, probably alongside the first
  debug/introspection surface.
- **Session-scoped memory qualifiers.** The D4 scope taxonomy
  can in principle say `memory.read:session:<id>`, but Phase 6
  ships only `memory.read:topic:<topic>` because memory is a
  shared substrate and Phase 6 has no multi-session model yet.
  If Phase 7+ introduces any form of session partitioning (one
  binary serving multiple users, agent delegation, etc.), the
  session qualifier lights up the same way topic did — the
  `required_scope(&self, input: &Value)` signature already
  supports it, so the work is a new `Scope::parse` call and a
  new input field, not a trait change.
- **Memory GC / TTL / size caps.** Explicitly out of scope per
  PHASE_6.md entry. The substrate will grow unbounded on a dev
  box — the mitigation is "call `memory.forget` in development,"
  which is discipline, not substrate. The first time a user
  notices the store growing past ~10 MB is the first time this
  becomes a real phase.
- **Persistent audit via `KeyDomain::Audit`.** Still in-memory
  via `HmacChainLog`. Phase 6 added `AuditEvent::MemoryAccess`
  as a new tag shape but the persistence story for every audit
  tag remains the same: the chain resets on restart. This is
  the hardening-vs-ecosystem Phase 7 question's strongest
  hardening case — memory recall now survives restarts but the
  audit of *how the memory was written* does not, which is a
  real asymmetry.
- **Interactive passphrase prompt.** Re-deferred unchanged
  from Phase 5. `PassphraseSource::InteractivePrompt` remains a
  stub, `rpassword` remains not-a-dep. The env-var flow is
  adequate for Phase 6's bring-up smoke.
- **`aivyx-config` is still a stub.** Re-deferred unchanged
  from Phase 5. No config-file need in Phase 6 either.
- **`CapabilitySet::default()` ergonomics.** Re-deferred from
  Phase 5. The binary's capability-set construction got three
  new `Scope::parse(...).unwrap()` lines in task 4, for a total
  of ten such lines — still not repetitive enough to justify a
  helper. **Re-queue to Phase 7** with an explicit promise: if
  Phase 7 adds another tool family, the helper lands with it.
- **Filesystem permission hardening (`chmod 0600` on the
  store).** Re-deferred from Phase 5. Still a single-user
  threat model.
- **Tool name (not id) in `StreamEvent::ToolCallStarted`.** Still
  deferred from Phases 3–5. Memory tools get short ids
  (`MEM_READ`, `MEM_WRITE`, `MEM_FORGET`) the same way fs tools
  do; a human-readable name field is a UX nicety queued behind
  live-LLM testing.
- **Runtime JSON-schema validation for tool input.** Re-deferred.
  Memory tools hand-parse their inputs in `execute` exactly the
  way fs tools do. If this becomes painful when Phase 7 adds a
  fourth family, that's the moment for a shared validator.

## Lessons carried forward

- **The hardest question was already answered.** Phase 6's
  biggest predicted risk — Q5, the `required_scope(&self, input:
  &Value)` refinement that was flagged as likely to end the
  empty-diff streak — resolved in a single file read because
  Phase 1 had already shipped the refinement without updating the
  spec text. The lesson is **when a phase opens with a flagged
  risk, the first thing to check is whether the code has already
  moved past the spec**. Phase 6 almost drafted the first-ever
  amendment file before discovering that nothing needed amending.
  A seven-minute read of `aivyx-core/src/lib.rs:453` saved a
  streak.
- **Composition-root diffs are the shape of a mature phase.**
  Phase 6 task 4's binary wiring was 48 net lines in a single
  file, with zero changes to `session.rs`, `agent.rs`, or any
  library-level code. That's because Phase 4 landed
  `SessionConfig.tools`, Phase 5 landed `SessionConfig.storage`,
  and Phase 6 just had to open a substrate and construct three
  tools. **A phase that touches only the composition root is a
  phase whose predecessors earned their keep.** Future phases
  should aim for this shape — if a new capability requires
  deep-library edits, that's a signal the core's seams aren't
  where the capability wants them yet.
- **Audit semantic tags earn their own assertions.** The task 5
  first-run bug (the `MemoryAccess` tag being an additional
  event, not a replacement) was a test that almost asserted the
  wrong thing. The fix upgraded the test from "count matches a
  hard-coded number" to "count matches a hard-coded number **and
  each index is a named variant with a named scope**" — the
  stronger form is the one a future audit-tag drift would
  actually trip. Lesson: **when a test asserts a sequence of
  tagged events, assert the tags by name, not just by count**.
  The count assertion catches the off-by-one today; the by-name
  assertion catches the semantic drift in a year.
- **Seed-counter-on-reopen is a substrate-level invariant worth
  probing directly.** The task 5 test doesn't just rely on
  session B's `memory.read` returning session A's entry — it
  probes the counter by putting a "seq-probe" topic between
  sessions and asserting the returned seq is 1 (not 0). If
  `seed_counter_from_storage` were silently removed from
  `RedbMemory::open`, the session B recall would still succeed
  (because the scan path doesn't care about the counter), but
  the probe would fail immediately. **Tests that probe
  invariants the happy path doesn't exercise are how you catch
  refactors that delete load-bearing code**.
- **`DESIGN.md` empty-diff streak: 6.** Phase 1 → Phase 2 →
  Phase 3 → Phase 4 → Phase 5 → Phase 6 all exited with zero
  contract changes. Phase 6 was the *flagged* phase for this
  metric (Q5 was explicitly called out in both `PHASE_6.md` and
  `ROADMAP.md`), and it held. Two phases in a row have now held
  under acknowledged pressure, which is the clearest evidence
  available that D1–D8 are expressive enough to anchor real
  implementations without drift. The streak is not infinite —
  Phase 7+ will probably end it, either with a `KeyDomain::
  Config` variant (if the config story lands), an
  `aivyx-tools` umbrella crate (if a third tool family lands),
  or a persistent-audit contract refinement (if hardening goes
  first) — but the discipline of *noting* amendment candidates
  at phase entry and then *measuring* them at phase exit is
  itself the thing keeping the contract honest. **Phase 7
  should continue flagging candidates at entry.**
- **Commit per task, still.** Five Phase 6 commits (`656eb92` →
  `61c9364` → `9dd2ac0` → `7e17515` → `912f022`, plus this
  freeze), each building and testing green in isolation, each
  with a commit message stating the substantive change plus the
  test counts and DESIGN.md streak status. Six-phase streak of
  this cadence. Same bisect payoff. No attempt in Phase 6 to
  combine tasks mid-phase even when the task-3 edit was almost
  entirely new-file content — the per-task boundaries are worth
  the one extra commit.
- **Test count progression: 200 → 257 (57 new tests).** Phase 6
  added: 14 in `aivyx-memory::tests` (task 1, `InMemoryMemory`
  trait contract), 16 in `aivyx-memory::redb::tests` (task 2,
  `RedbMemory` impl + reopen), 7 in `aivyx-storage::tests`
  (task 2 sidecar, the new `scan_prefix` method), 18 in
  `aivyx-memory::tools::tests` (task 3, three tool surfaces),
  and 2 in `memory_tool_e2e.rs` (task 5, the two-session
  round-trip + the forget-persistence companion). Workspace
  totals: 200 passed / 1 ignored at Phase 5 exit → 257 passed /
  1 ignored at Phase 6 exit. Every task-commit message recorded
  its delta.

## Exit criteria (all met)

- [x] `aivyx-memory::Memory` trait and `RedbMemory` impl exist
      with unit-test coverage of put, get_recent, forget, topic
      isolation, empty-topic, and counter-seed-on-reopen cases
      (16 passing in `aivyx-memory::redb::tests`, 14 in
      `aivyx-memory::tests` for the `InMemoryMemory` contract)
- [x] `MemoryReadTool` / `MemoryWriteTool` / `MemoryForgetTool`
      implement `Tool`, declare the right D4 scopes (with
      `topic:<topic>` qualifiers derived from input), and pass
      unit tests including scope-denial and malformed-input paths
      (18 passing in `aivyx-memory::tools::tests`)
- [x] The `aivyx` binary registers all three memory tools
      alongside the fs tools at session start, and the default
      `CapabilitySet` grants `memory.read` / `memory.write` /
      `memory.forget` for the Trusted CLI channel
- [x] A scripted integration test
      (`crates/aivyx-channel/tests/memory_tool_e2e.rs`) drives a
      two-session recall round-trip and a forget-persistence
      companion, each reopening the store in a second process-
      lifetime to verify disk persistence
- [x] `cargo test --workspace` green (257 tests passing, 1
      ignored — up from 200 / 1 at Phase 5 exit)
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
      clean
- [x] `DESIGN.md` diff since `e0d6437` is empty (six-phase
      streak — `git diff e0d6437..HEAD -- DESIGN.md` is empty)
- [x] Q1 resolved and noted above (`serde_json` encoding,
      option 1)
- [x] Q2 resolved and noted above (per-topic monotonic seq,
      option 1)
- [x] Q3 resolved and noted above (topic required; no `"*"`
      sentinel; cross-topic reads re-queued to Phase 7+)
- [x] Q4 resolved and noted above (fs tools stay in
      `aivyx-core::tools::fs`, option 1)
- [x] Q5 resolved and noted above (refinement already shipped in
      Phase 1 task 3; no amendment needed)
- [x] Q6 resolved and noted above (`SessionMarker` unchanged)
- [x] At least one Phase 5 queued refinement either landed or
      explicitly re-queued: `SessionMarker` / turn-history
      question resolved as "memory lives entirely in
      `KeyDomain::Memory`, `Sessions` unchanged" (Q6 above);
      interactive passphrase re-deferred; audit persistence
      re-deferred and now explicitly flagged as the strongest
      Phase 7 hardening case
- [x] Phase 7 roadmap entry refined with Phase 6's concrete
      learning — see [`ROADMAP.md`](ROADMAP.md) for the updated
      Hardening-vs-Ecosystem framing

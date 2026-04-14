# Phase 9 — Refinements on the second-adapter pattern

**Status:** Active (opened 2026-04-14)
**Predecessor:** [PHASE_8.md](PHASE_8.md) (exit commit `9736d3a`, last content commit `0484606`)
**Contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all
LOCKED — unchanged since `e0d6437`, **eight phases running**)

This document is the **working journal** for Phase 9. It will churn.
At phase exit it is frozen under the same convention as
[`PHASE_8.md`](PHASE_8.md) — no edits except through commits tagged
`docs(phase-9):`.

## Goal

Phase 8 shipped `aivyx-telegram` as the second concrete
`ChannelContext`, proving that D2's `ChannelContext` trait and D4's
four-rung `TrustTier` ladder generalize beyond `LocalChannel`.
Phase 9's job is **not** "ship a third adapter as a headline
feature." Phase 9's job is:

1. **Consolidate** what Phase 8 learned about the adapter seam —
   the design decisions Phase 8 made under time pressure that are
   worth revisiting now that there's a second data point.
2. **Pay down the two Phase 8 deferrals** that are implicitly
   Phase 9's inheritance: `/cancel` in-band over Telegram (Phase 8
   Q8, full task sketch in PHASE_8.md Task 5) and multi-chat
   pumping (Phase 8 Task 1's one-chat-per-channel simplification).
3. **Stress-test** the adapter pattern on one additional axis so
   the pattern is validated-by-use, not validated-by-argument.
   Which axis — third adapter (Matrix), opposite-tier adapter
   (desktop GUI), or config-source unification (`aivyx-config`) —
   is the central Phase 9 scoping call, resolved in Q1 below.

The headline outcome: when Phase 9 closes, a future phase that
wants to add a *fourth* `ChannelContext` adapter should know
exactly which lines to write and which seams to reuse, with no
ambiguity about "but Phase 8 did it this way and Phase 9 did it
that way, which is right?" This is the phase that **earns the
right to claim the adapter pattern is stable**, not the phase
that discovers it.

## Why now

Phase 8 left three concrete debts on the table:

1. **A pre-existing per-task clippy-validation gap.** Phase 8
   Task 4 shipped a `clippy::too_many_arguments` regression on
   `run_async` that Task 8's exit sweep caught four commits later.
   Fixed in the Phase 8 exit commit `9736d3a`, with a policy note
   in the PHASE_8.md Task 8 ship record: **run `cargo clippy
   --workspace --all-targets -- -D warnings` at every task**, not
   just at phase exit. Phase 9 inherits this policy and should
   validate it by never allowing a regression to survive more
   than one task.
2. **Two explicit deferrals** with task sketches:
   - `/cancel` in-band over Telegram. Phase 8 Q8 was deferred
     with a five-step task sketch in PHASE_8.md Task 5's "Phase 9
     task sketch" subsection. The sketch is concrete enough that
     Phase 9 can execute it directly rather than re-discovering
     the design.
   - Multi-chat pumping. `TelegramChannel` is Phase 8-bound to
     one `chat_id`; `run_telegram_session` filters inbound
     `get_updates` batches by target chat and silently drops
     everything else. Real deployments want one aivyx process
     serving N chats over one bot token, which means one
     `TelegramChannel` per active chat_id, a shared store and
     audit chain, and a multiplexing layer that routes inbound
     updates to the right channel.
3. **An unanswered scoping call:** whether to ship a *third*
   adapter now (Matrix is the leading candidate) or to spend
   Phase 9 on the two deferrals and a cross-cutting polish pass
   (`aivyx-config`, scaffolding docs, etc.) instead. Shipping a
   third adapter exercises the pattern by use; skipping one
   keeps the phase narrower and lets the deferrals land without
   a second parallel track. The right call depends on how
   confident we are that the Phase 8 pattern is correct — see
   Q1 below.

Phase 9 is the right time to pay these down because the
architectural debt compounds if ignored: if Phase 10 adds a third
channel adapter while the Telegram `/cancel` story is still a
sketch, the pattern Phase 10 inherits is the *wrong* pattern
(one that still has a known gap in cancellation UX). Better to
fix the known gaps now, one phase's worth of work, than to
propagate them into a third adapter and then fix them across
three adapters.

## Non-goals

- **Not a D2 or D4 amendment.** D2's `ChannelContext` picked up
  `session_partition()` in Phase 8 as a non-breaking optional
  refinement, and that's enough. D4's four-rung `TrustTier`
  ladder (Kernel / Trusted / SemiTrusted / Untrusted) has been
  in the contract since `2026-04-13` and already names every
  rung Phase 9 would care about — the "does D4 need a fourth
  rung" question from the ROADMAP entry is **stale**; Phase 8's
  correction was against the *ROADMAP/PHASE_8 draft wording*,
  not against a D4 gap. If Phase 9 finds a real contract bug,
  the Phase 6 Q5 rule ("honesty over streak preservation")
  applies and an amendment file lands — but the prior is that
  the contract fits.
- **Not rich media in Telegram.** Phase 8 explicitly non-goaled
  photo/sticker/voice, and Phase 9 inherits that. A
  `telegram.download_media` tool family is a Phase 10+ concern
  (or its own dedicated phase if the use case materializes).
- **Not webhook mode.** Still long-poll. The `TelegramTransport`
  trait seam is the drop-in point for a future webhook swap;
  Phase 9 does not exercise it.
- **Not the Channel Activation Milestone.** The milestone is
  explicitly scheduled for after the phase sequence closes (see
  ROADMAP.md). Phase 9 inherits Phase 8 Task 7's deferred Telegram
  smoke test *into* the milestone, not *out of* it — the milestone
  is not a Phase 9 deliverable.
- **Not a multi-tenant hosting story.** One aivyx process per bot
  token, same as Phase 8. Multi-chat pumping inside one process
  is fine and explicitly in scope; running N bot tokens from one
  process is not.
- **Not inline queries, callback buttons, or BotFather config.**
  Still out of scope. Phase 9 ships the same "text in, text out"
  message model Phase 8 shipped.
- **Not a rewrite of `run_session` / `run_telegram_session` into a
  single shared function** unless Q1 resolves to "ship a third
  adapter" *and* the third adapter's lifecycle actually makes the
  extraction earn its keep. Phase 8 rejected the extraction with
  two data points; a third data point that still doesn't justify
  it is evidence to keep the sibling pattern permanently, not
  evidence to force the extraction.

## Entry criteria (all met from Phase 8 exit)

- [x] `aivyx-telegram` crate exists and ships `TelegramChannel:
      ChannelContext` at `TrustTier::SemiTrusted` with a private
      `TelegramTransport` seam, `ReqwestTransport` production
      impl, `ScriptedTransport` test double, and unit-test
      coverage of round-trip, concurrent chats, cancellation
      rotation, buffered-send contract, and tool-marker rendering.
      *(Phase 8 Task 1.)*
- [x] `ChannelContext::session_partition() -> Option<String>` is
      live as a non-breaking trait addition. `TelegramChannel`
      returns `Some(chat_id.to_string())`; `LocalChannel` inherits
      the `None` default. `aivyx_memory::tools::namespaced_topic`
      wraps logical topics with the partition prefix at tool
      call-time. *(Phase 8 Task 2.)*
- [x] Tier-table scope attenuation is wired at the turn-loop
      channel boundary and has a negative pin test locking the
      `Local` vs `SemiTrusted` ceiling ratios. *(Phase 8 Task 3.)*
- [x] `aivyx --channel local|telegram` is the binary surface.
      `AIVYX_TELEGRAM_TOKEN` + `AIVYX_TELEGRAM_CHAT_ID` resolve
      the Telegram branch; `--channel` is mutually exclusive with
      `--verify-only`. *(Phase 8 Task 4.)*
- [x] Per-turn `CancellationToken` rotation works over the
      Telegram channel (matches `LocalChannel::reset_cancellation`
      pattern). A mid-turn cancel fires correctly and does not
      poison the next turn. *(Phase 8 Task 5.)*
- [x] Two-chats persistent e2e test lives in
      `crates/aivyx-telegram/src/tests.rs` and asserts an 8-event
      audit chain with per-chat dual-qualifier scopes, a cold
      `verify_from_disk` reopen, and physical-topic partition
      read-back proving chat isolation survives AEAD seal + HMAC
      replay + store reopen. *(Phase 8 Task 6.)*
- [x] Phase 8 exit freeze landed at `9736d3a` with the
      **DESIGN.md** empty-diff streak rolling forward to eight
      phases (byte-identical from contract-lock `e0d6437`).
      `crates/aivyx-core/` had **one additive, non-amendment
      change** during Phase 8 Task 2 (`c3883be`, +46 lines): a
      new `ChannelContext::session_partition()` default method
      and the turn-loop hook that injects the partition into
      tool input. Phase 8's commit message argued (and PHASE_8.md
      Task 2 "Streak impact" section ratified) that this
      *extends* D2 rather than overrides it, so the streak
      semantics tightened at Phase 8 to **"DESIGN.md unchanged
      + non-breaking aivyx-core extensions allowed"** — the older
      "core+contract byte-identical" wording stops being literally
      true at Phase 8 Task 2 and Phase 9 inherits the tighter
      framing. *(Phase 8 Tasks 2 + 8.)*
- [x] Phase 8 Task 7 (real-bot smoke test) is **deferred by
      design** to the Channel Activation Milestone; no operator
      credential is required for Phase 9 to open or run.
- [x] `cargo test --workspace` green on entry (revalidated below
      at phase open).
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
      clean on entry (revalidated below; Phase 8 Task 8 fixed the
      pre-existing `run_async` regression).

## Draft task breakdown

This is a *draft*. The Phase 6/7/8 pattern is that the draft
breakdown survives mostly intact from entry to exit, with tasks
shuffled and rescoped as each one's ship record gets written.
Phase 9's draft is **explicitly gated on Q1** — depending on how
Q1 resolves, Task 3 and Task 4 below either become a third-
adapter track (Matrix) or a polish-pass track (config + docs).

1. **`/cancel` in-band cancellation over Telegram.** Execute the
   five-step task sketch from PHASE_8.md Task 5 "Phase 9 task
   sketch". Concretely:
   - Upgrade `run_telegram_session_with_transport` to run each
     turn as `tokio::select!(agent.turn, scan_for_cancel)` with
     a ~2-second `scan_for_cancel` that wraps a short-timeout
     `get_updates` call.
   - Thread the scan's result into the main loop's cursor so no
     update is redelivered or lost.
   - Add a private `ScanResult` enum + `scan_for_cancel` helper;
     the main loop consumes the result and reshuffles queued
     updates.
   - Unit test: scripted transport with a turn that stalls,
     inject `/cancel` into the update queue, assert the turn
     finishes as `Cancelled` and the cursor advances past the
     `/cancel` `update_id`.
   - Unit test: turn finishes before any scan finds `/cancel`;
     updates that arrived during the scan are preserved and
     drive the next turn.
   Zero-core-touch target: all of this lives inside
   `crates/aivyx-telegram/` — `aivyx-core`'s turn loop already
   owns the 120-second wall-clock budget, and `/cancel` is a
   cooperative signal at the channel layer, not a core change.

2. **Multi-chat pumping for `aivyx --channel telegram`.** One
   aivyx process, N chats, N `TelegramChannel` instances, one
   shared `PersistentAuditLog` and one shared `RedbMemory`.
   Concretely:
   - Refactor `run_telegram_session` to spawn a per-chat inner
     task (one `TelegramChannel` + one session loop) from an
     outer multiplexer that owns the `get_updates` cursor and
     routes inbound updates by `chat_id` to the right inner
     task's mailbox.
   - The outer multiplexer lazy-spawns inner tasks: the first
     time an unknown `chat_id` arrives, a new `TelegramChannel`
     is constructed, shared store + audit refs are handed down,
     and the inner task enters its per-chat session loop.
   - Inner tasks are JoinHandles; the outer loop tracks them in
     a `HashMap<i64, JoinHandle>`; ctrl-C cancels all of them.
   - A scripted end-to-end test drives three concurrent chats,
     asserts each chat's memory is isolated (same partition
     shape as Task 6's e2e test), asserts the shared audit
     chain records turns from all three chats in interleaved
     order, and asserts `verify_from_disk` on the combined chain
     reports the expected combined event count.
   - The `AIVYX_TELEGRAM_CHAT_ID` env var becomes optional in
     this mode. When set, the outer multiplexer filters inbound
     updates to the single chat (Phase 8 compatibility). When
     unset, the multiplexer accepts all inbound chats.

3. **Third-adapter stress test *or* polish pass** — scope gated
   on Q1 below. Two forks:
   - **Q1 = third adapter:** ship a minimal third `ChannelContext`
     — likely Matrix, possibly a toy "HTTP-POST" JSON adapter —
     that deliberately exercises the sibling `run_*_session`
     pattern to see whether a third data point either (a)
     reveals the pattern holds cleanly and Phase 9 can claim it
     stable, or (b) forces the pattern to break because three
     differently-shaped lifecycles can no longer share ~100 lines
     of wiring by sibling-copy. Either outcome is a win; the
     outcome determines Phase 10's scope.
   - **Q1 = polish pass:** ship `aivyx-config` as a unified
     config-source layer replacing Phase 8's env-var sprawl
     (`ANTHROPIC_API_KEY`, `AIVYX_PASSPHRASE`, `AIVYX_TELEGRAM_
     TOKEN`, `AIVYX_TELEGRAM_CHAT_ID`, `AIVYX_STORAGE_PATH`,
     `AIVYX_MEMORY_MAX_PER_TOPIC`) with a single typed config
     object that reads from env vars → TOML file →
     `KeyDomain::Secrets` row, in a fall-through order. Ships
     with a scripted config-load test and a migration note for
     the binary.

4. **Adapter-seam documentation.** One short document —
   `docs/ADAPTER_PATTERN.md` (or inlined into an existing
   `docs/` file, TBD) — that captures the Phase 8 + Phase 9
   learnings as a future-proof checklist for "how to add a new
   `ChannelContext` adapter." Contents:
   - The sibling `run_*_session` pattern and when to break it
     (empirically: break it if a fourth adapter forces it, not
     before).
   - Per-channel transport ownership via a private
     `XxxTransport` trait seam with a `ScriptedTransport` test
     double in `src/tests.rs`.
   - `session_partition()` usage and the `namespaced_topic`
     tool-layer primitive as the only supported way to carry
     per-channel identity into tools.
   - Trust tier selection: authenticated remote human =
     `SemiTrusted`, local user = `Trusted` (GUI) or `Trusted`
     (CLI REPL), anonymous internet = `Untrusted`.
   - The per-task `-D warnings` policy established in Phase 8
     Task 8.
   This task is code-free; it ships pure docs.

5. **Phase 7 deferred item sweep.** The `CapabilitySet::default()`
   ergonomics deferral has rolled forward through Phases 7 → 8 →
   9 without comment. Phase 9 explicitly decides: either land it
   opportunistically if Task 3 (polish pass fork) or Task 4
   (docs) naturally touches the surface, or re-re-queue it to
   Phase 10 with a single sentence explaining why. No more
   silent rollover.

6. **Phase 9 exit.** Freeze PHASE_9.md, update README.md and
   `docs/ROADMAP.md` for Phase 10, refine the Phase 10 entry with
   whatever Phase 9 taught us about the adapter pattern and
   whichever fork of Task 3 we took.

## Open questions

### Q1. Third-adapter stress test or polish pass for Task 3?

**The central Phase 9 scoping call.** Two forks with materially
different outcomes.

**Fork A: ship a third adapter (likely Matrix or an HTTP-POST
toy).** Pros: the adapter pattern is validated-by-use, not
validated-by-argument. Phase 8's sibling `run_*_session` rejection
of the shared-trait extraction was made on *two* data points, and
every software-architecture principle says "two is not a pattern,
three is." If Matrix lands cleanly by copying `aivyx-telegram`'s
shape with ~100 lines of wiring diffs, we claim the pattern stable
for real. If Matrix forces the extraction, we learn the pattern
was a Phase 8 coincidence and fix it now, not in Phase 10.

Cons: a third adapter is the single largest chunk of work in
either fork. Matrix specifically has its own dependency weight
(the `matrix-sdk` crate is substantial), encrypted-room and
device-verification complexity that's orthogonal to the adapter
pattern, and federation semantics that don't map onto
`session_partition()` cleanly (a Matrix "room" is not a 1:1 user
session, it's an N-user group with its own membership lifecycle).
Shipping Matrix *and* Tasks 1+2 *and* a docs pass is probably too
much for one phase.

**Fork B: ship polish work (unified config, adapter-seam doc, no
third adapter).** Pros: Phase 9 is narrower, faster, and lands
the two deferrals from Phase 8 cleanly without a parallel
adapter track competing for attention. `aivyx-config` is a
real pain-point — the env-var sprawl is already at six vars and
will grow with every future adapter; shipping a unified config
layer now buys everything after Phase 9 a cleaner foundation.
The `ADAPTER_PATTERN.md` doc captures knowledge that's currently
only in my head and Phase 8's ship records, which is a risk:
the next person to add an adapter will re-derive half of it.

Cons: no new empirical data on the adapter pattern. We'd still
be relying on *two* data points at Phase 9 exit. If Phase 10 then
adds a third adapter and it breaks the sibling pattern, we'll
wish we'd stress-tested in Phase 9 when the cost was one extra
adapter, not a post-mortem refactor.

**My lean, writing this at Phase 9 entry:** Fork B. The config
debt is tangible and compounds; the adapter-pattern claim can
stay tentative for one more phase. The explicit promise is that
Phase 10 (or the first future phase that ships a third adapter)
either confirms or refutes the pattern claim with data, and
Phase 9's docs make the pattern cheap to re-derive if the third
adapter uncovers a break.

But I want to make this decision with the user's input rather
than unilaterally, because the trade-off is about project
cadence as much as it is about correctness. **Open for decision
at Task 1 ship.**

### Q2. Does multi-chat pumping require a D2 or D4 amendment?

The Phase 8 `TelegramChannel` is one instance per `chat_id`. The
Phase 9 multi-chat design wants one `TelegramChannel` *per active
chat*, sharing one outer `get_updates` cursor. Two candidate
shapes:

**Option A: `TelegramChannel` stays one-chat-bound; the outer
multiplexer spawns multiple.** Zero trait change. The outer
multiplexer owns the transport and the cursor; each inner
`TelegramChannel` owns its chat_id and buffer. This is the
Phase 8-compatible shape and the one Task 2 assumes.

**Option B: `TelegramChannel` learns a "route this inbound
update to the right internal chat" method.** Requires enriching
the type to hold a map of chat_id → state, which blurs the
boundary between "channel" and "multiplexer" and spills
concurrency control into the channel itself.

Option A is cleaner. The question is whether the outer
multiplexer's lifetime and ownership story introduces a new
shape the D2 trait doesn't currently support — specifically
whether the shared `Arc<dyn Storage>` / `Arc<dyn AuditHook>`
handoff requires any new hook on `ChannelContext`. My prior:
**no amendment needed**. The current trait surface (a channel
is an owned object with its own session, per-turn cancellation
token, and streaming buffer) is independent of how many channels
a process owns. Multi-chat is a binary-level concurrency
concern, not a trait-level contract concern.

If Task 2's implementation discovers otherwise, the Phase 6 Q5
rule applies: honesty over streak preservation, ship an
amendment file, document the need. **Resolution expected:**
during Task 2, with a strong lean toward "no amendment."

### Q3. Where does `aivyx-config` live if Q1 = Fork B?

Two candidate homes:

**Option A: new crate `crates/aivyx-config`.** Clean boundary,
independently testable, easy to reuse across binaries. Cons:
it's a thin crate with few types, and the workspace already has
`aivyx-crypto` / `aivyx-storage` / `aivyx-capability` as examples
of small per-concern crates, so the precedent is fine.

**Option B: module inside `crates/aivyx-channel` (the binary
crate).** Since the binary is the only caller for now, a
`crates/aivyx-channel/src/config.rs` module would be smaller-
footprint and avoid adding a crate for ~200 lines of code. Cons:
if a future tool or adapter wants to read config, it would have
to pull in the binary crate, which is wrong.

**My lean:** Option A (new crate). The binary is only
coincidentally the sole caller today; the Phase 9 non-goal "not
a multi-tenant hosting story" explicitly anticipates a future
where it's not. Making the config layer reusable from day one
costs one crate's worth of boilerplate and saves a refactor
later.

**Open for decision at Task 3 ship**, contingent on Q1 = Fork B.

### Q4. Does the per-task `-D warnings` policy need mechanical enforcement?

Phase 8 Task 8 established the policy *by convention*: run
`cargo clippy --workspace --all-targets -- -D warnings` at every
task, not just at phase exit. Phase 9 has three levels of
enforcement available:

**Level 0: convention only** (current). The phase journal says
"do this," the tasker remembers. Phase 8's Task 4 regression
happened under Level 0.

**Level 1: a pre-commit hook in the repo.** Add `.git/hooks/
pre-commit` (template committed under `scripts/` so contributors
install it) that runs `cargo clippy -- -D warnings` before any
commit. Zero CI change. Cons: hooks are easy to skip with
`--no-verify`, and the Phase 8 working convention explicitly
forbids `--no-verify` already.

**Level 2: CI gate.** A GitHub Actions workflow (or equivalent)
that runs clippy on every pushed branch with `-D warnings` and
fails the PR if it's dirty. The repo does not currently have CI
configured — this would be a new surface.

**My lean:** Level 1 for Phase 9, with Level 2 queued for
whenever CI generally gets set up. Level 1 is one script file,
zero infrastructure, and catches the exact regression shape
Phase 8 Task 4 shipped. **Open for decision at Task 1 ship** (so
the hook lands before the first real Phase 9 code change has a
chance to regress clippy again).

### Q5. Is `namespaced_topic` the right home for per-channel partition logic, or should it move to `aivyx-capability`?

Phase 8 Task 2 put `namespaced_topic` inside
`aivyx_memory::tools` because the first consumer was the memory
tool family. The function builds a physical byte string
(`\x01s\x01<session>\x01<logical>`) that's specific to the
memory substrate's flat topic→entries map. Phase 9's Task 4
(adapter-seam doc) will want to name a single place future tools
should reach for when they need partition-awareness.

If the answer is "no other tool family ever needs this," leave
`namespaced_topic` where it is — it's a memory-tool-specific
helper. If the answer is "yes, future tools will need partition-
awareness too," promote it to `aivyx-capability` (which already
owns `Scope` and is the natural home for cross-tool scope
qualifiers) and export it from there.

**My prior:** leave it where it is. Memory is the only tool
family whose state is *partitioned by session*; file-system
tools use `fs_root` which is per-process, shell-exec tools
shouldn't be scoped to a chat at all, and LLM-provider tools
don't carry state. If Phase 9 ships a tool that contradicts
this prior, we move the helper then. **Resolution expected:**
during Task 4 (doc pass) after a grep across the tool families.

### Q6. Does `LocalChannel::session_partition()` stay `None`, or does it return some local identity?

The Phase 8 `LocalChannel::session_partition()` inherits the
trait default (returns `None`), which means local CLI sessions
share one unpartitioned memory namespace. A user running
`aivyx --channel local` in two terminals talks to the same
memory. The Phase 8 working assumption is that's fine — local
CLI is a single-user affordance — but the distinction deserves
one deliberate look at Phase 9 entry.

Two alternative shapes:

**Option A: keep `None` (current).** Local CLI memory is shared
across invocations. Same as Phase 7 behavior. Pro: nothing to
change. Con: a user who wants two separate memory spaces from
two terminals can't get one.

**Option B: return `Some("<session_uuid>")`** per invocation.
Pro: every local session has its own memory namespace; different
terminals don't bleed into each other. Con: every invocation
starts cold with no memory of prior runs, which **regresses
Phase 6's cross-restart recall story** for the local channel —
that's a bug, not a feature.

**Option C: return `Some("<user>")` or `Some("local")`** — some
stable-per-machine identifier. Pro: cross-restart recall still
works, and there's a clean partition boundary if local ever
grows a second local identity. Con: it's a distinction without
a difference today because there's only one local identity.

**My lean:** Option A (keep `None`) until a real use case forces
Option C. Option B is a non-starter because it breaks Phase 6.
**Resolution expected:** during Task 4 (doc pass) as a one-line
note in `ADAPTER_PATTERN.md`, unless an earlier task surfaces a
reason to change.

### Q7. Should `aivyx-core/src/agent.rs`'s session-injection site keep accepting `Option<String>`, or gain a richer partition type?

Phase 8 Task 2 injects `channel.session_partition()` into tool
input JSON under the `"session"` key as a plain string. The
partition type is `Option<String>` because that was the cheapest
shape and the memory-tool consumer only needed an opaque bag of
bytes.

Phase 9's Task 2 (multi-chat pumping) doesn't need anything
richer — each chat_id is still a single scalar. But Task 3's
third-adapter fork might: a Matrix room has both a `room_id`
and a `homeserver`, and collapsing them into one `String`
loses structure a thread-aware tool might want.

**My lean:** keep `Option<String>` for Phase 9. If Task 3's
third-adapter fork ships and Matrix wants structured identity,
that's a clean follow-up to extend the type — it can be done
as a non-breaking refinement (add a second method with a
default that parses the string, or add a typed wrapper that
round-trips through the current shape). **Resolution
expected:** during Task 3 if Fork A; otherwise defer to the
phase that ships the third adapter.

## Exit criteria (draft — revised as work lands)

- [ ] Tasks 1 and 2 (`/cancel` over Telegram + multi-chat
      pumping) shipped with scripted-transport unit tests
      matching the Task 6 pattern from Phase 8.
- [ ] Task 3 shipped under whichever fork Q1 resolves to. If
      Fork A (third adapter): the new crate exists, the sibling
      `run_*_session` pattern either held cleanly or a
      documented extraction landed. If Fork B (polish pass):
      `aivyx-config` exists with typed config access, and
      Phase 8's env-var sprawl is either replaced or explicitly
      bridged.
- [ ] Task 4 (`ADAPTER_PATTERN.md` or equivalent) shipped as a
      future-proof adapter-addition checklist grounded in Phase
      8 + Phase 9 learnings.
- [ ] Task 5 (`CapabilitySet::default()` ergonomics) either
      landed or explicitly re-queued to Phase 10 with a
      one-sentence reason. No silent rollovers.
- [ ] `cargo test --workspace` green. Net test-count delta ≥
      +10 (Phase 9 is a consolidation phase, not an expansion
      phase, so the heuristic is lower than Phase 8's +20).
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
      clean, run **at every task** per the policy established
      in PHASE_8.md Task 8. Zero regressions surviving more
      than one task.
- [ ] **Either** `DESIGN.md` is still unchanged (streak rolls
      to **nine phases**) **or** a single amendment file under
      `docs/amendments/` documents whichever question required
      a contract change, with a pointer from the affected D2 /
      D4 text. Either outcome is acceptable — honesty over
      streak preservation.
- [ ] Q1 (third adapter or polish), Q2 (multi-chat amendment
      need), Q3 (aivyx-config home, if Q1 = Fork B), Q4 (clippy
      enforcement level), Q5 (`namespaced_topic` home), Q6
      (`LocalChannel::session_partition` return), and Q7
      (partition type richness) all resolved and recorded
      under "Decisions made during Phase 9 that aren't in
      DESIGN.md" in the freeze doc.
- [ ] Phase 8 deferrals paid down: `/cancel` over Telegram is
      no longer a sketch (Task 1), multi-chat pumping is no
      longer a one-chat simplification (Task 2).
- [ ] Phase 10 roadmap entry refined with whatever Phase 9
      uncovered, and the Channel Activation Milestone entry in
      ROADMAP.md is updated with any new adapter tests Phase 9
      added to the deferral pool.

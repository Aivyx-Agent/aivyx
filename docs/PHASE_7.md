# Phase 7 — Hardening (audit persistence first)

**Status:** Active (opened 2026-04-14)
**Predecessor:** [PHASE_6.md](PHASE_6.md) (exit commit `912f022`, frozen at `a21d341`)
**Contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all
LOCKED — unchanged since `e0d6437`, **six phases running**)

This document is the **working journal** for Phase 7. It will churn.
At phase exit it is frozen under the same convention as
[`PHASE_6.md`](PHASE_6.md) — no edits except through commits tagged
`docs(phase-7):`.

## Goal

Close Phase 6's most visible asymmetry: **memory now survives
restarts, but the audit of how memory was written does not**. The
`HmacChainLog` resets on every process start, so every
`AuditEvent::MemoryAccess` tag that Phase 6 made load-bearing for
D1's "memory is a tool" commitment is ephemeral — a crash, or an
attacker who can force one, truncates the chain.

Phase 7 fixes this by landing **persistent audit via
`KeyDomain::Audit`**: every `AuditEvent` the turn loop emits is
written to the encrypted store under a monotonic sequence key, the
HMAC chain key survives reopen, and a cold start can verify the full
chain from disk before handing out the first turn. The headline
outcome: `aivyx` can be killed mid-turn, restarted, and produce a
tamper-evident log of every prior turn's tool calls and memory
accesses — not because the process stayed alive, and not because
logs were flushed to a file, but because the audit chain is its own
persistent substrate with the same crypto contract as session state
and memory.

In the wake of that headline Phase 7 also lands three smaller
hardening items whose shapes are all downstream of the audit
decision: **interactive passphrase prompting** (because the audit
chain key ends up sourced from the passphrase one way or another,
and the rpassword cut is now a ~20-line follow-up), **memory GC /
size caps** (because Phase 6 shipped an unbounded substrate and the
first restart of a real dev box will find one), and **filesystem
permission hardening** on the store + salt sidecar (because a
persistent audit log on a shared Unix box that other users can read
defeats the entire point).

This is the phase that **closes the loop on D2**. Phase 2 shipped
the in-memory HmacChainLog, Phases 3–6 each added new audit event
shapes that depend on it, and Phase 7 is the phase that makes the
chain *durable in the same sense the rest of the stack already is*.

## Non-goals

- **Not an amendment to D2.** Phase 7 builds on the existing
  `AuditHook` / `AuditBridge` / `HmacChainLog` trio from Phase 2
  (contract `ed8da37` of DESIGN.md, still live). If the on-disk
  record shape or the chain-verification story requires a contract
  change, that's a Phase 7 open question (Q4 below) — not a
  foregone conclusion.
- **Not a replacement for `HmacChainLog`.** The chain computation
  stays where it is. Phase 7 adds a second `AuditHook` impl — call
  it `PersistentAuditLog` or similar — that wraps `HmacChainLog`
  and forwards every event to the store. The in-memory layer is
  the source of truth for "has this turn's chain been validated";
  the on-disk layer is the source of truth for "what did the chain
  look like five minutes ago." Same composition pattern Phase 5
  used with `derive_master_key` wrapping the raw Argon2id path.
- **Not a chain-rotation story.** If the HMAC key ever needs to
  rotate, that's a phase of its own — the right shape is probably
  "cut a new chain with a link record back to the old one's tip
  hash," but there's no call for it in Phase 7. The phase's job is
  to make the chain persistent, not to add key-management
  features.
- **Not a multi-consumer audit dispatch.** There is still exactly
  one `AuditBridge` behind `Arc<dyn AuditHook>` at the binary
  edge. A future phase that wants, e.g., to stream audit events to
  syslog *and* persist them to disk will need a fan-out layer —
  but Phase 7 has exactly two consumers (in-memory chain, on-disk
  store) and wires them as a wrapper, not a fan-out.
- **Not cross-topic `memory.read`.** Re-deferred from Phase 6's Q3
  re-queue. Phase 7 has no bandwidth for a new memory surface; the
  substrate stays as-is.
- **Not session-scoped memory qualifiers.** Re-deferred from
  Phase 6's deferred list. No multi-session model in Phase 7.
- **Not an `aivyx-config` crate.** Still a stub. Phase 7 adds two
  new env vars (Q2 and Q3 below) but nothing file-based.
- **Not ecosystem work.** Per `ROADMAP.md`, Ecosystem (remote
  channels, desktop GUI, federation) is firmly Phase 8+. Phase 7
  is the last hardening phase before the core is handed to
  adapters.

## Entry criteria (all met from Phase 6 exit)

- [x] Encrypted storage ships with `KeyDomain::Audit` reachable via
      `storage.domain(KeyDomain::Audit)` (Phase 5 task 2,
      `772c39e`). The domain subkey has existed since Phase 5 but
      has never had a user; Phase 7 is that user.
- [x] `aivyx-audit` ships `HmacChainLog`, `AuditBridge`,
      `AuditEvent`, and `AuditTag` with a chain-verification unit
      test (Phase 2 task 5, `2b6f876`). Phase 7 inherits this
      in-memory contract unchanged.
- [x] Every audit event shape Phase 7 needs to persist is already
      defined: `TurnStarted`, `TurnEnded`, `ToolCall`,
      `CapabilityGranted`/`CapabilityDenied`, and
      `MemoryAccess` — the last of those was Phase 6 task 3's
      contribution, and is the shape that gives audit persistence
      its D1 payoff.
- [x] The `aivyx` binary already holds `Arc<dyn Storage>` and
      `Arc<dyn AuditHook>` side by side in its composition root
      (Phase 5 task 4, `dd175de`, plus Phase 2 task 4, `82e29cd`) —
      Phase 7's wiring will construct a new `PersistentAuditLog`
      that holds both and register it as the single hook.
- [x] Six-phase DESIGN.md empty-diff streak on entry.
      `git diff e0d6437..HEAD -- DESIGN.md` is empty as of
      `a21d341`.

## Refinements queued from Phase 6

The threads Phase 6 left behind, sorted by "likely to bite Phase 7"
vs "safe to punt."

- **`AuditHook::on_event` is sync.** The D2 contract has
  `fn on_event(&self, AuditTag)` as a synchronous method, because
  Phase 2's only concrete impl (`HmacChainLog`) is pure in-memory
  HMAC computation with no I/O. Persistent audit changes that: a
  `put` on an `Arc<dyn Storage>` is `async`, and the session loop
  emits audit events from inside its async runtime. Three options
  for handling the async gap: (a) wrap every storage call in
  `tokio::task::spawn_blocking` inside a sync `on_event`, (b) make
  `AuditHook::on_event` async (first D2 amendment, ends the
  empty-diff streak), (c) queue events to a background task via a
  bounded channel, so `on_event` stays sync and the async write
  happens off the audit path. **This is Phase 7's likeliest
  streak-ender.** See Q4 below.
- **Interactive passphrase prompting.** Re-deferred three phases
  running. Phase 7 touches the passphrase surface directly when it
  decides how the audit chain key is sourced (see Q1), so this is
  the natural phase to light up `PassphraseSource::InteractivePrompt`
  at the same time. `rpassword = "7"` as a new dep. The binary
  should prompt on first-run *and* on any reopen where the env
  var is unset — the current "env var or crash" flow is fine for
  CI and painful for a human opening a laptop lid.
- **`SessionMarker` growing a `last_audit_seq` counter.** The
  40-byte record from Phase 5 has exactly four fields. A fifth
  field (`last_audit_seq: u64`) would let reopen verify the chain
  contains every seq up to the last recorded one *without* scanning
  the whole `KeyDomain::Audit` range. But that's a contract change
  on the session marker (bump from 40 to 48 bytes, plus the salt
  version bump from `aivyx-v1-storage` to `aivyx-v2-storage`).
  **Leaning:** skip for Phase 7 if a range-scan at open is fast
  enough (single-digit ms on a 100k-entry chain is plausible),
  re-evaluate at task 4 if not.
- **`CapabilitySet::default()` ergonomics.** Re-queued for the
  fourth phase running. The binary's capability-set construction
  now has ten `Scope::parse(...).unwrap()` lines. Phase 6 promised
  "if Phase 7 adds another tool family, the helper lands with
  it." Phase 7 isn't adding a tool family — persistent audit is a
  cross-cutting capability, not a new tool — so the helper has
  thin justification *again*. **Leaning:** land it anyway this
  time. The Phase 6 promise was "Phase 7 will be the moment,"
  not "Phase 7 will be the moment if a tool family happens to
  justify it," and re-queuing it a fifth time is how small
  refinements become technical debt.
- **Tool name in `StreamEvent::ToolCallStarted`.** Still deferred.
  Not related to the Phase 7 theme. Re-queue unchanged.
- **Runtime JSON-schema validation for tool input.** Still
  deferred. Unrelated to audit persistence. Re-queue unchanged.

## Draft task breakdown

This is a *draft*. Phase 6's breakdown survived intact from entry
to exit (as did Phases 3–5 before it), but Phase 2's did not —
tasks are allowed to reorder and re-scope as we learn.

1. **Design the `PersistentAuditLog` composition.** ✅ Shipped
   2026-04-14. `crates/aivyx-audit/src/persistent.rs`,
   `PersistentAuditLog` holds `Arc<HmacChainLog>` plus
   `Arc<dyn Storage>`, implements `aivyx_core::AuditHook` directly
   (sibling to `AuditBridge`, not a replacement), and writes each
   event to `KeyDomain::Audit` under `b"a\0" || seq_be`. 10 unit
   tests under `persistent::tests` land the round-trip, reopen,
   wrong-key, seq-monotonic, tamper-detection, truncation-gap,
   corrupt-value, key-layout, and error-handler paths. Workspace
   test count 257 → 267.

   **Resolution of the Q1 / Q2 / Q3 / Q4 gates this task depended
   on:**

   - **Q1 (chain key):** HKDF from master via
     `KeyDomain::Audit`. Exposed through a new public
     `aivyx_crypto::SubKey::as_bytes()` with a doc comment
     naming `PersistentAuditLog` as the single legitimate caller
     and forbidding broader use. The binary extracts the raw
     `[u8; 32]` at wire-up time, so `aivyx-audit` stays free of
     any direct `aivyx-crypto` prod dep.
   - **Q2 (continuous vs per-session chain):** Continuous.
     `HmacChainLog` is keyed once per store and chains across
     every session/reopen. Per-session linking was considered
     and rejected as scope creep — it can be layered on later
     via a `session_marker` event variant without rewriting
     this task.
   - **Q3 (on-disk record shape):** `serde_json::to_vec(&SignedEntry)`.
     The MAC already uses `serde_jcs` for canonical integrity
     bytes; the durable envelope only needs to round-trip the
     struct faithfully, and plain `serde_json` does that for
     every field currently in `SignedEntry`.
   - **Q4 (async trait streak-ender):** **Not an amendment —
     streak holds.** `aivyx_core::AuditHook::on_event` stays
     sync. The async impedance is absorbed inside
     `persistent.rs`: the sync `on_event` path calls
     `HmacChainLog::append` inline (so the chain MAC is
     computed before return, invariant 1), then `try_send`s the
     freshly-chained `SignedEntry` into a bounded tokio mpsc
     channel (capacity 1024). A background drain task — spawned
     by `open`, aborted in `Drop` — consumes the channel in
     FIFO order and persists each entry with `DomainHandle::put`.
     A health flag (`Arc<AtomicBool>`) flips false on the first
     drain failure; subsequent `on_event` calls observe the flag
     and short-circuit through the error handler. `git diff
     e0d6437..HEAD -- DESIGN.md` = empty, now across six frozen
     phases plus the first task of Phase 7.

   **Invariants preserved by the bounded-channel design** (the
   five Task 1 had to defend to claim "audit survives a crash"):
   (1) chain MAC computed synchronously on-path before
   `on_event` returns; (2) drain order = disk order via mpsc
   FIFO and a single writer; (3) persist failures observed ≤1
   event later via the health flag; (4) a chain-rejected event
   is never acked as audited because the handler runs before
   the channel push; (5) drain task lifetime bounded by
   `PersistentAuditLog` via `Drop::abort`.

   **Supporting API additions landed by this task:**
   - `aivyx_crypto::SubKey::as_bytes(&self) -> &[u8; KEY_LEN]`
     — documented single-caller escape hatch.
   - `aivyx_audit::HmacChainLog::from_verified_entries(key,
     entries)` — reopen-path constructor that inserts
     externally-verified entries without recomputing MACs
     (recomputation would mask any tamper the reopen verifier
     missed).
   - `aivyx_audit::AuditError::{Storage, CorruptStoredEntry}`
     — two new variants for the storage-boundary and
     decode-boundary failure modes.

   Task 2 (`verify_from_disk` as a named entry point) and Task 3
   (binary wire-up) remain. Task 2 is partially pre-absorbed:
   verification already runs inside `PersistentAuditLog::open`,
   so Task 2 is mostly about exposing it as a standalone public
   fn for a `--verify-only` CLI mode (Q6) and deciding the
   mandatory-vs-logged policy (Q5).

2. **Ship chain verification on open.** (Shipped 2026-04-14.)
   `PersistentAuditLog::verify_from_disk(storage, audit_key) ->
   Result<VerifyReport, AuditError>` lands as a standalone async
   associated fn: callers hand it an `Arc<dyn Storage>` and a raw
   `[u8; 32]`, it range-scans `KeyDomain::Audit` in seq order,
   decodes each row, and replays the HMAC chain from the genesis
   seed. No live `PersistentAuditLog` is constructed — the
   verify-only path never spawns a drain task, never takes a
   write handle. On success it returns a `VerifyReport` with
   `entries_verified`, `head_seq`, and `head_mac`; on failure it
   returns `AuditError::ChainBroken { seq }` or
   `AuditError::CorruptStoredEntry { seq, reason }` unchanged.

   **Refactor hygiene.** Both `open` and `verify_from_disk` now
   route through a shared private `scan_decode_verify` helper
   that owns the "what counts as a valid on-disk chain"
   definition. The ten pre-existing Task 1 reopen tests doubled
   as the regression harness for the extraction — they would
   have failed loudly if the helper changed behavior. Four new
   tests land for the cold-start path: empty store reports
   `(0, None, genesis)`; five-event write reports
   `(5, Some(4), entries[4].mac)`; tampering `entry1.mac[7]`
   surfaces `ChainBroken { seq: 1 }` (deliberately non-boundary
   to catch seq-off-by-one regressions); two back-to-back
   `verify_from_disk` calls followed by a live `open` all
   succeed, proving no lock is held across await points.

   **Q5 resolution — policy at the boundary.** Neither "mandatory
   at open" nor "logged and continued" became the rule inside
   `aivyx-audit`. `verify_from_disk` returns the error unchanged;
   the binary (Task 3) decides whether to exit non-zero, restore
   from backup, or (in a future dev tool) display the break and
   continue. This keeps aivyx-audit out of any
   `AuditEvent::ChainBreakDetected` schema-extension territory
   that would have amended D4 — the six-phase DESIGN.md empty-
   diff streak survives a second task.

   **Q6 (`--verify-only` CLI mode).** Deferred to Task 3, where
   the binary decides whether to expose the standalone entry
   point as a CLI flag. The library-level plumbing is already in
   place.

3. **Wire `PersistentAuditLog` into the `aivyx` binary.**
   (Shipped 2026-04-14.) `crates/aivyx-channel/src/bin/aivyx.rs`
   now derives the audit chain key via
   `master_key.derive_subkey(b"audit")?.as_bytes()` *before* the
   `RedbStorage::open` call that consumes `master_key`, then
   constructs `PersistentAuditLog::open(storage.clone(),
   audit_chain_key)` inside `run_async`. The old
   `AuditBridge::new(HmacChainLog::new(...))` site collapses
   into a single `Arc::new(persistent_audit)` — no bridge
   intermediary on the persistent path, because
   `PersistentAuditLog` implements `AuditHook` directly. The
   orphan `rand_bytes_from_os` helper is deleted; its single
   caller was the old ephemeral chain-key.

   **Startup banner.** A new `audit: persistent ({N} events
   verified from disk)` line lands in the `SessionConfig::banner`
   block, sourced from `persistent_audit.len()` after `open`
   returns. No second scan — `open` already verified the chain
   on the way in, and reading `len()` is free. The four-line
   banner now reads: version → fs sandbox → memory → audit.

   **Q6 resolution — `--verify-only` ships as option 2.** The
   binary now recognizes a single CLI flag via
   `parse_verify_only_flag()`: bare `--verify-only` with no
   extra args. The verify-only path branches *after* storage
   open but *before* session bring-up: it calls
   `PersistentAuditLog::verify_from_disk(storage, chain_key)`,
   prints `audit: verified N events (head_seq=...)`, and
   returns `Ok(())`. On chain break, the binary maps the
   `AuditError` to `ExitCode::FAILURE` via the existing
   `main`-level `match`. Critically, `ANTHROPIC_API_KEY` is
   **not required** in verify-only mode — an operator running
   forensic verification on a production store should not have
   to hand the cloud key to a read-only tool. Smoke-tested
   against a fresh redb store: exits 0 with
   `audit: verified 0 events (head_seq=none (empty chain))`.

   **Q3d resolution — capability-set helper stays re-queued.**
   The Phase 6 promise at `PHASE_6.md:518-523` was conditional:
   the helper lands *if Phase 7 adds another tool family*.
   Phase 7 tasks 1–8 are all hardening work — no new tool
   family. The binary's five `Scope::parse(...).unwrap()` call
   sites are *less* repetitive than the ten Phase 6 measured,
   so the ergonomics argument is weaker today than when the
   refinement was first re-queued. Helper stays deferred; the
   conditional commitment rolls forward to whichever future
   phase adds the next tool family. The prior Task 3 draft
   text that said "the capability-set helper from the Phase 6
   refinements-queue lands here" was written assuming Phase 7
   would include a tool family, and was stale by the time
   Task 1 shipped.

   **Six-phase DESIGN.md empty-diff streak survives a third
   task-level decision.** Task 3 had three potential streak-
   enders: (a) chain-key derivation route could have needed a
   new `MasterKey::audit_chain_key()` accessor on
   `aivyx-crypto`'s public API — resolved instead by using
   the already-public `derive_subkey` + already-public
   `SubKey::as_bytes`, both of which were in place since
   Task 1. (b) `--verify-only` could have needed a new
   `AuditEvent::ChainVerified` variant — resolved by printing
   the outcome directly to stdout, no audit-event surface
   touched. (c) The banner line could have needed a new
   `SessionConfig::audit_status` field — resolved by
   extending the existing `banner: Option<String>` with one
   more line, same shape.

4. **Interactive passphrase prompting.** Add `rpassword = "7"`
   as a dep on `aivyx-channel`. Light up
   `PassphraseSource::InteractivePrompt` with the real
   `rpassword::prompt_password("aivyx passphrase: ")` call. The
   binary's passphrase-sourcing logic changes from "env var or
   crash" to "env var, or interactive prompt if we have a tty,
   or crash with a clear message." Unit tests cover the
   `passphrase::tests` redaction tripwire (unchanged) plus a new
   `interactive_prompt_reads_from_stdin` test that pipes input.

5. **Memory GC tripwire.** Add a size-cap check inside
   `MemoryWriteTool::execute` that counts the current topic's
   entries before the put and refuses to write if the count
   exceeds a configurable threshold (default 10 000 per topic,
   overridable via `AIVYX_MEMORY_MAX_PER_TOPIC`). Return
   `ToolOutcome::Failed { reason: "topic size cap reached" }`.
   This is not a real GC — it's the "tripwire" version that
   surfaces the unbounded-substrate problem to the agent as a
   tool error, so the planner can call `memory.forget` or move
   to a new topic. Real GC (TTL, LRU, compaction) is re-deferred
   to a future phase. Unit tests add: put-at-cap succeeds,
   put-over-cap fails with the right error, forget-and-retry
   works.

6. **Filesystem permission hardening.** On Linux (the only
   supported platform per D5), `chmod 0600` the store file, the
   salt sidecar, and any audit-chain state file at create time.
   A ~20-line change in `aivyx-channel::passphrase::load_or_
   create_salt` and `aivyx-storage::RedbStorage::open`. Unit
   tests assert `metadata().permissions().mode() & 0o777 == 0o600`.

7. **Scripted audit persistence integration test.** New file
   `crates/aivyx-channel/tests/audit_persistence_e2e.rs`. Same
   shape as Phase 5 task 5's `storage_persistence_e2e.rs` and
   Phase 6 task 5's `memory_tool_e2e.rs`: two sessions against
   the same `$TMPDIR`-based store. Session A runs a scripted
   turn that calls both `memory.write` and `fs.read`, drops,
   session B reopens and verifies the chain from disk, and the
   test asserts the verified chain contains every event session
   A emitted in the right order with the right tags. Add a
   negative case: session C reopens after the test manually
   flips a byte in the on-disk audit state → `verify` must fail
   with a specific `AuditError::ChainBreak { at_seq }` error.

8. **Phase 7 exit.** Freeze `PHASE_7.md`, update `README.md` and
   `ROADMAP.md` for Phase 8, refine the Phase 8 entry with
   whatever Phase 7 taught us. At Phase 7 exit the core should
   be **stable enough to hand to an adapter** — that's the
   prerequisite ecosystem work has been waiting on since Phase 0.

## Open questions

### Q1. Where does the audit chain HMAC key come from?

**Status:** open at phase entry. Must resolve before task 1.

Phase 2's `HmacChainLog::new` takes a `[u8; 32]` key parameter and
the binary passes `[0u8; 32]` as a placeholder because "every
restart resets the chain anyway." Phase 7 needs a real key. Four
options:

1. **HKDF from the master key via a new `KeyDomain::Audit` subkey
   info.** Phase 5 already derives five domain subkeys at
   `RedbStorage::open`; a sixth derivation (`info =
   b"aivyx-v1-audit-chain"`) is one line and gives us a key that
   rotates with the master passphrase. Symmetric with how every
   other crypto secret in the project is sourced.
2. **Stored in `KeyDomain::Secrets` under a known key at first
   open, generated randomly.** The chain key is independent of
   the master and survives a passphrase change — but adds a
   "first run" branch to the open path and a second thing to
   back up.
3. **Hand-rolled derivation via `derive_master_key` with a
   chain-specific salt.** Re-runs Argon2id per session, which is
   too expensive for something that happens at every open.
   Dismissed.
4. **An env var `AIVYX_AUDIT_KEY` that the operator sets.** Makes
   the chain verifiable on a machine that has the key without
   also having the passphrase — useful for split-trust audit
   review workflows — but adds a new secret to manage and
   duplicates what (1) gives for free in the single-user case.

**Leaning:** (1). Derive the HMAC key via HKDF from the master
with `info = b"aivyx-v1-audit-chain"`. That's symmetric with the
Phase 5 pattern, requires zero new fields on `RedbStorage::open`,
and means "the same passphrase unlocks the chain" is the full
story. Option (4) is a real capability (remote audit review) but
it belongs to whatever phase actually builds that workflow, not
Phase 7.

### Q2. What is "chain start" on reopen — continuous or per-session?

**Status:** open at phase entry. Must resolve before task 1.

Two shapes:

1. **Continuous chain.** There is exactly one HMAC chain per
   store, growing forever. Session B's first event links to the
   last event session A wrote. Simple, one MAC per event, but a
   single truncation attack removes all history past the
   truncation point — if an attacker crashes the process right
   after turn N and then deletes every record after N, the chain
   still verifies and the `at_seq = N+1` evidence is gone.
2. **Per-session chain with explicit link records.** Each session
   starts a new chain; session B's first record is a special
   `ChainLinked { prev_tip_hash, prev_tip_seq }` event that
   embeds the tip of session A's chain. An attacker who wants to
   hide session A must also hide session B's link record (and
   session C's, and so on) — strictly stronger tamper-evidence.
3. **Merkle tree over sessions.** Each session is a chain; a
   separate tree links session tips. Overkill.

**Leaning:** (2). The linking cost is one extra record per
session open, the verification cost is one extra MAC per
session boundary at `verify_from_disk`, and the security
property is qualitatively stronger. The Phase 6 bug catch
("audit tags earn their own assertions") is the lesson that
applies here: named invariants are harder to silently break
than inferred ones. A `ChainLinked` record is a named
invariant; "the seq after N exists somewhere" is not.

### Q3. What's the on-disk record shape for each `AuditEvent`?

**Status:** open at phase entry. Must resolve before task 1.

Each persisted row needs to hold: the event itself, the HMAC,
and enough metadata to verify the chain. Three options:

1. **`serde_json::to_vec(&AuditEvent)` + a separate `mac` field
   in an outer record.** Same pattern Phase 6 used for
   `MemoryEntry`. Zero new deps, human-readable in debug dumps,
   slightly bloated.
2. **`bincode` or `postcard`.** Compact, fast, a new dep.
3. **Hand-rolled layout.** Every schema change is a migration,
   and `AuditEvent` has grown three times in the project's
   history (Phase 2, Phase 3, Phase 6) — the next two growths
   are near-certain.

**Leaning:** (1) on symmetry. Phase 6 resolved Q1 the same way
for memory entries and cited the same reasons — zero new deps,
the store is encrypted so on-disk readability matters only to
tests, and the integration test can assert-against-JSON for
free. A growing enum with a serde layout is exactly the case
`serde_json` handles gracefully.

The outer record layout: `{seq: u64, prev_mac: [u8; 32],
event: serde_json::Value, mac: [u8; 32]}` encoded as JSON.
`prev_mac` is redundant with "read the prior row," but having
it inline means `verify_from_disk` only needs a single forward
pass without a lookback.

### Q4. Does `AuditHook::on_event` become async?

**Status:** open at phase entry. Must resolve before task 1.
**This is the phase's streak-ender candidate.**

The D2 trait today:

```rust
pub trait AuditHook: Send + Sync {
    fn on_event(&self, event: AuditTag);
}
```

Persistent audit requires writing to an `Arc<dyn Storage>`,
whose methods are all `async`. Three paths:

1. **Keep `on_event` sync; wrap the storage call in
   `tokio::task::spawn_blocking` inside the impl.** Zero
   contract change, but `spawn_blocking` over a tokio-aware
   handle (redb's own `spawn_blocking` wrapper) is wasteful —
   we'd be blocking a blocking thread to schedule an async
   task that itself spawns a blocking thread. Also: if the
   session loop emits an event from an async context, this
   path requires a runtime handle, which the current
   synchronous `AuditHook` doesn't have in its signature.
2. **Make `on_event` async (first D2 amendment).** Change the
   trait to `async fn on_event(&self, event: AuditTag)`, update
   `HmacChainLog` to implement it (trivial — the body is still
   sync), update `AuditBridge` to propagate the async call,
   update every call site in `aivyx-core::agent`,
   `aivyx-core::session`, and `aivyx-core::tools::fs` /
   `aivyx-memory::tools` to `.await` the hook. This is a
   cross-crate change with a measurable blast radius, and it
   ends the six-phase empty-diff streak — the first amendment
   file under `docs/amendments/`. It is also strictly correct:
   persisted audit is genuinely async work, and hiding that
   behind a sync facade is exactly the kind of "lies about I/O"
   shape that Phase 3's cancellation story taught us to avoid.
3. **Add a background writer task via a bounded channel.**
   Construct a `tokio::sync::mpsc::channel::<AuditEvent>(N)` at
   binary startup, spawn a background task that drains the
   channel and writes to storage, and have `on_event` do a
   non-blocking `try_send` (falling back to a blocking send if
   the channel is full). `on_event` stays sync, the D2 contract
   doesn't move, and the async write happens off-path. The
   downside: backpressure becomes asynchronous, so a write
   failure *after* the session loop has moved on is orphaned —
   the loop already believes the audit write succeeded. That's
   a real weakening of the audit guarantee: right now an event
   either makes it into `HmacChainLog` or doesn't, and the loop
   sees the failure synchronously.

**Leaning:** (2), with amendment. Option (3)'s orphan-write
failure mode is exactly the class of bug audit persistence is
supposed to eliminate — if a write fails silently the whole
point of the chain is gone. Option (1) is technically feasible
but awkward in a way that will age badly. Option (2) is a real
contract change, writes the first amendment file, and ends the
streak, but **the streak is not worth lying for** (the exact
Q5 rule from Phase 6). If Q4 resolves to (2), Phase 7 will
draft the amendment carefully, document it in the amendment
file with a pointer from the `AuditHook` trait's doc comment,
and note the streak break in the freeze doc alongside a clean
explanation of why it was the right call.

**If Q4 goes option (1) or (3):** the streak holds at seven.
**If Q4 goes option (2):** the streak breaks at six, Phase 7
ships `docs/amendments/2026-04-XX-audit-hook-async.md`, and
that's the right outcome because the contract is wrong.

### Q5. Is chain verification at open mandatory or logged-and-continued?

**Status:** open at phase entry. Resolves during task 2.

When `PersistentAuditLog::verify_from_disk` detects a chain
break (tampered record, truncation, missing seq), two responses:

1. **Refuse to start.** The binary prints the error and exits
   non-zero. Maximum safety; an operator who sees this must
   either restore from backup or accept the tamper evidence and
   start a fresh chain via an explicit `--reset-audit` flag.
2. **Log and continue.** Print a loud stderr warning, record a
   synthetic `AuditEvent::ChainBreakDetected { at_seq, reason }`
   as the new chain's first event, and proceed. This is the
   Phase 5 pattern for session-marker decode failures: the
   system continues, the problem is visible, the audit trail
   *records* the detection rather than hiding it.

**Leaning:** (2). The Phase 5 session-marker path chose
log-and-continue on the explicit reasoning that "storage is
observable but not fatal" — but audit persistence inverts that
ratio, because the whole point is to detect tampering. Still,
option (2) has a subtle advantage: it turns the chain break
*into an audit event*, which means a downstream monitoring tool
can page on it without also having to parse exit codes. Option
(1) is strictly safer but operationally unfriendly in the way
"refuse to start if the session marker decodes wrong" would
have been unfriendly in Phase 5. Revisit at task 2 once there's
a real chain to verify.

### Q6. Does the `aivyx` binary need a `--verify-only` mode?

**Status:** open at phase entry. Nice-to-have; may defer.

A future audit-review workflow wants "run the binary against an
existing store, verify the chain, exit without starting a
session." This is a 10-line addition to the binary's arg
parsing and a direct call to `verify_from_disk`, but it's a new
user-facing surface and every such surface wants UX design. Not
a Phase 7 requirement — the integration test in task 7 drives
`verify_from_disk` directly without going through the binary —
but it'd be useful for dev-box debugging.

**Leaning:** ship if it falls out for free in task 3's binary
wiring; defer to Phase 8+ otherwise.

## Exit criteria (draft — revised as work lands)

- [x] `aivyx-audit::PersistentAuditLog` exists, implements
      `AuditHook`, and has unit-test coverage of: in-memory +
      persist round-trip, reopen-and-verify, wrong-key negative,
      seq monotonicity across reopen, chain-break detection
      (tampered byte at mid-sequence), and truncation detection.
      *(Task 1, 2026-04-14. 10 new `persistent::tests` lines;
      workspace 257 → 267.)*
- [x] Chain verification at `PersistentAuditLog::open` runs
      against the on-disk range-scan and returns a typed result
      a caller can branch on. *(Task 2, 2026-04-14. Shipped
      as a standalone `verify_from_disk(storage, key) ->
      Result<VerifyReport, AuditError>` associated fn; shared
      `scan_decode_verify` helper now backs both `open` and
      verify-only; 4 new tests; workspace 267 → 271. Q5 resolved
      as policy-at-the-boundary: error returned unchanged, no
      `AuditEvent::ChainBreakDetected` schema touch, DESIGN.md
      streak preserved.)*
- [x] The `aivyx` binary constructs a `PersistentAuditLog` in
      place of the bare `HmacChainLog`, verifies the chain on
      startup, and surfaces the verified event count in the
      startup banner. *(Task 3, 2026-04-14. Audit chain key now
      derived from `master_key.derive_subkey(b"audit")` before
      `RedbStorage::open` consumes the master key; old
      `rand_bytes_from_os` ephemeral helper deleted. New
      `--verify-only` CLI flag ships Q6 as a read-only forensic
      surface that runs `verify_from_disk` without requiring
      `ANTHROPIC_API_KEY`. Banner gains the
      `audit: persistent (N events verified from disk)` line.
      Q3d: capability-set helper stays re-queued — Phase 7 has
      no new tool family, Phase 6's conditional promise does
      not trigger. DESIGN.md empty-diff streak preserved a
      third task in a row.)*
- [ ] `PassphraseSource::InteractivePrompt` is lit up behind
      `rpassword`, not a stub.
- [ ] `MemoryWriteTool` refuses writes above
      `AIVYX_MEMORY_MAX_PER_TOPIC` (default 10 000) with a
      typed `Failed` outcome the planner can observe.
- [ ] The store file, its salt sidecar, and any audit-chain
      state files are `chmod 0600` at create time on Linux.
- [ ] A scripted integration test
      (`crates/aivyx-channel/tests/audit_persistence_e2e.rs`)
      drives a two-session audit round-trip *and* reopens a
      third session with a tampered audit state to assert the
      chain-break detection fires with a specific
      `at_seq`.
- [ ] `cargo test --workspace` green.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] **Either** `DESIGN.md` is still unchanged (streak rolls to
      seven) **or** a single amendment file under
      `docs/amendments/` documents the Q4 `async fn on_event`
      refinement with a pointer from D2's trait text. Either
      outcome is acceptable — **honesty over streak
      preservation**, the Phase 6 Q5 rule.
- [ ] Q1 (chain key source), Q2 (continuous vs per-session
      chain), Q3 (on-disk record shape), Q4 (async trait?), and
      Q5 (mandatory-or-logged verification) resolved and noted
      under "Decisions made during Phase 7 that aren't in
      DESIGN.md" in the freeze doc — regardless of which option
      won.
- [ ] At least one Phase 6 queued refinement either landed or
      explicitly re-queued to Phase 8+ with a reason. The
      interactive-passphrase refinement is the headline one; the
      `CapabilitySet::default()` ergonomics is the small one
      Phase 6 committed to landing in Phase 7.
- [ ] Phase 8 roadmap entry refined with whatever Phase 7
      uncovered — this is the first phase where "Phase N+1 is
      Ecosystem" is a real answer rather than a placeholder.

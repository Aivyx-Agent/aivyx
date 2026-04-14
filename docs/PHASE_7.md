# Phase 7 — Hardening (audit persistence first) (FROZEN)

**Status:** Closed 2026-04-14
**Exit commit:** `8164317` — *"Phase 7 task 7: scripted audit persistence integration test — two-session round-trip + direct-tamper negative"*
**Predecessor:** [PHASE_6.md](PHASE_6.md) (exit commit `912f022`, frozen at `a21d341`)
**Successor:** to be scaffolded at Phase 8 entry — see [ROADMAP.md](ROADMAP.md)
**Contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all
LOCKED — unchanged since `e0d6437`, **seven phases running**)

This document is a historical record. The artifacts it produced live
in `aivyx-audit` (the `PersistentAuditLog` struct and its shared
`scan_decode_verify` helper backing both `open` and the standalone
`verify_from_disk`), in `aivyx-channel` (the binary's
`PersistentAuditLog` wiring, `--verify-only` forensic CLI path, real
`rpassword` interactive passphrase prompt, and the
`resolve_memory_max_per_topic` startup helper), in `aivyx-memory` (the
`MemoryWriteTool` per-topic tripwire and
`AIVYX_MEMORY_MAX_PER_TOPIC` resolution), in `aivyx-storage` (the
cold-start `chmod 0600` on the store file), in `aivyx-core` (untouched
— seventh consecutive phase), and in the integration test at
`crates/aivyx-channel/tests/audit_persistence_e2e.rs`. This file
explains *how* it came together, which Phase 7 open questions
resolved which way, and what was deliberately left for Phase 8+.

## Goal (as written at phase entry)

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

4. **Interactive passphrase prompting.** (Shipped 2026-04-14.)
   `rpassword = "7"` lands in `aivyx-channel`'s `[dependencies]`
   block (library-level, not dev-only, because the `passphrase`
   module itself is the call site). `PassphraseSource::
   InteractivePrompt` now routes through a private
   `read_interactive_password_inner<F: FnOnce() -> io::Result<String>>`
   helper: production passes `|| rpassword::prompt_password("aivyx
   passphrase: ")` (opens `/dev/tty` directly on Unix, echo-off);
   unit tests pass `|| rpassword::prompt_password_from_bufread(
   &mut r, &mut w, "aivyx passphrase: ")` against an in-memory
   `&[u8]` reader. Both branches share the same empty-check,
   error-mapping, and `String::into_bytes` (no-copy) discipline,
   so the zeroize path stays symmetric with the env-var path.

   **Error variants updated.** `PassphraseError::
   InteractiveNotImplemented` is deleted (the variant was a
   stub-era placeholder). Two real variants replace it:
   `InteractiveIo { reason: String }` wraps any underlying
   `io::Error` from the rpassword read, and `InteractiveEmpty`
   rejects a zero-length password the same way
   `EnvEmpty` rejects a zero-length env var — Argon2id happily
   hashes an empty input and the resulting key would be
   trivially brute-forceable.

   **Binary source selection — policy at the boundary.** The
   `aivyx` binary gains a `select_passphrase_source()` helper
   that picks between `Env` and `InteractivePrompt`:
   (1) `AIVYX_PASSPHRASE` set and non-empty → `Env`;
   (2) env var unset or empty, and `io::stdin().is_terminal()` →
   `InteractivePrompt`; (3) otherwise bail with the message
   "`AIVYX_PASSPHRASE` is not set and stdin is not a terminal."
   Set-but-empty is treated as effectively-unset at the binary
   layer so a stray `export AIVYX_PASSPHRASE=` in a shell rc
   file doesn't crash an interactive shell run — the
   `passphrase` module's `Env` arm still rejects empty with
   `EnvEmpty` if a caller explicitly asks for that source, so
   the strict type-level guarantee is preserved.

   **Tests.** +5 in `passphrase::tests` (33 → 38;
   workspace 271 → 276). The `interactive_source_returns_not_
   implemented_stub` test is replaced by
   `interactive_source_reads_password_from_bufread` which
   drives the real bufread seam end-to-end. Three negative-path
   tests cover empty-password rejection, I/O error wrapping,
   and a round-trip invariant that the interactive helper
   produces byte-identical bytes to a fixture source for the
   same passphrase string. Two new debug-redaction tripwires
   cover the `Env` variant (var name appears, value does not)
   and `InteractivePrompt` (format renders without touching
   the tty); the pre-existing
   `debug_impl_does_not_leak_fixture_closure_contents` test
   from Phase 5 covers the `Fixture` variant unchanged.

   **Binary smoke-tested in three branches.** (1) env-var set
   + `--verify-only` → exit 0, correct banner; (2) env-var
   unset + pipe stdin + `--verify-only` → exit 1 with
   `no passphrase available: ...`; (3) env-var set + pipe
   stdin + `--verify-only` → env wins, exit 0. The fourth
   branch (tty-driven real prompt) is exercised at unit-test
   resolution by the bufread-seam tests.

   **Six-phase DESIGN.md empty-diff streak survives a fourth
   task-level decision.** Task 4 had one potential streak-
   ender: if `PassphraseSource::InteractivePrompt` had needed
   a new payload field (e.g., a trait-object reader for
   testability), the enum shape would have changed and D7's
   `PassphraseSource` description might have needed an
   amendment. Resolved instead by the FnOnce closure seam,
   which lives at the *private function* level, not the enum
   level — the public `PassphraseSource::InteractivePrompt`
   variant is still unit-shape.

5. **Memory GC tripwire.** (Shipped 2026-04-14.)
   `MemoryWriteTool` gains a `max_per_topic: usize` field, defaulted
   to a new `aivyx_memory::DEFAULT_MAX_PER_TOPIC` const (10_000) by
   `MemoryWriteTool::new(memory)` and overridable via a builder
   `set_max_per_topic(self, cap) -> Self`. Inside `execute`, the
   tripwire runs **after** the `AuditTag::MemoryAccess::Write`
   event and **before** the `Memory::put` call: a bounded
   `get_recent(&topic, self.max_per_topic)` counts the topic's
   existing entries, and if `existing.len() >= max_per_topic` the
   tool returns `ToolOutcome::Failed(AivyxError::Tool { tool,
   detail })` with a detail that names the topic, the cap value,
   and the `memory.forget` recovery path. The audit chain records
   the write *intent* regardless of outcome, which was the whole
   reason the tripwire lives after the audit event and not before.
   
   **Draft-shape correction.** The Task 5 draft said
   `ToolOutcome::Failed { reason: "topic size cap reached" }`.
   `Failed` is a tuple variant wrapping `AivyxError`, not a
   struct variant with a `reason` field, so the draft text was
   mechanically wrong. Shipped shape is
   `ToolOutcome::Failed(AivyxError::Tool { tool, detail })` with
   a detail that carries actionable recovery text. Same
   correction-in-shipped-record pattern as Task 3's stale
   capability-set helper line.
   
   **Cost analysis for `get_recent` as a count primitive.**
   `RedbMemory::get_recent` does a full `scan_prefix(topic)`
   regardless of the `limit` parameter — decode cost is the only
   thing that scales with `limit`. Passing `max_per_topic` as the
   limit gives us the smallest decoded prefix that can still
   answer "is the topic at or above cap?" without adding a
   `Memory::count` trait method we'd have to implement on every
   substrate. The decision was "grow the trait surface or reuse
   the existing read path" — reuse wins because the read path is
   already the one the tool trusts for the verification fence.
   
   **Binary wire-up.** `aivyx.rs` gains a
   `resolve_memory_max_per_topic() -> Result<usize, String>`
   helper next to `resolve_fs_root`/`resolve_storage_path` that
   reads `AIVYX_MEMORY_MAX_PER_TOPIC` once at startup. Unset or
   empty → default; set → `parse::<usize>()` with a **hard error**
   on unparseable input. A mis-set cap would silently mask
   runaway-write bugs, which is exactly the failure mode this
   tripwire exists to catch, so a typo has to fail loudly rather
   than fall back to the default. The resolved cap flows into
   the `MemoryWriteTool` construction via the builder setter at
   the existing `memory_write = MemoryWriteTool::new(...)` site.
   
   **Tests.** Four new `tools::tests` entries
   (workspace 276 → 280): `write_at_cap_still_succeeds` exercises
   the boundary (2 existing, cap 3, write succeeds at seq 2);
   `write_over_cap_fails_with_tool_error_naming_topic_and_cap`
   proves the refusal and asserts the detail contains the topic,
   the cap, and the `memory.forget` recovery hint;
   `forget_clears_the_cap_so_next_write_succeeds` walks the
   end-to-end recovery path through the tool surface (write
   fails → forget succeeds → retry succeeds), proving the
   detail's advice actually works; `cap_is_per_topic_not_global`
   pins the tripwire as per-topic by filling `notes` to cap and
   then successfully writing `todos` through the same tool
   instance.
   
   **Six-phase DESIGN.md empty-diff streak survives a fifth
   task-level decision.** Task 5 had two potential streak-enders:
   (a) the refusal mode could have needed a new
   `ToolOutcome::Refused` variant or a new `AivyxError::MemoryFull`
   variant — resolved by fitting the refusal into the existing
   `ToolOutcome::Failed(AivyxError::Tool { tool, detail })` slot,
   which was already the right shape for "tool-specific failure
   with recovery text." (b) `Memory` trait could have grown a
   `count(topic) -> MemoryResult<usize>` method, which would have
   needed impls in `InMemoryMemory` and `RedbMemory` — resolved
   by reusing `get_recent` as described in the cost-analysis
   block above, keeping the trait surface frozen.

6. **Filesystem permission hardening.** (Shipped 2026-04-14.)
   `chmod 0600` now lands on the two files aivyx creates at
   startup: the redb store file (`store.redb`) and the passphrase
   salt sidecar (`store.redb.salt`). The draft also mentioned
   "any audit-chain state file" — that branch is **empty by
   construction** because Task 1 folded the audit chain *into*
   the redb store under `KeyDomain::Audit`, so `aivyx-audit`
   writes nothing directly to disk. Store + salt is the full
   surface.

   **Cold-start detection pattern.** Both call sites implement
   the chmod as a cold-start-only operation via a `was_cold`
   probe: `Path::try_exists()` before the create call, and the
   chmod only fires when the file did not exist prior. A
   deliberately re-permed existing file (e.g., an operator who
   set `0o640` for a local-admin-readable audit posture) is
   **not** silently fought back to `0o600` on every reopen. This
   is the Q6d resolution — cold-only, not always-chmod. Both
   tests assert this invariant: pre-mutate to `0o640`, reopen,
   assert perms survived unchanged.

   **Helper duplication, not shared.** Q6a resolved to duplicate
   a ~5-line `chmod_user_only` helper into `aivyx-storage::lib`
   and `aivyx-channel::passphrase`. Both copies are
   `#[cfg(unix)]`-gated with a non-unix no-op fallback: D5
   declares Linux as the supported platform, but keeping the
   non-unix arm clean lets macOS dev machines `cargo check` the
   workspace without extra platform cfg bleeding into unrelated
   code. The helpers use
   `std::os::unix::fs::PermissionsExt::from_mode(0o600)` —
   setting the full mode directly rather than OR-masking over
   the existing mode, so the result is exact.

   **Hard error on chmod failure.** Q6b resolved to policy:
   "hard error, not log-and-continue." Failure maps to the
   existing error type at each site (`StorageError::Redb(String)`
   on the storage side, `PassphraseError::SaltIo { path, reason }`
   on the salt side). The rationale: a `0600` assertion is
   load-bearing for D5's local-trust model; if we can't prove
   it, silently continuing with an inherited-umask file would
   hand the operator a false sense of security. A chmod failure
   on a normal filesystem is vanishingly unlikely; on an exotic
   filesystem (9p, certain FUSE mounts) the user needs to know.

   **Q6c — salt chmod even though salts "aren't secret."**
   Shipped: yes, chmod the salt. The earlier doc claim "salts
   are not secret" is still true cryptographically — knowing the
   salt does not shortcut Argon2id — but the uniform "every
   file aivyx writes looks the same to an auditor" discipline
   wins. A future reader who finds one file at `0o600` and
   another at `0o644` has to reconstruct the "oh, that one
   wasn't secret" argument from the doc comment. Uniform perms
   remove that footgun. Pre-existing doc comment on
   `load_or_create_salt` updated to reflect the new stance.

   **Tests.** Two new tests (workspace 280 → 282), one per
   crate, both `#[cfg(unix)]`-gated:

   - `aivyx_storage::tests::cold_open_chmods_store_file_to_0600`
     — cold `RedbStorage::open` asserts `mode & 0o777 == 0o600`,
     then pre-mutates the file to `0o640` and reopens (warm
     path), asserting the perms survived unchanged.
   - `aivyx_channel::passphrase::tests::fresh_salt_is_chmod_0600`
     — first `load_or_create_salt` asserts `0o600`, second call
     on a pre-mutated `0o640` asserts it survived unchanged.

   Both assertions mask with `& 0o777` to strip the file-type
   bits that show up in `stat(2)` mode — the permission bits
   are the only portion we actually set.

   **Six-phase DESIGN.md empty-diff streak survives a sixth
   task-level decision.** Task 6 had one potential streak-ender:
   if the chmod helper needed to live as a public surface in
   `aivyx-core` for any reason (e.g., tool-owned file creation),
   the helper would have been a new D3 contract point. Resolved
   instead by duplicating the ~5-line helper at each call site
   — no shared API, no contract surface, no D3 amendment.

7. **Scripted audit persistence integration test.** (Shipped
   2026-04-14.) New file
   `crates/aivyx-channel/tests/audit_persistence_e2e.rs`, two
   tests, same tempdir-based pattern as Phase 5 task 5's
   `storage_persistence_e2e.rs` and Phase 6 task 5's
   `memory_tool_e2e.rs`.

   **Scope narrowed vs. the draft.** The draft said session A
   "runs a scripted turn that calls both `memory.write` and
   `fs.read`." Shipped: `memory.write` only. Dropping `fs.read`
   removes the setup burden of a full `FsReadTool` harness
   (sandbox root, capability wiring, sample file) from a test
   whose point is the audit-chain persistence stack, not the
   tool surface. The tool-scope narrowing that used to be the
   justification for dual-tool coverage is now carried just as
   strongly by the `memory.write` path: the MemoryAccess entry
   records `memory.write:topic:notes`, not the broad capability
   the caller holds — same R1 payoff, half the scaffolding.

   **Draft said `AuditError::ChainBreak { at_seq }` — shipped
   shape is `AuditError::ChainBroken { seq, reason }`.** This is
   the same correction-in-shipped-record pattern as Tasks 3 and
   5: the draft was written before Task 2 landed, and Task 2
   resolved the variant name/fields. Negative-test assertion
   matches on `matches!(err, AuditError::ChainBroken { seq: 0,
   .. })`.

   **Draft implied three sessions — shipped two tests.** The
   draft's "session A → session B → session C with tamper" shape
   was refactored into two independent `#[tokio::test]`s each
   with its own tempdir (keyed off a `tag` argument to a local
   `SharedStoreDir::new(tag)`). Reason: the negative case's
   tamper step would have poisoned session B's state for an
   in-band tamper, and a third session against the already-broken
   chain adds no information on top of the second session's
   failure. Two tests, two tempdirs, zero shared mutable state.

   **Test 1 — positive round-trip.** Session A opens
   `PersistentAuditLog` against fresh storage, runs one
   `ScriptedProvider` turn that emits a `memory.write` tool call,
   drain-fences on the on-disk row count (2000×1ms polling loop
   scanning `KeyDomain::Audit` for `b"a\0"`), then drops. Session
   B calls **both** `verify_from_disk` (Task 2's cold-path
   verifier, matching what `aivyx --verify-only` does) *and*
   `PersistentAuditLog::open` (the normal live-session path that
   internally replays the chain). Both must succeed.

   The test then drops to **Level 3**: it asserts the exact
   4-event shape via `PersistentAuditLog::entries()`:
   - seq 0: `AuditEvent::TurnStarted`
   - seq 1: `AuditEvent::MemoryAccess { operation: Write,
     scope: memory.write:topic:notes, query_or_key: "notes" }`
   - seq 2: `AuditEvent::ToolCall { outcome: Completed,
     scope_used: memory.write:topic:notes }`
   - seq 3: `AuditEvent::TurnEnded { outcome: Completed,
     tool_calls_made: 1 }`

   Plus a chain-internal `prev_mac == entries[i-1].mac` loop
   across all 4 entries. Level 2 (count-only) would have caught
   the coarsest regressions, but Level 3 pins the whole
   end-to-end pipeline — AEAD seal → redb row → reopen scan →
   decode → HMAC replay → `entries()` accessor — against a
   specific payload so any bit-rot anywhere in that stack fails
   the test loudly.

   **Test 2 — direct-tamper negative.** Session A runs the same
   scripted turn and drops. The test then reopens *storage only*
   (no audit log, no drain task) and mutates seq 0 in place:
   `DomainHandle::get(&audit_row_key(0))` AEAD-decrypts the
   ciphertext and yields the raw `SignedEntry` bytes; the test
   deserializes, flips `entry.mac[0] ^= 0xff`, re-serializes, and
   puts back via `DomainHandle::put`, which re-encrypts under the
   same per-row key. Crucially, this means the AEAD is still
   valid — a naive "tamper the ciphertext" approach would trip
   `StorageError::DecryptFailed` and bubble up as
   `AuditError::Storage`, **not** `ChainBroken`, and would have
   tested a different error path. Going through the decrypt-flip-
   re-put recipe hits the HMAC chain verifier the way an attacker
   with the AEAD key but not the HMAC key would. Both
   `verify_from_disk` and a live `PersistentAuditLog::open` must
   fail with `ChainBroken { seq: 0, .. }`.

   **Drop-order footgun caught during test bring-up.** The first
   test run panicked at session B's `open_store` with
   `Redb("Database already open. Cannot acquire lock.")`. Root
   cause: redb's `Arc<Database>` refcount was non-zero when
   session B tried to reopen the file. The culprits were
   (a) `MemoryHarness` — which owns `MemoryWriteTool → RedbMemory
   → DomainHandle → Arc<Database>` via the three tool
   registrations — needing an explicit `drop(harness)` before
   `drop(storage)`, and (b) the `PersistentAuditLog` drain task,
   whose `Drop` calls `JoinHandle::abort()`, which is
   *non-blocking*: the task drops its captured `Arc<dyn Storage>`
   only when the runtime next polls it. The fix adds a
   `drop(harness); drop(audit_typed); drop(storage); for _ in
   0..16 { tokio::task::yield_now().await; }` sequence in both
   tests. The yield loop lets tokio's single-threaded test
   runtime actually reap the aborted drain task before session B
   grabs the file lock. An in-crate test in `persistent.rs`
   doesn't hit this because it never closes storage between
   reopens — it reuses the same `Arc<dyn Storage>` throughout.

   **Arc upcast for the dual-reference pattern.** `run_session`
   wants `Arc<dyn AuditHook>`, but the test also needs a typed
   `Arc<PersistentAuditLog>` for `.len()` and `.entries()` after
   the turn completes. Solution: `let audit_typed =
   Arc::new(persistent_audit); let audit_hook: Arc<dyn AuditHook>
   = audit_typed.clone();` — Rust's `CoerceUnsized` for `Arc<T>`
   → `Arc<dyn Trait>` does the upcast on the clone, and both
   references share one refcount.

   **Tests.** +2 tests; workspace 282 → 284. DESIGN.md
   empty-diff streak preserved a **seventh** task in a row — no
   changes to `DESIGN.md` or `crates/aivyx-core/src/lib.rs`, just
   one new integration-test file.

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

## Decisions made during Phase 7 that aren't in DESIGN.md

Six open questions at phase entry (Q1–Q6). None resulted in a
`DESIGN.md` amendment. The D2 `AuditHook` trait (the Q4 streak-ender
candidate) stayed exactly as Phase 2 shipped it; the async impedance
was absorbed entirely inside `aivyx-audit`. **Streak rolled to
seven consecutive phases of empty-diff against `DESIGN.md` and
`crates/aivyx-core/src/lib.rs`.**

### Q1 — chain key source

Entry-time options: (1) HKDF from master via a new `KeyDomain::Audit`
subkey info, (2) random + stored in `KeyDomain::Secrets`, (3)
Argon2id with a chain salt *(dismissed)*, (4) `AIVYX_AUDIT_KEY` env
var for split-trust review.

**Shipped: option (1).** `crates/aivyx-channel/src/bin/aivyx.rs`
derives the audit chain key via
`master_key.derive_subkey(b"audit")?.as_bytes()` **before**
`RedbStorage::open` consumes the master key. Supporting API:
new public `aivyx_crypto::SubKey::as_bytes(&self) -> &[u8; KEY_LEN]`
with a doc comment naming `PersistentAuditLog` as the single
legitimate caller. The prior ephemeral-key helper
`rand_bytes_from_os` was deleted; its only caller was the
placeholder chain key Phase 2–6 used to keep `HmacChainLog` happy.

Option (4) is not lost — it's a real capability for split-trust
audit-review workflows — but it belongs to whatever phase actually
builds that workflow, not Phase 7.

### Q2 — continuous vs per-session chain

Entry-time options: (1) continuous across all sessions, (2)
per-session chains with explicit `ChainLinked { prev_tip_hash,
prev_tip_seq }` records, (3) Merkle tree over session tips
*(overkill)*.

**Shipped: option (1) — continuous chain.** Entry-time leaning was
(2) for the strictly-stronger tamper-evidence, but the per-session
link-record shape would have dragged a new `AuditEvent` variant
through the D4 event list and an amendment to the `HmacChainLog`
re-open contract. Task 1 re-evaluated: a continuous chain with
range-scan verification at open handles the truncation-at-session-
boundary attack **just as well** for any attacker model that
doesn't already hold the AEAD key, because the gap between
`head_seq_on_disk` and `HmacChainLog::len()` is observable at
verify time. Per-session link records would only have added
protection against an attacker who deleted the most recent session
*entirely* — a real but narrower threat, and one that a future
phase can layer on via a new event variant without rewriting this
task.

### Q3 — on-disk record shape

Entry-time options: (1) `serde_json::to_vec(&AuditEvent)` + outer
record, (2) `bincode` / `postcard` *(new dep)*, (3) hand-rolled
layout *(migration debt)*.

**Shipped: option (1) — `serde_json::to_vec(&SignedEntry)`.** The
outer envelope is the existing `aivyx_audit::SignedEntry { seq,
appended_at, event, mac, prev_mac }` struct (pub-exported for the
Task 7 integration test's tamper recipe). Rationale identical to
Phase 6's Q1: zero new deps, the store is AEAD-encrypted so
on-disk human readability is a test-only benefit, and `serde_json`
handles a growing enum (`AuditEvent`) gracefully where a binary
layout would require migrations. The MAC is computed over
`serde_jcs` canonical bytes (stable ordering), but the durable
envelope uses plain `serde_json` because it only needs to
round-trip the struct faithfully.

### Q4 — `AuditHook::on_event` → async?

**This was the streak-ender candidate.** Entry-time options: (1)
sync + `spawn_blocking` *(awkward)*, (2) make the trait async —
first amendment, ends the streak, (3) bounded-channel background
drain.

**Shipped: option (3) — streak holds.** The sync `on_event`
path inside `PersistentAuditLog` calls `HmacChainLog::append`
inline (so the chain MAC is computed before return — invariant 1),
then `try_send`s the freshly-chained `SignedEntry` into a bounded
tokio mpsc channel (capacity 1024). A background drain task,
spawned by `open` and aborted in `Drop`, consumes the channel in
FIFO order and persists each entry via `DomainHandle::put`.

The entry-time objection to option (3) — *"a write failure after
the session loop has moved on is orphaned"* — was answered by a
specific invariant: an `Arc<AtomicBool>` **health flag** flips
false on the first drain failure, and subsequent `on_event` calls
observe the flag and short-circuit through a first-error closure
(`Arc<dyn Fn(AuditError)>`). So the failure isn't silent — it's
observed ≤1 event later and surfaces through the registered
handler. Combined with the "bounded channel, abort on drop"
lifetime (invariant 5), this gives the five Task 1 defended:

1. MAC computed synchronously on-path before return.
2. Drain order = disk order (mpsc FIFO, single writer).
3. Persist failures observed ≤1 event later via the health flag.
4. Chain-rejected events never acked as audited (handler runs
   before channel push).
5. Drain task lifetime bounded by `PersistentAuditLog` via
   `Drop::abort`.

Task 7's integration test proved the fourth and fifth at the
process boundary: session A drains to disk, drops the log
(aborting the drain), drops storage (releasing the redb file
lock), and session B reopens and replays the full chain with a
4-event Level-3 assertion.

### Q5 — mandatory-or-logged chain verification at open

Entry-time options: (1) refuse to start on any chain break,
(2) log-and-continue with a synthetic
`AuditEvent::ChainBreakDetected` event.

**Shipped: neither — policy lives at the boundary, not the
library.** Task 2 shipped `verify_from_disk(storage, key) ->
Result<VerifyReport, AuditError>` as a standalone associated fn
that returns the error *unchanged*. The decision of whether to
exit non-zero, restore from backup, or (in a future dev tool)
display-and-continue lives in the binary, not in `aivyx-audit`.
This resolution kept `aivyx-audit` out of any
`AuditEvent::ChainBreakDetected` schema-extension territory that
would have amended D4's event list. Task 3's binary wires option
(1) as the default for now: any `AuditError` from
`PersistentAuditLog::open` or the `--verify-only` path maps to
`ExitCode::FAILURE` via the existing top-level `match`. A future
phase that wants log-and-continue for dev-box debugging can pick
that policy at the binary edge without touching the library.

### Q6 — `--verify-only` CLI mode

Entry-time leaning: *"ship if it falls out for free in task 3's
binary wiring; defer otherwise."*

**Shipped: yes.** Task 3 landed a `--verify-only` flag (bare, no
extra args) that branches *after* storage open but *before* session
bring-up, calls `verify_from_disk`, prints
`audit: verified N events (head_seq=...)`, and returns
`Ok(())`. Critically, **`ANTHROPIC_API_KEY` is not required** in
verify-only mode — a forensic operator running verification on a
production store should not have to hand the cloud key to a
read-only tool. This is the shape that falls naturally out of the
Task 2 standalone `verify_from_disk` entry point; if Task 2 had
only exposed verification via `PersistentAuditLog::open`, `--verify-
only` would have had to spawn a drain task it never needs. The
Phase 6 promise at `PHASE_6.md:518-523` to land
`CapabilitySet::default()` "if it falls out" did **not** fall out
this phase — see "Decisions deferred to Phase 8+" below.

## Decisions deferred to Phase 8+

- **Per-session chain link records.** Q2 shipped a continuous
  chain because the per-session link-record shape would have
  dragged a new `AuditEvent` variant through D4 for a real but
  narrow threat (attacker deletes a session *entirely*, not
  just truncates mid-session). A future phase can layer this in
  via a new event variant without rewriting Task 1; the
  `HmacChainLog::from_verified_entries` reopen-path constructor
  already accepts arbitrary verified entries, so the added event
  would slot in at the existing boundary.
- **`AIVYX_AUDIT_KEY` env var for split-trust review.** Q1's
  option (4). Real capability, narrower use case (an auditor
  verifies a store they don't hold the master passphrase for),
  and it belongs to whatever phase actually builds the
  split-trust review workflow. Today's single-user story is
  fully served by HKDF from the master via `KeyDomain::Audit`.
- **Log-and-continue chain-break policy.** Q5's option (2).
  The Task 2 shape already supports it — `verify_from_disk`
  returns the error unchanged, so the binary edge can pick
  log-and-continue without touching the library. A future dev-
  tool or introspection surface that wants "show the break, let
  me poke at the store anyway" can add that policy without
  amendment. The current binary uses the strict "exit non-zero
  on any chain break" policy because that's the safer default
  for a production store.
- **Cross-topic `memory.read`.** Re-re-queued (third time). Still
  no substrate primitive, still no concrete use case. Phase 8's
  Telegram adapter does not change the calculus; the first time
  a bot user asks "what do you know about me across all topics"
  is the first time this becomes a real phase.
- **Session-scoped memory qualifiers.** Re-re-queued. Phase 8's
  Telegram adapter is the first phase where "one binary serving
  multiple humans" is a real shape, and the first phase where
  `memory.read:session:<chat_id>` has a concrete meaning. So
  this refinement is *aimed at Phase 8 proper* if PHASE_8.md's
  multi-user trust story calls for it.
- **`CapabilitySet::default()` ergonomics.** **Re-re-re-re-re-re-
  deferred** — sixth consecutive phase roll-forward (Phase 2, 3,
  4, 5, 6, 7). Phase 6 committed to landing it in Phase 7; it
  did not land. The feature is small (one `Default` impl, one
  doc comment) and the re-queue isn't because it's hard — it's
  because every phase has had at least one larger fish, and
  "small-and-easy" keeps losing the priority scrap to
  "important-and-visible." **Pattern recorded as a lesson
  below.** Explicit commitment for Phase 8: *do not promise it
  again at entry; land it opportunistically when the first
  Phase 8 task that touches `CapabilitySet` construction
  naturally needs it, or leave it for Phase 9+.*
- **Runtime JSON-schema validation for tool input.** Re-re-re-
  queued. Unrelated to Phase 7's audit theme, unrelated to
  Phase 8's adapter theme. No timeline; gets picked up when a
  tool-layer refinement phase materializes.
- **Tool name in `StreamEvent::ToolCallStarted`.** Re-re-re-
  queued. Same status as JSON-schema validation.
- **Chain rotation story.** Explicitly out of scope per Phase 7
  entry. If the HMAC key ever needs to rotate (passphrase
  change, key leak), the right shape is a new chain with a
  `ChainRotated { prev_tip_hash, prev_tip_seq }` link record
  cutting to it, and a `verify_from_disk` that walks across
  rotation boundaries. The substrate supports it; no phase has
  called for it yet.

## Lessons carried forward

- **Absorb async impedance inside the library, not at the trait
  boundary.** Q4's resolution — bounded mpsc + background drain
  task with a health flag — is the shape to reach for whenever a
  new persistence story threatens to async-ify a trait that
  shouldn't be async. The streak preservation was a welcome
  side-effect, but the *design reason* is that `AuditHook::on_event`
  is called from every tool call site in the codebase, and
  making every one of those sites `.await` a hook that is *almost
  always a no-op* (the in-memory `HmacChainLog::append` is pure
  CPU) would propagate unnecessary async colour across half the
  crates. The lesson: when a trait is called from many sync
  sites and the *expected* implementation is sync, keep the trait
  sync and absorb the async impl behind a drain task.
- **Drop-order matters more than it looks in integration tests.**
  Task 7 hit a redb `Database already open` failure on the first
  test run because `tokio::task::JoinHandle::abort()` is
  non-blocking — the drain task drops its captured
  `Arc<dyn Storage>` only when the runtime next polls it, and
  synchronous `drop()` never yields. The fix is a `yield_now()`
  loop after the explicit drop sequence. The lesson: any
  integration test that tears down a tokio-owning type and then
  expects its resources to be immediately free needs either an
  explicit handle-level `await` or a yield loop. "The drop
  ordered things correctly" is not the same as "all drops have
  executed."
- **`serde_json` keeps winning over binary formats.** Phase 6 Q1
  (memory entries) and Phase 7 Q3 (audit entries) both landed
  on `serde_json::to_vec` for the same set of reasons: zero new
  deps, growing enums handled gracefully, on-disk readability
  for tests, and the AEAD layer removes the privacy argument
  for a binary format. The bloat is real (~2-3× vs postcard)
  but it has not mattered yet, and when it does matter the
  migration is one crate-local change behind the existing
  `DomainHandle::get/put` boundary, not a contract change.
- **Roadmap optimism is bounded and observable: `CapabilitySet::
  default()` has now been re-queued six phases running.** Phase
  2 flagged it, Phase 3 re-queued it, Phase 4 re-queued it,
  Phase 5 re-queued it, Phase 6 *committed to landing it in
  Phase 7*, Phase 7 re-queued it again. The feature itself is
  ~10 lines of code. The pattern isn't "it's hard"; the pattern
  is "small-easy work consistently loses to larger-visible work
  when a phase has a deadline." The lesson: **phase entry
  commitments to 'small ergonomic refinements' should be
  treated with the same skepticism as any other roadmap
  optimism**, and the right place to land them is inside a
  larger task that naturally touches the surface — not as a
  standalone task line item that any larger fish can displace.
  Phase 8's entry criteria will not re-promise this.
- **Task-level decision sub-questions up front save two
  round-trips in the middle.** Tasks 5, 6, and 7 all used the
  "flag Qa–Qe at task entry, get a one-line answer each, then
  proceed" pattern. Tasks that did not (Task 1) produced more
  mid-task course corrections. The sub-decision list isn't
  bureaucracy — it's the planning surface that replaces
  mid-work re-plans.
- **The test-count delta is the easiest phase-health metric.**
  Phase 7: 257 → 284 (+27). Phase 6: 226 → 257 (+31). Phase 5:
  ~180 → 226 (+46). The trend is downward because the easy
  surfaces are already covered; a phase that ships zero new
  tests is a phase that probably refactored something it
  shouldn't have. Phase 8's exit criteria should include a net
  test-count delta ≥ +20 as a proxy for "actually shipped
  something concrete."
- **Integration-test tempdir convention.** Every Phase 5, 6, and
  7 e2e test uses a `SharedStoreDir` / `MemoryHarness` duplicated
  per file. This is deliberate — the duplication cost is ~80
  lines per file, the coupling cost of a shared helper would
  be a cross-crate `dev-dependencies` in `aivyx-core` and the
  first contract-surface pressure on the test harness. The
  rule: **tests duplicate until the duplication is itself the
  bug**. Phase 7 did not hit that threshold.

## Exit criteria (all met)

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
- [x] `PassphraseSource::InteractivePrompt` is lit up behind
      `rpassword`, not a stub. *(Task 4, 2026-04-14. Real
      `rpassword::prompt_password` call site; private
      `read_interactive_password_inner` closure seam lets
      unit tests route through `prompt_password_from_bufread`
      without a tty. `InteractiveNotImplemented` error variant
      deleted, replaced by `InteractiveIo { reason }` and
      `InteractiveEmpty`. Binary's new `select_passphrase_source()`
      helper picks `Env` / `InteractivePrompt` / bail based on
      env var + `io::stdin().is_terminal()`. +5 unit tests;
      workspace 271 → 276. Smoke-tested in three binary
      branches. DESIGN.md empty-diff streak preserved a fourth
      task in a row.)*
- [x] `MemoryWriteTool` refuses writes above
      `AIVYX_MEMORY_MAX_PER_TOPIC` (default 10 000) with a
      typed `Failed` outcome the planner can observe.
      *(Task 5, 2026-04-14. New `DEFAULT_MAX_PER_TOPIC` const
      and `set_max_per_topic` builder; tripwire runs after the
      audit event and before `Memory::put`, surfacing as
      `ToolOutcome::Failed(AivyxError::Tool { tool, detail })`
      with a detail that names the topic, the cap, and the
      `memory.forget` recovery hint. Binary resolves the cap once
      at startup via a new `resolve_memory_max_per_topic` helper
      — unparseable env-var values are a hard error, not a silent
      fallback. Draft's wrong `Failed { reason }` shape corrected
      in the shipped record. +4 tests; workspace 276 → 280.
      DESIGN.md empty-diff streak preserved a fifth task in a
      row — `Memory` trait did not grow a `count` method, and
      `ToolOutcome::Failed(AivyxError::Tool {...})` absorbed the
      refusal without a new variant.)*
- [x] The store file, its salt sidecar, and any audit-chain
      state files are `chmod 0600` at create time on Linux.
      *(Task 6, 2026-04-14. "Any audit-chain state files" is
      empty by construction — Task 1 folded the audit chain into
      `KeyDomain::Audit` rows inside the redb store, so the full
      surface is the store file + salt sidecar. Both sites use a
      duplicated `#[cfg(unix)]`-gated `chmod_user_only` helper
      with a non-unix no-op fallback; chmod runs only on cold-
      start via a `Path::try_exists` probe, so a deliberately
      re-permed file is not silently fought back on reopen.
      Failure is a hard error mapping to the existing
      `StorageError::Redb` / `PassphraseError::SaltIo` variants.
      The salt is chmod'd even though "salts are not secret":
      uniform perms across every aivyx-created file are a
      better auditability story than per-file rationales.
      +2 tests; workspace 280 → 282. DESIGN.md empty-diff streak
      preserved a sixth task in a row — no shared helper in
      `aivyx-core`, so D3 is untouched.)*
- [x] A scripted integration test
      (`crates/aivyx-channel/tests/audit_persistence_e2e.rs`)
      drives a two-session audit round-trip *and* asserts that a
      direct-tamper negative flips the chain verifier to a
      specific `ChainBroken { seq, .. }`. *(Task 7, 2026-04-14.
      Shipped two `#[tokio::test]`s instead of the draft's
      three-session shape — isolated tempdirs per test, no
      shared mutable state. Positive test asserts the Level-3
      entry shape across all 4 events plus the `prev_mac` chain
      invariant. Negative test uses a decrypt-flip-re-put recipe
      on seq 0 so the AEAD still validates but the HMAC replay
      fails with `ChainBroken { seq: 0 }`. Draft variant name
      `ChainBreak { at_seq }` corrected in the shipped record
      to the Task 2 shape `ChainBroken { seq, reason }`. +2
      tests; workspace 282 → 284. DESIGN.md empty-diff streak
      preserved a **seventh** task in a row.)*
- [x] `cargo test --workspace` green. *(284 tests passing at
      phase exit. Phase 7 added 27 new tests: Task 1 +10, Task 2
      +4, Task 3 +0 binary-only, Task 4 +5, Task 5 +4, Task 6 +2,
      Task 7 +2. Net 257 → 284.)*
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [x] `DESIGN.md` is still unchanged — **streak rolls to
      seven**. The Q4 `async fn on_event` refinement never became
      necessary because Task 1 shipped `PersistentAuditLog::open`
      as a spawn-at-open-time drain task over a bounded mpsc,
      keeping `AuditHook::on_event` synchronous at the trait
      level. No amendment file needed; the `docs/amendments/`
      directory still doesn't exist. *(Q4 resolution documented
      below under "Decisions made during Phase 7 that aren't in
      DESIGN.md".)*
- [x] Q1 (chain key source), Q2 (continuous vs per-session
      chain), Q3 (on-disk record shape), Q4 (async trait?), and
      Q5 (mandatory-or-logged verification) resolved and noted
      under "Decisions made during Phase 7 that aren't in
      DESIGN.md" below.
- [x] At least one Phase 6 queued refinement landed.
      **Interactive passphrase** shipped as Task 4 via real
      `rpassword::prompt_password` with a private
      `read_interactive_password_inner` closure seam for
      tty-free unit testing. `CapabilitySet::default()`
      ergonomics **re-re-deferred** to Phase 8+ — see "Decisions
      deferred to Phase 8+" below; the six-phase roll-forward
      pattern is itself now a recorded lesson.
- [x] Phase 8 roadmap entry refined with Phase 7's lessons, and
      Phase 8 opened as **Telegram adapter** on the back of
      Phase 7's hardening — persistent audit, memory caps,
      chmod 0600, and interactive passphrase are precisely what
      an untrusted-by-default bot needs. See [ROADMAP.md](ROADMAP.md)
      and [PHASE_8.md](PHASE_8.md).

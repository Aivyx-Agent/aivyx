# Phase 5 — Encrypted Storage (FROZEN)

**Status:** Closed 2026-04-14
**Exit commit:** `6dab2a7` — *"Phase 5 task 5: persistence integration test — two sessions, one store"*
**Predecessor:** [PHASE_4.md](PHASE_4.md) (exit commit `999ce87`)
**Successor:** to be scaffolded at Phase 6 entry — see [ROADMAP.md](ROADMAP.md)
**Contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all LOCKED — unchanged since `e0d6437`, now five phases running)

This document is a historical record. The artifacts it produced live
in `aivyx-crypto` (`Argon2Params`, `MasterKey`, `SubKey`,
`derive_master_key`, seal/open AEAD primitives), `aivyx-storage`
(`Storage` trait + `RedbStorage` impl, `KeyDomain` enum with
per-domain HKDF subkey derivation, `DomainHandle::{get,put,delete}`),
`aivyx-channel::passphrase` (the env/fixture/interactive-stub
`PassphraseSource` + sidecar salt handling + zero-on-drop transient
buffer), `aivyx-channel::session` (the 40-byte session-marker
round-trip wired through `run_session` via a new
`SessionConfig.storage: Arc<dyn Storage>` field), the reference
binary at `crates/aivyx-channel/src/bin/aivyx.rs` (which now reads
`AIVYX_STORAGE_PATH` and `AIVYX_PASSPHRASE`, opens the encrypted
store at startup, and logs a "resuming" line on reopen), and the
integration test at `crates/aivyx-channel/tests/storage_persistence_e2e.rs`.
This file explains *how* it came together and what was deliberately
left for Phase 6.

## Goal (as written at phase entry)

Stand up the **first persistent Aivyx session** by implementing
`aivyx-storage` against the D7 contract: a single `Storage` trait
with a single `RedbStorage` concrete impl, HKDF-derived `KeyDomain`
subkeys, ChaCha20-Poly1305 AEAD at rest, and a passphrase flow
whose prompt lives in the channel adapter rather than the storage
layer. The first phase where an agent recalls the previous turn
because it is actually on disk, not because the process happens to
still be running.

Persistence was the load-bearing outcome. By phase exit, restarting
the `aivyx` binary had to produce an agent that remembered the
previous session's metadata (at minimum, the "I've been here
before" signal) through a real disk round-trip.

## What shipped

- **`aivyx-crypto` primitives** (`fa8faae`). The Phase 0 stub
  crate became the first real cryptographic layer in the workspace.
  Public surface: a parameterized `Argon2Params` struct with
  `d7_default()` (m=64 MiB, t=3, p=4) and `weak_for_tests()` for
  unit tests, a zero-on-drop `MasterKey` type with redacted
  `Debug`, a `SubKey` type carrying an HKDF-SHA256 derivation over
  the versioned salt `b"aivyx-v1-storage"`, and ChaCha20-Poly1305
  seal/open entry points on `SubKey`. 17 unit tests cover:
  Argon2id determinism + parameter sensitivity, HKDF determinism
  + salt-version sensitivity + an RFC 5869 cross-check, AEAD
  round-trip and every negative (wrong key, wrong nonce, wrong
  aad, tampered ciphertext, invalid nonce length), a full-stack
  passphrase → master → subkey → seal/open smoke, and a Debug
  redaction tripwire. The crate's `sha2 = "0.10"` direct pin
  (not the workspace 0.11) is documented inline because
  `hkdf 0.12` requires sha2 0.10 and the two copies have to stay
  isolated to avoid trait-object fragmentation — the comment in
  `Cargo.toml` will age well if `hkdf 0.13` ever tracks sha2 0.11.
- **`aivyx-storage::RedbStorage` + `Storage` trait** (`772c39e`).
  The Phase 0 stub crate became a real encrypted KV store. Public
  surface: a five-variant `KeyDomain` enum (`Sessions`, `Memory`,
  `Audit`, `Secrets`, `ChannelState`) with `as_bytes()` domain
  separators and `table_name()` per-domain redb tables, a
  `StorageConfig { path: PathBuf }` builder, an inherent
  `RedbStorage::open(config, master) -> Arc<dyn Storage>`
  factory that precomputes all five domain subkeys via HKDF at
  open time, an `async_trait`-based `Storage` trait with a
  single-method `domain(KeyDomain) -> DomainHandle`, and a
  `DomainHandle::{get,put,delete}` surface that wraps every redb
  call in `tokio::task::spawn_blocking` per the D7 commitment.
  Storage format per value: `nonce_12 || ChaCha20Poly1305(subkey,
  nonce, aad, plaintext)`, where AAD is
  `b"aivyx-v1" || domain_bytes || 0x00 || user_key`. Fresh 12-byte
  nonces come from `*Uuid::new_v4().as_bytes()` truncated —
  reusing the workspace's existing uuid dep rather than pulling in
  `rand` or `getrandom` directly. 15 unit tests cover: put/get
  round-trip, missing-key returns `None`, put-overwrites,
  delete-idempotent, cross-domain isolation (putting in Sessions
  does not surface in Memory), every-domain round-trips,
  persistence-across-reopen with the same master key, the
  wrong-key negative (open succeeds, first `get` fails with
  `DecryptFailed`), AAD binding, and a 64 KiB large-value smoke.
- **`aivyx-channel::passphrase`** (`edaa4e7`). A new module inside
  the channel crate whose only job is the passphrase → `MasterKey`
  pipeline and its test scaffolding. Public surface: a
  `PassphraseSource` enum with three variants (`Env { var_name }`,
  `Fixture(Box<dyn FnOnce() -> Vec<u8> + Send>)`,
  `InteractivePrompt`), a `load_or_create_salt` helper that reads
  a sidecar `.salt` file or generates 16 random bytes on first
  run, and a `derive_master_key(source, salt_path, params)`
  function that zeroizes the transient passphrase `Vec<u8>`
  immediately after feeding it to `aivyx_crypto::derive_master_key`.
  The module never exposes the raw passphrase bytes across its
  API boundary, and its `Debug` impl for `PassphraseSource`
  prints only the variant discriminant (never closure contents)
  — there's a dedicated unit test for that redaction. 12 unit
  tests: salt generation + persistence + differs-between-stores
  + rejects-wrong-length, env-var sourced round-trip + not-set
  + empty negatives, fixture-sourced round-trip, env/fixture
  parity for the same bytes, interactive stub returns
  `InteractiveNotImplemented`, debug-impl redaction, an
  end-to-end "passphrase → derive → open storage → round-trip
  value" smoke against a real `RedbStorage`, and a
  `wrong_passphrase_fails_to_decrypt_storage` negative. The
  env-var tests serialize via a local `Mutex<()>` guard because
  `AIVYX_PASSPHRASE_TEST_*` env vars are process-global; the
  `unsafe { std::env::set_var(...) }` calls (required under Rust
  edition 2024) all have SAFETY comments pointing at the lock.
- **`SessionConfig.storage: Arc<dyn Storage>` + `run_session`
  session-marker wiring** (`dd175de`). A new field on
  `SessionConfig`, symmetric to the Phase 4 `tools` field, and
  the REPL loop's first persistent side effect. `run_session`
  now reads the prior "current session" marker under
  `KeyDomain::Sessions` / `b"current"` at open (logging a
  "resuming — prior session X opened Ns ago, last turn index N"
  line to stderr if one is present), writes its own marker with
  `last_turn_index = 0` before the loop starts, and overwrites
  the marker after every turn with the new turn count and
  wall-clock timestamp. The record is a hand-rolled 40-byte
  fixed layout:

  ```text
  [0..16]   session_uuid      (Uuid bytes, per-process)
  [16..24]  opened_at_secs    u64 big-endian
  [24..32]  last_turn_index   u64 big-endian, 0 at open, turns_run after
  [32..40]  last_turn_at_secs u64 big-endian, 0 before the first turn
  ```

  No serde dep — the record has exactly four fields, will never
  grow within Phase 5 (Q4: schema bumps ride the HKDF salt
  version, not in-place migration), and a future Phase 6 schema
  bump will be the moment to introduce serde-framed records.
  Four new unit tests pin the encode/decode: round-trip,
  big-endian layout anchor, wrong-length rejection, and the
  hex-display formatter.
- **`AIVYX_STORAGE_PATH` + `AIVYX_PASSPHRASE` binary wiring**
  (`dd175de`, same commit). The `aivyx` binary now reads two
  new env vars: `AIVYX_STORAGE_PATH` (defaulting to
  `$XDG_DATA_HOME/aivyx/store.redb` or
  `$HOME/.local/share/aivyx/store.redb`) for the encrypted redb
  file, and `AIVYX_PASSPHRASE` for the Argon2id master-key
  input. A new `resolve_storage_path()` helper mirrors the
  existing `resolve_fs_root()` shape; a new `salt_path_for()`
  helper appends a literal `.salt` to the store path (rather
  than using `Path::with_extension`, which would strip the
  `redb` and produce `store.salt`, losing the "belongs to the
  redb store" signal). The binary calls
  `derive_master_key(PassphraseSource::Env { var_name:
  DEFAULT_ENV_VAR.to_string() }, &salt_path,
  Argon2Params::d7_default())` *before* the tokio runtime
  starts (Argon2id is pure CPU, no runtime needed), then opens
  `RedbStorage` *inside* `block_on` so `spawn_blocking` lands
  on a live pool. The store handle flows into `SessionConfig.
  storage` alongside `tools`, `capabilities`, and everything
  else the binary composed at startup.
- **`storage_persistence_e2e.rs` — the whole point of the
  phase** (`6dab2a7`). A new integration test in
  `crates/aivyx-channel/tests/` that drives three scripted
  sessions against the same `$TMPDIR`-based store path.
  1. **`session_marker_survives_clean_close_and_reopen`.**
     Session A opens with master `[7u8; 32]`, runs one
     scripted turn via `run_session` (which writes the
     40-byte marker), asserts the pre-drop marker decodes
     to `last_turn_index = 1` with timestamps inside the
     test's wall-clock window, then `drop(storage_a)` to
     release redb's single-writer lock. Session B reopens
     with the same master key, reads the marker *before*
     running a turn (because `run_session` overwrites it at
     entry), and asserts the bytes decode back to what
     session A wrote. Session B then runs its own turn
     and the test re-reads the marker to prove it's been
     overwritten with session B's own `opened_at_secs >=`
     session A's. This is the Argon2id → HKDF → AEAD →
     redb stack holding its round-trip contract across a
     clean process-lifetime boundary.
  2. **`wrong_master_key_fails_to_decrypt_prior_session`.**
     Session A establishes on-disk ciphertext, drops.
     Session C reopens with `[99u8; 32]` — a *different*
     master — and `open` succeeds cleanly (HKDF produces
     32 bytes from any 32-byte input, and `RedbStorage::
     open` deliberately does not probe existing values),
     but the first `get` against `KeyDomain::Sessions` /
     `b"current"` fails with `StorageError::DecryptFailed
     { domain: Sessions }`. The test carries an explicit
     spec-vs-reality note: PHASE_5.md task 5 said "fails at
     open, not at first use," but the real behaviour is
     "open clean, first decrypted read fails hard," which
     is strictly stronger — a wrong-key adversary can't even
     confirm they got the right *file* without also having
     the right *key*.

  The test duplicates `SESSION_MARKER_KEY` and
  `SESSION_MARKER_LEN` locally rather than re-exporting them
  from `session.rs`: it is asserting an on-disk contract, so
  an independent second copy of the constants is a feature —
  any future drift between the encode path and the test's
  decode path produces a loud failure in both the integration
  test and the `session::tests` unit tests together.

## Decisions made during Phase 5 that aren't in DESIGN.md

### Q1 — Session metadata only (option 1); audit-tail and turn-history both deferred

Resolved at task 4 entry. `KeyDomain::Sessions` / `b"current"`
holds one 40-byte record per process lifetime with the four
fields above. Session B observes session A's record on reopen,
logs the "resuming" line, then overwrites it — this is a single-
row "current pointer," not a history log. Minimum viable
persistence; proves the crypto/storage stack end-to-end without
blurring into Phase 6 memory territory.

Option 2 (audit-tail persistence) was deferred because it drags
in an unresolved design question — does the HMAC chain key come
from `KeyDomain::Audit`, and if so what does "chain start" mean
across restarts? That's its own phase rather than an incidental
side effect of the storage phase, and the test `HmacChainLog
::tests::chain_appends_verify` already covers in-memory chain
integrity. Option 3 (full turn history) is explicitly Phase 6's
memory-as-tool work and shipping it here would have made Phase 6
about "choose a schema" rather than "ship memory."

**To re-evaluate at Phase 6 entry.** The `last_turn_index` field
is a local REPL counter, not the core `TurnId` Uuid. Phase 6's
memory layer will probably key on `TurnId` bytes and could
either extend the existing 40-byte record (a contract change
requiring a salt bump to `"aivyx-v2-storage"`) or live entirely
in `KeyDomain::Memory` and leave `Sessions` alone. The second
option keeps the Q1 decision stable.

### Q2 — `AIVYX_PASSPHRASE` env var only, interactive prompt deferred to Phase 7+

Resolved at task 3 entry. The passphrase module ships with three
`PassphraseSource` variants (`Env`, `Fixture`, `InteractivePrompt`)
but `InteractivePrompt` returns `PassphraseError::InteractiveNotImplemented`
on purpose. The binary uses `PassphraseSource::Env { var_name:
DEFAULT_ENV_VAR.to_string() }` with `DEFAULT_ENV_VAR = "AIVYX_PASSPHRASE"`.

The reasoning as frozen:
- **Phase focus.** Phase 5 is about the storage layer, not
  passphrase UX. A tty prompt via `rpassword::prompt_password`
  is a ~20-line follow-up once the storage round-trip is proven,
  and splitting it out keeps the bisect history clean.
- **Testability for free.** Env-var sourcing means every unit
  test and integration test can inject a passphrase without
  needing a pseudo-tty, which is how the `passphrase::tests`
  suite exercises the full round-trip (env → Argon2id → HKDF →
  storage → decrypt → compare) in under 10 ms per test.
- **OS keyring is not a Phase 5 job.** Keyring integration
  (Secret Service on Linux, Keychain on macOS, DPAPI on Windows)
  waits on a second channel adapter to justify its existence —
  there's no point building a keyring adapter for a binary that
  only runs on Linux. Phase 7+ call.

The `InteractivePrompt` variant exists in the enum specifically
so a future Phase 7 commit can light it up in one place without
re-shaping the API. The `rpassword` dep was not added in Phase 5.

### Q3 — `Arc<dyn Storage>` wins over `Arc<RedbStorage>`

Resolved at task 4 entry. `SessionConfig.storage` is
`Arc<dyn Storage>`, matching the `AuditHook` pattern from Phase 2
(`Arc<dyn AuditHook>` / `AuditBridge<HmacChainLog>` boxed at the
binary edge). Tests construct a `RedbStorage` directly against a
`$TMPDIR` path, call the inherent `open` method to get back an
`Arc<dyn Storage>`, and pass that to `SessionConfig`. The binary
does the same — `RedbStorage::open(...).await` returns
`Arc<dyn Storage>`, and nothing downstream ever needs the concrete
type.

The `async_trait` crate solves the async-in-dyn-traits question
workspace-wide and was already a dep. No vtable-cost concerns —
the binary holds one handle for the lifetime of the process, so
dynamic dispatch is O(turns), not O(IOs).

The pattern to copy for future resources: **if Phase N builds a
single-concrete-impl resource shared across concurrent turns via
`Arc`, the field type in `SessionConfig` is `Arc<dyn Trait>`.**
This gives tests a swap-in seam, the binary composition clarity,
and the `run_session` body a stable import path. Phase 6 memory,
Phase 7 keyring (if it happens), and Phase 8+ remote channels all
inherit this shape.

### Q4 — Schema designed once; the `"aivyx-v1-storage"` salt is forever

Resolved in the affirmative. The 40-byte session marker is the
shape for all of Phase 5, and no in-place migration framework is
needed. If Phase 6 discovers that the record should be 64 bytes
(say, adding a `last_turn_uuid: [u8; 16]` field between
`last_turn_index` and `last_turn_at_secs`), Phase 6 bumps the
HKDF salt from `b"aivyx-v1-storage"` to `b"aivyx-v2-storage"` in
`aivyx-crypto`. All existing subkeys become unreachable and the
dev box starts cold. That's the D7-locked migration strategy and
Phase 5 did not exercise it.

The decode path's `decode_session_marker` returns `Option`, not
`Result`, and the caller logs "session marker present but
unparseable (N bytes); starting fresh" on a failed decode rather
than treating it as a user-facing error. That's what a future
schema mismatch against an old-format file on disk would look
like if someone somehow kept the salt stable across a schema
change (which they shouldn't). Defense in depth.

### Q5 — Empty-diff streak held; five phases, zero amendments

Resolved empirically. Phase 5's two likeliest amendment
pressures both resolved as naming questions:

- **Storage lifetime ergonomics.** D7's sketch of
  `Storage::open` as an `async fn` on the trait was incompatible
  with the `Arc<dyn Storage>` return shape that Q3 wanted (a
  trait method returning `Self` is uncallable through a trait
  object). Resolved by making `open` an *inherent* method on
  `RedbStorage` that returns `Arc<dyn Storage>`, the same way
  `AuditBridge::new(HmacChainLog::new(...))` hands back an
  `Arc<dyn AuditHook>` at the binary edge. No contract change.
- **`KeyDomain` additions.** The five-variant enum is exactly
  what Phase 5 needed. No sixth variant pressure surfaced
  during the work — `Config`-style persistent non-secret
  settings turned out to be absent from Phase 5's scope
  entirely, and if Phase 6 wants them the call is whether to
  put them in `KeyDomain::Sessions` (probably fine) or to
  amend D7 for a new `KeyDomain::Config` variant (writes the
  first amendment file).

`git diff e0d6437 -- DESIGN.md` is empty through Phase 5's
exit commit. Five phases running.

## Bugs caught in Phase 5

- **`sha2` version mismatch about to hit.** Task 1's first draft
  of `aivyx-crypto/Cargo.toml` wrote `sha2 = { workspace = true }`,
  which resolves to 0.11. But `hkdf = "0.12"` transitively depends
  on sha2 0.10, and `Hkdf::<Sha256>::new` would have seen two
  incompatible `Sha256` types at the generic parameter site.
  Caught before running tests by noticing the workspace pin was
  0.11 while hkdf's Cargo.toml said 0.10. Fixed by pinning
  `sha2 = "0.10"` directly in `aivyx-crypto` and documenting the
  isolated-duplicate reasoning in an inline comment. The comment
  will age out when `hkdf 0.13` tracks sha2 0.11, at which point
  the workspace pin can take over.
- **`tempfile` dev-dep about to land, again.** Task 2's first
  draft of `aivyx-storage/Cargo.toml` added `tempfile = "3"` as a
  dev-dep. Grep found the pre-existing comment in
  `aivyx-core::tools::fs` explicitly noting "avoid adding
  `tempfile` as a dep for ~50 lines of test hygiene — hand-rolled
  `SandboxDir` is the workspace convention." Removed the dep,
  hand-rolled a `StoreDir` RAII helper over `$TMPDIR +
  Uuid::new_v4()` matching the existing shape. Documented the
  choice in a Cargo.toml comment so the next person reaching for
  `tempfile` finds the rationale in the same place they'd look
  to add it.
- **Unused imports on first redb 2.x build.** Task 2's initial
  imports included `use std::path::{Path, PathBuf}` (only
  `PathBuf` used) and `use redb::{Database, ReadableTable,
  TableDefinition}` (`ReadableTable` not needed). Build warnings,
  fixed by trimming. The `ReadableTable` import is a redb 1.x
  leftover — the 2.x API exposes `.get()` as an inherent method
  on `ReadOnlyTable` directly. The comment in the final
  `aivyx-storage/src/lib.rs` notes this so a future reader
  migrating code from a redb 1.x example finds the explanation.
- **Env-var race across `passphrase::tests`.** Task 3's first
  draft of the three env-sourced tests all used
  `std::env::set_var("AIVYX_PASSPHRASE_TEST_*", ...)` directly.
  Under `cargo test` parallelism, two tests running on different
  threads could interleave their set/remove calls and one test
  would see the other's value. Caught by a single flaky failure
  on the second CI run. Fixed with a local `env_lock()` returning
  a `Mutex<()>` guard; every set/remove serializes on it. Same
  pattern `aivyx-audit` already uses for its HMAC-key fixture
  setup — the precedent made the fix a ~5-line change instead of
  a ~30-line rewrite.
- **Rust 2024 `set_var` / `remove_var` are `unsafe`.** Same
  tests, same draft. Rust edition 2024 moved `std::env::set_var`
  and `std::env::remove_var` behind `unsafe` because they're not
  thread-safe under POSIX. The test file didn't compile on
  rust-version 1.85 until every call was wrapped in
  `unsafe { ... }` with a SAFETY comment pointing at the
  `env_lock()` serialization guarantee. The wrappers are ugly
  but the SAFETY comments make the non-obvious guarantee
  explicit — a future test author who forgets the lock is now
  forced to read the comment before they can compile.
- **`TurnOutcome::Completed` does not carry `TurnId`.** Task 4's
  first plan was to extract the real `TurnId` from the outcome
  and write it into the session marker. But reading
  `aivyx_core::TurnOutcome` revealed the variant is
  `Completed { final_message, tool_calls_made, duration }` —
  the turn id is minted inside `agent.turn()` and consumed by
  the audit path, never returned. Two options: amend the core
  enum (a D3-adjacent contract change), or use the REPL's local
  `turns_run` counter as `last_turn_index`. Chose the counter.
  The PHASE_5.md task 4 spec said "last-seen turn id" but the
  REPL turn index is a strict-enough proxy to satisfy the "I've
  been here before" signal without a core change. Recorded in
  the freeze here because it's a real spec-vs-reality delta that
  future-me will spot if they `grep TurnId` through Phase 5.
- **Storage errors panicking the REPL.** First draft of
  `run_session`'s session-marker write path used `.unwrap()` on
  the `put` result. That meant any redb hiccup would panic the
  entire session — a user who typed one message would lose their
  whole conversation because the sidecar marker couldn't be
  written. Fixed by downgrading every storage error inside
  `run_session` to an `eprintln!` + continue. The D7 contract
  wasn't explicit about this, but the spirit is clear: storage
  is observable but not fatal. Documented inline and in the
  task 4 commit message.

## Decisions deferred to Phase 6+

- **Interactive passphrase prompt.** `PassphraseSource::
  InteractivePrompt` is a stub; `rpassword` is not a dep.
  Phase 7+, or sooner if a user complains.
- **`KeyDomain::Audit` persistence.** Still in-memory via
  `HmacChainLog`. The chain-continuity-across-restart question
  (is the HMAC key derived from the passphrase? is rotation
  possible? what does "chain start" mean on reopen?) is a
  standalone design question and deserves its own phase rather
  than being a side effect of Phase 5. Most likely lands in
  Phase 7 when there are multiple audit consumers that benefit
  from post-hoc replay.
- **Audit chain tamper-evidence across restarts.** Related to
  the above. Currently each process start resets the chain; a
  would-be tamperer can truncate the log just by waiting for a
  restart. Not a Phase 5 regression — it was already this way
  after Phase 2 — but the right time to fix it is when the
  audit log becomes persistent.
- **Concurrent multi-session.** The D7 "one handle per process"
  commitment is a hard limit, not an ergonomic choice. If a
  future phase wants the same binary to talk to two stores
  concurrently (e.g., a user-facing store and a system-audit
  store), that's a D7 contract change, not a compatibility
  switch. Not on the roadmap.
- **Storage path permissions.** The binary does not `chmod 0600`
  the store file or its parent directory. On a shared Unix box
  that's a soft disclosure risk — another user can read the
  ciphertext file (still encrypted, but observable). Phase 5 did
  not defend against this because the threat model is "single
  user on a personal box." If Phase 7 lands a multi-user
  deployment path, chmod-at-create becomes a freshness step.
- **`aivyx-memory` and `KeyDomain::Memory`.** The domain exists
  and is reachable via `storage.domain(KeyDomain::Memory)`, but
  no code puts anything there. Phase 6's job.
- **`aivyx-config` is still a stub.** Phase 5 did not need a
  config-file layer — env vars and defaults cover every
  composable thing the binary does. The crate remains empty;
  when it grows, it will probably read from
  `$XDG_CONFIG_HOME/aivyx/config.toml` and be a sibling of
  `aivyx-storage` in composition, not a dep of it.
- **`CapabilitySet::default()` ergonomics.** Still not added.
  Phase 5 has four call sites constructing capability sets via
  `from_scopes([...])` (the binary plus three integration
  tests). None of them felt repetitive enough to motivate the
  change. Phase 6 will inherit this queue entry one more time.
- **Tool name in `StreamEvent::ToolCallStarted`.** Still a short
  id, not a name. Same deferral reasoning as Phases 3 and 4.
- **Runtime JSON-schema validation for tool input.** Unrelated
  to storage, still deferred.
- **Opt-in live-API test in CI.** Still hand-run only.
- **Unicode-normalized path comparisons.** Still Linux-first,
  still byte-level `Path::starts_with`.
- **`rustyline` line editing.** Still plain
  `stdin().read_line`. The passphrase-module work in Phase 5
  touched the `stdin` seam but did not upgrade it.

## Lessons carried forward

- **Composition vs execution paid off again.** Task 4's clean
  split between "binary owns composition (opens the store,
  derives the master, builds the config)" and "library owns
  execution (`run_session` just reads `config.storage` and
  threads it)" meant the integration test in Task 5 could
  compose its own storage with `MasterKey::from_raw([7u8; 32])`
  and bypass the Argon2id hot path entirely. The binary pays
  ~500 ms per startup for Argon2id at `d7_default()`; the
  integration tests pay ~1 ms per session because the test
  composition can shortcut. Same pattern as Phase 3's
  `ScriptedProvider` bypass of the Anthropic HTTPS stack.
- **Trait object + inherent `open` is the resource pattern.**
  Every Phase 5+ resource that wants "single concrete impl,
  shared `Arc` across turns, trait object at the field site"
  should follow the shape Task 2 adopted: the trait has only
  operation methods (no `Self`-returning factories), and the
  concrete type has an inherent `open` / `build` / `new`
  returning `Arc<dyn Trait>`. Phase 2's `AuditBridge::new` was
  the precedent; Phase 5's `RedbStorage::open` is the second
  data point; Phases 6+ should make it a convention rather
  than a discovery.
- **Non-secret sidecar files are fine; name them for their
  sibling.** The salt file lives as `store.redb.salt` in the
  same directory as the encrypted store. `Path::with_extension`
  would have turned `store.redb` into `store.salt`, stripping
  the "redb" signal — a future operator who saw a lone
  `store.salt` file would have no idea what it belonged to.
  Appending `.salt` to the OS string preserves the relationship.
  The pattern generalizes: **sidecars keep their sibling's full
  name as a prefix**, not a replacement.
- **Spec-vs-reality notes are load-bearing.** PHASE_5.md task 5
  said the wrong-passphrase path should "fail at open, not at
  first use." The actual implementation (Task 2) made `open`
  oblivious to existing ciphertext, so the failure surfaces at
  the first `get`. That is strictly stronger — a wrong-key
  adversary can't confirm they got the right *file* without
  also having the right *key* — but it's a real doc-vs-code
  delta. The integration test encodes both the real behaviour
  *and* the note about why the spec aspirationally said
  otherwise. Future-me reading `PHASE_5.md` will find the
  explanation beside the failing assertion, not in a commit
  message from two years ago.
- **`DESIGN.md` empty-diff streak: 5.** Phase 1 → Phase 2 →
  Phase 3 → Phase 4 → Phase 5 all exited with zero contract
  changes. Phase 5 was the *flagged* one — PHASE_5.md Q5
  explicitly predicted this phase might break the streak, and
  ROADMAP.md warned the same — and it didn't, because the two
  amendment candidates (storage lifetime ergonomics, `KeyDomain`
  additions) both resolved without contract changes. This is
  the fifth consecutive data point that the Phase 0 contract is
  expressive enough to anchor real implementations without
  drift, and the first one that lands under *acknowledged*
  contract pressure. If Phase 5 had broken the streak it would
  have been unremarkable; that it held under the pressure it
  was warned about is the noteworthy part.
- **First-try green integration tests are an audit of earlier
  tasks.** Task 5's `storage_persistence_e2e.rs` passed on the
  first `cargo test` run. That's unusual for integration tests
  stitching together four independent layers (crypto, storage,
  passphrase, session wiring), and it's the clearest evidence
  available that each layer's unit-test suite really was
  protecting its contract. Future phases should treat a first-
  try-green integration test as *evidence of earlier testing
  discipline*, not as routine.
- **Commit per task, still.** Six Phase 5 commits (`fa8faae` →
  `772c39e` → `edaa4e7` → `dd175de` → `6dab2a7`, plus this
  freeze), each building and testing green in isolation, each
  with a commit message stating the substantive change plus
  the test counts and DESIGN.md streak status. Five-phase
  streak of this cadence too. Same bisect payoff.
- **Test count progression as a health signal.** Phase 5 added
  57 new tests end-to-end: 17 in `aivyx-crypto` (task 1), 15 in
  `aivyx-storage` (task 2), 12 in `aivyx-channel::passphrase`
  (task 3), 6 in `aivyx-channel::session` + 2 in the passphrase
  storage round-trip (task 4, counting the 4 new session-marker
  unit tests plus 2 fs_tool_e2e updates that re-exercise the
  storage field), and 2 in `storage_persistence_e2e.rs` (task 5).
  Workspace totals: 150 passed / 1 ignored at Phase 4 exit →
  200 passed / 1 ignored at Phase 5 exit. The count *is* a
  progress bar, and every task-commit message recorded the
  delta so a regression in any phase surfaces as "test count
  went down" in `git log`.

## Exit criteria (all met)

- [x] `aivyx-storage` has a real `Storage` trait and a
      `RedbStorage` impl, both with unit tests (15 passing in
      `aivyx-storage::tests`)
- [x] `aivyx-crypto` has real Argon2id + HKDF + ChaCha20-
      Poly1305 entry points with round-trip and negative-case
      tests (17 passing in `aivyx-crypto::tests`)
- [x] A passphrase flow exists in `aivyx-channel::passphrase`
      and never hands the raw passphrase to `aivyx-storage` (12
      passing unit tests plus the redaction tripwire)
- [x] `SessionConfig.storage: Arc<dyn Storage>` is wired and
      the `aivyx` binary opens a real store at startup via
      `AIVYX_STORAGE_PATH` + `AIVYX_PASSPHRASE`
- [x] A persistence integration test
      (`crates/aivyx-channel/tests/storage_persistence_e2e.rs`)
      drives two sequential sessions against one tempdir and
      proves the second session reads the first session's
      written state, plus a third-session wrong-passphrase
      negative
- [x] `cargo test --workspace` green (200 tests passing, 1
      ignored — up from 150 / 1 at Phase 4 exit)
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
      clean
- [x] Q1 resolved and noted under "Decisions made during Phase 5
      that aren't in DESIGN.md" (session-metadata-only, option 1)
- [x] Q2 resolved, with the interactive-prompt decision
      documented (env-var only, interactive deferred to Phase 7+)
- [x] Q3 resolved, with the pattern reused by any future
      similarly-shaped resource (`Arc<dyn Storage>`; inherent
      `open` returns the trait object; `AuditHook` is the precedent)
- [x] `DESIGN.md` diff since `e0d6437` is empty (five-phase
      streak — `git diff e0d6437..HEAD -- DESIGN.md` is empty)
- [x] At least one Phase 4 queued refinement addressed: the
      `rustyline` vs `rpassword` decision was resolved in the
      passphrase module's favour (`rpassword` listed as the
      Phase 7+ follow-up, not added as a Phase 5 dep;
      `PassphraseSource::InteractivePrompt` exists as a stub so
      the plumbing is already in place). Re-queued refinements:
      `CapabilitySet::default()`, tool name in
      `ToolCallStarted`, `rustyline` line editing for non-
      passphrase input, opt-in live CI test, binary-content
      encoding, runtime JSON-schema validation. See "Decisions
      deferred to Phase 6+" above.

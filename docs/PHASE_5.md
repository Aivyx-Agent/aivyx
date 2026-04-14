# Phase 5 — Encrypted Storage

**Status:** Active (opened 2026-04-14)
**Predecessor:** [PHASE_4.md](PHASE_4.md) (exit commit `e36fefd`)
**Contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all
LOCKED — unchanged since `e0d6437`, four phases running)

This document is the **working journal** for Phase 5. It will churn.
At phase exit it is frozen under the same convention as
[`PHASE_4.md`](PHASE_4.md) — no edits except through commits tagged
`docs(phase-5):`.

## Goal

Stand up the **first persistent Aivyx session** by implementing
`aivyx-storage` against the D7 contract: a single `Storage` trait
with a single `RedbStorage` concrete impl, HKDF-derived `KeyDomain`
subkeys, ChaCha20-Poly1305 AEAD at rest, and a passphrase flow
whose prompt lives in the channel adapter rather than the storage
layer. The first phase where an agent recalls the previous turn
because it is actually on disk, not because the process happens
to still be running.

The phase is organized around three concrete pressure points, each
of which will tell us something the Phase 0 contract can't by
itself:

- **Argon2id + HKDF at rest.** Real parameters, real key schedule,
  real cost at startup. This is where we find out whether m=64MB
  / t=3 / p=4 is the right starting point for the local CLI or
  whether it's too slow to be a good default.
- **"One handle per process" as an ergonomic claim.** D7 says the
  storage handle is opened once and shared via `Arc<RedbStorage>`.
  Phase 4 proved the same shape works for the fs sandbox root
  (`FsReadToolConfig::build() → Arc<FsReadTool>`, shared across
  concurrent turns). Phase 5 is where that pattern lands on a
  resource whose failure mode at open time is user-visible
  (wrong passphrase, corrupt file, schema mismatch) and whose
  successful-open cost is seconds, not microseconds.
- **Passphrase flow in the channel adapter.** D7 locks the
  decision that storage never sees raw passphrases — the local
  CLI prompts, derives the master key via `aivyx-crypto::derive`,
  and hands the *derived key* to `Storage::open`. Phase 5 is
  where that boundary becomes code. Cleanly-separated prompts
  will also be the seam that Phase 7+ remote channels reuse
  unchanged.

Persistence is the load-bearing outcome. By phase exit, restarting
the `aivyx` binary should produce an agent that remembers the
previous conversation's audit-chain tail (or at least a "hello,
you were here before" signal proving the disk round-trip). The
exact persistence surface — session metadata vs history vs
audit chain — is a task-2 decision, scoped below.

## Non-goals

- **Not a memory tool.** `aivyx-memory` stays stubbed. Memory
  as a tool is Phase 6's job and belongs behind the D7 storage
  layer Phase 5 is building — trying to ship both at once would
  hide which layer caused any bug.
- **Not a schema-migration framework.** D7 explicitly rejects
  this. Schema changes ride the HKDF salt version (bump to
  `"aivyx-v2-storage"` → entirely new subkeys → cold start).
  Any "migration" work this phase means "rewrite the open-time
  schema decode path," not "build a migration runner."
- **Not multi-writer, not multi-process.** The redb file lock
  enforces this and Phase 5 trusts it — attempting to open the
  same file from two processes is expected to fail loudly, not
  to be coordinated.
- **Not a config file.** Passphrase still comes from a prompt
  (or, for tests, an env var). A config-file layer that reads
  `~/.config/aivyx/config.toml` is Phase 7 at the earliest.
- **Not OS keyring integration.** The passphrase flow is a
  terminal prompt and an env-var escape hatch; keyring
  (Secret Service / Keychain / DPAPI) is a keyring-adapter
  task that waits on a second channel to justify its existence.
- **Not a storage-side audit log.** D7 lists `KeyDomain::Audit`
  because a future phase will persist the audit chain there, but
  Phase 5's job is the domain plumbing, not the audit-writer
  switch. `HmacChainLog` remains in-memory; the storage handle
  gets wired through `SessionConfig` so Phase 6+ can flip the
  switch without re-plumbing.
- **Not cross-platform polish.** Linux-first, same as Phase 4.
  macOS / Windows path differences, keyring adapters, and
  filesystem-normalization quirks are deferred.

## Entry criteria (all met from Phase 4 exit)

- [x] Phase 4 frozen at `e36fefd`; README + ROADMAP updated.
- [x] `SessionConfig` carries composed dependencies
      (`capabilities`, `tools`, and soon `storage`) rather than
      building them inside `run_session`. The Phase 4 split of
      "binary owns composition, session owns execution" is the
      pattern Phase 5 extends.
- [x] `aivyx-core::tools::fs` proves the `Config::build() →
      Arc<T>` pattern for a fallible startup resource shared
      across concurrent turns. Phase 5's storage handle has the
      same shape.
- [x] `DESIGN.md` unchanged since `e0d6437` — four-phase streak
      entering Phase 5.
- [x] `aivyx-storage` crate exists as a Phase 0 stub
      (`crates/aivyx-storage/`, deps section intentionally
      empty, `KeyDomain` placeholder in `lib.rs`).
- [x] Workspace builds green: `cargo test --workspace` reports
      150 passing / 1 ignored on Phase 4 exit.

## Refinements queued from Phase 4

Phase 4 explicitly deferred a handful of decisions to "Phase 5+."
Each is in scope *if it becomes load-bearing for storage work*
and otherwise consciously punted to Phase 6+.

- **`CapabilitySet::default()` ergonomics.** Phase 4 observed
  that three call sites (`bin/aivyx.rs`, `cli_e2e.rs`,
  `fs_tool_e2e.rs`) build capability sets via `from_scopes([...])`.
  Phase 5's storage tests will want a fourth. First time it
  feels repetitive, add the default — otherwise hold.
- **`fs.read` binary-content encoding.** Not touched in Phase 5
  unless the storage layer needs to ingest binary blobs via a
  tool (it shouldn't — storage is a trait the agent holds, not
  a tool the agent calls).
- **Runtime JSON-schema validation for tool input.** Unrelated
  to storage. Deferred.
- **`rustyline` line editing.** A passphrase prompt is the
  first time `stdin().read_line` feels wrong, because it echoes.
  `rpassword` is a smaller dep than `rustyline` and does exactly
  one thing. Plan: pull in `rpassword` if and when the
  interactive path is implemented. Testing uses an env-var
  escape hatch.
- **Tool name in `StreamEvent::ToolCallStarted`.** Still
  deferred; Phase 5 doesn't introduce new tools.
- **Opt-in live-API test in CI.** Same — still a Phase 5+ ops
  decision, still not a Phase 5 blocker.

## Task list (draft — revised as work lands)

Commit-per-task, `cargo test --workspace` green + clean clippy
on every one. Eleven consecutive task-commits across Phases 2–4
have proven this cadence; keep it.

1. **Wire `aivyx-crypto` primitives.** `aivyx-crypto` is a
   Phase 0 stub. Phase 5 is its first real user, so it has to
   grow an Argon2id parameterized `derive_master_key(passphrase,
   salt, params)` entry point, an HKDF-SHA256 `derive_subkey(
   master, domain)` entry point that reads the versioned salt
   and `KeyDomain::as_bytes()`, and the ChaCha20-Poly1305 seal/
   open primitives. Unit tests: round-trip a subkey derivation
   against a known vector, round-trip a seal/open against a
   `Vec<u8>`, assert the versioned salt change produces a
   different subkey. No redb contact yet. This is "the crypto
   side compiles and is tested in isolation" so task 2 can wire
   it without debugging crypto and I/O at the same time.
2. **`RedbStorage::open` happy path.** Flesh out `aivyx-storage`
   with `StorageConfig { path: PathBuf, argon_params:
   Argon2Params }`, a `StorageConfig::open(master_key:
   &MasterKey) -> Result<Arc<RedbStorage>, StorageError>`
   async factory, and enough of the `Storage` trait to handle
   `domain(KeyDomain)` lookup + `DomainHandle::{get,put,delete}`.
   Tests use a tempdir redb file and assert round-trip per
   domain, cross-domain isolation (putting in `Sessions` does
   not surface in `Memory`), and the "wrong key ⇒ decrypt
   error" negative. `scan` can stub to a `todo!()` until task 4
   proves it's needed.
3. **Passphrase flow in the channel adapter.** Add an
   `aivyx-channel::passphrase` module that can source a
   passphrase from one of three places in priority order:
   `AIVYX_PASSPHRASE` env var (for tests and non-interactive
   runs), a terminal `rpassword::prompt_password` call (the
   human default), or a test fixture closure (injected via a
   new `run_session` argument or `SessionConfig` field — TBD).
   The module *derives the master key* via `aivyx-crypto` and
   hands a `MasterKey` to storage; the raw passphrase never
   crosses the module boundary. Unit tests exercise the env-var
   path without a tty.
4. **`SessionConfig.storage: Arc<dyn Storage>` + binary
   wiring.** New field on `SessionConfig`, symmetric to the
   Phase 4 `tools` field. `run_session` clones the handle per
   turn and passes it through `ConcreteAgent` for whatever
   persistence the turn loop gains in this phase (minimum:
   record session start timestamp + last-seen turn id under
   `KeyDomain::Sessions` so a second process start can detect
   "I've been here before"). The `aivyx` binary gains an
   `AIVYX_STORAGE_PATH` env var (default
   `$XDG_DATA_HOME/aivyx/store.redb` or `$HOME/.local/share/
   aivyx/store.redb`), opens the store during `run()` before
   it builds the tools, and passes the `Arc<dyn Storage>` into
   the session. The Phase 3/4 CLI regression test stays
   chat-only by passing a `NullStorage` or an in-memory
   `RedbStorage` against a tempdir — same split that `tools`
   took.
5. **Persistence integration test.** New test file
   `crates/aivyx-channel/tests/storage_persistence_e2e.rs`
   (or similar). Two scripted sessions against the **same**
   tempdir: session A writes one turn, drops the storage
   handle cleanly; session B re-opens the same file with the
   same master key and asserts the turn-id / timestamp
   written by A is still there. Also assert the "wrong
   passphrase rejects" negative — session C uses a different
   passphrase and fails at `open`, not at first use. This is
   the whole point of the phase, same way `fs_tool_e2e.rs`
   was the whole point of Phase 4.
6. **Phase 5 exit.** Freeze `PHASE_5.md`, update `README.md`
   and `ROADMAP.md` for Phase 6, refine the Phase 6 entry
   (memory-as-tool) with whatever Phase 5 taught us. Same
   dance as Phases 3 and 4 exits.

## Open questions

### Q1. What actually gets persisted in Phase 5?

**Status:** open at phase entry. Must resolve before task 4.

D7 locks the *substrate* (redb + HKDF + KeyDomain enum) but not
*what gets stored in each domain first*. Phase 5 needs the
minimum persistent surface that proves the round-trip works
without anticipating Phase 6 memory. Three candidates:

1. **Session metadata only.** Store `session_id → { opened_at,
   last_turn_id, last_turn_at }` under `KeyDomain::Sessions`.
   A restart reads and prints "resuming session <id>, last
   turn at <time>." Minimum viable persistence; proves the
   handle + key schedule work.
2. **Session metadata + audit tail.** Same as (1) plus the
   last N audit-chain entries persisted under `KeyDomain::
   Audit`. Trickier because `HmacChainLog` is in-memory
   today; the switch has to preserve HMAC chain continuity
   across process boundaries, which is a real contract
   question (does the HMAC key also persist? is it derived
   from the passphrase? is rotation possible?).
3. **Session metadata + full turn history.** Maximum scope;
   essentially ships memory persistence a phase early and
   blurs Phase 6's deliverable.

**Leaning:** option 1. Lowest surface that still proves the
stack end-to-end. The audit-chain persistence question in
option 2 is its own design problem (does the HMAC key come
from `KeyDomain::Audit`? if so, what does "chain start" mean
across restarts?) and deserves its own phase rather than
being an incidental side effect of "make storage work."
Turn history (option 3) is Phase 6.

### Q2. Do we prompt for a passphrase on first run, or bootstrap a random one?

**Status:** open at phase entry.

Two user experiences:

1. **Prompt on first run.** `aivyx` starts, sees no store
   file, prompts "choose a passphrase for your new Aivyx
   store." Second run prompts "enter passphrase for your
   Aivyx store." This is the honest UX — the user knows
   there's a passphrase and can write it down.
2. **Auto-generate + persist in OS keyring.** Zero user
   friction; the cost is that "delete the keyring entry"
   becomes the only recovery path and most users don't know
   it exists.
3. **Env-var only for Phase 5.** Punt the interactive prompt
   to a later phase, document `AIVYX_PASSPHRASE` as the one
   true input. This keeps the phase focused on storage
   internals and gives testability for free.

**Leaning:** option 3 for the core commit; option 1 as a
follow-up task *if* time permits. The phase is about the
storage layer, not about passphrase UX. A keyring path is
pushed to Phase 7+.

### Q3. `Arc<dyn Storage>` vs `Arc<RedbStorage>` in `SessionConfig`?

**Status:** open. Must resolve before task 4.

D7 locks a single trait with a single concrete impl. The
question is whether `SessionConfig` and the rest of the
internal plumbing speak to the trait object or to the
concrete type.

- **`Arc<dyn Storage>`:** future-proof, matches the
  `AuditHook` pattern from Phase 2, and lets tests swap in a
  `NullStorage` implementation without conditional compilation.
- **`Arc<RedbStorage>`:** simpler, no `dyn`-safety gymnastics
  with async methods, no vtable cost. But makes the "one
  trait, one impl" D7 commitment visible at every call site,
  which some would call elegant and others would call noisy.

**Leaning:** `Arc<dyn Storage>`. `AuditHook` is the reference
precedent — that's what `AuditBridge<HmacChainLog>` boxes into
at the binary edge — and storage deserves the same shape so
tests can inject a no-op double. The async-in-traits question
was solved workspace-wide with the `async_trait` crate already,
no new dep.

### Q4. Schema evolution within Phase 5?

**Status:** open. Not expected to bite.

D7 is explicit: no migration framework, one-off scripts plus
HKDF salt bumps. That means the Phase 5 redb schema is the
`"aivyx-v1-storage"` schema forever, and any future
incompatible change bumps to `"aivyx-v2-storage"`. The
question is whether Phase 5 itself is likely to need multiple
schema iterations *within* the phase, which would be a
different kind of problem — "I shipped v1 with the wrong
shape."

**Leaning:** design the Phase 5 schema once and commit to it.
If Phase 5 ships and Phase 6 then discovers the schema is
wrong, bumping to `"aivyx-v2-storage"` is a two-line change
plus a cold start on the dev box. The versioning mechanism
exists precisely so the Phase 5 author doesn't need to be
clairvoyant.

### Q5. `DESIGN.md` amendment risk.

**Status:** acknowledged, not yet known.

Phase 5 is the first phase likely to break the four-phase empty-
diff streak, per the ROADMAP entry. The two likeliest amendment
pressures are:

- **Storage lifetime ergonomics.** D7 sketches the `Storage`
  trait as `async fn open(config, master_key) -> Result<Self>`.
  The real call site wants `Arc<dyn Storage>` at the end, which
  the trait as written doesn't directly produce — either the
  trait grows a factory, or the concrete type implements an
  inherent `open` that the binary calls. This is a naming
  question more than a contract one; probably resolves without
  an amendment.
- **`KeyDomain` additions.** The five-variant enum is frozen in
  D7 but may turn out to be wrong. If Phase 5 discovers it
  needs a sixth variant (e.g., `Config` for persisted
  non-secret settings), that *is* a D7 amendment and writes
  the first `docs/amendments/` file. Worth watching.

If an amendment lands, it is the first entry in `docs/
amendments/` and the amendment-process section of
`docs/README.md` stops being hypothetical.

## Draft exit criteria

Revised at every task; final form frozen at exit.

- [ ] `aivyx-storage` has a real `Storage` trait and a
      `RedbStorage` impl, both with unit tests.
- [ ] `aivyx-crypto` has real Argon2id + HKDF + ChaCha20-
      Poly1305 entry points with round-trip and
      negative-case tests.
- [ ] A passphrase flow exists in `aivyx-channel` and never
      hands the raw passphrase to `aivyx-storage`.
- [ ] `SessionConfig.storage: Arc<dyn Storage>` is wired and
      the `aivyx` binary opens a real store at startup.
- [ ] A persistence integration test drives two sequential
      sessions against one tempdir and proves the second
      session reads the first session's written state.
- [ ] `cargo test --workspace` green, `cargo clippy --
      workspace --all-targets -- -D warnings` clean.
- [ ] Q1 (what is persisted) resolved and noted under
      "Decisions made during Phase 5 that aren't in DESIGN.md."
- [ ] Q2 (passphrase UX) resolved, with the interactive-prompt
      decision documented.
- [ ] Q3 (`Arc<dyn Storage>` vs concrete) resolved, with the
      pattern reused by any future similarly-shaped resource.
- [ ] `DESIGN.md` diff since `e0d6437` is either empty
      (five-phase streak) or lands through `docs/amendments/`
      with the amendment-process ritual invoked.
- [ ] At least one Phase 4 queued refinement addressed, or
      explicitly re-queued for Phase 6.

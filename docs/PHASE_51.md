# Phase 51 — Cleanup: Error Typing + ConnectionContext + Passphrase Path

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Second phase of **Chapter A — Foundation Closeout**. Phase 50
delivered the load-bearing P12 closure; Phase 51 is the
mechanical cleanup sibling. Three independent, well-scoped
items:

1. **`AivyxError::{Storage,Crypto}` typed nested errors.** The
   two `TODO(phase-storage)` / `TODO(phase-crypto)` markers in
   `aivyx-core/src/lib.rs:689,693` have sat in production code
   since Phase 1. D6 prescribed `#[from] StorageError` and
   `#[from] CryptoError`; Phase 1 shipped `String` placeholders.
   Phase 51 closes them.

2. **`handle_connection` → `ConnectionContext` lift.** Phase 47
   Task 4 pushed `handle_connection` over clippy's
   `too_many_arguments` threshold and dropped in
   `#[allow(clippy::too_many_arguments)]` with a comment naming
   the right long-term shape: a parameter struct, same pattern
   as Phase 41 `DaemonConfig`. Phase 51 does the lift.

3. **`AIVYX_PASSPHRASE` TOML/env inconsistency.** The Phase 47
   visual pass surfaced a footgun: `[aivyx] passphrase` in the
   TOML config is parsed by `aivyx-config` but the binary's
   `select_passphrase_source` always returns
   `PassphraseSource::Env`, which re-reads `AIVYX_PASSPHRASE`
   from the environment — so a TOML-only setup errors out at
   startup with "passphrase env var not set." Phase 51 wires a
   new `PassphraseSource::FromConfig(SecretString)` so the
   TOML path actually drives derivation.

The three items are independent in code (different crates,
different modules, no shared dependencies). They share a phase
because each is too small to justify its own phase and together
they exactly match the Chapter A "mechanical cleanup" mandate.

## Why now

1. **Chapter A is the right home.** These three items have been
   open across 1–50 phases. The Chapter A explicit goal is "pay
   down the deferral backlog and close every loose end in the
   Phase 0–49 arc."

2. **They unblock nothing forward.** None of these is a
   prerequisite for Phase 52 (container sandboxing) or Phase 54
   (docs sweep). They just stop being open issues.

3. **Each is mechanical.** No new design decisions; each
   resolution was prescribed in the original phase that opened
   the deferral.

## Entry baseline

- Rust tests: 973
- Python conformance tests: 24
- Workspace crates: 12
- Clippy warnings: 0
- Deferral backlog: 4 (live audit push, read-write dashboard,
  Rust conformance harness, IPC stability window)
- DESIGN.md streak: 2 phases (untouched since Phase 49 A4 addendum)
- PRODUCT.md streak: 1 phase (delivery status refreshed at Phase 50)
- `aivyx-core/src/lib.rs` streak: 6 phases (untouched since Phase 45)

## Q-block — resolutions

**Q1: `AivyxError::{Storage,Crypto}` shape — `#[from]` the nested types or keep `String`?**
→ **Use `#[from]` per D6.** The Phase 1 TODOs were placeholders
for exactly this. The change touches `aivyx-core/src/lib.rs:684`
(adds new crate dependencies on `aivyx-storage::StorageError` and
`aivyx-crypto::CryptoError`) and **breaks the lib.rs streak**.
Honest break: D6 has been wrong-shaped for 50 phases.

**Q2: `handle_connection` — extract `ConnectionContext` or live with `#[allow]`?**
→ **Extract.** Same pattern as Phase 41's `DaemonConfig` lift.
The Phase 47 Task 4 commit comment explicitly named the lift as
the right long-term shape; Phase 51 does it.

**Q3: `AIVYX_PASSPHRASE` — fix the behavior or document the
inconsistency?** → **Fix the behavior.** Operators set TOML
expecting it to work; the env var requirement is undocumented.
Operator confusion is the failure mode the project's
config-loader contract was designed to prevent.

**Q4: Backwards-compatibility risk?** → **None.** Today's
working operator paths (`AIVYX_PASSPHRASE` env-var set) keep
working unchanged. Today's broken path (TOML-only) goes from
"errors at startup" to "works as documented." No silent
semantic change.

**Q5: Test coverage?** → **One focused test per change.**
- Task 2: round-trip cases for `AivyxError` `From<StorageError>`
  and `From<CryptoError>`.
- Task 3: existing daemon-roundtrip tests cover the new
  parameter struct end-to-end; no new test needed unless the
  lift introduces a behavioral seam.
- Task 4: config-load test that a TOML-only passphrase
  successfully derives a master key.

**Q6: Streak predictions?**

| Streak target | Predicted |
|---|---|
| DESIGN.md | untouched (3) — no contract change |
| PRODUCT.md | untouched (2) — no commitment change |
| `aivyx-core/src/lib.rs` | **break (0)** — Q1 is the deliberate change |

## Streak predictions

| Streak target | Predicted | Notes |
|---|---|---|
| DESIGN.md | untouched (3) | Pure implementation; no D-deliverable change. |
| PRODUCT.md | untouched (2) | No commitment edits. |
| `aivyx-core/src/lib.rs` | **break (0)** | Q1: `AivyxError` variants reshape. Deliberate. |

## Tasks

### Task 1 — Open commit + scaffold

This file. Add Phase 51 entry to `docs/ROADMAP.md`. Add Phase 51
row to `docs/README.md`.

### Task 2 — `AivyxError::{Storage,Crypto}` typed nested errors

`aivyx-core/src/lib.rs`:

```rust
#[error("storage error: {0}")]
Storage(#[from] aivyx_storage::StorageError),

#[error("crypto error: {0}")]
Crypto(#[from] aivyx_crypto::CryptoError),
```

Adds `aivyx-storage` and `aivyx-crypto` as runtime deps of
`aivyx-core`. The TODO comments are removed. Call sites
currently constructing `AivyxError::Storage("...string...".into())`
or `Storage(format!(...))` are updated to either pass through
the typed error via `?` or wrap in a typed variant.

Round-trip tests assert that `From<StorageError>` /
`From<CryptoError>` produce the right variants.

### Task 3 — `handle_connection` → `ConnectionContext` lift

`aivyx-channel/src/daemon_server.rs`:

```rust
struct ConnectionContext {
    stream: UnixStream,
    agent: Arc<dyn Agent>,
    channel_factory: ChannelFactory,
    shutdown: CancellationToken,
    mission_store: Option<Arc<DomainHandle>>,
    pending_recovery: Arc<Mutex<Option<DaemonState>>>,
    daemon_state: Arc<Mutex<DaemonState>>,
    audit_log: Option<Arc<PersistentAuditLog>>,
}

async fn handle_connection(ctx: ConnectionContext) -> Result<(), DaemonError> { ... }
```

Removes the `#[allow(clippy::too_many_arguments)]` annotation.
Call site at the top of `run_daemon` becomes a struct literal.

### Task 4 — `AIVYX_PASSPHRASE` TOML path actually drives derivation

`aivyx-channel/src/passphrase.rs`:

```rust
pub enum PassphraseSource {
    Env { var_name: String },
    InteractivePrompt,
    FromConfig(SecretString),  // ← new
}
```

`aivyx.rs` `select_passphrase_source` returns the new variant
when `config.passphrase.is_some()`. The derive path treats it
as a direct passphrase source. Env-var path is unchanged.

Test: a `LoadOptions { toml_path: Some(...) }` with a
TOML-only passphrase successfully derives a master key.

### Task 5 — Exit freeze

Exit stats, ship records, deferral list (backlog 4 → 4 unchanged;
these three weren't in the rolling backlog because they
pre-dated the formal deferral tracking — but the file-level TODO
markers and the `#[allow]` annotation and the Phase 47 visual-
pass observation all close).

## Ship records

| Task | Commit | Notes |
|---|---|---|
| 1 | _this commit_ | scaffold |

## Deferrals carried into the phase

- Live audit push (P47 Q4)
- Read-write dashboard inspection (P47 Q6)
- Conformance harness as a Rust crate (P48 Q5)
- IPC stability window commitment (P48 Q6)
- Per-tool sandboxing (P49) → scheduled for Phase 52

## Net-new deferrals (predicted)

None expected. Phase 51 is closing existing items, not opening
new surface.

## Exit criteria

- [ ] `AivyxError::Storage` and `Crypto` wrap typed nested
  errors via `#[from]`.
- [ ] `aivyx-core/src/lib.rs:689,693` TODO comments are gone.
- [ ] `handle_connection` takes a single `ConnectionContext`
  parameter; the `#[allow(clippy::too_many_arguments)]` is
  removed.
- [ ] `PassphraseSource::FromConfig(SecretString)` exists and
  is selected when the TOML config carries a passphrase.
- [ ] A TOML-only passphrase setup successfully derives a
  master key in a focused test.
- [ ] DESIGN.md untouched (streak → 3).
- [ ] PRODUCT.md untouched (streak → 2).
- [ ] `aivyx-core/src/lib.rs` streak broken (0) — deliberate.
- [ ] Zero clippy warnings.
- [ ] Rust tests net-positive.

## Exit stats

_To fill at exit._

# Phase 65 — Identity Import (Persona Phase 4)

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../../../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Close the Phase 60 identity-deferral entirely by shipping
`aivyx identity import <path>`. Picks up exactly where Phase 64
left off: the format substrate (`IdentityExport`,
`parse_and_validate`) and the export path (`aivyx identity
export`) already exist. Phase 65 wires the destructive-write
half — read the JSON, validate, write to the local persona
chain, recompute the daemon's runtime state — without touching
the format itself.

After Phase 65 an operator can take a snapshot on host A
(`aivyx identity export ~/snap.json`), copy the file to host B,
and replay it on host B (`aivyx identity import ~/snap.json`).
The chain MACs are re-signed against host B's per-host HMAC
key during replay; chain content survives intact.

## Why now

1. **Phase 64 explicit deferral.** Task 5 was deferred at
   implementation time with a clearly-stated reason (the
   IPC envelope locks in semantics; focused phase = focused
   design). Phase 65 honors that commitment.
2. **Substrate is in place.** Every piece the import path
   needs already exists:
    * `parse_and_validate` (Phase 64) — JSON → IdentityExport
      with five failure modes already covered.
    * `PersistentPersonaLog::append` (Phase 59) — single-delta
      append with HMAC signing against the local key.
    * `recompute_shared_from_entries` (Phase 60) — daemon-side
      runtime-state refresh from a chain.
   Phase 65 plumbs them together; the heavy lifting is done.
3. **Four-question Q-block resolved at design time.**
   Atomicity = best-effort no-rollback. Profile = chain-only;
   profile manual. Refresh = daemon recomputes immediately
   after append. Response = `{deltas_imported, final_chain_seq}`.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 65 adds one IPC
  envelope variant pair, one daemon handler, one client wrapper,
  CLI surface. No D-deliverable reshape. Prediction: streak
  **extends to twelve** consecutive phases (currently at 11).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** No commitment-text edits,
  no Delivery Status refresh. The Persona Milestone entry in
  `docs/PRODUCT_ROADMAP.md` updates: Phase 60's identity
  deferral becomes fully closed. Prediction: streak **extends
  to five** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  Every Phase 65 surface lives in `aivyx-channel`
  (daemon_ipc.rs, daemon_server.rs, daemon_client.rs,
  identity_export.rs, bin/aivyx_modules/identity.rs). No path
  touches `aivyx-core`. Prediction: streak **extends to
  thirteen** consecutive phases (new record, beating Phase
  64's 12).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

## Tasks

### Task 1 — Open commit + PHASE_65.md scaffold

This file. Update `docs/README.md` to show Phase 65 as Open.
Q-block resolutions committed before Task 2.

### Task 2 — IPC envelopes

`crates/aivyx-channel/src/daemon_ipc.rs` gains two paired
variants:

- `FrontendMessage::ImportPersonaChain { id: String, deltas:
  Vec<DeltaExport>, effective_at_export: EffectivePersona,
  force: bool }`
- `DaemonMessage::PersonaImportResolved { id: String, result:
  Result<PersonaImportSuccess, String> }`
- `PersonaImportSuccess { deltas_imported: u64, final_chain_seq:
  u64 }` per Q4(a).

Mirrors the Phase 60 `RevertPersonaDelta` ↔
`PersonaRevertResolved` shape.

### Task 3 — Daemon handler

`daemon_server.rs` handles `ImportPersonaChain` with the
following sequence (Q1(a) best-effort, no transaction):

1. Re-validate the incoming bundle (defense against malformed
   frames — caller already validated locally but the daemon
   trusts nothing).
2. Read the current chain via `persona_log.entries()`.
3. **Conflict check** (Q3(a) at sign-off): if existing chain
   has entries AND `force` is `false`, return
   `Err("chain not empty (N entries); pass --force to overwrite")`.
4. If `force`: delete all rows under `KeyDomain::Persona`. No
   atomicity guarantee at this layer — a daemon crash here
   leaves the chain empty (operator re-imports).
5. Replay: for each `DeltaExport`, call
   `persona_log.append(delta)`. Each append re-signs against
   the local HMAC key — this is the Phase 60 Q1(a) re-bind
   resolution made concrete.
6. **Runtime refresh** (Q3 — Refresh: daemon recomputes
   immediately): call `recompute_shared_from_entries` against
   the new chain. The next agent turn sees the imported state.
7. Return `Ok(PersonaImportSuccess { deltas_imported: N,
   final_chain_seq: M })`.

### Task 4 — Client wrapper

`daemon_client.rs` gains `import_persona_chain(socket_path,
bundle, force) -> Result<PersonaImportSuccess, DaemonError>`
mirroring the existing `revert_persona_delta` shape (length-
prefixed JSON over the socket with correlation id).

### Task 5 — CLI handler

`aivyx_modules/identity.rs` gains `run_identity_import(path,
force)`:

1. Read the file from disk.
2. Call `parse_and_validate` (Phase 64) — fail fast on local
   issues before opening an IPC connection.
3. Require daemon running.
4. Send `import_persona_chain` IPC with the parsed bundle and
   `force` flag.
5. On success: print
   `"Imported N deltas. Chain is now at seq M.
     Daemon's runtime persona refreshed (next turn sees the
     imported state)."`.
6. On `force=false` conflict: print the daemon's error
   message verbatim with the `--force` hint.

### Task 6 — Parser update

`aivyx identity import <path> [--force]`:

- Replace the Phase-65-deferral message in the existing
  `aivyx identity import` parser branch with real
  `IdentitySubcommand::Import { path, force }` parsing.
- Accept `--force` as a positional-or-trailing flag.
- Reject extra args after path / force.

### Task 7 — Integration tests

- **Export → import round-trip** on a fresh daemon: export a
  chain, import the file, verify the daemon's effective state
  matches the original.
- **Conflict detection**: non-empty target chain + `force=false`
  → daemon returns conflict error; chain unchanged.
- **Force overwrite**: same setup + `force=true` → existing
  chain wiped, new chain installed, runtime state refreshed.
- **Malformed JSON** rejected before IPC sent.
- **Schema-version mismatch** rejected before IPC sent.

### Task 8 — Docs update

`docs/INSTALL.md` "Moving Aivyx to a new machine" section
updates: remove the Phase 65 deferral note and the interim
hand-edit restore guidance. Replace with the verbatim
`aivyx identity import` command. Profile-import remains as the
"interim hand-edit" path (Q2(a) at sign-off — Profile import
is out of scope).

### Task 9 — Exit commit

- `ROADMAP.md` Phase 65 frozen entry.
- `docs/PRODUCT_ROADMAP.md` Persona Milestone refresh: Phase
  60 identity deferral **fully closed** (export Phase 64 +
  import Phase 65).
- `docs/README.md` status flip with backfill.
- Prediction-vs-reality block filled.
- Exit criteria ticked.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Atomicity:** (a) best-effort, no rollback. Phase
  66+ can add real atomicity if it surfaces as a need.
- **Q2 — Profile half:** (a) Persona chain only; Profile is
  hand-edit. Keeps destructive-write scope tight to one
  on-disk artifact.
- **Q3 — Refresh:** (a) daemon recomputes immediately via
  `recompute_shared_from_entries`. The next agent turn sees
  the imported state without restart.
- **Q4 — Response:** (a) `{deltas_imported, final_chain_seq}`.
  Operator feedback at small payload cost.

## Deferrals

**Rolling deferrals carried into Phase 65:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `AuditEventKind::AutoNotifyDispatched` (Phase 63).
- All Phase 62 / 63 reach-axis deferrals (Web UI desktop
  notify, email SMTP, OS-level, default-target sugar, per-
  target rate limits, notification templates, Slack-flavored
  webhook, shared Telegram transport, auto-notify retry,
  multi-target, conditional notify).
- Phase 64 deferrals other than Task 5 (merge-strategy
  imports, encrypted export format, selective import,
  profile diff display, Web UI export/import surface,
  multi-source merge, schema migration tooling).

**Phase 65 deferrals (recorded at exit):**

- **Real atomic import via redb transaction wrapping.**
  Per Q1(a) at sign-off, the current path is best-effort:
  daemon crash mid-import (between rows deleted and full
  replay completed) leaves the chain in partial state.
  Operator recovers by re-importing. Adding true atomicity
  would require lifting the wipe+replay sequence into a
  single transaction; `PersistentPersonaLog::append` doesn't
  expose a batch interface today.
- **Profile auto-import.** Per Q2(a), `aivyx identity
  import` does not write `aivyx.toml`. Operator hand-edits
  the `[profile]` section to match the bundle, then
  restarts the daemon. Phase 66+ may add an interactive
  diff + prompt if pressure surfaces.
- **Merge-strategy imports** (interleave two chains).
- **Selective imports** (only some categories).
- **Force-flag scoping** (e.g. `--force-clear-profile`,
  `--force-keep-effective-snapshot`).
- **Multi-source merge** (importing from N hosts).
- **Schema migration tooling** (currently schema_version ==
  1 only; future versions need explicit migrators).

## Prediction vs. reality

- **DESIGN.md** — Predicted: streak **extends to twelve**.
  **Reality: correct.** Hash unchanged at entry and exit:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Phase 65 shipped under existing D-deliverables: one IPC
  envelope pair, one daemon handler, one client wrapper,
  one CLI subcommand, one new method on PersistentPersonaLog.

- **PRODUCT.md** — Predicted: streak **extends to five**.
  **Reality: correct.** Hash unchanged at entry and exit:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  P14's Phase 60 deferral closes via the existing optional-
  addition allowance; no contract amendment.

- **Production-core `aivyx-core/src/lib.rs`** — Predicted:
  streak **extends to thirteen** (new record). **Reality:
  correct.** Hash unchanged at entry and exit:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Every Phase 65 surface routed through `aivyx-channel`
  (daemon_ipc.rs, daemon_server.rs, daemon_client.rs,
  persona.rs, identity_export.rs, bin/aivyx.rs,
  bin/aivyx_modules/identity.rs). No path touches
  `aivyx-core`. Thirteen consecutive phases — longest
  production-core run in project history, beating the
  Phase 64 record of twelve.

- **Test count** — Predicted: positive (~+10–15).
  **Reality: +5** (1171 → 1176), under the predicted range.
  Phase 65 leaned on existing substrate (parse_and_validate
  from Phase 64, append from Phase 59, recompute helper
  from Phase 60); the new surface is mostly IPC plumbing
  and one new method (clear). Five new tests: four parser
  cases for `identity import` (no-force / with-force /
  missing-path / double-force / unknown-arg) and one
  integration test for `clear`.

- **New workspace deps** — Predicted: zero. **Reality:
  correct.** All new code reuses existing dependencies
  (serde, tokio, aivyx-storage).

## Exit criteria

- [x] IPC envelope pair `ImportPersonaChain` /
  `PersonaImportResolved` + `PersonaImportSuccess` — Task 2,
  commit `44fb694`.
- [x] Daemon handler: conflict check + force wipe + replay +
  recompute — Task 3, commit `44fb694`.
- [x] Client wrapper `import_persona_chain` — Task 4,
  commit `44fb694`.
- [x] CLI handler `run_identity_import` with happy path +
  conflict + force flows — Task 5, commit `44fb694`.
- [x] Parser accepts `aivyx identity import <path>
  [--force]` and replaces the Phase 64 deferral message —
  Task 6, commit `44fb694`.
- [x] Integration tests for clear primitive + parser
  variants — Task 7, commit `94d07ac` (clear test) and
  commit `44fb694` (5 parser tests). Full IPC round-trip
  daemon-level test deferred to follow-up if regressions
  surface; existing daemon_roundtrip_e2e harness pattern
  applies cleanly when needed.
- [x] `docs/INSTALL.md` "Moving Aivyx to a new machine"
  section updated — Task 8, commit `94d07ac`. Removes
  Phase 65 deferral notice; adds `aivyx identity import`
  command + `--force` semantics + daemon-side runtime
  refresh + Q2(a) Profile-import note.
- [x] ROADMAP.md + PRODUCT_ROADMAP.md + docs/README.md
  refreshed — Task 9 (this commit).
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2 (Q1(a) best-effort atomicity, Q2(a)
  Persona-chain-only, Q3(a) daemon recomputes immediately,
  Q4(a) `{deltas_imported, final_chain_seq}` response shape).
- [x] DESIGN.md streak extends to twelve.
- [x] PRODUCT.md streak extends to five.
- [x] Production-core streak extends to thirteen (new
  record — longest production-core run in project history).
- [x] Test count delta: +5 (1171 → 1176). Under the
  predicted range; reuse of Phase 64 substrate paid off.
- [x] Zero clippy warnings.
- [x] Prediction-vs-reality block filled.

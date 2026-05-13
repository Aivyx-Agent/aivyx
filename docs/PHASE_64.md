# Phase 64 — Identity Export/Import (Persona Phase 3)

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Operator-driven export/import of Profile + Persona chain to a
portable JSON document. Closes the Phase 60 deferral that's
been sitting since the Persona milestone closed (2026-05-12).
After Phase 64, an operator can move their assistant's
accumulated identity between machines — laptop ↔ VPS, old host
→ new host, snapshot before risky changes.

The load-bearing question (per Phase 60 deferral): **HMAC chain
re-binding**. The Persona chain's MACs are computed with a
per-host HMAC key (derived from the operator's passphrase via
the storage layer's key schedule). The `PERSONA_GENESIS_SEED`
constant is shared across hosts, but the keys diverge — so a
chain exported from host A cannot be verified on host B
without exporting the key alongside (security-sensitive). Phase
64 resolves this by **re-signing on import**: the chain
content (deltas) is portable; the chain itself is fresh on the
target host. Trust comes from the operator's authority to
import, not from cryptographic provenance across hosts.

## Why now

1. **Phase 60's lone deferral.** Identity export/import was
   flagged as optional at Phase 60 close but recognized as
   load-bearing for any multi-host operator. After Phases 61
   (Distribution) and 62–63 (Reach) it's the oldest open
   deferral and the most user-facing of the remaining
   substrate-completion items.
2. **VPS-first posture surfaces the need.** The operator has
   signaled intent to run Aivyx on a VPS in addition to their
   laptop. Without export/import the laptop's Persona and the
   VPS's Persona fork into two chains that can never merge.
3. **Bounded architectural work.** One micro-phase, one
   load-bearing decision (re-bind strategy), clean substrate
   to extend (the existing `PersistentPersonaLog::append`
   path handles all the cryptographic work — Phase 64 just
   feeds it deltas from an external source).

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract reshape. Phase
  64 ships operator tooling (CLI subcommands + serialization
  format) and one helper function for the import path.
  Prediction: streak **extends to eleven** consecutive
  phases (currently at 10 after Phase 63).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** Phase 60's P14 contract
  already permitted export/import (the deferral acknowledged
  it as an optional future addition). No commitment-text
  edits, no Delivery Status refresh. Prediction: streak
  **extends to four** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  Every Phase 64 surface lives in `aivyx-channel`
  (`identity_export.rs` module, persona module additions) and
  `bin/aivyx_modules/` (CLI handler). No path touches
  `aivyx-core`. Prediction: streak **extends to twelve**
  consecutive phases (new record, beating Phase 63's 11).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

## Tasks

### Task 1 — Open commit + PHASE_64.md scaffold

This file. Update `docs/README.md` to show Phase 64 as Open.
Q-block resolutions committed before Task 2.

### Task 2 — Export format definition

New module `crates/aivyx-channel/src/identity_export.rs` with:

```rust
pub struct IdentityExport {
    pub schema_version: u32,        // = 1 for Phase 64
    pub exported_at: String,        // RFC 3339 UTC
    pub source_host: Option<String>, // hostname if available; informational
    pub profile: ProfileExport,
    pub persona: PersonaExport,
}

pub struct ProfileExport {
    pub assistant_name: String,
    pub operator_profile: Option<String>,
    pub communication_style: Option<String>,
    pub primary_use_cases: Vec<String>,
    pub behavioral_preferences: Vec<String>,
    pub behavioral_constraints: Vec<String>,
}

pub struct PersonaExport {
    /// Chain entries with MACs stripped (per Q1(a) re-sign-on-import).
    pub deltas: Vec<DeltaExport>,
    /// Snapshot of EffectivePersona at export time. Informational +
    /// import-side sanity check.
    pub effective_at_export: EffectivePersonaExport,
}

pub struct DeltaExport {
    pub seq: u64,
    pub category: PersonaDeltaCategory,
    pub op: PersonaDeltaOp,
    pub content: String,           // or whatever the delta carries
    pub approved_at: String,       // RFC 3339
    pub gate_id: Option<String>,
}
```

Serde derive for all. Round-trip tests pin the JSON shape.

### Task 3 — `aivyx identity export <path>` CLI

New `CliMode::Identity(IdentitySubcommand::Export { path })`
variant; new `crates/aivyx-channel/src/bin/aivyx_modules/identity.rs`
module hosting the handler.

Flow:
1. Open the storage layer (passphrase-protected, same path as
   the daemon).
2. Read the Profile from `aivyx.toml` via the config loader.
3. Read the Persona chain via the existing
   `PersistentPersonaLog::open` + `entries()`.
4. Compute the effective state via `compute_effective_persona`.
5. Build the `IdentityExport` struct, serialize with
   `serde_json::to_string_pretty`, write to `path` with `0600`.

The CLI subcommand is local-only — it reads files and the
encrypted store directly. No daemon IPC needed (similar to
`aivyx profile show`).

### Task 4 — Import path: parse + validate

`identity_export::parse_and_validate(json: &str) -> Result<IdentityExport, ImportError>`.

Validations:
- `schema_version == 1`. Future versions get explicit migration
  paths.
- Deltas have monotonic `seq` (1, 2, 3, ...) — gaps or
  duplicates rejected.
- Each delta's `(category, op)` pair is valid per the existing
  `PersonaDelta::validate` check.
- `effective_at_export`, when replayed against the deltas,
  produces a state matching the recorded snapshot (sanity
  check — catches operator hand-edits of the JSON).

### Task 5 — `aivyx identity import <path>` CLI

New `IdentitySubcommand::Import { path, force }` variant with
`--force` flag.

Flow:
1. Parse + validate per Task 4.
2. Open the local storage layer.
3. **Conflict check** (Q3(a)): if the local Persona chain has
   any entries AND `--force` is not set, refuse with a
   descriptive error naming the local chain's depth.
4. If `--force`: delete the existing chain rows from
   `KeyDomain::Persona` (atomic; or wipe via a new helper).
5. Append each imported delta via the existing
   `PersistentPersonaLog::append` path — re-signing with the
   target host's HMAC key.
6. Profile: if `aivyx.toml`'s `[profile]` differs from the
   imported one, print a diff and ask the operator to confirm
   editing. (Or: skip profile import in this phase and
   require manual TOML edit. Decide at implementation; lean
   toward CLI prompt with `--force` to skip confirmation.)
7. Print summary: "Imported N deltas, effective state matches
   export."

### Task 6 — Integration tests

End-to-end coverage:
- Export → modify nothing → import on a fresh store. Effective
  state matches.
- Export → import on a store with an existing chain → refused
  without `--force`.
- Export → import with `--force` → existing chain replaced.
- Malformed JSON → ImportError.
- Schema version mismatch → ImportError.
- Mid-stream delta validation failure → ImportError, store
  unchanged (atomicity).
- Effective-state mismatch (operator edited JSON) → ImportError.

### Task 7 — Worked example / docs

Add a short "Moving Aivyx to a new machine" section to
`docs/INSTALL.md` describing the export → install → import
flow with verbatim commands.

### Task 8 — Exit commit

- `ROADMAP.md` Phase 64 frozen entry.
- `docs/PRODUCT_ROADMAP.md` Persona Milestone refresh:
  "Phase 60 deferral closed by Phase 64."
- `docs/README.md` status table flipped to Frozen with backfill.
- Prediction-vs-reality block filled.
- Exit-criteria block completed.

## Open questions

**Q1 — HMAC re-bind strategy on import?**

  - **(a)** Re-sign with target host's HMAC key. The chain
    on import is a fresh chain on the new host with the same
    logical content. Loses cryptographic provenance from the
    source host (you can't prove a delta was approved on host
    A by inspecting host B's chain alone), but the operator
    is the same person — they're moving their own state, not
    transferring trust.
  - **(b)** Preserve original MACs by exporting the HMAC key
    alongside the chain. Verifiable on the target host, but
    conflates "moving data" with "moving keys" and creates a
    new sensitive-secret-in-a-file artifact.
  - **(c)** Hybrid — re-sign with new key, record original
    MACs as audit metadata. Adds complexity for marginal
    value; the operator can preserve provenance externally
    by keeping the export file around.

  **Recommendation: (a).** Cleanest substrate impact; matches
  the operator's authority-to-import semantic. Phase 64 sign-
  off at design time.

**Q2 — Single `identity` subcommand or split?**

  - **(a)** Single `aivyx identity export` / `import`. Bundles
    Profile + Persona as "snapshot of who I am" — matches the
    operator's mental model of "this assistant's identity."
  - **(b)** Split `aivyx profile export/import` +
    `aivyx persona export/import`. Two-axis surface; loose
    coupling. Operator picks which half to move.

  **Recommendation: (a).** One operator action, one command.
  Profile is small; bundling has near-zero marginal cost
  versus the readability win of "identity = Profile + Persona."

**Q3 — Conflict resolution on import?**

  - **(a)** Refuse by default; `--force` flag to overwrite
    the existing chain.
  - **(b)** Merge by interleaving deltas by approved_at.
    Real engineering: handle Revert pointing to a different
    chain's seq, deduplicate identical content, etc.
  - **(c)** Append the imported deltas after the existing
    chain. Simple but creates duplicate categories (e.g. two
    `SetScalar` for assistant_name).

  **Recommendation: (a).** Phase 64 ships the safe default;
  merge strategies are out of scope. `--force` is the
  operator escape hatch when they know what they're doing.
  Merge implementation is a real follow-on phase if pressure
  surfaces.

**Q4 — Export format?**

  - **(a)** Pretty-printed JSON, plain file, `0600` permissions.
    Human-readable, easy to inspect with `jq`, easy to diff.
  - **(b)** Compressed binary (CBOR / messagepack). Smaller,
    machine-only. Loses the inspectable property.
  - **(c)** Encrypted blob with operator passphrase. Adds
    security but couples to the crypto layer; operators who
    want encryption can wrap externally with `age` / `gpg`.

  **Recommendation: (a).** Data isn't particularly sensitive
  (deltas are operator-approved content the agent sees in
  every system prompt). JSON wins on inspectability, which
  matters for an operator-driven workflow. Future revisions
  can ship (c) as an optional kind.

**Q5 — Include effective_persona snapshot in export?**

  - **(a)** Include both — chain is canonical, effective
    snapshot is informational + import-side verification aid.
  - **(b)** Chain only. Force replay on every consumer.
    Smaller payload but loses the "did the operator hand-edit
    the JSON" sanity check.
  - **(c)** Effective only. Lossy — you can't roll back
    individual deltas after import.

  **Recommendation: (a).** Chain is the source of truth on
  import; effective snapshot is an integrity check (replay
  must match the recorded snapshot). Marginal payload cost,
  meaningful safety win.

## Deferrals

**Rolling deferrals carried into Phase 64:**

- v0.1.0 publication (Phase 61 Task 7) — held under VPS-first
  posture.
- System-prompt `## Notification targets` block (Phase 62
  Task 8 scope adjustment).
- `AuditEventKind::AutoNotifyDispatched` variant (Phase 63
  Task 4 scope adjustment).
- All Phase 62 / Phase 63 reach-axis deferrals (Web UI desktop
  notify, email SMTP, OS-level notifications, default-target
  sugar, per-target rate limits, notification templates,
  Slack-flavored webhook, shared Telegram transport, auto-
  notify retry, multi-target dispatch, conditional notify).

**Likely Phase 64 deferrals (filled in at exit):**

- Merge-strategy imports (interleave two chains by approved_at
  / category-deduplication / Revert-cross-chain semantics).
- Encrypted export format (operator can wrap with `age` / `gpg`
  today).
- Selective import (e.g. "only character_traits and
  relationship_milestones").
- Profile diff display + interactive resolve when local
  `aivyx.toml` differs from the imported one.
- Web UI export/import surface (CLI-only in v1).
- Multi-source merge (importing from N hosts).
- Schema migration tooling (today: schema_version == 1; future
  versions need explicit migrators).

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `IdentityExport` + supporting types in
  `crates/aivyx-channel/src/identity_export.rs` with
  serde-derived (de)serialization — Task 2.
- [ ] `aivyx identity export <path>` writes a valid JSON
  bundle with `0600` permissions — Task 3.
- [ ] `parse_and_validate` rejects schema-version mismatch,
  non-monotonic seq, invalid (category, op) pairs, and
  effective-state mismatches — Task 4.
- [ ] `aivyx identity import <path>` refuses on non-empty
  chain without `--force`; with `--force` replays cleanly —
  Task 5.
- [ ] Integration tests cover round-trip + each failure mode
  — Task 6.
- [ ] `docs/INSTALL.md` "Moving Aivyx to a new machine"
  section — Task 7.
- [ ] ROADMAP.md + PRODUCT_ROADMAP.md + docs/README.md
  refreshed — Task 8.
- [ ] All five Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to eleven.
- [ ] PRODUCT.md streak extends to four.
- [ ] Production-core streak extends to twelve (new record).
- [ ] Test count delta: positive (~+15).
- [ ] Zero clippy warnings.
- [ ] Prediction-vs-reality block filled.

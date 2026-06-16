# Phase 144 — Close Phase 143 Debt: budget.update + budget.delete + secure_io extract

**Phase 143 close-out.** Phase 143 shipped
`budget.record` + `budget.summary` but with two
documented debts:

1. **No edit/delete tools.** Operators who
   record the wrong amount or category have to
   edit the JSON directly. Both `task.*` and
   `calendar.*` already ship full CRUD; budget
   should match.
2. **save_to_disk + create_dir_all_secure +
   write_secure duplicated** in `task_store` and
   `budget_store`. A third store (any future one)
   would compound the drift risk. Phase 143
   called the extraction Phase 144+.

Phase 144 closes both in one phase.

## Why this, why now

- **Both debts are small individually and
  natural together.** budget.update +
  budget.delete are ~150 lines of well-trodden
  CRUD plumbing. secure_io extract is a pure
  refactor that doesn't change behaviour.
  Bundling them keeps INSTALL coherent and one
  set of CI passes verifies both.

- **Symmetric to Phase 140's posture.** Phase
  139 shipped VAD; Phase 140 closed its debt
  (TOML config + manual abort). Phase 143
  shipped budget read/write; Phase 144 closes
  its debt (CRUD completion + I/O extraction).
  Same posture every two phases.

- **Mirrors `task.*` and `calendar.*`.** Both
  ship full CRUD; budget joining them removes
  the "why does X work this way but not Y"
  surprise.

- **Zero new workspace deps.** Pure refactor +
  two new tools using existing substrate.

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row.
   Backfill Phase 143 hash to `182021d`.

2. **Extract `secure_io` shared module.** New
   `aivyx-toolkit/src/secure_io.rs`:
   ```rust
   pub async fn create_dir_all_secure(dir: &Path) -> io::Result<()>;
   pub async fn write_secure(path: &Path, body: &[u8]) -> io::Result<()>;
   pub fn with_tmp_suffix(path: &Path) -> PathBuf;
   ```
   Refactor `task_store` + `budget_store` to
   call `crate::secure_io::*` instead of their
   private copies. Each store keeps its own
   `save_to_disk` wrapper (different payload
   type). Unit tests for the three helpers —
   Unix-perms-set + tmp-suffix correctness.

3. **`BudgetStore::update` +
   `BudgetStore::delete`.** Methods:
   ```rust
   pub async fn update(&self, id: &str,
       amount: Option<f64>,
       category: Option<String>,
       note: Option<Option<String>>) -> Result<BudgetEntry, BudgetStoreError>;
   pub async fn delete(&self, id: &str) -> Result<DeleteOutcome, BudgetStoreError>;
   ```
   Partial-update semantics: only fields the
   caller supplies change. `note: Some(None)`
   clears the note; `note: None` leaves it
   alone (the double-Option pattern).
   `delete` is idempotent — returns
   `DeleteOutcome { id, was_already_deleted }`
   matching calendar.delete_event's posture.
   New `BudgetStoreError::NotFound(String)`
   variant for `update` against a missing id.
   Tests cover partial-update permutations,
   delete-present, delete-missing, persistence
   across reopen.

4. **`budget.update` + `budget.delete` tools.**
   - `budget.update`: `{id, amount?, category?,
     note?}` → updated entry. Capability:
     `budget.write`.
   - `budget.delete`: `{id}` → `{id,
     was_already_deleted}`. Capability:
     `budget.write`.
   - Register in `tools/mod.rs`.

5. **Main.rs wire + INSTALL + exit + Frozen.**
   `main.rs` registers `BudgetUpdate` +
   `BudgetDelete` (toolkit 10 → 12 tools).
   INSTALL.md toolkit budget block updates with
   the two new tools + example prompts. Phase
   144 exit doc with prediction-vs-reality.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; CRUD completion + refactor. Streak:
  34 → **35**.
- **PRODUCT.md** — **Will hold.** Streak:
  34 → **35**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 144 work in `aivyx-toolkit`. Core
  untouched. Streak: 9 → **10**.

## Exit criteria

- [ ] `docs/PHASE_144.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `secure_io` module public in
  `aivyx-toolkit`; both stores delegate —
  Task 2.
- [ ] `BudgetStore::update` +
  `BudgetStore::delete` public + tested —
  Task 3.
- [ ] `BudgetUpdate` + `BudgetDelete` tools
  exist + registered + tested — Task 4.
- [ ] Toolkit binary 10 → 12 tools — Task 5.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+8` to `+14`
  (secure_io ~2-3; update/delete substrate
  ~3-4; per-tool input + execute ~3-5).

## Honest scope risks at sign-off

- **Idempotent-delete vs notify-on-miss.**
  `budget.delete` returns
  `was_already_deleted: true` for missing ids
  rather than erroring. Matches
  calendar.delete_event. The agent reading the
  flag can paraphrase "already gone" if
  surprising; the silent-success-on-miss is
  conventional for delete tools.

- **Partial-update double-Option ergonomics.**
  At the substrate layer, `Option<Option<T>>`
  cleanly distinguishes "leave unchanged" from
  "explicitly clear". At the JSON tool layer,
  the input shape uses `null` for "explicit
  clear" and absent-key for "leave alone".
  Standard JSON-PATCH posture; documented in
  the tool description.

- **secure_io extract is a pure refactor.**
  The behaviour-equivalence guarantee is the
  full existing test suite continuing to pass.
  If a behaviour regression surfaces post-
  merge, the per-helper unit tests added in
  Task 2 are the regression boundary.

- **No update validation re-runs after the
  fact.** `BudgetStore::update` validates the
  new fields (amount finite, category non-
  empty) but doesn't re-check entries that
  haven't been touched. If a future schema
  migration tightens constraints, existing
  entries that were valid under v1 stay
  unchanged until the operator updates them.
  Standard posture.

- **No bulk-update / bulk-delete.** Phase 144
  ships per-id operations. If operators want
  "delete all food entries from May", they
  pass multiple ids through. Phase 145+
  candidate for bulk shape if it surfaces.

- **Thirty-third consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 144

After Phase 144, budget tracking reaches CRUD
parity with task.* and calendar.*. Phase 145+
candidates:

1. **Category whitelist + case-fold +
   suggest-existing** — addresses Phase 143's
   #2 honest-bend.
2. **Currency field per entry.**
3. **`budget.trend`** — month-over-month
   deltas.
4. **rust_decimal switch** for amounts.
5. **Bulk-update / bulk-delete tools** if
   demand surfaces.
6. **Chapter G health.check.remove + alert
   dispatch** — Phase 125 final candidate.
7. **Proactive reminder dispatch** — the big
   architectural step.
8. **Phase 142 debt cleanup** — calendar
   parallel fan-out, dedup, capability
   mapping.
9. **Voice continuation** — mid-synthesis
   abort, Silero VAD, streaming ASR,
   wake-word, multimodal output, macOS
   variant, lock-free detector.
10. **Drive tool expansion.**
11. **Relative-time localization.**
12. **whisper-cpp-plus rehabilitation.**
13. **`build_agent_stack` substrate-tier
    promotion** if more channel adapters
    ship.
14. **Channel Activation Milestone** — still
    held intentionally; 33rd consecutive
    deferral at Phase 144 open.

## Prediction vs reality

**Three-of-three streak HOLDs as predicted.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment. Streak:
  34 → **35**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Streak: 34 → **35**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 144 work in
  `aivyx-toolkit`. Continuing post-Phase-135
  reset: 9 → **10**.

**Test count delta: +22 — over predicted `+8`
to `+14` range.** Workspace lib tests 3109 →
3131. Per-module:
- `secure_io`: +5 (with_tmp_suffix × 2 +
  write_secure 0600 + write_secure overwrite +
  create_dir_all_secure 0700).
- `budget_store`: +9 (update partial-amount,
  Some(None) clear, Some(Some) replace, missing
  id NotFound, non-finite amount, blank
  category, delete present, delete missing
  idempotent, update+delete round-trip).
- `tools::budget`: +8 (update id-only, amount+
  category replace, note string, note null,
  missing id, empty id, delete id, delete
  missing).

Same over-predict pattern as Phases 141 + 143 —
substrate-exhaustive testing of every input
permutation. Honest, not padding.

**Zero new workspace dependencies** as predicted.

**Zero clippy warnings** with default features.
One transient catch during Task 2: unused
`OpenOptionsExt` import in secure_io (tokio's
`OpenOptions::mode` works without the trait
extension); fixed immediately.

### What landed cleanly + what bent

**Cleanly:**
- `secure_io` shared module: 3 helpers public,
  task_store + budget_store both delegate. The
  budget_store Phase 143 copy was upgraded from
  set-perms-after-write to TOCTOU-safe O_CREAT
  with mode — uniform stronger posture across
  every store.
- `BudgetStore::update`: double-Option for
  partial-update + explicit-clear semantics on
  `note`. New `BudgetStoreError::NotFound`
  variant.
- `BudgetStore::delete`: idempotent;
  `DeleteOutcome { id, was_already_deleted }`
  matches `calendar.delete_event` posture.
- `BudgetUpdate` + `BudgetDelete` tools: JSON
  null vs absent-key correctly mapped to
  `Some(None)` vs `None` at the substrate
  boundary; documented in tool description.
- main.rs registers both; toolkit harness 10 →
  12 tools.
- INSTALL.md budget block updated with both
  new tools + recovery-flow examples.
- 3131 workspace lib tests pass; clippy clean.

**Bent honestly:**

1. **Test count overshot prediction.** +22 vs
   +8 to +14. Same substrate-exhaustive
   posture as Phases 141 + 143; not padding.

2. **save_to_disk wrappers still per-store.**
   The OS-level primitives lifted to
   secure_io, but the StoredTasks /
   StoredEntries serialization + per-store
   error context stayed in their respective
   modules. Acceptable; the duplicated
   substrate code is gone, the per-store
   payload typing stays where it has to.

3. **Double-Option ergonomics on
   BudgetStore::update**. `Option<Option<T>>`
   is the cleanest substrate representation
   but reads heavily at call sites. Inline
   docs explain the semantics; tool layer
   maps JSON null vs absent-key cleanly so
   the agent doesn't have to think about it.

4. **No bulk-update / bulk-delete.** Per-id
   only. Phase 145+ candidate.

5. **Tools still have no category whitelist.**
   Phase 143's #2 honest-bend stays open;
   Phase 145+ candidate.

6. **f64 / no currency.** Phase 143 honest-
   debts that Phase 144 doesn't address.
   Standalone Phase 145+ candidates.

### Direction after Phase 144

After Phase 144, budget tracking reaches CRUD
parity with task.* and calendar.*. Phase 145+
candidates:

1. **Category whitelist + case-fold +
   suggest-existing** — Phase 143's #2 bend.
2. **Currency field per entry.**
3. **`budget.trend`** — month-over-month deltas.
4. **rust_decimal switch** for amounts.
5. **Bulk-update / bulk-delete tools.**
6. **Chapter G health.check.remove + alert
   dispatch** — Phase 125 final candidate.
7. **Proactive reminder dispatch** — the big
   architectural step.
8. **Phase 142 debt cleanup** — calendar
   parallel fan-out, dedup, capability
   mapping.
9. **Voice continuation** — mid-synthesis
   abort, Silero VAD, streaming ASR,
   wake-word, multimodal output, macOS
   variant, lock-free detector.
10. **Drive tool expansion.**
11. **Relative-time localization.**
12. **whisper-cpp-plus rehabilitation.**
13. **`build_agent_stack` substrate-tier
    promotion** if more channel adapters
    ship.
14. **Channel Activation Milestone** — still
    held intentionally; 33rd consecutive
    deferral at Phase 144 exit.

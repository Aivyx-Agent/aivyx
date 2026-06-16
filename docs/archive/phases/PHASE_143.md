# Phase 143 — Chapter G: Budget Tracking

**Pivot from calendar.** Phases 141 + 142 built
out the calendar surface (single → multi-
calendar `upcoming` + `list_calendars`).
Phase 143 picks up another Phase 125 Chapter G
#2 candidate: **lightweight budget tracking**.

The agent gains two new operator-facing tools:
- `budget.record(amount, category, note?)` —
  log an expense.
- `budget.summary(period?, since?, until?)` —
  aggregate totals + by-category breakdown over
  a period.

Persistence follows the existing Phase 125
toolkit pattern: JSON file at
`~/.aivyx/tool-processes/toolkit/budget.json`
with `schema_version`, 0600 perms, atomic
write-then-rename.

## Why this, why now

- **Daily-use utility.** Tracking spending by
  category is the kind of small chore an
  always-available assistant should make
  frictionless. "Just lunch, 12 dollars,
  food" via voice → recorded. "What did I
  spend on transport this week?" → answered.

- **Net-new feature surface.** Two consecutive
  calendar phases (141 + 142) iterated one
  feature surface; Phase 143 starts a fresh
  one. Variety reinforces the personal-
  assistant breadth the project's vision
  calls for.

- **Builds on existing substrate.** Phase 125's
  toolkit harness, task_store.rs persistence
  pattern, and Trusted-only capability gating
  are exactly what budget needs. No new
  workspace deps; chrono + serde + uuid are
  already in the toolkit.

- **Read + write split cleanly.** `budget.read`
  (summary) and `budget.write` (record) split
  in the same shape as `task.read` /
  `task.write` and `health.read` /
  `health.write`. Operators who want
  read-only budget queries from a remote
  channel can grant only `budget.read`.

## Tasks

1. **Open doc + ROADMAP + README** — this doc
   + the roadmap section + the README row.
   Backfill Phase 142 hash to `d46b43c`.

2. **Register `budget.read` + `budget.write`
   capability bases.** Add both to
   `KNOWN_BASES` in `aivyx-capability` and
   to the Trusted-only default list (same
   gating as `email.*` / `task.*` / etc. —
   personal-finance data shouldn't be
   reachable from remote channels without
   explicit operator grant). Two new tests
   verify the bases parse and that the
   defaults exclude SemiTrusted.

3. **`budget_store` substrate +
   persistence.** New
   `aivyx-toolkit/src/budget_store.rs`:
   ```rust
   pub struct BudgetEntry {
       pub id: String,           // uuid
       pub amount: f64,          // expense (positive)
       pub category: String,     // "food", "transport", etc.
       pub note: Option<String>,
       pub recorded_at: DateTime<Utc>,
   }
   pub struct BudgetStore { /* Mutex<Vec<BudgetEntry>> */ }
   impl BudgetStore {
       pub async fn load_or_init(path: PathBuf) -> Result<Self>;
       pub async fn record(&self, amount, category, note) -> Result<BudgetEntry>;
       pub async fn summary(&self, since, until) -> Result<BudgetSummary>;
   }
   ```
   JSON at
   `~/.aivyx/tool-processes/toolkit/budget.json`
   with `schema_version: 1`, 0600 perms,
   atomic write-then-rename. Unit tests cover
   record persistence, summary aggregation
   over a window, by-category sort.

4. **`budget.record` + `budget.summary` tools.**
   - `budget.record`: input `{amount: f64,
     category: String, note?: String}`;
     appends to store; returns the persisted
     entry. Capability: `budget.write`.
   - `budget.summary`: input `{period?:
     "today"|"this_week"|"this_month"|
     "this_year"|"all_time", since?: RFC3339,
     until?: RFC3339}`. Period maps to a
     since/until window using `now` as the
     anchor. Explicit since/until override
     period. Returns
     `{period, since, until, total, entry_count,
     by_category: [{category, total, count}]}`
     sorted descending by total. Capability:
     `budget.read`.

5. **Main.rs wire + INSTALL + exit + Frozen.**
   `main.rs` registers `BudgetRecord` +
   `BudgetSummary` in the toolkit harness.
   INSTALL.md toolkit section gains a Phase
   143 row for the two tools with example
   prompts. Phase 143 exit doc with
   prediction-vs-reality.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment. Streak: 33 → **34**.
- **PRODUCT.md** — **Will hold.** Operator-
  facing capability expansion reinforces
  the personal-assistant framing. Streak:
  33 → **34**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 143 work in `aivyx-capability`
  (new bases) + `aivyx-toolkit` (substrate
  + tools). Core untouched. Streak:
  8 → **9**.

## Exit criteria

- [ ] `docs/PHASE_143.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `budget.read` + `budget.write` public
  in `aivyx-capability` + Trusted-only
  defaults — Task 2.
- [ ] `BudgetStore` substrate + persistence
  with unit tests — Task 3.
- [ ] `BudgetRecord` + `BudgetSummary` tools
  exist + registered + tested — Task 4.
- [ ] Toolkit binary registers the new
  tools — Task 5.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+10` to `+18`
  (capability bases ~2; budget_store
  ~5-7 for record/summary/persistence
  round-trip; tool input parsing + execute
  paths ~3-5 each).

## Honest scope risks at sign-off

- **No edit/delete tools.** Phase 143 ships
  only `record` + `summary`. If the operator
  records the wrong amount, no `budget.update`
  or `budget.delete` exists. Phase 144+
  candidate if mistakes are routine.

- **Categories are free-text.** No
  category whitelist; operators can record
  "fod" / "food" / "Food" and get three
  entries. Phase 144+ candidate: case-fold
  + suggest-existing-category. Acceptable
  for MVP — agent can paraphrase prompts to
  avoid the worst cases ("did you mean
  food?").

- **No currency.** Amounts are unitless
  f64. Operator's mental model is "this is
  in my local currency"; Phase 144+ could
  add a currency field per entry if
  multi-currency comes up.

- **f64 precision for money.** Standard
  finance practice prefers decimal types
  (rust_decimal crate). Phase 143 uses f64
  for simplicity and zero-new-deps. For the
  expected scale (personal-budget
  aggregation, hundreds of entries) the
  rounding error is invisible. Phase 144+
  could switch to a decimal type if
  operators care.

- **Period definitions are calendar-based,
  not rolling.** "this_week" = current
  ISO week (Mon-Sun); not "last 7 days".
  Operators wanting rolling windows pass
  explicit `since` / `until`. Documented
  in the tool description.

- **No "by category over time" trend
  data.** Summary is a single snapshot.
  Phase 144+ could ship a `budget.trend`
  tool if the operator wants
  month-over-month deltas.

- **Thirty-second consecutive deferral of
  the Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 143

After Phase 143, the agent can record + summarize
spending. Phase 144+ candidates:

1. **`budget.update` + `budget.delete`** —
   fix-typo tools. Small.
2. **Category whitelist + suggest-existing.**
3. **Currency field per entry** if multi-
   currency surfaces.
4. **`budget.trend`** — month-over-month
   deltas.
5. **Chapter G health.check.remove + alert
   dispatch** — Phase 125 final candidate.
6. **Proactive reminder dispatch** — the big
   architectural step.
7. **Phase 142 debt cleanup** — parallel
   fan-out, dedup, capability mapping.
8. **Voice continuation** — mid-synthesis
   abort, Silero VAD, streaming ASR,
   wake-word, multimodal output, macOS
   variant, lock-free detector.
9. **Drive tool expansion** (recent-files,
   etc.).
10. **Relative-time localization.**
11. **whisper-cpp-plus rehabilitation.**
12. **`build_agent_stack` substrate-tier
    promotion** if more channel adapters
    ship.
13. **Channel Activation Milestone** — still
    held intentionally; 32nd consecutive
    deferral at Phase 143 open.

## Prediction vs reality

**Three-of-three streak HOLDs as predicted.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment. Streak:
  33 → **34**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Streak: 33 → **34**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 143 work in
  `aivyx-capability` (KNOWN_BASES + ceiling
  defaults) + `aivyx-toolkit` (store + tools +
  binary). Core untouched. Continuing
  post-Phase-135 reset: 8 → **9**.

**Test count delta: +22 — over predicted `+10`
to `+18` range.** Workspace lib tests 3087 →
3109. Per-module:
- `aivyx-capability`: +1 (budget bases parse
  via Scope::parse; plus the count-pin test
  bumped 67 → 69).
- `budget_store`: +9 (aggregate × 3 +
  record/summary round-trip + persist-across-
  reopen + non-finite-amount + blank-category +
  out-of-window + schema-too-new).
- `tools::budget`: +12 (record parse × 4 +
  summary period × 6 + override + reject ×
  2).

Overshot the predicted range honestly: each
period (today/this_week/this_month/this_year)
got its own deterministic-now test plus a
December-rollover edge case + an unknown-period
reject + an inverted-window reject. The
substrate exhaustive-testing posture from
Phase 141 carried over.

**Zero new workspace dependencies** as
predicted. chrono + serde + uuid already in
toolkit; budget_store's tests use the existing
`scratch_dir()` pattern from task_store
(PID + atomic counter under TMPDIR) — no
tempfile crate added.

**Zero clippy warnings** with default features.

### What landed cleanly + what bent

**Cleanly:**
- `budget.read` + `budget.write` capability
  bases registered + Trusted-only defaults +
  A3 addendum updated (67 → 69 bases).
- `BudgetStore` substrate: open / record /
  summary. 0600 perms, atomic write-then-rename,
  schema_version: 1 forward-compat.
- Pure `aggregate()` helper enabling unit-test
  coverage of the by-category sort + tie-break +
  empty-input edge cases without touching the
  store.
- `BudgetRecord` + `BudgetSummaryTool` tools
  registered in the toolkit harness. Capability
  scopes wired through `required_scope`.
- Period semantics: today / this_week (ISO) /
  this_month (calendar) / this_year (calendar)
  / all_time, with deterministic-now testing.
- Explicit since/until override period bounds
  for rolling windows.
- `main.rs` opens the budget store + registers
  the two tools alongside the existing eight.
  10-tool toolkit harness (was 8).
- INSTALL.md toolkit section gains a
  Phase 143 row + per-tool docs + example
  operator prompts.
- 3109 workspace lib tests pass; clippy clean.

**Bent honestly:**

1. **No edit / delete tools.** Phase 143 ships
   record + summary only. Mistakes are
   operator-recoverable by editing the JSON
   directly. Phase 144+ for `budget.update` /
   `budget.delete` if mistakes are routine.

2. **Categories free-text.** No whitelist, no
   case-fold, no fuzzy-match. "food" /
   "Food" / "fod" are three categories.
   Acceptable for MVP; agent can paraphrase
   operator input to bias toward
   already-used categories.

3. **f64 for money.** Standard finance
   practice prefers decimal types. For
   personal-budget scale the precision
   loss is invisible; rust_decimal switch
   deferred.

4. **No currency.** Operator's mental
   model is local currency; multi-currency
   support is Phase 144+ if it surfaces.

5. **Period definitions are calendar-based,
   not rolling.** "this_week" = ISO week, not
   "last 7 days". Rolling windows require
   explicit since/until. Documented in the
   tool description.

6. **save_to_disk + create_dir_all_secure +
   write_secure duplicated from task_store.**
   Two copies now; extraction to a shared
   `aivyx-toolkit::secure_io` module is a
   Phase 144+ candidate when a third store
   would surface drift risk. Currently the
   duplication is intentional — substrate
   stability over premature abstraction.

7. **Test count overshot prediction.** +22 vs
   predicted +10 to +18. Same honest
   substrate-exhaustive posture as Phase 141.
   Each period gets its own deterministic-now
   test + a December-rollover edge case.

### Direction after Phase 143

After Phase 143, the agent records + summarizes
spending. Phase 144+ candidates:

1. **`budget.update` + `budget.delete`** —
   fix-typo tools. Small.
2. **Category whitelist + case-fold +
   suggest-existing-category.**
3. **Currency field per entry.**
4. **`budget.trend`** — month-over-month
   deltas.
5. **rust_decimal switch** for amounts.
6. **Extract `secure_io` shared module** when a
   third store surfaces.
7. **Chapter G health.check.remove + alert
   dispatch** — Phase 125 final candidate.
8. **Proactive reminder dispatch** — the big
   architectural step.
9. **Phase 142 debt cleanup** — parallel
   fan-out, dedup, capability mapping.
10. **Voice continuation** — mid-synthesis
    abort, Silero VAD, streaming ASR,
    wake-word, multimodal output, macOS
    variant, lock-free detector.
11. **Drive tool expansion.**
12. **Relative-time localization.**
13. **whisper-cpp-plus rehabilitation.**
14. **`build_agent_stack` substrate-tier
    promotion** if more channel adapters
    ship.
15. **Channel Activation Milestone** — still
    held intentionally; 32nd consecutive
    deferral at Phase 143 exit.

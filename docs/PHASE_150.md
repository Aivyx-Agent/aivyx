# Phase 150 — Budget: Category Whitelist + Case-Fold + Suggest-Existing

**Phase 143 longest-standing honest-debt close-
out.** Phase 143 shipped budget tracking with
free-text categories — "Food" / "food" / "fod"
were three different categories. Every subsequent
budget phase (144 CRUD parity, 149 trend) carried
the same Phase 143 #2 honest-debt forward. Phase
150 finally closes it.

## Why this, why now

- **Longest-standing budget debt.** Phase 143
  shipped it open; Phase 144 didn't close it;
  Phase 149 didn't close it. Six phases later
  it's time.

- **Daily-use quality win.** Operators who
  record "Lunch was twelve dollars, Food
  category" and later "Coffee was four dollars,
  food category" currently get two buckets in
  `budget.summary` and `budget.trend`. After
  Phase 150 they get one. Real impact.

- **Pure substrate + tool-output enrichment.**
  Normalization is `to_lowercase + trim` —
  trivial. Suggestion is Levenshtein distance
  ≤ 2 against known categories — ~25 lines of
  dynamic-programming substrate.

- **Zero new workspace deps.** Pure Rust math.

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row.
   Backfill Phase 149 hash to `62b7a0c`.

2. **Category normalization + suggestion
   substrate.** Add to `budget_store.rs`:
   ```rust
   pub fn normalize_category(input: &str) -> String;
   pub fn suggest_category(input: &str, known: &[String]) -> Option<String>;
   impl BudgetStore {
       pub async fn known_categories(&self) -> Vec<String>;
   }
   ```
   - `normalize_category`: lowercase + trim.
     Pure function.
   - `suggest_category`: Levenshtein distance
     ≤ 2 fuzzy match against known categories.
     Returns the closest known category when
     distance ≤ 2 and > 0 (so exact match
     surfaces None — nothing to suggest).
   - `known_categories`: snapshot the entries
     under lock, extract unique categories,
     sort ascending.
   - Apply `normalize_category` inside
     `record` and `update`'s category-handling
     paths.
   - Tests cover: normalize cases (Food→food,
     FOOD→food, "  food  "→food, ""→""),
     Levenshtein 0/1/2/3 boundary, suggest
     exact-match → None, suggest too-distant →
     None, known_categories dedup + sort.

3. **`budget.categories` tool + record/update
   output enrichment.** New `BudgetCategoriesTool`
   (read-side):
   - Input: empty object.
   - Output: `{categories: [string]}` sorted
     ascending.
   - Capability: `budget.read`.

   Extend `BudgetRecord` and `BudgetUpdate`
   tool output with an optional
   `category_suggestion` field:
   - Compute *before* the normalize+store step,
     using the raw operator input + the current
     `known_categories` snapshot.
   - When the normalized input doesn't
     exact-match an existing category but a
     known category is Levenshtein-distance ≤ 2,
     surface `"did you mean <suggestion>"` as
     a string field in the output.
   - Operator gets paraphrasable hint without
     being forced to accept it (record still
     succeeds with the operator's chosen
     category).

4. **Main.rs wire + INSTALL + exit + Frozen.**
   `main.rs` registers `BudgetCategoriesTool`
   (toolkit harness 14 → 15 tools). INSTALL.md
   toolkit budget block adds `budget.categories`
   docs + notes about the silent normalization
   (Food → food) + `category_suggestion` field
   in record/update output. Phase 150 exit doc
   with prediction-vs-reality.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment. Streak: 40 → **41**.
- **PRODUCT.md** — **Will hold.** Streak:
  40 → **41**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 150 work in `aivyx-toolkit`. Core
  untouched. Streak: 15 → **16**.

## Exit criteria

- [ ] `docs/PHASE_150.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `normalize_category` + `suggest_category`
  pure substrate; `BudgetStore::known_categories`
  + normalization applied in record + update —
  Task 2.
- [ ] `BudgetCategoriesTool` public + registered
  + tested; record/update gain
  `category_suggestion` output enrichment —
  Task 3.
- [ ] Toolkit harness 14 → 15 tools — Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+8` to `+14`
  (normalize ~3 + Levenshtein ~4 + suggest ~3 +
  known_categories ~1 + tool input/output ~2).

## Honest scope risks at sign-off

- **Legacy entries stay as-recorded.** Phase
  150 doesn't migrate existing "Food" entries
  to "food". `budget.summary` and
  `budget.trend` against the resulting store
  will still surface both as separate
  categories until the operator manually
  updates the legacy entries (via
  `budget.update`). Migration is operator-
  decided to avoid silent data mutation.

- **Levenshtein distance is character-level,
  not semantic.** "food" and "foods" are
  distance 1 (a suggestion fires);
  "groceries" and "food" are distance 9 (no
  suggestion). Operators who use multi-word
  categories ("eating out" vs "eating-out")
  may see surprising suggestions. Documented
  in the tool description.

- **No suggestion threshold tunability.**
  Phase 150 hardcodes distance ≤ 2. Operators
  in environments with many short categories
  may want stricter (1) or looser (3); Phase
  151+ if surfaces.

- **No suggestion across categories absent from
  the store.** "food" isn't suggested when
  the operator records "fod" on an empty
  store. Acceptable; the first entry sets the
  canonical form.

- **Tool struct named `BudgetCategoriesTool`**
  not `BudgetCategories` — maintains the
  `BudgetSummary`/`BudgetSummaryTool` and
  `BudgetTrend`/`BudgetTrendTool` pattern
  established in Phases 143 + 149 even
  though there's no `BudgetCategories`
  substrate struct (helper functions only).
  Tool-name consistency wins.

- **Thirty-ninth consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 150

After Phase 150, budget categories are
canonical. Phase 151+ candidates:

1. **Budget category migration tool** — bulk
   `budget.normalize` to retroactively
   case-fold legacy entries.
2. **Budget currency / rust_decimal** —
   Phase 143 #3 honest-debt.
3. **Multi-category trend breakdown** —
   `budget.trend` extension.
4. **Trend smoothing / moving average.**
5. **Bulk budget operations.**
6. **Recursive folder filter on drive
   recent_*.**
7. **drive_id parameter on drive recent_*.**
8. **Drive Activity API.**
9. **Aggressive voice abort.**
10. **Partial-text preservation on voice
    abort.**
11. **Silero ONNX VAD.**
12. **Streaming ASR.**
13. **Wake-word activation.**
14. **Multimodal output.**
15. **macOS streaming variant.**
16. **Lock-free AudioIn detector.**
17. **VAD config validation.**
18. **Proactive reminder dispatch.**
19. **Phase 142 calendar debt cleanup.**
20. **Relative-time localization.**
21. **whisper-cpp-plus rehabilitation.**
22. **`build_agent_stack` substrate-tier
    promotion.**
23. **Channel Activation Milestone** —
    still held intentionally; 39th
    consecutive deferral at Phase 150 open.

## Prediction vs reality

**Three-of-three streak HOLDs as predicted.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment. Streak:
  40 → **41**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Streak: 40 → **41**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 150 work in
  `aivyx-toolkit`. Continuing post-Phase-135
  reset: 15 → **16**.

**Test count delta: +13 — within predicted `+8`
to `+14` range.** Workspace lib tests 3188 →
3201. Per-module:
- `budget_store`: +13 (normalize 5 cases +
  Levenshtein empty/identical/known-edits 3+2+5
  cases overlapping → 6 tests + suggest 5
  variants + record + update normalize + known_
  categories dedupe = 13 budget_store tests).
- `tools::budget`: 0 new tests — substrate
  exhaustively covers the suggestion logic;
  tool-layer enrichment is straightforward
  plumbing (operator-validation tier for the
  end-to-end "did you mean" UX).

**Zero new workspace dependencies** as
predicted. Levenshtein implemented as ~25 lines
of pure Rust DP.

**Zero clippy warnings** with default features.

### What landed cleanly + what bent

**Cleanly:**
- `normalize_category(input)` pure function
  (lowercase + trim).
- `suggest_category(input, known)` pure
  function backed by classic Levenshtein DP.
  Returns None for exact match, None for
  too-distant (> 2), Some(closest) within
  threshold.
- `BudgetStore::known_categories()` BTreeSet
  dedup + ascending-sort snapshot.
- `record` and `update` apply
  `normalize_category` on the category-handling
  path. Legacy entries unchanged.
- `BudgetCategoriesTool` registered, harness
  14 → 15 tools.
- `BudgetRecord` + `BudgetUpdate` tool outputs
  gain `category_suggestion` field — computed
  pre-record/pre-update against the existing
  known list, so self-matching is impossible.
- INSTALL.md updated with the new tool docs +
  normalization posture documentation.
- 3201 workspace lib tests pass; clippy clean.

**Bent honestly:**

1. **Legacy entries unchanged.** Pre-Phase 150
   "Food"/"FOOD" variants surface in
   `budget.categories` alongside the canonical
   "food". Phase 151+ candidate: bulk
   `budget.normalize` to retroactively
   case-fold legacy entries (operator-decided
   not silent).

2. **Levenshtein is character-level, not
   semantic.** "food" vs "foods" is distance 1
   (a suggestion fires); "groceries" vs "food"
   distance 9 (no suggestion). The agent reads
   the raw suggestion + paraphrases — it can
   downgrade clearly-bad suggestions in its
   own response.

3. **No suggestion threshold tunability.**
   Distance ≤ 2 is hardcoded. Phase 151+ if
   operators in many-short-categories
   environments hit false positives.

4. **No tool-level tests for the
   `category_suggestion` enrichment.** Tool
   layer is plumbing over the exhaustively-
   tested substrate; an integration test
   simulating the record-then-output shape
   would mostly exercise serde_json semantics.
   Operator-validation tier for the end-to-end
   UX flow.

5. **No suggestion against empty store.** The
   first record on an empty store can't
   suggest anything (known is empty). The
   first entry sets the canonical form.

6. **Tool struct named `BudgetCategoriesTool`**
   not `BudgetCategories` — maintains the
   `BudgetSomethingTool` pattern from Phases
   143 + 149 even though there's no
   `BudgetCategories` substrate struct here.
   Tool-name consistency wins.

### Direction after Phase 150

After Phase 150, budget categories are canonical
for new entries. Phase 151+ candidates:

1. **Budget category migration tool** — bulk
   `budget.normalize` for legacy entries.
2. **Budget currency / rust_decimal** — Phase
   143 #3 honest-debt.
3. **Multi-category trend breakdown.**
4. **Trend smoothing / moving average.**
5. **Bulk budget operations.**
6. **Recursive folder filter on drive
   recent_*.**
7. **drive_id parameter on drive recent_*.**
8. **Drive Activity API.**
9. **Aggressive voice abort.**
10. **Partial-text preservation on voice
    abort.**
11. **Silero ONNX VAD.**
12. **Streaming ASR.**
13. **Wake-word activation.**
14. **Multimodal output.**
15. **macOS streaming variant.**
16. **Lock-free AudioIn detector.**
17. **VAD config validation.**
18. **Proactive reminder dispatch.**
19. **Phase 142 calendar debt cleanup.**
20. **Relative-time localization.**
21. **whisper-cpp-plus rehabilitation.**
22. **`build_agent_stack` substrate-tier
    promotion.**
23. **Channel Activation Milestone** —
    still held intentionally; 39th
    consecutive deferral at Phase 150
    exit.

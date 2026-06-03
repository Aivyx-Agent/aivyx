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

_Populated at Phase 150 exit. Predictions at
sign-off: DESIGN.md HOLD → 41; PRODUCT.md HOLD
→ 41; lib.rs HOLD → 16; zero new deps; test
count delta `+8` to `+14`; zero clippy
warnings._

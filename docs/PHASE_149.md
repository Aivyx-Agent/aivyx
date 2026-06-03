# Phase 149 — Budget: `budget.trend` (Month-Over-Month Deltas)

**Phase 143 #4 honest-debt close-out.** Phase 143
shipped `budget.record` + `budget.summary`;
Phase 144 added `budget.update` + `budget.delete`
for CRUD parity. Both phases called out
`budget.trend` as a follow-on candidate. Phase
149 ships it.

Pivot from drive/calendar after four consecutive
debt-closure phases (146-148). Phase 149 adds a
genuinely new operator-facing capability: the
agent can answer "is my food spending up this
quarter," "what's my biggest trending category,"
"how does this month compare to last."

## Why this, why now

- **Variety after 4 debt-closure phases.**
  Phases 146-148 closed three honest-debt
  bundles (voice mid-synthesis abort, Chapter G
  list, drive list_drives + folder filter).
  Phase 149 is net-new operator capability, not
  debt-closure.

- **Daily-use utility.** "Is my coffee budget
  out of control" / "did I spend less this
  month" are the kinds of questions an
  always-available assistant should answer
  naturally. `budget.summary` covers
  point-in-time aggregation; `budget.trend`
  surfaces the change-over-time story.

- **Pure substrate on existing data.** Phase
  143's BudgetStore already keeps every entry
  with a recorded_at timestamp. Phase 149
  bucketizes them into calendar months (same
  posture as `budget.summary`'s `this_month`)
  and computes deltas. No new data, no new
  storage shape — just a new aggregation.

- **Zero new workspace deps.**

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row.
   Backfill Phase 148 hash to `36b6158`.

2. **`BudgetStore::trend` substrate method.**
   ```rust
   pub async fn trend(
       &self,
       now: DateTime<Utc>,
       months_back: u32,
       category: Option<&str>,
   ) -> BudgetTrend;

   pub struct BudgetTrend {
       pub months: Vec<MonthBucket>,
       pub category: Option<String>,
   }

   pub struct MonthBucket {
       pub month: String,                 // "YYYY-MM"
       pub total: f64,
       pub entry_count: usize,
       pub delta_vs_prior: Option<f64>,   // None for first month
       pub pct_change_vs_prior: Option<f64>, // None for first or zero-prior
   }
   ```
   `now` parameterized so tests pin
   deterministic month boundaries. Calendar
   months (not rolling 30-day). Optional
   category filter applies before bucketing.
   Tests cover: empty store, multi-month
   aggregation, single-month case, category
   filter, delta math, year-boundary handling.

3. **`budget.trend` tool.** New `BudgetTrend`
   in `tools/budget.rs`:
   - Input: `{months_back? default 6 (capped at
     36), category? optional}`.
   - Output: `{months: [...], category,
     months_back}`.
   - Capability: `budget.read`.
   - Tests cover input parsing variants +
     clamp.

4. **Main.rs wire + INSTALL + exit + Frozen.**
   `main.rs` registers `BudgetTrend` (toolkit
   harness 13 → 14 tools). INSTALL.md toolkit
   budget block adds `budget.trend` docs +
   example operator prompt. Phase 149 exit doc
   with prediction-vs-reality.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment. Streak: 39 → **40**.
- **PRODUCT.md** — **Will hold.** Streak:
  39 → **40**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 149 work in `aivyx-toolkit`. Core
  untouched. Streak: 14 → **15**.

## Exit criteria

- [ ] `docs/PHASE_149.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `BudgetStore::trend` + `BudgetTrend` +
  `MonthBucket` public + tested — Task 2.
- [ ] `BudgetTrend` tool public + registered +
  tested — Task 3.
- [ ] Toolkit harness 13 → 14 tools — Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+8` to `+14`
  (substrate aggregation ~5-7 tests + tool
  input parsing ~3-5 tests).

## Honest scope risks at sign-off

- **Calendar months, not rolling windows.**
  "Trend over the last 6 months" means six
  calendar months ending with the current
  month-in-progress. Operators asking "trend
  over the last 180 days" still go via
  `budget.summary` with explicit `since` /
  `until`. Same posture as Phase 143's period
  semantics.

- **No moving-average or smoothing.** Phase
  149 ships raw monthly totals + per-month
  deltas. Operators wanting a "is the trend
  up overall" smoother answer paraphrase
  themselves. Phase 150+ candidate.

- **No multi-category breakdown.** Optional
  category filter is one-at-a-time. Operators
  wanting "trend across food + transport
  side-by-side" call twice. Phase 150+
  candidate.

- **Percentage change for zero-prior is null.**
  When the prior month had no entries (or no
  entries matching the category filter), the
  pct_change_vs_prior field is null rather
  than infinity or +100%. Clean for the agent;
  no special-case math needed.

- **First month's delta + pct are null.**
  There's no prior month to compare against;
  the agent reads null and skips the change
  narrative for that bucket.

- **f64 precision matches the rest of
  budget.** Phase 143's honest-debt about
  f64-vs-decimal stays — Phase 149 doesn't
  shift the storage type. Acceptable at
  personal-budget scale.

- **months_back cap at 36.** Three years of
  monthly buckets. Operators wanting longer
  trend windows hit the cap; raise it if
  someone hits it.

- **Thirty-eighth consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 149

After Phase 149, budget has 5 tools (record,
summary, update, delete, trend) — full
operator-facing surface for personal-finance
tracking. Phase 150+ candidates:

1. **Category whitelist + case-fold for
   budget** — Phase 143 #2 honest-debt.
2. **Budget currency / rust_decimal.**
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
    still held intentionally; 38th
    consecutive deferral at Phase 149 open.

## Prediction vs reality

_Populated at Phase 149 exit. Predictions at
sign-off: DESIGN.md HOLD → 40; PRODUCT.md HOLD
→ 40; lib.rs HOLD → 15; zero new deps; test
count delta `+8` to `+14`; zero clippy
warnings._

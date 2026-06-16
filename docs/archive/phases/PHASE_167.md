# Phase 167 — drive.recent_activity Follow-Ons (Action Filter + Consolidation Knob + Parent Folder)

**Phase 159 close-out, deferred 8 phases.**
Phase 159 shipped `drive.recent_activity` with
three documented honest-debts. The user
declined this surface six times before picking
it for Phase 167. The wait wasn't strategic —
just operator preference — but the bundle still
fits the established close-out cadence.

## Why this, why now

- **Phase 159 was last touched 8 phases ago.**
  Longest gap in the project's close-out
  ledger; matches the cumulative deferral
  count.

- **All three honest-debts are operator-facing
  inputs.** No DESIGN.md amendment. The
  Activity API's filter DSL already supports
  action-type and ancestor-scope filtering;
  Phase 167 just surfaces the knobs.

- **Zero new workspace deps.** Pure string-
  composition against the existing
  `post_json_activity` substrate.

## Tasks

1. **Open doc + ROADMAP + README.** This doc +
   roadmap section + README row. Backfill
   Phase 166 hash to `108bc1c`.

2. **`action_type_filter` input.** New input
   field accepting an array of action types
   from the Activity API's enumeration:
   `["edit", "create", "rename", "delete",
   "move", "comment", "permissionChange",
   "restore", "reference", "settingsChange"]`.
   Builds the `detail.action_detail_case:CASE1
   detail.action_detail_case:CASE2 ...` filter
   clause. Composes with the existing time
   filter via space-separated AND. Validation:
   normalize to upper-snake-case for the API
   (`edit` → `EDIT`), reject unknown types
   with a clear "supported: ..." error.

3. **`consolidation` strategy knob.** New
   input field accepting one of `"legacy"`
   (Phase 159 default) or `"none"`. Builds
   the `consolidationStrategy` request body
   field accordingly:
   - `legacy` → `{"legacy": {}}`
   - `none` → `{"none": {}}`
   The `consolidated` strategy isn't supported
   by the public API (it's an internal Drive
   field); honest scope risk documented.

4. **`parent_folder_id` composition.** New
   input field that maps to the Activity API's
   `ancestorName: "items/<folder_id>"` request
   body field. Scopes activities to those
   occurring on items within the parent folder
   subtree. Composes cleanly with
   `action_type_filter` and the time window.
   Validation: reject quoted IDs (parent_folder_id
   must not contain single quotes — same posture
   as recent_files / recent_changes).

5. **INSTALL + exit + Frozen.** INSTALL.md
   `drive.recent_activity` row picks up three
   new inputs; exit doc with prediction-vs-
   reality + the honest "actor filter deferred"
   note. README + ROADMAP Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Pure input
  additions; no contract change. Streak:
  3 → **4**.
- **PRODUCT.md** — **Will hold.** Streak:
  57 → **58**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 167 work in `aivyx-drive`. Streak:
  3 → **4**.

## Exit criteria

- [ ] `docs/PHASE_167.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `action_type_filter` honored, normalized,
  validated — Task 2.
- [ ] `consolidation` knob honored on the
  request body — Task 3.
- [ ] `parent_folder_id` mapped to
  `ancestorName` — Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+15` to `+25`.

## Honest scope risks at sign-off

- **No native actor filter in the Activity
  API DSL.** Phase 159's exit doc named
  "actor / target filters" as a single
  carry-over. Phase 167 closes the
  action-type and target-scope (via
  ancestorName) pieces. Actor-specific
  filtering (e.g. "only activities by
  user@example.com") would need post-fetch
  shaping against `actor_email` on the
  enriched output. Phase 168+ candidate if
  surfaces.

- **`consolidated` strategy intentionally
  omitted.** Drive's internal `consolidated`
  consolidation isn't exposed in the public
  Activity API v2; including it would be a
  silent footgun. Phase 167 ships `legacy`
  and `none` only.

- **`action_type_filter` validation against
  a hand-maintained enumeration.** New
  Activity API action types added after
  Phase 167 fail-closed with "unsupported
  action type" until the list is widened. The
  fix is one-line additive.

- **`parent_folder_id` semantics differ
  subtly from `recent_files`'s.** Activity
  API's `ancestorName` is recursive (all
  descendants); `recent_files`'s
  `parent_folder_id` defaulted to direct
  children with a separate `recursive: true`
  toggle. Phase 167 inherits the API's
  recursive-by-default behavior. Documented.

- **Combined filters can be empty-set.**
  Operator passing `action_type_filter:
  ["delete"]` + `parent_folder_id: "X"` +
  a 1-hour window might get zero results
  even though more recent activity exists
  outside the filter. Empty is empty;
  not an error.

- **Fifty-sixth consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 167

After Phase 167, Phase 159's filtering carry-
overs clear (modulo actor filter, deferred
with documented honest scope). Phase 168+
candidates:

1. **Actor email post-fetch filter** for
   `drive.recent_activity`. (Phase 167
   carry-over.)
2. **Compressed-stream-aware PDF page count.**
   (Phase 165 carry-over.)
3. **Streaming document blocks.**
4. **503 / 429 retry classification on URL
   fetch.** (Phase 166 carry-over.)
5. **Backoff jitter on URL fetch retry.**
   (Phase 166 carry-over.)
6. **Read-stalled-bytes timeout (slow-trickle
   defense).** (Phase 161 carry-over.)
7. **Mid-recording or mid-reply /image
   command.**
8. **Clipboard-based image source.**
9. **Voice abort UX knob.**
10. **Silero ONNX VAD.**
11. **Streaming ASR.**
12. **Wake-word activation.**
13. **macOS streaming variant.**
14. **Lock-free AudioIn detector.**
15. **calendarList cache TTL knob.**
16. **access_role deprecation.**
17. **Budget category migration tool.**
18. **Budget currency / rust_decimal.**
19. **Multi-category trend breakdown.**
20. **Trend smoothing / moving average.**
21. **Bulk budget operations.**
22. **Proactive reminder dispatch.**
23. **Relative-time localization.**
24. **whisper-cpp-plus rehabilitation.**
25. **`build_agent_stack` substrate-tier
    promotion.**
26. **Secret-store integration** for
    url_headers.
27. **Channel Activation Milestone** —
    still held intentionally; 56th
    consecutive deferral at Phase 167
    open.

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 4 | Untouched | ✅ |
| PRODUCT.md HOLD → 58 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 4 | Untouched | ✅ |
| Zero new workspace deps | All work uses existing primitives (serde_json, hand-maintained enum tables) | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean | ✅ |
| Test count delta `+15` to `+25` | `+29` (action_type +13, consolidation +9, parent_folder_id +7) | ⚠️ (over by 4; see correction) |

All five exit criteria functionally met. Three
of Phase 159's named honest-debts closed; the
fourth (actor filter) deferred to Phase 168+
with documented honest scope.

### Honest correction — test count over-band

Open band was `+15..+25`; actual is `+29`. The
over-shoot is consistent with Phase 165's
similar pattern: each pure-substrate input
warrants more parser-edge-case coverage than
the open's conservative estimate budgeted.
Per task:

- Task 2 (action_type_filter): 13 tests cover
  6 input-normalization shapes (absent / null
  / empty / case-fold / camel-snake / dedupe),
  3 input rejection shapes (unknown / non-
  string / non-array), 3 compose_filter
  cases, and one regression pin on the
  hand-maintained enumeration.
- Task 3 (consolidation): 9 tests cover 5
  parsing shapes + 3 rejection shapes
  (including the explicit `consolidated`
  rejection with explanation) + the
  as_api_key pin.
- Task 4 (parent_folder_id): 7 tests cover
  the input-validation matrix (absent / null
  / empty / extract / trim / single-quote-
  reject / non-string-reject).

Each piece's surface honestly warranted its
test count; the `+15..+25` band was a
conservative estimate that didn't account
for the parser-edge-case multiplier.

### Phase 159 honest-debt status — three of four cleared

Phase 159's exit doc named three carry-overs.
Phase 167 closes:

1. ✅ Action type filter (Task 2, commit
   `fb69430`).
2. ✅ Consolidation strategy knob (Task 3,
   commit `ced68cb`).
3. ✅ parent_folder_id composition (Task 4,
   commit `2ff62ad`).

Actor filter — named in Phase 159's
"actor / target filters" carry-over as a
single item — splits in Phase 167's
honest framing:

- ✅ Target (via ancestorName) is the
  parent_folder_id input.
- ⏳ Actor-specific filtering (e.g. "only
  activities by user@example.com") deferred
  to Phase 168+. The Activity API DSL has no
  native actor predicate; honest path is
  post-fetch shaping against the enriched
  output's `actor_email` field.

### Fifty-sixth deferral of Channel Activation Milestone

Per operator framing — intentional hold.
Recorded for the record.

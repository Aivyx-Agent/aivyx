# Phase 126 — Textual Tool-Call Extraction

**Not "Local-LLM Rehab #5".** Phase 124's secondary finding
motivated this phase, but the substrate isn't local-LLM-
specific: any model emitting tool-call JSON in response text
gets rescued. The framing reflects that — Phase 126 is a
generic planner-substrate improvement that happens to land
the load-bearing rescue for qwen3.

## Motivation: Phase 124 secondary finding

The Phase 124 live verification surfaced that qwen3.6:27b
emits structurally correct tool-call JSON, but in the wrong
channel — response TEXT instead of the Ollama protocol's
`tool_calls` array:

```text
qwen3.6:    <tool_code>
              {"name": "fs.write", "arguments": {
                "path": "test.txt",
                "content": "phase 124 verification"
              }}
            </tool_code>

gemma4:31b: <tool_call>
              {"tool": "fs.write_file", "parameters": {...}}
            </tool_call>
```

Two observed wrapper tags. Two JSON shapes. qwen3 emits a
REAL tool name with correct arguments — extraction +
dispatch should land a clean rescue. gemma4 emits a
hallucinated alternative (`fs.write_file`) that Phase 120's
fuzzy-recovery substrate could catch at a lowered threshold
(2/3 Jaccard ≈ 0.667; default 0.80).

**The Phase 124 exit framing:** "Phase 125 candidate:
textual-tool-call extraction. A planner-side parser that
detects `<tool_code>` / `<tool_call>` blocks in response
text, extracts the JSON, validates against the tool
registry, and dispatches as a real tool call. This is the
only substrate option Phase 124 surfaces that isn't either
'give up and use cloud' or 'more prompt engineering.'"

## Why this, why now

- **Operator pressure** chose this over Chapter G #2 / SDK
  harness lift / Channel Activation / release prep — the
  remaining substrate option with a concrete rescue
  trajectory.
- **Phase 124 four-substrate-phase ceiling stands**, BUT
  the qwen3 textual emission was the most informative
  signal of those four phases. Rescuing it converts a
  "model got it almost-right but in the wrong channel"
  into "model got it right." That's a real win
  conditional on the substrate working.
- **Phase 120 composition.** The extraction substrate +
  Phase 120 fuzzy-recovery substrate compose: an extracted
  call with a hallucinated tool name passes through fuzzy-
  recovery on the way to dispatch. gemma4's
  `fs.write_file` becomes `fs.write` at threshold ≤ 0.67.
- **Operator-facing pressure was specific:** "we really
  need to NAIL all these tools." Phase 125 expanded the
  tool surface; Phase 126 attempts to make MORE OF THAT
  SURFACE actually invocable from local models.

## Scope (Q-block sign-off — all four Recommended)

- **Q1a — Planner-side extraction** (Recommended). In
  `aivyx-core/src/llm_planner.rs`. Provider-agnostic;
  reuses Phase 120 fuzzy-recovery substrate; substrate
  works for any LLM provider that ever emits text-form
  tool calls.

- **Q2a — Both `<tool_code>` AND `<tool_call>` wrappers,
  both JSON shapes** (Recommended). Covers the Phase 124
  empirically-observed patterns. Permissive shape parser
  accepts `name`/`arguments` (qwen3) and `tool`/`parameters`
  (gemma4) interchangeably.

- **Q3a — New `extracted_from_text: Option<String>` field
  on `AuditTag::ToolCall`** (Recommended). Forensic
  visibility for operators debugging "the model said it
  called fs.write but the file isn't there." Same pattern
  as Phase 120's `auto_corrected_from`. **Breaks aivyx-
  core/src/lib.rs streak (currently 6).** Honest framing:
  Q3a chose forensic visibility over streak preservation
  because the alternative makes audit-chain queries
  misleading.

- **Q4a — Live verification against qwen3 + gemma4 at
  exit** (Recommended). This phase's whole point is
  empirical rescue; honest reporting required regardless
  of outcome.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract amendment.
  Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to seventeen** (was 16
  after Phase 125).

- **PRODUCT.md** — **Will hold.** No contract change. G6
  + the planner-substrate envelope cover this case. Hash:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to seventeen**.

- **`aivyx-core/src/lib.rs`** — **Will break.** Q3a adds
  `extracted_from_text: Option<String>` to
  `AuditTag::ToolCall`. Hash:
  `b420405bf9a5576ecb10f6ea04a965f7ad3a1a92ae9bd8f4abf46e22ef4d3c16`.
  Prediction: streak **resets to 0**. Phase 6 Q5 honesty
  applied at sign-off — the alternative (Q3b synthesize-
  as-normal) was rejected because it makes the audit
  chain misleading.

- **New workspace deps** — Zero anticipated.

- **Test count** — Pattern parser + planner integration +
  audit field. Prediction: **`+40` to `+60`** (parser is
  the bulk; planner glue is small; audit field touches
  many fixture sites but each is mechanical).

## Tasks

Five sub-tasks plus exit + backfill:

### Task 1 — Open (this commit)

`docs/PHASE_126.md` + `docs/ROADMAP.md` Phase 126 entry +
`docs/README.md` status row. Documents the lib.rs streak
break prediction up-front + the Q-block resolutions + the
empirical rescue conditions for both qwen3 (direct) and
gemma4 (paired with lowered fuzzy-recovery threshold).

### Task 2 — Textual extractor module (pure parser)

New `aivyx-core/src/textual_tool_call.rs` module:

```rust
pub struct ExtractedToolCall {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    /// Which wrapper-tag the extraction came from
    /// (`"tool_code"` or `"tool_call"`). Used by the
    /// planner to populate `AuditTag::ToolCall.extracted_from_text`.
    pub wrapper_tag: String,
}

pub fn extract_tool_calls(text: &str) -> Vec<ExtractedToolCall>;
```

Implementation:
- Scan for `<tool_code>...</tool_code>` and
  `<tool_call>...</tool_call>` blocks (any number per
  text, in order).
- For each block, parse inner content as JSON.
- Permissive shape match: try `{"name": ..., "arguments":
  ...}` first; fall back to `{"tool": ..., "parameters":
  ...}`. Either shape produces an `ExtractedToolCall`.
- Malformed JSON / unrecognized shapes silently dropped —
  no extracted call rather than a failed one.

**Comprehensive test coverage** (target ~25 tests):
- Single `<tool_code>` block with `name`/`arguments`.
- Single `<tool_call>` block with `tool`/`parameters`.
- Single `<tool_call>` block with `name`/`arguments`
  (gemma4 happens to use `<tool_call>` but other models
  may use the `name`/`arguments` shape inside that
  wrapper).
- Multiple blocks in one response — extracted in order.
- Wrapper-tag mismatch (`<tool_code>...</tool_call>`) —
  dropped.
- Malformed JSON inside a block — dropped silently.
- No blocks at all — returns empty Vec.
- Prose text BEFORE / AFTER the block — preserved
  irrelevant; extraction succeeds.
- Nested blocks (`<tool_code><tool_call>...</tool_call></tool_code>`)
  — outer wins; inner is "content" of outer.
- Empty block (`<tool_code></tool_code>`) — dropped.

NO lib.rs touch in this task; the new module is purely
additive at the crate level.

### Task 3 — `AuditTag::ToolCall.extracted_from_text` (streak break)

The Phase 120 pattern:
- Add field to `AuditTag::ToolCall` in
  `aivyx-core/src/lib.rs`.
- Mirror field on `AuditEvent::ToolCall` in
  `aivyx-audit/src/lib.rs` with
  `#[serde(default, skip_serializing_if = "Option::is_none")]`
  for backward chain compat.
- Audit chain HMAC is byte-identical for the dominant
  case (`None`); the field only appears in the serialized
  form when populated.
- Update every fixture site that constructs
  `AuditTag::ToolCall` to add `extracted_from_text: None`
  for the default case. Python brace-walking script
  precedent from Phase 120 Task 4 covers the mechanical
  update; expected ~30-40 fixture sites.

This task is delicate but mechanical. Streak breaks at
this commit (predicted).

### Task 4 — Planner integration

Wire the Task 2 extractor into the llm_planner turn flow:

```text
LlmResponse received:
  if response.tool_calls.is_empty() and response.content.has_extractable_text():
    extracted = extract_tool_calls(&response.content)
    if !extracted.is_empty():
      // Synthesize tool_calls vector; carry wrapper_tag
      // through to the planner's per-call dispatch
      for each ExtractedToolCall:
        dispatch with extracted_from_text: Some(wrapper_tag)
        (Phase 120 fuzzy-recovery fires if tool name unknown)
      return
  
  // Fall through to existing normal handling
```

Tests:
- Extraction fires only when protocol tool_calls is
  empty AND content has extractable text.
- Extracted call with real tool name dispatches normally;
  audit records `extracted_from_text: Some("tool_code")`.
- Extracted call with hallucinated tool name + lowered
  fuzzy threshold → Phase 120 recovery fires; audit
  records BOTH `extracted_from_text` AND
  `auto_corrected_from`.
- Extracted call with hallucinated tool name + default
  fuzzy threshold (0.80) → Phase 120 doesn't recover;
  surfaces as unknown-tool error.
- Failed extraction (no matches) falls through to text-
  response handling — operator sees the model's prose
  unchanged.

### Task 5 — Live verification + INSTALL.md sweep + exit

**Verification cycle (per Q4a):**

- `./scripts/dev-run.sh --release --reset --model qwen3.6:27b`
  with default fuzzy threshold (0.80). Same probes as
  Phase 124. Expected: qwen3 emits `<tool_code>` JSON;
  extraction synthesizes the tool call; fs.write actually
  runs; audit chain shows `extracted_from_text:
  Some("tool_code")`.

- `./scripts/dev-run.sh --release --reset --model gemma4:31b`
  with default threshold (0.80). Expected: gemma4 emits
  `<tool_call>` with hallucinated `fs.write_file`;
  extraction synthesizes the call; fuzzy recovery
  REJECTS at 0.80; surfaces as unknown-tool error.

- Then with `[providers] tool_name_auto_correct_threshold
  = 0.60` set: gemma4 same probe. Expected: extraction +
  recovery compose; fs.write actually runs; audit chain
  shows BOTH `extracted_from_text` AND
  `auto_corrected_from: Some("fs.write_file")`.

**Three outcome cases per Q4a:**

1. **Both models invoke fs.write (after threshold tune
   for gemma4).** Substrate breakthrough; closes the
   Phase 124 secondary finding cleanly; the
   local-LLM-rehab axis has one final answer that works
   for both models.
2. **qwen3 invokes; gemma4 doesn't even with lowered
   threshold.** Substrate works as designed for qwen3;
   gemma4's response shape changed since Phase 124 OR
   the hallucinated alternative is too far from `fs.write`
   in token similarity. Honest reporting; operators get
   the qwen3 rescue.
3. **Neither invokes** — the substrate is technically
   correct (parsing fires; dispatch fires) but the
   extracted call fails for some unforeseen reason
   (input schema validation? tool registry lookup?).
   Exit doc names the residual gap.

**Document outcomes regardless** per Q4a. INSTALL.md
gets the operator-facing recipe — how to enable
extraction (no flag; on by default at the planner
substrate level) and how to lower
`tool_name_auto_correct_threshold` for gemma4.

## Q-block resolutions (signed off pre-Task 2)

- **Q1a** — Planner-side extraction (Recommended).
- **Q2a** — Both Phase 124 observed shapes (Recommended).
- **Q3a** — New `extracted_from_text` field on
  `AuditTag::ToolCall` (Recommended; **streak break**).
- **Q4a** — Live verification against both models
  (Recommended).

**All four Recommended — third all-Recommended phase in a
row** (Phase 124, 125, 126).

## Exit criteria

- [ ] `docs/PHASE_126.md` + ROADMAP Phase 126 entry +
  `docs/README.md` status row — Task 1.
- [ ] Textual extractor module + comprehensive tests —
  Task 2.
- [ ] `extracted_from_text` field added + every fixture
  site updated + audit chain HMAC verification —
  Task 3.
- [ ] Planner integration + tests — Task 4.
- [ ] Live verification + INSTALL.md sweep + exit —
  Task 5.
- [ ] Q1 / Q2 / Q3 / Q4 resolved pre-Task 2 (all
  Recommended).
- [ ] DESIGN.md streak — predicted HOLD (streak → 17).
- [ ] PRODUCT.md streak — predicted HOLD (streak → 17).
- [ ] `aivyx-core/src/lib.rs` streak — **predicted
  BREAK** (resets 6 → 0). Honest framing at sign-off.
- [ ] Zero new workspace dependencies.
- [ ] Test count delta within `+40` to `+60`.
- [ ] Zero clippy warnings.
- [ ] **Live verification outcome** documented in exit
  doc regardless of result. Phase 126's load-bearing
  question: does the substrate produce empirical
  rescue?

## Honest scope risks at sign-off (carried forward)

- **The streak break is locked in by Q3a.** Phase 6 Q5
  honesty: the Q3b alternative (synthesize-as-normal)
  preserves the streak but makes the audit chain
  misleading. Q3a chose the harder-to-recover-from but
  more-honest path. Streak resets to 0; rebuild starts
  Phase 127+.

- **Extraction works but model intent may not.** Phase
  124's substrate-ceiling finding still holds: a model
  emitting `<tool_call>` text might just be "describing
  what it would do" rather than actually wanting to do
  it. Extraction substrate can't fix model intent; only
  rescue calls the model genuinely meant to dispatch.

- **gemma4 needs paired threshold tuning.** Phase 126
  rescues gemma4 only when the operator has also
  lowered `[providers] tool_name_auto_correct_threshold`
  to ~0.60. INSTALL.md documents this as
  operator-actionable; honest scope flag that the
  rescue isn't automatic for gemma4.

- **False-positive extraction risk.** An operator
  literally pasting `<tool_code>` JSON into a chat (as
  part of a conversation about tool-call syntax) would
  trigger spurious extraction. Mitigation: extraction
  only fires when the protocol tool_calls array is
  EMPTY (so legitimate tool calls happen normally; the
  text is just the model's reasoning prose).

- **The Phase 124 secondary finding may not reproduce.**
  Models are non-deterministic; qwen3's `<tool_code>`
  emission was observed in one session. If a future
  live test doesn't show the pattern, the substrate
  works but has nothing to extract. Exit doc reports
  honestly.

## Direction after Phase 126

After Phase 126, Phase 127 candidates:

1. **Multi-tool SDK harness lift** — still outstanding;
   now triple-relevant if Phase 127 ships any new tool
   process. Cheap (~55 LoC).
2. **Chapter G #2 — second tool bundle**.
3. **Chapter F #2 — Calendar/Drive/GitHub**.
4. **Channel Activation Milestone** — fourteenth
   consecutive deferral if skipped (Phase 126 is the
   14th). Audit's #1.
5. **Release prep (v0.1.0 + installer)**.
6. **Operator-pressure-driven new direction**.

Phase-by-phase decision at Phase 126 exit, sharpened by
the empirical rescue findings.

## Prediction vs reality

**Two of three streak predictions correct; one break as
predicted.**

- **DESIGN.md** — HELD as predicted (`c2be6d51…`
  unchanged). No contract amendment. Streak: 16 → 17.
- **PRODUCT.md** — HELD as predicted (`6e840cef…`
  unchanged). G6 + the planner-substrate envelope cover
  this case. Streak: 16 → 17.
- **`aivyx-core/src/lib.rs`** — **BROKE as predicted via
  Q3a** (`b420405b…` → `32d5730…` at Task 2 module
  declaration → `9692e5d…` at Task 3 audit field
  addition). Streak: 6 → 0. Honest framing held; the
  alternative (Q3b synthesize-as-normal) was rejected
  pre-Task-2 because it made the audit chain misleading.

**Test count `+35` undershot the predicted `+40 to +60`
range — honest report.** Per-task breakdown:
- Task 2 (extractor module): **+23** — 4 wrapper×shape
  combinations + 8 dropped/malformed cases + 4 multi-
  block + 3 edge + 2 prose-surrounded + 4 argument-shape
  preservation.
- Task 3 (audit field + plumbing): **+4** — wire-compat
  round-trip with field populated, None-skips-serialize,
  both-fields-compose, pre-Phase-126 wire decodes with
  None.
- Task 4 (planner integration): **+8** — known dispatch
  (both wrapper shapes), falls-through-when-no-blocks,
  composes-with-Phase-120, unknown-tool-loops-with-error,
  multiple-extracted-batch, malformed-drops-silently,
  history-preserves-raw-text.

The undershoot reflects the **Task 4 helper-refactor
benefit**: instead of duplicating the Phase 120/101 loop
between the protocol-channel ToolCalls arm and the new
extraction branch, the refactor introduced
`process_one_call` and reused it. The existing 55
llm_planner tests cover the helper's behavior via the
ToolCalls path; new tests focused only on what's distinct
about the extraction branch. Fewer tests added, same
coverage. Phase 6 Q5 honest report: the undershoot is a
"didn't need as many tests as anticipated" finding, not
a "skipped tests" finding.

**Q-block went through as picked.** All four Recommended
(Q1a planner-side + Q2a both wrappers/shapes + Q3a
audit field + Q4a live verification). No mid-task re-
asks. Q3a's streak-break trade-off was named at sign-off
and held at exit.

**Zero new workspace dependencies** as predicted.

**Helper-refactor surface (unplanned but honest).** Task
4 extracted `process_one_call` from the inline Phase
120/101 loop in the `ToolCalls` arm; the refactor was
necessary to share the dispatch logic between protocol
and extraction paths cleanly. The 55 existing
llm_planner tests passed unchanged post-refactor —
behavioral preservation confirmed. Not a Q-block item;
mentioned here for completeness.

### Live verification (Task 5)

> **PLACEHOLDER — populated post-live-test.**
>
> Verification against qwen3.6:27b and gemma4:31b
> through `./scripts/dev-run.sh --release --reset --model
> <name>`. Same probes as Phase 122/124. Banner should
> show no Phase-126-specific change (extraction is on
> by default at the planner-substrate layer; no
> `prompt_strategy` value covers it).
>
> The exit-doc backfill commit replaces this block with
> the empirical four-cell trajectory table:
>
> | Surface | qwen3 Phase 124 | qwen3 Phase 126 | gemma4 Phase 124 | gemma4 Phase 126 (default thresh) | gemma4 Phase 126 (thresh=0.60) |
> |---|---|---|---|---|---|
> | Enumeration | Empty | _observed_ | Vague prose | _observed_ | _observed_ |
> | Invocation | `<tool_code>` text, no real call | _observed_ | `<tool_call>` text w/ `fs.write_file`, no real call | _observed_ | _observed_ |
> | Audit ToolCalls | 0 | _observed_ | 0 | _observed_ | _observed_ |
> | `extracted_from_text` populated? | n/a (no extraction) | _observed_ | n/a | _observed_ | _observed_ |
> | `auto_corrected_from` populated? | n/a | _observed_ | n/a | _observed_ | _observed_ |
>
> **Three outcome cases pre-enumerated** per Q4a:
>
> 1. **qwen3 invokes fs.write; gemma4 invokes with
>    lowered threshold.** Substrate breakthrough. Phase
>    126 closes the local-LLM-rehab axis cleanly. Exit
>    doc validates the extraction + Phase 120
>    composition.
> 2. **qwen3 invokes; gemma4 doesn't even with lowered
>    threshold.** Honest asymmetric outcome. Exit doc
>    documents the qwen3 rescue and the gemma4 residual
>    gap; future investigation could lower threshold
>    further OR document gemma4 as unsupported for tool-
>    use workloads.
> 3. **Neither invokes.** The substrate is technically
>    correct (parsing + dispatch + audit chain all fire
>    in unit tests) but the extracted call fails on
>    some path the unit tests didn't cover. Exit doc
>    names the residual gap concretely; possible
>    Phase 127 follow-up.
>
> **Probe pattern (deterministic so the comparison
> against Phase 124 is honest):**
>
> 1. Banner observation.
> 2. `> What tools do you have available?` (enumeration
>    probe).
> 3. `> Please call fs.write to create a file at test.txt
>    with the content 'phase 126 verification'.`
>    (invocation probe).
> 4. Ctrl-D.
>
> Then for gemma4, a second pass with
> `[providers] tool_name_auto_correct_threshold = 0.60`
> in `aivyx.toml`. Three sessions total
> (qwen3, gemma4-default-thresh, gemma4-low-thresh) for
> the full empirical signal.

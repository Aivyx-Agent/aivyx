# Phase 127 — Multi-Format Tool-Call Extraction (substrate completion)

**Phase 126's substrate gap finished.** Phase 126 shipped
a textual extractor handling two emission formats — the
two Phase 124 happened to observe. Literature research at
Phase 126 close-out catalogued ~10 distinct text-form
tool-call formats across local LLM families. Phase 127
finishes the substrate by adding the four
load-bearing-missing parsers plus a hybrid family-hint
architecture so the substrate is genuinely model-agnostic,
not Phase-124-shaped.

## Motivation: end-users can pick any local model

Operator framing at Phase 126 close-out was load-bearing
correct:

> "Lets conduct some further deep research in Ollama as
> not every end user wants to use or connect to cloud based
> LLM and so having a Local LLM that can use and interact
> with all major local tools is critical."

> "We need to actually FIX the tooling issue first, not
> just run another test to verify what we already know
> as this is wasted time, tokens and money. We need to
> research tool calling for any potential Local LLM's as
> the End User could use any depending on their hardware."

The end-user's local hardware dictates the model. Llama 3.x
for fast/CPU-friendly setups, Qwen3-Coder for code-heavy
work, Mistral Nemo for the balanced midrange, Phi-4-mini
for edge devices, Gemma 3/4 for the Google-fine-tuned
path, DeepSeek R1 for reasoning. Aivyx's tool-call
substrate has to handle whichever the operator picked.

## The format catalog (Phase 126 close-out research)

| Family | Training format | Wrapper | Inner shape | Phase 126 | Phase 127 |
|---|---|---|---|---|---|
| Llama 3.1/3.2/3.3 | Python-call | `<\|python_tag\|>` | `[func(k=v)]` | ❌ | deferred (Python-call dispatch is a separate concern) |
| Mistral Nemo / Small 3.x | Special token | `[TOOL_CALLS]` | JSON array | ❌ | deferred (Mistral's native protocol path generally works server-side) |
| Qwen3 (Hermes pipeline) | XML | `<tool_call>` | JSON `{name, arguments}` | ✅ | ✅ |
| **Qwen3-Coder (qwen3.5/3.6)** | XML inner | `<tool_call>` | `<function=N><parameter=K>V</parameter></function>` | ❌ | **✅ Task 2** |
| DeepSeek R1 | Dynamic XML | `<TOOL_NAME>` (named after fn) | `<param>V</param>` | ❌ | deferred (dynamic-XML lookup is materially different; needs registry-driven match) |
| **Phi-4-mini** | Special tokens | `<\|tool_call\|>...<\|/tool_call\|>` | JSON list | ❌ | **✅ Task 3** |
| **Gemma 3** | Pythonic | ` ```tool_code ` markdown fence | Python call `func(k=v)` | ❌ | **✅ Task 4** |
| Gemma 4 | Special tokens | `<\|tool_call>` | `call:N{k:<\|"\|>v<\|"\|>}` | ❌ | deferred (special-token format; Gemma 4's native pipeline in Ollama 0.20.0-rc1+ should handle this) |
| **Bare JSON (qwen3:32b#11662)** | None | (no wrapper) | raw `{name, arguments}` | ❌ | **✅ Task 5** |
| Tool-code JSON (Phase 124 qwen3.6 obs) | Markdown-fence-like | `<tool_code>` | JSON | ✅ | ✅ |

**Phase 127 covers four of the six deferred patterns.**
The remaining two (Llama Python-call, DeepSeek dynamic XML,
Gemma 4 special-token, Mistral [TOOL_CALLS]) are honest
deferrals — they need either materially different parser
architectures (Python-call → JSON-args translator;
dynamic-XML registry lookup), OR they're already handled
server-side by Ollama's working pipelines for those models.
Phase 128+ can add the deferred ones if operator pressure
surfaces them.

## Why this, why now

- **Operator-pressure-driven.** The Phase 126 amendment
  was explicit: fix the tooling, not document the gap.
- **Phase 126 substrate landed cleanly enough to extend.**
  The `textual_tool_call` module is purely additive at
  the crate level; Phase 127 grows the parser registry
  without touching lib.rs, the audit chain, or the
  planner integration's outer shape.
- **The user's local stack needs it.** `qwen3.6:27b` is
  the operator's preferred local model; per the Phase 126
  research it's emitting Qwen3-Coder XML which Phase 126
  drops. Phase 127 Task 2 alone produces empirical rescue
  for the user's actual setup.
- **Substrate-axis-honest.** Phase 124 named the
  local-LLM-rehab axis as ceiling-reached. Phase 126
  partially walked that back ("textual extraction is a
  generic planner substrate, not local-LLM-specific").
  Phase 127 commits to the walk-back: substrate
  completion against the multi-model landscape, not
  cloud-only retreat.

## Q-block sign-off (Recommended for all six)

- **Q1a — Add Qwen3-Coder XML parser** (Recommended).
  The user's qwen3.6:27b is the load-bearing case; per
  Ollama issue #14745 and #14493 the format is
  empirically observed.

- **Q2a — Add bare-JSON parser with false-positive
  guard** (Recommended). Issue #11662 shows qwen3:32b
  emits raw JSON without any wrapper. Guard: only
  fires when JSON object is the entire response content
  (after trimming whitespace + optional `<think>...
  </think>` prefix). Mid-prose JSON is NOT extracted.

- **Q3a — Add Phi-4-mini `<|tool_call|>` wrapper**
  (Recommended). Phi-4-mini is widely-deployed for
  edge inference; format documented in Microsoft's
  PhiCookBook + Ollama's phi4-mini modelfile template.

- **Q4a — Add Gemma 3 ```tool_code``` markdown-fence +
  Python-call translator** (Recommended). Gemma 3 emits
  pythonic `func(k=v)` inside markdown code fences;
  this is the Phase 127 cost-heaviest task (needs an
  AST-light Python-call → JSON-args translator) but
  it's load-bearing for the Gemma family which is
  popular for fine-tuned use cases.

- **Q5a — Hybrid family-hint architecture**
  (Recommended). Query Ollama `/api/show` once per
  session (or at provider construction), cache the
  reported family string, use it to reorder the parser
  registry's priority — family-specific parser tried
  first, permissive fallback to all parsers if family-
  specific misses or family unknown. NOT family-strict
  dispatch (because Ollama lies about family vs.
  pipeline-wiring per the upstream bug literature).

- **Q6a — INSTALL.md substrate-coverage matrix +
  model-recommendation guidance** (Recommended).
  Operators picking a local model should see which
  formats Aivyx handles natively + which models the
  upstream Ollama wiring breaks (qwen3.5/3.6 even with
  Phase 127 substrate; recommend qwen3-coder if the user
  wants Qwen). Llama 3.1, mistral-nemo, phi4-mini are
  the "reliable native tool-use" trio per the Ollama
  blog announcement + this phase's literature.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract amendment.
  Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to eighteen** (was 17
  after Phase 126).

- **PRODUCT.md** — **Will hold.** No contract change.
  G6 + the planner-substrate envelope cover this case
  exactly as Phase 126. Hash:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to eighteen**.

- **`aivyx-core/src/lib.rs`** — **Will hold.** Phase
  127 is purely additive at the `textual_tool_call`
  module level + new `family_hint` module + Ollama
  client query. NO changes to `AuditTag::ToolCall`,
  `AuditEvent`, or any other lib.rs-resident type.
  Hash:
  `9692e5d102ca0721f1a6a958fda1d24f287e0b1295a09193db92ad82eb5cde35`.
  Prediction: streak **extends from 0 to 1** (Phase 126
  reset; Phase 127 rebuilds).

- **New workspace deps** — Zero anticipated. The Ollama
  client already exists for the provider; family-hint
  query reuses the existing reqwest/serde stack.
  Python-call parser is hand-written (small grammar);
  no `pyo3` or full Python AST dep.

- **Test count** — Four new format parsers + family-
  hint architecture + bare-JSON false-positive guard +
  INSTALL.md (no tests). Per-task estimate:
  - Task 2 (Qwen3-Coder XML): ~20 tests
  - Task 3 (Phi-4-mini): ~10 tests
  - Task 4 (Gemma 3 + Python-call): ~25 tests
    (Python-call grammar is the test-heaviest)
  - Task 5 (bare JSON + FP guard): ~15 tests
  - Task 6 (family-hint architecture): ~8 tests
  - Task 7 (planner integration sanity): ~4 tests

  Prediction: **`+70` to `+90`**. Compares to Phase
  126's `+35`; this phase is substrate-broader but
  reuses the planner-integration outer shape.

## Tasks

Eight sub-tasks plus exit + hash-backfill:

### Task 1 — Open (this commit)

`docs/PHASE_127.md` + `docs/ROADMAP.md` Phase 127 entry +
`docs/README.md` status row. Documents the format catalog
+ the four-pattern scope + family-hint architecture +
streak predictions.

### Task 2 — Qwen3-Coder XML parser

Extend `aivyx-core/src/textual_tool_call.rs` with a
Qwen3-Coder XML inner-parser. Format:

```text
<tool_call>
<function=fs.write>
<parameter=path>test.txt</parameter>
<parameter=content>phase 127 verification</parameter>
</function>
</tool_call>
```

Implementation:

- New `InnerFormat` enum variant `Qwen3CoderXml`.
- Permissive-parse: inside a `<tool_call>` block, first
  try existing JSON shapes, then try Qwen3-Coder XML
  shape, first match wins.
- The XML shape: `<function=NAME>` opens, `<parameter=K>V</parameter>`
  bodies (any number, any order), `</function>` closes.
- Parameter values are JSON-coerced: numeric strings →
  numbers, `"true"`/`"false"` → booleans, otherwise
  string. Conservative coercion (matches what
  llama.cpp's Qwen3-Coder parser does).
- `wrapper_tag` field is still `"tool_call"`; new
  `inner_format` field on `ExtractedToolCall` records
  whether the inner was JSON or Qwen3-Coder XML for
  audit forensics.

Tests (~20):

- Single function block with one parameter.
- Single function block with multiple parameters.
- Function block with no parameters (still valid).
- Value coercion: int, float, bool, string, JSON-array,
  JSON-object as parameter value.
- Multiple function blocks inside one `<tool_call>`
  wrapper (model emitted batch).
- Malformed: missing `<function=>` → dropped.
- Malformed: unclosed `<parameter>` → dropped.
- Mixed: `<tool_call>` containing JSON shape (existing
  Phase 126 parser handles) vs `<tool_call>` containing
  XML (Task 2 parser handles); same wrapper, different
  inner.
- Whitespace tolerance around parameter values.
- Parameter name with dot (`fs.write` style) preserved
  verbatim.

### Task 3 — Phi-4-mini `<|tool_call|>` wrapper

Extend the wrapper registry with `<|tool_call|>` /
`<|/tool_call|>` as a new wrapper variant. Inner format:
JSON list of `{"name": ..., "arguments": ...}` objects
(Phi-4-mini emits batch calls as a list).

Implementation:

- New wrapper entry in `WRAPPERS`.
- Inner-format `JsonListNameArgs` — parses JSON array,
  iterates, each element becomes an `ExtractedToolCall`.
- `wrapper_tag` field set to `"|tool_call|"` (literal
  string with bars) so audit can distinguish from the
  bare `<tool_call>` wrapper.

Tests (~10):

- Single-element JSON list.
- Multi-element JSON list (parallel calls).
- Empty JSON list → no extractions, not an error.
- Inner not a JSON list (object instead) → drop.
- Mixed in same response: `<|tool_call|>` block and
  separate `<tool_call>` block → both extracted in
  source order.

### Task 4 — Gemma 3 ```tool_code``` python-fence + Python-call translator

Two pieces:

1. **Markdown-fence wrapper** in the registry:
   - Open: ` ```tool_code` (backticks-tool_code, optionally
     followed by newline)
   - Close: ` ``` `
   - Permissive: accept `\n```\n` or `\n```` at end.

2. **Python-call inner-format parser** translating
   `func_name(param1=value1, param2=value2)` into
   `ExtractedToolCall` with arguments-as-object.

   Grammar (hand-written; no Python AST dep):

   ```text
   call    := name '(' [args] ')'
   name    := IDENT ( '.' IDENT )*
   args    := arg ( ',' arg )*
   arg     := IDENT '=' value
   value   := STRING | NUMBER | BOOL | NONE | LIST | DICT
   STRING  := ' ... ' | " ... "  (single or double quoted)
   NUMBER  := int | float
   BOOL    := True | False
   NONE    := None
   LIST    := '[' [value (',' value)*] ']'
   DICT    := '{' [STRING ':' value (',' STRING ':' value)*] '}'
   ```

   Values translated:
   - Python `True`/`False` → JSON `true`/`false`
   - Python `None` → JSON `null`
   - Quoted strings → JSON strings (handle backslash
     escapes minimally: `\\`, `\"`, `\'`, `\n`, `\t`)
   - Numbers passed verbatim into JSON.
   - Lists/dicts recurse.

Tests (~25):

- Single-call, one string arg.
- Single-call, mixed-type args (string + int + bool).
- Single-call, list arg.
- Single-call, nested dict arg.
- Single-call, no args (`func()`).
- Dotted function name (`fs.write(path='x')`).
- Multiple calls in one fence block (extracted as
  separate ExtractedToolCalls).
- Whitespace tolerance inside the fence + around args.
- Malformed: unclosed paren → drop.
- Malformed: missing `=` between key and value → drop.
- Malformed: unmatched quotes → drop.
- Quoted string with escape sequences.
- Single-quoted vs double-quoted strings.
- Integer + float number parsing.
- `None` as arg value.
- Empty list / empty dict.
- Markdown fence with leading language tag variant
  (` ```tool_code\n` vs ` ```python\n` — only the
  former extracts).

### Task 5 — Bare-JSON extractor with false-positive guard

Detection happens **only when the wrapper-based parsers
returned zero extractions** (so this is a fallback, not
a primary scanner — avoids extracting JSON the operator
mentioned in prose).

Guard logic:

- Strip leading/trailing whitespace from the response.
- Strip a single leading `<think>...</think>` block if
  present (thinking-mode response prefix).
- Test whether the remaining string is exactly one
  top-level JSON value (object) parseable as a tool-call
  shape (`{name, arguments}` or `{tool, parameters}`).
- If yes → extract one call. If no → no extraction.

Tests (~15):

- Pure JSON content extracts.
- JSON with leading whitespace extracts.
- JSON after a `<think>` block extracts.
- JSON embedded in prose ("the answer is `{...}`")
  does NOT extract (load-bearing FP guard).
- JSON immediately followed by prose does NOT extract.
- JSON missing `name`/`arguments` shape does NOT
  extract.
- Empty content returns no extractions.
- Just `<think>...</think>` with no JSON returns no
  extractions.
- JSON array (not object) does NOT extract via this
  path (only `<|tool_call|>`-wrapped JSON lists
  extract).
- Two JSON objects concatenated does NOT extract
  (ambiguous; not a single-call response).

### Task 6 — Family-hint architecture

Query Ollama `/api/show` once at provider construction
(cache the result for the provider's lifetime). Parse
the `details.family` field; use it as a priority hint
into the parser registry.

Implementation:

- New module `aivyx-llm/src/ollama_family.rs` (lives in
  the LLM provider crate, not core — it's Ollama-
  specific).
- Single fetch + cache; failure to fetch is silently
  treated as "unknown family" (permissive fallback).
- The provider passes the family string into the
  planner's text extraction call site via the existing
  `LlmResponse` envelope (already carries provider
  metadata).
- `textual_tool_call::extract_tool_calls_with_hint(text,
  family_hint: Option<&str>)` is the new entry point;
  the existing `extract_tool_calls(text)` becomes a
  zero-hint wrapper for backward compatibility (and
  for the non-Ollama providers that won't carry a
  family).

Family → parser priority table:

| Family (from `/api/show`) | Priority order |
|---|---|
| `qwen35` / `qwen3` | Qwen3-Coder XML → JSON `<tool_call>` → JSON `<tool_code>` → bare JSON |
| `qwen3-coder` | Qwen3-Coder XML → JSON `<tool_call>` → bare JSON |
| `gemma3` / `gemma` | Gemma 3 python-fence → JSON `<tool_code>` → JSON `<tool_call>` → bare JSON |
| `phi4` / `phi` | Phi-4-mini `<|tool_call|>` → JSON `<tool_call>` → bare JSON |
| `llama3` / `llama` | JSON `<tool_call>` → bare JSON (Python-call deferred) |
| `mistral` | JSON `<tool_call>` → bare JSON ([TOOL_CALLS] deferred) |
| _unknown / missing_ | All parsers tried in alphabetical order; first match wins |

Tests (~8):

- Family hint reorders priority correctly.
- Unknown family falls back to full scan.
- Family hint of `"qwen35"` prefers Qwen3-Coder XML
  parser over JSON parser when both match.
- `extract_tool_calls(text)` (no hint) behaves
  identically to permissive-scan mode.
- Empty family string treated as no-hint.
- `/api/show` 404 (older Ollama) handled gracefully.

### Task 7 — Planner integration sanity + composition tests

The planner integration site (`llm_planner.rs` line ~799)
calls `extract_tool_calls`. With Task 6's hint plumbing,
the call site becomes:

```rust
let family_hint = self.provider.tool_call_family_hint();
let extracted = textual_tool_call::extract_tool_calls_with_hint(
    &text, family_hint.as_deref(),
);
```

Task 7 confirms the composition works end-to-end:

- Existing 63 llm_planner tests pass unchanged.
- New tests (~4):
  - Qwen3-Coder XML extraction → dispatch.
  - Phi-4-mini JSON-list extraction → batch dispatch.
  - Gemma 3 python-fence extraction → dispatch.
  - Bare-JSON extraction → dispatch + Phase 120 fuzzy-
    recovery still composes.

### Task 8 — INSTALL.md substrate-coverage matrix + exit

INSTALL.md gets two new sub-sections under the existing
local-LLM-rehab section:

1. **Substrate-coverage matrix** — the 10-format table
   above, with per-row "Phase 127 handles?" column.
2. **Operator model-picking guidance:**
   - **Reliable native-protocol path:** Llama 3.1+,
     Mistral Nemo, Phi-4-mini (per Ollama tool-support
     blog post + this phase's literature).
   - **Reliable via Phase 127 substrate:** qwen3 family
     (Hermes pipeline), qwen3-coder (with Ollama's
     correct upstream pipeline), gemma3 (python-fence),
     phi4-mini (Phase 127 covers the wrapper).
   - **Known upstream issues:** qwen3.5 / qwen3.6 wired
     to wrong renderer/parser per Ollama issue #14493 —
     Phase 127's Qwen3-Coder XML parser catches the
     emission qwen3.5 actually produces, but tool
     definitions may still be malformed via the modelfile
     template bug (#14601). Workaround: use
     `qwen3-coder:N` model name where N is the
     parameter size; that gets Ollama's correct
     upstream pipeline.
   - **Known gaps:** Llama Python-call (need separate
     dispatch substrate; deferred), DeepSeek dynamic
     XML (need registry-driven match; deferred), Gemma 4
     special-token (handled server-side by Ollama
     0.20.0-rc1+).

Phase 127 exit doc with prediction-vs-reality + final
streak update.

## Exit criteria

- [ ] `docs/PHASE_127.md` + ROADMAP Phase 127 entry +
  `docs/README.md` status row — Task 1.
- [ ] Qwen3-Coder XML parser + tests — Task 2.
- [ ] Phi-4-mini `<|tool_call|>` parser + tests — Task 3.
- [ ] Gemma 3 python-fence + Python-call translator
  + tests — Task 4.
- [ ] Bare-JSON parser with FP guard + tests — Task 5.
- [ ] Family-hint architecture via Ollama /api/show +
  tests — Task 6.
- [ ] Planner integration composition tests — Task 7.
- [ ] INSTALL.md substrate-coverage matrix + Phase 127
  exit doc — Task 8.
- [ ] Q1 / Q2 / Q3 / Q4 / Q5 / Q6 resolved pre-Task 2
  (all Recommended).
- [ ] DESIGN.md streak — predicted HOLD (streak → 18).
- [ ] PRODUCT.md streak — predicted HOLD (streak → 18).
- [ ] `aivyx-core/src/lib.rs` streak — predicted HOLD
  (0 → 1; Phase 126 reset, Phase 127 rebuilds).
- [ ] Zero new workspace dependencies.
- [ ] Test count delta within `+70` to `+90`.
- [ ] Zero clippy warnings.
- [ ] **No live verification.** Phase 127 substrate is
  unit-tested per-format; family-hint architecture is
  unit-tested for routing. The operator can run an
  interactive session post-exit to validate empirically,
  but this is operator-discretionary, not a phase exit
  criterion.

## Honest scope risks at sign-off

- **Qwen3.5/3.6 may still fail even after Task 2.**
  Phase 127's Qwen3-Coder XML parser catches the
  emission format the model produces, but per Ollama
  issue #14601 tool *definitions* are also malformed
  (Go struct strings in the modelfile template). If
  the model can't see correct schemas, it may not
  generate the right tool name/args even when the
  format is parseable. Honest acknowledgement: Phase
  127 closes the *parsing* gap, not the upstream
  Ollama schema-rendering gap. INSTALL.md surfaces the
  `qwen3-coder:N` workaround for users who want a
  working Qwen tool-use path right now.

- **Python-call translator is the most complex piece.**
  Task 4's hand-written parser handles the common
  Gemma 3 emission patterns but is not a full Python
  expression evaluator. Malformed-but-recoverable
  inputs may drop instead of repairing. The conservative
  posture matches Phase 126's "drop silently rather
  than dispatch wrong call."

- **Family-hint is a hint, not a contract.** Ollama's
  `details.family` is what the model registry says, not
  necessarily what the pipeline wiring uses (the
  qwen3.5 → Hermes-pipeline mismatch is exactly that:
  family `qwen35` but pipeline `Qwen3VLRenderer`).
  Permissive fallback covers this; family-hint just
  reorders priority, never excludes.

- **Test-count overshoot risk on Task 4.** The
  Python-call parser's grammar is the test-heaviest
  surface. If grammar edge cases compound, Task 4 could
  push test count toward the `+90` end of the range.
  Acceptable; honest report at exit.

- **Phase 127 doesn't address LLM-side tool-call
  reliability.** A model that doesn't *want* to call
  a tool (chooses to describe what it would do, in
  prose, no wrapper, no JSON) can't be rescued by any
  parser. Phase 127 closes the substrate gap; model
  intent is unchanged from Phase 124's substrate-
  ceiling finding.

- **Fifteenth consecutive deferral of the Channel
  Activation Milestone.** Honest tracking continues.
  Audit's #1.

## Direction after Phase 127

Phase 128 candidates (operator-pressure-driven):

1. **Cloud-LLM-side substrate work** — Anthropic /
   OpenAI provider polish; cache hit-rate
   instrumentation; tool-use latency profiling.
2. **Channel Activation Milestone** — fifteenth
   consecutive deferral if skipped; substrate work
   may now be at a coverage point that channels become
   the load-bearing question.
3. **Llama Python-call dispatch substrate** — adds the
   Phase 127 deferral for Llama 3.x.
4. **DeepSeek dynamic-XML dispatch substrate** — adds
   the second Phase 127 deferral.
5. **Chapter G #2 / Chapter F #2** — second tool
   bundles or productivity integrations.
6. **Release prep (v0.1.0 + installer)** — substrate-
   completion postcard would mark a good release
   milestone if Phase 127 lands clean.

Phase-by-phase decision at Phase 127 exit, sharpened by
substrate-coverage findings and operator pressure.

## Prediction vs reality

_Populated at Phase 127 exit. Predictions captured at
sign-off: DESIGN.md HOLD → 18; PRODUCT.md HOLD → 18;
lib.rs HOLD → 1 (rebuild from 0); test count `+70` to
`+90`; zero new deps; zero clippy warnings._

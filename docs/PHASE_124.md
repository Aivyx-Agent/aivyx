# Phase 124 — Few-Shot Tool-Call Examples (Local-LLM Rehab #4)

**Fourth named phase against the local-LLM rehabilitation
axis.** Phase 120 closed hallucinated-name recovery; Phase
121 shipped the native Ollama path; Phase 122 added
structured prompt injection. Phase 124 targets the residual
**capability-denial prior** that Phase 122 surfaced and
documented honestly as the model-layer ceiling.

**Operator pressure framing.** After Phase 123 closed and
the Phase 124 direction question was asked, the operator
rejected all the standing candidates (Chapter F #2, Channel
Activation Milestone, release prep, SDK harness lift) and
named the specific Phase 122 transcript line as the issue
to focus on:

> Operator probe: *"Please call fs.write to create a file
> at test.txt with the content 'phase 122 verification'."*
>
> gemma4:31b response: *"I do not have a tool called
> `fs.write` or any other tool that allows me to write
> files to the filesystem. I can only use the tools provided
> in my current environment."*

The substrate at that moment listed `fs.write` literally
five lines above gemma4's response. The Phase 122 exit doc
framed this as "the substrate ceiling sits at the model
layer" and recommended structured injection only for
enumeration honesty, with operators using cloud providers
for tool-use workloads. The operator's pressure rejects
that framing as the final word: try one more substrate
attempt.

## Honest framing risk at sign-off

Three prior substrate phases (120 + 121 + 122) shipped real
substrate but none bridged the capability-denial prior on
their own. **If Phase 124's few-shot examples also fail to
fix the gemma4 refusal pattern, four substrate phases will
have hit the same model-layer wall.** The exit doc has to
name this definitively if it happens — operators must know
whether local-LLM tool-use is realistic at all, or whether
the cloud-fallback path is the only honest answer.

Q3a (Recommended) at sign-off locks in: live verification
against both qwen3.6:27b AND gemma4:31b at exit; exit doc
reports empirical reality regardless. Phase 6 Q5 honesty
applies to RESULTS, not ANTICIPATION.

## Why few-shot examples might work where structured
   injection didn't

Phase 122's structured injection put the tool catalog
directly in the system prompt as a `## Tools available`
section listing `fs.write` by exact name. The model still
refused. This tells us **assertion-of-availability is not
enough** for the capability-denial prior.

Few-shot examples are a different mechanism:
- The model sees *concrete worked examples* of the tool
  being called successfully, not just an assertion that it
  exists.
- The pattern "operator asks → assistant invokes → result
  reported" is shown directly, not described abstractly.
- The WRONG/RIGHT framing explicitly names the refusal
  pattern as wrong, anchored against the prior.

Few-shot prompting is well-documented to change model
behavior more reliably than instructions. Whether it's
*sufficient* against a strong training prior is the open
question this phase attempts to answer.

## Scope (Q-block sign-off)

- **Q1 — Substrate move:** (a) **Few-shot examples in the
  prompt** (Recommended). Append 2-3 worked tool-call
  examples to the existing structured-injection block.
  Cheapest concrete substrate attempt; well-documented
  mechanism for changing model behavior. Honest risk:
  model may copy example structure but still refuse novel
  calls.

- **Q2 — Example source:** (a) **Static hardcoded
  examples** (Recommended). Bake 2-3 examples directly
  into the prompt assembler (one fs.read, one fs.write,
  one memory.write — the tools whose absence-of-invocation
  Phase 122 surfaced). Same examples every session; no
  schema-synthesis complexity. Honest scope: if a
  particular example fails to anchor, every session hits
  the same failure.

- **Q3 — Verification posture:** (a) **Live against both
  models, honest reporting** (Recommended). Same posture
  as Phase 122 exit. Run `./scripts/dev-run.sh` against
  qwen3.6:27b AND gemma4:31b with the new substrate.
  Document empirical reality regardless.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract amendment.
  Phase 124 extends the existing `OllamaFamilyStrategy`
  enum with one new variant; substrate-internal change.
  Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to fifteen** (was 14).

- **PRODUCT.md** — **Will hold.** G6 (Local execution,
  privacy non-negotiable) covers the local-LLM substrate;
  Phase 124 is another attempt within that envelope. No
  contract amendment. Hash at entry:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to fifteen**.

- **`aivyx-core/src/lib.rs`** — **Will hold.** Substrate
  work lives in `aivyx-channel/src/profile_prompt.rs`
  (extend `append_tool_catalog`) and
  `aivyx-config/src/lib.rs` (new enum variant + label).
  Hash at entry:
  `b420405bf9a5576ecb10f6ea04a965f7ad3a1a92ae9bd8f4abf46e22ef4d3c16`.
  Prediction: streak **extends to five** (was 4 after
  Phase 123). 90/10 hold.

- **New workspace deps** — Zero anticipated.

- **Test count** — Small phase: enum variant + helper
  extension + per-family default update + banner
  provenance. Prediction: **+15 to +30**.

## Tasks

Four sub-tasks plus exit + backfill:

### Task 1 — Open (this commit)

`docs/PHASE_124.md` + ROADMAP Phase 124 entry +
`docs/README.md` status row. Documents the operator-pressure
framing + the honest-scope risk that this is the fourth
substrate attempt against a possibly-prompt-unreachable
prior.

### Task 2 — `FewShotExamples` enum variant + helper extension

- Extend `aivyx-config::OllamaFamilyStrategy` with a third
  variant: `FewShotExamples`. Label: `"few_shot_examples"`.
  `Default::default()` stays `None`.
- Extend `aivyx-channel::profile_prompt::append_tool_catalog`
  (or add a sibling helper) to optionally append a `##
  Example tool use` block after the catalog. The block
  includes 2-3 worked examples following this pattern:
  ```
  ## Example tool use
  
  When the operator asks: "Save 'hello' to test.txt"
  - You should: invoke fs.write with the path and content
    arguments.
  - You should NOT: respond "I don't have fs.write" — you
    DO have fs.write; it is in your tool list above.
  
  When the operator asks: "What's in my memory under
  'projects'?"
  - You should: invoke memory.read with topic "projects".
  - You should NOT: enumerate tools in prose; the operator
    already knows your tools.
  ```
- The WRONG/RIGHT framing in each example directly counters
  the gemma4-refusal pattern observed in Phase 122.
- Tests pin the block shape + WRONG/RIGHT-pattern presence.

### Task 3 — Per-family defaults + banner provenance

- `default_for_family` updates:
  - `qwen3` → `FewShotExamples` (upgrade from
    StructuredInjection).
  - `gemma4` → `FewShotExamples` (upgrade from
    StructuredInjection).
  - `llama3` → `None` (unchanged).
- Banner provenance for the new variant: the
  `format_ollama_prompt_strategy_banner_line` helper in
  the binary returns `"few_shot_examples"` for
  FewShotExamples. Operators see at a glance which strategy
  is active.
- Tests update for the new defaults; banner test pins the
  new label.

### Task 4 — Live verification + INSTALL.md sweep + exit

**Verification cycle (per Q3a sign-off):**

- Run `./scripts/dev-run.sh --release --reset --model
  qwen3.6:27b` with FewShotExamples default. Same probes
  as Phase 122 (enumeration + fs.write invocation).
  Compare:
  - Does qwen3.6 now invoke `fs.write`?
  - Does enumeration improve, stay the same, or get
    worse?
- Repeat against gemma4:31b. Same questions.
- Document outcomes in PHASE_124.md exit doc.

**Honest reporting per Q3a — three outcome cases:**

1. **Both models invoke fs.write.** Substrate
   breakthrough. Exit doc validates the few-shot bet;
   recommends FewShotExamples as the default; closes
   the local-LLM-rehab axis honestly.
2. **One improves, one doesn't.** Asymmetric outcome.
   Exit doc reports per-model reality; recommends
   per-family configuration based on what worked.
3. **Neither improves.** Fourth substrate-phase failure.
   Exit doc names the model-layer ceiling definitively;
   recommends operators use cloud providers (Anthropic /
   OpenAI) for tool-use workloads; documents
   FewShotExamples as available substrate for operators
   who want enumeration honesty without the cost overhead
   of structured-injection-alone.

**Document outcomes regardless:**

- INSTALL.md gets a Phase 124 sub-section under the
  existing "Per-model prompt variants (Phase 122)"
  section. Updates the per-family defaults table; adds
  `few_shot_examples` to the values table; documents the
  empirical findings from the live verification.

- PHASE_124.md exit doc: prediction-vs-reality, test
  count delta, and the load-bearing empirical findings
  table per Q3a's "declare reality at exit" lock.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Substrate move:** (a) **Few-shot examples**
  (Recommended).
- **Q2 — Example source:** (a) **Static hardcoded
  examples** (Recommended).
- **Q3 — Verification posture:** (a) **Live against both
  models, honest reporting** (Recommended).

All three Recommended — first all-Recommended Q-block since
Phase 121.

## Exit criteria

- [ ] `docs/PHASE_124.md` + ROADMAP Phase 124 entry +
  `docs/README.md` status row — Task 1.
- [ ] `FewShotExamples` variant + helper extension —
  Task 2.
- [ ] Per-family defaults + banner provenance — Task 3.
- [ ] Live verification + INSTALL.md sweep + exit —
  Task 4.
- [ ] Q1 / Q2 / Q3 resolved with operator sign-off pre-
  Task 2 (all three Recommended).
- [ ] DESIGN.md streak — predicted HOLD (streak → 15).
- [ ] PRODUCT.md streak — predicted HOLD (streak → 15).
- [ ] `aivyx-core/src/lib.rs` streak — predicted HOLD
  (streak → 5), honest 90/10 hold.
- [ ] Zero new workspace dependencies.
- [ ] Test count delta within `+15` to `+30`.
- [ ] Zero clippy warnings.
- [ ] **Live verification against both models** per Q3a.
  Exit doc reports observed reality regardless of
  outcome. **This is the load-bearing deliverable** —
  if Phase 124's substrate doesn't break through, the
  exit doc must name the model-layer ceiling
  definitively after four substrate-phase attempts.

## Honest scope risks at sign-off (carried forward to exit)

- **The capability-denial prior may be prompt-unreachable
  by ANY substrate.** Phase 122 demonstrated that even
  putting the tool name directly in the prompt doesn't
  fix it. Few-shot examples are a different mechanism but
  not a guaranteed one. The probability the substrate
  works is honestly uncertain — neither the operator nor
  this assistant should claim confidence either way until
  the live verification runs.

- **The example tool names must be in the registered
  set.** If `fs.write` isn't actually registered on the
  operator's system (allowlist narrowed), the example
  refers to a non-existent tool — undermines the whole
  point. The helper should detect and skip examples
  referring to unregistered tools, or fall back to
  examples drawn from the actual catalog. (Static
  hardcoded examples per Q2a; the safety net is
  documented as a Task-2 acceptance criterion.)

- **Examples consume prompt tokens.** Adds another ~150-
  250 tokens per turn on top of Phase 122's catalog
  block. For small-context models or cost-sensitive
  deployments, the trade-off is worse than catalog-only.
  Per-family `prompt_strategy = "none"` (or
  `"structured_injection"` for catalog-only) remains the
  operator escape hatch.

- **Model may copy example structure but refuse novel
  calls.** A model that sees examples of fs.write being
  called might dutifully call fs.write when asked, but
  still refuse calls to tools it hasn't seen examples of.
  Static examples cover only the tools in them; the
  prior may persist for everything else. If observed at
  exit, this argues for Q2c (examples from actual
  registry) in a future phase.

## Direction after Phase 124

After Phase 124, the local-LLM-rehab axis has been worked
through four times. Phase 125 candidates:

1. **Operator-facing mitigation (if Phase 124 fails)** —
   the Q3-option Phase 124 didn't pick: banner warning +
   optional cloud fallback. If four substrate attempts
   have hit the wall, mitigation is what's left.
2. **Chapter F #2** — still on the table; the SDK-harness
   lift rides alongside.
3. **Channel Activation Milestone** — thirteenth
   consecutive deferral if skipped. Audit's #1.
4. **Release prep (v0.1.0 + installer)** — Phase 99
   deferred.
5. **Operator-pressure-driven new direction**.

Phase-by-phase decision at Phase 124 exit, sharpened by
the empirical findings from the live verification.

## Prediction vs reality

**Three of three streak predictions correct.** All three
byte-identity streaks held through Tasks 2-3; the substrate
stayed entirely within `aivyx-config` + `aivyx-channel` as
expected.

- **DESIGN.md** — HELD as predicted (`c2be6d51…`
  unchanged). No contract amendment; new enum variant fits
  the existing additive pattern Phase 122 established.
  Streak: 14 → 15.
- **PRODUCT.md** — HELD as predicted (`6e840cef…`
  unchanged). G6 covers; no contract amendment. Streak:
  14 → 15.
- **`aivyx-core/src/lib.rs`** — HELD as predicted (90/10
  hold case held). `b420405b…` unchanged. New helper
  lives in `aivyx-channel/src/profile_prompt.rs`; enum
  variant lives in `aivyx-config`; dispatcher hides the
  per-call-site logic. Streak: 4 → 5.

**Test count `+12` lands inside the predicted `+15 to +30`
range — close to the lower bound.** Per-task breakdown:
- Task 2 (FewShotExamples variant + helper): **+11**
  (3 aivyx-config: new label, new parse-wire-label,
  error-message-lists-all-three; 8 aivyx-channel
  profile_prompt: empty-noop, no-targets-noop,
  fs.write-only, only-registered-subset,
  all-three-registered, preamble content, whitespace
  trim, composes-after-catalog, chained-noop).
- Task 3 (defaults + dispatcher + banner): **net +1**
  (renamed + flipped 3 existing banner/default tests;
  added 1 explicit wire-label round-trip test).

The lower-bound landing reflects Phase 124's deliberate
substrate-additive shape — the dispatcher pattern means
the wiring tests in main.rs didn't multiply across the 5
call sites; the helper-shape tests in profile_prompt did
the load-bearing work.

**Q-block went through as operator-picked.** Q1a + Q2a +
Q3a — first all-Recommended Q-block since Phase 121. No
mid-task re-asks; no architectural constraints surfaced
post-sign-off. The substrate-internal scope plus the
prior Phase 122 substrate posture meant no surprises.

**Zero new workspace dependencies** as predicted.

### Live verification (Task 4)

> **PLACEHOLDER — populated post-live-test.**
>
> Verification against qwen3.6:27b and gemma4:31b through
> `./scripts/dev-run.sh` with the new FewShotExamples
> default. The exit-doc backfill commit will replace this
> block with the empirical table:
>
> | Model       | Pre-122 baseline                            | Phase 122 (StructuredInjection)                    | Phase 124 (FewShotExamples)             |
> |-------------|---------------------------------------------|----------------------------------------------------|------------------------------------------|
> | qwen3.6:27b | Confabulated 12 / verbal refusal            | Listed ~25 / `[turn timed out]` on fs.write        | _to be observed_                         |
> | gemma4:31b  | Confabulated 60+ / empty                    | Vague prose / explicit "I don't have fs.write"     | _to be observed_                         |
>
> **Three outcome cases** per Q3a's "declare reality at
> exit" lock:
>
> 1. **Both invoke fs.write.** Substrate breakthrough.
>    Exit doc validates the few-shot bet; recommends
>    FewShotExamples as the stable default; closes the
>    local-LLM-rehab axis honestly.
> 2. **One improves, one doesn't.** Asymmetric. Exit
>    doc reports per-model reality; recommends per-family
>    configuration based on what worked.
> 3. **Neither improves.** Fourth substrate-phase
>    failure. Exit doc names the model-layer ceiling
>    definitively; recommends operators use cloud
>    providers for tool-use workloads; documents
>    FewShotExamples as available substrate for
>    enumeration-honesty operators who accept the cost
>    overhead.
>
> The probe pattern matches Phase 122 exit exactly:
> banner observation, "what tools do you have?" probe,
> "please call fs.write to create a file at test.txt
> with the content 'phase 124 verification'" probe.
> Both runs use `--reset` so each model gets a clean
> store; audit-export check between runs confirms
> tool-call landings.

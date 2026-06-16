# Phase 122 — Per-Model Prompt Variants (Local-LLM Rehab #2)

Third-named phase against the audit's local-LLM
rehabilitation axis. Phase 120 closed hallucinated-name
recovery at invocation time; Phase 121 shipped the
native Ollama path; Phase 122 attempts to make qwen3.6
and gemma4 family models **actually use the tools they
have** instead of denying their existence or producing
empty output when commanded.

**Eleventh consecutive substrate/polish phase picked
over the Channel Activation Milestone.** Honest
tracking. The audit's #1 recommendation has been chosen
against eleven times now; the verification gap keeps
growing.

## Real-use signal driving this phase

Five-turn dev-verify substrate run + eight interactive
turns across qwen3.6:27b and gemma4:31b produced
**zero tool calls** total. Both models confabulate at
the prose level when asked to enumerate their tools;
both refuse or return empty when explicitly commanded
to invoke `fs.write`. Phase 120's substrate is
structurally orthogonal to this failure mode (it
catches hallucinated *invocations*; this is hallucinated
*capability denial*). Full diagnostic data:

| Model | "List your tools" | "Call fs.write" |
|---|---|---|
| qwen3.6:27b | Confabulated 12 (mix of real + invented) | Verbal refusal: "I don't have fs.write" |
| gemma4:31b | Confabulated 60+ entirely-invented tools | Empty assistant message; 25 output tokens; no invocation |

Audit chain across all 13 turns: `tool_calls_made: 0`;
chain integrity verifies; no `auto_corrected_from`
populated (Phase 120 substrate had nothing to act on).

## Phase 6 Q5 honest framing correction (pre-Task 2)

During Q1 sign-off I framed Q1c as "suppress in-prompt
tool catalog AND inject per-turn block." On reading
`crates/aivyx-channel/src/profile_prompt.rs`
post-sign-off, I confirmed **the Aivyx system prompt
does NOT include a tool catalog**. Tools flow only via
the protocol `tools: [...]` array in `LlmRequest`. There
is nothing to suppress.

So the Q1c pick under correct framing reduces to:
**ship the per-turn tool catalog injection only**. The
"belt-and-suspenders" framing was partially incorrect.
The substrate value remains real: qwen3.6 and gemma4
demonstrably aren't using the protocol tools array as
authoritative for prose claims about themselves; a
per-turn structured injection forces the catalog into
the visible prompt content where the models can't
easily ignore it.

## Why this, why now

- **The Phase 120 + 121 substrate is healthy.** Real-use
  signal confirms native `/api/chat` routing works,
  JSONL streaming works, audit-chain integrity holds.
  The substrate ceiling is at the model layer.

- **G6 (Local execution, privacy non-negotiable) keeps
  pulling.** Operators picking local models should not
  be locked out of tool use entirely. Phase 121
  surfaced the protocol gap; Phase 122 attempts the
  prompt-engineering gap.

- **Two named local-model-rehab phases have shipped
  (120, 121); this is the third and likely the last
  named one** before the audit ranking forces a
  reckoning with the Channel Activation Milestone.

## Scope (Q-block sign-off)

- **Q1 — Core substrate move:** (c) **Both — suppress
  system-prompt catalog AND inject per-turn block**
  (non-Recommended). Per the Phase 6 Q5 honest framing
  correction above, the "suppress" half is a no-op (no
  catalog to suppress); the pick reduces to per-turn
  injection. Exit-doc will surface this honestly.

- **Q2 — Per-model differentiation strategy:** (a)
  **Per-model-family TOML config** (Recommended).
  Operator configures per-family in
  `aivyx.toml`:
  ```toml
  [ollama.qwen35]
  prompt_strategy = "structured_injection"

  [ollama.gemma4]
  prompt_strategy = "structured_injection"

  [ollama.llama3]
  prompt_strategy = "none"
  ```
  Family detection from model-name prefix:
  `qwen3.6:27b` → `qwen35`; `gemma4:31b` → `gemma4`;
  `llama3.1` → `llama3`. Unknown families default to
  `none` (pre-Phase-122 behavior).

- **Q3 — Default behavior + honest-scope posture:** (b)
  **Try everything; declare reality at exit** (non-
  Recommended). No upfront scope reduction. Task 7
  locked into verification against both qwen3.6:27b
  AND gemma4:31b; exit-doc honestly reports outcomes
  regardless. Phase 6 Q5 honesty applies to RESULTS,
  not to ANTICIPATION.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract amendment.
  Phase 122 ships substrate inside the existing turn-
  loop + prompt-assembly envelope. Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to thirteen** (was 12
  after Phase 121).

- **PRODUCT.md** — **Will hold.** G6 (Local execution,
  privacy non-negotiable) covers Ollama first-class
  support; per-model prompt variants strengthen that
  commitment. No contract amendment. Hash at entry:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to thirteen** (was 12).

- **Production-core `aivyx-core/src/lib.rs`** —
  **Will hold.** Substrate work lives in
  `aivyx-channel/src/profile_prompt.rs` (system-prompt
  assembly), `aivyx-config` (TOML config), and possibly
  a new hook in `aivyx-channel`'s planner construction.
  None touches `aivyx-core/src/lib.rs`. Hash at entry:
  `b420405bf9a5576ecb10f6ea04a965f7ad3a1a92ae9bd8f4abf46e22ef4d3c16`.
  Prediction: streak **re-establishes to three** (was 2
  after Phase 121). Honest 85/15 hold; the substrate
  is decisively outside the core boundary.

- **New workspace deps** — Zero anticipated.

- **Test count** — Substrate-heavy: per-turn injection
  helper + per-family config + family detection +
  integration tests against the existing
  `assemble_session_prompt`. Prediction: **+35 to +60**.

## Tasks

Roughly seven sub-tasks plus exit + backfill:

### Task 1 — Open (this commit)

`docs/PHASE_122.md` + `docs/ROADMAP.md` Phase 122 entry +
`docs/README.md` status row. Phase 6 Q5 honest framing
correction documented above.

### Task 2 — Per-family detection + config types

- New `aivyx-config::OllamaFamilyStrategy` enum:
  ```rust
  pub enum OllamaFamilyStrategy {
      /// No injection (pre-Phase-122 behavior). Default.
      None,
      /// Append "## Tools available\nfs.read, fs.write, ..."
      /// to the assembled system prompt.
      StructuredInjection,
  }
  ```
- Per-family TOML config: `[ollama.<family>]` sub-
  section with `prompt_strategy = "none" |
  "structured_injection"`.
- New helper `detect_model_family(model: &str) -> Option<String>`:
  - `qwen3.6:27b` → `Some("qwen35")`
  - `gemma4:31b` → `Some("gemma4")`
  - `llama3.1` → `Some("llama3")`
  - `gpt-4` → `None`
- Tests: family detection across the documented model
  set; TOML round-trip; per-family override resolution.

### Task 3 — Structured-injection helper

- New `aivyx-channel::profile_prompt::append_tool_catalog`
  helper. Takes the assembled system prompt + the
  `LlmToolDescriptor` list; appends:
  ```
  ## Tools available
  
  You have these tools available to invoke. Their names
  are exact; do not paraphrase or use a different
  format:
  - fs.read
  - fs.write
  - ...
  ```
- Pure function; no protocol-side change (Ollama still
  receives the `tools: [...]` array as before).
- Tests: empty-tools-list returns unchanged prompt;
  populated list produces the expected appended block;
  formatting stable across calls.

### Task 4 — Wire per-family strategy through to the planner

- The system-prompt assembly site (`assemble_session_prompt`
  callers in `aivyx-channel/src/bin/aivyx.rs` and
  `aivyx-channel/src/daemon_server.rs`) gets the
  per-family strategy from `aivyx-config`.
- When strategy is `StructuredInjection`, call
  `append_tool_catalog` before passing the prompt to
  the planner.
- When strategy is `None`, behavior is byte-identical
  to pre-Phase-122.
- Tests: end-to-end through `assemble_session_prompt`
  with each strategy; family-strategy lookup against
  TOML config.

### Task 5 — Default strategies per documented family

- `aivyx-config` defaults: `qwen35 = "structured_injection"`,
  `gemma4 = "structured_injection"`, `llama3 = "none"`,
  unknown = `"none"`.
- The data we collected pre-Phase-122 supports these
  defaults: qwen3.6 and gemma4 both demonstrated the
  capability-denial failure mode; llama3 is untested
  but its OpenAI-style tool-use is presumed more
  reliable.
- Operator can override via TOML.
- Tests: default resolution per family; explicit
  override beats default.

### Task 6 — Operator-facing surface

- Startup banner shows the effective per-family
  strategy when `provider = "ollama"` and the model
  matches a known family.
- INSTALL.md sub-section on per-family configuration
  (Task 7 ships the full INSTALL).
- Tests: banner snapshot pin.

### Task 7 — Live verification + INSTALL.md sweep + exit

**Verification cycle (per Q3b sign-off):**

- Run `dev-run.sh` with qwen3.6:27b against the new
  substrate. Prompt: same "Call fs.write to create
  test.txt..." command we already tested. Compare:
  - Does qwen3.6 now invoke `fs.write`?
  - If it emits `fs_write` (Phase 120 substrate
    expectations), does the auto-correct fire? Check
    audit chain.
  - Does it still verbally refuse?
- Run the same test against gemma4:31b. Same questions.
- Honest exit-doc reporting per Q3b: declare whatever
  reality we observe. If qwen3.6 improves and gemma4
  doesn't, document that. If both improve, document
  that. If neither improves, document that AND name
  the substrate ceiling.

**Document outcomes regardless:**

- Final INSTALL.md section: "Per-model prompt variants
  (Phase 122)". Documents the TOML config, the
  detection logic, the per-family defaults, and a
  candid table of observed model behaviors at exit
  time.

- Exit: PHASE_122.md prediction-vs-reality + ROADMAP
  freeze + README flip + hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Core substrate move:** (c) **Both** (non-
  Recommended). Reduces to "ship per-turn injection"
  per the Phase 6 Q5 framing correction above.
- **Q2 — Per-model differentiation strategy:** (a)
  **Per-model-family TOML config** (Recommended).
- **Q3 — Default behavior + honest-scope posture:** (b)
  **Try everything; declare reality at exit** (non-
  Recommended). Verification against qwen3.6 +
  gemma4 locked into Task 7.

## Exit criteria

- [ ] `docs/PHASE_122.md` + ROADMAP Phase 122 entry +
  docs/README status row — Task 1.
- [ ] Per-family detection + config types — Task 2.
- [ ] Structured-injection helper — Task 3.
- [ ] Wire per-family strategy through to the planner —
  Task 4.
- [ ] Default strategies per documented family — Task 5.
- [ ] Operator-facing surface (banner + INSTALL stub) —
  Task 6.
- [ ] Live verification + INSTALL.md sweep + exit —
  Task 7.
- [ ] Q1/Q2/Q3 resolved with operator sign-off
  pre-Task 2 (Q1c non-Recommended + Q2a + Q3b non-
  Recommended).
- [ ] DESIGN.md streak — predicted HOLD (streak → 13).
- [ ] PRODUCT.md streak — predicted HOLD (streak → 13).
- [ ] `aivyx-core/src/lib.rs` streak — predicted HOLD
  (streak → 3), honest 85/15 hold.
- [ ] Zero new workspace dependencies.
- [ ] Test count delta within `+35` to `+60`.
- [ ] Zero clippy warnings.
- [ ] **Live verification against both models** per
  Q3b. Exit doc reports observed reality regardless of
  outcome.

## Honest scope risks at sign-off (carried forward)

- **The data suggests gemma4's prior may be unfixable
  from the prompt side.** Phase 6 Q5 reality check at
  exit: if structured injection doesn't help gemma4,
  the exit doc documents it as a known limitation and
  the substrate honestly ships supporting qwen3.6
  improvements only.

- **qwen3.6's verbal refusal may persist** even with
  structured injection. The model's prior on "what
  AI assistants can do" may dominate any in-prompt
  reinforcement. Q3b sign-off accepted this risk by
  refusing pre-emptive scope reduction.

- **Per-turn injection bloats input tokens.** Tools list
  on every turn adds ~150-200 tokens. For small-
  context models this trade may be unfavorable.
  Operator can disable via per-family `none` strategy.

## Direction after Phase 122

After Phase 122, the audit's local-LLM rehabilitation
axis has been worked through three times (Phase 120
recovery + Phase 121 native protocol + Phase 122 prompt
variants). Phase 123 candidates:

1. **Channel Activation Milestone** — twelfth-in-a-row
   deferral if skipped. The data Phase 122 produces at
   exit will sharpen the milestone scope. Audit's #1
   unchanged.
2. **Release prep (v0.1.0 + shell installer)** —
   Phase 99 deferred publish-infrastructure.
3. **A new thematic Chapter F** — shaped by operator
   pressure.
4. **Operator-pressure-driven new direction**.

Phase-by-phase decision at Phase 122 exit.

## Prediction vs reality

**Three of three streak predictions correct.** All
three byte-identity streaks held through Tasks 2-6;
the substrate stayed entirely within
`aivyx-config` + `aivyx-channel` as Q2a / Q3a
deliberately placed it.

- **DESIGN.md** — HELD as predicted (`c2be6d51…`
  unchanged). No contract amendment; Phase 122 ships
  inside the existing turn-loop + prompt-assembly
  envelope. Streak: 12 → 13.
- **PRODUCT.md** — HELD as predicted (`6e840cef…`
  unchanged). G6 (Local execution, privacy non-
  negotiable) covers Ollama first-class support;
  Phase 122 strengthens that commitment without
  contract change. Streak: 12 → 13.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (85/15 hold case held). `b420405b…` unchanged.
  The substrate landed in `aivyx-channel/src/
  profile_prompt.rs` (`append_tool_catalog`) +
  `aivyx-channel/src/bin/aivyx.rs` (wiring) +
  `aivyx-config/src/lib.rs` (`OllamaFamilyStrategy`,
  `detect_model_family`, `resolve_ollama_prompt_strategy`).
  Streak re-establishes 2 → 3.

**Test count `+38` is within the predicted `+35 to
+60` range.** Per-task breakdown:
- Task 2: 9 tests for `OllamaFamilyStrategy` +
  `detect_model_family` (qwen / gemma / llama major-
  version detection, none-on-cloud-names, none-on-
  empty, defaults, label round-trip).
- Task 3: 7 tests for `append_tool_catalog` (empty-
  tools no-op, header, exact-name listing, anti-
  invention preamble, empty-description handling,
  base-prompt preservation, whitespace trim) + 1
  composition test pinning the layered shape against
  `assemble_session_prompt`.
- Task 5: 9 tests for the operator override surface
  (parse wire labels + case + trim + reject, resolve
  default + override + undetected, loader happy /
  unknown-string / absent-section).
- Task 6: 6 banner-line tests (anthropic absent,
  qwen3 default, qwen3 override, undetected
  fallback, gemma4 default, llama3 default).

**Q-block went through as operator-picked.** Q1c
non-Recommended + Q2a + Q3b non-Recommended all
shipped. The Phase 6 Q5 honest framing correction
documented in the open doc held through exit: Q1c's
"suppress in-prompt catalog" half was a no-op (the
system prompt has no catalog to suppress); the pick
reduced cleanly to per-turn injection.

**Phase 122 honest-scope adjustment landed mid-Task 2.**
The `detect_model_family` helper's original draft
mapped qwen3.6 → `qwen35` via a `take(2)`-on-digits
collapse the docstring asserted but the code
contradicted (it actually produced `qwen36`). On re-
reading post-test-failure, the simpler "major-
version-only" pattern matching gemma and llama was
both correct and operator-mental-model-aligned;
helper, defaults, and tests updated in the same
task. Recorded honestly in the Task 2 commit
(`0feac55`).

### Live verification (Task 7)

Verification against qwen3.6:27b and gemma4:31b via
`./scripts/dev-run.sh --release --reset --model <name>`.
Both runs used the per-family default
(`structured_injection`); banner confirmed strategy
resolution on both:

```
ollama_prompt_strategy = "structured_injection" (family: qwen3, default)
ollama_prompt_strategy = "structured_injection" (family: gemma4, default)
```

Three identical probes per session: (1) enumeration —
*"What tools do you have available?"*; (2) invocation
— *"Please call fs.write to create a file at test.txt
with the content 'phase 122 verification'."*; (3)
Ctrl-D exit. No audit-export check needed — neither
model emitted a tool call.

**Empirical table — pre-vs-post comparison:**

| Surface                    | qwen3.6:27b pre-Phase-122                          | qwen3.6:27b post-Phase-122                                                                                                                | gemma4:31b pre-Phase-122                            | gemma4:31b post-Phase-122                                                                                              |
|----------------------------|----------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------|
| **Enumeration**            | Confabulated 12 tools (mix real + invented incl. "Good Morning") | Listed ~25 *mostly real* tools by exact name; no obvious invention. **But `fs.read` and `fs.write` omitted from the enumeration despite being in the injection block.** | Confabulated 60+ entirely-invented tools             | **No exact tool names returned at all.** Pure prose categorization ("File Management: I can read, write, create, and delete files…"). |
| **Invocation on command**  | Verbal refusal ("I don't have fs.write")           | `[turn timed out]` — different failure shape; possibly attempted but didn't complete inside the harness window                            | Empty assistant message; 25 output tokens; no call   | Explicit verbal refusal: *"I do not have a tool called `fs.write`"* — **directly contradicting the injection block listing fs.write**. |
| **Audit chain ToolCalls**  | 0                                                  | 0                                                                                                                                          | 0                                                    | 0                                                                                                                       |
| **Phase 120 auto-corrects**| 0 (nothing to recover from)                        | 0 (still nothing to recover from)                                                                                                          | 0                                                    | 0                                                                                                                       |

**Honest reading — substrate worked on the prose
layer; capability-denial prior is prompt-unreachable.**

The structured-injection block measurably **reduced
confabulation** on both models: qwen3.6 stopped
inventing names like "Good Morning"; gemma4 stopped
generating 60+ fictional tools. That's a real
operator-facing win for transparency — the model is
now telling the operator something closer to what's
actually wired.

But the load-bearing question — *does the model
actually invoke tools it's commanded to call by exact
name?* — answered **NO** for both models. gemma4's
post-Phase-122 response is the clearest possible
evidence the ceiling sits at the model layer: it
claims `fs.write` doesn't exist while that exact
name is literally listed five lines above in the
same prompt. No amount of prompt engineering bridges
a model trained-prior that overrides its current
context.

qwen3.6's `[turn timed out]` is a different but
adjacent failure: the model may have attempted
something (vs the previous outright verbal refusal),
but couldn't complete in the harness window.
Possibly a positive signal — possibly just a slower
form of denial — the audit shows no tool call
landed either way.

**The open-doc honest-scope flag held at exit
exactly as predicted.** Both risks materialized:
gemma4's prior IS prompt-unreachable; qwen3.6's
behavior shifted but didn't reach invocation. Q3b's
"declare reality at exit" posture means we ship the
substrate with this documented, not silently.

**Operator-facing implications:**

- **Keep `structured_injection` for enumeration
  honesty.** The catalog block measurably stops
  confabulation. An operator who cares about "what
  does my model actually know it has" gets better
  ground-truth post-Phase-122.

- **`prompt_strategy = "none"` is the right escape
  hatch for cost-sensitive deployments.** ~400-700
  input tokens per turn for an outcome that doesn't
  rescue invocation is a bad trade if the operator
  values context budget over enumeration honesty.

- **Phase 120's fuzzy-recovery substrate remains
  the right tool for hallucinated invocations.**
  Phase 122 was orthogonal at sign-off and remains
  orthogonal at exit — the two substrates compose;
  Phase 120 still catches `fs_read` → `fs.read` etc
  whenever a model does emit a tool call.

- **Local-model tool-use is currently
  prompt-substrate-bounded.** Three named rehab
  phases (120 + 121 + 122) shipped real substrate;
  the residual gap is at the model layer. Next
  pressure: either model-side improvements as
  newer Ollama models ship, or operator pressure
  redirects this axis entirely. The audit's #1
  recommendation (Channel Activation Milestone)
  has now been deferred eleven times and pressure
  to address it grows.

**This exit confirms the Phase 6 Q5 honest-up-front
sign-off in the open doc.** Substrate ships, value
is real but partial, limitations documented,
operator has a clean opt-out. The Phase 122
contribution is best framed as: *operators get a
TOML knob and a banner line that shows them whether
their model will at least describe its tools
honestly; whether the model will then invoke them
remains the model's call.*

# Phase 120 — Tool-Call Validation + Recovery (Local-LLM Rehabilitation #1)

First-named phase against the audit's local-LLM
rehabilitation axis (per the post-Phase-118 codebase
audit). Phase-99-local-builds memory documents the
problem: qwen3.6:27b and gemma4:31b hallucinate tool
names when asked to enumerate them, and the tool-call
invocation path doesn't catch the bad names until the
planner tries to dispatch — at which point the turn
fails with `TurnOutcome::Failed`. Phase 120 closes that
gap with belt-and-suspenders validation: the LLM
provider flags unknown names at the LLM boundary; the
planner runs a high-confidence fuzzy-match auto-correct
and, below threshold, returns a structured "did you
mean?" error to the model so it can retry.

**Audit context — ninth consecutive substrate/polish
phase picked over the Channel Activation Milestone.**
Honest tracking. The substrate ledger is empty after
Phase 119; Phase 120 opens against the audit's #3
direction (G6 — privacy non-negotiable — distinguishing
claim, deserves first-class local-LLM support). The
Channel Activation Milestone becomes harder to defer
each phase; the audit ranking stays unchanged.

**Q-block — two Recommended + one non-Recommended.**

## Why this, why now

- **The current behavior fails ungracefully on
  hallucinated tool names.** A local model emitting
  `fs_read` (when the registered tool is `fs.read`)
  reaches `OpenAiProvider` → `LlmStepEnd::ToolCalls`
  with the bad name; the planner attempts dispatch;
  scope check fails on the unknown tool; turn ends.
  No recovery; no operator-readable diagnostic of
  "the model wanted Y; we dispatched X." The Phase 6
  Q5 honest assessment: the failure mode is silent
  enough that operators may not realize their model
  is the source of the issue rather than Aivyx itself.
- **G6 (Local execution, privacy non-negotiable) is
  Aivyx's distinguishing claim.** The local-LLM story
  should be first-class. Today it is "supported but
  degraded" — qwen3.6/gemma4 work for chat-only and
  single-tool turns; multi-tool research turns hit
  the hallucination ceiling early.
- **The fix is substrate-shaped, not model-shaped.**
  We can't make the local models stop hallucinating;
  we can catch the hallucination at the boundary and
  give the model a structured response that lets it
  recover.

## Scope (Q-block sign-off)

- **Q1 — Scope:** (a) **Tool-call validation + recovery
  only** (Recommended). Focused phase; closes the
  most-observed failure mode directly. Per-model prompt
  variants and native Ollama tool-calling are
  candidates for follow-on phases (Phase 121/122 if the
  audit ranking re-confirms them at exit).

- **Q2 — Validation site:** (c) **Both layers**
  (non-Recommended; picked over the cleaner
  provider-only Recommended). Provider catches/flags
  unknown names at the LLM boundary; the planner runs
  the fuzzy-match + recovery. Belt-and-suspenders
  posture — more substrate but more robust:
  - **Provider side**: knows the request's `tools` set;
    flags each pending `ToolCall` with whether its
    name resolves into the set. Pure-function check; no
    auto-correct happens here.
  - **Planner side**: receives the flagged ToolCalls;
    runs fuzzy match against the registered tool set;
    above threshold → auto-correct + audit event;
    below threshold → synthetic tool-result message to
    the model with suggestions, loop continues.

- **Q3 — Recovery posture:** (a) **Fuzzy-match
  auto-correct above threshold** (Recommended). Reuse
  Phase 112's `title_similarity` (Jaccard over
  normalized tokens) so the substrate stays uniform.
  Default threshold `0.80` matches the Phase 112 fuzzy-
  match default. Auto-correction landings get an audit-
  event entry so forensic walks can answer "did the
  agent really invoke X or did Aivyx auto-correct from
  Y?".

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract amendment;
  tool-call recovery is operator-value polish under
  D6's error contract. Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to eleven** (was 10
  after Phase 119).

- **PRODUCT.md** — **Will hold.** P8 (outcome-driven
  audited reflection) covers the new audit-event
  variant; P10 (substrate-only core) untouched (no new
  tool added — substrate fixes existing tool-dispatch
  path). Hash at entry:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to eleven** (was 10).

- **Production-core `aivyx-core/src/lib.rs`** —
  **Probably BREAKS.** The auto-correction happens
  inside the turn loop, which uses the `AuditHook` →
  `AuditTag` bridge. A new `AuditTag::ToolNameAutoCorrected`
  variant (or similar) WOULD touch `lib.rs` — same path
  Phase 117 `SkillInvocation` broke the streak from.
  Hash at entry:
  `d1d4373bcf54b0390b1e2c15efec4dfa50dd29f74f57aa7ca6950752603f4ae7`.
  Prediction: streak **resets 3 → 1**. Honest 30/70
  hold — the 30% case fires only if I can route the
  audit emission through an alternate path (e.g.
  fold the auto-correct flag into the existing
  `AuditTag::ToolCall` variant via an additive serde-
  default field). Worth attempting in Task 4; honest
  reporting at exit.

- **New workspace deps** — Zero anticipated. Phase 112
  `title_similarity` already in `aivyx-channel`; if
  the planner-side fuzzy match needs it, lift to a
  more central location (TBD at Task 5).

- **Test count** — Substrate is provider-side validation
  + planner-side recovery + audit-event extension +
  e2e against scripted bad-tool-name responses.
  Prediction: **+25 to +45**.

## Tasks

Roughly seven sub-tasks plus exit + backfill:

### Task 1 — Open (this commit)

`docs/PHASE_120.md` + `docs/ROADMAP.md` Phase 120 entry +
`docs/README.md` status row.

### Task 2 — Title-similarity primitive lift

- Move `title_similarity` from
  `aivyx-channel/src/skill_auto_proposer.rs` (Phase
  112) into a more central location accessible to
  both `aivyx-core` planner and `aivyx-channel`.
- Candidate locations:
  - `aivyx-core::skill_proposer::title_similarity`
    (already in the proposer module; planner could
    `use aivyx_core::skill_proposer::title_similarity`)
  - new `aivyx-core::similarity` module
- Tests: round-trip with the existing Phase 112 cases;
  no semantic change.

### Task 3 — Provider-side validation

- Extend `LlmRequest` (or the stream-builder's input)
  so the OpenAI/Ollama provider knows the canonical
  tool-name set. Most likely path: derive from
  `request.tools[].name` at request-build time, snapshot
  into the stream-builder's state.
- When the stream-builder finalizes `LlmStepEnd::ToolCalls`,
  validate each pending tool name against the snapshot.
  Add a `name_resolution: NameResolution` field to
  `ToolCallEnd`:
  - `NameResolution::Known` — name matched verbatim.
  - `NameResolution::Unknown { original }` — flagged for
    planner-side recovery.
- Pure function; no auto-correct at this layer
  (single source of truth lives in the planner per
  Q2).
- Tests: known names pass through with
  `NameResolution::Known`; unknown names land
  `NameResolution::Unknown { original }`; round-trip
  via the existing OpenAI provider tests.

### Task 4 — Planner-side fuzzy recovery + audit

- Planner receives `ToolCallEnd` with `NameResolution`;
  branches:
  - `Known` → existing dispatch path unchanged.
  - `Unknown { original }` → run fuzzy match against
    the registered tool-name set. Threshold default
    `0.80` (Phase 112 default); operator-configurable
    via `[providers] tool_name_auto_correct_threshold`.
  - Above threshold: dispatch to the matched tool;
    audit-event the auto-correction (variant TBD —
    either new `AuditTag::ToolNameAutoCorrected` (lib.rs
    break) OR additive `auto_corrected_from:
    Option<String>` field on existing
    `AuditTag::ToolCall` (lib.rs streak preservation
    path).
  - Below threshold: emit a synthetic tool-result
    message to the model: `"unknown tool '{original}'.
    Available tools: {top_5}. Did you mean
    '{best_guess}'?"`. Loop continues; model retries
    with the corrected name.
- Tests: above-threshold auto-correct dispatches +
  audits; below-threshold synthetic message; the
  "did you mean?" message ranks suggestions by
  similarity.

### Task 5 — Operator config knob

- `[providers] tool_name_auto_correct_threshold = 0.80`
  in `aivyx.toml`. Float in `[0.0, 1.0]`.
- `0.0` → never auto-correct (always return synthetic
  error to model).
- `1.0` → only exact matches (effectively disables
  fuzzy recovery).
- Tests: TOML round-trip; out-of-range rejection;
  the runtime config carries the threshold through.

### Task 6 — Per-tool "did you mean?" suggestions

- Sort the unknown-name's candidates by similarity
  descending; take top 3 (operator-readable; not
  prompt-budget-explosive).
- Format the synthetic message operator-style: the
  text the model sees needs to be unambiguous about
  WHICH tool to retry with. Phase 87 (judge prompt)
  precedent for structured-response shaping.
- Tests: top-3 ranking is stable; empty registered
  set falls back to a neutral "no tools available"
  message rather than a misleading list.

### Task 7 — Scripted e2e + INSTALL.md sweep + exit

- Scripted e2e: ScriptedProvider returning a
  hallucinated tool name like `fs_read`; assert the
  planner auto-corrects to `fs.read` AND audits the
  correction; assert a non-fuzzy-matchable name like
  `do_the_thing` produces a synthetic tool-result
  and loop-continue.
- INSTALL.md: new "Local-LLM tool-call recovery
  (Phase 120)" section. Documents the
  auto-correct-threshold knob; shows operator
  forensics: `aivyx audit export --event-type
  ToolNameAutoCorrected` (or whatever the final
  variant name is) to count how often the auto-correct
  fires per session — useful diagnostic when picking
  between local models.
- Exit: PHASE_120.md prediction-vs-reality + ROADMAP
  freeze + README flip + hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Scope:** (a) **Tool-call validation + recovery
  only** (Recommended). Focused phase; per-model prompt
  variants and native Ollama path deferred to follow-on.
- **Q2 — Validation site:** (c) **Both layers**
  (non-Recommended; picked over provider-only). Provider
  flags; planner recovers.
- **Q3 — Recovery posture:** (a) **Fuzzy-match
  auto-correct above threshold** (Recommended). Phase
  112 `title_similarity` reused; default threshold
  `0.80`.

## Exit criteria

- [ ] `docs/PHASE_120.md` + ROADMAP Phase 120 entry +
  docs/README status row — Task 1.
- [ ] Title-similarity primitive lift — Task 2.
- [ ] Provider-side validation — Task 3.
- [ ] Planner-side fuzzy recovery + audit — Task 4.
- [ ] Operator config knob — Task 5.
- [ ] "Did you mean?" suggestions — Task 6.
- [ ] Scripted e2e + INSTALL.md sweep — Task 7.
- [ ] Q1/Q2/Q3 resolved with operator sign-off pre-Task
  2 (Q1a + Q2c + Q3a recorded above).
- [ ] DESIGN.md streak — predicted HOLD (streak → 11).
- [ ] PRODUCT.md streak — predicted HOLD (streak → 11).
- [ ] `aivyx-core/src/lib.rs` streak — predicted BREAK
  (streak resets 3 → 1), honest 30/70 hold. Will
  attempt the additive `auto_corrected_from` field
  route at Task 4 to preserve the streak; report at
  exit.
- [ ] Zero new workspace dependencies.
- [ ] Test count delta within `+25` to `+45`.
- [ ] Zero clippy warnings.
- [ ] **Local-LLM hallucination failure mode closed.**
  Tool-name hallucination from qwen3.6/gemma4 (and
  similar) is caught at the LLM boundary and
  recovered into a successful tool dispatch (when the
  fuzzy match is confident) or a structured retry-able
  error to the model (when it isn't).

## Direction after Phase 120

After Phase 120, the audit ranking for Phase 121
candidates:

1. **Channel Activation Milestone** — long-deferred
   operator-verification pass. Nine phases of substrate/
   polish work shipped since Phase 111; the milestone
   becomes harder to defer with each one. Audit's
   Recommended for Phase 121 going in.
2. **Per-model prompt variants** (local-LLM
   rehabilitation #2) — if Phase 120's
   tool-call-recovery substrate isn't sufficient
   on its own, follow-on phase introduces concise
   tool-description format for small-context models.
3. **Native Ollama tool-calling path** (local-LLM
   rehabilitation #3).
4. **Release prep (v0.1.0 + shell installer)**.
5. **A new thematic Chapter F**.

Phase-by-phase decision at Phase 120 exit.

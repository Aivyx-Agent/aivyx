# Phase 137 — Voice Agent Feature Parity with Local

**Phase 136 follow-on.** Phase 135 shipped the voice
substrate; Phase 136 wired the audio I/O loop end-to-
end. But the voice arm in the binary still
constructs a *minimal* `ConcreteAgent` — no role
overrides, no recall context, no memory prune sinks,
no prompt refresher. The Local channel's
`run_session` carries all those features. Phase 137
closes the parity gap by extracting the agent-stack-
construction logic into a reusable helper.

## Why this, why now

- **Voice is currently second-class.** Operators who
  switch from `--channel local` to `--channel voice`
  lose the rich Local-channel features (Phase 60
  Persona refresh, Phase 76 auto-recall, Phase 79
  adaptive Persona, Phase 117 tool/skill relevance
  refiner). The substrate is identical — only the
  binary-layer wiring is missing.

- **Smallest scope possible.** The agent-stack
  construction lives in lines 218-284 of
  `aivyx-channel/src/session.rs` — ~67 lines that
  already encapsulate every feature voice is
  missing. Extracting it into
  `build_agent_stack(provider, audit, spec)` lets
  both `run_session` and the voice dispatch arm
  reuse one code path.

- **Sets up Phase 138+ channels.** Any future
  channel adapter (web, REST, etc.) inherits the
  same feature set automatically by calling
  `build_agent_stack`. Substrate hygiene.

## Tasks

1. **Open doc + ROADMAP + README** — this doc + the
   roadmap section + the README row. Backfill
   Phase 136 hash to `0157ce0`.

2. **Extract `build_agent_stack` from `run_session`.**
   New public function in `aivyx-channel/src/session.rs`:
   ```rust
   pub fn build_agent_stack(
       provider: Arc<dyn LlmProvider>,
       audit: Arc<dyn AuditHook>,
       spec: AgentStackSpec,
   ) -> Arc<dyn Agent>
   ```
   `AgentStackSpec` carries the agent-relevant
   fields from `SessionConfig` (model, system_prompt,
   max_tokens, capabilities, tools, tool_allowlist,
   memory_topic_prefix, role_overrides,
   prompt_refresher, context_window_tokens,
   prune_sink, context_provider,
   system_prompt_refiner). `run_session` converts
   its `SessionConfig` into a spec and delegates.
   Pure refactor; every existing Local-channel test
   continues to pass.

3. **Wire voice arm to `build_agent_stack`.**
   Replace the Phase 136 inline `ConcreteAgent`
   construction in the binary's `ChannelKind::Voice`
   arm with a call to `build_agent_stack`. Threads
   `role_overrides`, `prompt_refresher`,
   `context_window_tokens`, `prune_sink`,
   `context_provider`, `system_prompt_refiner` from
   upstream binary state — the same values the
   `ChannelKind::Local` arm uses.

4. **INSTALL update + exit doc + Frozen.** Update
   the INSTALL voice section to reflect feature
   parity. Phase 137 exit doc with prediction-vs-
   reality. Flip Frozen.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment. Streak: 27 → **28**.
- **PRODUCT.md** — **Will hold.** Voice feature
  parity reinforces the existing AI-personal-
  assistant framing. Streak: 27 → **28**.
- **`aivyx-core/src/lib.rs`** — **Will hold.** All
  Phase 137 work in `aivyx-channel/src/session.rs`
  + the binary. Core untouched. Streak:
  2 → **3**.

## Exit criteria

- [ ] `docs/PHASE_137.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `build_agent_stack` + `AgentStackSpec` public
  in `aivyx-channel/src/session.rs` — Task 2.
- [ ] Voice dispatch arm calls `build_agent_stack`
  with the rich feature set — Task 3.
- [ ] Every existing Local-channel test continues
  to pass — Task 2 regression boundary.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD as
  predicted.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings with default features.
- [ ] Test count delta: `+1` to `+5` (mostly the
  pure refactor doesn't add tests; new tests if
  Task 2 surfaces a regression that wasn't
  previously covered).

## Honest scope risks at sign-off

- **Refactor risk in `run_session`.** The extraction
  reshuffles ~67 lines of agent-stack-construction
  code. Local-channel tests are the regression
  boundary; if any break we know we lost a
  behaviour. The Phase 136 push-to-talk loop's
  unit-tested stub agents continue to use a
  manually-constructed ConcreteAgent (the
  substrate seam doesn't change), so the voice
  loop tests are unaffected.

- **Voice feature-parity is plumbing, not
  validation.** The new fields (role overrides,
  recall context, etc.) are wired through but
  their interaction with voice-specific UX —
  e.g. does the operator hear when the recall
  context surfaces a related memory? — needs
  operator validation. Documented in INSTALL.md.

- **`build_agent_stack` is a one-off helper, not
  a substrate-tier lift.** If Phase 138+ ships
  more channel adapters (web, REST, etc.) we
  may iterate the signature. The function is
  pub at this phase but its shape is not yet a
  locked contract.

- **Twenty-sixth consecutive deferral of the
  Channel Activation Milestone.** Per operator
  framing — intentional hold. Tracking continues.

## Direction after Phase 137

After Phase 137, voice and Local are
feature-equivalent. Phase 138+ candidates:

1. **Streaming TTS during LLM generation.** Pipeline
   text chunks from the planner into the TTS engine
   on sentence boundaries — operator hears the
   first sentence while the LLM is still generating.
2. **Voice activity detection** for trim-on-silence
   push-to-talk (no more "press Enter twice"). Adds
   the `silero` MIT/Apache crate.
3. **Wake-word activation** ("Hey Aivyx") via
   Porcupine or Silero-wakeword.
4. **Multimodal output** — agent speaks descriptions
   of images via vision-capable LLMs.
5. **whisper-cpp-plus rehabilitation** — close out
   the Phase 135 Q2c deferral.
6. **Channel Activation Milestone** — still held
   intentionally; 26th consecutive deferral at
   Phase 137 exit.

## Prediction vs reality

**Three-of-three streak HOLDs as predicted.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment. Streak:
  27 → **28**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Streak: 27 → **28**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 137 work in
  `aivyx-channel/src/session.rs` (extraction) +
  `aivyx-channel/src/bin/aivyx.rs` (voice arm
  rewire). Continuing post-Phase-135 reset:
  2 → **3**.

**Test count delta: 0 — below predicted `+1` to
`+5` range.** Workspace lib tests stayed at 3026.
Honest framing: the refactor is pure (no new code
paths, no surprising edge cases surfaced); all 734
aivyx-channel lib tests continued to pass under
the lift. Behaviour preservation was the
correctness bar and the regression-guard tests
already covered it.

**Zero new workspace dependencies** as predicted.

**Zero clippy warnings** with default features.

### What landed cleanly + what bent

**Cleanly:**
- `AgentStackSpec` + `build_agent_stack` public in
  `aivyx-channel::session`. 67 lines of inline
  construction lifted into a reusable helper.
- `AgentStackSpec::from_session_config` bridge so
  `run_session` materializes a spec from its
  existing config and delegates.
- Voice arm rewrite: inline minimal ConcreteAgent
  → full `build_agent_stack` call with the same
  6 rich features Local uses (role overrides,
  prompt refresher, context window, prune sink,
  recall context, system prompt refiner).
- 734 aivyx-channel lib tests still pass — the
  refactor is regression-free under the existing
  coverage.

**Bent honestly:**

1. **Prompt-refresher closure duplicated between
   Local and Voice arms.** Both arms construct
   the same ~20-line closure independently
   (refresher_profile + refresher_role_name +
   refresher_role_prompt + refresher_shared +
   refresher_catalog → assemble_session_prompt →
   apply_ollama_prompt_strategy). A Phase 138+
   helper could DRY this; the duplication is
   acceptable for one channel adapter pair.

2. **`build_agent_stack` is one-off helper.** Not
   substrate-tier-locked. If Phase 138+ ships
   more channel adapters (web, REST), we may
   iterate the signature. Documented in the open
   doc's honest-risk section.

3. **Voice feature-parity is plumbing, not UX
   validation.** The features now flow through;
   whether the operator hears auto-recall
   surfacing a related memory in a useful way is
   operator UX testing — not Phase 137 work.

### Direction after Phase 137

Voice and Local are now functionally equivalent.
Phase 138+ candidates:

1. **Streaming TTS during LLM generation.** Pipe
   text chunks from the planner into the TTS
   engine on sentence boundaries so the operator
   hears the first sentence while the LLM is
   still generating. Significant latency win.
2. **Voice activity detection** for trim-on-
   silence push-to-talk (no more "press Enter
   twice"). Adds the `silero` MIT/Apache crate.
3. **Wake-word activation** ("Hey Aivyx") via
   Porcupine or Silero-wakeword.
4. **Multimodal output** — agent speaks
   descriptions of images via vision-capable
   LLMs.
5. **whisper-cpp-plus rehabilitation** — close out
   the Phase 135 Q2c deferral.
6. **`build_agent_stack` substrate-tier promotion**
   if Phase 138+ ships more channel adapters and
   the helper's signature stabilizes across
   them. Pure-substrate work; small scope.
7. **Channel Activation Milestone** — still held
   intentionally; 26th consecutive deferral at
   Phase 137 exit.

# Phase 43 — Context Window Management

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../../../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Prevent the agent from exceeding LLM context limits during long
multi-step turns by adding conversation history pruning to the
planner. A turn with many tool calls accumulates history entries
— tool calls, tool results, assistant continuations — that can
push past the provider's context window (200k for Anthropic,
128k for OpenAI). Without pruning, the turn fails with a
provider error mid-step.

The pruning layer sits inside `LlmPlanner::next_step`: before
building each `LlmRequest`, it estimates token count and, if
over budget, drops the oldest messages while preserving the
system prompt and recent context. Pruned content is optionally
persisted to memory so it's recoverable via `memory.read`.

## Why now

1. **Long missions hit limits first.** Autonomous scheduled
   turns (G5) and multi-step mission chains (P2) accumulate
   the most history. Without pruning, the daemon's most
   valuable use cases are the most fragile.

2. **Memory GC just landed.** Phase 42's `gc_topic` and
   `gc_expired` provide the substrate for bounded pruning
   persistence — pruned context written to memory will be
   GC'd naturally by the TTL timer.

3. **Token usage reporting.** Phase 31 added `TokenUsage` to
   `TurnEnded` audit events. Extending it with pruning metrics
   lets operators observe context pressure without adding a new
   event type.

## Entry baseline

- Tests: 839
- Clippy warnings: 0
- Deferral backlog: 0
- DESIGN.md streak: 1 phase (touched in Phase 41 for A7)
- PRODUCT.md streak: 6 phases (untouched since Phase 38)
- lib.rs streak: 3 phases (untouched since Phase 40)

## Streak predictions

| Streak target | Predicted | Actual | Notes |
|---|---|---|---|
| DESIGN.md | untouched (2) | untouched (2) | No new protocol or contract changes |
| PRODUCT.md | untouched (7) | untouched (7) | Internal planner improvement |
| lib.rs | **touched** | **touched** | TokenUsage fields added (Task 5) |

## Product commitment coverage

- **G3 (Memory Reflection):** Pruned context persisted to memory
  makes long-turn history recoverable.
- **G5 (Autonomous Execution):** Long autonomous missions are
  the primary beneficiary — they hit context limits first.

## Tasks

### Task 1 — Open commit + PHASE_43.md scaffold

This file.

**Ship:** `d603ac6` 2026-04-21

### Task 2 — Token counting in aivyx-llm

Add `estimate_tokens(messages: &[LlmMessage]) -> usize` to
`aivyx-llm`. Simple `chars.div_ceil(4)` heuristic (zero new deps).
Add `estimate_system_tokens(system: Option<&str>) -> usize`.
Add `context_window_tokens` field to `LlmPlannerConfig` with
per-provider defaults via `ProviderKind::default_context_window()`
(200_000 for Anthropic, 128_000 for OpenAI, 8_000 for Ollama).

**Files:** `crates/aivyx-llm/src/lib.rs`,
`crates/aivyx-core/src/llm_planner.rs`,
`crates/aivyx-config/src/lib.rs`

**Ship:** `c193210` 2026-04-21

### Task 3 — History pruning in LlmPlanner

In `LlmPlanner::next_step`, before each LLM call, check token
estimate against 80% of `context_window_tokens`. If over budget,
prune oldest messages using a grow-tail-backwards algorithm,
replace with a `[Earlier context pruned: N messages]` sentinel
user message. The most-recent message is always preserved.

Config wiring at all 3 planner construction sites (child agent,
daemon-run, session) plus `SessionConfig::context_window_tokens`.

**Files:** `crates/aivyx-core/src/llm_planner.rs`,
`crates/aivyx-channel/src/session.rs`,
`crates/aivyx-channel/src/bin/aivyx.rs`,
`crates/aivyx-config/src/lib.rs`,
5 test files updated for new `SessionConfig` field.

**Ship:** `2159d38` 2026-04-21

### Task 4 — Pruning-to-memory bridge

`PruneSink` trait defined in `aivyx-core` (dependency inversion —
core can't depend on `aivyx-memory`). `MemoryPruneSink` implementation
in `aivyx-channel` backed by `Memory::put()`. Persists truncated
summaries to `context:pruned:<session_id>` memory topic.

Wired at all 3 planner construction sites in the binary.
`SessionConfig` gains optional `prune_sink` field.

**Files:** `crates/aivyx-core/src/llm_planner.rs`,
`crates/aivyx-core/src/lib.rs`,
`crates/aivyx-channel/src/prune_sink.rs` (new),
`crates/aivyx-channel/src/lib.rs`,
`crates/aivyx-channel/src/session.rs`,
`crates/aivyx-channel/src/bin/aivyx.rs`

**Ship:** `ec8d27f` 2026-04-21

### Task 5 — TokenUsage reporting extension

Add `context_tokens_before_pruning` and
`context_tokens_after_pruning` fields to `TokenUsage`.
Populated by the planner when pruning fires. Both `#[serde(default)]`
for backwards-compatible deserialization. **Breaks the lib.rs streak.**

**Files:** `crates/aivyx-core/src/lib.rs`,
`crates/aivyx-core/src/llm_planner.rs`

**Ship:** `9226cbf` 2026-04-21

### Task 6 — Exit freeze

Tests, streak report, `docs/ROADMAP.md` rollover,
`docs/README.md` phase table update, ship records and
exit criteria.

## Decisions

1. **80% threshold, not configurable.** The 80% budget leaves
   room for the LLM's response tokens. Making this configurable
   adds complexity for a value that rarely needs tuning.

2. **Dependency inversion for PruneSink.** The `aivyx-core`
   crate defines the `PruneSink` trait; `aivyx-channel`
   implements it with `MemoryPruneSink`. This avoids a
   core → memory dependency cycle.

3. **No persist_pruned_context config flag.** The plan called
   for a `bool` flag defaulting to `true`. Since the default
   is always-on and the feature has no user-facing cost, the
   sink is wired unconditionally when memory is available.
   An opt-out can be added later if needed.

4. **`chars.div_ceil(4)` token heuristic.** Overestimates for
   pure ASCII (where BPE typically yields ~3.5 chars/token),
   roughly accurate for mixed English. Zero new dependencies
   vs. pulling a tokenizer library.

## Exit criteria

- [x] `estimate_tokens` function in aivyx-llm.
- [x] `context_window_tokens` field in `LlmPlannerConfig`.
- [x] History pruning activates when over 80% budget.
- [x] Pruned content persisted to memory via `PruneSink`.
- [x] `TokenUsage` extended with pruning metrics.
- [x] All tests pass with net-positive delta (839 → 857, +18).
- [x] Zero clippy warnings.
- [x] DESIGN.md untouched (streak → 2).
- [x] PRODUCT.md untouched (streak → 7).

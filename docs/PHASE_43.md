# Phase 43 — Context Window Management

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

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

| Streak target | Predicted | Notes |
|---|---|---|
| DESIGN.md | untouched (2) | No new protocol or contract changes |
| PRODUCT.md | untouched (7) | Internal planner improvement |
| lib.rs | **touched** | TokenUsage fields added (Task 5) |

## Product commitment coverage

- **G3 (Memory Reflection):** Pruned context persisted to memory
  makes long-turn history recoverable.
- **G5 (Autonomous Execution):** Long autonomous missions are
  the primary beneficiary — they hit context limits first.

## Tasks

### Task 1 — Open commit + PHASE_43.md scaffold

This file.

### Task 2 — Token counting in aivyx-llm

Add `estimate_tokens(messages: &[LlmMessage]) -> usize` to
`aivyx-llm`. Simple `chars / 4` heuristic (zero new deps).
Add `context_window_tokens` field to `LlmPlannerConfig` with
per-provider defaults (200_000 for Anthropic, 128_000 for
OpenAI, configurable for Ollama).

**Files:** `crates/aivyx-llm/src/lib.rs`,
`crates/aivyx-core/src/llm_planner.rs`

### Task 3 — History pruning in LlmPlanner

In `LlmPlanner::next_step`, before building `LlmRequest`,
check token estimate against 80% of `context_window_tokens`.
If over budget, prune oldest messages, replace with a
`[Earlier context pruned: N messages]` sentinel user message.
Keep system prompt and most-recent N messages intact.

**Files:** `crates/aivyx-core/src/llm_planner.rs`

### Task 4 — Pruning-to-memory bridge

When messages are pruned, persist a summary to
`context:pruned:<session_id>` memory topic. Controlled by
`persist_pruned_context: bool` config flag (default: true).
Requires passing optional `Arc<dyn Memory>` to `LlmPlanner`.

**Files:** `crates/aivyx-core/src/llm_planner.rs`,
`crates/aivyx-config/src/lib.rs`,
`crates/aivyx-channel/src/bin/aivyx.rs`

### Task 5 — TokenUsage reporting extension

Add `context_tokens_before_pruning` and
`context_tokens_after_pruning` fields to `TokenUsage` so
operators and the reflection loop can observe pruning
frequency. **This breaks the lib.rs streak.**

**Files:** `crates/aivyx-core/src/lib.rs`

### Task 6 — Exit freeze

Tests, streak report, `docs/ROADMAP.md` rollover,
`docs/README.md` phase table update, ship records and
exit criteria.

## Exit criteria

- [ ] `estimate_tokens` function in aivyx-llm.
- [ ] `context_window_tokens` field in `LlmPlannerConfig`.
- [ ] History pruning activates when over 80% budget.
- [ ] Pruned content persisted to memory (opt-in).
- [ ] `TokenUsage` extended with pruning metrics.
- [ ] All tests pass with net-positive delta.
- [ ] Zero clippy warnings.
- [ ] DESIGN.md untouched (streak → 2).
- [ ] PRODUCT.md untouched (streak → 7).

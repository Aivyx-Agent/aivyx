# Phase 40 — Parallel Tool Execution

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Resolve the deferred concurrency decision (D1 line 67) and
implement parallel tool dispatch. When an LLM response contains
multiple tool-use blocks, execute all tools concurrently via
`join_all` rather than sequentially. This removes the primary
throughput bottleneck in the turn loop.

## Why now

1. **Last deferred core decision.** D1 explicitly deferred
   "Concurrency model (sequential? parallel tools?)" at Phase 0.
   39 phases later, the turn loop is stable, audited, and
   well-tested. Time to resolve the deferral.

2. **Both providers support it.** Anthropic and OpenAI wire
   formats emit multiple tool-use blocks. The providers
   currently discard all but one — a mechanical limitation,
   not a design choice.

3. **`run_tool_call` is `&self`.** The method already takes a
   shared reference. No locking changes needed for concurrent
   execution.

## Architecture

Amendment A6 documents the full design. Key points:
- Batch of N tools = 1 step toward MAX_STEPS_PER_TURN
- `tool_calls_made` counts individual calls (sum of batch sizes)
- Escalation: complete all, observe all, then break
- Anthropic serializer merges consecutive ToolResult entries

## Streak predictions

| Streak target | Predicted | Notes |
|---|---|---|
| DESIGN.md | **touched** | Amendment A6 resolves deferred concurrency |
| PRODUCT.md | untouched (3) | Internal architecture change |
| lib.rs | **touched** | NextStep, turn loop in core |

## Tasks

### Task 1 — Open commit + PHASE_40.md + Amendment A6

This file + A6 amendment + doc updates.

### Task 2 — `LlmStepEnd` pluralization

Replace `LlmStepEnd::ToolCall` with `LlmStepEnd::ToolCalls`.
Update Anthropic and OpenAI providers to accumulate all
tool-use blocks.

### Task 3 — `NextStep::ToolCalls` variant

Add `ToolCallRequest` struct and `NextStep::ToolCalls(Vec<...>)`
to the planner module.

### Task 4 — `LlmPlanner` batch adaptation

Change `pending_call_id` to `pending_call_ids: VecDeque`.
Map multi-tool LLM responses to `NextStep::ToolCalls`.

### Task 5 — Anthropic tool_result grouping

Merge consecutive `ToolResult` entries into one user message
at serialization time.

### Task 6 — Turn loop parallel dispatch

Add `NextStep::ToolCalls` match arm with `join_all`. Refactor
existing `ToolCall` arm to delegate.

### Task 7 — Tests + clippy

Full test coverage for batch dispatch, provider accumulation,
history grouping, escalation-in-batch.

### Task 8 — Exit freeze + docs

## Exit criteria

- [ ] `LlmStepEnd::ToolCalls` surfaces all tool-use blocks from both providers.
- [ ] `NextStep::ToolCalls` variant exists and VecPlanner can use it.
- [ ] `LlmPlanner` maps multi-tool responses to batch NextStep.
- [ ] Turn loop dispatches batch tools via `join_all`.
- [ ] Anthropic serializer groups consecutive ToolResults correctly.
- [ ] Escalation-in-batch observed correctly (non-escalated first).
- [ ] All tests pass (801 + new).
- [ ] Zero clippy warnings.
- [ ] Amendment A6 filed and referenced in DESIGN.md.
- [ ] PRODUCT.md untouched (streak 3 from Phase 39).

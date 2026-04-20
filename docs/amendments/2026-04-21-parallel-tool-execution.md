# Amendment A6 — Parallel Tool Execution

**Date:** 2026-04-21
**Phase:** 40
**Supersedes:** Resolves the deferred concurrency decision in D1
(line 67: "Concurrency model (sequential? parallel tools?) —
deferred"). The turn loop now supports batch tool dispatch
when the LLM returns multiple tool-use blocks in a single
response.
**Implementing phase:** 40

---

## What changed

The turn loop in `ConcreteAgent::turn` gains a parallel
dispatch path. When the `TurnPlanner` returns
`NextStep::ToolCalls(Vec<ToolCallRequest>)`, the loop
executes all tool calls concurrently via
`futures::future::join_all`. Each tool still passes through
the same capability gate, audit logging, and stream-event
emission. The existing `NextStep::ToolCall` (singular)
continues to work unchanged.

---

## Design decisions

### Batch semantics

A batch of N tool calls counts as **1 step** toward
`MAX_STEPS_PER_TURN`. The step limit exists to catch
runaway planners, not to limit throughput. The
`tool_calls_made` counter in `TurnOutcome` still counts
individual calls (sum of batch sizes across all steps).

### Escalation in a batch

If any tool in a batch returns `RequiresEscalation`, all
tools in the batch run to completion (they were already
launched concurrently). Observations for non-escalated
tools are recorded to `observed` and fed to
`observe_tool_outcome` before the escalation breaks the
loop. This ensures the planner's conversation history
stays consistent.

### LLM history format

The `LlmMessage::Assistant { tool_calls: Vec<...> }`
variant already supports multiple tool calls per message.
For results, each tool's outcome is appended as a separate
`LlmMessage::ToolResult`. The Anthropic request serializer
merges consecutive `ToolResult` entries into a single
`role: "user"` message with multiple `tool_result` content
blocks (required by Anthropic's API). The OpenAI serializer
emits one `role: "tool"` message per result (as OpenAI
expects).

### Tool safety

Parallel execution trusts the LLM's judgment — it batched
the calls because it believes they are independent. This
is the same model used by Claude Code, ChatGPT, and other
production agents. Tools that have internal ordering
dependencies should not be batched by the LLM; if they are,
the results may be non-deterministic. This is a tool-usage
concern, not a framework concern.

---

## The amended rule

> **Tool execution concurrency.** The turn loop dispatches
> tool calls either sequentially (one at a time, as before)
> or in parallel (batch of N via `join_all`), depending on
> what the `TurnPlanner` returns. `NextStep::ToolCall`
> dispatches one tool. `NextStep::ToolCalls(Vec<...>)`
> dispatches all tools in the batch concurrently. A batch
> counts as one step toward `MAX_STEPS_PER_TURN`.
> `tool_calls_made` counts individual calls. The capability
> gate, audit chain, and stream events fire per-tool
> regardless of dispatch mode.

---

## What this does not change

- `MAX_STEPS_PER_TURN` (32) and `TURN_TIMEOUT` (120s) are unchanged.
- The `Agent` trait signature is unchanged.
- The `Tool` trait signature is unchanged.
- The `ChannelContext` streaming protocol is unchanged.
- The `AuditTag::ToolCall` event fires once per tool, not once per batch.
- Single-tool responses (Vec of length 1) behave identically to the old `ToolCall` path.

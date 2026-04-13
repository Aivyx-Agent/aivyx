# Phase 2 — First Real LLM Provider (ACTIVE)

**Status:** Active — opened 2026-04-13
**Predecessor:** [PHASE_1.md](PHASE_1.md) (frozen 2026-04-13)
**Contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all LOCKED)

This is the active working doc for Phase 2. It is edited freely
during the phase and frozen at phase exit. Contract changes go
through the amendment process described in [README.md](README.md).

## Goal

Implement the `LlmProvider` trait (D3 / D6) with an **Anthropic
reference impl**, and replace Phase 1's `VecPlanner` with an
LLM-backed `TurnPlanner` so that a real turn actually consults a
model and the tool-calling loop runs against real completions.

The turn loop, capability system, and audit chain do **not** change.
If Phase 1's seams are right, Phase 2 is entirely about making
`aivyx-llm` real and wiring a second `TurnPlanner` impl behind the
same interface the Phase 1 tests validated. No new channels. No
new tools. No storage. No contract edits.

## Non-goals

- Ollama / local models. Deferred — see Open Questions below.
- A real `ChannelContext` impl. That's Phase 3's `LocalChannel`.
- Any concrete `Tool` impl beyond whatever is needed to drive a
  live tool-calling loop in tests (probably a built-in `echo` or
  `memory.read` fake — **not** a filesystem tool).
- Persistent storage. `aivyx-storage` stays stubbed through Phase 2;
  in-memory state is fine for the Phase 2 exit test.
- The `impl<T: AuditWriter> AuditHook for T` blanket bridge, if it
  isn't already in before Phase 2 opens the real `aivyx-audit` →
  `aivyx-core` pathway — in which case it lands in Phase 2 task 1.

## Entry criteria (inherited from Phase 1 exit)

- [x] Phase 1 frozen ([PHASE_1.md](PHASE_1.md))
- [x] 57-test suite green on `main`
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [x] `DESIGN.md` unchanged since Phase 0 exit (`1b4f271`)
- [x] `TurnPlanner` seam exists and is dyn-compatible
- [x] `ConcreteAgent` takes a planner factory, so a second impl can
      drop in without touching the loop body
- [ ] `AuditHook` bridge to `aivyx-audit` exists (deferred from
      Phase 1 — first thing to confirm/land in Phase 2 task 1)

## Known refinements queued from Phase 1

*None yet. This section will fill up as Phase 2 bumps into the
first real Anthropic request shape.*

## Phase 2 task list

*Tasks are added as the phase opens; empty entries are intentional.*

- [ ] **Task 1 — Audit bridge.** Add
      `impl<T: AuditWriter + ?Sized> AuditHook for T` (or a concrete
      adapter, if the blanket impl hits orphan rules) in `aivyx-audit`
      so `HmacChainLog` can be handed to `ConcreteAgent::new` directly.
      Verify by running a Phase 1-style test against a real
      `HmacChainLog` instead of `RecordingAudit`.
- [ ] **Task 2 — `LlmProvider` trait.** Implement the shape from D3
      in `aivyx-llm`: a streaming completion interface with tool
      calls, usage accounting, and cancellation. Dyn-compatible.
      Unit tests run against a fake transport.
- [ ] **Task 3 — Anthropic reference impl.** Concrete `LlmProvider`
      against the Anthropic Messages API. HTTP client + streaming
      parser + tool-call extraction. Secrets via env var or config
      file — not hard-coded. Covered by a transport-fake test; a
      real-API smoke test is **optional** and opt-in.
- [ ] **Task 4 — LLM-backed `TurnPlanner`.** A `TurnPlanner` impl
      that wraps an `LlmProvider`, maintains the conversation state,
      streams assistant text to the channel, and yields tool calls
      as they appear. Lives in `aivyx-llm` or `aivyx-core` depending
      on whether it needs LLM types in its signature.
- [ ] **Task 5 — End-to-end LLM-driven turn.** Replace the
      `VecPlanner` in one of the Phase 1 tests with the new LLM
      planner + a recorded/fake transport. Same assertions on the
      audit trail, same `TurnOutcome` shape — just driven by a
      (fake but realistic) model.
- [ ] **(Exit)** Document Phase 2 outcome + known issues in this
      file, freeze it, open `PHASE_3.md` (first real channel).

## Open questions

- **Q1 — Ollama reference impl alongside Anthropic?** Deferred from
  Phase 1 exit. Argument for: local models are where Aivyx's
  privacy story actually matters, so validating `LlmProvider` against
  two transports early forces the trait to not accidentally encode
  Anthropic-specific assumptions. Argument against: doubling the
  task 3 surface area on a phase that is already about getting the
  *first* model talking. **Resolution path:** land Anthropic first
  (task 3). Revisit at task 4: if `LlmProvider` looks suspiciously
  Anthropic-shaped, add Ollama as a same-phase second impl to force
  generality. Otherwise, defer to Phase 3+.
- **Q2 — Where does the LLM-backed planner live?** Two options: in
  `aivyx-core` alongside `VecPlanner` (keeps all planners in one
  place) or in `aivyx-llm` (keeps LLM types out of core). Decision
  at task 4 entry. Tentative preference: `aivyx-llm`, since the
  planner needs full LLM types in its constructor and core is
  already deliberately runtime-agnostic.
- **Q3 — Cancellation mid-stream.** The Phase 1 loop checks
  `CancellationToken` at the top of each step. An LLM turn may
  spend most of its time *inside* a single streaming completion
  call, between step boundaries. Does the LLM planner poll the
  token inside its stream loop? Does the HTTP client respect it?
  Probably both — flagged so task 3 and task 4 don't forget.

## Exit criteria (draft — revised once the task list stabilizes)

- [ ] `aivyx-llm` defines a dyn-compatible `LlmProvider` trait
      against the D3 shape
- [ ] One concrete `LlmProvider` impl exists (Anthropic)
- [ ] An LLM-backed `TurnPlanner` impl drives the same turn loop
      `VecPlanner` drove in Phase 1, with all Phase 1 tests' audit
      assertions still holding (modulo different derived scope
      strings)
- [ ] Tests run without hitting a real API by default; real-API
      smoke tests are opt-in
- [ ] `cargo test --workspace` green
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] `DESIGN.md` unchanged (or, if changed, an amendment file
      exists in `docs/amendments/` and is linked from this doc)

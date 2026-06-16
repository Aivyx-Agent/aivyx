# Phase 2 — First Real LLM Provider (FROZEN)

**Status:** Closed 2026-04-14
**Exit commit:** `2b6f876` — *"Phase 2 task 5: end-to-end LLM-driven turn integration test"*
**Predecessor:** [PHASE_1.md](PHASE_1.md) (exit commit `33012be`)
**Successor:** [PHASE_3.md](PHASE_3.md)
**Contract:** [`../DESIGN.md`](../../../DESIGN.md) (Deliverables 1–8, all LOCKED — unchanged)

This document is a historical record. The LLM-driven turn loop it
produced lives in `aivyx-llm` (new Anthropic reference provider),
`aivyx-core` (new `llm_planner` module, planner-seam extensions, and
the `MAX_STEPS_PER_TURN` guard), and `aivyx-audit` (end-to-end bridge
integration test). This file explains *how* it came together and what
was deliberately left for Phase 3.

## Goal (as written at phase entry)

Implement the `LlmProvider` trait (D3 / D6) with an **Anthropic
reference impl**, and replace Phase 1's `VecPlanner` with an
LLM-backed `TurnPlanner` so that a real turn actually consults a
model and the tool-calling loop runs against real completions. No
contract edits. No new channels. No new concrete tools beyond a
minimal fake for the exit integration test.

## What shipped

- **`aivyx-audit` → `aivyx-core` bridge.** The Phase 1 TODO blanket-
  impl is replaced by `AuditBridge<W: AuditWriter>`, a newtype adapter
  that implements `AuditHook` and translates the forward-declared
  `AuditTag` into real `AuditEvent`s. The adapter is the only public
  seam between the two crates and keeps `aivyx-core`'s forward-declared
  trait intact. A panic-on-error default + an explicit
  `on_audit_failure` callback seam means test code can wire any
  writer — recording fake, in-memory `HmacChainLog`, or on-disk log —
  without touching core.
- **`aivyx-llm` public trait surface.** `LlmProvider` is a dyn-
  compatible two-method interface: `chat_stream(req) -> LlmStream` +
  `name()`. `LlmStream` itself is another two-method trait
  (`next_event`, `finish`) so that implementors can own a boxed async
  stream without running into `async fn in dyn trait` object-safety
  issues. Tool-call shapes (`LlmToolDescriptor`, `LlmToolCall`),
  usage accounting (`LlmUsage`), and step-end variants (`FinalMessage`
  / `ToolCall`) all live in `crates/aivyx-llm/src/lib.rs`. **8 unit
  tests** against a hand-written fake provider.
- **`aivyx-llm::anthropic` reference provider** behind the
  `provider-anthropic` Cargo feature. The feature gates the entire
  `anthropic` submodule plus its transport/SSE/reqwest/rustls/secrecy
  dependency tail — default-feature `aivyx-llm` still compiles with
  zero HTTP stack. Three sub-modules:
  - **`transport.rs`** — `HttpTransport` trait + `ReqwestTransport`
    impl. The trait is the *only* way the provider talks to the
    network, which means every provider test runs against a
    `FakeTransport` replaying canned byte streams.
  - **`sse.rs`** — hand-rolled `SseReader` with LFLF and CRLFCRLF
    frame-terminator support (real Anthropic streams use LF; mixing
    CRLF in dev proxies was the bug that forced the second
    terminator). **6 unit tests** covering one-chunk, split-across-
    chunks, multiple-in-one, CRLF, comment lines, and trailing
    partial errors.
  - **`provider.rs`** — request-body builder, response state machine,
    and `AnthropicProvider::with_transport` constructor. State machine
    tracks `accumulated_text`, `usage`, `pending_tool`,
    `completed_tool`, and `stop_reason`, converting Anthropic
    `content_block_start` / `content_block_delta` / `message_delta` /
    `message_stop` events into `LlmStreamEvent::TextChunk` +
    `LlmStepEnd::{FinalMessage, ToolCall}`. **6 unit tests** plus an
    `#[ignore]` opt-in live smoke test that skips unless
    `ANTHROPIC_API_KEY` is set.
- **`aivyx-core::llm_planner`** — new module alongside the Phase 1
  `planner` seam. `LlmPlanner` is the `TurnPlanner` impl that wraps
  an `Arc<dyn LlmProvider>`, maintains an `LlmMessage` history across
  steps, streams `TextChunk` events to the channel as they arrive,
  and yields `NextStep::ToolCall` / `NextStep::FinalMessage` based on
  the provider's `LlmStepEnd`. Tool results are rendered as either
  verbatim JSON (success) or a structured envelope
  `{"error": "<kind>", "message": "<detail>"}` (denied / failed /
  requires_escalation). **7 unit tests** against an inline fake
  provider.
- **Extended `TurnPlanner` trait.** Two additive default methods
  added in `aivyx-core/src/planner.rs`:
  - `begin_turn(&mut self, msg: &Message)` — seed the history with
    the triggering user message. Default no-op preserves `VecPlanner`.
  - `observe_tool_outcome(&mut self, tool_id, outcome)` — feed the
    full `ToolOutcome` back to the planner after each dispatched
    tool call. Default no-op.
  - `next_step` now takes `channel: &dyn ChannelContext` so streaming
    planners can relay text mid-step. This is a breaking signature
    change for `VecPlanner` (updated in place) but every existing
    Phase 1 test passed unchanged because the Phase 1 tests never
    read the channel param.
- **`MAX_STEPS_PER_TURN` loop guard.** The Phase 1 loop would happily
  run forever if a planner kept emitting `ToolCall`. Phase 2 adds
  `pub const MAX_STEPS_PER_TURN: usize = 32;` and a
  `LoopOutcome::MaxStepsExceeded` internal variant that maps to
  `TurnOutcome::Failed(AivyxError::Internal(...))`. A runaway-
  planner regression test feeds 64 synthetic `ToolCall`s and asserts
  exactly 32 audited `ToolCall` events before the loop terminates.

**Totals:** 87 tests (default features and `--all-features`
identical), 1 ignored (the opt-in live-API smoke test).
`cargo test --workspace` green. `cargo clippy --workspace --all-
targets -- -D warnings` clean. `DESIGN.md` unchanged from Phase 0
exit — second phase in a row with no contract amendment.

## Refinements landed

*None. Phase 2's refinements were all inside the Phase 1 seams*
*(planner trait, loop guard, audit bridge) — none of them touched*
*the Phase 0 contract shapes. The refinement queue stayed empty*
*the whole phase.*

## Decisions made during Phase 2 that aren't in DESIGN.md

### `LlmStream` as a two-method trait, not `async fn` in a trait

`LlmProvider::chat_stream` returns `Box<dyn LlmStream + Send>`, and
`LlmStream` itself has `next_event(&mut self) -> Option<...>` plus
`finish(self: Box<Self>) -> LlmStepEnd`. The obvious alternative —
`async fn chat_stream(&self, req) -> impl Stream + ...` — is not
dyn-compatible without `async-trait` machinery that still forces
allocation, and the resulting trait is harder to implement for an
HTTP streamer that needs per-chunk state. The two-method shape is
uglier at the call site but keeps the trait dyn-safe without any
trait-object-of-stream acrobatics, and the planner consumes it from
a single small loop.

### Feature-gating the Anthropic provider

`aivyx-llm` has two personalities: a tiny dependency-light trait crate
(default features) and a full HTTP provider crate
(`--features provider-anthropic`). The split was deliberate because
`reqwest` + `rustls` + `secrecy` + `futures-util` + `bytes` is a
non-trivial dependency tail, and the *core* planner seam only needs
the trait — crates downstream of `aivyx-core` should be able to pull
in `aivyx-llm` for its types without pulling in a TLS stack. The
feature gate lives at `crates/aivyx-llm/Cargo.toml` and the gated
re-exports in `crates/aivyx-llm/src/lib.rs`. Every default-feature
test still passes with the feature off.

### Hand-rolled SSE parser instead of `eventsource-stream`

Considered `eventsource-stream` and `tokio-stream` SSE helpers but
both brought their own framing assumptions (and another dependency)
for a protocol that is ~40 lines of state machine. The hand-rolled
`SseReader` handles LFLF and CRLFCRLF terminators and treats everything
else as an error — deliberately strict. Tradeoff accepted: if
Anthropic ever ships a novel framing wrinkle we bear the maintenance
cost, but the debuggability wins (every failure is a type error in
`sse.rs`, not a "dep changed a default" regression) were decisive.

### `HttpTransport` seam, not "use `reqwest` directly"

The provider talks to `Box<dyn HttpTransport>`, and `ReqwestTransport`
is the only production impl. This adds one indirection layer on the
hot path but makes the offline test story trivial: every provider
test and every end-to-end integration test uses `FakeTransport` with
canned byte streams. The Phase 2 exit test drives four crates through
the same fake. Without the seam the E2E test would either need a
local HTTP server (slow, flaky, new infra) or a mocking library
(another dep). Seam was cheap; dropping it would hurt every future
provider test.

### `SecretString` for API keys

`AnthropicConfig` stores the API key as `secrecy::SecretString` rather
than `String`. Prevents accidental `Debug`-logging of the key through
a future `#[derive(Debug)]` on the config struct. `secrecy` is a tiny
crate and the ergonomic cost (`.expose_secret()` at the one call site
in the request builder) is a feature, not a bug — it turns key access
into something greppable.

### Model IDs as `String`, not a typed enum

`LlmPlannerConfig::new` takes `impl Into<String>` for the model
rather than an `AnthropicModel` enum. Two reasons: Anthropic ships
new model IDs roughly monthly and typed enums would need a crate
bump per release; and `model: "claude-haiku-4-5-20251001"` is the
exact string the API expects, so any translation layer is pure
bookkeeping. The provider crate does zero validation on the model
string — it's sent verbatim and errors come back from the API.

### Tool-result JSON envelope (success vs error)

`LlmPlanner::render_tool_result` emits two shapes:
- **Success:** the raw `output: Value` serialized verbatim. The LLM
  sees exactly the JSON the tool returned.
- **Denied / Failed / RequiresEscalation:** a structured envelope
  `{"error": "<kind>", "message": "<detail>"}` where `<kind>` is a
  stable short string (`"scope_denied"`, `"tool_error"`,
  `"requires_escalation"`).

The asymmetry is deliberate. Success payloads are tool-defined and
need to round-trip unmodified (a `memory.read` returning structured
items should feed the next planner step the same structure). Error
payloads are agent-defined and need a stable key (`error`) that a
prompt engineer can key retries/recovery on regardless of which tool
failed. The `unknown_tool` error path in `next_step` uses the same
envelope, so an LLM-authored retry loop sees one error schema across
every failure mode.

### LLM planner lives in `aivyx-core`, not `aivyx-llm`

Phase 2's Q2 at entry flagged this as an open question. Resolution
at task 4: `llm_planner.rs` lives in `aivyx-core` next to
`planner.rs`. The planner's *constructor* takes an
`Arc<dyn LlmProvider>` — which is a trait defined in `aivyx-llm` —
but since `aivyx-core` already depends on `aivyx-llm` for the trait,
moving the planner into `aivyx-llm` would only have bought the cost
of a second crate boundary without actually removing any dep edges.
Kept it in core. Future planners (Ollama, composite) will follow
the same pattern: trait in `aivyx-llm`, impl in `aivyx-core` (or
wherever the impl's ancillary types already live).

### `AuditBridge<W>` as a newtype, not a blanket impl

Phase 1's "TODO" suggested `impl<T: AuditWriter> AuditHook for T`,
but that runs into orphan rules the moment a downstream crate wants
to implement both traits on its own type. `AuditBridge<W>` is a
concrete generic wrapper `pub struct AuditBridge<W>(W, FailureHook)`
that implements `AuditHook` for *any* `W: AuditWriter`, and an
explicit `new(writer)` constructor means the conversion site is
grep-able. Also lets the bridge carry a configurable failure policy
(default: panic) that a blanket impl couldn't. Zero-cost at runtime;
strictly better ergonomics.

### `MAX_STEPS_PER_TURN = 32` (not a config knob)

Hardcoded `const`, not `LlmPlannerConfig` field. A 32-step turn is
already beyond any reasonable LLM tool-calling chain — the whole
point of the limit is to catch *buggy* planners, not to be tuned.
If a real workflow hits 32 steps the answer is "something is broken,
inspect the planner" not "raise the limit." Making it a config field
would normalize exceeding it, which is the exact thing the guard
exists to prevent. The value is revisitable later, in one place, if
a real workload proves 32 is wrong.

### `StepObservation` stays summary-only, even with the full-outcome hook

Phase 1 chose `StepObservation` to carry only a `ToolOutcomeSummary`
so planners couldn't branch on secret payload bytes. Phase 2's LLM
planner *does* need the full `ToolOutcome` (to render the payload
into the conversation history), but rather than widening
`StepObservation` we added `observe_tool_outcome(&mut self, tool_id,
&ToolOutcome)` as a separate hook. The loop calls it before asking
for the next step. The split means deterministic planners still
can't peek at payloads — only planners that explicitly override
the hook can, and the override makes the access visible.

## Bugs caught in Phase 2

- **`tokio::select!` on a dev-only dep.** First draft of
  `ReqwestTransport::post_sse` used `tokio::select!` to race the
  cancellation token against the request. But `tokio` was a
  *dev*-dep in `aivyx-llm`, not a prod dep, and the feature-gated
  cfg meant the select macro was live in production builds. Fixed
  by dropping the select in favor of a one-line `is_cancelled()`
  pre-check before the send — enough for the interruption semantics
  the turn loop actually needs, and with no new dep cost.
- **`find_double_newline` only matched LF.** SSE parser shipped
  handling `\n\n` only. I initially wrote a test documenting
  "doesn't handle CRLF" as a known limitation — then deleted the
  test because "document the bug" is a test smell, and extended the
  function to prefer `\r\n\r\n` (4 bytes) before falling back to
  `\n\n` (2 bytes). The returned `(usize, usize)` tuple is `(end,
  skip)` so the caller can slice correctly for either case.
- **`0.3f32` JSON round-trip.** A test for the request-body builder
  set `temperature: 0.3f32` and asserted the resulting JSON contained
  `"temperature": 0.3`. `serde_json` went through `f64` and produced
  `0.30000001192092896`, failing the string equality. Fixed by
  using `0.5f32` (exactly representable in binary32) in the test —
  real values also have this floating-point tail, but the test only
  needs to prove the field round-trips.
- **`dyn LlmStream` Debug constraint on test assertion.** A provider
  unit test used `provider.chat_stream(req).await.unwrap_err()`,
  which required the `Ok` side to implement `Debug`. `Box<dyn
  LlmStream>` does not. Fixed with an explicit `match` statement.
- **`ToolRegistryExt` over-abstraction.** Task 4 draft added a test-
  adjacent extension trait with `iter()` / `find_by_name()` on
  `ToolRegistry` because the planner needed both. Then realized the
  trait had exactly one caller and exactly one impl — classic
  abstraction-for-nothing. Deleted the trait, added inherent
  `iter_tools()` and `find_by_name()` methods directly. One fewer
  file, one fewer concept.
- **`CapabilitySet::default()`.** Several new tests used
  `Default::default()` on `CapabilitySet`. The type doesn't
  implement `Default`. Fixed with explicit
  `CapabilitySet::from_scopes([])` — probably worth adding `Default`
  for ergonomics in a future phase, but not in-scope for Phase 2.

## Decisions deferred to Phase 3

- **Real `ChannelContext` impl (`LocalChannel`).** The Phase 2 E2E
  test uses a `FakeChannel` that records streamed text into a
  `Mutex<Vec<String>>`. A real CLI channel is Phase 3's entire point.
- **Timeout enforcement.** Still untouched. `TurnOutcome::TimedOut`
  remains a public variant with no code path that emits it. Phase 2
  added `MAX_STEPS_PER_TURN` (step-count guard) but not a wall-clock
  deadline. The natural place for it is inside the LLM planner's
  stream loop, polling the cancellation token with a deadline — but
  no Phase 2 test needed it, and adding untested timeout code would
  be worse than leaving the gap visible.
- **Ollama / local model provider.** Q1 at phase entry asked whether
  to add a second provider alongside Anthropic to force trait
  generality. Resolution: `LlmProvider` did not look Anthropic-
  shaped by task 4, so the second provider was deferred. Phase 3+
  can add it as a one-file addition (new module under `aivyx-llm`,
  same trait).
- **`RequiresEscalation` propagation.** Still public, still unused.
  First tool that needs tier escalation will wire it.
- **Mid-stream cancellation in the provider.** The `ReqwestTransport`
  does a pre-send cancellation check but does not poll the token
  during stream consumption. Good enough for Phase 2 (the step-count
  guard prevents infinite chains), not good enough for a real
  long-running completion that the user wants to interrupt mid-token.
  Revisit in Phase 3 when there's a real channel that can signal
  the interrupt.

## Lessons carried forward

- **Trait seams beat mocks for cross-crate integration tests.** The
  Phase 2 exit test drives four crates (`aivyx-core`, `aivyx-llm`,
  `aivyx-audit`, `aivyx-capability`) through a single fake HTTP
  transport. No mocking library, no local HTTP server, no
  `#[cfg(test)]`-only public surface — every seam is a trait that
  already exists for its own reasons. The `HttpTransport` trait
  alone paid for itself inside one phase.
- **Additive trait extensions are cheap; enum/signature breaks are
  free when tests are tight.** The Phase 1 `TurnPlanner` trait grew
  two new default methods and one new parameter on `next_step`. The
  existing 19 `VecPlanner` tests required *zero changes* because
  the default methods were no-ops and the new parameter was
  ignored. Design the seam so that later additions are defaults,
  not required overrides.
- **Audit stays authoritative even with richer planner observation.**
  Adding `observe_tool_outcome(&ToolOutcome)` as a *separate* hook,
  rather than widening `StepObservation`, means the split between
  "what audit records" and "what the planner branches on" stays
  visible. Future planners that need payload access have to
  explicitly override the hook — the access is greppable.
- **Feature-gate expensive dep tails.** Putting `reqwest` + `rustls`
  + `secrecy` behind `provider-anthropic` meant every Phase 1 test
  still runs with the default-feature dep tree, and every crate
  downstream of `aivyx-core` can pick whether to pull in the HTTP
  stack. A one-line Cargo feature decision saved the default build
  from growing a 30-crate tail.
- **Commit per task, not per phase.** Five clean commits
  (`aaa2f1c` → `2b6f876`) each build and test green in isolation.
  If a Phase 3+ bisect ever needs to find where an LLM-path
  regression entered, the granularity pays for itself. The
  discipline cost ~30 seconds per task.
- **`DESIGN.md` stayed empty-diff twice in a row.** Phase 1 and
  Phase 2 both exited without a single contract amendment. The
  Phase 0 up-front design work keeps earning compound returns —
  every phase that holds the contract makes the next phase's
  contract-hold easier, because the test suite has absorbed more
  of the invariants.

## Exit criteria (all met)

- [x] `aivyx-llm` defines a dyn-compatible `LlmProvider` trait against
      the D3 shape
- [x] One concrete `LlmProvider` impl exists (Anthropic, feature-gated)
- [x] An LLM-backed `TurnPlanner` impl drives the same turn loop
      `VecPlanner` drove in Phase 1
- [x] `AuditBridge` connects `HmacChainLog` to `ConcreteAgent`
      without any Phase 1 seam edits
- [x] End-to-end integration test (`llm_driven_turn_e2e_tool_call_
      then_final_message`) drives a real `ConcreteAgent` through
      `LlmPlanner` → `AnthropicProvider` → `FakeTransport` → SSE
      parser → `AuditBridge<HmacChainLog>` and verifies the full
      audit chain
- [x] Tests run without hitting a real API by default; the opt-in
      live smoke test skips unless `ANTHROPIC_API_KEY` is set
- [x] `cargo test --workspace` green (87 tests, 1 ignored)
- [x] `cargo test --workspace --all-features` green (same totals)
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [x] `DESIGN.md` unchanged since Phase 0 exit (`git diff 1b4f271..
      HEAD -- DESIGN.md` is empty)

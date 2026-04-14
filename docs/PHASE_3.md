# Phase 3 — First Real Channel (FROZEN)

**Status:** Closed 2026-04-14
**Exit commit:** `fa0f4ea` — *"Phase 3 task 5: end-to-end CLI integration test"*
**Predecessor:** [PHASE_2.md](PHASE_2.md) (exit commit `2b6f876`)
**Successor:** [PHASE_4.md](PHASE_4.md)
**Contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all LOCKED — unchanged)

This document is a historical record. The CLI channel it produced
lives in `aivyx-channel` (new `LocalChannel`, `render` module,
`session::run_session` reusable REPL, and the `aivyx` reference
binary) and `aivyx-core` (new `TURN_TIMEOUT` const + deadline task,
mid-stream cancellation in the LLM planner, first emission of
`TurnOutcome::TimedOut`). This file explains *how* it came together
and what was deliberately left for Phase 4.

## Goal (as written at phase entry)

Implement the first real `ChannelContext` — `LocalChannel`, a CLI
channel that reads a user turn from stdin, drives an LLM-backed
`ConcreteAgent` through the Phase 2 turn loop, and streams the
resulting `StreamEvent`s back to stdout. First phase where a
**person** can interact with an Aivyx agent without writing test
code. No new tools, no remote channels, no contract edits.

## What shipped

- **`aivyx-channel::LocalChannel<W>`** (`beb0aac`). A `ChannelContext`
  impl generic over `W: Write + Send + 'static`, holding an
  `Arc<Mutex<W>>` writer (so production wires `io::Stdout` and tests
  wire `Vec<u8>` against the same code path). `TrustTier::Trusted` +
  `ChannelPlatform::Local` hardcoded. `flush()` after every text
  chunk so streaming is actually visible at LLM token rates. The
  cancellation token is wrapped in `Arc<Mutex<CancellationToken>>`
  for per-turn rotation (see task 4 below). 6 unit tests.
- **`aivyx-channel::render` module** (`35d97a4`). Pure rendering
  functions extracted from `LocalChannel` so the channel becomes a
  thin lock-write-flush wrapper around stateless renderers.
  `RenderMode` enum (single `Human` variant today, extensible),
  `render_stream_event` and `render_finalize` public functions, and
  a `short_id` helper that prints the first 8 chars of a UUID for
  tool-call markers (`→ tool[a1b2c3d4] {…}` / `← tool[a1b2c3d4]
  summary`). Finalize prints `\n[turn <state>]`. 10 unit tests
  cover every variant in isolation, so the channel module's tests
  reduced to a single delegation smoke test. Pure-function design
  is what eventually let the Phase 5+ remote channels share renderers
  by importing the module — no class hierarchy needed.
- **`aivyx` reference binary** (`d45a63f`). New `[[bin]]` target
  *under* `aivyx-channel/src/bin/aivyx.rs`, not a new crate, so D8's
  9-crate workspace lock holds. Reads `ANTHROPIC_API_KEY` (required),
  `AIVYX_MODEL` and `AIVYX_SYSTEM_PROMPT` (optional). Constructs
  `AnthropicProvider` + `AuditBridge<HmacChainLog>` + `LocalChannel`,
  spawns a `tokio::signal::ctrl_c` task wired to the channel's
  cancellation token, and runs the REPL. The first interactive
  Aivyx surface — typing at the terminal drives the full Phase 0–3
  stack end-to-end.
- **Wall-clock turn deadline** (`d875ea9`). `pub const TURN_TIMEOUT:
  Duration = Duration::from_secs(120);` in `aivyx-core::agent`. The
  turn loop spawns a deadline task on entry that sleeps for
  `TURN_TIMEOUT`, sets an `AtomicBool` flag, and cancels the
  channel's token. The loop's post-`next_step` cancellation check
  uses the flag to translate the cancel into `TurnOutcome::TimedOut`
  vs `TurnOutcome::Cancelled`. **First code path in the project
  that emits `TurnOutcome::TimedOut`** — the variant has been
  public since Phase 0 with no emission site until now. The
  deadline task is `abort()`ed on normal turn exit so a fleet of
  long-running agents doesn't leak background tasks.
- **Mid-stream cancellation in `LlmPlanner`** (`d875ea9`). The
  `one_step` consumer rewritten with `tokio::select! { biased; _ =
  cancellation.cancelled() => …; event = stream.next_event() => …; }`.
  When the cancel branch wins, the stream is dropped (which
  propagates through the provider's reqwest body stream and aborts
  the underlying connection) and `LlmError::Cancelled` is returned.
  `next_step` adds an `Err(LlmError::Cancelled) => return
  NextStep::Stop` arm so the loop's own post-`next_step` cancel
  re-check takes over and emits the right outcome. The same code
  path serves both ctrl-C and the wall-clock deadline — there is
  one interrupt primitive (`CancellationToken`), and what *caused*
  the cancel only matters at the outcome-translation step.
- **`LocalChannel::reset_cancellation()` + per-turn token rotation**
  (`d875ea9`). `tokio_util::CancellationToken` is monotonic — once
  cancelled, it stays cancelled forever. The naive task-3 binary
  used a single process-wide token, which meant the next turn after
  any ctrl-C would see a pre-cancelled token and the loop would
  bail before doing any work. Fixed by wrapping the channel's token
  in `Arc<Mutex<CancellationToken>>` and exposing two methods:
  `reset_cancellation()` (called from the binary at the top of each
  REPL iteration to swap in a fresh token) and `token_slot()`
  (handed to the signal task so it reads the *current* token on
  every ctrl-C, picking up post-rotation tokens automatically).
  Channel contract untouched — `cancellation_token()` still returns
  an owned clone of whatever token is currently in the slot. 2 new
  unit tests cover the rotation path and the orphaned-handle
  semantics.
- **`run_session` library function** (`fa0f4ea`). The REPL was
  extracted from `bin/aivyx.rs` into `aivyx-channel::session`
  parameterized by `Arc<dyn LlmProvider>`, `Arc<dyn AuditHook>`,
  `SessionConfig`, `LocalChannel<W>`, and `R: BufRead`. The binary
  shrank to ~80 lines of pure wiring; the integration test reuses
  the same function with a `ScriptedProvider`, a `Cursor<&[u8]>`
  reader, and a `Vec<u8>` sink. **The test exercises the exact
  code path the binary does, not a parallel reimplementation** —
  this was the load-bearing refactor for the whole task 5 design.
- **End-to-end CLI integration test** (`fa0f4ea`). `crates/aivyx-
  channel/tests/cli_e2e.rs` drives two scripted turns through
  `run_session` and asserts: streamed text chunks appear in stdout
  in the right order, finalize markers separate turns, the audit
  chain has exactly 4 entries (`TurnStarted`, `TurnEnded`,
  `TurnStarted`, `TurnEnded`) and `verify()` passes, the two turns
  share the channel's `session_id` while carrying distinct
  `turn_id`s, blank input lines are skipped, and reader EOF returns
  a clean `SessionReport { turns_run: 2 }`.

**Totals:** 107 tests (default features and `--all-features`
identical), 1 ignored (the opt-in live-API smoke test from Phase 2,
unchanged). `cargo test --workspace` green. `cargo clippy --
workspace --all-targets -- -D warnings` clean. `DESIGN.md` unchanged
since the Phase-0/Phase-1 docs split (`e0d6437`) — **third phase in
a row with no contract amendment**.

## Refinements landed (from Phase 2's queue)

- **Mid-stream cancellation in the provider/planner.** Phase 2
  flagged this: `ReqwestTransport` did a pre-send cancellation check
  but the planner didn't poll the token while consuming the stream.
  Fixed in `LlmPlanner::one_step` via the `tokio::select!` race
  rather than in the transport, because that's where the token is
  already in scope and the cancel needs to surface as `LlmError::
  Cancelled` to the planner anyway.
- **Wall-clock timeout enforcement.** Also from Phase 2's queue.
  `TurnOutcome::TimedOut` had been a public variant with no
  emission site since Phase 0; this phase finally wired it. Lives
  in the loop, not the planner, because a turn-level budget is the
  only place that can distinguish "stuck planner" from "slow tool"
  from "user wants to give up."
- **Per-turn cancellation token lifecycle (from Phase 3 task 3).**
  Surfaced as a known bug at task-3 commit time; fixed in task 4
  alongside the timeout work because both touch the same token-
  lifecycle seam. The rotation lives in `LocalChannel`, not on the
  `ChannelContext` trait, so the contract stayed unchanged.

## Decisions made during Phase 3 that aren't in DESIGN.md

### `aivyx-core` becomes tokio-runtime-bound

Phase 0 through Phase 2, `aivyx-core` only depended on
`tokio-util`'s runtime-agnostic `CancellationToken` and
`async-trait`. Phase 3 task 4 added `tokio` as a *runtime* dep
(features `rt`, `time`, `macros`) because the deadline task uses
`tokio::spawn` and `tokio::time::sleep`. This locks every Aivyx
agent to the tokio runtime — not just tokio-flavored futures.
Considered: an executor-agnostic timer abstraction. Rejected: it
would have required a much bigger trait surface just to avoid one
`tokio::spawn`, and tokio is already the de-facto Rust async
runtime. The trade is documented here so future contributors don't
think this was an oversight.

### Renderer as a pure module, channel as a thin wrapper

`render_stream_event` and `render_finalize` are pure functions
taking `&mut dyn Write` + the event. `LocalChannel` does nothing
but lock the mutex, call the renderer, and flush. The split has
two concrete payoffs: (a) per-variant render assertions live in
`render::tests` against an in-memory `Vec<u8>`, with no async
machinery; (b) when Phase 5+ adds remote channels (Telegram,
Discord, etc.) they import the same renderer rather than
re-implementing the format. The `RenderMode` enum exists today
with one variant (`Human`) precisely so a future `Json` or `Tui`
mode can land as an additional match arm in the same module
instead of a parallel renderer hierarchy.

### Tool-call markers use a short ID, not a tool name

`→ tool[a1b2c3d4] {input json}` — the 8-char prefix is `ToolId`'s
UUID truncated. Considered embedding the tool *name* in
`StreamEvent::ToolCallStarted` instead, but that would have
required a `name: &str` field on a D3-locked enum. Short ID is
informative enough for grep ("which tool calls happened in this
turn?") and changes nothing in the contract. Phase 4's first real
tool will tell us whether the lack of a name in the user view is
actually painful; if it is, a renderer-only enrichment (look up
the tool by ID through a registry passed at render time) is local
to one module.

### Binary target lives under `aivyx-channel`, not as a 10th crate

D8 locks the workspace at 9 crates. A new `aivyx-cli` would have
been the 10th and required a contract amendment. The binary lives
at `crates/aivyx-channel/src/bin/aivyx.rs` instead — Cargo's
`[[bin]]` target inside an existing lib crate. The binary-only
dependency tail (`aivyx-audit`, `aivyx-llm` with `provider-anthropic`,
`tokio` runtime features, `secrecy`) is in `aivyx-channel`'s
regular `[dependencies]` because Cargo doesn't have a "bin-only
deps" section, but the lib target only `use`s the small subset
(`aivyx-core`, `aivyx-capability`, `async-trait`). Trade accepted:
slightly larger lib dep tree, no contract amendment.

### `run_session` extraction over a binary subprocess test

Task 5 originally pictured an integration test that spawned the
binary as a child process and pumped bytes through its stdio. That
shape would have been hermetic but had two problems: (a) the live
binary builds an `AnthropicProvider` which talks to the network,
so the test would have needed a backdoor env var or a mock server;
(b) child-process tests are slower and have flakier teardown than
in-process tests. Refactoring the REPL into `run_session` got both
properties for free: the binary becomes a thin wiring layer, the
test calls the same function with swapped edges, and the only thing
the test cannot exercise is the signal task (which is fine —
SIGINT handling is not what an E2E test should be verifying anyway).

### `R: BufRead` in `run_session`, no `Send` bound

`io::StdinLock<'static>` does not implement `Send`. The naive
`R: BufRead + Send` bound would have broken the binary caller.
Dropped the `Send` because `run_session` is a single-task await
chain — there is no internal `tokio::spawn` that crosses the
reader between threads. The phantom `Send` requirement would have
been load-bearing for nothing.

### The integration test goes through `LlmProvider`, not `FakeTransport`

Phase 3's task list said "FakeTransport supplying canned SSE."
Resolved at task 5 entry to use a `ScriptedProvider` directly
(implementing `LlmProvider` rather than `HttpTransport`). The
transport seam is the right test surface for the Anthropic SSE
parser, which already has its own coverage in `aivyx-llm`. Going
through it again at the channel level would just have re-tested
the parser. The session test is about the *session*, not the wire
format — `LlmProvider` is the right layer.

### `TURN_TIMEOUT = 120s`, hardcoded const

Same philosophy as `MAX_STEPS_PER_TURN = 32` from Phase 2: not a
config knob, not in `LlmPlannerConfig`, not in `SessionConfig`. A
turn that takes more than 2 minutes is *almost always* a bug
(stuck planner, runaway tool chain, or a hung HTTP body), not a
performance regime that needs tuning. Making it configurable would
normalize exceeding it. 120 seconds is generous enough to cover
realistic multi-step tool chains and tight enough to catch genuine
hangs. Revisit later in one place if a real workload proves
otherwise.

### Audit chain inspectable via `AuditBridge::writer()`

The integration test reaches through the bridge to inspect the
underlying `HmacChainLog` directly: `audit_bridge.writer().entries()`
+ `verify()`. This is the same pattern Phase 2's bridge tests
already used; no new surface needed. The bridge's whole point is
that it does not hide its underlying writer — `writer()` returns
`&W`, and tests get full read access to whatever the production
writer would have logged.

## Bugs caught in Phase 3

- **Monotonic `CancellationToken` poisoning the next turn.** Task 3
  shipped a binary that captured one process-wide token via
  `channel.cancel_handle()` and relied on the signal task cancelling
  it. `tokio_util`'s token is monotonic — once cancelled it stays
  cancelled — so after the user ctrl-C'd a turn and returned to the
  prompt, the next turn would see a pre-cancelled token and the
  loop's top-of-loop cancel check would bail immediately. The
  task-3 binary worked around this with a pre-turn `is_cancelled()`
  guard that exited the process; documented as a known issue in
  the freeze of task 3's commit. Real fix landed in task 4: wrap
  the token in `Arc<Mutex<CancellationToken>>`, add
  `reset_cancellation()`, and have the binary call it at the top of
  each REPL iteration. The signal task switched to reading the
  slot per-ctrl-C so it picks up rotated tokens.
- **`AnthropicProvider::new` is fallible — task 3 assumed it wasn't.**
  First draft of `bin/aivyx.rs` did `Arc::new(AnthropicProvider::
  new(config))`, which doesn't compile because `new` returns
  `Result<Self, LlmError>`. The Phase 2 tests all used
  `with_transport`, which is infallible, so I'd never seen the
  fallible path. Fixed with `.map_err(|e| format!("…: {e}"))?` and
  bound the result before wrapping in `Arc`.
- **`ChannelContext` trait not in scope in the binary.** Same first
  draft of `bin/aivyx.rs` called `channel.session_id()` and
  `channel.cancellation_token()` without importing the trait. Rust
  reports this as "method not found" instead of "trait not in
  scope," which is the usual hint. Fixed by adding `ChannelContext`
  to the `aivyx_core::{…}` import block.
- **`std::time::Instant` is not virtualized by `tokio::time::pause`.**
  Task 4's `wall_clock_timeout_emits_timed_out_outcome` test used
  `tokio::time::pause` + `advance(TURN_TIMEOUT)` to simulate the
  120-second wait without actually waiting. The test correctly
  walked the deadline task to completion, but the assertion
  `elapsed >= TURN_TIMEOUT` failed — `elapsed` was ~90µs because
  `agent.rs` measures elapsed with `std::time::Instant`, which
  always reads the real monotonic clock. Considered switching to
  `tokio::time::Instant`; rejected because production uses real
  time and faking the production type just to make the test feel
  symmetric was the wrong trade. Test now asserts only on outcome
  variant + `tool_calls_made == 0`, which is sufficient to prove
  the deadline path; production `elapsed` remains a real value.
- **`SessionConfig` would-have-been over-abstracted.** First draft
  of `session.rs` had `SessionConfig` accepting a planner factory
  closure as a field, on the theory that a future test might want
  a non-`LlmPlanner`. Realized it had exactly one caller, deleted
  the field, and let `run_session` build the planner directly from
  `provider + registry + config`. Same anti-abstraction lesson as
  Phase 2's `ToolRegistryExt` deletion.
- **Lib-vs-bin Cargo dep tail.** `aivyx-channel`'s `Cargo.toml`
  pulls `aivyx-audit`, `aivyx-llm` (with `provider-anthropic`),
  `secrecy`, and runtime tokio features into the regular
  `[dependencies]` block — not because the *library* needs them,
  but because Cargo has no "bin-only deps" section. Documented in
  a comment in `Cargo.toml` so a future contributor wondering "why
  does the channel lib pull in reqwest?" gets the answer in-place
  rather than from `git blame`.

## Decisions deferred to Phase 4

- **First concrete `Tool`.** Phase 3 ships zero real tools. The CLI
  works for chat-only turns, and an empty `ToolRegistry` is what
  the binary registers. Phase 4's first tool is the natural moment
  to validate that D4's prefix-attenuated scopes (R1) actually work
  on real input — the leading candidate is a filesystem tool
  (`fs.read` / `fs.write`) precisely because it exercises path-glob
  qualifiers.
- **Tool name in `StreamEvent::ToolCallStarted`.** The renderer
  shows `→ tool[a1b2c3d4]` with a short ID, not a tool name.
  Phase 4 will tell us whether that's painful in practice. If it
  is, a `name: &str` field on the event is a contract amendment;
  a renderer-side lookup against a registry handle is local to the
  renderer module.
- **Persistent session history.** `aivyx-storage` is still stubbed.
  The CLI keeps in-memory session state for the life of a process
  and that's it. Phase 5's encrypted-storage phase wires this.
- **Real-API live test in CI.** The opt-in `#[ignore]` smoke test
  from Phase 2 still works (`ANTHROPIC_API_KEY=… cargo test -- --
  ignored`) but nothing wires it into CI. That's a Phase 5+ ops
  decision, not a Phase 3 oversight.
- **Per-user multi-tenant CLI.** Single user, single process, single
  session, by design. Phase 7+ ecosystem decision.
- **`rustyline` / line editing / history.** Bare `stdin().read_line`
  in the binary. The `ChannelContext` trait does not care which
  line-reading library the binary uses; swapping later is local
  to one file. The ergonomics gap hasn't proved painful yet.
- **Ctrl-C safety on Windows.** `tokio::signal::ctrl_c` works on
  Windows but the SIGINT semantics differ. Untested. Phase 3 is
  Linux-first; cross-platform polish is a later phase.
- **`CapabilitySet::default()` ergonomics.** Phase 2's queue flagged
  this. Phase 3 didn't add it because the only places that needed
  it (the binary, the integration test) were happy with explicit
  `from_scopes([…])`. Phase 4 will probably take it incidentally
  if a tool registration site needs it.

## Lessons carried forward

- **One run loop, swapped edges.** The `run_session` extraction is
  the model for how every future channel's E2E test should work.
  The binary is a thin wiring layer; the library function is what
  the test drives. The test exercises the exact code path the
  binary does, not a parallel reimplementation. This shape is
  worth replicating for Phase 5's storage CLI and Phase 6's memory
  tools.
- **Cancellation is one primitive with many callers.** Ctrl-C, the
  wall-clock deadline, and any future admin-kill all set the same
  `CancellationToken`. The loop only needs to *distinguish* causes
  at the outcome-translation step (via the `AtomicBool`
  `deadline_fired` flag for `TimedOut`). This unification means a
  new interrupt source — say, a "memory budget exceeded" signal —
  is a one-line addition: cancel the token, set a flag, the loop
  emits the right outcome variant.
- **Trait surface is harder to amend than impl behavior.** Three
  Phase 3 tasks would have been simpler with a trait extension
  (`ChannelContext::begin_turn`, a `name` field on
  `StreamEvent::ToolCallStarted`, a `SessionConfig` on the trait).
  All three were resolvable inside the impl without amending the
  contract: `LocalChannel::reset_cancellation()` (inherent method),
  the renderer's short-ID format (no trait change), `run_session`
  taking config by value (not via the channel). When the contract
  is locked, look at the impl seam *first* before reaching for an
  amendment.
- **Pure functions are the cheapest seam.** The `render` module is
  10 unit tests against `Vec<u8>` with zero async, zero locking,
  zero channel context. The `LocalChannel` tests reduced to a
  single delegation smoke test because the renderer carries the
  format-correctness story. Future channels (Telegram, Discord)
  will import the same module instead of re-rendering.
- **Hardcoded budgets beat config knobs for safety guards.**
  `MAX_STEPS_PER_TURN = 32` (Phase 2), `TURN_TIMEOUT = 120s`
  (Phase 3), and the empty-input skip (Phase 3) are all
  hardcoded. Making them configurable would normalize exceeding
  them. The pattern: a guard that exists to catch *bugs* should
  not be configurable by the caller writing the buggy code.
- **`DESIGN.md` empty-diff streak: 3.** Phase 1 → Phase 2 → Phase
  3 all exited with zero contract changes. The Phase 0 up-front
  design work keeps compounding: every locked decision that holds
  through another phase makes the next phase's hold easier,
  because more invariants are in code rather than in prose.
  Three phases in is when this pattern stops being luck and starts
  being evidence the contract was right.
- **Commit per task, not per phase. Still.** Five clean commits
  (`beb0aac` → `fa0f4ea`), each builds and tests green in
  isolation. Same discipline as Phase 2; same payoff for any
  future bisect.

## Exit criteria (all met)

- [x] `aivyx-channel` defines a `LocalChannel: ChannelContext`
      implementation running on stdin/stdout
- [x] A CLI binary exists (`cargo run -p aivyx-channel --bin aivyx`)
      that starts a single session and accepts user turns
- [x] A real Anthropic-backed turn can be driven end-to-end by a
      human typing at the terminal (opt-in, requires
      `ANTHROPIC_API_KEY`)
- [x] A non-interactive E2E test drives the session via scripted
      stdio with a `ScriptedProvider`, with the full audit chain
      verified (`crates/aivyx-channel/tests/cli_e2e.rs`)
- [x] Ctrl-C cancels the in-flight turn and returns to the prompt;
      a second ctrl-C exits (Q4 resolution: option (c))
- [x] Wall-clock timeout path exists and emits
      `TurnOutcome::TimedOut` (`agent::tests::wall_clock_timeout_
      emits_timed_out_outcome`)
- [x] `cargo test --workspace` green (107 tests, 1 ignored)
- [x] `cargo test --workspace --all-features` green (same totals)
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [x] `DESIGN.md` unchanged since the Phase-0/Phase-1 docs split
      (`git diff e0d6437..HEAD -- DESIGN.md` is empty)

# Phase 3 — First Real Channel (ACTIVE)

**Status:** Active — opened 2026-04-14
**Predecessor:** [PHASE_2.md](PHASE_2.md) (frozen 2026-04-14, exit `2b6f876`)
**Contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all LOCKED)

This is the active working doc for Phase 3. It is edited freely
during the phase and frozen at phase exit. Contract changes go
through the amendment process described in [README.md](README.md).

## Goal

Implement the first real `ChannelContext` — `LocalChannel`, a CLI
channel that reads a user turn from stdin, drives an LLM-backed
`ConcreteAgent` through the Phase 2 turn loop, and streams the
resulting `StreamEvent`s back to stdout. This is the first phase
where a **person** can interact with an Aivyx agent without
writing test code.

Everything below the channel stays fixed. Phase 2's `LlmPlanner`,
`AnthropicProvider`, `AuditBridge`, and `HmacChainLog` are all
exactly the components a real `LocalChannel` turn will run against.
Phase 3 is the last mile — the code that turns a terminal into a
`ChannelContext` impl.

## Non-goals

- Remote channels (Telegram, Discord, Slack, Matrix, Email). The
  whole point of nailing `LocalChannel` first is to prove the
  `ChannelContext` seam carries a non-trivial channel before any
  remote transport hits it.
- A real concrete `Tool`. Phase 4's scope. Phase 3 can use the same
  `memory.read`-style fake the Phase 2 E2E test uses, or a built-in
  echo tool, for its smoke tests.
- Persistent session history. `aivyx-storage` stays stubbed; Phase 5's
  scope. The CLI keeps in-memory session state across turns within a
  single process lifetime, and that's it.
- A pretty TUI. Phase 3 is a plain readline loop with token-by-token
  stdout streaming. Colors and cursor tricks can come later, once
  the channel seam is proven to carry them through unchanged.
- Per-user authentication / multi-tenant CLI. One user, one process,
  one session — the CLI is the single most trusted channel on
  the box.
- Real-API integration by default. Phase 3's test suite runs against
  the same `FakeTransport` Phase 2 uses. Live-API CLI runs are
  opt-in via the same `ANTHROPIC_API_KEY` env var pattern.

## Entry criteria (inherited from Phase 2 exit)

- [x] Phase 2 frozen ([PHASE_2.md](PHASE_2.md))
- [x] 87-test workspace suite green on `main`
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [x] `DESIGN.md` unchanged since Phase 0 exit (`1b4f271`)
- [x] `LlmProvider` + `LlmPlanner` + `AnthropicProvider` all live
- [x] `AuditBridge` glues `HmacChainLog` to `ConcreteAgent`
- [x] `ChannelContext` trait definition exists in `aivyx-core` and
      is re-exported from `aivyx-channel` (Phase 1 seam)

## Known refinements queued from Phase 2

- **Mid-stream cancellation in the provider.** Phase 2's
  `ReqwestTransport` only checks `CancellationToken::is_cancelled()`
  before the request is sent. A real CLI with ctrl-C wants to
  interrupt a completion mid-token. Revisit inside the provider's
  stream consumer. Not a contract change — the `LlmProvider` already
  takes `&CancellationToken`; the refinement is in the reqwest impl.
- **Wall-clock timeout.** `TurnOutcome::TimedOut` still has no
  emission site. A CLI session is exactly the place where a 60-second
  stuck completion should surface as a timeout rather than a hang.
  Phase 3 is the natural landing zone.
- **`CapabilitySet::default()` ergonomics.** Several Phase 2 tests
  hit the missing `Default` impl and worked around it. Cheap add,
  probably drops in at the start of Phase 3 if a `LocalChannel`
  smoke test needs it.
- **Per-turn cancellation token lifecycle (from task 3).** The task
  3 binary uses a single process-wide `CancellationToken` held by
  `LocalChannel`. `tokio-util`'s token is monotonic — once cancelled
  it stays cancelled forever — so after the user ctrl-Cs a turn and
  returns to the prompt, the next turn would see a pre-cancelled
  token. The task-3 binary works around this by exiting the process
  in the pre-turn guard; it's safe but user-hostile. Proper fix:
  `LocalChannel` owns a `Mutex<CancellationToken>` and rotates it
  per turn, while the signal task watches a *separate*
  process-lifetime token for the exit path. This is natural to
  land alongside task 4's timeout work because both touch the same
  token-lifecycle seam.

## Phase 3 task list

*Tasks are added as the phase opens; empty entries are intentional.*

- [ ] **Task 1 — `LocalChannel` skeleton.** A `ChannelContext` impl
      in `aivyx-channel` that owns a `SessionId`, a `CancellationToken`,
      and a writer handle. `TrustTier::Trusted`, `ChannelPlatform::
      Local`. Unit tests against an in-memory writer.
- [ ] **Task 2 — `StreamEvent` → stdout renderer.** Translate
      `StreamEvent::Text` / `StreamEvent::ToolCall` / `StreamEvent::
      Finalize` into terminal output. Token-by-token text streaming
      with a `flush()` per chunk so the user sees partial output.
      Tool calls get a one-line "→ tool(…)" marker; finalization
      prints the completed turn outcome summary.
- [ ] **Task 3 — `aivyx-cli` binary.** A new binary target (either a
      `bin/aivyx.rs` under an existing crate or, if cleaner, a tiny
      `aivyx-cli` bin crate — **decision at task 3 entry**, not now).
      Readline loop, config loading (API key via env), agent
      construction with `AuditBridge<HmacChainLog>`,
      `LlmPlanner` + `AnthropicProvider`. Ctrl-C cancels the
      in-flight turn via the channel's `CancellationToken`.
- [ ] **Task 4 — Wall-clock timeout.** Add a deadline to each turn
      (configurable, sane default ~60s). First code path that emits
      `TurnOutcome::TimedOut`. Poll the cancellation token inside
      the `LlmPlanner`'s stream consumer so the timeout actually
      interrupts mid-completion.
- [x] **Task 5 — End-to-end CLI integration test.** Drive the CLI
      via a scripted stdio harness (`Cursor` stdin, `Vec<u8>` stdout)
      with a `ScriptedProvider` returning canned `TextChunk` +
      `FinalMessage` steps. Verifies token-streaming order, audit
      chain contents + verification, and clean EOF exit. Landed as
      `crates/aivyx-channel/tests/cli_e2e.rs`, built on the new
      `run_session` extraction that refactored the binary's REPL
      into a reusable library function so the test drives the same
      loop the binary does — no parallel reimplementation.
      **Decision note:** the task description said "FakeTransport +
      canned SSE," but that would have tested the Anthropic SSE
      parser (which already has its own coverage) rather than the
      CLI. Used a `ScriptedProvider` at the `LlmProvider` layer
      instead, which is one layer above the transport and is the
      right surface for an E2E test that's about the *session*, not
      the *wire format*.
- [ ] **(Exit)** Document Phase 3 outcome + known issues in this
      file, freeze it, open `PHASE_4.md` (first real `Tool`).

## Open questions

- **Q1 — Binary layout: `bin/` under an existing crate, or a new
  `aivyx-cli` crate?** D8 locks the workspace at 9 crates. A new
  `aivyx-cli` would be the 10th. Leaning toward a binary target
  under `aivyx-channel` (the channel crate already holds the
  `LocalChannel` impl, and a `bin/aivyx.rs` under it keeps the
  9-crate lock honest). **Resolution path:** decide at task 3 entry
  based on how large `main.rs` ends up. If it's under ~300 lines,
  `aivyx-channel/src/bin/aivyx.rs` is fine. If it grows, revisit
  with a D8 amendment.
- **Q2 — Readline library.** `rustyline` is the obvious choice but
  is a non-trivial dep. Alternative: a plain `stdin().lines()` loop
  with no history / no line editing. Decision at task 3: start with
  the bare loop, upgrade to `rustyline` only if the ergonomics gap
  proves painful. The `ChannelContext` trait does not care which
  one we pick, and swapping later is local to `aivyx-channel`.
- **Q3 — Where do secrets come from?** Phase 2 uses
  `ANTHROPIC_API_KEY` env var in tests. For Phase 3's CLI: env var
  (simple, works with shell profiles) or a config file at
  `~/.config/aivyx/config.toml` (discoverable but introduces
  `aivyx-config` logic). **Leaning env-var-first, config-file-
  later** — Phase 3 ships the env-var path, Phase 5's encrypted
  storage phase adds the config file when there's somewhere secure
  to put it. The `SecretString` type stays the same across both.
- **Q4 — Ctrl-C semantics.** Three options: (a) cancel the current
  turn and return to the prompt; (b) cancel the current turn and
  exit the CLI; (c) first ctrl-C cancels, second exits. Unix CLI
  convention leans (c). The `CancellationToken` machinery already
  supports it — the decision is purely in the signal handler.
  Decide at task 3.
- **Q5 — How does the CLI learn about tools?** Phase 3 needs *some*
  tool for its smoke tests to exercise a tool-calling path. Options:
  (a) hardcode a test-only `echo` tool in the binary; (b) wait for
  Phase 4's real filesystem tool and ship Phase 3 with no tools
  registered (the CLI still works for chat-only turns); (c) stand
  up a trivial `memory.read` fake in `aivyx-channel`. Leaning (b):
  Phase 3's scope is the channel, not the tools, and a chat-only
  turn still exercises `LlmPlanner::next_step` → `FinalMessage` →
  audit chain. If task 5's E2E test needs a tool path, fall back
  to (a) with an explicitly test-scoped fake tool.

## Exit criteria (draft — revised once the task list stabilizes)

- [ ] `aivyx-channel` defines a `LocalChannel: ChannelContext`
      implementation running on stdin/stdout
- [ ] A CLI binary exists (`cargo run -p <...>` or installable via
      `cargo install`) that starts a single session and accepts
      user turns
- [ ] A real Anthropic-backed turn can be driven end-to-end by a
      human typing at the terminal (opt-in, requires
      `ANTHROPIC_API_KEY`)
- [ ] A non-interactive E2E test drives the binary via scripted
      stdio and `FakeTransport`, with the full audit chain verified
- [ ] Ctrl-C cancels the in-flight turn and returns to the prompt
      (Q4 decision pending)
- [ ] Wall-clock timeout path exists and emits `TurnOutcome::TimedOut`
- [ ] `cargo test --workspace` green
- [ ] `cargo test --workspace --all-features` green
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] `DESIGN.md` unchanged (or, if changed, an amendment file
      exists in `docs/amendments/` and is linked from this doc)

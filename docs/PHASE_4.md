# Phase 4 — First Real Tool

**Status:** Active (opened 2026-04-14)
**Predecessor:** [PHASE_3.md](PHASE_3.md) (exit commit `fa0f4ea`)
**Contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all
LOCKED — unchanged since `e0d6437`, three phases running)

This document is the **working journal** for Phase 4. It will churn.
At phase exit it is frozen under the same convention as
[`PHASE_3.md`](PHASE_3.md) — no edits except through commits tagged
`docs(phase-4):`.

## Goal

Implement the first **concrete `Tool`** — an agent-callable
capability that does real work on the host, goes through the
Phase 0 planner → registry → scope-check → audit pipeline end-to-
end, and is exercised by an LLM-driven integration test. This is
the first phase where an Aivyx agent can *do things* beyond
streaming text back.

The leading candidate is a **filesystem tool** (`fs.read`,
`fs.write`, possibly `fs.list`) because:

- It is the cleanest exercise of D4's **prefix-attenuated scopes**
  (R1) — `fs.read:/home/julian/notes/**` must admit reads under
  `/home/julian/notes/` and deny reads elsewhere, with real paths,
  real canonicalization, real symlink traversal decisions. Nothing
  short of a filesystem tool stress-tests scope attenuation against
  a live attacker surface.
- It is stateless at the capability level — no background daemon,
  no network, no persistent handle — so it does not anticipate
  Phase 5's storage work.
- The failure modes are well-understood (path traversal,
  TOCTOU, symlink races) which means we will argue about the
  right thing instead of inventing new categories of failure.

This phase is **not** about "tools" as a category. It is about
*one* tool, deeply wired, so that Phase 6's memory-as-tool work
has a proven reference implementation to copy.

## Non-goals

- **Not a plugin system.** One tool, statically registered, in
  the binary. No dynamic loading, no discovery protocol, no
  manifest format. Those are Phase 7+.
- **Not a tool catalog.** No `fs.stat`, no `fs.mkdir`, no
  `fs.delete`, unless one of them turns out to be the *only*
  way to make the integration test reach a point the planner
  can sensibly drive. Minimum surface to prove the pipeline
  works end-to-end.
- **Not a security review of the whole scope system.** The
  scope system has unit tests from Phase 0. Phase 4 is the first
  *concrete* user, but a comprehensive audit of the scope DSL
  is deferred. If Phase 4 surfaces a sharp edge, we document it
  and either fix it here or queue it for Phase 5+.
- **Not a new channel.** `LocalChannel` is the only channel
  this phase touches. Remote channels are Phase 7+.
- **Not persistence.** The tool reads/writes *the real
  filesystem*; it does not talk to `aivyx-storage` (still
  stubbed). Phase 5's job.
- **Not JSON-schema tool input validation.** The tool declares
  an input schema via serde and lets the planner stringify it
  for the LLM. No runtime schema validator beyond what serde
  already gives us. If the LLM hands back malformed input, the
  tool returns an error; the loop handles it.

## Entry criteria (all met from Phase 3 exit)

- [x] `ConcreteAgent` turn loop is LLM-driven and tested end-to-end
      (Phase 2 task 5 / Phase 3 task 5).
- [x] `ChannelContext` has one working impl (`LocalChannel`) that
      streams real output to a human.
- [x] Cancellation + wall-clock timeout work (Phase 3 task 4).
- [x] Audit chain integration is wired through `AuditBridge` and
      verified in an integration test (Phase 3 task 5).
- [x] `DESIGN.md` is still at its `e0d6437` baseline — no contract
      drift to reconcile before this phase starts.

## Refinements queued from Phase 3

These are the open threads Phase 3 left behind that Phase 4 should
handle if they become load-bearing for the tool work, and otherwise
punt to Phase 5 consciously rather than silently.

- **Tool name in `StreamEvent::ToolCallStarted`.** Phase 3's
  renderer prints `→ tool[a1b2c3d4]` with a short ID because there
  is no tool name on the event. Phase 4 will tell us whether that
  is painful in practice. **Decision point:** first time a user
  watches their CLI and can't tell which tool just ran, either
  amend the D3 enum (contract amendment) or do a renderer-side
  lookup through the registry. Prefer the renderer lookup unless
  a real case forces an amendment.
- **`CapabilitySet::default()` ergonomics.** Queued from Phase 2,
  still not added. Phase 4's tool registration site is the first
  place an empty default would obviously be useful. Likely taken
  incidentally. Small, local.
- **Tool-call marker richness.** Today's `{input json}` render
  is naive; a long path in a filesystem tool call will look ugly
  in the terminal. Decision deferred until we *see* the bad
  render — no speculative rewrites.
- **Opt-in live-API test in CI.** Still only runnable by hand
  with `cargo test -- --ignored`. Not a blocker for Phase 4's
  tool work; still a Phase 5+ ops decision.
- **`rustyline` line editing.** Still `stdin().read_line`. Not
  touching unless the Phase 4 interactive loop reveals a specific
  pain point.

## Task list (draft — revised as work lands)

Each task is a single commit with a passing `cargo test --
workspace` and clean `cargo clippy --workspace --all-targets --
-D warnings`. Phase 2 + Phase 3 have now demonstrated this cadence
works for 10 commits in a row; keep it.

1. **`aivyx-core::tool::Tool` trait is real.** Today the loop
   calls `registry.get(&id)?.invoke(...)` through a stub. Audit
   the existing `Tool` surface against what a filesystem tool
   actually needs: `input: serde_json::Value`, `ToolContext<'_>`,
   returning `Result<ToolOutput, ToolError>`. If the existing
   surface already supports this (it should — D4 specified it),
   this task is documentation-only and the commit is a test
   asserting the registered tool's `descriptor()` surfaces for
   the planner. If it *doesn't*, this task is where we find out,
   and the contract implications get weighed before we invest in
   task 2.
2. **`FsReadTool` + unit tests.** Concrete impl. Takes
   `fs.read` scope with a prefix qualifier, canonicalizes the
   input path, rejects traversal outside the scope prefix (the
   scope system's prefix-attenuation logic does the actual
   rejection — this task just wires it). Returns file contents
   as a `ToolOutput::Text` with a byte cap. Refuses symlinks
   that escape the allowed prefix. Unit tests cover: happy-path
   read, out-of-scope path denial, symlink-escape denial,
   byte-cap truncation, nonexistent file error, binary file
   handling (stream as base64 or refuse).
3. **`FsWriteTool` + unit tests.** Concrete impl. Takes
   `fs.write` scope, canonicalizes the target path, creates
   parent dirs only within the scope prefix, refuses symlinks
   and absolute-path inputs outside scope. Same unit-test
   shape as task 2. **Open question:** atomic-write via
   `tempfile` + rename, or plain `std::fs::write`? Default to
   atomic because partial writes across a ctrl-C are a worse
   failure mode than a slower happy path.
4. **Registry wiring + binary surface.** `bin/aivyx.rs`
   constructs a `ToolRegistry::new(vec![
   Arc::new(FsReadTool::new(...)), Arc::new(FsWriteTool::new(...))])`
   and passes it through `SessionConfig` (new field) into
   `run_session`. The CLI gains an `AIVYX_FS_ROOT` env var that
   determines the scope prefix; default to `$HOME/aivyx-sandbox`
   and create it if missing. The integration test from Phase 3
   does **not** register the tools — it continues to run chat-
   only turns — because its job is to be a narrow regression
   test for the session loop, not for tool execution.
5. **LLM-driven tool integration test.** New test file
   `crates/aivyx-channel/tests/fs_tool_e2e.rs`. Uses a
   `ScriptedProvider` whose first step emits a
   `LlmStepEnd::ToolCall { name: "fs.read", input: ... }`,
   whose second step (after the tool result lands) emits
   `LlmStepEnd::FinalMessage { text: "...file contents
   summarized..." }`. Asserts: the tool was invoked, the
   output contains the actual file bytes, the audit chain
   has `ToolCalled` + `ToolCompleted` entries bracketing the
   turn, scope denial on a bad path routes through
   `TurnOutcome::Completed` with the planner's error-text
   recovery rather than a loop-level failure. This test is the
   whole point of the phase — it proves the scope system
   survives a round-trip through an LLM's tool-call JSON.
6. **Phase 4 exit.** Freeze `PHASE_4.md`, update `README.md`
   and `ROADMAP.md` for Phase 5, refine the Phase 5 entry with
   whatever Phase 4 taught us. Same dance as Phase 3 exit.

## Open questions

### Q1. Does `FsReadTool` live in `aivyx-core` or a new crate?

**Status:** open at phase entry. Must resolve before task 2.

D8 locks the workspace at 9 crates:

```
aivyx-core aivyx-capability aivyx-audit aivyx-llm
aivyx-channel aivyx-storage aivyx-memory aivyx-ui aivyx-policy
```

`aivyx-core` is where the `Tool` trait itself lives. A concrete
tool could:

1. **Live in `aivyx-core`.** Simplest. No amendment. But
   `aivyx-core` starts accumulating a "standard library of
   tools" grab bag, and future tools (network, shell, image)
   all end up in the same place.
2. **Live in `aivyx-channel`.** Unnatural — channel is about
   how messages flow, not about what tools do. Rejected.
3. **Spin up a new `aivyx-tool-fs` crate.** Clean separation.
   Requires a D8 amendment — the first amendment since the
   DESIGN.md split — because we'd be at 10 crates.
4. **Start an `aivyx-tools` umbrella crate** that will eventually
   hold fs, shell, network, image, etc. One amendment for N
   tools. Slightly bigger surface than `aivyx-tool-fs`, amortized
   across every future concrete tool.

**Leaning:** (1) for this phase. `aivyx-core::tools::fs`
module. Minimizes churn, avoids a premature amendment, and
when Phase 6 arrives with memory-as-tool we'll have *two*
concrete tools and a much clearer picture of whether the split
pays for itself. Revisit at Phase 4 exit based on how big
`aivyx-core` actually got. Note the pattern: amendments should
be driven by evidence, not by taxonomy.

### Q2. How does the planner see a tool's input shape?

`Tool::descriptor()` today returns a `ToolDescriptor` with a
name and a description. The LLM needs an input schema so it
can produce valid JSON. Three options:

1. Serde-derive the input struct, use `schemars` to emit JSON
   schema, embed in the descriptor. One new dep.
2. Hand-write the schema as a `serde_json::Value` in each
   tool's `descriptor()`. Zero new deps, verbose per tool.
3. Skip formal schema; put the shape in the description text
   and trust the LLM to read it. Cheapest, fragile on smaller
   models.

**Leaning:** (2) for Phase 4 (one tool, cheap to hand-write),
revisit at Phase 6 when there are >2 tools and hand-writing
starts to rot. This is a "defer the abstraction until it has
three concrete callers" move — same discipline as Phase 2's
`ToolRegistryExt` deletion.

### Q3. Does `fs.write` create a backup file?

Standard Unix convention is destructive writes. An Aivyx agent
writing to a user's filesystem has weaker context than a human
editor, so the failure mode of a bad write is worse. Options:

1. Plain write, trust the scope prefix to bound damage. Simple.
2. Atomic write (`tempfile` + rename). Prevents partial writes
   but not wrong-content writes.
3. Move existing file to `.aivyx-backup-<timestamp>` before
   writing. Prevents both, but clutters the directory.

**Leaning:** (2) for task 3. The atomic-vs-plain decision is
load-bearing for ctrl-C-during-write safety; the backup-vs-no-
backup decision is load-bearing for trust and can be revisited
once we see the first real mistake. Log the decision either
way.

### Q4. Does `fs.read` follow symlinks?

D4's prefix attenuation matches on the string form of a scope.
If the scope is `fs.read:/home/julian/notes/**` and a symlink
inside that tree points to `/etc/shadow`, following it would
bypass the scope prefix check entirely. Options:

1. Refuse symlinks that leave the prefix. Canonicalize first,
   check prefix against the canonicalized path.
2. Refuse all symlinks inside the prefix. Cautious, breaks
   real workflows (dotfiles).
3. Follow symlinks freely, trust the scope check. **Wrong** —
   this is the TOCTOU attack the scope system is supposed to
   prevent.

**Leaning:** (1). Canonicalize the path on every read, compare
the canonicalized path against the scope prefix, deny if it
escapes. The scope check happens *after* canonicalization —
this is the same fence the scope system assumes in its unit
tests, but Phase 4 is the first time we exercise it against
real filesystem primitives. Task 2's unit tests must cover
the symlink-escape case explicitly.

### Q5. How does a scope-deny surface to the user?

Two sub-questions. First, at the **tool boundary:** a denied
invocation returns `ToolError::ScopeDenied(...)` from the
registry's check, before the tool's `invoke` runs. The loop
already handles `ToolError` by feeding the error text back to
the planner as a tool result. That's the right shape — no
change needed. Second, at the **user channel:** today the
renderer prints `← tool[id] <summary>` for completed tools.
A denial shows up as a completed tool with an error summary.
That is *probably* right — the user sees "tool failed: scope
denied" and can course-correct — but may feel abrupt. Hold on
a decision until the integration test actually runs and we see
the rendered output.

## Exit criteria (draft — revised as work lands)

- [ ] `aivyx-core::tools::fs::FsReadTool` exists, implements
      `Tool`, and passes unit tests for happy-path, scope-denial,
      symlink-escape, byte-cap, nonexistent, and binary-file cases.
- [ ] `aivyx-core::tools::fs::FsWriteTool` exists with the
      equivalent unit-test coverage and atomic-write semantics.
- [ ] The `aivyx` binary registers both tools at a configurable
      sandbox root, and a manual `cargo run -p aivyx-channel --
      bin aivyx` session can actually read and write files under
      that root via a live LLM.
- [ ] A scripted integration test
      (`crates/aivyx-channel/tests/fs_tool_e2e.rs`) drives an
      LLM-scripted tool-call round-trip and verifies the audit
      chain includes `ToolCalled`/`ToolCompleted` pairs in the
      right order.
- [ ] `cargo test --workspace` green.
- [ ] `cargo test --workspace --all-features` green.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `DESIGN.md` still unchanged (streak to 4) **or** a single
      amendment file under `docs/amendments/` documents the one
      exception, referenced inline from the `DESIGN.md` section
      it supersedes.
- [ ] Q1 resolved and noted under "Decisions made during Phase 4
      that aren't in DESIGN.md" in the freeze doc, regardless of
      which option won.
- [ ] At least one Phase 3 queued refinement either landed or
      explicitly re-queued to Phase 5 with a reason.

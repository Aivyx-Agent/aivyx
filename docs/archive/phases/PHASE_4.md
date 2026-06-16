# Phase 4 — First Real Tool (FROZEN)

**Status:** Closed 2026-04-14
**Exit commit:** `999ce87` — *"Phase 4 task 5: scripted fs.read round-trip integration test"*
**Predecessor:** [PHASE_3.md](PHASE_3.md) (exit commit `fa0f4ea`)
**Successor:** to be scaffolded at Phase 5 entry — see [ROADMAP.md](../../ROADMAP.md)
**Contract:** [`../DESIGN.md`](../../../DESIGN.md) (Deliverables 1–8, all LOCKED — unchanged)

This document is a historical record. The tools it produced live in
`aivyx-core::tools::fs` (`FsReadTool`, `FsWriteTool`, plus the
`FsReadToolConfig` / `FsWriteToolConfig` builders that canonicalize
the sandbox root once at startup), `aivyx-channel::session`
(`SessionConfig.tools` field so the binary owns its registry), and
the reference binary at `crates/aivyx-channel/src/bin/aivyx.rs`
(which now reads `AIVYX_FS_ROOT`, creates the sandbox if missing,
and registers both tools at session start). This file explains
*how* it came together and what was deliberately left for Phase 5.

## Goal (as written at phase entry)

Implement the first **concrete `Tool`** — an agent-callable
capability that does real work on the host, goes through the
Phase 0 planner → registry → scope-check → audit pipeline end-to-
end, and is exercised by an LLM-driven integration test. First
phase where an Aivyx agent can *do things* beyond streaming text
back.

The leading candidate at phase entry was a filesystem tool,
because it is the cleanest stress-test of D4's **prefix-attenuated
scopes** (R1): `fs.read:/home/user/sandbox/**` has to admit reads
inside the sandbox and deny reads elsewhere, with real paths, real
canonicalization, and real symlink-traversal decisions. Nothing
short of a filesystem tool exercises scope attenuation against a
live attacker surface.

## What shipped

- **`aivyx-core::tools::fs::FsReadTool`** (`3074b02`). Concrete
  `Tool` impl for reading UTF-8 or binary files under a sandbox
  root. Holds an `Arc<Path>` canonical root (pre-canonicalized at
  `FsReadToolConfig::build()` time so `required_scope` stays pure
  and the first fence never touches the filesystem), a stable JSON
  input schema built once via `OnceLock`-style lazy init, and a
  256 KiB read cap. `required_scope` uses a shared `lexical_resolve`
  helper to join `input.path` against the sandbox and collapse
  `.`/`..` components without any syscalls; lexical escapes fall
  back to a deny sentinel (`fs.read:/aivyx/__deny__/invalid-input`)
  so the loop's scope gate produces `Denied` at the audit layer.
  `execute` re-runs the lexical resolve and then adds a canonical
  fence (`std::fs::canonicalize` + `starts_with(sandbox_root)`) as
  the second TOCTOU-resistant check. Binary files are returned as
  `from_utf8_lossy` strings to keep the output JSON-serializable.
  18 unit tests cover: happy-path read, 0-byte file, exactly-at-cap
  vs one-over-cap truncation, missing path, non-string path,
  traversal via `..`, absolute path outside sandbox, intra-sandbox
  symlink followed, symlink that escapes the sandbox refused after
  canonicalization, nonexistent file, lexical-escape input produces
  deny sentinel, purity tripwire (`required_scope` against a
  nonexistent path proves no I/O happens there), the broad-scope-
  grants-narrow-scope case from lib.rs tests reprised against the
  fs shape, and the D4 negative (sandbox capability does not grant
  `/etc/passwd`).
- **`aivyx-core::tools::fs::FsWriteTool`** (`443ef2c`). Mirror of
  the read tool with atomic-write semantics. 8-step `execute`
  flow: validate string path + string content (content ≤ 256 KiB),
  lexically resolve the target (internal-error if the scope gate
  let a lexical escape through), split into parent + filename and
  refuse to clobber the sandbox root itself, `create_dir_all` the
  parent (only inside the sandbox), canonicalize the parent
  directory and re-check `starts_with`, handle an existing target
  (canonicalize it, refuse if it escapes, explicit `remove_file`
  if it's a symlink — "replace the link, not what it points at"),
  write to `<parent>/.aivyx-fswrite-<uuid>.tmp` via `create_new`
  then `rename` (POSIX atomic within one filesystem), and re-stat
  via `metadata().len()` to emit `Verification::Verified` or
  `Verification::Unverified` (D1: "tool success ≠ intent
  completed"). 19 unit tests: happy-path write, overwrite existing,
  binary content, content cap, parent-creation inside sandbox,
  traversal rejection, refuse-sandbox-root clobber, denial when
  target symlink escapes, intra-sandbox symlink overwrite
  semantics (link becomes a regular file, original target keeps
  its old content), D4 cross-base negative (`fs.write` sandbox
  capability does not grant `fs.read`), verified/unverified
  reporting, tmp file cleanup on error. Atomic-write justification
  for Q3: partial writes across a ctrl-C are the dominant failure
  mode; backup files over-defend for the tool's intended audience
  (an LLM that's about to overwrite a file under human watch).
- **`FsReadToolConfig` / `FsWriteToolConfig`** (shipped with the
  tools). Builder types whose `build()` method canonicalizes the
  sandbox root once, produces an infallible `FsReadTool` /
  `FsWriteTool` holding the canonical `Arc<Path>`, and surfaces
  config errors (missing root, non-directory root) at *startup*
  rather than at tool-call time. The binary maps these errors to
  user-facing strings during `run()`; tests assert on the specific
  failure shapes (`build_fails_if_root_does_not_exist`,
  `build_fails_if_root_is_a_file_not_directory`).
- **`Tool` trait surface audit** (`830c8dc`). A test-only
  `tool_surface_audit` module inside `planner.rs` that builds a
  skeleton `FsReadSkeleton` shaped exactly like the real tool
  (sandbox_root field, R1 path-derived scope, stable JSON schema
  via `OnceLock`) and proves the existing Phase 0–3 trait surface
  supports it with **no contract amendment**. Four tests exercise
  every registry-lookup path the LLM planner uses (`get`,
  `find_by_name`, `iter_tools`), the R1 purity invariant, the
  broad-capability-grants-narrow-scope case, and the negative
  (sandbox capability does not grant `/etc/passwd`). Task 1 is
  the only task in Phase 4 that would have been a no-op if the
  Phase 0 contract were wrong — its job was to catch contract
  drift *before* task 2 invested in a real filesystem tool. It
  found none.
- **`SessionConfig.tools` field + registry wiring** (`be38084`).
  `run_session` previously built a hardcoded empty
  `ToolRegistry::new(Vec::new())` inline (Phase 3's "first real
  channel, not first real tool" compromise). Phase 4 moved that
  registry onto `SessionConfig` as an `Arc<ToolRegistry>` field so
  the binary can register real tools at startup while the Phase 3
  chat-only integration test keeps passing an empty one and stays
  a narrow regression of the session loop itself. The planner
  factory closure in `run_session` still clones the registry per
  turn — no change there. One-line core change, plus the matching
  test-fixture update.
- **`AIVYX_FS_ROOT` + binary registration** (`be38084`). The
  `aivyx` binary now resolves a sandbox root from
  `AIVYX_FS_ROOT` (if set and non-empty) or `$HOME/aivyx-sandbox`
  (default), creates the directory with `create_dir_all` if it
  doesn't exist, builds both tools via their `Config::build()`
  methods, and uses the canonicalized root reported by
  `fs_read.sandbox_root()` to anchor the capability scopes
  (`fs.read:<canonical>/**` and `fs.write:<canonical>/**`). The
  *canonicalized* anchor is load-bearing: if the binary had
  formatted the scope from the raw `AIVYX_FS_ROOT` string, a
  symlink in `$HOME` could have widened or shifted the grant
  relative to what the tool compares against at execute time.
  The banner now prints the resolved sandbox path so a user
  sees immediately where the tools are allowed to write.
- **`fs_tool_e2e.rs` — LLM-driven round-trip** (`999ce87`). The
  whole point of the phase. A new integration test in
  `crates/aivyx-channel/tests/` that drives the full
  `ScriptedProvider → LlmPlanner → ConcreteAgent → FsReadTool →
  real filesystem → audit chain` pipeline with two scenarios:
  1. **Happy path.** Step 1 of the scripted provider emits
     `LlmStepEnd::ToolCall { name: "fs.read", input: {"path":
     <sandboxed file>} }`; the planner dispatches, the loop's
     scope gate admits, the tool reads the real file, the
     outcome flows back through `observe_tool_outcome` as a
     `ToolResult` appended to history, and step 2 of the
     provider emits a `FinalMessage`. Asserts: turn completes
     with one tool call, the final message streams through the
     channel, the planner's second-step history contains the
     tool result keyed by `call_id`, and the audit chain has
     exactly `[TurnStarted, ToolCall(Completed), TurnEnded]`
     with all three entries sharing the turn id.
  2. **Scope denial.** Same shape but the scripted tool call
     targets `/etc/passwd`. `FsReadTool::required_scope`
     produces its deny sentinel (because the lexical resolve
     returns `None` for an absolute path that isn't under the
     sandbox), the loop's scope gate denies, and the planner
     synthesizes a `{"error":"denied",...}` tool result for
     step 2 to observe. Asserts: turn still lands as
     `Completed` (D1: denials are normal loop outcomes, not
     loop-level failures), the audit chain is
     `[TurnStarted, ScopeDenied, TurnEnded(Completed)]` with
     **no** `ToolCall` entry (the loop's early-return path at
     `agent.rs:316-338` is what the negative assertion
     regression-tests), and `tool_calls_made == 1` in the
     final outcome.

## Decisions made during Phase 4 that aren't in DESIGN.md

### Q1 — `FsReadTool` lives in `aivyx-core::tools::fs`, not a new crate

The Phase 4 entry doc listed four options (core submodule, channel
crate, new `aivyx-tool-fs` crate, umbrella `aivyx-tools` crate) and
resolved to option 1 at task 2 entry. The reasoning, now frozen:

- Every type the filesystem tool needs (`Tool`, `ToolContext`,
  `ToolOutcome`, `Scope`, `Verification`, `AivyxError`) already
  lives in `aivyx-core`. A new `aivyx-tool-fs` crate would exist
  to `pub use` all of them back into visibility — pure overhead,
  and a D8 amendment to boot.
- One concrete tool is not evidence of a "tools umbrella" pattern.
  The right time to split is when there are at least two tools
  and the split pays for itself in compile-time isolation or
  test-surface isolation. Phase 4 shipped one tool that happens
  to have a read variant and a write variant.
- D8's nine-crate lock holds without amendment, which keeps the
  DESIGN.md empty-diff streak alive (now 4 phases running).

**To re-evaluate at Phase 6 entry.** Phase 6's memory-as-tool work
is where a second concrete tool lands, and the choice between
"keep piling into `aivyx-core::tools`" and "spin up a tools
umbrella" should be made *with evidence* rather than *with
taxonomy*. Whichever phase decides to split is the phase that
writes the first `docs/amendments/` file.

### Q2 — Hand-written `serde_json::Value` schemas, not `schemars`

At phase entry this was a "leaning (2) for now, revisit at Phase 6"
call. Phase 4 shipped with hand-written schemas stored in
`OnceLock`-initialized `Value`s inside each tool's struct (well,
morally — `read_input_schema_value()` / `write_input_schema_value()`
free functions built once in `Config::build()`). Zero new deps,
~10 lines per tool, and the two tools' schemas are nearly
identical anyway so the duplication cost is real but tiny. Phase 6
will face N > 2 and should reconsider.

### Q3 — Atomic temp+rename writes, no backup

Resolved for `FsWriteTool`. Write path creates
`<parent>/.aivyx-fswrite-<uuid>.tmp` via `OpenOptions::create_new`,
drops the handle, then `rename`s to the final target. POSIX
guarantees atomicity within one filesystem; a ctrl-C at any point
leaves either the old file or the new file, never a half-written
file. Explicit `remove_file` on an existing symlink target before
rename ensures "replace the link, not what it points at"
semantics regardless of filesystem (belt-and-suspenders against
NFS/fuse quirks).

**No backup file.** The Phase 4 entry doc floated a
`.aivyx-backup-<ts>` option; dropped because backup clutter
compounds across turns, the scope prefix is already a hard blast-
radius bound, and the LLM's consent model is "the user is watching
this happen in the terminal" — undo is a reversed `fs.write` call,
not a hidden backup file. Revisit if a real incident shows that
the scope prefix alone is insufficient.

### Q4 — Canonicalize-then-compare symlink policy (two-fence design)

Every `FsReadTool` / `FsWriteTool` call runs two fences:

1. **Lexical fence** inside `required_scope`. Pure-logic
   `lexical_resolve`: join input path against the sandbox root,
   collapse `.` / `..` components without any I/O, verify the
   result still starts with the sandbox root's components.
   Produces either a scope like `fs.read:<abs-joined-path>` or
   the deny sentinel (`fs.read:/aivyx/__deny__/invalid-input`).
   The loop's scope gate runs against the effective capability
   set, so a lexical escape becomes a `Denied` outcome at
   `agent.rs:316-338` with an audit `ScopeDenied` entry.

2. **Canonical fence** inside `execute`. Re-parse input, re-run
   the lexical resolve (internal error if the result changes —
   the gate should have caught it), then `std::fs::canonicalize`
   the target (reads) or the *parent directory* (writes, because
   the target may not exist yet), and check `starts_with` against
   the canonicalized sandbox root. This is the TOCTOU-resistant
   step the lexical fence cannot perform, because only the
   kernel can tell you whether a symlink inside the sandbox
   points somewhere outside it.

The two fences catch different attack classes: the lexical fence
catches naive traversal (`../../../etc/passwd`, absolute paths
outside the sandbox) before any I/O; the canonical fence catches
symlinks that lexically look fine but physically escape the
sandbox. A tool test for each class ships with task 2.

**`required_scope` stays pure.** This is the invariant tripwire
test `scope_is_pure_no_io` exists to defend: the scope derivation
path never calls into the filesystem, even when asked about a
nonexistent file. The canonicalization cost is paid exactly once
at `Config::build()` time (the sandbox root) and once at
execute-call time (the target) — never inside the R1 step.

### Q5 — Scope-deny shows up as `TurnOutcome::Completed`, not as a loop failure

Confirmed empirically in `fs_tool_e2e.rs` test 2. The loop emits a
`ScopeDenied` audit entry via an early return and feeds the
planner a synthetic `{"error":"denied",...}` tool result via
`observe_tool_outcome`; the planner uses that to decide its next
step, which in the test's case is a "can't read that path" final
message. The turn lands as `Completed` with `tool_calls_made == 1`.
No user-channel treatment beyond what the renderer already does
for any tool — denials render as a completed tool with an error
summary. Revisit if real usage shows this feels abrupt.

### `ToolCall` and `ScopeDenied` are disjoint audit events, not redundant views

This one is not in the Phase 4 entry doc — it was discovered while
writing `fs_tool_e2e.rs` test 2. The audit loop at
`agent.rs:316-338` takes an early return after emitting
`ScopeDenied`, so a denied call never reaches the `ToolCall`
append site at `agent.rs:358-365`. The invariant is therefore
*exactly one audit entry per tool call attempt, of one of the two
disjoint kinds* — not "both views of the same call." The
integration test's absence assertion (`no ToolCall entry on
denial`) regression-tests this. PHASE_4.md's task-5 prose said
"`ToolCalled` + `ToolCompleted` entries bracketing the turn,"
which was pre-implementation naming that didn't match the real
enum shape (`AuditEvent::ToolCall { outcome }` plus a separate
`AuditEvent::ScopeDenied` variant). Corrected in the freeze, no
doc amendment needed — the phase journal is allowed to be wrong
until the freeze, which is what this section exists to catch.

## Bugs caught in Phase 4

- **`FsReadTool` missing `Debug` derive.** Task 2's unit tests
  used `.expect_err(...)` on `ToolOutcome::Failed` cases, and
  `Result::expect_err` requires `T: Debug`. First compile error
  after the test file landed. Fixed by adding `#[derive(Debug)]`
  to the struct; all 18 tests passed on the next run. Tiny,
  but worth noting because the same requirement will land on
  every future concrete tool — the ergonomic default is "derive
  Debug on tool structs unless you have a secret-y reason not to."
- **`Scope::parse` returns `Option`, not `Result`.** Task 4's
  first-draft binary called `.map_err(|e| format!(...))` on the
  `fs.read:<canonical>/**` and `fs.write:<canonical>/**` parse
  results. Clippy was not reached; the type checker stopped the
  build immediately. Fixed with `.ok_or_else(|| format!(...))`.
  Muscle-memory-from-rust bug — `Result::map_err` is the default
  reach for fallible ops, and the scope DSL is narrow enough
  that "can't parse" collapses to a single bit. Any future code
  that parses a scope from a user-controlled string needs the
  `Option` ergonomics, not `Result`.
- **`ToolOutcomeSummary::Completed` is a struct variant.** First
  draft of `fs_tool_e2e.rs` asserted
  `assert_eq!(*outcome, ToolOutcomeSummary::Completed)`. The
  enum variant is actually `Completed { verified:
  VerificationSummary }`, so the assertion didn't even compile.
  Rewrote as `matches!(outcome, ToolOutcomeSummary::Completed
  { .. })` which is more maintainable anyway — a future
  addition of a field to the summary shouldn't force every
  integration test to relearn the shape.
- **Assumed `ScopeDenied` and `ToolCall(Denied)` coexist.** First
  draft of test 2 asserted both entries in the denial audit
  chain. The test failed at the second assertion; the debug
  dump showed only three entries (`TurnStarted`, `ScopeDenied`,
  `TurnEnded`). Investigated: the loop returns early at
  `agent.rs:316-338` after emitting `ScopeDenied`, so
  `ToolCall` is never appended. Corrected the test to assert
  the *disjoint* invariant (exactly one or the other, never
  both) — this is a cleaner check than the original assumption
  and caught a real misconception I had about the audit
  vocabulary. Decision recorded in "Decisions made during
  Phase 4 that aren't in DESIGN.md" above.
- **Deny scope qualifier is a sentinel, not the input path.**
  Related to the previous bug. After fixing the chain shape,
  the next draft of test 2 asserted
  `scope_requested.qualifier() == Some("/etc/passwd")`. The
  real value is `Some("/aivyx/__deny__/invalid-input")` because
  `FsReadTool::required_scope` falls back to the deny sentinel
  when `lexical_resolve` returns `None`. Asserting the sentinel
  is actually *more* informative than asserting the input path
  — it proves the lexical-escape code path ran, not just that
  some scope was denied.
- **Post-rename reference in `FsReadToolConfig::build`.** Task 3
  renamed `input_schema_value` → `read_input_schema_value` (so
  the write tool could have its own `write_input_schema_value`)
  and missed one call site at the bottom of `FsReadToolConfig::
  build`. Caught by grep before running tests; fixed via
  targeted edit. Lesson: when renaming a helper used in more
  than one place, grep for the old name across the whole module
  before assuming the first edit covered it.

## Decisions deferred to Phase 5+

- **`fs.list` / `fs.stat` / `fs.mkdir` / `fs.delete`.** Not
  shipped. Phase 4 is about *one* tool deeply wired, not a
  filesystem catalog. The two tools that shipped (`fs.read`,
  `fs.write`) are enough to prove the R1 round-trip; more
  tools are a Phase 6+ surface decision, not a Phase 4 oversight.
- **`fs-tool-umbrella` crate split.** Q1's leaning is to hold
  the umbrella until Phase 6 lands a second concrete tool. If
  memory-as-tool + the filesystem tools together look like "a
  standard library of tools," Phase 6 gets to write the first
  D8 amendment; if they don't, the `aivyx-core::tools` module
  keeps its residency.
- **Runtime JSON schema validation for tool input.** Phase 4's
  tools do minimal shape checking in `execute` (is `path` a
  string? is `content` a string? is it under the byte cap?) and
  trust the LLM to produce well-formed input otherwise. A real
  schema validator (via `schemars` or hand-written) lands when
  there are enough tools that hand-validating every input field
  in every tool stops being cheaper than centralizing it.
- **`fs.read` binary content encoding.** Phase 4 returns binary
  files via `from_utf8_lossy`, which is *lossy* in the literal
  sense — invalid UTF-8 becomes replacement characters. For a
  chat LLM summarizing a text file this is fine; for an agent
  that wants to hash a binary it's wrong. Phase 6+ can revisit
  with either a `{"encoding": "base64", "bytes": "..."}` output
  shape or a dedicated `fs.read_bytes` variant. No forcing
  function yet.
- **Tool name in `StreamEvent::ToolCallStarted`.** Still a short
  ID, not a name. Phase 3 queued this as "watch for it to become
  painful"; Phase 4 shipped tools and did not find it painful
  enough to re-queue. Renderer-side registry lookup remains the
  preferred fix when it does.
- **`CapabilitySet::default()` ergonomics.** Still not added.
  Phase 4 constructs capability sets via `from_scopes([...])`
  in three places (the binary, the cli_e2e test, the
  fs_tool_e2e test); none of them wanted an empty default
  enough to motivate the addition. Phase 5 will probably take
  it when the storage layer needs a "no capabilities required"
  sentinel.
- **Unicode-normalized path comparisons.** Today's fence uses
  byte-level `Path::starts_with`. A macOS HFS+ filesystem
  normalizing NFD ↔ NFC could in theory let a visually-identical
  filename round-trip through canonicalize and still match.
  Not exercised in tests, not defended against. Linux-first
  phase; cross-platform polish waits.
- **`rustyline` line editing.** Still bare `stdin().read_line`
  in the binary. Not touched because typing a tool-call prompt
  at a real LLM in Phase 4 was not ergonomically painful
  enough to motivate the dep.
- **Opt-in live-API test in CI.** Still runnable by hand only
  (`cargo test -- --ignored`). Not a Phase 4 blocker. Phase 5+
  ops decision.

## Lessons carried forward

- **Scope derivation stays pure; canonicalization is the second
  fence, not the first.** The two-fence design is the phase's
  most important structural idea. Keeping `required_scope`
  pure (no I/O) means the scope gate at the loop can compare
  thousands of inputs per second without touching the
  filesystem, and `execute`'s canonical fence catches the
  cases the pure fence cannot. Any future tool that needs to
  scope-check against filesystem state (memory, storage,
  network) should copy this shape rather than invent its own.
  The tripwire test `scope_is_pure_no_io` exists to make
  regressions in this invariant loud.
- **Canonicalize at startup, not at every call.** The sandbox
  root is canonicalized exactly once in `FsReadToolConfig::
  build()`, stored as an `Arc<Path>`, and shared across every
  concurrent turn. The cost of `fs::canonicalize` is paid at
  binary startup where it's invisible, not on the tool-call
  hot path where it would compound. The same trick generalizes
  to any per-session resource whose canonical form is stable:
  compute once at session build time, share via `Arc`, never
  re-derive.
- **Config-error-at-startup, not config-error-at-call-time.**
  `FsReadToolConfig::build()` returns `Result<FsReadTool,
  AivyxError>`, failing if the root doesn't exist or isn't a
  directory. The binary surfaces this as a startup error the
  user sees immediately (`aivyx: failed to build fs.read tool:
  ...`). The alternative — an infallible `FsReadTool::new()`
  that fails later at the first tool call — would move the
  error from startup-visible to LLM-visible, which is strictly
  worse for operational clarity.
- **Atomic temp+rename over plain-write-with-backup.** The
  failure mode that matters for an agent-driven write is
  ctrl-C across a partial write; the failure mode that matters
  for a human-driven write is "oops, wrong content." The tool
  defends against the first (atomicity) and not the second
  (no backup) because scope-prefix blast-radius bounding is
  already a stronger guarantee than a backup file could be —
  a backup doesn't help if you've clobbered the one file the
  user cared about, but a scope prefix prevents the clobber.
- **The freeze doc corrects the working-journal wording.**
  PHASE_4.md as a working journal said "ToolCalled / ToolCompleted"
  bracketing the turn; the real enum has a single `ToolCall`
  variant with an `outcome` field plus a disjoint `ScopeDenied`.
  The freeze doc records the real names so a future phase that
  greps for `ToolCalled` finds the explanation. Phase journals
  are allowed to drift from the eventual implementation; freezes
  are not.
- **`DESIGN.md` empty-diff streak: 4.** Phase 1 → Phase 2 →
  Phase 3 → Phase 4 all exited with zero contract changes. The
  R1 input-derived scope signature and the nine-crate lock
  both held through their *first concrete user*, which is when
  a specification usually breaks. Phase 0 work keeps
  compounding: every phase that holds makes the next phase's
  hold easier, because there are fewer "this was always wrong"
  sentences to revisit. Phase 5's encrypted-storage work will
  stress D7 for the first time; the streak may or may not
  survive, and the point of tracking it is that the decision
  will be visible as a diff, not silent.
- **Defense-in-depth is cheaper when the two layers are at
  different abstraction levels.** The lexical fence is pure
  Rust logic; the canonical fence is a syscall. They catch
  disjoint attack classes (naive traversal vs symlink escape)
  and neither is redundant with the other. If both fences were
  at the same level — say, two lexical resolves with slightly
  different rules — the second would be dead code as soon as
  someone noticed the duplication. Because they speak to
  different threats, they stay load-bearing even when a
  reviewer asks "why do we do this twice?"
- **Commit per task, not per phase. Still.** Five Phase 4 task
  commits (`830c8dc` → `999ce87`), each building and testing
  green in isolation, each with a commit message that states
  the substantive change plus the test counts and DESIGN.md
  streak status. Same discipline as Phase 3; same bisect
  payoff if a future regression lands.

## Exit criteria (all met)

- [x] `aivyx-core::tools::fs::FsReadTool` exists, implements
      `Tool`, and passes unit tests for happy-path, scope-denial,
      symlink-escape, byte-cap, nonexistent, and binary-file cases
- [x] `aivyx-core::tools::fs::FsWriteTool` exists with equivalent
      unit-test coverage and atomic-write semantics
- [x] The `aivyx` binary registers both tools at a configurable
      sandbox root (`AIVYX_FS_ROOT`, default `$HOME/aivyx-sandbox`),
      creates the root if missing, and anchors capability scopes
      on the canonicalized path
- [x] A scripted integration test
      (`crates/aivyx-channel/tests/fs_tool_e2e.rs`) drives an
      LLM-scripted tool-call round-trip and verifies the audit
      chain shape for both the happy path and the scope-denial
      path
- [x] `cargo test --workspace` green (150 tests, 1 ignored)
- [x] `cargo test --workspace --all-features` green (same totals)
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [x] `DESIGN.md` unchanged since `e0d6437` (streak of 4 —
      `git diff e0d6437..HEAD -- DESIGN.md` is empty)
- [x] Q1 resolved and noted under "Decisions made during Phase 4
      that aren't in DESIGN.md" (tools live in `aivyx-core::tools`,
      to be re-evaluated at Phase 6 entry)
- [x] At least one Phase 3 queued refinement addressed: `fs` tools
      now drive the `ChannelContext` seam for real (re-queued
      refinements: tool name in `ToolCallStarted`, `CapabilitySet::
      default()`, `rustyline`, opt-in live CI test — see "Decisions
      deferred to Phase 5+" above)

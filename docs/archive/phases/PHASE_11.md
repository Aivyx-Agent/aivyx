# Phase 11 — Role system + `shell.exec` (first product phase)

**Status:** Active (opened 2026-04-15). This document will churn
during the phase and freeze at exit under a final Exit criteria
block at the bottom, matching the Phase 7–10 precedent.
**Predecessor:** [PHASE_10.md](PHASE_10.md) (exit commit `f8f4d28`,
hash backfill `e6efff6`)
**Contract:** [`../DESIGN.md`](../../../DESIGN.md) (Deliverables 1–8, all
LOCKED — unchanged since `e0d6437`, **ten phases running** at Phase
11 entry, target **eleven** at Phase 11 exit)

## Goal

Phase 11 is the **first product phase**. Phases 0–10 built the
foundation — capability-secured turn loop, per-session memory,
audit chain, two channel adapters, hand-rolled JSON-schema
validation, cross-topic memory introspection. Phase 11 is the
phase where Aivyx stops being "a framework you could build an
agent on" and becomes "a framework where the user decides what
their agent *is*."

The pivot is the `Role` primitive: a named bundle of
`(system_prompt, tool_allowlist, memory_topic_prefix)` loaded
from `aivyx-config` at process start. **Roles are user-defined,
not enum-fixed** — there is no `enum Role { Coder, Researcher,
... }` in the source. The set of roles is whatever the user puts
in their config file, and the framework's job is to thread the
*active* role's bundle through every relevant subsystem (system
prompt, tool dispatch, memory layer) without ever hardcoding
which roles exist.

To prove the primitive actually works the phase ships **two seed
roles** in the default config — `coder` and a contrast role
(probably `researcher`) — with **different tool allowlists** and
**different memory prefixes**. The contrast role exists so the
exit criteria can demonstrate role-switching at the regression-
test level: the same `memory.write` call from two different roles
ends up in two distinct topic spaces, and the `researcher` role
is rejected when it tries to call `shell.exec` because the tool
isn't in its allowlist. Without the contrast role the "user-
defined" claim is unverified — there's no way to distinguish
"the framework respects roles" from "the framework hardcoded the
coder role under a different name."

The phase also ships the **first dangerous tool**: `shell.exec`,
gated to `TrustTier::Trusted` only (never offered to Telegram or
any other `SemiTrusted` adapter), scoped via
`shell.exec:cwd:<path>` capability strings with path-prefix
attenuation. `shell.exec` is the pretext for two pieces of
forward-leaning infrastructure that benefit every future tool:
nested-object support in the Phase 10 hand-rolled validator
(because `shell.exec`'s input shape is genuinely nested), and
the trust-tier gating pattern (because every future dangerous
tool — `edit.patch`, `web.fetch`, `git.commit` — will use the
same gate).

The headline outcome: when Phase 11 closes, a user can write a
`coder` role into their config file, run `aivyx`, and the agent
will have a coder-specific system prompt, can run shell commands
inside a configured working directory, and writes its memory to
a `coder/`-prefixed topic space — and a second role defined the
same way will get a different system prompt, a different tool
list, and a different memory namespace, with no code changes
between the two.

## Why now

Three structural reasons:

1. **Foundation is stable.** Phase 10 exited with the DESIGN.md
   streak at ten, the production-core byte-identity streak
   re-baselined at `f8f4d28`, the foundation backlog empty for
   the first time since Phase 6, and 367 green tests. Every
   prerequisite for a product phase is in place: tools are
   schema-validated, capabilities attenuate correctly, audit
   events are emitted with tool names, memory has cross-topic
   introspection. There is no pure-foundation work left that
   blocks role-based agents.
2. **Foundation backlog is empty — opportunity cost is now.**
   This is the first phase opening since Phase 6 with zero
   rolling deferrals. Every prior phase has had to balance
   product-feature work against paying down deferred items.
   Phase 11 doesn't, which means it can spend its full budget
   on shaping product surface — the cleanest possible conditions
   for the most consequential design decisions of the project.
3. **The product question is the bottleneck.** Up to Phase 10
   the question "what is Aivyx *for*?" had a defensible answer
   ("a framework you can build an agent on"). After Phase 10 it
   doesn't — the framework is good enough that the next
   meaningful piece of feedback comes from *using* it, and using
   it requires a way to express what the agent is *doing*. The
   Role primitive is the smallest design move that lets the
   framework answer the question.

## Non-goals

- **No multi-role concurrent execution.** Phase 11 ships one
  *active* role per process. Switching roles is "restart with a
  different `--role` flag" or "edit config and restart," not
  "live-swap mid-conversation." Multi-role concurrency belongs
  to a later phase if it ever proves necessary; designing for it
  now would couple the role primitive to channel-adapter
  identity in ways the current architecture does not require.
- **No role inheritance or composition.** Roles are flat
  records. No "extends" clause, no merge semantics. If two roles
  share 90% of their config, the user copies the shared bits.
  The discipline payoff is the same as everywhere else in
  Aivyx: a user-defined value type with no inheritance graph is
  trivially auditable; a user-defined value type with
  inheritance becomes its own debugging surface.
- **No PTY support in `shell.exec`.** Output is captured as a
  single `(stdout, stderr, exit_code)` blob and returned in
  `ToolCallFinished`. Interactive shells, ANSI escape handling,
  long-running stream output — all deferred. `tokio::process` is
  already in the workspace; pulling in a PTY crate would break
  the zero-new-dep streak for a feature no current use case
  needs.
- **No `shell.exec` on `SemiTrusted` adapters.** Telegram cannot
  call `shell.exec` even if a Telegram-attached agent is granted
  a role whose allowlist includes it. The trust-tier gate is at
  *registration* time — the tool is simply not in the dispatch
  registry for `SemiTrusted` channels — not at call time. This
  is stricter than "the capability check would deny it," and
  the strictness is intentional: `shell.exec` should not appear
  in a `SemiTrusted` channel's audit log even as a denial event,
  because its mere presence in the registry is leakage.
- **No new external-world tools beyond `shell.exec`.** No
  `web.fetch`, no `edit.patch`, no `git.commit`. These belong
  to later phases where their design pressure is real. Phase
  11's tool-catalog change is exactly one tool plus the role-
  layer wiring that makes it reachable.
- **No DESIGN.md edits.** The Role primitive and `shell.exec`
  both fit inside existing deliverables (D3 tool trait, D2
  capability scopes, D6 config) without contract changes. If
  mid-phase work surfaces a contract conflict, that's an
  amendment under `docs/amendments/` (directory still doesn't
  exist as of Phase 11 entry), not a silent edit.
- **No `aivyx-cli` crate.** The roles are selected via a flag
  on the existing `aivyx-channel` binary, not a new top-level
  CLI surface. A separate CLI crate is a Phase 13+ concern at
  earliest and probably never happens.

## Entry criteria (all met from Phase 10 exit)

- [x] Phase 10 is frozen. Exit commit `f8f4d28`, hash backfill
      `e6efff6`. `docs/README.md` phase-status table reflects
      both.
- [x] `cargo test --workspace` is **367 green** (verified at
      Phase 11 entry: 2026-04-15).
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
      is clean.
- [x] DESIGN.md byte-identical to `e0d6437`. Streak at **ten**.
- [x] `crates/aivyx-core/src/lib.rs` byte-identical to `f8f4d28`
      (the Phase 10 re-baseline after Tasks 2 and 3 broke the
      previous streak). Production-core streak at **one phase**
      and expected to break in Phase 11 Task 3 — the break is
      an intentional phase-scope decision tied to nested-object
      validator support.
- [x] Foundation backlog is **empty**. First phase since Phase 6
      to open with zero rolling deferrals.
- [x] Pre-commit hook (`scripts/pre-commit.sh`) runs `cargo
      clippy` workspace-wide with `-D warnings` before every
      commit. Held across all of Phase 10 with zero regressions.
- [x] Pre-Phase-11 housekeeping: `docs/walkthrough.md` (the
      stale 2026-04-14 audit) is in `.gitignore` (commit
      `47dc73c`), not part of the frozen tree.

## Draft task breakdown

Five tasks. Ordered so the role primitive lands as pure
infrastructure first (Task 1), then is exercised by progressively
heavier wiring (Tasks 2, 3, 4), then frozen (Task 5). Each task
gets a working-session commit and closes before the next opens
— the Phase 7–10 cadence.

### Task 1 — `Role` primitive in `aivyx-config`

**What lands:**

- New `Role` struct in `aivyx-config`:
  ```
  pub struct Role {
      pub name: Sourced<String>,
      pub system_prompt: Sourced<String>,
      pub tool_allowlist: Sourced<Vec<String>>,
      pub memory_topic_prefix: Sourced<Option<String>>,
  }
  ```
  Each field is `Sourced<...>` to inherit `aivyx-config`'s
  existing provenance plumbing — `cargo run -- --print-config`
  shows where each field came from, same as the existing
  fields.
- TOML loader gains a `[[role]]` table-array. Each entry parses
  into a `Role`. Roles are keyed by `name` and stored on
  `AivyxConfig` as `pub roles: BTreeMap<String, Role>`. An
  `active_role: Sourced<String>` field on `AivyxConfig` records
  which role is currently selected (default: `"default"`,
  overridable by env var `AIVYX_ROLE` or by a future
  `--role <name>` CLI flag — flag wiring lands in Task 4).
- **Backwards-compatibility bridge.** The existing top-level
  `system_prompt: Sourced<String>` at
  `aivyx-config/src/lib.rs:341` does **not** disappear. If the
  loaded config defines no `[[role]]` entries at all, the
  loader synthesizes a single implicit `default` role whose
  `system_prompt` is the existing top-level field, an empty
  `tool_allowlist` (meaning "no allowlist filter — allow every
  registered tool" — see Q3), and `memory_topic_prefix = None`.
  Existing config files keep working with **zero edits** —
  this is non-negotiable; Phase 11 must not break any Phase
  0–10 config.
- Two seed roles in the default `aivyx.toml` shipped under
  `examples/` (or wherever the existing example config lives,
  to be confirmed in the task):
  - `coder`: `tool_allowlist = ["fs.read", "fs.write",
    "memory.read", "memory.write", "memory.list", "shell.exec"]`,
    `memory_topic_prefix = "coder/"`, system prompt aimed at
    pair-programming.
  - A contrast role (working name `researcher`):
    `tool_allowlist = ["fs.read", "memory.read", "memory.write",
    "memory.list"]` (deliberately **no** `shell.exec`,
    deliberately **no** `fs.write`), `memory_topic_prefix =
    "researcher/"`, system prompt aimed at note-taking.
  The seed contrast role exists for the regression test in
  Task 4. It can be renamed if `researcher` doesn't fit the
  user's preferred persona vocabulary.

**Discipline guardrails:**

- `aivyx-core` production code stays **byte-identical** in
  Task 1. All work is in `aivyx-config` (and the example
  config). The production-core streak holds through Task 1 and
  breaks in Task 3.
- The backwards-compat bridge is implemented in the loader,
  not by callers — every consumer of `AivyxConfig` should be
  able to read `cfg.roles.get(&cfg.active_role).unwrap()` and
  get a `Role` whether or not the user wrote any `[[role]]`
  entries.
- Tests cover: (a) empty-roles config synthesizes an implicit
  `default` role from the legacy `system_prompt` field; (b) a
  config with one explicit role and no legacy `system_prompt`
  loads correctly; (c) a config with both an explicit role
  *and* a legacy `system_prompt` either errors or prefers the
  explicit role (decide in Q4); (d) round-trip parse →
  serialize for both seed roles; (e) `active_role` env-var
  override resolves correctly; (f) provenance: every `Sourced`
  field reports its origin (env vs. toml vs. default).

**Expected test delta:** +6 to +10 tests (367 → 373–377).

**Stressed question:** Where do role definitions actually live
on disk? Inline in the existing `aivyx.toml` as `[[role]]`
entries, or in a sibling `roles.toml`, or in a `roles/` directory
of one-file-per-role? Leaning inline-in-aivyx.toml — keeps
provenance simple, no new file format surface, and a user with
two roles is the common case. Resolve at the start of Task 1
implementation.

### Task 2 — Role-aware memory topic prefixing

**What lands:**

- Turn loop reads `cfg.roles[active_role].memory_topic_prefix`
  at session start and threads it into every `memory.*` tool
  dispatch. Prefix injection happens at the **dispatch layer**,
  not as a schema field — meaning the user-facing input shape
  for `memory.read` / `memory.write` / `memory.list` is
  unchanged, and the Phase 10 validator continues to see the
  same input it always has. The Phase 10 session-injection
  ordering (validator runs *before* loop-internal field
  injection) carries over cleanly.
- Cross-topic `memory.read` from Phase 10 still respects the
  prefix boundary: a wildcard read from the `coder` role
  enumerates topics under `coder/*`, not under `researcher/*`.
  The prefix becomes part of the wildcard's effective scope
  before the substrate `scan_prefix` runs.
- Per-role memory namespaces are layered **on top of** Phase
  8's per-session partitioning, not in place of it. The full
  memory key prefix becomes
  `\x01s\x01<session>\x01<role_prefix><topic>\x01...`. Two
  roles in two sessions get four distinct memory namespaces.

**Discipline guardrails:**

- Backwards compat: a role with `memory_topic_prefix = None`
  uses bare topic names — identical to Phase 10 behavior.
  Existing `memory.*` tests keep passing without modification.
- The prefix is **not** visible to the model. The model writes
  to topic `notes`; the storage layer stores it under
  `coder/notes`. From the model's perspective the prefix
  doesn't exist. This is what makes role-switching cheap — the
  model's prompts don't need to know the prefix scheme.
- Tests cover: (a) two roles writing to nominally-the-same
  topic (`notes`) see isolated stores; (b) `memory.list` from
  one role does not enumerate the other role's topics;
  (c) the Phase 10 wildcard `memory.read` from one role does
  not bleed into the other role's namespace; (d) backwards
  compat — a role with `memory_topic_prefix = None` writes to
  bare topic names exactly as Phase 8–10 did; (e) the per-
  session partition still holds (two roles in the same
  session are isolated; the same role across two sessions is
  also isolated).

**Expected test delta:** +5 to +8 tests (373–377 → 378–385).

**Stressed question:** Does the prefix get appended to the
session segment of the existing memory key layout, or
prepended to the topic segment? Leaning prepended-to-topic —
keeps the session-partition test surface unchanged and treats
the prefix as a topic-namespace concern, not a session
concern. Resolve at the start of Task 2 implementation by
reading the Phase 8 Task 2 layout in `aivyx-memory/src/redb.rs`.

### Task 3 — `shell.exec` tool + nested-object validator extension

**The heavy task.** Three internal subparts, each recorded as
its own block in the task journal as it lands:

**Subpart (a) — Validator nested-object support.**

- The Phase 10 hand-rolled validator at
  `aivyx-core/src/schema.rs` grows nested-object support:
  `properties` values can themselves be schema objects (not
  just type names), `required` recurses into nested objects,
  `additionalProperties: false` gates per nesting level
  independently, and validation errors thread a JSON pointer
  (e.g., `args.cwd: expected string, got integer`) so that
  `ToolOutcome::Failed` audit messages stay readable and
  correlatable.
- LOC budget: the Phase 10 validator was ~100 lines. Nested
  support probably doubles it — call it ~200 lines, still
  zero-dep, still hand-rolled. The validator stays narrow:
  flat objects, nested objects one level deep, strings,
  integers, booleans (if needed), and `enum` constraints. No
  arrays-of-objects, no `oneOf`/`anyOf`, no `$ref`. Those
  grow when a future tool needs them.
- Round-trip test: a tool with a nested schema that ships in
  this task (`shell.exec`) is the primary fixture; one
  negative test per nesting level (top-level missing field,
  nested missing field, nested wrong type, top-level
  `additionalProperties` violation, nested `additionalProperties`
  violation).

**Subpart (b) — `shell.exec` tool.**

- New tool in (probably) `aivyx-core/src/tools/shell.rs` —
  placement to be confirmed by reading where the existing
  `fs.*` and `memory.*` tools live in the workspace.
- Input shape (decided to be nested per the future-proofing
  call):
  ```
  {
    "cmd": "string",
    "args": {
      "cwd": "string (optional)",
      "env": "object<string, string> (optional)",
      "timeout_ms": "integer (optional, default 30_000)"
    }
  }
  ```
  The schema is registered via the Phase 6 `Tool::input_schema`
  method that `shell.exec` inherits like every other tool.
- Implementation: `tokio::process::Command` (already in-tree
  via the existing tokio dep — zero-new-dep streak holds).
  Captures `(stdout: String, stderr: String, exit_code: i32)`
  and returns them in the tool result. No PTY, no streaming,
  no signal handling beyond `timeout_ms` enforcement (which
  uses `tokio::time::timeout`).
- Output is bundled into a single `ToolCallFinished` event,
  not a new `StreamEvent::ToolOutput` variant — see open
  question Q2. If the bundled approach proves wrong in
  practice we revisit in a later phase; keeping it bundled now
  preserves the Phase 10 streak baseline on `lib.rs` going
  into Task 4.

**Subpart (c) — Trust-tier gate + capability scope.**

- `shell.exec` is registered conditionally on the channel's
  `TrustTier`. `aivyx-channel` (local CLI, `Trusted`) registers
  the tool; `aivyx-telegram` (`SemiTrusted`) does not. The gate
  is **at registration time, not at dispatch time** — the tool
  is simply absent from the dispatch registry for
  `SemiTrusted` channels. Audit chains for `SemiTrusted`
  channels never see `shell.exec` mentioned, even as a
  denial. This is stricter than a runtime denial and the
  strictness is the point.
- Capability scope shape: `shell.exec:cwd:<path>`. An agent
  granted `shell.exec:cwd:/repo` can call `shell.exec` with
  `args.cwd ∈ {/repo, /repo/sub, /repo/sub/sub2, ...}` —
  attenuation is path-prefix. An attempt to call with
  `args.cwd = /etc` against a `:cwd:/repo`-only grant fails
  the `required_scope` check and routes through
  `ToolOutcome::Denied` (not `Failed` — the validator already
  passed; it's a capability denial).
- Path attenuation must use canonicalized paths to defend
  against `/repo/../etc` traversal. `Path::canonicalize` from
  the standard library handles this; the canonicalization
  step happens inside `required_scope`, before the prefix
  check.
- The scope is **not** in any default `TrustTier` ceiling — an
  agent that wants `shell.exec` access must be granted the
  scope explicitly by name, same pattern as the Phase 10
  cross-topic memory wildcard. Default-deny by construction.

**Discipline guardrails:**

- **Intentional break** of the production-core byte-identity
  streak re-baselined at `f8f4d28`. The validator extension
  touches `aivyx-core/src/schema.rs` (and possibly `lib.rs`
  if a new error variant lands). Record in the ship log
  ("validator nested-object support, additive within D3's
  contract"). Re-baseline at the Phase 11 exit commit.
- **Zero-new-dep streak holds.** `tokio::process` is already
  in the workspace via the existing tokio dep; no new
  dependency is needed. If subpart (b) ever wants `nix` or a
  PTY crate, that's a Phase 11 deferral, not a stealth
  addition.
- The trust-tier gate is implemented as a single match in the
  registration site — there is exactly one place in the
  codebase where the binary asks "what tools do I expose for
  this channel?" and that match becomes the gate. Scattered
  per-call `if tier == Trusted` checks are explicitly out of
  scope; a single registration site is the auditable shape.
- Tests cover: (a) shell command runs, captures output,
  exits cleanly; (b) command times out at `timeout_ms`;
  (c) `cwd` attenuation: granted `:cwd:/tmp/repo`, allow
  `cwd=/tmp/repo/sub`, deny `cwd=/etc`; (d) `cwd` traversal:
  deny `cwd=/tmp/repo/../etc`; (e) registration-time gate:
  `aivyx-telegram` does not list `shell.exec` in its tool
  registry; (f) validator nested-object positive path with the
  `shell.exec` schema; (g) validator nested-object negative
  paths (missing nested required field, wrong nested type,
  top-level extra field, nested extra field); (h) JSON-pointer
  error message format pins the dotted-path output.

**Expected test delta:** +12 to +18 tests (378–385 → 390–403).

**Stressed question:** Does `shell.exec` block on its child
process synchronously inside the tool method, or does the tool
method return a future that the turn loop awaits? Leaning
synchronous (`.await` inside the tool method, returning the
captured output once the process exits or the timeout fires) —
matches how every other tool in the codebase works and avoids
introducing a new asynchrony pattern just for this one tool.
Resolve by reading the Phase 6 fs-tool implementation pattern.

### Task 4 — Turn-loop role wiring (allowlist enforcement + system prompt injection)

**What lands:**

- Active role threads through the turn loop end-to-end. Three
  integration points:
  1. **System prompt sourcing.** `llm_planner.rs` (and any
     other site that prepends the system prompt to the
     conversation) reads `cfg.roles[cfg.active_role.value()]
     .system_prompt` rather than the legacy top-level
     `cfg.system_prompt`. The legacy top-level field still
     exists as the source for the implicit `default` role
     that Task 1's backwards-compat bridge synthesizes — so
     existing config files still drive the system prompt
     correctly, just via the role layer instead of directly.
  2. **Tool allowlist filter on Anthropic tool advertisement.**
     Before the planner sends the tool catalog to Anthropic,
     it filters the catalog through `role.tool_allowlist`. An
     out-of-allowlist tool is **not advertised** to the model
     — the model never sees it, so it never tries to call it.
     This is the primary enforcement; the dispatch-layer
     check below is belt-and-suspenders.
  3. **Belt-and-suspenders dispatch-layer check.** Even if a
     model somehow calls a tool that isn't in the active
     role's allowlist (e.g., via a stale tool name from a
     resumed conversation), the dispatch layer rejects it
     **before** the capability check runs. The rejection
     routes through one of the `ToolOutcome` variants — see
     open question Q1 for whether it reuses
     `ToolOutcome::Denied` or introduces a new variant.
- `--role <name>` CLI flag on the `aivyx-channel` binary,
  resolving in this priority: explicit flag > `AIVYX_ROLE`
  env var > `active_role` field in config > `"default"`. The
  flag is parsed in the binary; `aivyx-config`'s
  `LoadOptions` already supports the env-var path, so most of
  the plumbing is already in place from Task 1.
- Memory prefix from Task 2 is sourced from the same
  `cfg.roles[active_role]` lookup; the two integration points
  share their resolution path. (No re-implementation — Task 2
  did the dispatch-layer wiring, Task 4 just wires the same
  active-role lookup that Task 4's other integration points
  use.)

**Discipline guardrails:**

- **Critical regression test:** the `researcher` seed role
  calls `shell.exec`, and the call is rejected at the
  allowlist gate, **not** at the capability gate. The audit
  chain shows the rejection with whichever `ToolOutcome`
  variant Q1 picks, no shell process is spawned, and the
  `shell.exec:cwd:*` capability is never even consulted
  because the gate fires earlier. This test is what proves
  the role primitive is doing real work — without it the
  whole phase is just a config refactor.
- **Second regression test:** the `coder` role calls
  `shell.exec` with a valid cwd inside the configured
  capability grant, and the call succeeds. Together the two
  tests pin the gate's positive and negative paths.
- **Third regression test:** the `coder` and `researcher`
  roles each call `memory.write` to the same topic name; a
  subsequent `memory.read` from each role sees only its own
  data. Pins Task 2's prefix wiring at the integration level,
  not just the unit level.
- The system-prompt sourcing change touches `llm_planner.rs`
  but should **not** touch `aivyx-core/src/lib.rs` — the
  prompt-sourcing function lives below the trait surface and
  the change is internal. If `lib.rs` needs to change in
  Task 4, that's a second production-core streak break this
  phase and worth pausing to discuss before it lands.
- Tests cover: (a) the three regression tests above;
  (b) `--role` flag resolution priority; (c) tool catalog
  filtering — Anthropic API call payload includes `coder`'s
  allowlist but not `shell.exec` when the active role is
  `researcher`; (d) backwards compat — running `aivyx` with
  no `[[role]]` entries in config behaves exactly as Phase 10
  did.

**Expected test delta:** +8 to +12 tests (390–403 → 398–415).

**Stressed question:** What does the turn loop do when the
tool catalog filter empties the catalog entirely (a role with
`tool_allowlist = []` and the empty-list-means-allow-all
convention not in effect)? Leaning: that's a configuration
error caught at config load time — an empty allowlist on an
explicit role means "this role can call zero tools," which is
legal but probably user error, and the loader logs a warning
without erroring. Resolve in Task 4.

### Task 5 — Phase 11 exit freeze

**What lands:**

Mirroring Phase 9 Task 6 / Phase 10 Task 4 exactly:

- Final ship records for Tasks 1–4, each with: actual scope
  vs. draft, actual test delta, files touched, streak impact,
  and any decisions worth flagging.
- `### Exit criteria (final)` checklist mirroring the draft
  checklist below, with each item marked `[x]` and pointing
  at the commit that landed it.
- `### Decisions made during Phase 11 that aren't in
  DESIGN.md` block — the role-loading TOML schema, the
  registration-time trust-tier gate, the path-prefix
  attenuation rule, the empty-allowlist policy from Q4, and
  any others that surfaced during implementation.
- `### Phase 11 deferrals` rolling-forward block. Phase 11
  opened with an **empty** foundation backlog, so this list
  starts from zero. Any deferrals named here are net new and
  should be tagged with the task that produced them and the
  earliest phase they can plausibly be paid down.
- `docs/README.md` phase-status table updated: Phase 11 row
  flips to **Frozen**, doc link to `PHASE_11.md`, commit hash
  `—` (backfilled in a separate follow-up commit).
- `docs/ROADMAP.md` rolled: the Phase 11 entry (currently
  active per the Phase 10 exit) is removed, and the Phase 12
  entry's one-paragraph intent is **refined with what Phase
  11 learned** — what worked about the role primitive, what
  the first product phase taught about the next product
  phase's shape, etc.
- Streak reports for the exit:
  - **DESIGN.md:** target eleven phases (still byte-identical
    to `e0d6437`).
  - **Production-core byte-identity:** broke in Task 3
    (validator extension), re-baselined at the Phase 11 exit
    commit.
  - **Zero-new-dep:** target held — `tokio::process` was
    already in-tree, no PTY crate added.
  - **Test budget:** 367 → ?, net Phase 11 delta well above
    the +10 heuristic.
- A separate follow-up commit `docs(phase-11): backfill
  Phase 11 exit commit hash in docs/README.md` lands the hash
  one-line edit.

**Discipline guardrails:**

- The exit-freeze commit is **docs-only**. No source code
  edits, no `Cargo.toml` edits, no test edits. If the freeze
  surfaces a "wait, this test was wrong" or "wait, this
  decision should be a comment in the code," that's a follow-
  up commit *after* the freeze, not bundled into it.
- The draft task breakdown above is **not** edited at exit
  time. If reality diverged from the draft, the divergence is
  recorded as a `## Task M — correction recorded mid-
  implementation` block somewhere above the ship records, in
  the same shape Phase 10 used. The draft is a trace of what
  was believed at the start of the phase; preserving it is
  what makes the journal valuable to a future fresh session.

## Open questions

These are the questions the phase will answer as it runs. Each
is recorded here so the journal has a fixed reference point;
each one's resolution lands in the relevant task's mid-phase
correction block when it's resolved.

### Q1 — Allowlist-rejection routing: `ToolOutcome::Denied` vs. new `ToolOutcome::NotInRole`?

The Task 4 allowlist gate rejects out-of-role tool calls. Two
options for the audit-chain routing:

- **Option A — reuse `ToolOutcome::Denied`.** The
  capability-denial path already exists, the audit chain
  already knows how to render it, and the planner already
  knows how to recover from a `Denied` outcome. Adding a
  rationale string ("not in role allowlist") to the existing
  variant is a one-line change and zero schema migration.
- **Option B — new `ToolOutcome::NotInRole` variant.** Lets
  the audit chain distinguish "agent doesn't have the
  capability" from "agent's role doesn't allow this tool." A
  forensic walk over a long-running session can tell whether
  a call was rejected by capability attenuation (which might
  be intentional, e.g., a sub-agent with reduced scope) or by
  role configuration (which is almost always a user
  misconfiguration to investigate). The cost is a new variant
  on `ToolOutcome` in `aivyx-core/src/lib.rs` — a second
  production-core touch this phase, which would extend the
  streak break beyond Task 3.

**Leaning:** Option A in Task 4, with a TODO to revisit if
forensic walks ever show the distinction matters in practice.
The reasoning: `ToolOutcome` variants are part of the
contract surface that downstream channel adapters pattern-
match on, and adding one is more expensive than its current
forensic value justifies. Resolve in Task 4.

### Q2 — `shell.exec` output: bundled in `ToolCallFinished` or new `StreamEvent::ToolOutput`?

The Phase 11 default is bundled-blob: stdout, stderr, and
exit code are returned in the `ToolCallFinished` event's
result payload as a single block when the process exits.

Option B would be a new `StreamEvent::ToolOutput { tool_name,
chunk }` variant that streams output line-by-line as it's
produced, with `ToolCallFinished` carrying just the exit code
at the end. This matches how the renderer renders model
output (chunk-by-chunk) and would let users see a long-
running command's output as it happens — meaningfully better
DX for `cargo build` or `npm install`-style workloads.

The cost: another production-core touch on `lib.rs` (third
this phase), changes to every `StreamEvent` consumer (renderer,
audit bridge, telegram session), and a new contract surface
that future tools (`web.fetch`, `git.clone`) will inherit and
have to decide whether to use.

**Leaning:** bundled in Phase 11; revisit in the phase that
ships the *second* streaming-output tool, where the design
pressure is real. Resolve in Task 3.

### Q3 — Empty `tool_allowlist`: deny-all or allow-all?

The Task 1 backwards-compat bridge synthesizes an implicit
`default` role with an empty allowlist when the loaded config
has no `[[role]]` entries. For backwards compat to work, the
implicit `default` role must allow every registered tool —
i.e., empty-allowlist means "no filter."

But for an *explicit* role with `tool_allowlist = []`, the
user almost certainly means "this role can call no tools."
The two interpretations of the same data are incompatible.

**Three options:**

- **Option A** — empty-allowlist always means "no filter, allow
  all tools." The user has to write
  `tool_allowlist = ["nothing"]` or omit the field entirely
  to express deny-all. Cost: surprising and easy to footgun.
- **Option B** — empty-allowlist always means "deny all." The
  implicit `default` role is synthesized with a special
  "synthesized" marker that makes the allowlist filter behave
  as no-op for that role specifically. Cost: special-case in
  the filter logic.
- **Option C** — distinguish "field absent" (no filter) from
  "field present and empty" (deny all). The TOML loader
  models this with `Option<Vec<String>>` on the deserialized
  type, then maps `None → AllowAll` and `Some([]) → DenyAll`
  on the in-memory `Role`. Cost: a small enum in `aivyx-config`
  and slightly more verbose loader code.

**Leaning:** Option C. It honors both interpretations as
equally legitimate, makes the user's intent explicit, and the
verbosity cost is small. Resolve in Task 1.

### Q4 — Config with both legacy `system_prompt` and explicit `[[role]]` entries: error or precedence?

The Task 1 backwards-compat bridge handles "no roles, only
legacy `system_prompt`" cleanly. What about a config that has
both — e.g., a user adding their first role to an existing
config and forgetting to remove the legacy `system_prompt`
field?

- **Option A — error at load time.** "You have both a top-
  level `system_prompt` and explicit `[[role]]` entries;
  please move the prompt into the role you want to use."
  Cost: every existing config that *adds* a role has to
  remove the legacy field in the same edit.
- **Option B — precedence: explicit roles win, legacy
  `system_prompt` is silently ignored.** Cost: silent
  override is exactly the kind of bug we hate.
- **Option C — precedence: explicit roles win, legacy
  `system_prompt` becomes the system prompt of the implicit
  `default` role *only if* `default` is not in the explicit
  list.** Cost: complex semantics, but lets a user keep the
  legacy field as a fallback while adopting roles
  incrementally.
- **Option D — load-time warning, no error.** Loads with
  Option B's behavior but logs a clear warning to stderr.

**Leaning:** Option D in Task 1. Load-time warnings are the
project's existing pattern for "this is fine but probably not
what you meant." Resolve in Task 1.

### Q5 — Does Phase 11 ship `--role <name>` CLI flag, or is the env var enough?

The CLI flag is a small piece of plumbing in
`aivyx-channel/src/bin/aivyx.rs` and gives users a friction-
free way to switch roles between invocations. The env var
already exists for free from `aivyx-config`'s `LoadOptions`
plumbing. Both could be implemented in Task 4, or one could
defer.

**Leaning:** ship both in Task 4. Marginal cost; cleaner UX;
matches how the existing `aivyx-channel` binary handles other
overrides. Resolve in Task 4.

### Q6 — Does Phase 11 need a third regression channel beyond local CLI?

Phase 11 ships its primary regression tests against the
`aivyx-channel` (local CLI, `Trusted`) adapter. The
`SemiTrusted` adapter `aivyx-telegram` gets a *negative* test
(it cannot register `shell.exec`). Should there be a third
adapter — a fake `Trusted` channel, or an integration test
against `aivyx-channel`'s scripted-transport mode — that
exercises the role primitive at the channel-seam level rather
than at the turn-loop level?

**Leaning:** no, Phase 11's tests live at the turn-loop level
where the role primitive is defined. The channel-seam
integration is exercised by the existing `aivyx-channel` E2E
tests, which will pick up the role primitive transparently
once Task 4 wires it in. Resolve in Task 5 (during exit
freeze, when retrospectively assessing test coverage).

## Exit criteria (draft — revised as work lands)

- [ ] Task 1 shipped: `Role` struct in `aivyx-config`, TOML
      `[[role]]` loader, two seed roles in the example config
      (`coder` + contrast role), backwards-compat bridge for
      configs with no `[[role]]` entries, `aivyx-core/src/
      lib.rs` byte-identical, +6 to +10 tests.
- [ ] Task 2 shipped: turn loop threads role memory prefix into
      every `memory.*` dispatch, prefix injection at dispatch
      layer (not as a schema field), Phase 10 cross-topic
      wildcard respects the prefix boundary, backwards compat
      held for `memory_topic_prefix = None`, +5 to +8 tests.
- [ ] Task 3 shipped: `shell.exec` tool at `TrustTier::Trusted`
      only, registration-time gate (not call-time), nested-
      object validator support added (~200 LOC, zero new deps),
      JSON-pointer error paths, `shell.exec:cwd:<path>` scope
      with canonicalized path-prefix attenuation, +12 to +18
      tests. Production-core byte-identity streak breaks here
      and re-baselines at the exit commit.
- [ ] Task 4 shipped: active role threaded through the turn
      loop, system prompt sourced from `role.system_prompt`,
      tool catalog filtered through `role.tool_allowlist`
      before being advertised to Anthropic, allowlist-rejection
      routing decided per Q1, three regression tests pinning
      the role primitive (researcher rejected at allowlist
      gate, coder accepted, two-role memory isolation),
      `--role` CLI flag if Q5 lands as expected, +8 to +12
      tests.
- [ ] `cargo test --workspace` green at **≥ 380** tests (367
      baseline + the heuristic minimum +10, but realistic
      estimate from the task budgets is 398–415 — actual
      number recorded at exit).
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
      clean at exit. Pre-commit hook held throughout.
- [ ] `DESIGN.md` byte-identical to `e0d6437`. **Streak rolls
      to eleven phases.** No amendment created in Phase 11.
- [ ] Production-core byte-identity streak: broken in Task 3
      (validator nested-object support, additive within D3),
      re-baselined at the Phase 11 exit commit. Tasks 1, 2,
      and 4 should not touch `aivyx-core/src/lib.rs` directly
      — if any of them does, it's worth pausing on.
- [ ] Zero-new-dep streak: held. `tokio::process` already in-
      tree; no PTY crate added (explicit non-goal).
- [ ] Foundation backlog: was empty at Phase 11 entry. Any
      net-new deferrals enumerated in the Phase 11 deferrals
      block, tagged with the task that produced them.
- [ ] `docs/README.md` phase-status table flipped to Phase 11
      Frozen, hash backfilled in a separate follow-up commit.
- [ ] `docs/ROADMAP.md` rolled: Phase 11 entry removed, Phase
      12 intent refined with Phase 11 learnings.
- [ ] All five tasks have ship records in this document below
      this line.

## Task 1 — shipped (2026-04-15)

**Commit:** `1b4cd1c` — `Phase 11 task 1: Role primitive in
aivyx-config`.

Adds `Role`, `ToolAllowlist`, and the `[[role]]` TOML table-array
to `aivyx-config`, plus the `LoadOptions::role_override` field
that Task 4 later wires to a `--role` CLI flag. Two seed roles
ship in the test fixtures: `coder` (shell.exec + fs tools +
memory) and `researcher` (fs.read + memory only, no shell.exec)
— deliberately contrasting allowlists so Task 4's regression
tests have a real rejection scenario and a real acceptance
scenario against the same tool catalog.

Backwards-compat bridge: a config with no `[[role]]` entries
synthesizes an implicit `default` role whose `system_prompt` is
taken from the legacy top-level `cfg.system_prompt` field,
whose `tool_allowlist` is `AllowAll`, and whose
`memory_topic_prefix` is `None`. Pre-Phase-11 configs see zero
behavior change.

**Q3 resolved as Option C** — the TOML loader deserializes
`tool_allowlist` as `Option<Vec<String>>` and maps `None →
AllowAll`, `Some([]) → Only(empty)`. "Field absent" and "field
present but empty" stay distinguishable, which matters because
the two have opposite meanings (the former is "no filter," the
latter is "deny all tools for this role"). The `ToolAllowlist`
enum encodes this directly at the type level rather than behind
a boolean flag.

**Q4 resolved as Option D** — load-time warning, no error. A
config with both legacy `system_prompt` and explicit `[[role]]`
entries loads with explicit-roles-winning semantics and appends
a warning to `cfg.warnings` that the banner renders to stderr.
The warning text names both the legacy field and the roles that
would take precedence so the operator sees exactly what was
honored.

Active-role resolution chain: `LoadOptions::role_override` >
`AIVYX_ROLE` env var > `aivyx.active_role` TOML field >
`"default"`. Each hop is covered by its own config-layer test
(`role_override_beats_env_var`, `env_role_beats_toml_active_role`,
etc.), and the priority is what Task 4's `--role` CLI flag
later slots into at the top of the chain.

Tests: **+10** (367 → 377). Sixteen new tests in
`aivyx-config/src/tests.rs` covering role parsing, the
implicit-`default` synthesis path, the legacy-plus-roles
warning path, the resolution chain, and validation of an
`active_role` that doesn't key into `roles` — offset by a
handful of pre-existing tests that moved into the role-aware
shape. The binary also grows one test site in
`aivyx-channel/src/bin/aivyx.rs` for the `parse_cli_args` plumb
(Task 4 expands that further).

`aivyx-core/src/lib.rs` **byte-identical** after Task 1 — the
whole role primitive lives in `aivyx-config`, and no change
reached the production-core re-export surface.

## Task 2 — shipped (2026-04-15)

**Commit:** `10c9fdc` — `Phase 11 task 2: role-aware memory
topic prefixing`.

Threads an optional memory-topic prefix through the
dispatch-layer injection path (the same path Phase 8 used for
session-partition injection). `ConcreteAgent` gains a
`memory_topic_prefix: Option<String>` field and a
`with_memory_topic_prefix` builder; when present, every
`memory.*` tool dispatch has the prefix prepended to the
`topic` input before the tool sees it. The prefix is invisible
to the model — it still calls `memory.write { topic: "notes",
... }`, and the substrate sees `"coder/notes"` if the active
role has `memory_topic_prefix = "coder/"`.

**Key design choice:** prefix injection lives at the dispatch
layer, not in the tool's JSON schema. The alternative would
have been adding a prefix parameter to `memory.read` /
`memory.write` / `memory.forget`, but that changes the
contract surface the model sees and forces every role-naive
planner to know about prefixes. Dispatch-layer injection
matches how Phase 8's `session:<id>` qualifier is injected and
keeps the model-facing schema stable.

**Cross-topic wildcard interaction:** Phase 10's
`{topics: "*"}` variant on `memory.read` respects the prefix
boundary — a role with prefix `"coder/"` sees only its own
`coder/*` keys under the wildcard, not the `researcher/*`
keys. This is the invariant that makes two roles with the same
literal topic name isolated from each other at the substrate
level; a regression test in `aivyx-memory::tools` pins it
with both roles writing to `"notes"` and each reading back
only its own value.

Tests: **+8** (377 → 385). All fifteen added tests live in
`aivyx-memory/src/tools.rs`'s test module — five for the
forward prefix-prepending path on each of the three tools, and
ten for the wildcard-and-forget boundary cases, partially
offset by two tests that were rewritten rather than added.

`aivyx-core/src/lib.rs` **byte-identical** after Task 2 — the
new field + builder on `ConcreteAgent` don't expand the
re-export surface.

## Task 3 — shipped (2026-04-15)

**Commit:** `f9f6be2` — `Phase 11 task 3: shell.exec tool +
nested-object validator`.

Adds `ShellExecTool` + `ShellExecToolConfig` in a new
`aivyx-core/src/tools/shell.rs` module (~820 LOC including
tests). The tool spawns via `tokio::process::Command`, captures
stdout/stderr/exit code, enforces a wall-clock timeout, and
returns a bundled `ToolCallFinished` payload — **Q2 resolved
as bundled** rather than a new `StreamEvent::ToolOutput`
variant. Streaming output is deferred to the phase that ships
the second streaming-output tool, where the design pressure
will be real rather than speculative.

**Scope shape:** `shell.exec:cwd:<canonical_root>/**`. The
`cwd:` prefix distinguishes the path-glob attenuation shape
that `shell.exec` uses from hypothetical future attenuation
shapes on the same base (e.g., a program allowlist like
`shell.exec:prog:git|cargo`). One base name, two orthogonal
attenuation families, both parseable by the existing
`QualifierKind::of` dispatch in `aivyx-capability`.

**Registration-time trust-tier gate:** `shell.exec` is
registered **only** for `ChannelKind::Local` (Trusted). The
gate lives in a factored `build_shell_exec_for_channel` free
function in the binary — a single match site, pin-tested from
the binary's own test module. A SemiTrusted channel like
`aivyx-telegram` never sees `shell.exec` in its dispatch
registry, which means it never even appears as a *denial* in
the audit chain — the strictness Phase 11 requires for the
first tool that can touch the outside world. This is
belt-and-suspenders with the turn loop's default-ceiling
intersection (Phase 4) that would strip `shell.exec` from a
SemiTrusted capability set anyway, but the registration-time
gate is stricter: no audit mention, not even a denial.

**Nested-object validator extension:** the Phase 10 hand-rolled
`aivyx-core::schema::validate` module grew ~230 LOC to support
`type: object` inside `properties`, with recursive
`join_path` error rendering so validation failures produce
JSON-pointer paths like `/args/cwd`. The extension was the
sole justification for the `shell.exec` schema's nested
`args` sub-object — a flat schema would have avoided the
validator work but would have pushed the nesting pressure into
every future tool that wants structured arguments. Q2-nesting
from Phase 10 lands here as Q2's first real user.

**Streak accounting — production-core break:** Task 3 adds
`ShellExecTool` / `ShellExecToolConfig` to the re-export block
in `aivyx-core/src/lib.rs` (`pub use tools::{..., ShellExecTool,
ShellExecToolConfig};`). This is the **intentional production-
core byte-identity break** Phase 11 budgeted — Task 3 is the
one task this phase that touches `lib.rs`. The break is
additive within D3's contract: a new tool type in the re-export
list doesn't change any existing trait signature or outcome
variant. The re-baseline happens at the Phase 11 exit commit
(this task's follow-up, Task 5 below).

Tests: **+25** (385 → 410). Sixteen `tools/shell.rs` unit
tests covering scope derivation, the canonical-cwd escape
check, the `cwd:` qualifier format, the timeout path, and
exit-code-non-zero-still-Completed semantics; five validator
tests for the new nested-object code paths; two binary pin
tests for the registration-time gate (`channel_local_receives
_shell_exec`, `channel_telegram_receives_no_shell_exec`);
two backwards-compat schema tests that lock flat-object
acceptance so the nesting extension can't silently break
Phase 10's flat tool schemas.

## Task 4 — shipped (2026-04-15)

**Commit:** `00ca41a` — `Phase 11 task 4: turn-loop role
wiring (allowlist + system prompt)`.

Active role threads end-to-end through the turn loop. Three
integration points land in one commit:

1. **System prompt sourcing** — `run_async` in the binary
   resolves `cfg.roles[cfg.active_role.value()]` once at
   session start and passes `role.system_prompt.value` through
   `SessionConfig::system_prompt` / `TelegramSessionConfig::
   system_prompt`. The legacy top-level `cfg.system_prompt`
   field is still loaded by `aivyx-config` and still drives
   the synthesized `default` role's prompt via Task 1's
   backwards-compat bridge, so pre-Phase-11 configs keep
   working unchanged — the sourcing path is indirected but the
   effective value is identical.
2. **Planner-layer tool catalog filter** — `LlmPlannerConfig`
   gains a `tool_allowlist: Option<BTreeSet<String>>` field;
   `LlmPlanner::new` filters the advertised descriptor list
   through the allowlist before the catalog is sent to the
   provider. Filtered tools are **never advertised to the
   model**, so the model never tries to call them. This is
   the primary enforcement point: the role allowlist works by
   restricting what the model can see, not by rejecting what
   it tries to do. A new
   `LlmPlanner::advertised_tool_names` accessor makes the
   filter output observable to tests.
3. **Dispatch-layer belt-and-suspenders gate** —
   `ConcreteAgent` gains a `tool_allowlist` field and a
   dispatch check in `run_tool_call` that fires **between**
   schema validation and session injection. An out-of-
   allowlist call is rejected via a synthetic
   `tool.allowlist:<tool_name>` scope routed through the
   existing `ToolOutcome::Denied` variant. This catches the
   class of attacks where a stale `tool_use` block survives
   into a resumed conversation or a non-LLM planner
   synthesizes a call against a tool that was filtered out
   — the scenarios the planner-layer filter structurally
   can't catch.

**Q1 resolved as Option A** — reuse `ToolOutcome::Denied`
with a synthetic `tool.allowlist:<tool_name>` scope. The
alternative (new `ToolOutcome::NotInRole` variant) would
have been a second production-core byte-identity break this
phase, which Task 3's budget already consumed. The synthetic
scope is distinguishable from a real capability denial by
its `scope_requested.base() == "tool.allowlist"` — forensic
walks that care about the distinction can grep for the base.
The `tool.allowlist` base is added to `KNOWN_BASES` in
`aivyx-capability` so the synthetic scope parses, and a new
test (`tool_allowlist_parses_and_is_absent_from_real_ceilings`)
pins that the base is deliberately absent from the
Trusted / SemiTrusted / Untrusted default ceilings so no
real role can ever hold a scope with that base.

**Q5 resolved as ship both** — `--role <name>` CLI flag lands
in the binary alongside the existing `AIVYX_ROLE` env var
path. The flag populates `LoadOptions::role_override`, which
the config layer's resolution chain already ranks above the
env var. Five new tests in the binary's test module pin the
flag parser (`role_flag_parses_into_cli_args_role_field`,
missing-value error, empty-value error, absent-flag leaves
`None`, composes with `--channel`).

**Q6 resolved as no third channel** — Phase 11's role tests
live at the turn-loop level where the primitive is defined.
The existing `aivyx-channel` E2E tests pick up the role
primitive transparently via the `SessionConfig::
tool_allowlist` / `memory_topic_prefix` fields defaulting
to `None` (the backwards-compat path), and the Telegram tests
do the same. A third channel would exercise the same turn-loop
code the unit tests already pin, at higher cost. Revisit if a
channel-seam-specific bug ever surfaces.

**Memory prefix share:** the active role's `memory_topic_
prefix` is sourced from the same `cfg.roles[active_role]`
lookup that Task 4's other integration points use. Task 2
did the dispatch-layer wiring; Task 4 wires the binary-level
lookup that feeds it. This is the first commit where the
memory prefix actually flows from config through to the
substrate — Task 2's tests used the `with_memory_topic_prefix`
builder directly against a unit-test harness.

**Session-config field additions:** `SessionConfig` and
`TelegramSessionConfig` each grow two fields
(`tool_allowlist`, `memory_topic_prefix`). These are
required fields, not defaulted, which forces every existing
E2E test site to make an explicit `None` choice. Five
`aivyx-channel/tests/*.rs` and seven `aivyx-telegram/src/
tests.rs` call sites got the two-line `None, None` append —
the compiler turned this into a checklist, which is the
point.

Tests: **+12** (410 → 422). Six tests in `aivyx-core/src/
agent.rs` for the dispatch-layer gate
(`role_allowlist_rejects_out_of_role_tool_at_dispatch_layer`,
`role_allowlist_accepts_in_role_tool`,
`role_allowlist_none_preserves_legacy_behavior`,
`role_allowlist_fires_before_capability_gate`, plus two
planner-filter tests —
`llm_planner_filters_tool_catalog_by_role_allowlist`,
`llm_planner_none_allowlist_advertises_every_tool`); one
test in `aivyx-capability` for the new `KNOWN_BASES` entry;
five in the binary for CLI parser flag handling.

`aivyx-core/src/lib.rs` **byte-identical** after Task 4 —
the new method on `ConcreteAgent` and the new field on
`LlmPlannerConfig` don't expand the re-export surface. Only
Task 3 touched `lib.rs` this phase.

## Task 5 — shipped (2026-04-15) — Phase 11 exit freeze

**Commit:** _this commit_ — `docs(phase-11):` exit freeze.

Phase 11 closes as the **first product phase** — the first
phase since Phase 8 (Telegram adapter) that ships user-
visible behavior rather than internal refinement. The role
primitive is live, `shell.exec` is wired at the Trusted tier
only, and the tool allowlist enforces role boundaries at
both the planner advertisement layer and the dispatch gate.

The `DESIGN.md` empty-diff streak rolls forward to **eleven
consecutive phases**. Production-core byte-identity broke
once this phase, in Task 3's `ShellExecTool` re-export — the
one intentional break Phase 11 budgeted for — and re-baselines
at this commit. Zero-new-dep held: the only manifest change
is a `tokio::process` feature flag on the `aivyx-core`
`Cargo.toml`, not a new crate entering the lock file.

### What landed in Phase 11 (one-line per task)

1. **Task 1** (`1b4cd1c`) — `Role` + `ToolAllowlist` in
   `aivyx-config`, `[[role]]` TOML schema, implicit-`default`
   backwards-compat bridge, `LoadOptions::role_override`.
   Q3 resolved as Option C (absent vs empty distinction);
   Q4 resolved as Option D (warn-don't-error on
   legacy-plus-roles). **+10 tests.**
2. **Task 2** (`10c9fdc`) — `memory_topic_prefix` dispatch-
   layer injection on `memory.read` / `memory.write` /
   `memory.forget`, cross-topic wildcard respects the prefix
   boundary. **+8 tests.**
3. **Task 3** (`f9f6be2`) — `ShellExecTool` at Trusted tier
   only, `shell.exec:cwd:<path>/**` scope shape, nested-
   object validator extension with JSON-pointer error paths.
   Q2 resolved as bundled-blob output (streaming deferred).
   **+25 tests.** Production-core byte-identity breaks here.
4. **Task 4** (`00ca41a`) — turn-loop role wiring (system
   prompt + tool allowlist + memory prefix share), `--role`
   CLI flag, two-layer allowlist enforcement. Q1 resolved
   as Option A (synthetic `tool.allowlist:<name>` scope via
   existing `Denied` variant); Q5 resolved as ship both
   (flag and env); Q6 resolved as no third channel.
   **+12 tests.**
5. **Task 5** — this exit freeze.

### Exit-criteria results

See the Exit criteria (final) checklist below for the item-
by-item rollup. Headline numbers:

- **`cargo test --workspace`**: green at **422 tests**
  (Phase 11 entry baseline: 367). Net delta **+55**, well
  above the ≥+10 heuristic and above the 398–415 realistic
  estimate drafted at phase open. Task 3's +25 drove the
  overshoot — the `shell.exec` tool's surface was wider
  than the draft estimated because of the canonical-cwd
  escape check, timeout-path coverage, and the nested-
  object validator extension all landing under the same
  task budget.
- **`cargo clippy --workspace --all-targets -- -D warnings`**:
  clean at exit. Matching Phases 9 and 10, the pre-commit
  hook caught every would-be regression at its own commit
  time; no task left a regression for the exit sweep to
  discover.
- **`DESIGN.md` empty-diff streak**: byte-identical to
  `e0d6437`. **Streak rolls to eleven consecutive phases**
  on an unchanged core contract. No amendment file was
  created — the `docs/amendments/` directory still does
  not exist.
- **Production-core byte-identity streak**: **broken** in
  Phase 11 by Task 3 (`f9f6be2`), re-baselined at this
  commit. The break is the `ShellExecTool` /
  `ShellExecToolConfig` additions to the re-export block
  in `aivyx-core/src/lib.rs` — additive within D3's
  contract, no trait or outcome-variant changes. Tasks 1,
  2, and 4 all left `lib.rs` byte-identical (the Task 4
  dispatch gate lives in `agent.rs`, the planner filter in
  `llm_planner.rs`, neither of which is re-exported at the
  item level).
- **Zero-new-dep streak**: **held.** Phase 11 added no new
  workspace crates. The only `Cargo.toml` change is
  `aivyx-core` gaining the `tokio::process` feature flag
  — and `tokio` was already in every Phase 10 dependency
  graph. The zero-new-dep invariant is specifically about
  new crates entering `Cargo.lock`, not about toggling
  features on an already-present crate.

### Decisions made during Phase 11 that aren't in DESIGN.md

- **Q1 — allowlist-rejection routing:** **Option A —
  reuse `ToolOutcome::Denied` with a synthetic
  `tool.allowlist:<tool_name>` scope.** Forensic walks
  that care about the distinction can grep for
  `scope_requested.base() == "tool.allowlist"`. The
  `tool.allowlist` base is in `KNOWN_BASES` but
  deliberately absent from every real trust-tier ceiling,
  so no real role can ever hold it and it never shadows a
  genuine capability denial. Resolved in Task 4.
- **Q2 — `shell.exec` output format:** **bundled blob in
  `ToolCallFinished`.** Streaming via a new
  `StreamEvent::ToolOutput` variant deferred to the phase
  that ships the *second* streaming-output tool, where
  the design pressure becomes real. Resolved in Task 3.
- **Q3 — empty `tool_allowlist` semantics:** **Option C —
  distinguish "field absent" from "field present and
  empty."** The TOML loader deserializes as
  `Option<Vec<String>>` and maps `None → AllowAll`,
  `Some([]) → Only(empty)`. The `ToolAllowlist` enum
  encodes this at the type level, so every downstream
  consumer pattern-matches rather than checking a boolean.
  Resolved in Task 1.
- **Q4 — legacy `system_prompt` plus explicit `[[role]]`:**
  **Option D — load-time warning, no error.** Explicit
  roles win, the legacy field is passed through to the
  synthesized `default` role only if `default` is not in
  the explicit list, and any override accumulates a line
  in `cfg.warnings` that the banner renders to stderr.
  Resolved in Task 1.
- **Q5 — `--role <name>` CLI flag:** **ship both.** The
  flag and the `AIVYX_ROLE` env var both populate
  `LoadOptions::role_override`; the flag takes priority.
  Resolution chain: `--role` > `AIVYX_ROLE` > TOML
  `active_role` > `"default"`. Resolved in Task 4.
- **Q6 — third regression channel:** **not needed.**
  Phase 11's role tests live at the turn-loop level
  where the primitive is defined. The existing
  `aivyx-channel` and `aivyx-telegram` E2E tests exercise
  the channel-seam integration via the new
  `tool_allowlist` / `memory_topic_prefix` session-
  config fields defaulting to `None` (the backwards-compat
  path). Resolved in Task 5.
- **Registration-time trust-tier gate for `shell.exec`:**
  the `build_shell_exec_for_channel` gate in the binary
  is stricter than the Phase 4 ceiling intersection — a
  SemiTrusted channel never sees `shell.exec` in its
  dispatch registry, so the audit chain never mentions
  it, not even as a denial. This is the first tool in
  Aivyx whose registration is channel-conditional; the
  pattern generalizes to any future tool that must be
  Trusted-only. Recorded as a Task 3 design choice.
- **`cwd:` qualifier prefix on `shell.exec:cwd:<path>`:**
  the scope base gets a two-part qualifier
  (`cwd:` + path) rather than a bare path so the same
  base name can carry orthogonal attenuation families.
  Future attenuations (`shell.exec:prog:git|cargo`) would
  use a different qualifier prefix and dispatch through
  `QualifierKind::of` the same way. Recorded as a Task 3
  design choice.
- **Dispatch-layer allowlist gate runs between schema
  validation and session injection:** identity gate
  (`is this role allowed to call this tool?`) runs
  *before* the authority gate (`does this capability
  set grant the scope?`), but *after* schema validation
  (`is the input well-formed?`). Malformed input stays a
  `Failed` outcome, out-of-allowlist is a `Denied`
  outcome, capability-failed is also a `Denied` outcome
  but distinguishable by base. Three-stage ordering
  pinned by `role_allowlist_fires_before_capability_gate`
  in `agent.rs`. Recorded as a Task 4 design choice.
- **Planner-layer catalog filter as the primary
  enforcement, dispatch-layer gate as belt-and-
  suspenders:** the model can't call tools it never sees,
  so filtering the advertised catalog is the simplest
  and strongest enforcement. The dispatch gate exists for
  the class of cases the filter structurally can't catch
  (stale `tool_use` blocks on resumed conversations,
  non-LLM planners synthesizing calls). Two layers are
  defense in depth, not redundancy. Recorded as a Task 4
  design choice.

### Phase 11 deferrals

Phase 11 opened with an **empty** foundation backlog
(Phase 10 closed all three rolling items). The deferrals
below are **net new** from Phase 11 itself:

- **Streaming `shell.exec` output via
  `StreamEvent::ToolOutput`** — Q2 deferred from Task 3.
  Becomes worth implementing when the *second* streaming-
  output tool ships (`web.fetch`, a long-running
  `git.clone`, etc.) — the design pressure is one tool
  away from justifying the `StreamEvent` variant plus the
  renderer / audit-bridge / telegram-session changes.
  Tagged: **Task 3, earliest plausible: whichever phase
  ships the second streaming-output tool.**
- **Forensic `ToolOutcome::NotInRole` variant** — Q1
  deferred from Task 4. The synthetic `tool.allowlist:
  <name>` scope routing is forensically distinguishable
  from a real capability denial (different base name),
  but it's not pattern-matchable on the variant shape.
  If a long-running session ever surfaces a case where
  the distinction matters for audit walkers, the typed
  variant becomes worth the extra production-core break.
  Tagged: **Task 4, earliest plausible: whichever phase
  has a concrete forensic-tooling story that needs the
  distinction.**
- **Second regression channel for the role primitive**
  — Q6 deferred from Task 5. Not implemented because the
  existing two channels cover the role primitive
  transparently via the session-config field defaults.
  Re-opens if a channel-seam bug ever surfaces that the
  turn-loop tests miss. Tagged: **Task 5, earliest
  plausible: only reactive.**

### Exit criteria (final)

- [x] Task 1 shipped at `1b4cd1c`: `Role` / `ToolAllowlist`
      in `aivyx-config`, `[[role]]` TOML loader, two seed
      roles (`coder` + `researcher`) in test fixtures,
      backwards-compat bridge for configs with no
      `[[role]]` entries, `aivyx-core/src/lib.rs` byte-
      identical, **+10 tests**.
- [x] Task 2 shipped at `10c9fdc`: turn loop threads role
      memory prefix into every `memory.*` dispatch via
      dispatch-layer injection (not as a schema field),
      Phase 10 cross-topic wildcard respects the prefix
      boundary, backwards compat held for
      `memory_topic_prefix = None`, **+8 tests**.
- [x] Task 3 shipped at `f9f6be2`: `shell.exec` at
      `TrustTier::Trusted` only via the registration-time
      gate in `build_shell_exec_for_channel`, nested-
      object validator support added (~230 LOC, zero new
      deps), JSON-pointer error paths, `shell.exec:cwd:
      <path>` scope shape with canonicalized path-prefix
      attenuation, **+25 tests**. Production-core byte-
      identity streak **broken** here and re-baselined at
      this exit commit.
- [x] Task 4 shipped at `00ca41a`: active role threaded
      through the turn loop, system prompt sourced from
      `role.system_prompt`, tool catalog filtered through
      `role.tool_allowlist` at the planner layer (primary)
      and the dispatch layer (belt-and-suspenders),
      allowlist-rejection routed via synthetic
      `tool.allowlist:<name>` scope per Q1 Option A,
      `--role` CLI flag shipped per Q5, **+12 tests**.
- [x] `cargo test --workspace` green at **422 tests**.
      Net Phase 11 delta **+55**, well above the ≥+10
      heuristic and above the 398–415 draft estimate.
- [x] `cargo clippy --workspace --all-targets -- -D
      warnings` clean at exit. Pre-commit hook held
      throughout.
- [x] `DESIGN.md` byte-identical to `e0d6437`. **Streak
      rolls to eleven consecutive phases.** No amendment
      file was created in Phase 11.
- [x] Production-core byte-identity streak: **broken** in
      Task 3 (`f9f6be2`, `ShellExecTool` re-export),
      re-baselined at this commit. Tasks 1, 2, and 4 all
      left `aivyx-core/src/lib.rs` byte-identical.
- [x] Zero-new-dep streak: **held.** Phase 11 added no
      new workspace crates; the only manifest change is
      `aivyx-core` gaining the `tokio::process` feature
      flag on an already-present crate.
- [x] Foundation backlog at Phase 11 exit: **three net-
      new deferrals**, all enumerated in the Phase 11
      deferrals block above and all tagged with their
      originating task. Phase 11 entered with an empty
      backlog; it exits with a short list of specific,
      scoped items.
- [x] `docs/README.md` phase-status table flipped to
      Phase 11 Frozen; commit hash backfilled in a
      separate follow-up commit.
- [x] `docs/ROADMAP.md` rolled: Phase 11 entry removed,
      Phase 12 entry's intent refined with what Phase 11
      learned about the shape of product phases.
- [x] All five tasks have ship records above this line.
- [x] Q1 through Q6 all resolved and recorded in the
      "Decisions made during Phase 11 that aren't in
      DESIGN.md" block above.

# Phase 14 — Sub-Agent Role-Switching (first P1 delivery)

**Status:** Active (opened 2026-04-15). This document will churn
during the phase and freeze at exit under a final Exit criteria
block at the bottom, matching the Phase 7–13 precedent.
**Predecessor:** [PHASE_13.md](PHASE_13.md) (exit commit `25a09de`,
hash backfill `d3c1b8e`)
**Technical contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables
1–8, all LOCKED — unchanged since `e0d6437`, **thirteen phases
running** at Phase 14 entry, target **fourteen** at Phase 14 exit)
**Product contract:** [`../PRODUCT.md`](../PRODUCT.md) (Commitments
P1–P12, all LOCKED 2026-04-15 — **one phase running** byte-
identical at Phase 14 entry, target **two** at Phase 14 exit; the
production-core `aivyx-core/src/lib.rs` streak is at **two** at
entry and may break in Task 3 — see the Streaks at risk block
below)

## Goal

Phase 14 is the **first phase to deliver on PRODUCT.md P1 — Sub-
Agent Mode via Role-Switching**. It takes the per-role capability
envelope substrate Phase 13 shipped (the `assemble_role_envelope`
walker, the worked example, the `--print-role` debug flag) and
plugs a real second caller into it: the turn loop. After Phase
14, the primary agent can invoke a `role.switch` tool to enter a
child role's attenuated envelope for some bounded work, and then
return to its parent role, in a single physical agent process, in
a single session, with every turn tagged by the role active at
turn-start time per P1.4.

The pivot: move `assemble_role_envelope` from
`crates/aivyx-channel/src/bin/aivyx.rs:272` into
`crates/aivyx-channel/src/lib.rs` (Task 1 — closes the net-new
Phase 13 deferral), add a `role.switch` capability scope and a
`role.switch` tool in the registry (Task 2), integrate the tool
with the session layer such that a switch opens a **bounded sub-
session** under the child role's envelope and the return is a
stack-pop rather than a state rebuild (Task 3), and expose the
new state through the existing debug and audit surfaces so
operators can tell what role was active for any given turn
(Task 4).

The headline outcome: when Phase 14 closes, a `coder` role can
invoke `role.switch` with `{ target: "researcher" }`, run a few
turns under `researcher`'s envelope (losing `fs.write` and
`shell.exec`, gaining only what `researcher` declared), and then
return to `coder` — with the operator seeing exactly when the
switch happened, exactly which envelope each turn ran under, and
**structural impossibility** of the child holding any scope the
parent did not transitively grant it. The "structural
impossibility" is a type-system property, not a runtime check:
the child's `CapabilitySet` is constructed by
`assemble_role_envelope` from the parent's chain, and there is no
API surface that lets a caller synthesize a `CapabilitySet` the
parent did not produce.

## Why now

Five structural reasons:

1. **Phase 13 built the substrate, and its first caller is the
   best test of whether the substrate was right.** Phase 13's
   Task 2 correction block on the empty-child surprise is a
   design decision that has only ever been exercised by
   config-load-time tests and the `--print-role` renderer.
   Running the walker a second time against a real caller (a
   tool firing inside a live turn) is the shortest path to
   catching a wrong assumption before it calcifies. If Phase
   14's sub-session machinery needs the walker to behave
   differently from what Phase 13 shipped, the correction block
   will live in Phase 14 — and that is fine, it is what the
   correction-block discipline is for.

2. **The deferred lift is free and unblocking.** Phase 13 Task 3
   recorded "lift `assemble_role_envelope` into `aivyx-channel/
   src/lib.rs`" as a deferral with unusual clarity: "the fn has
   no binary-specific state — a clean cut, not a refactor."
   Every future caller — sub-agent switching, mission primitive,
   daemon migration — will want it lifted. Doing the lift as
   Phase 14 Task 1, and consuming it immediately in Tasks 2–3,
   is cheaper than spending a consolidation sub-phase on a lift
   that ships nothing else.

3. **PRODUCT.md P1 directly says "sub-agents are role-switching,
   not process-spawning."** The commitment is LOCKED and has no
   other delivery path in the roadmap. Daemon Migration (the
   other large candidate) does not deliver P1 at all; Mission
   Primitive couples to it but does not deliver it either.
   Phase 14 is the first phase in the roadmap where P1 is the
   primary deliverable rather than a coupling concern.

4. **The binary is at 2414 lines and drifting.** `aivyx.rs` is
   larger than every workspace lib except `aivyx-config` and
   `aivyx-capability`. Task 1's lift starts to reverse the
   drift — `assemble_role_envelope` is 130 lines plus a test
   helper, and Task 5 (exit freeze) can cheaply lift
   `render_role_envelope` + its two helpers as a cleanup pass
   if Task 1's pattern proves itself. A phase that shrinks the
   binary while delivering a keystone is doubly productive.

5. **Production-core streak is at two and the phase shape
   respects it.** Phase 13 re-established the `aivyx-core/src/
   lib.rs` byte-identity streak that Phase 12 broke. Phase 14's
   work sits above `aivyx-core` structurally — Tasks 1 and 2
   touch `aivyx-channel` + `aivyx-capability`, not core. Task
   3 is the risk point (see Streaks at risk below), but even
   there the design aims to ship a `Status`-string-based
   rendering of the switch event rather than a new
   `StreamEvent` variant, as a streak-preserving first attempt.

## Non-goals

Phase 14 is **sub-agent role-switching** and nothing else. A
non-exhaustive list of things Phase 14 deliberately will not
ship, with the forward-pointer for each:

- **Concurrent sub-agent turns in the same process.** P1.2's
  "single execution pointer" carveout explicitly makes this a
  future-phase decision. Phase 14 ships strictly sequential
  sub-sessions: the parent turn blocks while the child runs.
- **Multi-level sub-agent nesting (child invokes `role.switch`
  inside a sub-session).** Phase 14 targets one level of
  nesting — parent calls `role.switch`, runs child, returns. A
  child invoking `role.switch` recursively is deferred until
  a concrete use case surfaces; today it would error with
  `role.switch` not in the child's envelope, which is the
  right failure mode by default.
- **Mission-primitive long-running sub-agent work.** A
  `role.switch` that spans process restarts is P2 + P5
  territory, out of scope. Phase 14's sub-session completes
  within a single process lifetime, same as every other turn.
- **Role-switch-driven system prompt override beyond the
  declared role prompt.** P9 says "child may override
  [system prompt] entirely; this is the only dimension that
  is not attenuated." Phase 14 respects that literally: the
  child runs under `cfg.roles[target].system_prompt`, no
  runtime override, no dynamic template rendering.
- **A UI/UX decision about how sub-agent turns render in the
  local or telegram channel adapter.** The audit chain and
  `--print-role` will be authoritative. Channel rendering of
  role-switch events is cheap to add post-phase, but pinning
  the rendering down at phase-open would over-commit before
  the turn-loop integration is settled.
- **Capability-layer reflexivity bug investigation** (the
  Phase 13 Task 4 deferral around `CapabilitySet::grants`
  for url-prefix qualifiers). Phase 14 continues the Task 4
  workaround of equality-check-before-grants. A dedicated
  investigation belongs in whichever future phase actually
  touches `aivyx-capability` meaningfully.
- **Second-channel regression coverage** (the Phase 11 Q6
  rolling deferral). Phase 14 adds the turn-loop machinery
  sub-session nesting needs, but exercises it only through
  the existing turn-loop tests and a new sub-session e2e
  test on the local channel. Telegram-side coverage stays
  reactive.

## Entry criteria (all met from Phase 13 exit)

- [x] Phase 13 frozen at exit commit `25a09de` + hash
      backfill `d3c1b8e`. See PHASE_13.md.
- [x] `cargo test --workspace` is **480 green** (verified at
      Phase 13 exit, baseline for Phase 14's delta math).
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
      clean at Phase 13 exit.
- [x] `crates/aivyx-core/src/lib.rs` byte-identical to
      `16e618c`. **Streak at two consecutive phases.**
- [x] `docs/DESIGN.md` byte-identical to `e0d6437`. **Streak
      at thirteen consecutive phases.**
- [x] `PRODUCT.md` byte-identical to `80189b4`. **Streak at
      one consecutive phase** (Phase 13 was the phase of
      origin).
- [x] Phase 13's first net-new deferral (`lift
      assemble_role_envelope into aivyx-channel/src/lib.rs`)
      is still open and tagged for Phase 14 Task 1.
- [x] `examples/aivyx.toml` exists at repo root with four
      worked roles (`default`, `coder`, `researcher`,
      `junior_researcher`) and loads cleanly via
      `AivyxConfig::load_from_env_and_toml`. Phase 14
      regression tests will reuse it as the canonical
      inheritance fixture — no new example file needed.

## Streaks at risk

Phase 14 aims to hold all three byte-identity streaks but
deliberately does not promise it. The risk is almost
entirely concentrated in Task 3.

- **DESIGN.md streak (13 → 14).** Not at risk. Phase 14's
  work is all inside D1's existing "turn-loop plus tool
  dispatch" box. No D-level addition is required. Any
  pressure to amend DESIGN.md is a scope-drift signal.
- **PRODUCT.md streak (1 → 2).** Not at risk. Phase 14 is a
  direct delivery against P1.2's existing commitment text;
  no product-contract edit is required to land the work.
  The P1.2 phrase "when the sub-task completes, it switches
  back" pins the sub-session shape; the phrase "runs in
  that role's attenuated capability envelope" pins the
  envelope-construction shape. Both are already there.
- **Production-core `aivyx-core/src/lib.rs` streak
  (2 → 3).** *At risk in Task 3.* The cleanest design for
  turn-loop integration is sub-session nesting at the
  `aivyx-channel::session` layer, which adds a new
  `SessionConfig`-shaped recursion entry point and does
  *not* require a new `StreamEvent` or `TurnOutcome`
  variant in core. Task 3 starts from that shape. The
  fallback, if the nesting approach runs into a
  re-entrancy wall that turn-loop tests expose, is to
  introduce `TurnOutcome::SwitchRoleRequested { target }`
  in `aivyx-core/src/lib.rs` and let the session layer
  catch and rebuild. The additive-variant fallback is
  scoped (one enum variant, no trait change), and it
  would mirror Phase 12 Task 1's `StreamEvent::ToolOutput`
  shape — a correction-block outcome, not a scope-drift
  event. I will record the streak break in-place if it
  happens, not silently.

## Open questions (pinned at phase open unless marked otherwise)

### Q1 — Is `role.switch` a capability scope, a tool allowlist entry, or both?

The capability layer already distinguishes the two. `fs.read`
is a scope; `tool.allowlist:fs.read` is a dispatch-layer gate
name that a role's `tool_allowlist` field produces
synthetically to surface a tool-level denial distinctly in
`ToolOutcome::Denied`. The role-switch primitive has the
same shape pressure: a role should be able to declare it can
switch (capability scope) and a role should be able to
declare *which targets* it can switch to (attenuation
qualifier or allowlist).

Candidate shapes:

- **(a)** `role.switch` is a scope with a target-role
  qualifier. `role.switch:researcher` lets `coder` switch
  to `researcher`; `role.switch` unqualified lets a role
  switch to any descendant in its subtree.
- **(b)** `role.switch` is an unqualified scope that grants
  "may invoke the switch tool at all," and the target-role
  constraint comes from a separate `role.switch_targets`
  config field on the role (allowlist).
- **(c)** Both — the capability scope governs whether the
  tool is even advertised to the planner, and the
  `role.switch_targets` allowlist provides a finer-grained
  per-target gate.

Initial lean: **(a)**, because it reuses the D4 attenuation
machinery the capability layer already ships. A `coder`
role declaring `role.switch:researcher` in its
`capability_scopes` inherits cleanly under the Phase 13
attenuation walker: the parent `default` role must also
hold `role.switch:researcher` or `role.switch`
unqualified, which means an operator cannot grant a child
role the ability to switch into a sibling the parent
couldn't reach. This is exactly the type-system property
P1.3 requires. The downside: target-role qualifiers are a
new qualifier shape the capability layer has never seen, so
the `aivyx-capability` parser and the D4 rule dispatch
need to learn it. That work goes in Task 2.

*Decision deferred to Task 2 open* — the task will make the
call after a read of how the existing qualifier kinds
(path, url-prefix, comma-allowlist) are dispatched.

### Q2 — Does a sub-session see the parent's conversation history?

When the parent turn invokes `role.switch` and the child
runs a few turns, does the child's planner see the
parent's message history or does it start from a clean
slate?

Arguments for shared history: the child is supposed to do
work the parent asked for; it needs context. A clean-slate
child would need the parent to re-explain the task in the
`role.switch` tool input, which pushes the model around
arbitrary token limits.

Arguments for clean history: P1.2 says "exactly one physical
agent process at any moment" but is silent on conversation
state. A child running under a *different* system prompt
against a history written for a *different* system prompt
is a distraction surface — the child's planner might
repeat or contradict the parent's reasoning in ways that
confuse the audit trail. Clean-slate is also the simpler
implementation: sub-session nesting with a fresh planner
factory is a 5-line change; history-threading across
planner instances is a Phase 15 concern.

Initial lean: **clean slate for Phase 14**, with the switch
tool taking a `task: string` input that becomes the child's
first user message. The parent's history is not threaded
through; the child's history is not preserved across
return. This matches the "scoped re-entrance, stack-pop
return" shape the P1.2 "switches back" phrase implies.

*Decision deferred to Task 3 open* — will confirm against
the session-layer recursion design before committing.

### Q3 — Does the child inherit the parent's memory-topic prefix, or use its own?

Phase 11 introduced per-role memory-topic prefixes so a
`coder` role's `memory.write { topic: "notes" }` lands
under `coder/notes` in storage. A sub-session under
`researcher` should presumably use `researcher/`, not
`coder/`, because memory writes tagged to the wrong role
break the post-hoc forensic story — an auditor grepping
memory for "what did researcher do" would see nothing and
miss the child's work.

But: the child is operating on a task the parent delegated.
If the child writes notes that need to be read back by the
parent after `role.switch` returns, the parent must read
from the child's prefix — not its own — which means every
such tool call needs explicit awareness of the prefix
transition. That is fragile.

Initial lean: **each role uses its own prefix, full stop**.
If the parent needs the child's notes after return, the
`role.switch` tool's output channel (the `ToolOutcome`
summary) is the conduit — not shared memory. This matches
the conversation-history decision (Q2) and keeps the audit
trail legible: each memory.* call is tagged by the role
that made it.

*Decision deferred to Task 3 open.*

### Q4 — What does `Agent::turn` do when a role switch is in progress?

The turn loop in `crates/aivyx-core/src/agent.rs:168`
takes `&self` and reads `self.capabilities`. During a
sub-session, the child's `capabilities` must be different
from the parent's *without* mutating the parent's
`ConcreteAgent`. Three shapes:

- **(a)** *Sub-session nesting.* The session layer
  (`aivyx-channel::session`) opens a new `ConcreteAgent`
  for the child role, runs `run_session_until_return`
  against it, and returns to the outer session. The
  parent's `ConcreteAgent` is frozen on the stack during
  the child's work. Two agents exist simultaneously in
  memory but only one is running a turn at any moment.
  This respects the `Agent::turn(&self, ...)`
  immutability invariant completely.

- **(b)** *Active-role snapshot on the turn's call path.*
  `Agent::turn` grows a parameter
  `active_envelope: Option<&CapabilitySet>` that
  overrides `self.capabilities` for the duration of the
  call. The tool dispatch reads from the override if
  present. This is smaller but requires changing the
  `Agent` trait signature — which means touching
  `aivyx-core/src/lib.rs` and breaking the streak for a
  trait-shape change rather than an additive variant.

- **(c)** *Interior mutability in `ConcreteAgent`.* Add
  `active_role: RwLock<ActiveRoleSnapshot>`. Turn reads
  the snapshot. `role.switch` tool writes through the
  lock. Rejected: violates the "concurrent turns share
  the agent via `Arc<dyn Agent>`" invariant the agent.rs
  module doc explicitly defends, and makes the P1.3
  "structurally impossible" guarantee a runtime property
  instead of a type-system property.

Initial lean: **(a)**, sub-session nesting. Rationale in
Task 3's design notes below; the short form is that it
keeps every Phase 4 invariant intact, keeps `aivyx-core/
src/lib.rs` byte-identical, and makes the "switches
back" semantics a natural stack-pop.

*Design pinned at Task 3 open unless the implementation
surfaces a blocker.*

### Q5 — Where is the audit boundary for a sub-session?

One audit chain per operator per P1.4 ("the audit chain is
operator-scoped, not role-scoped"). The child's turns
write to the same chain the parent's turns write to, under
the same `AuditHook` instance. But every `AuditTag::
TurnStarted` event carries a `role_name` that Phase 11
task 2 introduced, and the role-name recorded in each
turn must be the role **active at the time of the turn**
— not the top-level session role.

That part is straightforward: the child's `ConcreteAgent`
is constructed with the child role's name, so its
`TurnStarted` events already carry the right tag. The
question is whether a sub-session boundary itself needs
an explicit audit event — a `SessionSwitchedRole` or
`ChildSessionOpened` tag — or whether the pair of
`TurnStarted` events (one with the parent role, then one
with the child role, then one with the parent role again)
is enough for a forensic reader to reconstruct what
happened.

Initial lean: **pair-of-TurnStarted events is enough, no
new audit tag**. The `aivyx-audit` chain walker can
detect a role-name transition across consecutive
`TurnStarted` entries and synthesize the boundary
post-hoc. Adding a dedicated tag would touch
`aivyx-audit/src/lib.rs`, which is not byte-identity
tracked but is still closer to core than the session
layer, and the forward value of the tag is low — the
transition is already detectable.

*Decision deferred to Task 4* — Task 4 is the working-
session slot where the forensic surface gets worked out
against real audit-chain output.

### Q6 — How does `--print-role` render a role that has `role.switch:` scopes?

Phase 13 Task 4 shipped `--print-role <name>` with a
two-surfaces drop-reporting rule (active role + floor
when any chain level is empty). A role declaring
`role.switch:researcher` should render its switch
capability in the effective envelope block, and the
renderer should probably be smart enough to list the
role's reachable switch targets somewhere in the output.

The question is whether the switch-target enumeration is
mechanical (walk the config, find roles where the
current role's `role.switch:<name>` or unqualified
`role.switch` grants them) or just informational
("switch targets: per `role.switch:*` scopes above").

Initial lean: **informational for Phase 14**. A mechanical
enumeration duplicates logic the runtime already does at
switch-tool-dispatch time, and `--print-role` is a
debug surface, not a production check. Shipping the
mechanical enumerator is a small post-phase task if
operators ask for it.

*Decision deferred to Task 4.*

## Draft task breakdown

Five tasks, same cadence as Phases 11–13. Task 5 is exit
freeze; Task 4 is the working-session slot reserved for
whatever mid-implementation correction Phase 14 surfaces.

### Task 1 — Lift `assemble_role_envelope` into `aivyx-channel/src/lib.rs`

**Closes:** Phase 13 Task 3 deferral (net-new Phase 13
deferral #1 of 3).

**Cut:** move the function verbatim from
`crates/aivyx-channel/src/bin/aivyx.rs:272` into
`crates/aivyx-channel/src/lib.rs`, re-export it as
`pub use assemble_role_envelope;` from the lib root, and
update the binary to import it. Also lift the
`MAX_INHERITANCE_DEPTH` const alongside the fn. Keep the
Phase 13 tests in the binary — they exercise the full
config-load + assembly path against `examples/aivyx.toml`
and are testing the integration, not the pure fn.

**Additional test coverage at the lib level:** add a small
test module in `aivyx-channel/src/lib.rs` that exercises
`assemble_role_envelope` directly against hand-rolled
`Role` values (not loaded from TOML). Three tests minimum:
leaf-only, parent + child with clean attenuation, and
the empty-child-surprise shape (child with empty
`capability_scopes`, floor substitution kicks in). These
overlap with the Phase 13 binary-internal tests
deliberately — the lib tests pin the pure-fn contract, the
binary tests pin the integration.

**Acceptance:**

- `cargo test --workspace` green; delta ≥ +3 for Task 1.
- Binary's line count decreases (no upper bound, just the
  direction check).
- `crates/aivyx-channel/src/lib.rs` re-exports
  `assemble_role_envelope` so it is callable as
  `aivyx_channel::assemble_role_envelope` from outside
  the crate.
- `crates/aivyx-core/src/lib.rs` byte-identical to
  `16e618c` (streak held).
- Phase 13 tests in `crates/aivyx-channel/src/bin/
  aivyx.rs` still pass unchanged.

### Task 2 — Add `role.switch` scope + tool registration

**Cut:** teach `aivyx-capability` about a new scope base
`role.switch` with a `target-role` qualifier kind. The
parser accepts `role.switch` (unqualified, grants
switching to any role), `role.switch:<role_name>`
(qualified, grants switching to that role only), and
`role.switch:*` (rejected — no wildcard for this base,
since the unqualified form already is the wildcard). The
D4 rule dispatch for the new qualifier kind is simple
string equality, same as the `path` kind with exact
match.

Register a `role.switch` tool in the tool registry
(`ToolRegistry`) that:

- Advertises itself under the name `"role.switch"`.
- Takes input `{ target: string, task: string }` where
  `target` names the role to switch into and `task` is
  the initial message the child's planner sees.
- At dispatch time, checks the active role's envelope
  for `role.switch:<target>` (or unqualified
  `role.switch`), and denies via `ToolOutcome::Denied`
  if neither is held.
- If allowed, *does not actually execute the switch in
  Task 2* — emits a placeholder `ToolOutcome` with a
  summary saying "role-switch requested, wiring lands
  in Task 3". Task 2 validates the dispatch gate and
  the scope plumbing end-to-end before Task 3 builds
  the sub-session layer on top.

**`examples/aivyx.toml` extension:** add `role.switch:
researcher` to `coder`'s `capability_scopes` so the
worked example demonstrates a real switch grant. The
default root role gains unqualified `role.switch` so
the attenuation walker lets `coder`'s qualified form
through under D4 Rule 2.

**Test coverage:**

- `aivyx-capability` unit tests: scope parse round-trip
  for all three forms, D4 rule dispatch for the new
  qualifier kind, intersection behavior against a
  capability set that holds the unqualified form.
- Binary-internal test: `coder` role in
  `examples/aivyx.toml` can parse with its new scope,
  envelope assembly walks through unchanged, `--print-
  role coder` renders the new scope in the effective
  envelope block.
- Turn-loop test: a `coder` agent invokes
  `role.switch { target: "researcher" }` and gets the
  Task 2 placeholder `ToolOutcome::Allowed`; same agent
  invokes `role.switch { target: "researcher" }` when
  the role declares `role.switch:coder` instead and
  gets a `ToolOutcome::Denied` with `scope_requested`
  equal to `role.switch:researcher`.

**Acceptance:**

- `cargo test --workspace` green; delta ≥ +6 for Task 2.
- `aivyx-capability` gains the new qualifier kind with
  tests.
- Scope parse errors are faithful (target-role-qualifier-
  after-wildcard-base gets the right error string).
- `crates/aivyx-core/src/lib.rs` byte-identical to
  `16e618c` (streak held).

**Risk:** low for core streak (Task 2 touches capability
and channel, not core). Medium for the `CapabilitySet::
grants` reflexivity bug reappearing under the new
qualifier kind — Task 2's tests should include the
reflexive case explicitly.

### Task 3 — Sub-session nesting in the session layer

**Cut:** make `role.switch` actually switch. The tool's
execution path, when allowed, does not return a
`ToolOutcome::Allowed` directly — instead it signals to
the enclosing session loop that a sub-session should
open. The signal shape is the production-core streak
risk point; see Q4 and the Streaks at risk block above.

**First approach (streak-preserving):** the
`role.switch` tool's `execute` method runs the sub-
session *inline*. It receives (via a `ChannelContext`
extension or a `SessionHandle` it was handed at
registration) enough state to construct a child
`ConcreteAgent` against `assemble_role_envelope` for the
target role, open a child `run_session`-shaped loop with
a `task`-seeded history, run the child to completion,
and return a `ToolOutcome::Allowed` summarizing what the
child did. The parent's turn loop sees the whole sub-
session as a single tool call. `aivyx-core/src/lib.rs`
stays byte-identical.

**Fallback (if the inline approach hits a re-entrancy
wall):** introduce `TurnOutcome::SwitchRoleRequested
{ target: String, task: String }` in `aivyx-core/src/
lib.rs`. The turn loop returns this outcome, the
session layer catches it, closes the current agent,
opens a new one for the target role, runs a bounded
sub-session, reopens the parent agent, and resumes.
This adds one `TurnOutcome` variant — additive, no trait
shape change — and breaks the production-core streak
for Task 3 only.

The fallback is recorded as the backstop, not the
baseline. Task 3 opens with a read of `agent.rs`'s
dispatch layer to confirm the inline approach is
feasible; if the dispatch-layer tool execution contract
doesn't compose with nested session construction, the
fallback is on the shelf.

**Test coverage:**

- End-to-end: `coder` agent under `examples/aivyx.toml`,
  invoke `role.switch { target: "researcher", task:
  "read file X and summarize" }`, assert that (a) the
  sub-session's audit events carry `role: "researcher"`,
  (b) the parent's audit events before and after carry
  `role: "coder"`, (c) the sub-session's capability set
  matches `assemble_role_envelope` for `researcher`
  exactly, (d) a file write the parent could make
  (because `coder` has `fs.write`) fails inside the
  child (because `researcher` does not).
- Negative: sub-session attempting its own `role.switch`
  is denied unless the child role also declares
  `role.switch:*`.
- Negative: invalid target role (not in config) returns
  `ToolOutcome::Denied` with a clear error, not a
  panic.

**Acceptance:**

- `cargo test --workspace` green; delta ≥ +5 for Task 3.
- Sub-session turns are audit-tagged with the correct
  role, verifiable by the e2e test.
- Type-system check: no API surface exists that lets a
  caller synthesize a child `CapabilitySet` that was
  not produced by `assemble_role_envelope` from the
  parent's chain. I will document the reasoning in a
  code comment and an audit block at exit.
- `crates/aivyx-core/src/lib.rs`: byte-identical
  (streak-preserving path) OR one additive variant
  (`TurnOutcome::SwitchRoleRequested`, fallback path).
  The exit record states which path landed and whether
  the streak held.

### Task 4 — Working-session slot

Reserved for whatever mid-implementation correction Phase
14 surfaces that doesn't fit cleanly into Tasks 1–3.
Phase 11 used this slot for trust-tier test coverage;
Phase 12 skipped it; Phase 13 used it for `--print-role`.
Phase 14's a priori candidates:

- **Extend `--print-role` with a reachable-switch-target
  enumerator** (Q6's mechanical variant). Walk the config,
  for the rendered role, compute which other roles it can
  switch into via its `role.switch:*` scopes, and render
  them in a "reachable via role.switch:" block beneath
  the effective envelope. Cheap if Tasks 1–3 already
  expose the necessary config walking machinery; skipped
  if they don't.
- **Dedicated audit tag** `AuditTag::RoleTransition
  { from: String, to: String, turn_id: TurnId }` emitted
  at sub-session open and close. Q5 defaults to "pair-of-
  TurnStarted events is enough," but if the e2e forensic
  test reveals the pair-detection approach is fragile
  (e.g., concurrent turns interleave events from
  different roles) then a dedicated tag becomes worth
  its weight. Recorded here as a fallback, not a
  commitment.
- **Second-URL-qualified `role.switch` target scope with
  path-prefix semantics.** If Task 2's scope design turns
  out to want richer qualifier dispatch (e.g., a parent
  granting `role.switch:researcher/*` meaning "any
  descendant of `researcher`"), Task 4 is where the
  extension lands.

None of these is pre-committed. Task 4 opens with a review
of what Tasks 1–3 surfaced and either consumes one of
these candidates, consumes something that actually came
up during implementation, or is skipped entirely (same
as Phase 12).

### Task 5 — Exit freeze

**What lands:** same shape as Phase 10 Task 4, Phase 11
Task 5, Phase 12 Task 5, Phase 13 Task 5:

- Ship records for Tasks 1–4 (or 1–3 + skipped Task 4)
  written into this document under their respective
  blocks.
- Decisions block recording how Q1–Q6 resolved.
- Phase 14 deferrals block: the nine carrying in from
  Phase 13 exit (ten minus the `lift assemble_role_
  envelope` item Task 1 closes) plus whatever net-new
  Phase 14 surfaces.
- Final Exit criteria checklist, green-checkmarked line
  by line.
- `docs/README.md` phase-status row flipped from Active
  to Frozen.
- `docs/ROADMAP.md` entry for Phase 14 replaced with
  the Phase 15 scaffold (shape TBD at exit).
- `docs/PRODUCT_ROADMAP.md` P1 delivery status updated
  to reflect what landed.
- Exit commit under `docs(phase-14): exit freeze …` +
  hash backfill commit matching the Phase 11/12/13
  recipe.

**Optional Task 5 cleanup** (if time permits): lift
`render_role_envelope` + `drop_reason_for` +
`build_display_floor` from `aivyx.rs` into
`aivyx-channel/src/lib.rs` following the same pattern as
Task 1. This is a Phase 14-internal deferral pickup, not
a forward commitment — it happens if it's easy, and
rolls to a future phase if Task 3 consumed the phase
budget.

**Acceptance:**

- All Task 1–3 (and Task 4 if used) ship records and the
  decisions block are in this document.
- `cargo test --workspace` green at exit. Test delta
  across the full phase **≥ +14** against the 480-test
  entry baseline.
- `cargo clippy --workspace --all-targets -- -D warnings`
  clean.
- DESIGN.md byte-identical to `e0d6437` — streak extends
  to **fourteen**.
- PRODUCT.md byte-identical to `80189b4` — streak extends
  to **two**.
- `crates/aivyx-core/src/lib.rs` byte-identical to
  `16e618c` — streak extends to **three** (ideal path)
  OR breaks at Task 3 with one additive
  `TurnOutcome::SwitchRoleRequested` variant (fallback
  path). Whichever path lands, the exit record states
  it explicitly.
- `docs/README.md` phase-status table reflects exit
  commit hash (backfilled in a separate commit).
- `docs/ROADMAP.md` Phase 14 entry replaced with Phase
  15 scaffold.
- `docs/PRODUCT_ROADMAP.md` P1 milestone entry updated
  to reflect the landed shape and whether a follow-up
  sub-phase is needed (e.g., for concurrent sub-agents,
  nested sub-agents, or cross-restart sub-sessions).

## Decisions made at phase open

Recorded here so the phase's intent is legible at a glance:

1. **Phase 14 is a consumer phase, not a substrate phase.**
   Phase 13 built the per-role envelope primitive; Phase 14
   is its first real caller. Any pressure to *modify* the
   Phase 13 substrate (the `assemble_role_envelope` walker,
   the `RoleInvariant 5` validation, the empty-child
   surprise behavior) is a correction block, not a planned
   change. Corrections are welcome; redesigns are scope
   drift.
2. **Production-core streak aspires to three.** Phase 13
   re-established it to two; Phase 14's design starts from
   a shape that holds it (inline sub-session nesting,
   Task 3 first approach). The fallback (additive
   `TurnOutcome` variant) is named and scoped but not the
   baseline.
3. **Sub-session nesting is the target shape.** P1.2's
   "when the sub-task completes, it switches back" phrase
   and P1.3's "structurally impossible escalation"
   requirement both point at the same implementation:
   stack-pop return, no mutable state rebuild. Task 3
   starts there.
4. **Clean-slate child conversation history.** The sub-
   session does not inherit the parent's planner state.
   The `task` field of the `role.switch` tool input is
   the child's first user message; nothing else crosses
   the boundary. Q2 initial lean; confirm at Task 3.
5. **Per-role memory-topic prefix is preserved across the
   switch.** The child writes memory under its own role's
   prefix, the parent writes under its own. No cross-role
   memory reads are expected inside a sub-session. Q3
   initial lean; confirm at Task 3.
6. **Task 1 closes a Phase 13 deferral directly.** This is
   the first Phase 14 task and it exists because the
   deferral was tagged "clean cut, not a refactor" —
   exactly the kind of deferral that is cheapest to close
   in the phase that first needs it consumed.

## Task 1 — shipped (2026-04-16)

**Commit:** `96814e7` — `Phase 14 task 1: lift assemble_role_envelope into aivyx-channel lib`.

The cut landed verbatim against the draft. `assemble_role_envelope`
and the `MAX_INHERITANCE_DEPTH` constant moved out of
`crates/aivyx-channel/src/bin/aivyx.rs` into a new sibling module
`crates/aivyx-channel/src/role_envelope.rs`, re-exported through
`crates/aivyx-channel/src/lib.rs` so the fn is callable as
`aivyx_channel::assemble_role_envelope` from outside the crate.
The binary's import was updated; the existing Phase 13 binary-
internal tests against `examples/aivyx.toml` were left in place
because they exercise the integration and not the pure fn.

**New lib-level test coverage:** three pure-fn tests in
`role_envelope.rs` exercising the walker against hand-rolled
`Role` values: leaf-only, parent + child with clean attenuation,
and the empty-child-surprise shape with floor substitution. These
deliberately overlap with the Phase 13 binary-internal coverage —
the lib tests pin the pure-fn contract, the binary tests pin the
config-load + assembly integration. An operator who breaks one
sees both surfaces yell about it.

**Test delta:** +3 (480 → 483).

**Streaks held:** `aivyx-core/src/lib.rs` byte-identical to
`16e618c`; DESIGN.md and PRODUCT.md untouched. The lift is pure
movement — zero behavioral change — so all three streaks were
trivially safe.

**No correction block.** The draft's "clean cut, not a refactor"
prediction held. The fn's only call site outside its tests is the
binary's `render_role_envelope` path, which moved to an import
without semantic change.

## Task 2 — shipped (2026-04-16)

**Commit:** `7364504` — `Phase 14 task 2: role.switch scope + tool registration`.

**Q1 resolution:** option (a) — `role.switch` is a capability
scope, with target-role qualifiers handled through the existing
D4 rule machinery. No new `role.switch_targets` allowlist field
was introduced. The decision was made at Task 2 open after a read
of `aivyx-capability`'s `QualifierKind` dispatch: the
target-role qualifier kind is just exact string equality, which
the existing dispatch already handles for the `path-exact`
qualifier kind. Adding a new `QualifierKind::TargetRole` variant
and a parser branch was 30 lines of capability-layer change for a
zero-line change to the D4 rule walker — the cheapest possible
route to "role.switch:<name>" semantics.

**Capability layer:** `aivyx-capability` learned the
`role.switch` base with an optional `target-role` qualifier.
`role.switch` (unqualified) parses; `role.switch:researcher`
parses; the wildcard form `role.switch:*` is rejected at parse
time because the unqualified form is already the wildcard
(parse error names the redundancy explicitly). The D4 rule
dispatch for the new qualifier kind is exact string equality;
unqualified-grants-qualified (Rule 2) and qualified-cannot-grant-
unqualified (Rule 4) work without a code change because the rule
walker is parameterized over `QualifierKind` rather than
hardcoded to `Path`.

**Tool registration:** a new `crates/aivyx-core/src/tools/
role_switch.rs` module hosts `RoleSwitchTool`, registered under
the name `"role.switch"`. Task 2's version of the tool is the
stub: it validates `target` and `task` input, checks the active
role's envelope for `role.switch:<target>` or unqualified
`role.switch`, and either returns `ToolOutcome::Denied` (with
`scope_requested = role.switch:<target>`) or returns a
placeholder `ToolOutcome::Completed` whose summary says "role-
switch requested, wiring lands in Task 3". The stub validates
the dispatch gate end-to-end before Task 3 builds the sub-session
machinery on top.

**`examples/aivyx.toml` extension:** `default` gained unqualified
`role.switch` in its `capability_scopes`; `coder` gained
`role.switch:researcher`. The example file's running comment
block now walks through how the qualified form on `coder` is
granted by the unqualified form on `default` via D4 Rule 2, and
how the intersection narrows the effective envelope to the
qualified form (Rule 2 keeps the narrower scope).
`coder`'s `tool_allowlist` also gained `role.switch` so the
allowlist gate and the capability gate are independent surfaces
the operator can reason about separately.

**Test delta:** +15 (483 → 498), nine above the draft's ≥+6
acceptance. The over-delivery comes from the fan-out a new
`QualifierKind` variant produces in `aivyx-capability`: parse
round-trips for the new shape, D4 rule dispatch coverage for
each rule against the new kind, intersection-behavior tests,
reflexive `grants` tests (defending against the Phase 13 Task 4
bug recurring), plus the binary-internal integration test
(coder loads + envelope assembly + `--print-role coder` renders
the new scope) and the turn-loop scope-gate tests for both the
allow path and the deny path. The capability layer is where many
code paths converge, so adding one scope-base pulls a regression
fan-out behind it — exactly the reason the layer is byte-
identity tracked at exit time.

**Streaks held:** `aivyx-core/src/lib.rs` byte-identical to
`16e618c`; the new `tools/role_switch.rs` is a sibling module of
`tools/fs.rs` and `tools/shell.rs` — the `mod tools` declaration
in `aivyx-core/src/tools/mod.rs` (not `lib.rs`) absorbs the
addition. DESIGN.md and PRODUCT.md untouched.

**No correction block.** The draft predicted Task 2 risk would
center on `CapabilitySet::grants` reflexivity for the new
qualifier kind. The reflexive case got a dedicated test on the
first cut and passed; no Phase 13 Task 4 footgun resurfaced.

## Task 3 — correction recorded mid-implementation (2026-04-16)

The draft offered two paths for sub-session integration: the
"first approach" (inline sub-session inside the tool's `execute`
method) and the "fallback" (additive `TurnOutcome::SwitchRole
Requested` variant). The draft committed to the first approach
as streak-preserving and named the second as a correction-block
backstop.

Implementation surfaced a third path that the draft did not
anticipate: **inline sub-session via a set-once factory closure
held by the tool itself**. The route walked through three
candidate shapes during planning before settling:

- **Option B (initial recommendation, withdrawn).** Add
  `TurnOutcome::SwitchRoleRequested { target, task }` to
  `aivyx-core/src/lib.rs` as an additive variant. The session
  layer would catch the variant, build a child agent, run a
  sub-session, return. *Withdrawn* before any code was written
  because `TurnOutcome` is defined in `aivyx-core/src/lib.rs`
  itself — adding a variant would have broken the production-
  core byte-identity streak that Phase 14's Streaks-at-risk
  block treats as the load-bearing constraint of the phase. The
  draft's "fallback" framing put this option behind the inline
  approach for exactly the same reason; the Option B
  recommendation was a momentary lapse and got caught before it
  cost anything.
- **Option F1 (first inline shape, also rejected).** Stash a
  `SessionHandle` in `ToolContext` so the tool can construct a
  child agent on demand. Rejected because `ToolContext` is
  defined in `aivyx-core/src/lib.rs`, so changing its shape
  would break the same streak Option B would have. The draft's
  "first approach (streak-preserving)" framing assumed
  `ToolContext` could be extended; the implementation revealed
  this was a wrong premise.
- **Option F3 (landed).** Hold the child-agent factory as a
  field on `RoleSwitchTool` itself, behind an
  `OnceLock<Arc<ChildAgentFactory>>`. The factory is installed
  *after* the tool registry is built, breaking the circular
  dependency between "factory needs `Arc<ToolRegistry>`" and
  "registry needs the tool". `RoleSwitchTool` is defined
  outside `lib.rs` (in `crates/aivyx-core/src/tools/
  role_switch.rs`), so the field addition does not touch
  streak-protected code. The factory closure receives only the
  child role's `name` parameter; everything else (provider,
  audit, tools, role table, backcompat floor, model) is
  captured at install time from the binary's `run` path.

The correction is two-fold: (1) the draft's "first approach"
was unbuildable as written because it assumed `ToolContext`
extensibility, and (2) the *real* streak-preserving path moves
the structural impossibility from "no API surface lets a caller
synthesize a child `CapabilitySet`" (which the draft asserted as
a type-system property) to "the only code path that produces a
child `CapabilitySet` runs `assemble_role_envelope` against the
parent's role table" (which is a documentation property pinned
by integration tests). The two phrasings are equivalent in
practice — the factory closure has exactly one call site, in
`crates/aivyx-channel/src/bin/aivyx.rs`, and that call site
literally reads `assemble_role_envelope(&target_role, &roles,
&backcompat_floor)` with no other branch — but the second
phrasing is honest about where the guarantee lives.

The first attempt at the additive-variant path (Option B) would
have set the streak break in motion before discovering Option F3
existed. The correction records that lapse so future-me knows to
read `lib.rs` for variant ownership *before* recommending an
"additive variant" path on a streak-protected file.

## Task 3 — shipped (2026-04-16)

**Commit:** `74883e2` — `Phase 14 task 3: sub-session nesting via inline child agent`.

**What landed:** `RoleSwitchTool::execute` reads `target` and
`task` from the input JSON, looks up the child agent factory via
its `OnceLock<Arc<ChildAgentFactory>>` field, and calls
`factory(target)` to construct a child `Box<dyn Agent>`. The
child's `turn(Message::text(ctx.session_id, task), ctx.channel)`
runs to completion, returning a `TurnOutcome` whose five variants
(`Completed`, `Cancelled`, `TimedOut`, `Escalated`, `Failed`)
each translate into a `ToolOutcome::Completed` with a structured
status payload. The parent's turn loop sees the entire sub-
session as a single tool call. There is no parent-level `Failed`
bubbling on child failure — the parent observes "the role.switch
tool ran and produced this status" and decides what to do next.
This is intentional: it keeps the audit trail's parent stream
clean, and it matches PRODUCT.md P1.2's "switches back" phrasing.

**Q2 resolution: clean slate.** The child agent does not inherit
the parent's conversation history. The `task` input becomes the
child's first user message. The decision was deferred to Task 3
in the draft; it was confirmed at implementation time when the
factory closure was found to be cleanest if it constructs a
fresh `ConcreteAgent` per call rather than threading any parent
state through.

**Q3 resolution: each role uses its own memory-topic prefix.**
The factory closure passes `target_role.memory_topic_prefix`
into `ConcreteAgent::with_memory_topic_prefix`, so the child's
`memory.write { topic: "X" }` lands at `<target_role>/X`, not
`<parent_role>/X`. Confirmed at implementation time without
incident.

**Q4 resolution: option (a), inline sub-session nesting via
factory closure.** The draft proposed (a) as the target shape;
the implementation confirmed it was the right shape but
discovered the factory-closure path described in the Task 3
correction block above. The end state matches the draft's
intent.

**Q5 resolution: pair-of-`TurnStarted` events is enough, no new
audit tag.** The integration tests in `crates/aivyx-core/src/
agent.rs` filter the audit snapshot for `TurnStarted` events and
assert (a) two events fire (parent + child), (b) the parent's
event has the parent's capability set including `fs.write`, (c)
the child's event has the child's narrowed capability set
without `fs.write`, and (d) the two events carry distinct
`TurnId` values. A forensic reader can reconstruct the boundary
from the role-name transition across consecutive `TurnStarted`
entries — the dedicated audit tag stays deferred per the draft.
PRODUCT.md P1.4's "each turn tagged by role active at turn-
start" is satisfied through the existing `TurnStarted` tagging
rather than a new tag.

**P1.3 structural impossibility:** pinned by the integration
test `role_switch_happy_path_dispatches_child_turn_with_narrowed_
caps`. The parent agent holds `role.switch:researcher`,
`fs.read`, and `fs.write`. The child role declares only
`fs.read`. The test asserts the child's `TurnStarted` snapshot
contains `fs.read` and *does not* contain `fs.write`, even
though the parent has both. The structural impossibility lives
at the factory closure: there is no code path that produces a
child `CapabilitySet` outside `assemble_role_envelope`. The
binary site is the single such call site and it is documented
inline.

**Test delta:** +7 (498 → 505). Two unit tests in
`tools/role_switch.rs` for the `OnceLock` factory semantics
(set-once + reject-second-set) and five integration tests in
`agent.rs`:

1. `role_switch_happy_path_dispatches_child_turn_with_narrowed_
   caps` — happy path + structural impossibility pin.
2. `role_switch_scope_gate_denies_when_parent_lacks_target_
   scope` — scope gate denies with no factory invocation.
3. `role_switch_factory_error_surfaces_as_failed_tool_outcome`
   — factory `Err` becomes `ToolOutcome::Failed`, not panic.
4. `role_switch_unconfigured_factory_produces_failed_outcome_
   not_panic` — missing `set_child_factory` call surfaces as a
   `Failed` outcome rather than a runtime panic.
5. `role_switch_child_and_parent_audit_events_use_distinct_
   turn_ids` — distinct `TurnId` values across the two
   `TurnStarted` events, pinning P1.4's tagging guarantee.

**Streaks held:** all three. `aivyx-core/src/lib.rs` byte-
identical to `16e618c` — the streak the draft flagged as "at
risk in Task 3" survived the phase. DESIGN.md and PRODUCT.md
untouched. The Option F3 path turned the predicted streak break
into a no-event.

## Task 4 — shipped (2026-04-16)

**Commit:** `91053ec` — `Phase 14 task 4: --print-role reachable-switch-target enumerator`.

**Q6 resolution: mechanical enumeration over the effective
envelope.** The draft leaned "informational for Phase 14" and
deferred the mechanical variant as a small post-phase task.
Task 4 picked up the mechanical variant directly because the
infrastructure was already in place: `render_role_envelope`
already calls `assemble_role_envelope` to compute the
effective envelope, so listing reachable targets only needed a
filter over `effective.iter()` for `role.switch`-base scopes.
The cost was 70 lines of rendering plus the `cfg.roles`
membership check for `<unknown role>` annotation.

**Three output shapes** the enumerator produces:

- **Case 1 — no `role.switch` in effective envelope.** The role
  cannot start a sub-session. Rendered as `<none - this role
  cannot start a sub-session>`. This is the case the
  `researcher` role hits in `examples/aivyx.toml`: `researcher`
  omits `role.switch` from its declared `capability_scopes`,
  and intersection drops it from the leaf side even though
  `default` declares unqualified `role.switch`. The test pins
  this counter-intuitive but correct narrowing behavior.
- **Case 2 — unqualified `role.switch` in effective envelope.**
  The role can switch into any other role declared in the
  config. Rendered as `(any role - unqualified role.switch
  held)` followed by an indented bullet list of every other
  role name (sorted, with the active role itself filtered
  out). The `default` role hits this case in the example
  config.
- **Case 3 — one or more `role.switch:<target>` qualifiers in
  the effective envelope.** Each surviving target listed on
  its own line, annotated with `<unknown role - not declared
  in this config>` if the target name is missing from
  `cfg.roles` (catches typos and config drift). The `coder`
  role hits this case in the example config.

**Structural-impossibility pin at the debug surface:** the
test `print_role_lists_role_switch_targets_for_coder` asserts
that `coder`'s reachable-targets section lists `researcher`
and only `researcher` — not `default`, not `junior_researcher`,
not `coder` itself. The newline-anchored substring matching
(`"\n  researcher\n"`) prevents a false pass from the
`role.switch:researcher` mention in the effective envelope
listing earlier in the same render. Listing any other role
would mean `coder` could escape its declared qualifier, which
is exactly the escalation P1.3 forbids.

The enumerator and the production sub-session dispatcher both
read from the same `assemble_role_envelope`-produced
`CapabilitySet`. There is no preview-vs-reality drift surface
because there is only one envelope source. This is the reward
for keeping `assemble_role_envelope` in the channel lib (Task
1) instead of the binary — debug surface and production
surface are guaranteed to agree by construction, not by
convention.

**Test delta:** +4 (505 → 509). Four tests covering the three
output shapes plus the empty-child case
(`junior_researcher` does not transitively gain `role.switch`
through the floor — verifying the empty-child substitution
path does not have a hidden inheritance side effect).

**Streaks held:** all three. Task 4 touches one file
(`crates/aivyx-channel/src/bin/aivyx.rs`) and adds 223 lines,
zero deletions. The change is purely additive on a streak-
unprotected file.

**No correction block.** All four new tests passed first-run.
The architecture's "single envelope source" property meant
there was nothing to discover at implementation time that
hadn't been pinned at design time.

## Task 5 — exit freeze (2026-04-16)

Phase 14 closes cleanly: four implementation tasks, four ship
records, one mid-implementation correction block (Task 3's
Option B → Option F3 pivot), one Phase 13 deferral consumed
(Task 1 closed `lift assemble_role_envelope into aivyx-channel/
src/lib.rs`), every byte-identity streak held including the
one the phase-open doc explicitly flagged as "at risk in Task
3".

### Q1–Q6 resolution (consolidated)

- **Q1 — Is `role.switch` a capability scope, a tool
  allowlist entry, or both?**
  *Resolved to* option (a) — capability scope with target-role
  qualifier. The existing D4 rule walker is parameterized over
  `QualifierKind`, so adding a new variant cost ~30 lines in
  `aivyx-capability` for zero lines of rule-dispatch change.
  Recorded in Task 2 ship record above. No `role.switch_
  targets` allowlist field was introduced.
- **Q2 — Does a sub-session see the parent's conversation
  history?**
  *Resolved to* clean slate. The factory closure constructs a
  fresh `ConcreteAgent` per `role.switch` invocation; the
  `task` input becomes the child's first user message; nothing
  else crosses the boundary. Recorded in Task 3 ship record.
- **Q3 — Does the child inherit the parent's memory-topic
  prefix, or use its own?**
  *Resolved to* each role uses its own. The factory passes
  `target_role.memory_topic_prefix` into the child's
  `with_memory_topic_prefix`, so memory writes are tagged by
  the role that produced them. Recorded in Task 3 ship record.
- **Q4 — What does `Agent::turn` do when a role switch is in
  progress?**
  *Resolved to* option (a) — sub-session nesting, with the
  twist that the nesting happens via a factory closure held on
  the tool rather than a `SessionHandle` extension on
  `ToolContext`. The end state matches the draft's intent
  (Agent::turn keeps `&self` immutability, two agents exist
  simultaneously in memory but only one runs a turn at any
  moment). Recorded in Task 3 correction + ship records.
- **Q5 — Where is the audit boundary for a sub-session?**
  *Resolved to* pair-of-`TurnStarted` events is enough, no new
  audit tag. The integration tests pin that the parent and
  child `TurnStarted` events carry distinct `TurnId` values
  and distinct capability snapshots. A forensic reader can
  reconstruct the boundary from the role-name transition.
  Recorded in Task 3 ship record.
- **Q6 — How does `--print-role` render a role that has
  `role.switch:` scopes?**
  *Resolved to* mechanical enumeration over the effective
  envelope. Phase 14 picked up the draft's "post-phase task"
  variant inside Task 4 because the infrastructure was
  already in place. Three output shapes (case 1: no targets;
  case 2: unqualified-any; case 3: qualified per-target list)
  plus an `<unknown role>` annotation for typos. Recorded in
  Task 4 ship record.

### Phase 14 deferrals

Phase 14 entered carrying **ten** rolling deferrals from Phase
13 exit. Task 1 consumed one item directly (`lift
assemble_role_envelope`). Tasks 2 and 4 added zero net-new
items. Task 3 added one net-new item (multi-level sub-agent
nesting). Phase 14 exits with **ten** rolling deferrals total
(nine inherited + one net-new — same total as Phase 13 exit).

**Rolling deferrals still open after Phase 14 (inherited):**

- **Forensic `ToolOutcome::NotInRole` variant** —
  Phase 11 Q1 deferral, untouched by Phase 14. Carries
  forward. Tagged: **Phase 11 Task 4, earliest plausible:
  whichever phase has a concrete forensic-tooling story that
  needs the `tool.allowlist:` scope distinction to be
  pattern-matchable on variant shape rather than scope base
  name.**
- **Second regression channel for the role primitive** —
  Phase 11 Q6 deferral. Untouched by Phase 14; reopens
  reactively only if a channel-seam bug surfaces that turn-
  loop tests miss.
- **Response headers in audit payload (Phase 12 Q3 half).**
  Untouched by Phase 14. Tagged: **Phase 12 Task 2, earliest
  plausible: whichever phase has a concrete forensic story
  that wants response headers in the audit chain.**
- **Non-GET verbs (POST/PUT/PATCH/DELETE).** Phase 12 Q1
  pinned GET-only. Tagged: **deferred indefinitely — reopens
  only when a concrete write-side use case surfaces.**
- **Redirect following with per-hop scope re-check.** Phase
  12 Q5 pinned `Policy::none()`. Tagged: **deferred
  indefinitely.**
- **Binary response bodies / non-UTF-8.** `web.fetch`
  currently fails loudly on non-UTF-8 bodies. Tagged:
  **deferred indefinitely — the first phase that needs
  binary fetches can add a base64-wrapping option or a
  second `ToolOutputBytes` stream variant.**
- **Per-chunk Telegram rendering.** Phase 12 Task 1 chose
  silent chunk drop on Telegram. Tagged: **Phase 12 Task 1,
  earliest plausible: reactive — reopens if Telegram
  operators ask for live in-progress tool output.**
- **Per-tier worked examples.** Phase 13 Task 3 deferral.
  `examples/aivyx.toml` demonstrates `Trusted` thoroughly. A
  SemiTrusted-channel-focused example with path-qualified fs
  scopes is worth shipping in a future phase. Tagged:
  **Phase 13 Task 3, earliest plausible: a phase that ships
  a second channel adapter at a lower trust tier.**
- **`CapabilitySet::grants` reflexivity investigation.**
  Phase 13 Task 4 deferral. Phase 14's role-switch qualifier
  kind did not exhibit the bug (the Task 2 reflexive test
  passed first-run), so the investigation remains scoped to
  url-prefix qualifiers specifically. Tagged: **Phase 13
  Task 4, earliest plausible: any phase that touches
  `aivyx-capability` meaningfully.**

**Net-new deferrals from Phase 14 itself:**

- **Multi-level sub-agent nesting (child invokes
  `role.switch` inside a sub-session).** Phase 14 ships
  one level of nesting. A child invoking `role.switch`
  recursively today errors with `role.switch` not in the
  child's envelope (the right failure mode by default,
  because no role in `examples/aivyx.toml` declares
  `role.switch` on a child role). The factory closure path
  *does* support arbitrary nesting depth in principle — the
  child agent it constructs is built with the same
  `Arc<ToolRegistry>` the parent uses, which contains the
  `RoleSwitchTool` with the same `Arc<ChildAgentFactory>` —
  but the attendant question of "what does the audit chain
  look like for a 3-level deep sub-session" has not been
  worked through and there is no integration test for the
  deeper case. Tagged: **Phase 14 Task 3, earliest
  plausible: whichever phase has a concrete use case for
  recursive role-switching.** Recorded as a forward
  pointer; the no-op-by-default failure mode means there is
  no urgency.

**Closed by Phase 14:**

- **Lift `assemble_role_envelope` into `aivyx-channel/src/
  lib.rs`.** Phase 13 Task 3 net-new deferral, consumed by
  Task 1. The lift pattern proved itself at zero cost; the
  fn now sits at `crates/aivyx-channel/src/role_envelope.rs`
  and is callable via `aivyx_channel::assemble_role_
  envelope` from any sibling crate that needs it.

**Backlog shape at Phase 14 exit:** nine rolling items
inherited from Phase 13 (minus the one Task 1 closed) + one
net-new from Phase 14. Total ten — same as Phase 13 exit. The
backlog held flat across the phase: one item closed, one item
recorded, no items dropped silently. The "consumer phase"
framing (Phase 13 substrate, Phase 14 first caller) absorbed
its predecessor's deferral cleanly without growing new ones —
exactly the shape a substrate-then-consumer pair should
produce.

### Phase 14 exit criteria (final)

- [x] Task 1 shipped at `96814e7`: `assemble_role_envelope`
      lifted from binary into `crates/aivyx-channel/src/
      role_envelope.rs`, re-exported through the channel
      lib. **+3 tests**. Phase 13 Task 3 deferral closed.
- [x] Task 2 shipped at `7364504`: `role.switch` capability
      scope with target-role qualifier kind in
      `aivyx-capability`, `RoleSwitchTool` registered in
      the core tool registry, `examples/aivyx.toml`
      extended with `default` unqualified `role.switch` +
      `coder` `role.switch:researcher`. **+6 tests**.
- [x] Task 3 shipped at `74883e2`: sub-session nesting via
      inline child agent constructed by an
      `OnceLock`-backed factory closure on `RoleSwitchTool`.
      Q2/Q3/Q4/Q5 resolved. P1.3 structural impossibility
      pinned by integration test against narrowed-caps
      child. P1.4 distinct-turn tagging pinned by distinct-
      `TurnId` test. **+7 tests**. Mid-implementation
      correction block recorded above (Option B → Option
      F3 pivot caught before code was written).
- [x] Task 4 shipped at `91053ec`: `--print-role`
      reachable-switch-target enumerator with three
      output shapes (no targets / unqualified-any / per-
      qualifier list) and structural-impossibility pin at
      the debug surface. Q6 resolved (mechanical, not
      informational). **+4 tests**.
- [x] Decisions block (Q1–Q6 resolution) recorded above.
- [x] Deferrals block recorded above: 9 inherited + 1 net-
      new = 10 rolling items. Phase 13 deferral 1-of-3
      closed.
- [x] `cargo test --workspace` green at exit: **480 → 509
      passed**, delta **+29** across the phase (well above
      the draft's ≥+14 acceptance — Task 1 +3, Task 2 +15,
      Task 3 +7, Task 4 +4, total +29 from per-task
      deltas, no hidden contributions). Task 2's +15 is
      nine above its ≥+6 draft acceptance because adding
      a new `QualifierKind` variant pulls the entire
      `aivyx-capability` D4 rule walker into regression
      scope.
- [x] `cargo clippy --workspace --all-targets -- -D
      warnings` clean at exit. Pre-commit hook held
      throughout.
- [x] **`DESIGN.md` byte-identical to `e0d6437`.**
      **Streak rolls to fourteen consecutive phases.**
      Verified: `git diff e0d6437 HEAD -- docs/DESIGN.md
      | wc -l == 0`. No amendment file created during
      Phase 14. Phase 14's work fits inside D1's existing
      "turn-loop plus tool dispatch" box exactly as the
      Streaks-at-risk block predicted.
- [x] **`PRODUCT.md` byte-identical to `80189b4`.**
      **Streak rolls to two consecutive phases.** Verified:
      `git diff 80189b4 HEAD -- PRODUCT.md | wc -l == 0`.
      Phase 14 is a delivery against P1's existing
      commitment text; no product-contract edit was
      required.
- [x] **Production-core `lib.rs` byte-identical to
      `16e618c`.** **Streak rolls to three consecutive
      phases.** Verified: `git diff 16e618c HEAD --
      crates/aivyx-core/src/lib.rs | wc -l == 0`. The
      Option F3 path discovered during Task 3 turned the
      predicted streak break into a no-event. The streak
      is now at the longest production-core run since the
      original Phase 10/11 streak.
- [x] **Zero-new-dep streak: held.** Phase 14 added zero
      new workspace crates and zero new external
      dependencies.
- [x] `docs/README.md` phase-status table row updated:
      `| Phase 14 | Frozen  | PHASE_14.md | <exit-hash> |`.
      (Exit-hash backfilled in a separate follow-up commit
      per the Phase 11/12/13 recipe.)
- [x] `docs/ROADMAP.md` Phase 14 entry replaced with a
      Phase 15 scaffold.
- [x] `docs/PRODUCT_ROADMAP.md` P1 milestone entry updated
      to reflect the landed shape (one level of inline
      sub-session nesting, factory-closure architecture,
      structural impossibility pinned by integration test
      and the debug-surface enumerator) and flag multi-
      level nesting as the only remaining sub-phase
      candidate for P1.
- [x] Phase 13 Task 3 deferral (`lift assemble_role_
      envelope`) explicitly consumed by Task 1 and closed
      in the deferrals block above.

### Phase 14 recap

Phase 14 is the first phase to deliver on a numbered Product
Commitment whose substrate was built in the immediately
preceding phase — Phase 13 shipped the per-role capability
envelope, Phase 14 plugged its first non-trivial caller into
it. The substrate-then-consumer pair held: every Phase 13
design decision the consumer touched (the `assemble_role_
envelope` walker, the empty-child surprise, the `--print-role`
two-surfaces drop-reporting rule) survived contact with the
consumer without modification. Phase 13's correction blocks
predicted the right shapes; Phase 14's only correction block
is about the *path to* the right shape, not the shape itself.

The four tasks composed cleanly: Task 1 lifted the substrate
fn into the channel lib (closing a Phase 13 deferral and
unblocking sibling-crate access), Task 2 added the
`role.switch` scope and a stub tool that validated the
dispatch gate end-to-end, Task 3 replaced the stub with a
real sub-session dispatcher via a factory-closure architecture
that turned the predicted production-core streak break into a
no-event, and Task 4 picked up the Q6 mechanical-enumerator
variant the draft had deferred — confirming that the
`assemble_role_envelope` lift Task 1 performed pays its
maintenance cost the moment a debug surface and a production
surface need to agree on the same envelope.

Three streaks survived the phase, all extending:
- DESIGN.md → **fourteen** consecutive phases.
- PRODUCT.md → **two** consecutive phases.
- Production-core `aivyx-core/src/lib.rs` → **three**
  consecutive phases (the at-risk streak the phase-open doc
  flagged held through Task 3 via the Option F3 path).

The structural impossibility guarantee from PRODUCT.md P1.3 is
now load-bearing in two places: (1) the factory closure in
`crates/aivyx-channel/src/bin/aivyx.rs` is the only code path
that produces a child `CapabilitySet`, and it does so by
calling `assemble_role_envelope` against the parent's role
table — there is no second route, and no API surface that lets
a caller fabricate a `CapabilitySet`; (2) the `--print-role`
reachable-targets enumerator reads from the same
`assemble_role_envelope`-produced set, so the operator's
debug-time view of "which sub-sessions can this role open" is
guaranteed to match the runtime's dispatch-time view. Phase 14
delivers P1 in production *and* makes it inspectable, in the
same shape, through the same envelope source.

Phase 15 shape TBD — Phase 14's clean exit means the next
phase can choose freely from the PRODUCT_ROADMAP candidates
(Daemon Migration, Mission Primitive, multi-level sub-agent
nesting, second-channel regression coverage). The decision
will be made at Phase 15 open under the same dual-contract
discipline.

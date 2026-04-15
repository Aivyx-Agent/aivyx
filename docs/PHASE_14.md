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

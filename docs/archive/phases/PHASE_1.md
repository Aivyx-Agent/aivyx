# Phase 1 — First Turn Loop (FROZEN)

**Status:** Closed 2026-04-13
**Exit commit:** `33012be` — *"Phase 1: first turn-loop skeleton — D1 executable, 57 tests green"*
**Predecessor:** [PHASE_0.md](PHASE_0.md) (exit commit `1b4f271`)
**Successor:** [PHASE_2.md](PHASE_2.md)
**Contract:** [`../DESIGN.md`](../../../DESIGN.md) (Deliverables 1–8, all LOCKED — unchanged)

This document is a historical record. The turn-loop skeleton it
produced lives in the `aivyx-core`, `aivyx-capability`, `aivyx-audit`,
and `aivyx-channel` crates; this file explains *how* it came together
and what was deliberately left for Phase 2.

## Goal (as written at phase entry)

Produce the **first compiling turn-loop skeleton** — the code path
that takes a `Message` in from a `ChannelContext` and returns a
`TurnOutcome`, with capability checks enforced, even if no real
tools or LLM providers exist yet. This is the structural proof that
the Phase 0 contract can carry real execution.

## What shipped

- **`aivyx-capability`** — real `Scope`, `CapabilitySet`, `TrustTier`
  replacing the D8 stubs. Prefix-attenuation intersection with four
  qualifier kinds (URL prefix / path glob / allowlist / simple glob)
  and all four D4 attenuation rules. Tier ceilings for `Untrusted`,
  `SemiTrusted`, `Trusted`, `Kernel`. **27 unit tests.**
- **`aivyx-audit`** — real `AuditEvent` (5 D4 variants), `SignedEntry`,
  `AuditLog` / `AuditWriter` traits, and `HmacChainLog` — HMAC-SHA256
  chain over RFC 8785-canonical event bytes with versioned genesis
  seed `"aivyx-audit-v1-genesis"`. Tamper detection covered on both
  event bytes and `prev_mac`. **11 unit tests.**
- **`aivyx-core`** — `ConcreteAgent` and the Phase 1 turn loop in
  `crates/aivyx-core/src/agent.rs`, the `TurnPlanner` seam and
  deterministic `VecPlanner` in `crates/aivyx-core/src/planner.rs`,
  R1-shaped `Tool::required_scope(&self, input: &Value) -> Scope`,
  forward-declared `AuditHook` + `AuditTag` to break the cycle with
  `aivyx-audit`, and the 14-variant `AivyxError` per D6. **19 unit
  tests, 6 of them end-to-end async turn-loop runs.**
- **`aivyx-channel`** — re-exports `ChannelContext`, `ChannelPlatform`,
  `StreamEvent`, `AttachmentKind`, `ChannelError` from `aivyx-core`.
  The trait itself lives in core (see "ChannelContext location" below)
  so that downstream channels can depend on either crate without
  drift. No runtime impls yet.

**Totals:** 57 tests, `cargo test --workspace` green,
`cargo clippy --workspace --all-targets -- -D warnings` clean.

## Refinements landed

### R1 — `Tool::required_scope` takes input

Landed in task 3. Signature changed from
`required_scope(&self) -> Scope` to
`required_scope(&self, input: &Value) -> Scope`. The turn loop in
`ConcreteAgent::run_tool_call` calls it on each tool call and records
the **derived** scope in the audit event, not the tool's nominal
scope. The `r1_derived_scope_recorded_in_audit_not_bare_scope` test
pins this behavior: a tool that derives `memory.read:session:abc`
from input still passes the scope check when the agent only holds
bare `memory.read`, and the audit trail shows `session:abc`.

### R2 — Window-control scopes stay Reserved

As planned. No Phase 1 tool needed them, so the D4 Reserved section
is unchanged. The concrete display-control taxonomy still belongs
to the downstream tool crate that first implements it.

## Decisions made during Phase 1 that aren't in DESIGN.md

These are the judgment calls and close-run choices that shaped the
implementation but don't appear as contract text. Recorded here so
Phase 2 has the reasoning, not just the result.

### `ChannelContext` lives in `aivyx-core`, not `aivyx-channel`

Task 3 discovered a dependency cycle: `aivyx-core` needs
`ChannelContext` in `Agent::turn`'s signature, but the first draft
had the trait in `aivyx-channel` depending on `aivyx-core` for
`SessionId` / `ChannelPlatform` / `TrustTier`. Two options:

1. Split `aivyx-channel` into a types crate and an impls crate.
2. Move the trait into `aivyx-core` and keep `aivyx-channel` as a
   thin re-export crate for the runtime channel impls that land in
   Phase 3.

Chose (2) — splitting would violate the 9-crate lock from D8 and
create a second boundary to re-export across. `aivyx-channel` still
exists, still holds its own reference impl (`LocalChannel` when it
arrives), and still publishes a `ChannelContext` type — the type
definition just happens to be re-exported from core.

### Forward-declared `AuditHook` trait in `aivyx-core`

The parallel cycle is core ↔ audit: `aivyx-core` needs to *call*
an audit sink from the turn loop, but `aivyx-audit` needs the
`AuditEvent` shape and capability types from `aivyx-capability`,
which core already depends on.

Resolved by declaring a minimal `AuditHook` trait and an
`AuditTag` enum inside `aivyx-core`, mirroring the five D4 event
variants. `ConcreteAgent` holds an `Arc<dyn AuditHook>` — it does
not know about `aivyx-audit` at all. The blanket bridge
`impl<T: AuditWriter> AuditHook for T` in `aivyx-audit` is still
TODO (see Open Issues below); nothing in Phase 1 requires it
because the tests use a recording fake, and Phase 2's LLM tests
will exercise the same seam.

### `TurnPlanner` as the loop's step source

Introduced in task 4 as the seam between the turn loop and whatever
drives tool calls. In Phase 1 the only impl is `VecPlanner` — a
deterministic walker over a fixed `Vec<NextStep>` — so the tests
can pin exact tool-call sequences. In Phase 2 the LLM provider
will supply its own `TurnPlanner` impl that streams tokens and
parses tool calls on the fly. The loop itself is oblivious.

The important consequence: Phase 1 is **not** waiting on an LLM
trait to validate the D1 paragraph. The planner seam means the
loop body can be tested end-to-end with zero LLM code — scope
denial, cancellation, R1-derived scopes, D1 Scenario 3 all land as
unit tests against fakes, not integration tests against a model.

### `StepObservation` carries a summary, not a full `ToolOutcome`

The planner sees `ToolOutcomeSummary` (Completed / Denied / Failed /
TimedOut), not the full `ToolOutcome` with its `serde_json::Value`
payload. This keeps the audit trail authoritative — a planner can't
branch on the raw output of a tool call and make decisions based on
secret-y data that never appears in the audit log. If Phase 2's LLM
planner eventually needs output text it will get it through a
*separate*, audited channel (likely the assistant-message thread),
not through observation peeking.

### `ConcreteAgent` in `aivyx-core`, not a new crate

Task 4's implementation could have lived in a new `aivyx-agent`
crate. Kept it in core because the D8 layout already collapses tools
and turn loop into core, and a separate agent crate would have the
same export-almost-everything symptom that the original 11→9 collapse
was designed to prevent.

### `ConcreteAgent` takes a planner *factory*, not a planner

`ConcreteAgent` holds
`planner_factory: Box<dyn Fn() -> Box<dyn TurnPlanner> + Send + Sync>`
instead of a `Mutex<Box<dyn TurnPlanner>>`. Two reasons: a real
agent runs turns concurrently behind an `Arc<dyn Agent>` and a
per-agent mutex would serialize them, and a fresh planner per turn
matches how an LLM stream will work in Phase 2 (each turn starts a
new completion request).

### `LoopOutcome` as an internal enum

The loop body translates to a small internal
`LoopOutcome { Completed, Cancelled }` before the public `TurnOutcome`
is built. Phase 1 only produces those two variants; `TimedOut`,
`Failed`, and `Escalated` exist in the public shape but nothing
emits them yet. The internal enum keeps the loop body terse and
gives a single synthesis point for the public outcome.

### `input_hash` in audit, not raw input

Every `ToolCall` audit event stores `input_hash: [u8; 32]`
(SHA-256 of the canonicalized JSON input bytes), not the raw input.
Secrets that appear in tool inputs never reach the audit log. The
hash is sufficient to verify that a later replay uses the same
input, which is all the forensic record needs.

### Allowlist qualifier dispatch: both-sides comma check

Q1 at phase entry asked whether qualifier-kind dispatch should look
at the *needed* or *held* qualifier. Convention adopted:

- URL and path qualifiers are dispatched by the shape of *either*
  side (paths always win over allowlists — checked first).
- Allowlist is recognized if *either* side contains a comma, because
  allowlists live on the held side by convention
  (`shell.exec:git,ls,cat` grants `shell.exec:git`) but the needed
  side is bare.
- Simple-glob is the fallback.

This was forced by a latent bug caught during task 1: a path
qualifier with brace-alternation
(`fs.read:/home/{julian,root}/**`) against a comma-free needed side
would have been misclassified as an allowlist pattern. Three
regression tests pin this:
`rule3_allowlist_subset_match`,
`dispatch_brace_alternation_path_not_allowlist`, and
`dispatch_url_with_comma_in_query`.

**Q1 resolution:** keep as phase-1 convention. Not promoted to a
D4 amendment — the convention is load-bearing but small, and the
tests are the authoritative record. Revisit if a real tool's scope
layout forces it.

## Bugs caught in Phase 1

- **`QualifierKind::of` dispatching on one side only** — the original
  `rule3_allowlist_subset_match` failed because the classifier only
  looked at the needed qualifier. Fixed by checking both sides in
  the right order (path → allowlist → glob). Three regression tests
  added. Caught by a realistic unit test, not by code review — the
  "one test, one bug" property held.
- **`hmac` 0.13 API rename** — `new_from_slice` moved from the `Mac`
  trait to `KeyInit` between 0.12 and 0.13. Compiler error in
  `HmacChainLog::append_locked`; one-line fix.
- **Orphan-rule `Display for Scope` attempt in core** — drafted a
  local `impl Display for Scope` inside `aivyx-core` for
  `AivyxError`'s `#[error("...{scope}...")]` templates. Orphan rule
  refused it. Correct fix: added real `Display` + `as_str()` to
  `aivyx-capability` where the type lives.
- **`trust_tier` field in `AuditTag::TurnStarted`** — added during
  task 4 for the D1 trail; broke one earlier test that constructed
  the variant without the field. Fixed by updating the test.

## Decisions deferred to Phase 2

- **`impl<T: AuditWriter> AuditHook for T` blanket bridge** in
  `aivyx-audit`. Phase 1 uses a test-local `RecordingAudit` that
  implements `AuditHook` directly, so the bridge is not on the
  critical path. Phase 2 will need it as soon as the first real
  agent run wants to persist to `HmacChainLog`. Flagged, not done.
- **Timeout enforcement.** `TurnOutcome::TimedOut` exists as a
  public variant but the Phase 1 loop does not consult a deadline.
  Phase 2's LLM integration is the natural place to add it — a
  real completion call is the first thing that can actually take
  long enough to matter.
- **`RequiresEscalation` propagation.** The `ToolOutcome` variant
  exists but no Phase 1 tool emits it. Lands when a tool that
  actually needs tier escalation is written.
- **`LlmProvider` trait.** Deliberately **not** in Phase 1, per D3.
  Phase 2's entire scope.

## Lessons carried forward

- **Forward-declared traits beat crate splits for cycles.** The
  `AuditHook` / `AuditTag` pattern in core let Phase 1 wire the turn
  loop against audit without either crate depending on the other in
  a way that would force a new crate boundary. Reach for this first
  when a cycle shows up.
- **The planner seam let Phase 1 test the loop end-to-end without
  an LLM.** The D1 paragraph is now validated as executable code,
  not just prose, *before* any real model integration exists. This
  is the payoff for keeping `Agent` and `Tool` dyn-compatible and
  for pushing the step source behind a trait rather than hard-coding
  it into the loop.
- **Run the realistic test early.** Q1 on allowlist dispatch was
  caught the first time a realistic unit test ran, not by reading
  the spec. The spec was ambiguous; the test forced the ambiguity
  into the open where it could be resolved.
- **`summary` vs. full-outcome split keeps audit authoritative.**
  `ToolOutcomeSummary`, `VerificationSummary`, `TurnOutcomeSummary`,
  `TrustTierSummary` — each exists so the audit format stays stable
  when the underlying full enum grows new fields. Write the summary
  type *first*, not as an afterthought, when adding new audit
  variants in later phases.
- **DESIGN.md did not move.** `git diff DESIGN.md` is empty at
  phase exit. Phase 0's contract carried Phase 1 without a single
  amendment — the first and most important signal that the Phase 0
  up-front design work was worth it.

## Exit criteria (all met)

- [x] `aivyx-core` turn loop compiles against real
      `aivyx-capability` and `aivyx-audit` (not stubs)
- [x] End-to-end test: fake channel → turn loop → fake tool →
      `TurnOutcome::Completed` with audit trail verified
      (`golden_path_completes_with_correct_audit_trail`)
- [x] End-to-end test: scope-denied case returns a completing
      `TurnOutcome` and emits a `ScopeDenied` audit event
      (`scope_denied_emits_scope_denied_audit_not_tool_call` +
      `d1_scenario3_rm_rf_from_telegram_e2e`)
- [x] R1 refinement landed end-to-end
      (`r1_derived_scope_recorded_in_audit_not_bare_scope`)
- [x] Cancellation path covered
      (`cancellation_before_first_step_yields_cancelled`)
- [x] `cargo test --workspace` green (57 tests)
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [x] All Phase 0 contract shapes still compile without modification
      (`git diff DESIGN.md` is empty — no silent drift, no amendment
      needed)

# Prompt-Injection Tripwire (aivyx-injection-guard) — Design

## Goal

`aivyx` has no active defense against prompt-injection payloads hidden in
untrusted tool output — only a passive one. `crates/aivyx-core/src/agent.rs`'s
Bulwark mechanism (`fence_untrusted_output`) wraps flagged tool output in an
`aivyx_untrusted_content_warning` envelope so the *model* is told to be
skeptical of it, but nothing actively scans for known injection phrasings or
halts execution on a match. Confirmed via grep: zero hits for "ignore
previous instructions" / `INJECTION_MARKERS` / `InjectionFinding` anywhere in
`aivyx`'s own codebase.

This gap matters specifically because of `aivyx`'s autonomy dial
(`AutonomyLevel`: Manual / Assisted / Supervised / Autonomous / Unleashed,
`crates/aivyx-config/src/autonomy.rs`) — at the `Autonomous`/`Unleashed`
tiers the agent runs fully unattended, exactly the scenario where an
undetected injection in fetched content could steer the agent with nobody
watching.

The sibling repo `aivyx-coder` already solved this exact problem:
`crates/aivyx-sandbox/src/injection_scan.rs` (260 lines) is a phrase-list
tripwire that scans untrusted content for known injection markers and pauses
an unattended run for human review. This is the fourth extraction in the
established `aivyx-confine` / `aivyx-checkpoint` / `aivyx-kvcache` pattern: a
narrow, already-solved, technical problem `aivyx-coder` still uses today,
adopted by `aivyx` rather than reimplemented.

## What already exists and will be reused, unchanged

- **`aivyx-coder`'s `injection_scan.rs`** — case-insensitive substring
  matching against a fixed `INJECTION_MARKERS` list ("ignore previous
  instructions", "you are now", "disregard your instructions", etc.), a
  64KB bounded scan window, and an `InjectionFinding { source,
  matched_pattern, excerpt }` result type. Explicitly documented as "a
  tripwire, not a classifier" — an accepted false-positive/negative rate in
  exchange for surfacing a match for a human to glance at, never a silent
  block.
- **`TurnOutcome::Escalated { reason: String, pending_tool: ToolId, scope:
  Option<Scope>, tool_calls_made: usize }`** (`crates/aivyx-core/src/lib.rs`)
  — an existing, fully-wired outcome variant. Today it's produced only from
  a tool's own `ToolOutcome::RequiresEscalation` (Phase 35). Nothing else
  triggers it.
- **The gate/headless-refusal split** (`crates/aivyx-channel/src/
  daemon_server.rs`, `escalation_parks(policy) = !policy.is_headless()`) —
  when a turn escalates: in an attended session, it creates a real
  `ApprovalGate` a human resolves from the mission dashboard; in a headless
  (unattended) session, it's refused outright and recorded as
  `AuditEvent::HeadlessRefusal` (Phase 78's "autonomous-action-must-stay-
  legible" posture) — the turn still finalizes as `Escalated`. This already
  correctly adapts behavior per attended-vs-unattended with zero
  tier-specific logic required from whatever triggers the escalation.
- **Bulwark's `output_is_untrusted()` tool set and call site**
  (`crates/aivyx-core/src/agent.rs:1372`) — the exact set of tools whose
  output could plausibly carry an injected payload is already identified
  and already has a hook point.

## What's new

### `aivyx-injection-guard` (new sibling repo)

Extracted from `aivyx-coder`'s `injection_scan.rs`. Public contract mirrors
what already exists and works there: a `scan(text: &str, source: &str) ->
Vec<InjectionFinding>` function, the `INJECTION_MARKERS` list, the 64KB scan
window, and the `InjectionFinding` struct. `aivyx-coder` migrates its own
call sites onto this crate and removes its local copy — the same relationship
`aivyx-coder`'s `aivyx-sandbox` already has with `aivyx-confine`.

Adopted into `aivyx` as a pinned-rev git dependency in
`[workspace.dependencies]` (`Cargo.toml`), with the same "MUST stay a public
repo" comment pattern Phase 192 established for `aivyx-confine` /
`aivyx-checkpoint` / `aivyx-kvcache`.

### Integration point in `aivyx-core`

**Correction (re-grounded before Phase 196's plan was written): the
integration point is one level lower than originally stated above.**
Bulwark's call site (`agent.rs:1372`) lives inside `Agent::run_tool_call`,
which returns `ToolOutcome` (per-tool-call), not `TurnOutcome` (per-turn) —
those are different types. `TurnOutcome::Escalated` is never constructed
directly at this site; it's the *turn loop* (the caller of
`run_tool_call`, `agent.rs` around line 566) that already inspects the
returned outcome:

```rust
if let ToolOutcome::RequiresEscalation { reason, scope } = &outcome {
    let escalated_scope = scope.clone();
    planner.observe_tool_outcome(tool_id, &outcome).await;
    loop_outcome = LoopOutcome::Escalated {
        reason: reason.clone(),
        pending_tool: tool_id,
        scope: escalated_scope,
    };
    break;
}
```

— and this existing check already produces the full escalation flow
(`LoopOutcome::Escalated` → `TurnOutcome::Escalated` →
`ApprovalGate`/`HeadlessRefusal` via `daemon_server.rs`'s
`escalation_parks`), with `pending_tool` populated automatically from the
turn loop's own already-known `tool_id` — not something the tripwire needs
to supply itself.

So the tripwire's actual, corrected job is much narrower: at the existing
Bulwark call site, before (or in place of) fencing, run
`aivyx_injection_guard::scan_for_injection_markers` against the tool's
completed output — serialized via the JSON value's own `to_string()`
(confirmed real untrusted-tool output, e.g. `fs.read`/`web.fetch`, returns
a structured JSON object like `{"content": ..., ...}`, not a bare string,
so the marker phrase could be nested anywhere inside it). If it returns a
match, overwrite that step's `outcome` — currently
`ToolOutcome::Completed { output, .. }` — with:

```rust
ToolOutcome::RequiresEscalation {
    reason: format!("content flagged as a likely prompt injection (matched \"{}\"): {}", finding.matched_pattern, finding.excerpt),
    scope: None,
}
```

- `scope: None` — this is not a capability-scope escalation, so the
  existing reversible/irreversible classification `escalation_parks`'s
  unattended-gate logic performs on `scope` doesn't apply here — an
  injection match always either parks (attended) or headless-refuses
  (unattended); it never silently auto-approves the way an allowlisted
  reversible action might.

No new plumbing is needed for the human-approval or audit path, the
`LoopOutcome`/`TurnOutcome` conversion, or `pending_tool` population — all
already exist and already work correctly for the *existing*
`RequiresEscalation` producer (a tool's own capability check). The
genuinely new code in `aivyx` itself is only: the scan call, the
match-to-`RequiresEscalation` conversion, and the new dependency
declaration.

## Testing

- **`aivyx-injection-guard`**: ports `aivyx-coder`'s existing unit tests for
  `scan` verbatim — marker matching, excerpt extraction, case-insensitivity,
  scan-window bounding.
- **`aivyx-core`** (new tests in `agent.rs`'s existing test module — per
  the corrected integration point above, the most direct test targets
  `run_tool_call` itself, since that's the function whose return value
  actually changes; a full-turn test confirming `TurnOutcome::Escalated`
  is the end-to-end integration check on top):
  - A fake untrusted tool returning content containing an injection marker
    confirms `run_tool_call` returns `ToolOutcome::RequiresEscalation` with
    a `reason` naming the match, and that a full `Agent::turn` call
    surfaces this as `TurnOutcome::Escalated` with the correct
    `pending_tool` (populated by the existing, unchanged turn-loop code).
  - A fake untrusted tool returning *clean* content confirms Bulwark's
    fencing still applies unaffected — the two mechanisms must coexist, not
    clobber each other.
  - A match under a headless gate policy confirms the `HeadlessRefusal`
    audit event fires, reusing the Phase 78 test pattern for that assertion.

## Out of scope for this adoption

Real candidates for a later phase, not forgotten — just not bundled into
this first extraction:

- **Expanding the phrase list** beyond what `aivyx-coder` already ships.
  Ported verbatim, not enhanced, in this phase.
- **A config knob to disable the tripwire.** Mirrors `aivyx-coder`'s own
  always-on behavior today; revisit if false positives prove disruptive in
  real operator use.
- **Scanning non-tool-output inputs.** `aivyx-coder`'s repo-map/`AGENTS.md`-
  equivalent inputs have no analog in `aivyx` today — scope is exactly the
  `output_is_untrusted()` tool set.

## Declined alongside this survey (not candidates)

Checked during the same investigation, real code opened in both repos, not
inferred:

- **`aivyx-sandbox`'s `confirmation.rs`** — tightly coupled to `aivyx-coder`'s
  own `PlanMode`/`AutonomousMode`/editor-approval modals; genuinely
  different shape from `aivyx`'s capability-based `Scope`/`CapabilitySet`/
  `TrustTier` permission model (DESIGN.md D4/D5). The two products solve
  "should this action be allowed" differently on purpose.
- **`aivyx-sandbox`'s `editor_approval.rs`** — ACP/editor-integration
  specific; `aivyx` has no in-editor surface.
- **`aivyx-acp`** — wraps the external `agent-client-protocol` crate for
  editor integration; no analog need in `aivyx`, which isn't an in-editor
  coding agent.

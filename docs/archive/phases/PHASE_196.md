# Phase 196 — Chapter Picket, Phase 3: Adopt aivyx-injection-guard into aivyx

**Chapter Picket, phase 3 — [SHIPPED] 2026-09-05. Chapter Picket is now COMPLETE.**

## Goal (carried from the design)

`aivyx` had only a passive defense against prompt injection: Bulwark's
`fence_untrusted_output` labels untrusted tool output for the model, but
nothing actively scanned for known injection phrasings or halted
execution on a match. Phases 194-195 extracted and adopted
`aivyx-injection-guard` (the phrase-list tripwire `aivyx-coder` already
used in production) as a shared crate. Phase 196 closes the loop: `aivyx`
itself gains an active tripwire, at Bulwark's existing untrusted-tool-
output call site, reusing the existing `TurnOutcome::Escalated` →
`ApprovalGate`/`HeadlessRefusal` flow.

## What shipped — and what the review cycle actually caught

The implementation task itself passed its task-level review clean. The
**final whole-branch review found a real, HIGH-severity bug in the
original plan's own design** (not an implementer deviation — the plan I
wrote and the implementer followed correctly): the scan ran *after*
`tool.execute()` and rewrote a mutating, already-executed tool's real
`ToolOutcome::Completed` into `RequiresEscalation`. The plan's own
grounding had only checked `fs.read`/`web.fetch` for
`output_is_untrusted()` — but every tool-process and MCP tool declares it
too, including irreversible ones (`kitchen.order.send`, a real purchase
order). Rewriting `Completed` into `RequiresEscalation` for an action that
already ran made the audit chain state the opposite of what happened, and
created a real risk: on approval-resume, the daemon just sends a generic
"continue" message and starts a fresh turn — it doesn't replay the
specific tool — but since the model's own history would falsely show the
action never completed, a reasonable model could plausibly retry it
(double-sending the order).

Given the choice between a narrower interim patch (only scan side-effect-
free tools) and a proper redesign, the operator chose to redesign now.
The fix: `Agent::run_tool_call` always returns the real, unmodified
`ToolOutcome` (audit chain and model context stay accurate for every
call, exactly as before this phase for non-flagged content), plus a
separate `Option<String>` side-channel signal the turn loop checks *after*
recording that real outcome — breaking for the *next* step instead of
misrepresenting the one that already ran. The existing
`RequiresEscalation` → `LoopOutcome::Escalated` → `TurnOutcome::Escalated`
→ `ApprovalGate`/`HeadlessRefusal` handling needed zero changes.

**The re-review didn't just trust this fix — it mutation-tested it**:
temporarily reintroducing the exact original bug into the fixed code to
confirm the new regression test genuinely fails when it should, not just
passes coincidentally. It held. But the same re-review found the fix's
own claim of also resolving a second finding (the injection excerpt
reaching model-readable surfaces) was only partially true — direct code
tracing turned up two real, previously-unchecked sinks: `mission.status`'s
gate output (no Bulwark fencing of its own) and, if enabled, the skill
auto-proposer's LLM judge prompt, both of which still carried the raw
~180-byte excerpt. A second fix dropped the excerpt entirely from the
escalation reason, keeping only the matched marker text (one of the
crate's own fixed, non-attacker-controlled strings) and the tool name — a
third, final review confirmed via direct grep that zero code references
to the excerpt remained anywhere in the escalation path, and flagged one
last non-blocking gap (no test guarded against the excerpt being
reintroduced — again confirmed via mutation testing), closed with one
precise regression assertion before merge.

Three real, independently-verified bugs found and fixed across this
single phase's review cycle — every one confirmed via direct code tracing
or mutation testing, not by trusting a report.

## The result

`aivyx` now has an active prompt-injection tripwire that reuses 100% of
existing escalation infrastructure, never misrepresents whether a
mutating action actually executed, and never re-surfaces the exact
payload it exists to catch back into a model-readable channel. Chapter
Picket — the fourth extraction in the `aivyx-confine`/`aivyx-checkpoint`/
`aivyx-kvcache` pattern — is complete: `aivyx-injection-guard` is a real,
public, two-consumer crate (`aivyx-coder` and `aivyx` both depend on it).

## Known follow-ups (not done here, logged for whenever they matter)

- **Finding 3, deliberately deferred across all three phases**: the
  marker list (ported verbatim from `aivyx-coder`) was tuned for a coding
  agent's file/web content, not evaluated against Gmail/Calendar/Slack/
  MCP traffic, where phrases like "you are now subscribed" are routine.
  Each false positive currently hard-stops a turn with no config knob to
  disable the tripwire. Documented directly in `check_for_injection`'s
  doc comment as a known follow-up, not fixed — a real product-scoping
  question (which tools should be scanned at all; does the list need a
  non-coding-agent variant) that deserves its own design pass, not a bolt-
  on to this phase's bug-fix cycle.
- **The gate-creation precondition** (`Some(mission_id) && Some
  (mission_store)`, `daemon_server.rs:2505`) predates this phase but is
  now reachable from any `web.fetch`-shaped tool, not just capability
  escalations: an attended session with no active mission produces no
  gate and no audit event on an injection match — the turn just finalizes
  `Escalated` with nothing further for a human to see. Noted by the final
  review as a pre-existing gap made more reachable, not a regression.

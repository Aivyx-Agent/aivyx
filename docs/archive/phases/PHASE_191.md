# Phase 191 — Daemon-Side Automatic Alert Dispatch for Tool Processes

**Chapter G, phase 2 — [SHIPPED] 2026-09-05.**

## Goal (carried from the roadmap entry)

Unlike Phases 186-190 (all Chapter I), Phase 191 had no pre-existing
Chapter I roadmap wording to ground against — that chapter's own
"Expected phases" list ended at 188, and Phase 189/190 each closed the
one remaining real gap left there. Redirected to Chapter G instead, whose
own Phase 125 exit had named three "Chapter G #2 candidates" — calendar
reminders, budget tracking, `health.check.remove` + automatic alert
dispatch — for a future phase to pick up "based on operator pressure and
observed first-real-use signal."

## Where this actually stood going in

Grounding found the first two candidates the operator initially pointed
at (budget tracking, health-check follow-ups) were both already fully
shipped: calendar reminders (Phases 141-142), budget tracking (Phases
143-144), and `health.check.remove` itself (Phase 147). But Phase 147's
own retrospective revealed the third candidate had been deliberately
split in two — it shipped `.remove` but explicitly held "automatic alert
dispatch" back, "intentionally held with Channel Activation." Digging
further found the real, original deferral: `crates/aivyx-toolkit/src/
tools/health_check.rs` and `docs/INSTALL.md` both said the same thing
verbatim since Phase 125 — "Substrate-minimal... no daemon-side automatic
alert dispatch (deferred to Phase 126+)" — and it had sat there, genuinely
untouched, for 65 phases. Today's real workaround required an operator
to configure an hourly cron that asked the *agent* to check
`health.check.recent_changes` and manually decide whether to call
`notify.send` — unreliable, and per `INSTALL.md`'s own finding, local
Ollama "won't reliably make the multi-tool call sequence" even for a
correctly-configured cron. `INSTALL.md` itself sketched the eventual fix:
"Phase 126+ may add a daemon-side IPC hook for tool processes to dispatch
notifications directly."

## What shipped

- **A new call_id-free wire protocol variant**,
  `ToolToDaemon::DispatchNotification { target, message, subject }` — the
  first mechanism letting a tool process's own background task (not a
  response to any daemon-initiated `InvokeTool`) say something unprompted.
- **`aivyx-tool` stayed dependency-clean.** `ToolProcessBridge` gained an
  injectable `NotificationSink` trait rather than a direct dependency on
  `aivyx-channel`/`aivyx-capability` — `aivyx-tool` is a generic substrate
  shared by many unrelated tool-process crates (Gmail, Calendar, Drive,
  Obsidian, Notion, n8n, Contacts, Apps) that have no business pulling in
  the full channel/capability stack.
- **`aivyx-toolkit`'s health-check polling loop** gained a side-channel
  (mpsc, multiplexed through the existing stdout writer so it can't
  corrupt normal `InvokeTool` response frames) to push a
  `DispatchNotification` on a real detected state transition, in either
  direction (down or recovered) — closing the actual gap: no cron, no
  agent turn, no model-reliability dependency required anymore.
- **A new `notify.dispatch` capability scope**, gating the daemon-side
  dispatch.
- **Three separate, real layers of the same bug class, found and fixed
  across this phase's own review cycle** — each one, if left in place,
  would have made the whole feature silently non-functional for some or
  all real operators:
  1. `KNOWN_BASES` membership (making a scope parseable) is not the same
     as `CEILING_TRUSTED` membership (making it actually grantable to a
     Trusted-tier role) — found by an implementer mid-task, not a
     reviewer, while implementing the capability check.
  2. The daemon-side notification sink shipped with a hardcoded-empty
     target string that no task in the plan could ever fill (the daemon
     has no access to a tool process's own private config file) — found
     by a task reviewer while fact-checking new documentation against the
     real shipped code, not by trusting the docs' own claims.
  3. Even with both of the above fixed, `notify.dispatch` still could
     never reach a real role's granted capability set through the actual
     production path (`compute_backcompat_floor`'s auto-grant sweep only
     picks up scopes declared by real `Tool` objects; this scope belongs
     to none) — found by the final whole-branch review, which traced the
     full grant chain end to end rather than trusting that "it's in
     `CEILING_TRUSTED`" was sufficient.
- **A real concurrency bug also caught by the final review**: the
  outbound-notification forwarder task was spawned before the
  daemon-tool-process handshake completed — a frame that raced ahead
  could be mistaken for the required first `ToolRegister` frame,
  dropping that tool process's entire surface (not just the
  notification) until restart. Fixed by moving the forwarder's spawn to
  after the handshake genuinely completes.
- **Real, non-trivial new test coverage added in the final fix wave**: a
  test constructing an actual `NotifyDispatcher` with a real registered
  backend, filling the daemon-side sink's deferred capability cell, and
  confirming a dispatched frame genuinely reaches the backend — the
  re-review explicitly confirmed this test would have caught bug layers
  #1 and #2 above, not just the one it was written to prevent going
  forward.

## The result

Chapter G's second phase closes a real, 65-phase-old, twice-independently-
verified gap. This phase is a strong data point for this session's
now-completely-established pattern — not just "a final review finds real
things," but that the SAME class of bug (a capability scope that looks
correctly wired but can never actually be granted through the real
production code path) can hide at multiple independent layers
simultaneously, and each layer needed a different kind of check to
surface: implementation-time code reading (layer 1), documentation
fact-checking against real code (layer 2), and full end-to-end call-chain
tracing on the most capable available model (layer 3). No single review
pass would have caught all three; the discipline of not accepting "it
passed its own tests" as proof, at every stage, is what did.

## Known follow-ups (not done here, logged for whenever they matter)

- **"Polish"** (Chapter I) remains the only item from that chapter's
  original roadmap wording still unscoped.
- **The forwarder-to-stdout multiplexing mechanism has no direct test**
  of its own concurrency behavior — its correctness currently rests on
  code reading (confirmed sound by the final review), not an automated
  regression guard. A real candidate for future hardening if this pattern
  gets reused by another tool process.
- **Per-watcher notify targets** — deliberately out of scope for this
  phase (one target per toolkit process); a real candidate if operator
  pressure surfaces wanting different watchers to alert different
  channels.

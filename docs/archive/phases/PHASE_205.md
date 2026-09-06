# Phase 205 — Reconciling FRONTEND.md's Studio Screen Inventory

**Chapter I documentation follow-up — [SHIPPED] 2026-09-07.**

## Goal

`docs/FRONTEND.md`'s §3 Studio screen inventory table had been flagged
stale in three consecutive phase retrospectives (188, 189, 190) and never
picked up. This phase came out of a scoping attempt for Chapter I's
separate "polish" placeholder — that investigation needs a live-backend
test environment (a real LLM + Playwright, the way Phase 189 verified
mobile-responsiveness) which isn't currently running in this environment
(confirmed: a real local GPU is present, but no Ollama/llama-server
process, no model files, and Playwright's browser binary was never
installed), so "polish" itself was paused rather than attempted with
inadequate infrastructure. This smaller, concretely-scoped documentation
fix was picked up instead.

## What shipped

- **Grounded the actual staleness directly against code** rather than
  trusting the retrospectives' own vague framing: the real `View` enum
  (`crates/aivyx-web/src/main.rs`) has 24 variants, the real `Sidebar`
  component renders 23 of them as nav items, and the table listed only
  13 — **11 real screens were missing entirely** (Schedules,
  Notifications, Loop, Reminders, MCP, Tools, Guide, Onboarding/"Create",
  Gallery, Audit, Sessions).
- **Found a second staleness the retrospectives hadn't named**: the
  existing Voice row was still marked "🔨 In progress," but `VoicePanel`,
  its `GetVoiceSettings`/`SetVoice` IPC, and the full documented
  `Voice.0`–`Voice.4` phase plan are all real and wired in — it shipped,
  and the state marker had simply never been updated.
- **Rewrote the table with a new `Group` column** mirroring the real
  `Sidebar` component's own 5 groups and item order exactly, rather than
  just appending the 11 missing rows to the existing flat list — a
  structural change intended to make the next drift more visible (a group
  with a missing member reads as obviously incomplete; a flat list's
  silent gap does not, which is exactly how this one went unnoticed for
  three phases).
- **Every new or changed cell traces to an existing source, nothing
  invented**: each new row's "Maps to" text is copied from that screen's
  own `View` enum doc comment (or, for the 3 without one — Loop,
  Reminders, Audit — from `AuditPanel`'s own doc comment and the existing
  prose already written in `docs/ROADMAP.md`'s Phase 186/188 entries).
- **Verification was direct code grounding, not automated tests** (a
  documentation-only change has none): the task's own steps diffed the
  new table's Group/order against `Sidebar`'s real group literals,
  spot-checked 4 "Maps to" cells against their cited doc comments, and
  confirmed `VoicePanel`'s wiring — all independently re-run by the task
  reviewer against the real file rather than trusted from the
  implementer's report, and separately re-confirmed a third time by the
  controller directly.
- No separate final whole-branch review was dispatched — a single-task,
  single-file, documentation-only change has no cross-task coherence for
  a broader review to catch that the task review (plus the controller's
  own direct verification) hadn't already covered twice.

## The result

`docs/FRONTEND.md` now accurately lists all 23 real Studio nav screens,
grouped to match the code, with an accurate Voice status. The three
straight phases of "flagged but not fixed" on this item are closed.

## Known follow-ups (not done here, logged for whenever they matter)

- **Chapter I's "polish" placeholder remains paused**, not resolved — see
  Phase 204's own follow-ups for the still-open question of whether/when
  live-backend test infrastructure becomes available for it, and the
  unexplored question of whether `docs/POLISH_WAVES.md` (an already-
  completed, unrelated 8-part v0.9 polish backlog that finished
  2026-09-03, one day before Chapter I's "polish" wording first appeared)
  is a genuine naming collision worth untangling before scoping any new
  "polish" work.

# Phase 128 — Google Calendar Integration (Chapter F #2)

**Chapter F second integration.** First Chapter F phase
since Phase 123 shipped Gmail. Operator-pressure pick
after Phase 127 exit listed Chapter F #2 / Chapter G #2 as
operator-tool-surface candidates; Calendar picked over
GitHub / Drive / Budget tracking for the "common operator
ask, load-bearing for the personal-assistant value prop"
framing.

**Lands the SDK harness lift alongside.** Phase 125 exit
explicitly named "land the lift alongside whichever
Chapter F #2 or Chapter G #2 ships first" — Phase 128 is
that moment. Three would-be in-tree copies (gmail +
toolkit + calendar) crosses the clear-win threshold; the
lift collapses to one shared substrate before the third
copy lands.

## Chapter F context (this is the second integration)

Phase 123 opened Chapter F with Gmail. The chapter pattern
locked at that exit:

- One target integration per phase as a **separate binary
  crate**.
- Per-service auth substrate (OAuth for Google services;
  PAT for GitHub; service-specific tokens for others).
- Per-service capability scopes registered into the
  existing capability machinery (no new core types
  required — `Scope` already accommodates arbitrary
  bases).
- INSTALL.md walkthrough covering operator-side setup
  (auth-app registration, `aivyx auth` flow,
  `[[tool_process]]` registration, per-role scope
  granting).

Phase 128 reuses the chapter pattern verbatim. Calendar
uses the **same Google OAuth flow** as Gmail — just a
different scope (`https://www.googleapis.com/auth/calendar`
instead of `https://www.googleapis.com/auth/gmail.*`).

## Why this, why now

- **Operator-pressure-driven.** Calendar picked at the
  Phase 127 direction question over GitHub / Drive /
  Budget tracking. Personal-assistant value prop has
  Calendar as a core capability; substantial operator
  utility from "agent can see and modify my calendar."

- **Chapter F substrate is paid for.** Phase 123's
  OAuth + harness + IPC substrate is reusable. Calendar
  is largely "Gmail with different API + different
  scope." Per-integration phases after the chapter
  opener are substantially cheaper, as predicted.

- **SDK harness lift is at clear-win threshold.** Phase
  125 exit twice-duplicated the harness (gmail +
  toolkit). Calendar adding a third copy would be three
  in-tree duplicates of the same multi-tool harness —
  exactly what Phase 125 named as the trigger to lift.
  Bundling the lift into Phase 128 collapses three would-
  be copies to one shared substrate.

- **Substrate completion enables product completion.**
  Phase 127 closed the substrate gap for local-LLM tool-
  call extraction. Operators with local stacks can now
  reliably invoke whatever tools we register. Calendar
  is one of the most-asked operator-value tools; making
  it available is high-leverage.

## Q-block sign-off (3 Recommended + 1 non-Recommended)

- **Q1a — Bundle SDK harness lift in Phase 128**
  (Recommended). Extract the multi-tool harness from
  `aivyx-gmail/src/harness.rs` and
  `aivyx-toolkit/src/harness.rs` to a shared module in
  `aivyx-tool` (the third-party-tool SDK crate). Both
  existing crates migrate to consume the lifted version.
  Verify all 1,108 existing tests across gmail + toolkit
  pass unchanged (behavior preservation).

- **Q2a — Copy OAuth substrate inline from aivyx-gmail**
  (Recommended). Inline-copy the `oauth/` module into
  `aivyx-calendar` with honest attribution in the
  preamble. Same posture Phase 125 used for the harness
  copy. The OAuth lift becomes worthwhile at three
  copies (gmail + calendar + future Google integration);
  Phase 128 isn't yet that moment for OAuth (it IS for
  the harness). Defer the OAuth lift to a focused
  substrate phase (Phase 129+ candidate) or to whichever
  Phase ships the third Google integration.

- **Q3b — Read + write + update (5 tools)** (non-
  Recommended; operator-picked over Q3a's 4-tool
  surface). Tools:
  - `calendar.list_events` — range query
  - `calendar.get_event` — single fetch by ID
  - `calendar.create_event` — Trusted-gated
  - `calendar.update_event` — Trusted-gated
  - `calendar.delete_event` — Trusted-gated

  **Honest framing per Phase 6 Q5:** Q3b's richer
  surface doubles the write-tool review surface vs Q3a.
  The operator picked it because `update_event` is a
  common operator ask (editing existing events is part
  of normal calendar use), and the marginal cost is one
  additional tool task (Task 7 below). Acceptable risk
  trade-off; honest report at exit if any of the five
  tools needs scope reduction.

- **Q4a — Operator-discretionary live verification**
  (Recommended). Substrate is unit-tested per-tool;
  OAuth flow is integration-tested with a fake transport
  (matches Phase 123 pattern). Live test against a real
  Google account is documented in INSTALL.md but not a
  phase exit gate. Matches the Phase 127 precedent.

**Three Recommended + one non-Recommended.** Q3b's
operator pick was explicit; PR-merge-time scope reduction
is the escape hatch if any of the five tools needs to
defer.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract amendment.
  Chapter F pattern is already established. Hash at
  entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to nineteen** (was 18
  after Phase 127).

- **PRODUCT.md** — **Will hold.** No contract change.
  G6 + P10 + P11 + P12 cover this case exactly. Hash:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to nineteen**.

- **`aivyx-core/src/lib.rs`** — **Will hold.** All
  Phase 128 work lives in `aivyx-calendar` (new crate),
  `aivyx-tool` (harness lift), `aivyx-gmail` +
  `aivyx-toolkit` (harness migration). NO core changes.
  Hash:
  `9692e5d102ca0721f1a6a958fda1d24f287e0b1295a09193db92ad82eb5cde35`.
  Prediction: streak **extends from 1 to 2** (Phase 127
  rebuilt to 1; Phase 128 ticks to 2).

- **New workspace deps** — Zero anticipated. `reqwest`,
  `serde`, `serde_json`, `tokio`, `thiserror`, `uuid`,
  `base64`, `toml` all already in use by aivyx-gmail
  (which Phase 128 mirrors). Calendar API is reqwest +
  JSON; no SDK crate needed.

- **Test count** — Five tools × per-tool input
  validation + schema tests + harness tests, plus OAuth
  tests carrying over from the inline copy (substantial
  body), plus the harness lift's behavior-preservation
  tests. Prediction: **`+150` to `+200`** anchored on
  Phase 123's `+151` for 4 tools through OAuth (Phase
  128 has one more tool + the harness lift overhead).

## Tasks

Nine sub-tasks plus exit + backfill:

### Task 1 — Open (this commit)

`docs/PHASE_128.md` + `docs/ROADMAP.md` Phase 128 entry +
`docs/README.md` status row. Documents the operator-
pressure framing + the Q-block resolutions + the harness-
lift bundling + the streak predictions.

### Task 2 — SDK harness lift

Extract the multi-tool harness:

- New module `aivyx-tool/src/harness.rs` (or new
  sub-module under the existing `aivyx-tool` crate) with
  the lifted harness logic.
- `aivyx-gmail/src/harness.rs` becomes a thin re-export
  shim OR is deleted entirely with `main.rs` importing
  from `aivyx-tool` directly.
- `aivyx-toolkit/src/harness.rs` same migration.
- Existing tests in both crates pass unchanged
  (behavior preservation is the load-bearing exit
  criterion for this task).
- Honest preamble in the lifted harness module names
  the two source files it lifted from and the Phase 125
  finding that triggered the lift.

NO new functionality; pure substrate refactor. Tests
exercise the lift via the existing gmail + toolkit
suites.

### Task 3 — `aivyx-calendar` crate skeleton + OAuth copy + capability bases

- New `crates/aivyx-calendar/` directory:
  - `Cargo.toml` — workspace member; deps mirror
    `aivyx-gmail`.
  - `src/lib.rs` — public surface (mostly re-exports
    from sub-modules).
  - `src/main.rs` — binary entry; CLI dispatch (auth |
    serve) mirroring `aivyx-gmail/src/main.rs`.
  - `src/oauth/*.rs` — copy from
    `aivyx-gmail/src/oauth/*.rs` verbatim with honest
    preamble naming the source and the Phase 125-style
    "lift to shared crate when N=3" disposition.
  - `src/auth_cli/*.rs` — copy from
    `aivyx-gmail/src/auth_cli/*.rs` with the same
    preamble; auth scope swapped from gmail.* to
    `https://www.googleapis.com/auth/calendar`.
  - `src/calendar_client.rs` — Calendar API client
    (REST against `https://www.googleapis.com/calendar/v3`).
    Empty skeleton at this task; per-tool tasks fill it.
- New capability bases registered in `aivyx-capability`:
  - `calendar.read` — gates `list_events` + `get_event`.
  - `calendar.write` — gates `create_event` +
    `update_event` + `delete_event`. CEILING_TRUSTED.
- Workspace `Cargo.toml` includes the new crate.

### Task 4 — `calendar.list_events` tool

- `CalendarClient::list_events(time_min, time_max,
  max_results, calendar_id)` — query Google Calendar's
  `/calendars/{calendarId}/events` endpoint with
  `timeMin`/`timeMax` parameters.
- IPC tool wrapper in `aivyx-calendar/src/tools/list_events.rs`
  (or similar layout).
- Input schema: `time_min` (RFC 3339), `time_max`,
  `max_results` (default 50, max 250), `calendar_id`
  (default "primary").
- Output: array of event summaries (id, summary, start,
  end, location, attendee count).
- Capability: `calendar.read`.
- Tests: input validation + happy-path round-trip with a
  fake HTTP transport + edge cases (empty range, max
  results clamping).

### Task 5 — `calendar.get_event` tool

- `CalendarClient::get_event(calendar_id, event_id)` —
  single GET against
  `/calendars/{calendarId}/events/{eventId}`.
- IPC tool wrapper.
- Input schema: `event_id` (required), `calendar_id`
  (default "primary").
- Output: full event detail (description, conference
  data, recurrence, organizer, all attendees with
  response status).
- Capability: `calendar.read`.
- Tests: input validation + happy-path + 404 handling +
  non-existent calendar_id.

### Task 6 — `calendar.create_event` tool (Trusted-gated)

- `CalendarClient::create_event(calendar_id, event)` —
  POST to `/calendars/{calendarId}/events`.
- IPC tool wrapper.
- Input schema: `summary` (required), `start` (RFC 3339,
  required), `end` (RFC 3339, required), `description`,
  `location`, `attendees` (array of email addresses),
  `calendar_id` (default "primary"), `reminders` (object
  or default).
- Output: created event ID + summary + start/end.
- Capability: `calendar.write` (CEILING_TRUSTED).
- Tests: input validation (required field enforcement +
  RFC 3339 validation) + happy-path + invalid-time
  rejection.

### Task 7 — `calendar.update_event` tool (Trusted-gated)

- `CalendarClient::update_event(calendar_id, event_id,
  patches)` — PATCH to
  `/calendars/{calendarId}/events/{eventId}` (Google
  Calendar supports partial updates via PATCH).
- IPC tool wrapper.
- Input schema: `event_id` (required), `calendar_id`
  (default "primary"), plus any of the create_event
  fields as optional patches (only fields present in
  the request are sent in the PATCH).
- Output: updated event summary.
- Capability: `calendar.write` (CEILING_TRUSTED).
- Tests: input validation + partial-patch round-trip +
  event-not-found.

### Task 8 — `calendar.delete_event` tool (Trusted-gated)

- `CalendarClient::delete_event(calendar_id, event_id)`
  — DELETE against
  `/calendars/{calendarId}/events/{eventId}`.
- IPC tool wrapper.
- Input schema: `event_id` (required), `calendar_id`
  (default "primary"), `notify_attendees` (bool,
  default false — passed as `sendUpdates=all` vs
  `sendUpdates=none`).
- Output: confirmation (just the deleted event_id).
- Capability: `calendar.write` (CEILING_TRUSTED).
- Tests: input validation + happy-path + idempotency
  on delete-already-deleted (Google returns 410 Gone).

### Task 9 — INSTALL.md walkthrough + Phase 128 exit

INSTALL.md operator-facing section under "External
productivity integrations (Chapter F)":

- "Google Calendar (Phase 128)" sub-section mirroring the
  Gmail (Phase 123) sub-section structure.
- One-time operator setup: GCP project (typically the
  SAME project as Gmail, just adding the Calendar API
  enablement), client_id + client_secret reuse (the
  same OAuth client works for both APIs), scope
  difference noted.
- `aivyx auth` subcommand: covered already for gmail;
  Calendar uses the same flow with `--service calendar`
  (or similar dispatch).
- `[[tool_process]]` registration in `aivyx.toml`.
- Capability scope granting per role.
- Troubleshooting section: scope-not-granted, token-
  refresh, Calendar-specific quotas.
- Honest scope notes: 5 tools landed (not 4 per the
  Q3a-default), Q3b operator-picked.

Phase 128 exit doc:
- Prediction-vs-reality.
- Streak summary.
- Harness-lift status (test count carried over from gmail
  + toolkit suites; no behavior regressions).
- Operator notes on the cross-service OAuth posture (one
  GCP project, two scopes, two tool processes).

## Exit criteria

- [ ] `docs/PHASE_128.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] SDK harness lifted to `aivyx-tool`; aivyx-gmail
  + aivyx-toolkit migrated; all prior tests pass — Task 2.
- [ ] `aivyx-calendar` crate skeleton + OAuth copy +
  capability bases — Task 3.
- [ ] `calendar.list_events` tool + tests — Task 4.
- [ ] `calendar.get_event` tool + tests — Task 5.
- [ ] `calendar.create_event` tool + tests — Task 6.
- [ ] `calendar.update_event` tool + tests — Task 7.
- [ ] `calendar.delete_event` tool + tests — Task 8.
- [ ] INSTALL.md walkthrough + Phase 128 exit doc —
  Task 9.
- [ ] Q1 / Q2 / Q3 / Q4 resolved pre-Task 2.
- [ ] DESIGN.md streak — predicted HOLD (streak → 19).
- [ ] PRODUCT.md streak — predicted HOLD (streak → 19).
- [ ] `aivyx-core/src/lib.rs` streak — predicted HOLD
  (1 → 2).
- [ ] Zero new workspace dependencies.
- [ ] Test count delta within `+150` to `+200`.
- [ ] Zero clippy warnings.
- [ ] **No live verification as exit criterion** —
  operator-discretionary per Q4a.

## Honest scope risks at sign-off

- **Q3b's richer surface doubles the write-tool review
  surface.** Five write tools (create + update + delete)
  is more capability surface than the Q3a default's
  four-tool shape. Phase 6 Q5 honest framing:
  PR-merge-time scope reduction is the escape hatch if
  Task 7 or Task 8 reveals issues that the simpler shape
  would have avoided.

- **OAuth copy adds tech debt.** Calendar is the second
  in-tree OAuth copy (gmail + calendar). The third copy
  (next Google integration: Drive, Photos, etc) will
  trigger the OAuth lift the same way the harness lift
  trigger fired at N=3 this phase. Honest tracking
  continues.

- **Harness lift could break behavior.** Extracting
  shared code from two consumers risks subtle behavior
  changes if either consumer relied on
  implementation-specific details. The behavior-
  preservation tests across gmail + toolkit are the
  load-bearing exit criterion. Phase 6 Q5 honest framing:
  if migration reveals semantic divergences, the lift
  may need scope reduction (lift the common subset, leave
  the divergent fragment inline).

- **Google Calendar API quotas.** Operators with heavy
  Calendar use may hit GCP project quotas. INSTALL.md
  documents the quota knobs but quota tuning is operator
  responsibility.

- **OAuth scope creep risk.** Calendar's
  `auth/calendar` scope is broad (read + write all
  calendars). A narrower scope
  (`auth/calendar.events`) covers most tool needs.
  INSTALL.md documents both options; default is the
  broader scope for fewer "re-auth with new scope" loops
  for operators.

- **Sixteenth consecutive deferral of the Channel
  Activation Milestone.** Honest tracking continues.
  Audit's #1.

## Direction after Phase 128

After Phase 128, Phase 129 candidates:

1. **Chapter F #3 — Drive or GitHub** — next external
   integration; pattern is now well-established.
2. **OAuth substrate lift** — if Chapter F #3 is another
   Google service (Drive), the OAuth N=3 trigger fires.
3. **Channel Activation Milestone** — sixteenth
   consecutive deferral if skipped. Audit's #1.
4. **Cloud-LLM-side substrate polish** — cache hit-rate
   instrumentation; tool-use latency profiling.
5. **Release prep (v0.1.0 + installer)** — substrate +
   product completion postcards a natural release
   milestone.

## Prediction vs reality

_Populated at Phase 128 exit. Predictions captured at
sign-off: DESIGN.md HOLD → 19; PRODUCT.md HOLD → 19;
lib.rs HOLD → 2; test count `+150` to `+200`; zero new
deps; zero clippy warnings._

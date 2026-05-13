# Phase 63 — Reach Phase 2: Trigger-Config Notify Sugar

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Each `[[schedule]]`, `[[webhook]]`, `[[file_watch]]` entry gains an
optional `notify_target = "..."` field. When the trigger fires
and the turn completes, the daemon auto-dispatches the agent's
final response to the named target — no agent involvement, no
system-prompt instruction needed. Closes the Phase 62-deferred
Q3 alternative and the "scheduled summary lands on my phone
without teaching the agent each time" use case.

## Why now

1. **Phase 62 substrate is in place.** `NotifyDispatcher`,
   `NotifyBackend`, two backends, and `NotifySendTool` all
   shipped. The substrate is mature enough that Phase 63 just
   adds a daemon-side caller; no new architecture.
2. **Phase 62 fixed the wrong half of the use case.** Today an
   operator who wants "9am summary on my phone" has to:
   (a) configure a schedule, (b) configure a notify_target,
   (c) write the schedule's `prompt` to instruct the agent to
   call `notify.send`. Step (c) is fragile — the agent may
   forget on long-running tasks or drift on system-prompt
   wording. Wiring the notify at trigger-fire time scales
   better than instructing the agent each time.
3. **Three pre-Q-block answers locked at design time** (sign-off
   2026-05-13):
   - Push content: agent's final response text + auto-subject
     `<kind>: <name>`.
   - Capability gate: fail at config-load time if the trigger's
     role lacks `notify.send` for the named target.
   - Failure: log + audit, no retry. One attempt per fire.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 63 adds a config field,
  a daemon-side hook, and an audit event variant. No
  D-deliverable reshape. Prediction: streak **extends to ten**
  consecutive phases (currently at 9 after Phase 62).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** No commitment-text edits, no
  Delivery Status refresh. The Reach Milestone's Phase 2 entry
  lands in `docs/PRODUCT_ROADMAP.md`, not in the contract
  document. Prediction: streak **extends to three** consecutive
  phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  Every Phase 63 surface lands in `aivyx-channel` (trigger
  dispatch hook, audit event variant), `aivyx-config` (config
  field + validation), and `aivyx-capability` (potentially
  consulted from the validator). No path touches `aivyx-core`.
  Prediction: streak **extends to eleven** consecutive phases
  (new record, beating Phase 62's 10).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

## Tasks

### Task 1 — Open commit + PHASE_63.md scaffold

This file. Update `docs/README.md` to show Phase 63 as Open.
Q-block resolutions committed before Task 2.

### Task 2 — `notify_target` config field + load-time validation

`aivyx-config` extends:

- `RawSchedule` / `RawWebhook` / `RawFileWatch` gain
  `notify_target: Option<String>`.
- Mirror to `ScheduleConfig` / `WebhookConfig` / `FileWatchConfig`.
- Load-time validation:
  1. If `notify_target` is `Some`, the named target must exist
     in the loaded `notify_targets` list. Error variant:
     `ConfigError::Invalid { field: "<trigger>.notify_target",
     reason: "unknown notify_target `<name>` referenced by
     <trigger-kind> `<trigger-name>`" }`.
  2. The trigger's role (`role` field) must have `notify.send`
     (qualified to the target, or unqualified) in its
     effective envelope as assembled by
     `assemble_role_envelope`. Error variant:
     `ConfigError::Invalid { field: "<trigger>.notify_target",
     reason: "role `<role>` used by <trigger-kind>
     `<trigger-name>` lacks `notify.send` capability required
     for notify_target `<name>`" }`.

Tests: happy path, missing-target error, missing-capability
error.

### Task 3 — Trigger dispatch hook

After each trigger-fired turn completes, if the config has
`notify_target`, extract the agent's final response text,
construct subject as `<kind>: <name>` (Q5(a) below), and call
`dispatcher.dispatch(target, message, Some(subject))`. Lives in
the trigger dispatch path so all three trigger kinds (schedule,
webhook, file_watch) inherit the behavior through one code
path.

Empty agent response → skip notify (audit with skipped=true).
Failed turn → fire notify with body `"Turn failed: <reason>"`
(Q3 below).

### Task 4 — Audit event for auto-notify

New `AuditEventKind::AutoNotifyDispatched` variant carrying
`{target_name, success, error_kind?, trigger_kind,
trigger_name}`. Sibling to `ToolCalled` / `TurnEnded`. Lets
forensic search distinguish agent-initiated `notify.send`
calls (recorded as `ToolCalled` with name="notify.send") from
daemon-initiated auto-notifies. See Q1 below.

### Task 5 — Integration tests

End-to-end tests per trigger kind:
- Schedule: build config with `notify_target`, fire trigger,
  assert dispatcher received the right message + subject.
- Webhook: same shape with a webhook trigger.
- File watcher: same shape with a file change.

Stub the dispatcher with a recording backend so tests don't
need real Telegram/webhook endpoints.

### Task 6 — Worked example update

`examples/aivyx.toml` gains `notify_target = "phone"` on the
(commented-out) schedule example, with a one-paragraph
explanation of the wiring.

### Task 7 — Exit commit

- `ROADMAP.md` Phase 63 frozen entry.
- `docs/PRODUCT_ROADMAP.md` Reach Milestone refresh: Phase 63
  moves from "deferred" to "delivered" alongside the Phase 62
  entry.
- `docs/README.md` status table flipped to Frozen with backfill
  per convention.
- Prediction-vs-reality block filled.
- Exit-criteria block completed.

## Open questions

**Q1 — Audit event shape for auto-notify?**

  - **(a)** New `AuditEventKind::AutoNotifyDispatched` variant.
    Lets the chain walker distinguish agent-initiated from
    daemon-initiated notifies without inspecting an actor
    field. Small audit-enum reshape; the existing HMAC chain
    format absorbs it transparently.
  - **(b)** Synthesize a `ToolCalled` event with `actor =
    "daemon"`. No enum reshape but requires a new field on
    `ToolCalled` to distinguish actors.
  - **(c)** Reuse the existing `DaemonLifecycleEvent` channel
    for "system did a thing." Conflates two different event
    domains.

  **Recommendation: (a).** Dedicated variant is the cleanest
  forensic-search shape. The audit enum has been extended
  before (Phase 21's mission events, Phase 30's role mutation
  events) without contract reshape.

**Q2 — Empty agent response handling?**

  - **(a)** Skip notify; audit with `success=true, skipped=true`.
    Sending an empty message to Telegram is rejected by the
    Bot API anyway; webhook endpoints with non-strict body
    validation might accept it but the operator gets no
    information from an empty notification.
  - **(b)** Send a placeholder like `"(no response)"`.
  - **(c)** Send the empty message; let the backend reject it.

  **Recommendation: (a).** Skipping is the operator-correct
  behavior; the audit event still records the trigger fired.
  If the operator sees no notification but the trigger fires,
  the audit log explains why.

**Q3 — Failed turn handling?**

  - **(a)** Fire notify with synthesized body `"Turn failed:
    <reason>"`. The operator probably wants to know their
    scheduled job blew up — silent failure is the worst
    failure.
  - **(b)** Skip notify on failed turns; rely on audit-log
    inspection. Risks operator never noticing scheduled jobs
    are broken.
  - **(c)** Send a different format (e.g. just the reason, no
    `"Turn failed:"` prefix). Less informative.

  **Recommendation: (a).** Notify-on-failure is the safer
  default. Audit event marks the failure context (Q1).

**Q4 — Trigger kind label in subject?**

  - **(a)** `<kind>: <name>` (e.g. `Schedule: morning-summary`,
    `Webhook: ci-events`, `FileWatch: notes-dir`).
  - **(b)** Just the trigger name (`morning-summary`).
  - **(c)** Operator-configurable per trigger.

  **Recommendation: (a).** Kind prefix tells the operator at
  a glance which trigger kind fired. The prefix is short
  enough to not crowd Telegram subjects or Pushover titles.

**Q5 — Validation surface: when does the validator consult the
role envelope?**

  - **(a)** At config-load time. The validator calls
    `assemble_role_envelope` for each trigger's role to check
    `notify.send` is granted. Mirrors the existing
    `validate_role_inheritance` walk; same load-time-not-
    runtime discipline.
  - **(b)** At daemon startup, after `assemble_role_envelope`
    has run for the active role. Slightly later but lets the
    error reference the actual effective envelope.

  **Recommendation: (a).** Earliest-failure discipline. The
  operator gets the error at `aivyx daemon run` startup, not
  at 9am the next morning when the schedule fires.

## Deferrals

**Rolling deferrals carried into Phase 63:**

- v0.1.0 publication (Phase 61 Task 7) — held under VPS-first
  posture.
- Identity export/import (Phase 60) — multi-device portability.
- System-prompt `## Notification targets` block (Phase 62
  Task 8 scope adjustment).
- Web UI desktop notify, email SMTP, OS-level notifications,
  default-target sugar, per-target rate limits, notification
  templates, Slack-flavored webhook, shared Telegram transport
  (all Phase 62 deferrals).

**Likely Phase 63 deferrals (filled in at exit):**

- Auto-notify rate limits (per-trigger cooldown — currently
  the trigger's own debounce/throttle is the only rate limit).
- Operator-templated notify body (`notify_template = "..."`
  with `{response}`, `{outcome}` placeholders).
- Auto-notify retry on failure (deferred from the pre-Q-block).
- Multi-target dispatch from one trigger (`notify_target =
  ["phone", "ops-alerts"]`).
- Conditional notify (`notify_on = "escalated"` etc.).

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `notify_target` field added to schedule/webhook/file_watch
  TOML; load-time validation passes for happy paths and rejects
  unknown-target / missing-capability — Task 2.
- [ ] Trigger dispatch hook fires `dispatcher.dispatch` after
  the trigger-fired turn completes — Task 3.
- [ ] `AuditEventKind::AutoNotifyDispatched` recorded for each
  fire (including skipped/failed cases) — Task 4.
- [ ] Integration tests cover schedule + webhook + file_watch
  paths — Task 5.
- [ ] `examples/aivyx.toml` schedule example annotated with
  `notify_target` and rationale — Task 6.
- [ ] ROADMAP.md + PRODUCT_ROADMAP.md + docs/README.md
  refreshed — Task 7.
- [ ] All five Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to ten.
- [ ] PRODUCT.md streak extends to three.
- [ ] Production-core streak extends to eleven (new record).
- [ ] Test count delta: positive (~+15–20).
- [ ] Zero clippy warnings.
- [ ] Prediction-vs-reality block filled.

# Phase 67 — Auto-Notify Audit Event

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Close the Phase 63 deferred Q1(a) sign-off:
`AuditEvent::AutoNotifyDispatched` lands as a new variant of
the audit chain enum, and `TriggerDispatch` gains the
audit-hook plumbing needed to emit it. Forensic search now
distinguishes agent-initiated `notify.send` calls (recorded
as the existing `ToolCall` event) from daemon-initiated
auto-notify fires (recorded as the new variant), and
operators can definitively answer "why didn't my morning
briefing arrive?" from the audit chain alone.

After Phase 67, every time a configured `notify_target` fires
(or is deliberately skipped, or fails), an
`AutoNotifyDispatched` entry lands in the persistent audit
log alongside the corresponding `TurnStarted` / `TurnEnded`
events. Correlation uses `session_id` (already minted per
trigger fire and recorded on `TurnStarted`).

## Why now

1. **Phase 63 deferral closure.** The Q1(a) sign-off chose
   the dedicated variant at design time but deferred
   implementation when the substrate ramifications surfaced
   (`TriggerDispatch` had no audit-hook reference). The
   plumbing turned out smaller than the deferral note
   suggested — `DaemonConfig` already carries
   `audit_log: Option<Arc<PersistentAuditLog>>` (Phase 47).
   Phase 67 just threads it into `TriggerDispatch`.
2. **Forensic-search-shaped substrate.** Aivyx already
   audit-logs every tool call, every turn start/end, every
   memory access. Auto-notify being eprintln-only is the
   conspicuous gap. Closing it makes the audit chain a
   complete record of operator-affecting daemon behavior.
3. **Q1–Q4 fully resolved at design time.** Rich event with
   `session_id` correlation (Q1, adjusted at scoping —
   `turn_id` requires a 120-site `TurnOutcome` refactor and
   is deferred). Audit every fire including skipped (Q2).
   Log + continue on audit emission failure (Q3). Narrow
   scope to auto-notify only (Q4) — mission state audit
   plumbing stays a future phase.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 67 extends the audit
  enum additively (new variant alongside existing
  `ToolCall`, `TurnStarted`, etc.) and wires the audit hook
  into `TriggerDispatch`. No D-deliverable reshape.
  Prediction: streak **extends to fourteen** consecutive
  phases (currently at 13).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** No commitment-text edits;
  no Delivery Status refresh. Auto-notify auditing is
  operator-feedback-shaped substrate, not a P1–P14 commit.
  Prediction: streak **extends to seven** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  Audit variant lives in `aivyx-audit`; plumbing lives in
  `aivyx-channel`. No path touches `aivyx-core`. Prediction:
  streak **extends to fifteen** consecutive phases (new
  record, beating Phase 66's 14).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

## Tasks

### Task 1 — Open commit + PHASE_67.md scaffold

This file. Update `docs/README.md` to show Phase 67 as Open.

### Task 2 — `AuditEvent::AutoNotifyDispatched` variant

`aivyx-audit::AuditEvent` gains:

```rust
AutoNotifyDispatched {
    session_id: SessionId,
    trigger_kind: TriggerKindSummary,
    trigger_id: String,
    target_name: String,
    outcome: AutoNotifyOutcomeSummary,
    dispatched_at_unix_ms: u64,
},
```

Plus two new supporting enums in the same crate:

```rust
pub enum TriggerKindSummary { Cron, Webhook, FileWatch }

pub enum AutoNotifyOutcomeSummary {
    /// Dispatch succeeded.
    Delivered,
    /// Skipped because the agent's turn produced an empty
    /// response (Phase 63 Q2(a)).
    SkippedEmptyResponse,
    /// Backend returned an error (network, auth, rejected,
    /// timeout, unknown target). `error_kind` mirrors the
    /// `notify.send` tool's classification.
    Failed {
        error_kind: String,
        error_message: String,
    },
}
```

The chain serialization shape: `#[serde(tag = "kind")]` so the
existing chain readers (Web UI `ListAuditEntries`, `aivyx
--verify-only`) parse the new variant cleanly without per-
reader changes.

### Task 3 — `TriggerDispatch` audit-hook plumbing

`crates/aivyx-channel/src/trigger.rs`:

- New `audit_log: Option<Arc<PersistentAuditLog>>` field on
  `TriggerDispatch`.
- New `with_audit_log` builder.
- `DaemonConfig` already carries the field (Phase 47); the
  `run_daemon` setup block passes it to
  `TriggerDispatch::with_audit_log` when `Some`.

### Task 4 — Emit `AutoNotifyDispatched` from the auto-notify path

In `fire()` after the dispatcher call (or its skip), emit:

- `outcome = Delivered` on dispatch success.
- `outcome = SkippedEmptyResponse` on the empty-body skip
  path.
- `outcome = Failed { error_kind, error_message }` on
  dispatcher error.

The emission uses `audit_log.append(SignedEntry { event, … })`
matching the existing chain-append pattern. On audit append
error, log to eprintln and continue per Q3(a).

### Task 5 — Tests

- Variant round-trips through JSON (Phase 47-style
  `#[serde(tag = "kind")]` parse + reparse).
- `TriggerKindSummary::from(&TriggerSource)` conversion test.
- `AutoNotifyOutcomeSummary::from(&NotifyError)` conversion
  test for the three error-classification cases.
- Integration test: spin up a `TriggerDispatch` with a
  scripted notify dispatcher + an in-memory audit log;
  fire a trigger; verify the audit log holds an
  `AutoNotifyDispatched` entry with the expected fields.
- Coverage for the three outcome variants (delivered,
  skipped, failed).

### Task 6 — Docs note

Brief addition to a section of `docs/INSTALL.md` (or
`docs/TEMPLATES.md`) mentioning that scheduled briefing fires
are recorded in the audit log under `AutoNotifyDispatched` for
debugging "why didn't my briefing arrive?" workflows.

### Task 7 — Exit commit

- `ROADMAP.md` Phase 67 frozen entry.
- `docs/PRODUCT_ROADMAP.md` Reach Milestone refresh: Phase
  63 Q1(a) deferral now reads "closed by Phase 67."
- `docs/README.md` status flip with backfill.
- Prediction-vs-reality block filled.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Field set:** Rich event with `session_id`,
  `trigger_kind`, `trigger_id`, `target_name`, `outcome`,
  `dispatched_at_unix_ms`. **Implementation-time scope
  adjustment:** `turn_id` correlation deferred because
  `TurnOutcome` doesn't carry `turn_id` and adding it
  touches 120 match sites — its own future phase. The
  `session_id` is already minted per trigger fire and
  recorded on `TurnStarted`, so it serves as a correlation
  key without the upstream refactor.
- **Q2 — Skip case:** Yes, audit every fire including
  skipped (operator can answer "why didn't my notification
  arrive?" from the chain).
- **Q3 — Audit emission failure:** Log + continue.
- **Q4 — Plumbing scope:** Narrow — just AutoNotifyDispatched.
  Mission state changes from triggers remain unaudited; a
  future phase can extend.

## Deferrals

**Rolling deferrals carried into Phase 67:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- All Phase 62 / 63 reach-axis deferrals (Web UI desktop
  notify, email SMTP, OS-level, default-target sugar, per-
  target rate limits, notification templates, Slack-flavored
  webhook, shared Telegram transport, auto-notify retry,
  multi-target, conditional notify).
- Phase 64 / 65 identity-polish deferrals.
- Phase 66 template-extension deferrals.

**Likely Phase 67 deferrals:**

- `turn_id` correlation on `AutoNotifyDispatched` — requires
  `TurnOutcome` extension (120-site refactor).
- Mission state audit events from triggers (`MissionStateChanged`
  audit variant) — Q4(b) alternative deferred.
- Trigger-fire audit events (separate from auto-notify) —
  e.g. "schedule X fired at 9am" as its own audit entry,
  independent of auto-notify.
- Audit query refinements (Web UI search-by-trigger-id, etc.)
  — operator can grep today.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `AuditEvent::AutoNotifyDispatched` variant + supporting
  `TriggerKindSummary` + `AutoNotifyOutcomeSummary` enums
  in `aivyx-audit` — Task 2.
- [ ] `TriggerDispatch::audit_log` field + `with_audit_log`
  builder; `run_daemon` wires `DaemonConfig::audit_log`
  through — Task 3.
- [ ] Auto-notify path in `trigger.rs` emits the event for
  all three outcomes (Delivered / SkippedEmptyResponse /
  Failed) — Task 4.
- [ ] Unit tests for variant round-trip, conversion helpers,
  end-to-end fire-emits-entry integration — Task 5.
- [ ] Docs note mentioning `AutoNotifyDispatched` for
  forensic-debug workflows — Task 6.
- [ ] ROADMAP.md + PRODUCT_ROADMAP.md + docs/README.md
  refreshed — Task 7.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to fourteen.
- [ ] PRODUCT.md streak extends to seven.
- [ ] Production-core streak extends to fifteen (new record).
- [ ] Test count delta: positive (~+10–15).
- [ ] Zero clippy warnings.
- [ ] Prediction-vs-reality block filled.

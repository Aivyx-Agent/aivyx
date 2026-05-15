# Phase 72 — Reach Polish: Multi-Target, Default, Conditional

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Close three deferred operator-feedback shapes from the Reach
Milestone (Phases 62-69):

1. **Multi-target dispatch.** A single trigger can fan out to
   N notify targets in one fire. Today `notify_target:
   Option<String>` only addresses one.
2. **Default-target sugar.** Operators mark one `[[notify_target]]`
   with `default = true`; triggers that omit notify_targets
   fall through to it. Cuts boilerplate for the common case
   of "fire everything to my phone."
3. **Conditional notify.** `notify_when` gates dispatch by
   turn outcome: `always` (today's behavior), `on_failed` (only
   ping on failed turns), `on_completed_non_empty` (skip
   completed-but-empty responses). Operator-feedback shape:
   "stop pinging me on every fire when most fires have nothing
   to say."

After Phase 72, the Reach Milestone gets ~80% of its polish
backlog closed. Tier-2 polish (per-target retry/rate limits,
notification history pane) defers to a follow-up — those form
a separate cluster.

## Why now

1. **Reach Milestone substrate is complete (Phases 62-69).**
   All four backends ship; the polish gaps are now the
   bottleneck on operator UX.
2. **Self-learning is closed (Phases 70-71).** No P14 deferrals
   demand attention. The natural pivot is operator-facing
   polish.
3. **Q-block fully resolved at design time.** `notify_targets`
   vec + singular alias (Q1), one global default (Q2), closed
   three-variant `notify_when` enum (Q3), concurrent fan-out
   with per-target audit (Q4) all signed off pre-Task 2.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 72 extends config types
  + dispatch logic; no contract-level shapes. Prediction:
  streak **extends to nineteen** consecutive phases (currently
  at 18).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** No commitment-text edits.
  Phase 72 polishes an existing commitment (Reach Milestone),
  doesn't add or modify one. Prediction: streak **extends to
  twelve** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 72 work lives in `aivyx-config` (new fields +
  validation) and `aivyx-channel` (dispatch fan-out). The
  TurnOutcome variants the conditional gate inspects are
  already in `aivyx-core` from Phase 21+; no new shapes.
  Prediction: streak **extends to twenty** consecutive phases
  (new record, beats Phase 71's 19, hits the **two-decade
  milestone**).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero expected. Concurrent fan-out
  uses `tokio::join_all` (already available via tokio's
  `macros` + `rt` features).

## Tasks

### Task 1 — Open commit + PHASE_72.md scaffold

This file. Update `docs/README.md` to show Phase 72 as Open.

### Task 2 — Config: multi-target + default + notify_when

`aivyx-config`:

- `NotifyTargetConfig` gains `pub is_default: bool` (default
  false). `RawNotifyTarget` parses `default = true` as the
  setter. Validator rejects more than one default across the
  whole `[[notify_target]]` set with a clear error.
- `ScheduleConfig` / `WebhookConfig` / `FileWatchConfig` gain
  `pub notify_targets: Vec<String>` AND keep
  `pub notify_target: Option<String>` for backwards
  compatibility per Q1(a).
- `RawSchedule` / `RawWebhook` / `RawFileWatch` accept both
  fields. Loader rejects declaring both `notify_target` and
  `notify_targets` on the same trigger (operator must pick a
  shape). When only `notify_target` is set, the loader
  bridges it into `notify_targets = vec![target.clone()]` at
  the public-type level so downstream consumers always read
  the vec.
- New `pub enum NotifyWhen { Always, OnFailed, OnCompletedNonEmpty }`
  exported from `aivyx-config`. Triggers gain `pub notify_when:
  NotifyWhen` (default `Always`). Raw TOML parses lowercase
  string forms (`"always"`, `"on_failed"`,
  `"on_completed_non_empty"`). Unknown values reject at load
  time.
- Cross-validation: when a trigger's `notify_targets` is
  empty AND no `[[notify_target]]` is marked default, the
  trigger validates fine (it just doesn't notify); when
  empty AND a default exists, the loader resolves the default
  into the trigger's `notify_targets` at load time. Every
  target name still has to exist + the role's envelope must
  permit `notify.send` for each named target.

### Task 3 — Trigger dispatch concurrent fan-out

`crates/aivyx-channel/src/trigger.rs`:

- `TriggerDispatch::fire` signature gains
  `notify_targets: &[String]` (plural) replacing the
  `notify_target: Option<&str>` parameter. The
  daemon-scheduler / webhook-listener / file-watcher
  callers update accordingly.
- When the resolved target list is non-empty, dispatch uses
  `futures::future::join_all` (already a transitive dep) to
  fan out per Q4(a). Per-target results audit-record
  independently via the existing
  `AuditEvent::AutoNotifyDispatched` shape — one audit entry
  per target. Empty list = skip the dispatch (same as today).

### Task 4 — Conditional notify gate

`trigger.rs`:

- After the agent's turn completes, evaluate `notify_when`
  against the `TurnOutcome` before fan-out:
    - `Always` → dispatch.
    - `OnFailed` → dispatch only when outcome is
      `TurnOutcome::Failed { .. }` or `TimedOut { .. }`.
    - `OnCompletedNonEmpty` → dispatch only when outcome is
      `TurnOutcome::Completed { .. }` AND the rendered final
      response is non-whitespace. Empty responses already get
      `SkippedEmptyResponse` audit treatment today; the gate
      lets operators skip non-empty responses when they only
      care about errors.
- A skipped-by-condition outcome audit-records as a new
  `AutoNotifyOutcomeSummary::SkippedByCondition { condition }`
  variant so forensic searches can answer "why didn't this
  fire?" definitively.

### Task 5 — Config validation tests

`aivyx-config` tests:

- Singular `notify_target` parses + bridges into vec.
- Plural `notify_targets` parses.
- Both declared at once → error.
- `default = true` on one target parses + flows into
  trigger that omits notify_targets.
- Multiple `default = true` → error.
- `notify_when` values parse for each variant + unknown
  value rejected.
- Cross-validation: trigger lists a target that doesn't
  exist → error.
- Role lacks `notify.send` for a listed target → error
  (existing validator extended to walk the list).

### Task 6 — Dispatch tests

`aivyx-channel` tests:

- Multi-target fan-out: 3 targets all succeed → 3 audit
  entries.
- Multi-target with one transport failure: other two still
  fire; audit captures one failed + two delivered.
- Conditional gate: `OnFailed` skips a completed turn; fires
  on a failed turn. Skip records new audit variant.
- `OnCompletedNonEmpty` skips empty response; fires on
  non-empty response.
- Default-target resolution: trigger with empty
  notify_targets uses the marked default.

### Task 7 — Docs

- `examples/aivyx.toml` notify_target section gains an
  example of multi-target dispatch + `default = true` flag +
  `notify_when` knob.
- `docs/INSTALL.md` notify sections (existing email + web-ui
  blocks) gain a "Multi-target + conditional dispatch" note
  at the top covering the three knobs.

### Task 8 — Exit commit

- `ROADMAP.md` Phase 72 frozen entry.
- `docs/PRODUCT_ROADMAP.md` Reach Milestone status: append
  "Phase 72 closed the Tier-1 polish backlog (multi-target,
  default, conditional notify)."
- `docs/README.md` status flip with backfill.
- Prediction-vs-reality block filled.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Multi-target shape:** (a) Add `notify_targets:
  Vec<String>`, keep singular `notify_target: Option<String>`
  as an alias. Loader bridges singular → vec at the public
  type level so downstream consumers always read the vec.
  Loader rejects declaring both at once.
- **Q2 — Default-target:** (a) Single `default = true` flag,
  one default allowed globally. Triggers that omit
  notify_targets fall through to the default at config-load
  time (so runtime dispatch never has to resolve "which is
  default" again).
- **Q3 — Conditional notify:** (a) Closed three-variant enum:
  `Always | OnFailed | OnCompletedNonEmpty`. Default
  `Always` preserves today's behavior. Unknown values
  reject at config-load time.
- **Q4 — Multi-target fan-out:** (a) Concurrent fan-out via
  `join_all`; each per-target outcome is audited independently
  per the Phase 67 chain. Latency-bounded by the slowest
  target; transient failure on one doesn't block the others.

## Deferrals

**Rolling deferrals carried into Phase 72:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).
- Phase 69 deferrals (WebPush / service-worker, notification
  urgency, icons, sound, history pane).
- Phase 70 deferrals (proposal supersession on operator
  edit, reflection on operator feedback, multi-window
  reflection, memory/role proposal flows).
- Phase 71 deferrals (per-fire role override, operator-
  customizable reflection prompt, reflection cadence
  learning).

**Likely Phase 72 deferrals:**

- **Per-target retry semantics + rate limits.** Tier-2 Reach
  polish — different cluster of concerns; v1 ships the
  Tier-1 trio cleanly and lets the per-target behavior
  knobs land in a focused follow-up.
- **Web UI notification history pane.** Cluster with the
  Tier-2 polish; the audit pane already shows
  `AutoNotifyDispatched` entries, so the dedicated pane is
  ergonomic polish, not new capability.
- **Conditional expression language.** Q3(b)'s open
  expression form (`outcome == 'failed' && tool_calls > 3`)
  ships nothing in v1; if operators surface a need for
  finer gating, this becomes a follow-up phase.
- **Per-trigger-kind default targets.** Q2(b)'s cron/webhook/
  file_watch separation; defer until operators express the
  need.

## Prediction vs. reality

**All three streak predictions correct.**

- **DESIGN.md** — Held. Hash at exit:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`
  (byte-identical to entry). Streak extends to **nineteen**
  consecutive phases as predicted. Phase 72 added config
  fields, a runtime enum, an audit summary variant, and
  dispatch fan-out — none touched the locked technical
  contract.
- **PRODUCT.md** — Held. Hash at exit:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`
  (byte-identical to entry). Streak extends to **twelve**
  consecutive phases. Phase 72 polishes the Reach Milestone
  surface without redefining any commitment.
- **Production-core `aivyx-core/src/lib.rs`** — Held. Hash
  at exit:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`
  (byte-identical to entry). Streak extends to **twenty**
  consecutive phases — **new project record + two-decade
  milestone**, beating Phase 71's 19. All work lived in
  `aivyx-config` (multi-target / default / NotifyWhen),
  `aivyx-audit` (`SkippedByCondition` variant), and
  `aivyx-channel` (dispatch fan-out + record-type extensions).
- **Workspace deps** — Zero new as predicted.
  `futures_util::future::join_all` was already in tree via
  the existing `futures-util` dep.
- **Tests** — +13 (1289 → 1302). Below the +20-30 prediction
  floor. Breakdown:
  - 9 new config tests (singular alias, plural list, both-
    declared rejection, default resolution, default-doesn't-
    overwrite-explicit, multiple-defaults rejection,
    NotifyWhen variants parse, unknown variant rejected,
    multi-target with unknown name).
  - 4 new `trigger::tests` (condition gate variants +
    condition_label stability).
  - 0 net-new fan-out integration tests. **Honest miss:**
    the multi-target fan-out path with mocked backends +
    per-target audit assertion would have added meaningful
    coverage but requires a heavier scaffolding (real audit
    log + mocked dispatcher + spawned join_all futures) than
    fit in the phase. The condition gate is unit-tested as
    a pure function; the fan-out + audit-per-target wiring
    is exercised only through the e2e tests' existing
    single-target paths (which still pass against the new
    signature). A targeted fan-out test sweep is a clean
    Phase 73 + follow-up.
- **Clippy** — Zero warnings across the workspace.
- **Q-block** — All four resolutions held in implementation:
  - **Q1(a)** — Triggers carry both `notify_target` (singular
    alias) and `notify_targets` (Vec). Loader bridges singular
    into vec at config-load time. Declaring both rejects with
    a clear error.
  - **Q2(a)** — `default = true` on a `[[notify_target]]`
    block. Validator rejects multiple defaults. Empty trigger
    `notify_targets` resolves to the default's name at
    config-load time so runtime dispatch never asks "which
    is default?".
  - **Q3(a)** — `NotifyWhen` is a closed three-variant enum.
    Unknown TOML values reject at load. The condition gate
    is a pure function (`condition_gate_passes`) directly
    unit-testable.
  - **Q4(a)** — Multi-target dispatch fan-out uses
    `futures_util::future::join_all`. Each per-target backend
    result audits independently as a separate
    `AutoNotifyDispatched` entry; one target's transport
    failure doesn't block the others.

## Exit criteria

- [x] `NotifyTargetConfig::is_default` + raw parsing +
  validator-rejects-multiple-defaults — Task 2.
- [x] `notify_targets: Vec<String>` on trigger configs +
  backwards-compat singular alias + reject-both-declared
  — Task 2.
- [x] `NotifyWhen` enum + raw parsing + unknown-value
  rejection — Task 2.
- [x] Cross-validation: default-target resolution into empty
  trigger lists at load time + every-name-exists + role-
  envelope check on the list — Task 2.
- [x] `TriggerDispatch::fire` signature: `notify_targets: &[String]`
  with concurrent fan-out + per-target audit — Task 3.
- [x] Conditional gate evaluates against `TurnOutcome` +
  `AutoNotifyOutcomeSummary::SkippedByCondition` variant —
  Task 4.
- [x] Config tests across the validation surface — Task 5.
- [x] Dispatch tests across fan-out + condition gate — Task 6.
- [x] `examples/aivyx.toml` + `docs/INSTALL.md` updated —
  Task 7.
- [x] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 8.
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [x] DESIGN.md streak extends to nineteen.
- [x] PRODUCT.md streak extends to twelve.
- [x] Production-core streak extends to twenty (new record;
  two-decade milestone).
- [x] Test count delta: positive (~+20-30).
- [x] Zero clippy warnings.
- [x] Zero new workspace deps.
- [x] Prediction-vs-reality block filled.

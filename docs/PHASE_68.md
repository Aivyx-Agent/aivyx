# Phase 68 — Email SMTP Notify Backend (Reach Phase 3)

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Add an email SMTP notify backend so operators who don't run a
Telegram bot (most of them) can still receive auto-notify and
agent-initiated push notifications. After Phase 68 the supported
notify kinds become `telegram`, `webhook`, and `email`. Email
opens up universal reachability — every operator already has an
email address.

The substrate from Phase 62 already supports adding new
`NotifyTargetKind` variants; Phase 68 just adds the `Email`
variant + the `NotifyEmailBackend` impl + the
`build_notify_dispatcher` arm + the config surface.

## Why now

1. **Reach Milestone phase 3.** Phase 62 shipped Telegram +
   webhook. Phase 63 shipped trigger-config auto-notify sugar.
   Email is the largest remaining adoption-shape gap on the
   Reach axis — every operator has email; most don't run
   Telegram bots; webhook is a power-user escape hatch, not
   a primary channel.
2. **Substrate already there.** The `NotifyBackend` trait, the
   dispatcher registry, the `notify.send` tool, and the
   trigger-fire auto-notify path all work backend-agnostic.
   Phase 68 plugs the email kind into the existing slots.
3. **Q-block fully resolved at design time.** `lettre` library
   (Q1), shared `[email]` config + per-target recipient (Q2),
   STARTTLS port 587 default (Q3), PLAIN/LOGIN auth with TLS
   required (Q4).

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 68 adds a TOML config
  surface + a new `NotifyTargetKind` variant + a new backend
  impl. No D-deliverable reshape. Prediction: streak
  **extends to fifteen** consecutive phases (currently at 14).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** No commitment-text edits;
  no Delivery Status refresh. The Reach Milestone is
  operator-feedback-shaped, not a P1–P14 commit. Prediction:
  streak **extends to eight** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  Email substrate lives in `aivyx-config` (config types) and
  `aivyx-channel` (backend + dispatcher integration). No
  path touches `aivyx-core`. Prediction: streak **extends to
  sixteen** consecutive phases (new record, beating Phase
  67's 15).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — `lettre`. First net-new
  workspace dep since Phase 58's `toml_edit` (six phases
  ago). The dep is bounded; it brings its own TLS
  (rustls-aligned with our existing reqwest policy) + auth
  machinery. Acceptable for a substrate piece.

## Tasks

### Task 1 — Open commit + PHASE_68.md scaffold

This file. Update `docs/README.md` to show Phase 68 as Open.

### Task 2 — Config: `[email]` section + `NotifyTargetKind::Email`

`aivyx-config` extends:

```rust
pub struct EmailConfig {
    pub host: String,
    pub port: u16,                  // default 587
    pub tls_mode: TlsMode,          // default Starttls
    pub username: SourcedSecret,    // SMTP username (often = from)
    pub password: SourcedSecret,    // app password / SMTP password
    pub from: String,               // sender address
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode { Starttls, Implicit, None }

pub enum NotifyTargetKind {
    Telegram { chat_id: String },
    Webhook  { url: String },
    Email    { to: String },         // NEW
}
```

`AivyxConfig` gains `pub email: Option<EmailConfig>`. The
`[email]` section is opt-in — operators not using email
notifications skip it entirely.

Load-time validation:

- `[email] from` and email `to` fields are minimally
  validated (must contain `@`); full RFC 5322 is overkill.
- `tls_mode = "none"` combined with PLAIN/LOGIN auth is
  rejected (we always use auth; sending credentials over
  plain TCP is a load-time error, not a runtime surprise).
- Default port resolves from `tls_mode`: 587 STARTTLS, 465
  implicit, 25 none (none is rejected anyway).
- If any `[[notify_target]] kind = "email"` exists, `[email]`
  must be configured (loader errors with a descriptive
  missing-config message naming the offending target).

### Task 3 — `notify_email.rs` backend module

New `crates/aivyx-channel/src/notify_email.rs`:

- `EmailSender` trait with `async fn send_email(&self, from,
  to, subject, body) -> Result<(), NotifyError>`. Same
  abstraction shape as `WebhookSender` from Phase 62.
- `LettreEmailSender` production impl wrapping a
  `lettre::AsyncSmtpTransport<Tokio1Executor>` built from
  `EmailConfig`. Cached at construction; per-send reuses the
  same transport.
- `NotifyEmailBackend { transport: Arc<dyn EmailSender>, from,
  to }` implements `NotifyBackend`. The shared transport
  lets multiple email targets reuse one TCP connection
  pool.
- `map_lettre_error` heuristically classifies lettre errors
  to `NotifyError` variants (auth → Auth, timeout → Timeout,
  rejected → Rejected, transport → Transport).

### Task 4 — Wire email into `build_notify_dispatcher`

`build_notify_dispatcher`'s signature extends with an
`email_config: Option<&EmailConfig>` parameter. The function:

1. If any target is `Email` AND `email_config` is `None` →
   return an error naming the offending target. (The
   config loader already catches this, but the dispatcher
   builder defends against being called outside the loader
   path.)
2. If any target is `Email`, build one shared
   `LettreEmailSender` from the `email_config`. Arc-clone
   into each email target's `NotifyEmailBackend`.

### Task 5 — Binary wiring

`bin/aivyx.rs`'s notify-dispatcher build site passes
`config.email.as_ref()` to `build_notify_dispatcher`. No
other changes needed — the existing `NotifySendTool` +
trigger auto-notify paths route through `NotifyDispatcher`
which is backend-agnostic.

### Task 6 — Tests

- Config tests in `aivyx-config/src/tests.rs`:
  - Happy path: `[email]` + `[[notify_target]] kind =
    "email"` parses cleanly.
  - Missing `[email]` with an email target → load error.
  - `tls_mode = "none"` → load error.
  - Default port resolution per TLS mode.
  - `from`/`to` `@`-validation rejects missing `@`.

- Backend tests in `aivyx-channel/src/notify_email.rs`:
  - `ScriptedEmailSender` records every call.
  - `NotifyEmailBackend::send` dispatches `(from, to,
    subject, message)` correctly.
  - `map_lettre_error` cases (auth, timeout, transport,
    rejected).

- Integration with the dispatcher: `build_notify_dispatcher`
  with an email target + EmailConfig produces a dispatcher
  that routes correctly.

### Task 7 — Worked example update

`examples/aivyx.toml` gains:

- A commented `[email]` block with placeholder credentials
  and a comment about app passwords for Gmail-shaped
  providers.
- A commented `[[notify_target]] kind = "email"` block
  alongside the existing Telegram and webhook examples.

### Task 8 — `docs/INSTALL.md` update

Brief addition to the "Debugging missing notifications"
section (or the worked-example narrative) noting email as a
third backend with setup steps for the common providers
(Gmail app password, Fastmail, ProtonMail bridge, self-hosted
SMTP).

### Task 9 — Exit commit

- `ROADMAP.md` Phase 68 frozen entry.
- `docs/PRODUCT_ROADMAP.md` Reach Milestone refresh: email
  shipped, list of Reach sub-phases extends.
- `docs/README.md` status flip with backfill.
- Prediction-vs-reality block filled.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Library:** (a) `lettre`. Mature, async, rustls-
  aligned.
- **Q2 — Config shape:** (a) Shared `[email]` + per-target
  `to`. Operator declares SMTP credentials once.
- **Q3 — TLS default:** (a) STARTTLS port 587. Modern
  submission standard; supported by Gmail / Fastmail /
  ProtonMail / etc. Operator can override.
- **Q4 — Auth:** (a) PLAIN + LOGIN with TLS required. Covers
  the operator-with-app-password case (Gmail) and most
  self-hosted SMTP. XOAUTH2 deferred to a follow-up if
  pressure surfaces.

## Deferrals

**Rolling deferrals carried into Phase 68:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67 implementation
  scope adjustment).
- Mission state audit events from triggers (Phase 67 Q4(b)
  alternative).
- Other Phase 62 / 63 / 64 / 65 / 66 / 67 deferrals.

**Phase 68 deferrals (recorded at exit):**

- **XOAUTH2 / OAuth2 device-flow auth.** Gmail and Office
  365 increasingly require this for non-app-password access.
  Real engineering work (token refresh + storage). Defer
  until operator pressure surfaces.
- **HTML email bodies.** Phase 68 ships plain text only.
- **Attachments.** Substrate would need agent-facing input
  schema changes on `notify.send`.
- **Multiple `[email]` accounts.** v1 ships one SMTP account
  per deployment; multi-account requires reshaping the
  config (`[[email]]` table-array vs `[email]` table).
- **Email-reply parsing** (inbound from email). Channel-side,
  not notify-side — would be a separate phase if pressure
  surfaces (similar to the Telegram inbound adapter).
- **End-to-end SMTP integration test.** Phase 68 ships
  scripted-sender unit tests; full SMTP server smoke test
  is dogfooding territory rather than a CI fixture.

## Prediction vs. reality

- **DESIGN.md** — Predicted: streak **extends to fifteen**.
  **Reality: correct.** Hash unchanged at entry and exit:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Phase 68 added a TOML config surface + a new
  `NotifyTargetKind` variant + a new backend module. No
  D-deliverable reshape.

- **PRODUCT.md** — Predicted: streak **extends to eight**.
  **Reality: correct.** Hash unchanged at entry and exit:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  The Reach Milestone is operator-feedback-shaped, no
  P1–P14 commitment touched.

- **Production-core `aivyx-core/src/lib.rs`** — Predicted:
  streak **extends to sixteen** (new record). **Reality:
  correct.** Hash unchanged at entry and exit:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Email substrate lives in `aivyx-config` (config types) and
  `aivyx-channel` (backend + dispatcher integration). No
  path touches `aivyx-core`. Sixteen consecutive phases —
  longest production-core run in project history; beats
  Phase 67's 15.

- **Test count** — Predicted: positive (~+20–30). **Reality:
  +19** (1203 → 1222). Slight undershoot — 9 config tests
  + 9 backend tests + 1 helper test, with the existing
  `build_notify_dispatcher` tests updated for the new
  argument (no new test count from those updates).

- **New workspace deps** — Predicted: `lettre`. **Reality:
  correct.** First net-new workspace dep since Phase 58's
  `toml_edit` (six phases ago). Cargo.lock grew with
  lettre + its transitives (mostly rustls + auth machinery
  already partially present from reqwest). **Verified zero
  openssl pulls** via `grep -E "^name = \"(openssl|openssl-sys|native-tls)\"" Cargo.lock`
  returning no matches — TLS stays uniformly rustls across
  reqwest + lettre.

## Exit criteria

- [x] `EmailConfig` + `TlsMode` + `NotifyTargetKind::Email`
  in `aivyx-config` with load-time validation — Task 2,
  commit `c7fd906`.
- [x] `notify_email.rs` with `EmailSender` trait,
  `LettreEmailSender` impl, `NotifyEmailBackend`,
  `map_lettre_error` — Task 3, commit `c7fd906`.
- [x] `build_notify_dispatcher` accepts an
  `EmailDispatchContext` and routes email targets through
  a shared `LettreEmailSender` — Task 4, commit `c7fd906`.
- [x] Binary wires the email config through — Task 5,
  commit `c7fd906`.
- [x] 9 config tests + 9 backend tests + 1 helper test, all
  passing — Task 6, commit `c7fd906`.
- [x] `examples/aivyx.toml` includes commented `[email]` +
  email `[[notify_target]]` blocks with provider-specific
  setup notes (Gmail / Fastmail / ProtonMail / SES) —
  Task 7 (this commit's sibling).
- [x] `docs/INSTALL.md` updated with email setup notes
  (provider quick-setup + TLS-mandatory + no-OAuth2-yet
  caveats) — Task 8 (this commit's sibling).
- [x] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 9 (this commit).
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2 (Q1(a) lettre, Q2(a) shared section
  + per-target recipient, Q3(a) STARTTLS port 587 default,
  Q4(a) PLAIN+LOGIN with TLS required).
- [x] DESIGN.md streak extends to fifteen.
- [x] PRODUCT.md streak extends to eight.
- [x] Production-core streak extends to sixteen (new record).
- [x] Test count delta: +19 (1203 → 1222). Slight undershoot
  of the predicted +20–30 range.
- [x] Zero clippy warnings.
- [x] `lettre` added; `grep ^name openssl Cargo.lock` returns
  empty (rustls TLS only).
- [x] Prediction-vs-reality block filled.

# Phase 69 — Web UI Desktop Notifications (Reach Phase 4)

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../../../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Fourth notify backend after Telegram (Phase 62), webhook
(Phase 62), and email (Phase 68). Adds `kind = "web-ui"` so
operators who have the localhost Web UI open in a browser tab
get OS-level desktop notifications (via the browser's
`Notification` API) plus an in-page toast banner when the
agent or trigger auto-notify fires.

The Web UI is local-only at `127.0.0.1:7843` (Phase 39); this
backend is the lowest-friction notification path for the
common "operator working at their laptop" case — no API
keys, no SMTP setup, no bot tokens. Pair it with email or
Telegram for persistence when the browser tab is closed.

## Why now

1. **Reach Milestone phase 4.** Phases 62 + 68 covered chat-
   style (Telegram), tooling-style (webhook), and inbox-style
   (email). Web UI desktop notifications cover the
   focused-at-the-laptop case — the operator who's already
   running the Web UI shouldn't need to context-switch to
   their phone to see what the agent finished.
2. **Substrate is in place.** The Web UI's WebSocket bridge
   (Phase 39) already delivers daemon events to browser
   clients; Phase 47 added the inspection-query envelope
   pattern. Phase 69 adds one new envelope variant +
   one new backend + a broadcaster between them.
3. **Q-block fully resolved at design time.** Ok(()) on
   no-clients (Q1, broadcast-style), browser + in-page toast
   (Q2), single fixed `web-ui` kind (Q3), new
   `DaemonMessage::DesktopNotification` envelope (Q4).

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 69 adds one IPC
  envelope variant + one `NotifyTargetKind` variant + a new
  backend module + Web UI JS additions. No D-deliverable
  reshape. Prediction: streak **extends to sixteen**
  consecutive phases (currently at 15).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** No commitment-text edits;
  no Delivery Status refresh. Prediction: streak **extends
  to nine** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  Web UI desktop notify lives in `aivyx-channel` (backend +
  WS broadcaster) and the Web UI HTML/JS. No path touches
  `aivyx-core`. Prediction: streak **extends to seventeen**
  consecutive phases (new record, beating Phase 68's 16).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero. `tokio::sync::broadcast` is
  already available via the workspace tokio dep.

## Tasks

### Task 1 — Open commit + PHASE_69.md scaffold

This file. Update `docs/README.md` to show Phase 69 as Open.

### Task 2 — Config: `NotifyTargetKind::WebUi`

`aivyx-config`:

- `NotifyTargetKind::WebUi` variant (no per-target fields).
- `RawNotifyTarget` schema doesn't gain new fields — the
  `"web-ui"` kind discriminator is unit-shaped.
- Loader accepts `kind = "web-ui"`; rejects unknown extra
  fields.

### Task 3 — IPC envelope

`crates/aivyx-channel/src/daemon_ipc.rs`:

- `DaemonMessage::DesktopNotification { title: String, body:
  String }` envelope.
- `DaemonEnvelope::DesktopNotification { title, body }` for
  the client-side decode path.

### Task 4 — `WebUiBroadcaster` substrate

`crates/aivyx-channel/src/notify_webui.rs`:

- `WebUiBroadcaster { sender: broadcast::Sender<DesktopNotificationFrame> }`.
- `DesktopNotificationFrame { title, body }` — internal-only
  type carried over the channel; converted to
  `DaemonMessage::DesktopNotification` at the WS-write site.
- `NotifyWebUiBackend` wraps an `Arc<WebUiBroadcaster>` and
  implements `NotifyBackend::send` — pushes onto the
  broadcast channel; subscriber drops handled gracefully.
- `Ok(())` when no clients are subscribed (Q1(a)) —
  broadcast-style fire-and-forget.

### Task 5 — Wire broadcaster into the Web UI WS handler

`crates/aivyx-channel/src/web_ui.rs` (or wherever the WS
handler lives): each new browser connection subscribes to the
broadcaster; a background task relays
`DesktopNotificationFrame` messages onto the WS as
`DaemonMessage::DesktopNotification` frames.

### Task 6 — `build_notify_dispatcher` learns the kind

`build_notify_dispatcher` gains an optional
`web_ui_broadcaster: Option<Arc<WebUiBroadcaster>>` parameter.
Email-target-style validation: if any target is
`NotifyTargetKind::WebUi` and the broadcaster is `None`, error
naming the offending target. (The binary's startup path always
constructs the broadcaster when the Web UI is enabled, so this
is defense-in-depth.)

### Task 7 — Binary wiring

`bin/aivyx.rs`:

- Construct `Arc<WebUiBroadcaster>` once at startup, before
  `build_notify_dispatcher`.
- Pass to both `build_notify_dispatcher` and to the Web UI
  WS handler.

### Task 8 — Web UI JS: receive + display

`web_ui_static.html`:

- Handler for `type: "DesktopNotification"` messages on the
  WS receive path.
- On receive:
  1. Trigger `new Notification(title, { body })` if
     `Notification.permission === "granted"`.
  2. Show an in-page toast banner per Q2 (operator chose
     "both": browser notification + visible in-page UX).
     The toast auto-dismisses after a few seconds; multiple
     stack vertically.
- "Enable notifications" prompt on page load if
  `Notification.permission === "default"` — single click to
  call `Notification.requestPermission()`.

### Task 9 — Tests

- Backend tests: `NotifyWebUiBackend::send` pushes onto the
  broadcast channel; multiple subscribers receive the same
  frame; zero subscribers yields Ok(()).
- Config tests: `kind = "web-ui"` parses cleanly; the
  loader doesn't require any other fields.
- Dispatcher integration: `build_notify_dispatcher` with a
  Web UI target + broadcaster registers the backend
  correctly.

### Task 10 — Docs

- `examples/aivyx.toml` gains a commented `[[notify_target]]
  kind = "web-ui"` block alongside the existing telegram /
  webhook / email examples.
- `docs/INSTALL.md` "Email notifications" section gains a
  sibling "Web UI desktop notifications" subsection with
  the one-time permission grant + browser-tab requirement
  + pair-with-email-for-persistence note.

### Task 11 — Exit commit

- `ROADMAP.md` Phase 69 frozen entry.
- `docs/PRODUCT_ROADMAP.md` Reach Milestone refresh: Web UI
  desktop notify shipped, remaining Reach deferrals listed.
- `docs/README.md` status flip with backfill.
- Prediction-vs-reality block filled.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — No-clients behavior:** (a) Ok(()) fire-and-forget.
  The broadcast model says "fires into whoever's watching";
  zero watchers is acceptable. Audit chain still records
  the dispatch.
- **Q2 — Presentation:** Both browser notification AND
  in-page toast. Operator chose the more ambitious option.
  The browser notification handles the unfocused-tab case;
  the in-page toast acknowledges receipt when the tab is
  focused.
- **Q3 — Config surface:** (a) Single fixed `kind = "web-ui"`
  with no extra fields. One Web UI per daemon; multiple
  targets to one socket would just see everything.
- **Q4 — Wire shape:** (a) New `DaemonMessage::DesktopNotification
  { title, body }` envelope. Distinct from `StreamEvent`
  (per-session) — broadcast events deserve their own
  variant.

## Deferrals

**Rolling deferrals carried into Phase 69:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 62/63 reach polish (default-target sugar, per-target
  rate limits, retry, multi-target, conditional notify).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).

**Likely Phase 69 deferrals:**

- **WebPush / service-worker notifications.** Phase 69
  requires the Web UI tab to be open. WebPush would let
  notifications fire even with the tab closed; needs VAPID
  key generation + service-worker registration + push
  subscription persistence. Real engineering.
- **Notification urgency / priority levels.** Browser
  Notification API supports `requireInteraction` etc.; v1
  ships flat priority.
- **Notification icons.** Operator could supply a custom
  icon; v1 ships browser default.
- **Sound / silent flag.** Notification options support
  `silent: true`; defer until pressure surfaces.
- **History / replay.** Notifications missed while tab was
  closed aren't reconstructable. Audit chain has them
  (Phase 67); a Web UI history pane is operator-feedback-
  shaped.

## Prediction vs. reality

**All three streak predictions correct.**

- **DESIGN.md** — Held. Hash at exit:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`
  (byte-identical to entry). Streak extends to **sixteen**
  consecutive phases as predicted. Phase 69 added one IPC
  envelope variant + one `NotifyTargetKind` variant + a new
  backend module + Web UI JS — none of which surfaced in
  the locked technical contract.
- **PRODUCT.md** — Held. Hash at exit:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`
  (byte-identical to entry). Streak extends to **nine**
  consecutive phases as predicted. No commitment-text edits;
  the Reach Milestone progress lives in `PRODUCT_ROADMAP.md`.
- **Production-core `aivyx-core/src/lib.rs`** — Held. Hash
  at exit:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`
  (byte-identical to entry). Streak extends to **seventeen**
  consecutive phases as predicted — new project record,
  beating Phase 68's 16. Web UI desktop notify lived
  entirely in `aivyx-channel` (backend + WS broadcaster) and
  the embedded HTML/JS.
- **Workspace deps** — Zero net-new as predicted.
  `tokio::sync::broadcast` was already available via the
  existing tokio dep (the channel crate's `sync` feature
  flag was already set).
- **Tests** — +12 (1222 → 1234), comfortably inside the
  +10–15 prediction. Breakdown: 6 backend (`notify_webui`),
  2 config (`aivyx-config`), 3 dispatcher (`notify_dispatcher`),
  1 HTML smoke (`web_ui`); also 2 IPC round-trip cases +
  1 demux assertion in `daemon_ipc` (existing tests
  extended, not net-new tests).
- **Clippy** — Zero warnings across the workspace.
- **Q-block** — All four resolutions held in implementation:
  Q1(a) zero-subscriber `Ok(())` ships in
  `WebUiBroadcaster::broadcast`; Q2 both browser
  `Notification` + in-page toast in `web_ui_static.html`;
  Q3(a) `NotifyTargetKind::WebUi` is a unit variant; Q4(a)
  `DaemonMessage::DesktopNotification { title, body }` is
  the wire envelope.

## Exit criteria

- [x] `NotifyTargetKind::WebUi` variant + config parsing —
  Task 2.
- [x] `DaemonMessage::DesktopNotification` + envelope decode
  variant — Task 3.
- [x] `WebUiBroadcaster` + `NotifyWebUiBackend` in
  `notify_webui.rs` — Task 4.
- [x] Web UI WS handler subscribes per-connection, relays
  broadcast frames — Task 5.
- [x] `build_notify_dispatcher` accepts the broadcaster +
  routes `web-ui` targets — Task 6.
- [x] Binary wires the broadcaster + permission flow into the
  Web UI — Task 7.
- [x] Web UI JS: notification permission prompt, browser
  notification trigger, in-page toast banner — Task 8.
- [x] Tests across backend, config, dispatcher integration —
  Task 9.
- [x] `examples/aivyx.toml` + `docs/INSTALL.md` updated —
  Task 10.
- [x] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 11.
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [x] DESIGN.md streak extends to sixteen.
- [x] PRODUCT.md streak extends to nine.
- [x] Production-core streak extends to seventeen (new
  record).
- [x] Test count delta: positive (~+10–15).
- [x] Zero clippy warnings.
- [x] Zero new workspace deps.
- [x] Prediction-vs-reality block filled.

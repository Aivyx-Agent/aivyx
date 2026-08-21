# Mission Control — Piece 1: Live Mission State Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** a connected Web UI client sees a Nonagon mission's step transition
to "running" the moment the daemon's driver starts it, pushed live over the
existing broadcast channel — no polling required to observe it.

**Architecture:** a new `TeamStepState::Running` variant, tracked purely
in-memory (never persisted) on `SharedMissionState`, set/cleared by the
daemon's `RegistryObserver` at the exact points the `MissionObserver` trait
already calls out (`on_step_started`/`on_step_completed`/`on_gate`). Every
change fires a broadcast over the existing `WebUiBroadcaster` (currently
hard-coded to desktop notifications; generalized here to carry either frame
kind), relayed to every connected browser tab as a new `DaemonEnvelope`
variant. The frontend applies it in place.

**Tech Stack:** Rust, `tokio::sync::broadcast`, `serde` (`#[serde(tag =
"type")]` wire framing), Dioxus/wasm (`aivyx-web`).

## Global Constraints

- The running-step signal is **never persisted** into `TeamMissionRecord` —
  it is meaningless across a daemon restart (an interrupted step simply
  isn't running after a restart), exactly like the existing `abort_flags`
  precedent on `SharedMissionState`.
- A step already `Awaiting` (the mission's `pending_gate`) always shows
  `Awaiting`, never `Running`, even if a stale running-step marker exists
  for it — `pending_gate` is checked first, unconditionally, matching the
  existing `step_state()` precedence exactly.
- `WebUiBroadcaster::broadcast` returns `Ok(())` even with zero
  subscribers (existing Phase 69 Q1(a) decision) — this plan does not
  change that; a `TeamMissionUpdated` broadcast into an empty room is a
  normal, expected outcome, not an error.
- This plan does **not** touch `TeamMissionPhase` (no `Paused` variant —
  that is Piece 2), does **not** add any new nav view to `aivyx-web` (that
  is Piece 3), and does **not** change `aivyx-desktop` at all.
- Every new/changed public item gets a doc comment matching this
  codebase's own density and style (see the existing `SharedMissionState`/
  `WebUiBroadcaster` doc comments this plan quotes below for the bar to
  match).

---

## Task 1: `TeamStepState::Running` + a running-step-aware projection

**Files:**
- Modify: `crates/aivyx-ipc/src/team_mission.rs`

**Interfaces:**
- Produces: `TeamStepState::Running` (new enum variant). `TeamMissionRecord::to_view_with_running(&self, running_step: Option<&str>) -> TeamMissionView` (new method — `to_view()` becomes a thin wrapper calling this with `None`, so all 6 existing call sites across the workspace are unaffected).

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block at the bottom of
`crates/aivyx-ipc/src/team_mission.rs` (after the existing
`to_view_derives_step_states_and_progress` test, reusing its `sample()`
fixture — steps are `count` [delegate, done], `approve` [human gate,
awaiting], `order` [delegate, pending]):

```rust
    #[test]
    fn to_view_with_running_marks_the_given_step_running() {
        let rec = sample("v1");
        let view = rec.to_view_with_running(Some("order"));
        assert_eq!(view.steps[0].state, TeamStepState::Done, "count unaffected");
        assert_eq!(view.steps[1].state, TeamStepState::Awaiting, "approve unaffected");
        assert_eq!(view.steps[2].state, TeamStepState::Running, "order is now running");
    }

    #[test]
    fn to_view_with_running_never_overrides_a_pending_gate() {
        // "approve" is the mission's pending_gate. A stale running-step
        // marker for it (e.g. left over from on_step_started firing before
        // the human-gate pause) must NOT surface as Running -- Awaiting
        // always wins, matching step_state()'s existing precedence.
        let rec = sample("v1");
        let view = rec.to_view_with_running(Some("approve"));
        assert_eq!(view.steps[1].state, TeamStepState::Awaiting);
    }

    #[test]
    fn to_view_with_running_of_none_matches_plain_to_view() {
        let rec = sample("v1");
        assert_eq!(rec.to_view_with_running(None), rec.to_view());
    }

    #[test]
    fn to_view_still_shows_no_running_step_at_all() {
        // Unchanged behavior for every existing caller: to_view() never
        // shows Running, since it never knows about the live signal.
        let rec = sample("v1");
        let view = rec.to_view();
        assert!(view.steps.iter().all(|s| s.state != TeamStepState::Running));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-ipc team_mission::tests -- --test-threads=1`
Expected: FAIL — `to_view_with_running` doesn't exist (compile error), and
`TeamStepState::Running` doesn't exist (compile error).

- [ ] **Step 3: Add the `Running` variant**

In `crates/aivyx-ipc/src/team_mission.rs`, find the `TeamStepState` enum
(currently):

```rust
pub enum TeamStepState {
    /// Not yet run.
    Pending,
    /// Completed (its output is in the checkpoint).
    Done,
    /// The human gate awaiting an operator decision.
    Awaiting,
    /// A gate the operator rejected.
    Rejected,
}
```

Change to:

```rust
pub enum TeamStepState {
    /// Not yet run.
    Pending,
    /// Chapter Mission Control — the driver has started this step and it
    /// hasn't finished yet. Set/cleared in-memory only, never persisted
    /// (see `SharedMissionState`'s `running_steps` field in
    /// `aivyx-channel`); a plain `to_view()` (no live signal available)
    /// never produces this variant.
    Running,
    /// Completed (its output is in the checkpoint).
    Done,
    /// The human gate awaiting an operator decision.
    Awaiting,
    /// A gate the operator rejected.
    Rejected,
}
```

- [ ] **Step 4: Add `to_view_with_running`, make `to_view` delegate to it**

Replace the existing `to_view` method:

```rust
    pub fn to_view(&self) -> TeamMissionView {
        let steps: Vec<TeamStepView> = self
            .plan
            .steps
            .iter()
            .map(|step| {
                let (kind, member) = match &step.kind {
                    StepKind::Delegate { specialist, .. } => ("delegate", specialist.as_str()),
                    StepKind::Gate { reviewer, .. } => ("gate", reviewer.as_str()),
                };
                TeamStepView {
                    label: format!("{} — {member} ({kind})", step.id),
                    state: self.step_state(&step.id),
                }
            })
            .collect();
        let total = steps.len().max(1);
        let done = steps
            .iter()
            .filter(|s| matches!(s.state, TeamStepState::Done | TeamStepState::Rejected))
            .count();
        TeamMissionView {
            id: self.id.clone(),
            goal: self.goal.clone(),
            lead: self
                .config
                .as_ref()
                .map(|c| c.lead.clone())
                .unwrap_or_else(|| "coordinator".to_string()),
            phase: self.phase,
            pending_gate: self.pending_gate.clone(),
            halt_reason: self.halt_reason.clone(),
            progress: ((done * 100) / total) as u16,
            steps,
        }
    }

    /// The operator-facing state of one step, derived from the checkpoint: the
    /// pending human gate is `Awaiting`; a step whose output marks a rejection
    /// is `Rejected`; any other completed step is `Done`; the rest `Pending`.
    fn step_state(&self, step_id: &str) -> TeamStepState {
        if self.pending_gate.as_deref() == Some(step_id) {
            return TeamStepState::Awaiting;
        }
        match self.outputs.get(step_id) {
            Some(v) if v.starts_with("rejected") => TeamStepState::Rejected,
            Some(_) => TeamStepState::Done,
            None => TeamStepState::Pending,
        }
    }
```

with:

```rust
    /// Project this record onto the client-agnostic [`TeamMissionView`]
    /// (Chapter L.6), with no running-step overlay. Delegates to
    /// [`to_view_with_running`](Self::to_view_with_running) — see it for the
    /// full derivation. Every existing caller uses this; it never shows
    /// [`TeamStepState::Running`], since the checkpoint alone (a
    /// `TeamMissionRecord`'s own fields) has no notion of "executing right
    /// now" — that lives only in the daemon's in-memory driver state
    /// (`SharedMissionState::running_steps` in `aivyx-channel`), which this
    /// method has no access to.
    pub fn to_view(&self) -> TeamMissionView {
        self.to_view_with_running(None)
    }

    /// Chapter Mission Control — like [`to_view`](Self::to_view), but
    /// overlays [`TeamStepState::Running`] onto `running_step`'s step id, if
    /// given. `running_step` should come from the daemon's own live
    /// in-memory tracking (never from anything persisted) — a wasm client
    /// projecting a bare `TeamMissionRecord` it already has has no way to
    /// supply a meaningful value here and should keep calling plain
    /// `to_view()`; only the daemon, building a `TeamMissionView` to
    /// broadcast, has the live signal in scope.
    pub fn to_view_with_running(&self, running_step: Option<&str>) -> TeamMissionView {
        let steps: Vec<TeamStepView> = self
            .plan
            .steps
            .iter()
            .map(|step| {
                let (kind, member) = match &step.kind {
                    StepKind::Delegate { specialist, .. } => ("delegate", specialist.as_str()),
                    StepKind::Gate { reviewer, .. } => ("gate", reviewer.as_str()),
                };
                TeamStepView {
                    label: format!("{} — {member} ({kind})", step.id),
                    state: self.step_state(&step.id, running_step),
                }
            })
            .collect();
        let total = steps.len().max(1);
        let done = steps
            .iter()
            .filter(|s| matches!(s.state, TeamStepState::Done | TeamStepState::Rejected))
            .count();
        TeamMissionView {
            id: self.id.clone(),
            goal: self.goal.clone(),
            lead: self
                .config
                .as_ref()
                .map(|c| c.lead.clone())
                .unwrap_or_else(|| "coordinator".to_string()),
            phase: self.phase,
            pending_gate: self.pending_gate.clone(),
            halt_reason: self.halt_reason.clone(),
            progress: ((done * 100) / total) as u16,
            steps,
        }
    }

    /// The operator-facing state of one step, derived from the checkpoint
    /// plus an optional live running-step overlay. Precedence, highest
    /// first: a pending human gate is always `Awaiting` (even if
    /// `running_step` stale-matches it — a step paused for operator input is
    /// never "running"); then `running_step`'s own match is `Running`; then
    /// the checkpoint: a rejected output is `Rejected`, any other output is
    /// `Done`, no output is `Pending`.
    fn step_state(&self, step_id: &str, running_step: Option<&str>) -> TeamStepState {
        if self.pending_gate.as_deref() == Some(step_id) {
            return TeamStepState::Awaiting;
        }
        if running_step == Some(step_id) {
            return TeamStepState::Running;
        }
        match self.outputs.get(step_id) {
            Some(v) if v.starts_with("rejected") => TeamStepState::Rejected,
            Some(_) => TeamStepState::Done,
            None => TeamStepState::Pending,
        }
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p aivyx-ipc team_mission::tests -- --test-threads=1`
Expected: PASS — all tests in the module, including the 4 new ones and the
pre-existing `to_view_derives_step_states_and_progress`.

- [ ] **Step 6: Confirm no other call site broke**

Run: `cargo build -p aivyx-channel -p aivyx-tui -p aivyx-cli 2>&1 | tail -40`
Expected: clean build — `to_view()`'s signature is unchanged, so
`crates/aivyx-channel/src/team_mission_driver.rs` (2 test call sites),
`crates/aivyx-tui/src/app.rs:174`, and `crates/aivyx-web/src/main.rs:6016`
all still compile untouched.

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-ipc/src/team_mission.rs
git commit -m "feat: add TeamStepState::Running + to_view_with_running

A pure, stateless addition: to_view_with_running(Option<&str>) overlays
Running onto one step id, with pending-gate precedence preserved exactly.
to_view() becomes a thin None-delegate -- unchanged for all 6 existing
call sites. No wiring yet; Task 3/4 make the daemon actually compute and
pass a real running-step id."
```

---

## Task 2: Generalize the Web UI broadcast channel + the wire message

**Files:**
- Modify: `crates/aivyx-channel/src/notify_webui.rs`
- Modify: `crates/aivyx-channel/src/web_ui.rs`
- Modify: `crates/aivyx-ipc/src/protocol.rs`

**Interfaces:**
- Consumes: `TeamMissionView` (Task 1, already exists regardless).
- Produces: `WebUiBroadcastFrame` enum (`DesktopNotification(DesktopNotificationFrame)` / `TeamMissionUpdated(TeamMissionView)`) — `WebUiBroadcaster::subscribe()`/`broadcast()` now operate on this enum instead of the bare `DesktopNotificationFrame`. `DaemonMessage::TeamMissionUpdated { view: TeamMissionView }` and `DaemonEnvelope::TeamMissionUpdated { view: TeamMissionView }` (new, mirrored, wire-identical variants — Task 3/4 construct and broadcast these; Task 6 consumes the `DaemonEnvelope` side).

- [ ] **Step 1: Write the failing tests**

`crates/aivyx-channel/src/notify_webui.rs`'s existing test module asserts
directly against `DesktopNotificationFrame` values popped off a
`WebUiBroadcaster` receiver (e.g. `broadcast_reaches_single_subscriber`).
Since this task changes what type the channel carries, every existing test
in that module needs its assertions updated in the same commit (this is
not new coverage so much as keeping the existing coverage green against
the new shape) — do this as part of Step 3 below, not as a separate
`git commit`. Before that, add one new test proving a **second** frame kind
can flow through the same channel and reach the same subscriber:

```rust
    #[tokio::test]
    async fn broadcast_relays_a_team_mission_updated_frame() {
        use aivyx_ipc::{TeamMissionPhase, TeamMissionView};
        let bc = Arc::new(WebUiBroadcaster::new());
        let mut rx = bc.subscribe();
        let view = TeamMissionView {
            id: "m1".into(),
            goal: "test".into(),
            lead: "coordinator".into(),
            phase: TeamMissionPhase::Executing,
            pending_gate: None,
            halt_reason: None,
            progress: 0,
            steps: vec![],
        };
        bc.broadcast(WebUiBroadcastFrame::TeamMissionUpdated(view.clone()))
            .expect("broadcast");
        match rx.recv().await.expect("recv") {
            WebUiBroadcastFrame::TeamMissionUpdated(got) => assert_eq!(got, view),
            other => panic!("expected TeamMissionUpdated, got {other:?}"),
        }
    }
```

Check `crates/aivyx-channel/Cargo.toml` for an existing `aivyx-ipc`
dependency before assuming the `use` above resolves — `TeamMissionView`
already crosses this exact boundary elsewhere in the crate (e.g.
`team_mission_driver.rs` already uses `aivyx_ipc::TeamMissionRecord`), so
the dependency almost certainly already exists; if the test doesn't
compile because it's missing, add it (`aivyx-ipc = { path = "../aivyx-ipc"
}`) as part of this step, not a separate one.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p aivyx-channel notify_webui::tests -- --test-threads=1`
Expected: FAIL to compile — `WebUiBroadcastFrame` doesn't exist yet.

- [ ] **Step 3: Generalize `WebUiBroadcaster`'s payload type**

In `crates/aivyx-channel/src/notify_webui.rs`, add the new enum right
after the existing `DesktopNotificationFrame` struct:

```rust
/// Frame carried over the broadcast channel. Converted to
/// [`crate::daemon_ipc::DaemonMessage::DesktopNotification`] at
/// the WS write-site, so the on-wire shape stays in
/// `daemon_ipc.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopNotificationFrame {
    pub title: String,
    pub body: String,
}

/// Chapter Mission Control — the broadcast channel now carries either kind
/// of Web UI push. `WebUiBroadcaster` itself stays a single channel/single
/// subscribe-point per connection (not two separate broadcasters + a
/// `select!` per connection) — the WS relay loop matches on this enum and
/// forwards each variant onto its own `DaemonEnvelope` shape.
#[derive(Debug, Clone, PartialEq)]
pub enum WebUiBroadcastFrame {
    DesktopNotification(DesktopNotificationFrame),
    /// A team mission's live state changed (a step started/finished, or the
    /// mission's phase transitioned) — carries the already-projected view,
    /// computed daemon-side where the live running-step signal is in scope
    /// (see `TeamMissionRecord::to_view_with_running` in `aivyx-ipc`).
    TeamMissionUpdated(aivyx_ipc::TeamMissionView),
}
```

Then change `WebUiBroadcaster`'s internals from `DesktopNotificationFrame`
to `WebUiBroadcastFrame` throughout:

```rust
pub struct WebUiBroadcaster {
    sender: broadcast::Sender<WebUiBroadcastFrame>,
}

impl WebUiBroadcaster {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_BROADCAST_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _initial_rx) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WebUiBroadcastFrame> {
        self.sender.subscribe()
    }

    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }

    pub fn broadcast(&self, frame: WebUiBroadcastFrame) -> Result<(), NotifyError> {
        let _ = self.sender.send(frame);
        Ok(())
    }
}
```

(`with_capacity`/`Default`/`Debug` bodies are otherwise unchanged — only the
generic payload type changes; don't rewrite what isn't shown above.)

Update `NotifyWebUiBackend::send` to wrap its frame:

```rust
    async fn send(
        &self,
        message: &str,
        subject: Option<&str>,
    ) -> Result<(), NotifyError> {
        let title = subject.unwrap_or("Aivyx").to_string();
        let body = message.to_string();
        self.broadcaster
            .broadcast(WebUiBroadcastFrame::DesktopNotification(DesktopNotificationFrame {
                title,
                body,
            }))
    }
```

Update every existing test in this file's `mod tests` that currently
matches a bare `DesktopNotificationFrame { title, body }` off `rx.recv()`
to instead match `WebUiBroadcastFrame::DesktopNotification(
DesktopNotificationFrame { title, body })` — there are 4 such assertions
(`broadcast_reaches_single_subscriber`,
`broadcast_fans_out_to_all_subscribers`,
`missing_subject_falls_back_to_aivyx_title`, and the new test from Step 1
uses the new shape already). `broadcast_with_zero_subscribers_is_ok` and
`kind_reports_web_ui` don't inspect frame contents and need no change.
`debug_impl_includes_receiver_count` doesn't touch the frame type either.

- [ ] **Step 4: Add the mirrored wire variant**

In `crates/aivyx-ipc/src/protocol.rs`, find the `DaemonMessage` enum's
`DesktopNotification` variant:

```rust
    /// Phase 69 — broadcast-style Web UI desktop notification.
    /// Fired by `NotifyWebUiBackend` and
    /// relayed onto every connected Web UI WebSocket. Distinct
    /// from `StreamEvent` (which is per-session); these are
    /// per-daemon notifications without a session correlation.
    DesktopNotification {
        title: String,
        body: String,
    },
}
```

Add a new variant right after it, before the closing `}`:

```rust
    /// Phase 69 — broadcast-style Web UI desktop notification.
    /// Fired by `NotifyWebUiBackend` and
    /// relayed onto every connected Web UI WebSocket. Distinct
    /// from `StreamEvent` (which is per-session); these are
    /// per-daemon notifications without a session correlation.
    DesktopNotification {
        title: String,
        body: String,
    },
    /// Chapter Mission Control — broadcast-style team-mission live update.
    /// Fired by the daemon's `RegistryObserver` whenever a step starts,
    /// finishes, or the mission's phase transitions, and relayed onto
    /// every connected Web UI WebSocket. Same broadcast shape as
    /// `DesktopNotification` (no session correlation) — carries the
    /// already-projected view rather than the raw record, since only the
    /// daemon has the live running-step signal in scope.
    TeamMissionUpdated {
        view: crate::TeamMissionView,
    },
}
```

Find the matching spot in the `DaemonEnvelope` enum (search for its own
`DesktopNotification` variant — it mirrors `DaemonMessage`'s variants per
this enum's own doc comment) and add the identical variant there too:

```rust
    TeamMissionUpdated {
        view: crate::TeamMissionView,
    },
```

(No new doc comment needed on the `DaemonEnvelope` copy — check how the
existing `DesktopNotification` variant is documented there; if it's
undocumented/minimally documented in that enum [since the real doc comment
lives on the `DaemonMessage` copy], match that same convention rather than
duplicating the full comment.)

- [ ] **Step 5: Add a round-trip decode test**

In `crates/aivyx-ipc/src/protocol.rs`'s existing wire round-trip test
(the one iterating a `cases: Vec<DaemonMessage>` array that includes the
two `DaemonMessage::DesktopNotification` cases — search for `// Phase 69 —
Web UI desktop notification broadcast.` to find it), add one more case
right after the two existing `DesktopNotification` entries:

```rust
            DaemonMessage::TeamMissionUpdated {
                view: crate::TeamMissionView {
                    id: "m1".into(),
                    goal: "test goal".into(),
                    lead: "coordinator".into(),
                    phase: crate::TeamMissionPhase::Executing,
                    pending_gate: None,
                    halt_reason: None,
                    progress: 42,
                    steps: vec![],
                },
            },
```

And add a dedicated demux test mirroring the existing "Phase 69 —
DesktopNotification demux from a DaemonMessage frame" test (search for
that comment to find it) right after it:

```rust
        // Chapter Mission Control — TeamMissionUpdated demux from a
        // DaemonMessage frame.
        let update = DaemonMessage::TeamMissionUpdated {
            view: crate::TeamMissionView {
                id: "m2".into(),
                goal: "another goal".into(),
                lead: "coordinator".into(),
                phase: crate::TeamMissionPhase::Done,
                pending_gate: None,
                halt_reason: None,
                progress: 100,
                steps: vec![],
            },
        };
        let frame = encode_frame(&update).expect("encode update");
        let (envelope, _): (DaemonEnvelope, _) = decode_frame(&frame).expect("decode");
        match envelope {
            DaemonEnvelope::TeamMissionUpdated { view } => {
                assert_eq!(view.id, "m2");
                assert_eq!(view.progress, 100);
            }
            other => panic!("expected TeamMissionUpdated, got {other:?}"),
        }
```

- [ ] **Step 6: Wire the WS relay loop's new match arm**

In `crates/aivyx-channel/src/web_ui.rs`, find the `broadcast_to_ws` future
(search for `// Broadcast→WS (Phase 69 Task 5)`). Its current match:

```rust
            loop {
                match rx.recv().await {
                    Ok(DesktopNotificationFrame { title, body }) => {
                        let envelope = DaemonEnvelope::DesktopNotification { title, body };
                        let json = match serde_json::to_string(&envelope) {
                            Ok(j) => j,
                            Err(e) => {
                                eprintln!("aivyx web ui: broadcast serialize error: {e}");
                                continue;
                            }
                        };
                        let mut sink = ws_sink.lock().await;
                        if sink
                            .send(tokio_tungstenite::tungstenite::Message::Text(json.into()))
                            .await
                            .is_err()
                        {
                            return; // WebSocket closed
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Skipped some frames; keep listening.
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
```

Change to:

```rust
            loop {
                let envelope = match rx.recv().await {
                    Ok(WebUiBroadcastFrame::DesktopNotification(DesktopNotificationFrame {
                        title,
                        body,
                    })) => DaemonEnvelope::DesktopNotification { title, body },
                    Ok(WebUiBroadcastFrame::TeamMissionUpdated(view)) => {
                        DaemonEnvelope::TeamMissionUpdated { view }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Skipped some frames; keep listening.
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                };
                let json = match serde_json::to_string(&envelope) {
                    Ok(j) => j,
                    Err(e) => {
                        eprintln!("aivyx web ui: broadcast serialize error: {e}");
                        continue;
                    }
                };
                let mut sink = ws_sink.lock().await;
                if sink
                    .send(tokio_tungstenite::tungstenite::Message::Text(json.into()))
                    .await
                    .is_err()
                {
                    return; // WebSocket closed
                }
            }
```

Update the `use crate::notify_webui::{DesktopNotificationFrame,
WebUiBroadcaster};` import at the top of the file (line 33) to also bring
in the new enum: `use crate::notify_webui::{DesktopNotificationFrame,
WebUiBroadcastFrame, WebUiBroadcaster};`. Update the doc comment
immediately above `broadcast_to_ws` (currently "if a WebUiBroadcaster is
configured, subscribe a fresh receiver and relay every
DesktopNotificationFrame onto the WS as DaemonEnvelope::DesktopNotification")
to say "relay every `WebUiBroadcastFrame` onto its matching
`DaemonEnvelope` variant" instead, so it doesn't go stale.

- [ ] **Step 7: Run all the tests to verify they pass**

Run: `cargo test -p aivyx-channel -p aivyx-ipc -- --test-threads=1`
Expected: PASS — all of `notify_webui::tests` (including the updated
assertions and the new frame test), all of `protocol`'s round-trip tests
(including the two new cases), and everything else in both crates
unaffected.

- [ ] **Step 8: Build the whole workspace to catch any other call site**

Run: `cargo build --workspace --exclude aivyx-desktop --exclude aivyx-web`
Expected: clean. (`aivyx-web` is wasm-only and not buildable with the
native target here — Task 6 verifies it separately with the right target;
`aivyx-desktop` is a known pre-existing broken build in this environment,
unrelated to this change.)

- [ ] **Step 9: Commit**

```bash
git add crates/aivyx-channel/src/notify_webui.rs crates/aivyx-channel/src/web_ui.rs crates/aivyx-ipc/src/protocol.rs
git commit -m "feat: generalize WebUiBroadcaster to carry TeamMissionUpdated too

WebUiBroadcastFrame replaces the bare DesktopNotificationFrame as the
channel's payload -- one broadcaster, one subscribe point per WS
connection, matching each variant onto its own DaemonEnvelope shape at
the relay site rather than running two broadcasters + a select!.
DaemonMessage/DaemonEnvelope both gain the mirrored TeamMissionUpdated
variant. No daemon-side caller constructs it yet -- Task 3/4."
```

---

## Task 3: `SharedMissionState` tracks the live running step + owns the broadcaster

**Files:**
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs`

**Interfaces:**
- Consumes: `WebUiBroadcaster`/`WebUiBroadcastFrame` (Task 2), `TeamMissionRecord::to_view_with_running` (Task 1).
- Produces: `SharedMissionState::with_broadcaster(self, broadcaster: Arc<WebUiBroadcaster>) -> Self` (builder method — Task 5 calls this at daemon startup). Internal (crate-private) `mark_running`/`clear_running` methods — Task 4's `RegistryObserver` calls these.

- [ ] **Step 1: Write the failing tests**

Find `SharedMissionState`'s own test module in
`crates/aivyx-channel/src/team_mission_driver.rs` (search for existing
tests calling `SharedMissionState::new` — e.g. the `anchor_observer_halts_when_aborted`
test near line 1913, or search `mod tests` in this file) and add:

```rust
    #[tokio::test]
    async fn with_broadcaster_is_none_by_default_and_broadcast_live_view_is_a_silent_noop() {
        let shared = SharedMissionState::new(team_domain().await);
        // No broadcaster configured -- must not panic, must not error.
        shared.mark_running("missing-mission", "step-a");
        shared.clear_running("missing-mission");
    }

    #[tokio::test]
    async fn mark_running_then_clear_running_round_trips_through_a_broadcast() {
        use crate::notify_webui::{WebUiBroadcastFrame, WebUiBroadcaster};
        let bc = Arc::new(WebUiBroadcaster::new());
        let mut rx = bc.subscribe();
        let shared = SharedMissionState::new(team_domain().await).with_broadcaster(bc);

        // Seed a real mission record so broadcast_live_view has something
        // to project.
        let plan = aivyx_team_types::MissionPlan::new(
            "goal",
            vec![aivyx_team_types::Step::delegate("a", "specialist", "do a")],
        );
        let record = TeamMissionRecord::new("m1", "goal", plan);
        shared.put(record).await.expect("put");

        shared.mark_running("m1", "a");
        match rx.recv().await.expect("recv running") {
            WebUiBroadcastFrame::TeamMissionUpdated(view) => {
                assert_eq!(view.id, "m1");
                assert_eq!(view.steps[0].state, TeamStepState::Running);
            }
            other => panic!("expected TeamMissionUpdated, got {other:?}"),
        }

        shared.clear_running("m1");
        match rx.recv().await.expect("recv cleared") {
            WebUiBroadcastFrame::TeamMissionUpdated(view) => {
                assert_eq!(view.steps[0].state, TeamStepState::Pending, "no longer running");
            }
            other => panic!("expected TeamMissionUpdated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn clear_running_on_a_mission_with_no_running_marker_is_a_noop_broadcast() {
        // Clearing a mission that was never marked running still broadcasts
        // its current (unchanged) view -- this is fine and expected (Task 4
        // relies on it: on_step_completed/on_gate always call clear_running
        // unconditionally, whether or not on_step_started happened to run
        // first).
        use crate::notify_webui::{WebUiBroadcastFrame, WebUiBroadcaster};
        let bc = Arc::new(WebUiBroadcaster::new());
        let mut rx = bc.subscribe();
        let shared = SharedMissionState::new(team_domain().await).with_broadcaster(bc);
        let plan = aivyx_team_types::MissionPlan::new(
            "goal",
            vec![aivyx_team_types::Step::delegate("a", "specialist", "do a")],
        );
        shared.put(TeamMissionRecord::new("m1", "goal", plan)).await.expect("put");

        shared.clear_running("m1");
        match rx.recv().await.expect("recv") {
            WebUiBroadcastFrame::TeamMissionUpdated(view) => {
                assert_eq!(view.steps[0].state, TeamStepState::Pending);
            }
            other => panic!("expected TeamMissionUpdated, got {other:?}"),
        }
    }
```

Check the top of the existing test module for a `team_domain()` async
helper (used by `anchor_observer_halts_when_aborted` and others to build a
`DomainHandle` test fixture) — reuse it exactly as the other tests in this
file do; do not invent a new fixture.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-channel team_mission_driver -- --test-threads=1`
Expected: FAIL to compile — `with_broadcaster`/`mark_running`/`clear_running`
don't exist yet.

- [ ] **Step 3: Add the fields, builder, and methods**

First, this file's top-of-file import only brings in two of the five types
`crate::team_mission` (this crate's own re-export module, `crates/
aivyx-channel/src/team_mission.rs`, itself `pub use aivyx_ipc::team_mission
::{TeamMissionPhase, TeamMissionRecord, TeamMissionView, TeamStepState,
TeamStepView};`) already makes available. The new tests below reference
`TeamStepState` bare, which isn't imported yet. Find:

```rust
use crate::team_mission::{
    list_team_missions, save_team_mission, TeamMissionPhase, TeamMissionRecord,
};
```

and add `TeamStepState`:

```rust
use crate::team_mission::{
    list_team_missions, save_team_mission, TeamMissionPhase, TeamMissionRecord, TeamStepState,
};
```

Now, in `crates/aivyx-channel/src/team_mission_driver.rs`, find the
`SharedMissionState` struct:

```rust
pub struct SharedMissionState {
    store: DomainHandle,
    registry: Arc<RwLock<BTreeMap<String, TeamMissionRecord>>>,
    /// Chapter Belay — runtime-only abort flags, keyed by mission id. The drive
    /// arms one when a mission starts executing; the observer reads it at each
    /// wave boundary; `request_abort` sets it. Not persisted (a flag is
    /// meaningless across a restart — an interrupted mission re-drives fresh).
    abort_flags: Arc<RwLock<std::collections::HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
}
```

Change to:

```rust
pub struct SharedMissionState {
    store: DomainHandle,
    registry: Arc<RwLock<BTreeMap<String, TeamMissionRecord>>>,
    /// Chapter Belay — runtime-only abort flags, keyed by mission id. The drive
    /// arms one when a mission starts executing; the observer reads it at each
    /// wave boundary; `request_abort` sets it. Not persisted (a flag is
    /// meaningless across a restart — an interrupted mission re-drives fresh).
    abort_flags: Arc<RwLock<std::collections::HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
    /// Chapter Mission Control — which step id is currently executing, keyed
    /// by mission id. Same "runtime-only, not persisted" rationale as
    /// `abort_flags`: a step "running" when the daemon crashed simply isn't
    /// running after a restart, and the checkpoint (`TeamMissionRecord::
    /// outputs`) has no business knowing about it. Read by
    /// `broadcast_live_view` to build a live `TeamMissionView`; never
    /// written into a record's own `outputs`.
    running_steps: Arc<RwLock<std::collections::HashMap<String, String>>>,
    /// Chapter Mission Control — the Web UI broadcaster, if the daemon has
    /// one configured (`None` for a daemon with no Web UI server running).
    /// `mark_running`/`clear_running` no-op the broadcast half when this is
    /// `None`, matching how `budget_guard: Option<...>` already makes the
    /// Ballast check a no-op when unset.
    broadcaster: Option<Arc<crate::notify_webui::WebUiBroadcaster>>,
}
```

Update `SharedMissionState::new` to initialize the two new fields:

```rust
    pub fn new(store: DomainHandle) -> Self {
        SharedMissionState {
            store,
            registry: Arc::new(RwLock::new(BTreeMap::new())),
            abort_flags: Arc::new(RwLock::new(std::collections::HashMap::new())),
            running_steps: Arc::new(RwLock::new(std::collections::HashMap::new())),
            broadcaster: None,
        }
    }
```

Add the builder method and the three new methods right after `new` (before
`arm_abort`):

```rust
    /// Chapter Mission Control — attach a Web UI broadcaster so live
    /// step-state changes are pushed to connected Mission Control clients.
    /// Builder-style, matching this codebase's own established shape for an
    /// optional cross-cutting collaborator (e.g. `SpecialistFactory::
    /// with_kv_cache`/`with_checkpointer` in `aivyx-team`). Omit for daemon
    /// configurations with no Web UI server — `mark_running`/`clear_running`
    /// stay silent no-ops in that case.
    pub fn with_broadcaster(mut self, broadcaster: Arc<crate::notify_webui::WebUiBroadcaster>) -> Self {
        self.broadcaster = Some(broadcaster);
        self
    }

    /// Chapter Mission Control — mark `step_id` as the currently-executing
    /// step for mission `id`, then broadcast the mission's live view. Called
    /// by `RegistryObserver::on_step_started`.
    pub(crate) fn mark_running(&self, id: &str, step_id: &str) {
        self.running_steps
            .write()
            .expect("running steps lock")
            .insert(id.to_string(), step_id.to_string());
        self.broadcast_live_view(id);
    }

    /// Chapter Mission Control — clear `id`'s running-step marker (a step
    /// finished, a gate resolved, or the mission's drive ended) and
    /// broadcast the update. A no-op removal (nothing was marked running)
    /// is fine — the broadcast still fires, reflecting whatever the record's
    /// current state actually is. Called by `RegistryObserver::
    /// on_step_completed`/`on_gate`, and once more at the end of `drive`
    /// mirroring `disarm_abort`'s own cleanup-on-drive-end call.
    pub(crate) fn clear_running(&self, id: &str) {
        self.running_steps.write().expect("running steps lock").remove(id);
        self.broadcast_live_view(id);
    }

    /// Chapter Mission Control — project `id`'s current record (with
    /// whatever running-step marker is currently set, if any) and push it
    /// onto the broadcaster. Silently does nothing if no broadcaster is
    /// configured, or if `id` isn't a known mission (e.g. a race against a
    /// mission that was just deleted — not a real scenario today, but a
    /// defensive no-op costs nothing here).
    fn broadcast_live_view(&self, id: &str) {
        let Some(broadcaster) = &self.broadcaster else { return };
        let Some(record) = self.snapshot(id) else { return };
        let running = self
            .running_steps
            .read()
            .expect("running steps lock")
            .get(id)
            .cloned();
        let view = record.to_view_with_running(running.as_deref());
        let _ = broadcaster.broadcast(crate::notify_webui::WebUiBroadcastFrame::TeamMissionUpdated(view));
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p aivyx-channel team_mission_driver -- --test-threads=1`
Expected: PASS — the 3 new tests plus everything pre-existing in this file
(this file has substantial existing coverage; a full pass here matters,
not just the new tests).

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-channel/src/team_mission_driver.rs
git commit -m "feat: SharedMissionState tracks the live running step + broadcasts it

running_steps mirrors abort_flags's own shape exactly (runtime-only,
keyed by mission id, never persisted). mark_running/clear_running are
pub(crate) -- Task 4's RegistryObserver is the only intended caller.
with_broadcaster is the daemon-startup builder hook -- Task 5."
```

---

## Task 4: Wire `RegistryObserver` to the live step signal

**Files:**
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs`

**Interfaces:**
- Consumes: `SharedMissionState::mark_running`/`clear_running` (Task 3).
- Produces: nothing new for later tasks — this is where the live signal actually starts flowing during a real mission drive.

- [ ] **Step 1: Write the failing test**

`RegistryObserver` is a private struct; its existing tests construct one
directly and call trait methods on it (search for `RegistryObserver {` to
find the existing test(s) that build one, e.g. inside
`anchor_observer_halts_when_aborted`). Add a new test near it:

```rust
    #[tokio::test]
    async fn on_step_started_marks_running_then_completion_clears_it() {
        use crate::notify_webui::{WebUiBroadcastFrame, WebUiBroadcaster};
        let bc = Arc::new(WebUiBroadcaster::new());
        let mut rx = bc.subscribe();
        let shared = SharedMissionState::new(team_domain().await).with_broadcaster(bc);
        let plan = aivyx_team_types::MissionPlan::new(
            "goal",
            vec![aivyx_team_types::Step::delegate("a", "specialist", "do a")],
        );
        shared.put(TeamMissionRecord::new("m1", "goal", plan)).await.expect("put");

        let (tx, _rx_ping) = mpsc::unbounded_channel();
        let observer = RegistryObserver {
            shared: shared.clone(),
            id: "m1".to_string(),
            tx,
            budget_guard: None,
            abort: None,
        };

        observer.on_step_started("a", "specialist");
        match rx.recv().await.expect("recv running") {
            WebUiBroadcastFrame::TeamMissionUpdated(view) => {
                assert_eq!(view.steps[0].state, TeamStepState::Running);
            }
            other => panic!("expected TeamMissionUpdated, got {other:?}"),
        }

        observer.on_step_completed("a", "done output");
        match rx.recv().await.expect("recv completed") {
            WebUiBroadcastFrame::TeamMissionUpdated(view) => {
                assert_eq!(
                    view.steps[0].state,
                    TeamStepState::Done,
                    "cleared running, checkpoint now shows Done"
                );
            }
            other => panic!("expected TeamMissionUpdated, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p aivyx-channel on_step_started_marks_running -- --test-threads=1`
Expected: FAIL — `on_step_started` isn't overridden on `RegistryObserver`
yet, so the first `rx.recv()` never gets a frame (the default no-op trait
method does nothing) and the test hangs/times out or the second assertion
sees the wrong ordering. (If this genuinely hangs rather than failing
cleanly, that itself confirms the gap — note it in the task report rather
than treating it as a broken test.)

- [ ] **Step 3: Implement the observer hooks**

In `crates/aivyx-channel/src/team_mission_driver.rs`, find
`impl MissionObserver for RegistryObserver`:

```rust
impl MissionObserver for RegistryObserver {
    fn on_step_completed(&self, step_id: &str, output: &str) {
        self.shared.touch_in_memory(&self.id, |r| {
            r.outputs.insert(step_id.to_string(), output.to_string());
        });
        let _ = self.tx.send(());
    }

    fn on_gate(&self, step_id: &str, _passed: bool, verdict: &str) {
        self.shared.touch_in_memory(&self.id, |r| {
            r.outputs.insert(step_id.to_string(), verdict.to_string());
        });
        let _ = self.tx.send(());
    }
```

Change to:

```rust
impl MissionObserver for RegistryObserver {
    /// Chapter Mission Control — the runtime is about to run this step's
    /// specialist/reviewer sub-turn. Marks it running (broadcasts
    /// immediately); if this step turns out to be a human gate that pauses
    /// the mission, `step_state`'s own pending-gate precedence (Task 1)
    /// keeps it showing `Awaiting`, not `Running`, regardless of this
    /// marker's stale presence until the gate resolves.
    fn on_step_started(&self, step_id: &str, _member: &str) {
        self.shared.mark_running(&self.id, step_id);
    }

    fn on_step_completed(&self, step_id: &str, output: &str) {
        self.shared.touch_in_memory(&self.id, |r| {
            r.outputs.insert(step_id.to_string(), output.to_string());
        });
        self.shared.clear_running(&self.id);
        let _ = self.tx.send(());
    }

    fn on_gate(&self, step_id: &str, _passed: bool, verdict: &str) {
        self.shared.touch_in_memory(&self.id, |r| {
            r.outputs.insert(step_id.to_string(), verdict.to_string());
        });
        self.shared.clear_running(&self.id);
        let _ = self.tx.send(());
    }
```

(`touch_in_memory` runs first in both, so `clear_running`'s own broadcast —
via `broadcast_live_view`'s `self.snapshot(id)` — reads the just-updated
checkpoint, showing `Done`/`Rejected` correctly rather than a stale
`Pending`.)

Now find the end of the `drive` function (search for `// Chapter Belay —
the drive is over; drop the abort flag.`):

```rust
    let phase = record.phase;
    shared.put(record).await?;
    // Chapter Belay — the drive is over; drop the abort flag.
    shared.disarm_abort(id);
    Ok(phase)
}
```

Change to:

```rust
    let phase = record.phase;
    shared.put(record).await?;
    // Chapter Belay — the drive is over; drop the abort flag.
    shared.disarm_abort(id);
    // Chapter Mission Control — belt-and-braces: every step that starts
    // should already clear its own running marker via on_step_completed/
    // on_gate, but this guarantees no stale entry survives past the
    // drive's own end (and broadcasts the mission's final phase either
    // way), mirroring disarm_abort's own cleanup-on-drive-end call above.
    shared.clear_running(id);
    Ok(phase)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p aivyx-channel team_mission_driver -- --test-threads=1`
Expected: PASS — the new test plus every pre-existing test in this file
(this file has significant existing coverage of `drive`/`RegistryObserver`
— a full, not partial, pass matters here, since this task touches
`on_step_completed`/`on_gate`, both already exercised by existing tests).

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-channel/src/team_mission_driver.rs
git commit -m "feat: RegistryObserver marks/clears the live running step

on_step_started -> mark_running (broadcasts immediately). on_step_completed
/on_gate -> clear_running, called after touch_in_memory so the broadcast's
own snapshot reflects the just-written checkpoint. drive()'s own end also
clears, mirroring disarm_abort's belt-and-braces cleanup."
```

---

## Task 5: Wire the daemon's Web UI broadcaster into `SharedMissionState` at startup

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs`

**Interfaces:**
- Consumes: `SharedMissionState::with_broadcaster` (Task 3), the existing `web_ui_broadcaster: Option<Arc<WebUiBroadcaster>>` local already constructed earlier in the same function.

- [ ] **Step 1: Locate the construction site**

In `crates/aivyx-cli/src/bin/aivyx.rs`, find the `team_missions` block
(search for `Chapter L (L.5) — the daemon's team-mission service`):

```rust
        let team_missions = {
            let state = aivyx_channel::team_mission_driver::SharedMissionState::new(
                storage.domain(KeyDomain::TeamMissions),
            );
            match state.reload().await {
```

Confirm `web_ui_broadcaster` (an `Option<Arc<aivyx_channel::notify_webui::
WebUiBroadcaster>>`, search for `Phase 69 — Web UI desktop notify
broadcaster` to find its construction, well earlier in the same function)
is already in scope at this point — it is, since it's used by
`build_notify_dispatcher` earlier in the same function body, and this
`team_missions` block runs after that.

- [ ] **Step 2: Thread it through**

Change:

```rust
        let team_missions = {
            let state = aivyx_channel::team_mission_driver::SharedMissionState::new(
                storage.domain(KeyDomain::TeamMissions),
            );
            match state.reload().await {
```

to:

```rust
        let team_missions = {
            // Chapter Mission Control — share the same broadcaster the
            // desktop-notification path uses (web_ui_broadcaster, built
            // above), so a step starting/finishing pushes a live update to
            // every connected Mission Control client the same way a
            // desktop notification does. `None` when no Web UI server is
            // configured -- with_broadcaster is simply not called, and
            // mark_running/clear_running stay silent no-ops.
            let mut state = aivyx_channel::team_mission_driver::SharedMissionState::new(
                storage.domain(KeyDomain::TeamMissions),
            );
            if let Some(bc) = web_ui_broadcaster.clone() {
                state = state.with_broadcaster(bc);
            }
            match state.reload().await {
```

Everything after that arm of the block (the `Ok(n) if n > 0 => ...`
match and whatever follows) is unchanged — only the `let state = ...`
binding's construction changes, from `let state = ...;` to `let mut state
= ...;` plus the new conditional `with_broadcaster` call before `state`
is used.

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build -p aivyx-cli 2>&1 | tail -40`
Expected: clean build. This task has no isolated unit to test (it's
startup wiring in a `fn main`-adjacent binary) — Task 3/4's own tests
already cover `with_broadcaster`'s real behavior; this step only proves
the two are connected correctly at the one real call site.

- [ ] **Step 4: Commit**

```bash
git add crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat: thread the daemon's Web UI broadcaster into SharedMissionState

Reuses the same Arc<WebUiBroadcaster> the desktop-notification path
already constructs -- one broadcaster instance, shared. No behavior
change when no Web UI server is configured (the Option stays None)."
```

---

## Task 6: `aivyx-web` consumes the live broadcast

**Files:**
- Modify: `crates/aivyx-web/src/main.rs`

**Interfaces:**
- Consumes: `DaemonEnvelope::TeamMissionUpdated { view: TeamMissionView }` (Task 2).

- [ ] **Step 1: Write the failing test for the upsert helper**

Confirmed before writing this task: `crates/aivyx-web/src/main.rs` and
`guide.rs` have **zero** existing `#[cfg(test)]` blocks and `Cargo.toml`
has no `[dev-dependencies]` section — this crate has no test coverage
today, but `cargo test -p aivyx-web` genuinely **does** compile and run
natively despite the crate's own `description` field saying it "compiles
only for `wasm32-unknown-unknown`" (that description is about the crate
functioning as a real browser app via `web-sys`/`wasm-bindgen`'s FFI, not
about whether `cargo test` can build it — verified directly: `cargo test
-p aivyx-web` on this host compiles cleanly and reports "0 passed; 0
failed" before this task adds anything). So a normal, real, automated test
is possible here — the only reason to keep the new logic in a small
**pure**, `Signal`/`Dioxus`-free helper rather than testing the WS match
arm directly is that a `Signal`/Dioxus-coupled test would need a live
Dioxus runtime scaffold this crate has no precedent for; a pure function
needs none.

Add near the top-level helper functions in `crates/aivyx-web/src/main.rs`
(a good spot: near other small pure functions like `schedule_sort_key` —
search for it to find a similar-sized existing pure helper's location and
style to match):

Add near the top-level helper functions in `crates/aivyx-web/src/main.rs`
(a good spot: near other small pure functions like `schedule_sort_key` —
search for it to find a similar-sized existing pure helper's location and
style to match):

```rust
/// Chapter Mission Control — apply a live `TeamMissionUpdated` broadcast to
/// the current missions list: replace the entry with a matching id, or
/// append it if this is a mission the client hasn't seen yet (e.g. it was
/// created after the last poll). Pure and Signal-free so it's unit-testable
/// without a Dioxus runtime.
fn upsert_mission_view(missions: &mut Vec<TeamMissionView>, updated: TeamMissionView) {
    if let Some(existing) = missions.iter_mut().find(|m| m.id == updated.id) {
        *existing = updated;
    } else {
        missions.push(updated);
    }
}

#[cfg(test)]
mod mission_control_tests {
    use super::*;

    fn view(id: &str, progress: u16) -> TeamMissionView {
        TeamMissionView {
            id: id.to_string(),
            goal: "goal".into(),
            lead: "coordinator".into(),
            phase: TeamMissionPhase::Executing,
            pending_gate: None,
            halt_reason: None,
            progress,
            steps: vec![],
        }
    }

    #[test]
    fn upsert_replaces_an_existing_mission_by_id() {
        let mut missions = vec![view("m1", 10), view("m2", 50)];
        upsert_mission_view(&mut missions, view("m1", 30));
        assert_eq!(missions.len(), 2, "no duplicate inserted");
        assert_eq!(missions[0].progress, 30);
        assert_eq!(missions[1].progress, 50, "m2 untouched");
    }

    #[test]
    fn upsert_appends_an_unseen_mission() {
        let mut missions = vec![view("m1", 10)];
        upsert_mission_view(&mut missions, view("m2", 0));
        assert_eq!(missions.len(), 2);
        assert_eq!(missions[1].id, "m2");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p aivyx-web mission_control_tests`
Expected: FAIL to compile — `upsert_mission_view` doesn't exist yet.

- [ ] **Step 3: Implement it (already done in Step 1's code block above)**

The function is already complete as written in Step 1 — this step is
just confirming there's no separate "minimal stub then fill in" split
needed here, since the full, real implementation was already shown.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p aivyx-web mission_control_tests`
Expected: PASS — both new tests green.

- [ ] **Step 5: Wire the new match arm into the WS read loop**

In `crates/aivyx-web/src/main.rs`, find the `match env` block handling
`DaemonEnvelope` variants (search for `DaemonEnvelope::QueryResponse {
payload: QueryResponsePayload::TeamMissionList { missions: records }`):

```rust
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::TeamMissionList { missions: records },
                    ..
                } => {
                    missions.set(records.iter().map(|r| r.to_view()).collect());
                }
```

Add a new arm right after it:

```rust
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::TeamMissionList { missions: records },
                    ..
                } => {
                    missions.set(records.iter().map(|r| r.to_view()).collect());
                }
                // Chapter Mission Control — a live push: apply it in place
                // rather than waiting for the next poll. The poll above
                // stays as-is (a reconnect/missed-broadcast reconciliation
                // fallback), not removed.
                DaemonEnvelope::TeamMissionUpdated { view } => {
                    let mut current = missions();
                    upsert_mission_view(&mut current, view);
                    missions.set(current);
                }
```

- [ ] **Step 6: Build for the real wasm target**

Run: `cd crates/aivyx-web && dx build` (or whatever this crate's own
`docs`/`README` names as its real build command — check for one before
assuming `dx build`; `Dioxus.toml` in this crate's own directory is a
strong signal it uses the Dioxus CLI's `dx` — confirm rather than guess).
Expected: clean build, no new warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "feat: aivyx-web applies TeamMissionUpdated broadcasts live

upsert_mission_view is a pure, Signal-free helper (unit-tested on the
native target) the new match arm calls to apply a push in place. The
existing poll path is unchanged -- it's now a reconciliation fallback,
not the only source of truth."
```

---

## Final verification (whole plan)

- [ ] `cargo build --workspace --exclude aivyx-desktop --exclude aivyx-web`
      — clean.
- [ ] `cargo test --workspace --exclude aivyx-desktop --exclude aivyx-web -- --test-threads=1`
      — all green; note the new total vs. the pre-plan baseline.
- [ ] `cargo clippy -p aivyx-ipc -p aivyx-channel -p aivyx-cli --all-targets`
      — no new warnings introduced by this plan's changes (record any
      pre-existing ones separately, don't attribute them to this work).
- [ ] Confirm (by reading the diff, not just running tests) that Piece 2
      (`TeamMissionPhase::Paused`, pause/resume IPC + driver methods) and
      Piece 3 (the new Mission Control nav view) were **not** touched —
      this plan's scope is Piece 1 only.

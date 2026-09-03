# Phase 188 — Studio Loop + Reminders Screens Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Studio (`aivyx-web`) gains two new screens — Loop (status +
start/stop controls) and Reminders (read-only pending list) — closing
the two genuinely-missing items from Phase 188's roadmap wording.

**Architecture:** Both screens follow this crate's own established
per-screen shape exactly (same as the MCP screen): a `Signal<XState>`
threaded through `ws_task`/`read_task`'s existing large parameter
lists, a `use_context_provider` in the app root, a
`QueryPayload`/`QueryResponsePayload` round trip against
already-shipped backend queries, and a `#[component]` panel function
that fetches on mount via `use_future`. No backend changes — every
query either already exists (`LoopStatus`/`LoopStart`/`LoopStop`) or
was shipped this session for a different frontend and is unused until
now (`GetReminders`).

**Tech Stack:** Rust, Dioxus (wasm), the existing `aivyx-web` crate.

## Global Constraints

- No new crate dependencies (matches the roadmap's own "dependency-free"
  framing for this phase, and the design's explicit no-new-backend
  scope).
- No new backend/IPC work — every query this plan uses already exists
  in `aivyx-ipc`/`aivyx-channel` before this plan starts.
- This crate's own established testing convention is **pure-function
  tests only** — no screen's `rsx!` output is asserted anywhere in this
  file today (verified: zero `VirtualDom`/render-assertion tests exist).
  Follow that convention: extract the two pieces of real logic (the
  Loop screen's button-disabled state, the Reminders screen's due-offset
  formatting) as standalone functions and test those directly, the same
  way `mcp_health_chip` is tested elsewhere in this file.
- No existing icon exists for either concept (checked both
  `crates/aivyx-web/assets/icons/` and the source `aivyx-brand/icons/`
  repo — neither has a loop/repeat or reminder/bell/clock icon).
  Reuse existing icons rather than commissioning new SVG art (out of
  scope for a code plan): Loop reuses `ICON_SCHEDULES`
  (`schedules.svg` — both concepts are background daemon activity),
  Reminders reuses `ICON_NOTIFICATIONS` (`notifications.svg` — both are
  future-dated operator alerts).
- Both screens land in the sidebar's existing `"System"` group
  (alongside `Schedules`/`Notifications`/`MCP`/`Tools` — the sidebar is
  a hardcoded `groups: Vec<NavGroup>`, not a match on `View::ALL`) — no
  new sidebar group. Separately, the topbar help button's
  `guide_page_for` match falls through to its own `_ => "screens"`
  catch-all for any view with no dedicated guide page — a different
  mechanism from the sidebar, and both new views correctly need no
  explicit arm there (the existing catch-all covers them).
- `cargo build -p aivyx-web --target wasm32-unknown-unknown` and
  `cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings`
  must stay green throughout (the wasm32 toolchain is reachable at
  `$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin` —
  prepend it to `PATH` before either command; the system `cargo` alone
  cannot target wasm32).
- `dist/` must be rebuilt from a `dx bundle --release --platform web`
  run taken *after* all source changes in this plan, and its freshness
  verified by content (`strings dist/assets/*.wasm | grep -F
  "<a literal new string introduced by this plan>"`), not just by
  having run the rebuild command — this crate's own established lesson
  from this session (a stale-bundle bug shipped once already).

---

### Task 1: Loop screen

**Files:**
- Modify: `crates/aivyx-web/src/main.rs` (state struct, threading
  through `ws_task`/`read_task`, the `#[component]` panel, `View`
  enum wiring, icon constant)

**Interfaces:**
- Consumes: `QueryPayload::{LoopStatus, LoopStart, LoopStop}` and
  `QueryResponsePayload::{LoopStatus, LoopControl}` (all pre-existing,
  `crates/aivyx-ipc/src/protocol.rs`); `LoopRunState` (pre-existing,
  `crates/aivyx-ipc/src/loop_state.rs` — fields `active: bool,
  iteration: u32, max_iterations: u32, started_at_unix_ms: u64,
  last_stop_reason: Option<String>, tokens_used: u64, spent_cents: u64,
  consecutive_idle: u32`).
- Produces: `LoopUiState` struct, `loop_button_state(armed: bool,
  active: bool) -> (bool, bool)` (returns `(start_disabled,
  stop_disabled)`) — no later task depends on these, but keep the exact
  names since Task 3's final grep step references them.

- [ ] **Step 1: Write the failing test for the button-state logic**

In `crates/aivyx-web/src/main.rs`, find the test module containing
`mcp_health_chip_no_stats_is_neutral` (search for that test name) and
add these tests directly after it:

```rust
    #[test]
    fn loop_button_state_both_enabled_when_armed_and_idle() {
        let (start_disabled, stop_disabled) = loop_button_state(true, false);
        assert!(!start_disabled, "armed + idle: Start should be enabled");
        assert!(stop_disabled, "idle: Stop should stay disabled");
    }

    #[test]
    fn loop_button_state_stop_enabled_when_active() {
        let (start_disabled, stop_disabled) = loop_button_state(true, true);
        assert!(start_disabled, "already active: Start should be disabled");
        assert!(!stop_disabled, "active: Stop should be enabled");
    }

    #[test]
    fn loop_button_state_start_disabled_when_not_armed() {
        let (start_disabled, stop_disabled) = loop_button_state(false, false);
        assert!(start_disabled, "not armed: nothing to start");
        assert!(stop_disabled, "not armed and idle: nothing to stop");
    }

    #[test]
    fn loop_button_state_stop_enabled_even_when_not_armed_if_somehow_active() {
        // Defensive case: armed=false but active=true shouldn't happen in
        // practice (armed reflects [loop] config presence, active reflects
        // a running driver), but Stop must never be the wrong answer if it
        // does -- an operator must always be able to stop a running loop.
        let (start_disabled, stop_disabled) = loop_button_state(false, true);
        assert!(start_disabled);
        assert!(!stop_disabled);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-web loop_button_state`
Expected: FAIL to compile — `loop_button_state` doesn't exist yet.
(This crate's tests run against the native target, not wasm32 — no
special `PATH` needed for this step.)

- [ ] **Step 3: Add the `LoopUiState` struct and `loop_button_state`**

Find `struct McpState {` (search for it) and add a new struct directly
after its closing `}`:

```rust
/// Chapter I Phase 188 — the Loop screen's state: the daemon's
/// autonomous-loop run status, fanned in by `read_task` from
/// `QueryResponsePayload::LoopStatus`. `loaded` distinguishes "still
/// loading" from "daemon reports armed=false, nothing running" (same
/// convention `McpState`/`ToolsState` already use).
#[derive(Clone, Default, PartialEq)]
struct LoopUiState {
    state: LoopRunState,
    remaining: usize,
    armed: bool,
    gate_enabled: bool,
    max_run_secs: Option<u64>,
    max_run_tokens: Option<u64>,
    max_run_usd: Option<f64>,
    max_idle_iterations: u32,
    loaded: bool,
    /// The last Start/Stop attempt's outcome, for the inline banner.
    /// `None` before any control action this session.
    last_control_result: Option<(bool, String)>,
}

/// Chapter I Phase 188 — pure Start/Stop button-disabled logic for the
/// Loop screen. Returns `(start_disabled, stop_disabled)`. Extracted
/// as its own function so it's testable without rendering anything —
/// this crate's own established convention (see `mcp_health_chip`).
fn loop_button_state(armed: bool, active: bool) -> (bool, bool) {
    let start_disabled = !armed || active;
    let stop_disabled = !active;
    (start_disabled, stop_disabled)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p aivyx-web loop_button_state`
Expected: all 4 tests PASS.

- [ ] **Step 5: Thread `LoopUiState` through the app**

Five touch points, in order. Find each by matching the surrounding
code shown (not by trusting any line number — this crate's `main.rs`
is large and other tasks/prior sessions may have shifted exact lines).

**5a.** Find `let notifications = use_signal(NotificationsState::default);`
and add directly after it:

```rust
    let loop_ui = use_signal(LoopUiState::default);
```

**5b.** Find the `ws_task(` call inside `use_coroutine(move |rx| { ... })`
(the one starting `ws_task(\n            rx, missions, running_overlay, ...`)
and add `loop_ui,` to the argument list, directly after `notifications,`:

```rust
        ws_task(
            rx, missions, running_overlay, dashboard, memory, memory_ui, wiki, lattice, settings, agents,
            teams, documents, voice, skills, skills_ui, mcp, mcp_config_ui, tools, gallery, schedules_ui,
            notifications, loop_ui, notify_config_ui, audit_page, sessions_page, connected, session, transcript, streaming,
            gate, mission_ui, server_info,
        )
```

**5c.** Find `use_context_provider(|| notifications);` and add directly
after it:

```rust
    use_context_provider(|| loop_ui);
```

**5d.** Find `async fn ws_task(` and its parameter `notifications:
Signal<NotificationsState>,` — add a new parameter directly after it:

```rust
    notifications: Signal<NotificationsState>,
    loop_ui: Signal<LoopUiState>,
```

Then find `ws_task`'s own body where it calls `read_task(` (search for
`spawn(read_task(` — the line starting `read, missions, running_overlay,
...`) and add `loop_ui,` to that argument list too, directly after
`notifications,`:

```rust
        spawn(read_task(
            read, missions, running_overlay, dashboard, memory, memory_ui, wiki, lattice, settings, agents,
            teams, documents, voice, skills, skills_ui, mcp, mcp_config_ui, tools, gallery, schedules_ui,
            notifications, loop_ui, notify_config_ui, audit_page, sessions_page, connected, session, transcript, streaming,
            gate, mission_ui, server_info,
        ));
```

**5e.** Find `async fn read_task(` and its parameter `mut notifications:
Signal<NotificationsState>,` — add a new parameter directly after it:

```rust
    mut notifications: Signal<NotificationsState>,
    mut loop_ui: Signal<LoopUiState>,
```

- [ ] **Step 6: Add the response-handling match arms**

Inside `read_task`'s body, find the match arm handling
`QueryResponsePayload::GetMcpServerConfigs { servers }` (the one that
does `mcp.write().configs = servers;`) and add two new arms directly
after its closing `}`:

```rust
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::LoopStatus {
                        state,
                        remaining,
                        armed,
                        gate_enabled,
                        max_run_secs,
                        max_run_tokens,
                        max_run_usd,
                        max_idle_iterations,
                    },
                    ..
                } => {
                    let mut l = loop_ui.write();
                    l.state = state;
                    l.remaining = remaining;
                    l.armed = armed;
                    l.gate_enabled = gate_enabled;
                    l.max_run_secs = max_run_secs;
                    l.max_run_tokens = max_run_tokens;
                    l.max_run_usd = max_run_usd;
                    l.max_idle_iterations = max_idle_iterations;
                    l.loaded = true;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::LoopControl { ok, message },
                    ..
                } => {
                    loop_ui.write().last_control_result = Some((ok, message));
                }
```

- [ ] **Step 7: Run a compile check**

Run: `cargo check -p aivyx-web`
Expected: compiles clean. (Native-target check — catches every wiring
mistake in Steps 5-6 without needing the wasm32 toolchain; the crate's
own `README.md`/`CLAUDE.md` convention is that `cargo check -p
aivyx-web` with the system compiler already catches most real bugs.)

- [ ] **Step 8: Add the `LoopPanel` component and `View::Loop` wiring**

Add the icon constant. Find `const ICON_SCHEDULES: Asset =
asset!("/assets/icons/schedules.svg");` and add directly after it:

```rust
const ICON_LOOP: Asset = asset!("/assets/icons/schedules.svg");
```

Add the `View::Loop` variant. Find `View::Notifications,` inside the
`View` enum's own field list (not `View::ALL` yet — the plain enum
definition) and add directly after it:

```rust
    Loop,
```

Add it to `View::ALL`. Find `View::Sessions,` at the end of the `const
ALL: [View; 22] = [` array and change the array size and add the new
entry:

```rust
    const ALL: [View; 23] = [
```

(only the `22` → `23` changes on that line; leave every existing entry
in the array alone) and add directly after `View::Sessions,` inside the
array body:

```rust
        View::Sessions,
        View::Loop,
    ];
```

Add the slug. Find `View::Sessions => "sessions",` inside `fn slug`
and add directly after it:

```rust
            View::Loop => "loop",
```

Add the label. Find `View::Sessions => "Sessions",` inside `fn label`
and add directly after it:

```rust
            View::Loop => "Loop",
```

Add the sidebar entry. The sidebar is a hardcoded `groups: Vec<NavGroup>`
local variable (not a match on `View::ALL`) — each group is `(&str,
Vec<(Asset, &str, View)>)`. Find the `"System"` group's `vec![` (the
one starting `(ICON_DOCUMENTS, "Documents", View::Documents),` and
ending with `(ICON_GUIDE, "Guide", View::Guide),`) and add a new tuple
directly after `(ICON_NOTIFICATIONS, "Notifications",
View::Notifications),`:

```rust
                (ICON_LOOP, "Loop", View::Loop),
```

Add the render dispatch. Find `View::Skills => rsx! { SkillsPanel {} },`
(the main content-area match on `state.view`) and add directly after
it:

```rust
                View::Loop => rsx! { LoopPanel {} },
```

Now add the component itself. Find `#[component]\nfn McpPanel() ->
Element {` and add a new component directly after `McpPanel`'s closing
`}`:

```rust
#[component]
fn LoopPanel() -> Element {
    let ws = use_context::<Sender>();
    let loop_ui = use_context::<Signal<LoopUiState>>();

    use_future(move || async move {
        ws.send(loop_status_query());
    });

    let l = loop_ui();
    let (start_disabled, stop_disabled) = loop_button_state(l.armed, l.state.active);
    rsx! {
        div { class: "settings",
            div { class: "panel-head",
                h3 { "Autonomous Loop" }
                button {
                    class: "btn-ghost",
                    onclick: move |_| { ws.send(loop_status_query()); },
                    "Refresh"
                }
            }
            if let Some((ok, text)) = l.last_control_result.clone() {
                div { class: if ok { "notice ok" } else { "notice err" }, "{text}" }
            }
            if !l.loaded {
                SkeletonCards { cards: 1 }
            } else if !l.armed {
                div { class: "glass-card empty",
                    p { class: "label-tech", "No [loop] section configured -- nothing to start." }
                }
            } else {
                div { class: "glass-card",
                    p {
                        if l.state.consecutive_idle > 0 && l.state.active {
                            "Stalled ({l.state.consecutive_idle} consecutive idle iterations)"
                        } else if l.state.active {
                            "Running -- iteration {l.state.iteration}/{l.state.max_iterations}"
                        } else {
                            {
                                let reason = l.state.last_stop_reason.as_deref().unwrap_or("never run");
                                rsx! { "Idle ({reason})" }
                            }
                        }
                    }
                    p { class: "label-tech",
                        "Spend: ${(l.state.spent_cents as f64 / 100.0):.2} -- {l.state.tokens_used / 1000}k tokens -- backlog: {l.remaining} remaining"
                    }
                    div { style: "display:flex; gap:10px; margin-top:12px;",
                        button {
                            class: "btn btn-primary",
                            disabled: start_disabled,
                            onclick: move |_| { ws.send(loop_start_query()); },
                            "Start"
                        }
                        button {
                            class: "btn-ghost",
                            disabled: stop_disabled,
                            onclick: move |_| { ws.send(loop_stop_query()); },
                            "Stop"
                        }
                    }
                }
            }
        }
    }
}
```

Add the three query helpers. Find `fn mcp_query() -> FrontendMessage {`
and add three new functions directly before it:

```rust
fn loop_status_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "loop-status".to_string(),
        payload: QueryPayload::LoopStatus,
    }
}

fn loop_start_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "loop-start".to_string(),
        payload: QueryPayload::LoopStart { max_iterations: None },
    }
}

fn loop_stop_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "loop-stop".to_string(),
        payload: QueryPayload::LoopStop,
    }
}

```

- [ ] **Step 9: Run the native compile check and native tests**

Run: `cargo check -p aivyx-web`
Expected: compiles clean.

Run: `cargo test -p aivyx-web`
Expected: all pass, including the 4 new `loop_button_state` tests, no
regressions.

- [ ] **Step 10: Run the real wasm32 build + clippy**

```bash
export PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH"
cargo build -p aivyx-web --target wasm32-unknown-unknown
cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings
```

Expected: both compile/lint clean. This is the real target Studio ships
on — the native `cargo check` in Step 9 catches most bugs but not
every wasm-specific one (see this plan's own Global Constraints).

- [ ] **Step 11: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "feat(Phase 188): Studio Loop screen

Status (active/idle/stalled, iteration progress, spend, backlog
remaining) + Start/Stop controls, reusing the already-shipped
LoopStatus/LoopStart/LoopStop queries (built for the CLI). No new
backend work. loop_button_state extracted as a pure, tested function
per this crate's own established testing convention."
```

---

### Task 2: Reminders screen

**Files:**
- Modify: `crates/aivyx-web/src/main.rs` (state struct, threading
  through `ws_task`/`read_task`, the `#[component]` panel, `View`
  enum wiring, icon reuse)

**Interfaces:**
- Consumes: `QueryPayload::GetReminders` and
  `QueryResponsePayload::Reminders { reminders: Vec<ReminderView> }`
  (both pre-existing, shipped this session for the TUI Dashboard —
  `crates/aivyx-ipc/src/protocol.rs`; `ReminderView` fields: `id:
  String, due_unix: i64, message: String, notify_targets: Vec<String>,
  created_unix: i64`).
- Produces: `RemindersState` struct, `format_due_offset(due_unix: i64,
  now_unix: i64) -> String` — no later task depends on these.

- [ ] **Step 1: Write the failing test for the due-offset formatter**

Find the test module's `loop_button_state_stop_enabled_even_when_not_armed_if_somehow_active`
test (added in Task 1) and add these tests directly after it:

```rust
    #[test]
    fn format_due_offset_future_minutes() {
        assert_eq!(format_due_offset(660, 60), "in 10m");
    }

    #[test]
    fn format_due_offset_future_hours() {
        assert_eq!(format_due_offset(7_260, 60), "in 2h");
    }

    #[test]
    fn format_due_offset_future_days() {
        assert_eq!(format_due_offset(90_060, 60), "in 1d");
    }

    #[test]
    fn format_due_offset_overdue() {
        assert_eq!(format_due_offset(60, 660), "10m overdue");
    }

    #[test]
    fn format_due_offset_exactly_now() {
        assert_eq!(format_due_offset(60, 60), "in 0s");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-web format_due_offset`
Expected: FAIL to compile — `format_due_offset` doesn't exist yet.

- [ ] **Step 3: Add the `RemindersState` struct and `format_due_offset`**

Find `struct LoopUiState {` (added in Task 1) and add a new struct
directly after its closing `}` (i.e. right before `loop_button_state`,
so the new struct sits between `LoopUiState` and its own helper
function — keep `loop_button_state` where it is, insert this struct
above it):

```rust
/// Chapter I Phase 188 — the Reminders screen's state: pending
/// reminders, fanned in by `read_task` from
/// `QueryResponsePayload::Reminders`. Read-only, matching the TUI
/// Dashboard's own posture for the same data (Phase 186) -- no
/// set/cancel UI exists in Studio either.
#[derive(Clone, Default, PartialEq)]
struct RemindersState {
    reminders: Vec<ReminderView>,
    loaded: bool,
}

/// Chapter I Phase 188 — a reminder's due time as a short relative
/// offset. Plain integer-second arithmetic, matching
/// `crates/aivyx-tui/src/render.rs`'s own `format_due_offset` (a
/// separate, non-wasm crate -- nothing is literally shared, this is
/// an independent re-implementation of the same approach for the
/// same reason: no new date/time dependency in this wasm-clean
/// crate).
fn format_due_offset(due_unix: i64, now_unix: i64) -> String {
    let delta = due_unix.saturating_sub(now_unix);
    let abs = delta.unsigned_abs();
    let (value, unit) = if abs < 60 {
        (abs, "s")
    } else if abs < 3_600 {
        (abs / 60, "m")
    } else if abs < 86_400 {
        (abs / 3_600, "h")
    } else {
        (abs / 86_400, "d")
    };
    if delta >= 0 {
        format!("in {value}{unit}")
    } else {
        format!("{value}{unit} overdue")
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p aivyx-web format_due_offset`
Expected: all 5 tests PASS.

- [ ] **Step 5: Thread `RemindersState` through the app**

Same five touch points as Task 1's Step 5, for `reminders` this time.

**5a.** Find `let loop_ui = use_signal(LoopUiState::default);` (added
in Task 1) and add directly after it:

```rust
    let reminders_ui = use_signal(RemindersState::default);
```

**5b.** Find the `ws_task(` call inside `use_coroutine` and add
`reminders_ui,` directly after `loop_ui,`:

```rust
        ws_task(
            rx, missions, running_overlay, dashboard, memory, memory_ui, wiki, lattice, settings, agents,
            teams, documents, voice, skills, skills_ui, mcp, mcp_config_ui, tools, gallery, schedules_ui,
            notifications, loop_ui, reminders_ui, notify_config_ui, audit_page, sessions_page, connected, session, transcript, streaming,
            gate, mission_ui, server_info,
        )
```

**5c.** Find `use_context_provider(|| loop_ui);` and add directly
after it:

```rust
    use_context_provider(|| reminders_ui);
```

**5d.** Find `async fn ws_task(`'s parameter `loop_ui: Signal<LoopUiState>,`
and add directly after it:

```rust
    loop_ui: Signal<LoopUiState>,
    reminders_ui: Signal<RemindersState>,
```

Then find `ws_task`'s own `spawn(read_task(` call and add
`reminders_ui,` directly after `loop_ui,`:

```rust
        spawn(read_task(
            read, missions, running_overlay, dashboard, memory, memory_ui, wiki, lattice, settings, agents,
            teams, documents, voice, skills, skills_ui, mcp, mcp_config_ui, tools, gallery, schedules_ui,
            notifications, loop_ui, reminders_ui, notify_config_ui, audit_page, sessions_page, connected, session, transcript, streaming,
            gate, mission_ui, server_info,
        ));
```

**5e.** Find `async fn read_task(`'s parameter `mut loop_ui:
Signal<LoopUiState>,` and add directly after it:

```rust
    mut loop_ui: Signal<LoopUiState>,
    mut reminders_ui: Signal<RemindersState>,
```

- [ ] **Step 6: Add the response-handling match arm**

Inside `read_task`'s body, find the `QueryResponsePayload::LoopControl
{ ok, message }` match arm (added in Task 1) and add a new arm
directly after its closing `}`:

```rust
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::Reminders { reminders },
                    ..
                } => {
                    let mut r = reminders_ui.write();
                    r.reminders = reminders;
                    r.loaded = true;
                }
```

- [ ] **Step 7: Run a compile check**

Run: `cargo check -p aivyx-web`
Expected: compiles clean.

- [ ] **Step 8: Add the `RemindersPanel` component and `View::Reminders` wiring**

`View::Reminders` reuses `ICON_NOTIFICATIONS` directly — no new icon
constant needed (unlike Task 1's `View::Loop`, which needed
`ICON_LOOP` as an alias since `schedules.svg` didn't already have a
second-use constant).

Add the `View::Reminders` variant. Find `View::Loop,` (added in Task
1, inside the plain enum definition) and add directly after it:

```rust
    Reminders,
```

Add it to `View::ALL`. Find `const ALL: [View; 23] = [` (from Task 1)
and change the size, then find `View::Loop,` inside the array body and
add directly after it:

```rust
    const ALL: [View; 24] = [
```

```rust
        View::Loop,
        View::Reminders,
    ];
```

Add the slug. Find `View::Loop => "loop",` inside `fn slug` and add
directly after it:

```rust
            View::Reminders => "reminders",
```

Add the label. Find `View::Loop => "Loop",` inside `fn label` and add
directly after it:

```rust
            View::Reminders => "Reminders",
```

Add the sidebar entry. Find `(ICON_LOOP, "Loop", View::Loop),` (added
in Task 1, inside the `"System"` group's `vec![`) and add directly
after it:

```rust
                (ICON_NOTIFICATIONS, "Reminders", View::Reminders),
```

Add the render dispatch. Find `View::Loop => rsx! { LoopPanel {} },`
and add directly after it:

```rust
                View::Reminders => rsx! { RemindersPanel {} },
```

Now add the component. Find `#[component]\nfn LoopPanel() -> Element
{` (Task 1) and add a new component directly after `LoopPanel`'s
closing `}`:

```rust
#[component]
fn RemindersPanel() -> Element {
    let ws = use_context::<Sender>();
    let reminders_ui = use_context::<Signal<RemindersState>>();

    use_future(move || async move {
        ws.send(reminders_query());
    });

    let r = reminders_ui();
    let now_unix = js_sys::Date::now() as i64 / 1000;
    rsx! {
        div { class: "settings",
            div { class: "panel-head",
                h3 { "Reminders" }
                if r.loaded && !r.reminders.is_empty() {
                    span { class: "label-tech", "{r.reminders.len()} pending" }
                }
                button {
                    class: "btn-ghost",
                    onclick: move |_| { ws.send(reminders_query()); },
                    "Refresh"
                }
            }
            if !r.loaded {
                SkeletonCards { cards: 2 }
            } else if r.reminders.is_empty() {
                div { class: "glass-card empty",
                    p { class: "label-tech", "No pending reminders." }
                }
            } else {
                div { class: "glass-card",
                    for reminder in r.reminders.iter() {
                        div { key: "{reminder.id}", class: "field-row",
                            span { class: "label-tech", "{format_due_offset(reminder.due_unix, now_unix)}" }
                            span { "{reminder.message}" }
                        }
                    }
                }
            }
        }
    }
}
```

Add the query helper. Find `fn loop_status_query() -> FrontendMessage
{` (Task 1) and add a new function directly before it:

```rust
fn reminders_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "reminders".to_string(),
        payload: QueryPayload::GetReminders,
    }
}

```

`js_sys::Date::now()` introduces no new dependency — `js-sys = "0.3"`
is already listed in `crates/aivyx-web/Cargo.toml`, and `let now =
js_sys::Date::now() as u64;` is already the established pattern used
twice elsewhere in this exact file (e.g. around line 3486). This
plan's `js_sys::Date::now() as i64 / 1000` (line above) matches that
existing convention, just cast to `i64` and divided to seconds to match
`ReminderView.due_unix`'s own type.

- [ ] **Step 9: Run the native compile check and native tests**

Run: `cargo check -p aivyx-web`
Expected: compiles clean.

Run: `cargo test -p aivyx-web`
Expected: all pass, including the 5 new `format_due_offset` tests and
Task 1's 4 `loop_button_state` tests, no regressions.

- [ ] **Step 10: Run the real wasm32 build + clippy**

```bash
export PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH"
cargo build -p aivyx-web --target wasm32-unknown-unknown
cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings
```

Expected: both compile/lint clean.

- [ ] **Step 11: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "feat(Phase 188): Studio Reminders screen

Read-only pending-reminders list, reusing the GetReminders query
Phase 186 shipped for the TUI Dashboard and left unused by Studio.
No set/cancel UI, no new backend work -- matches the approved scope
decision (reminders stay agent-set via chat). format_due_offset
extracted as a pure, tested function, independently re-implementing
aivyx-tui's own equivalent (separate crate, nothing literally
shared)."
```

---

### Task 3: Final sweep

**Files:**
- Modify: `crates/aivyx-web/dist/` (rebuilt bundle)

- [ ] **Step 1: Full default-members build, test, and clippy sweep**

Run: `cargo build`
Expected: compiles clean.

Run: `cargo test`
Expected: all pass, zero failures.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: zero warnings.

- [ ] **Step 2: Rebuild the release wasm bundle**

```bash
export PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH"
cd crates/aivyx-web
dx bundle --release --platform web
cd ../..
rm -rf crates/aivyx-web/dist && mkdir -p crates/aivyx-web/dist
cp -r target/dx/aivyx-web/release/web/public/. crates/aivyx-web/dist/
find crates/aivyx-web/dist -name '*.br' -delete
```

Expected: `dx bundle` completes with no errors (this also re-proves
Task 1/2's release-profile code is genuinely clean — `dx bundle
--release` has caught a real bug earlier this session that dev-profile
checks alone missed).

- [ ] **Step 3: Verify the rebuild is genuinely fresh, by content**

```bash
git status --porcelain crates/aivyx-web/dist/assets/
strings crates/aivyx-web/dist/assets/*.wasm | grep -F "No pending reminders."
strings crates/aivyx-web/dist/assets/*.wasm | grep -F "Autonomous Loop"
```

Expected: `git status` shows a clean one-`.wasm`-added-one-deleted
rename (plus the `.js` sibling); both `strings | grep` calls find a
match — confirms the new UI text is genuinely baked into this rebuilt
bundle, not a stale one left over from before this plan's changes.

- [ ] **Step 4: Confirm no stray placeholder or duplicate wiring**

Run: `grep -c "View::Loop =>" crates/aivyx-web/src/main.rs`
Expected: `3` (the slug arm, the label arm, the render-dispatch arm —
these are the only three `View::Loop` sites written as a match arm;
the enum variant, the `View::ALL` array entry, and the sidebar's
`(ICON_LOOP, "Loop", View::Loop),` tuple are each their own distinct
form, not a `=>` match arm, so they don't count here).

Run: `grep -c "(ICON_LOOP, \"Loop\", View::Loop)" crates/aivyx-web/src/main.rs`
Expected: `1` (the sidebar tuple entry — confirms it wasn't missed or
duplicated).

Run: `grep -c "View::Reminders =>" crates/aivyx-web/src/main.rs`
Expected: `3`, same shape as Loop's own count.

Run: `grep -c "(ICON_NOTIFICATIONS, \"Reminders\", View::Reminders)" crates/aivyx-web/src/main.rs`
Expected: `1`.

If any of these four counts don't match, re-check every touch point in
Step 5/8 of both Task 1 and Task 2 for a missed or duplicated site.

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-web/dist/
git commit -m "chore(Phase 188): rebuild dist/ for the Loop + Reminders screens"
```

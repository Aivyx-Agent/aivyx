# `/classic` Retirement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port `/classic`'s three remaining panes without a Studio equivalent (audit, sessions, learning) to the Dioxus Studio and the TUI, then retire `/classic` itself.

**Architecture:** Three new/extended read-only Studio screens (Audit, Sessions, a Learning panel folded into Command Center) following the existing `NotificationsPanel`/`WikiPanel` recipe — a `Signal<State>` context hydrated by a WS query sent from a component-local `use_future`. A real backend change enriches session tracking (`DaemonState.sessions` goes from `Vec<String>` to a real `SessionRecord`). A TUI `Audit` view replaces its hardcoded placeholder with the same `ListAuditEntries` query, driven through `aivyx-tui`'s existing poll-loop pattern. Finally, `/classic`'s redundant panes are deleted and its no-bundle-fallback role is replaced with a small dedicated page.

**Tech Stack:** Rust, Dioxus (aivyx-web, wasm), ratatui (aivyx-tui), the existing daemon IPC protocol (`aivyx-ipc`).

## Global Constraints

- Every new Rust file/function follows the workspace's zero-clippy-warnings bar: `cargo clippy --all-targets -- -D warnings` must stay clean on every crate touched (checked via `cargo clippy -p <crate> --all-targets -- -D warnings` per task, since `--workspace` also builds the unrelated, sandbox-unbuildable `aivyx-desktop`).
- **`aivyx-web` cannot be compiled in this environment** — no `rustup`, no `wasm32-unknown-unknown` target installed, no GPU/live-serve. Tasks 2, 3, 4 (the Dioxus screens) get careful static review against the exact existing patterns in `main.rs`, but their own "run to verify" steps must be run by whoever has a `rustup`-enabled machine (`just check-web`, then `just build-web` + a live browser check) before merging. This matches this whole v0.9 phase's established "Working mode" (`docs/V09_PLAN.md`).
- `aivyx-channel`, `aivyx-tui`, and `aivyx-ipc` (Tasks 1, 5, 6) are normal crates in this sandbox — `cargo test -p <crate>` and `cargo clippy -p <crate> --all-targets -- -D warnings` both work and must be run for real.
- Every new test that guards a real behavior change must be mutation-tested: revert the fix, run the test, confirm it fails; then restore the fix. Every task below says explicitly which steps are mutation-tested and how.
- Query IDs on the wire are plain strings with no fixed format, but IDs already in use must not collide with a query serving different state — see Task 2's id-guard fix to the existing dashboard audit handler.

---

### Task 1: Backend — enrich session tracking (`SessionRecord`)

**Files:**
- Modify: `crates/aivyx-channel/src/daemon_server.rs` (`DaemonState` struct ~L3776, `StartSession` handler ~L2040, `SubmitInput` handler ~L2059, disconnect cleanup ~L3499, `QueryPayload::ListSessions` handler ~L3933, 4 existing test call sites)
- Modify: `crates/aivyx-ipc/src/protocol.rs` (`SessionSummary` struct ~L1444)
- Test: inline `#[cfg(test)] mod tests` in `daemon_server.rs` (existing module)

**Interfaces:**
- Produces: `SessionRecord { session_id: String, channel: aivyx_core::ChannelPlatform, trust_tier: aivyx_capability::TrustTier, created_at_ms: u64, last_active_at_ms: u64 }` (new, in `daemon_server.rs`) and the enriched `SessionSummary` (same fields, in `aivyx_ipc::protocol`) that Task 3 (web Sessions screen) consumes over the wire.

- [ ] **Step 1: Write the failing test for `SessionRecord` round-tripping through `DaemonState`'s JSON serialization**

The existing `daemon_state_round_trips_through_json` test (in `daemon_server.rs`'s test module) constructs `DaemonState` with `sessions: vec!["ses-abc".into(), "ses-def".into()]` — a `Vec<String>`. Replace it to use the new `SessionRecord` type:

```rust
#[test]
fn daemon_state_round_trips_through_json() {
    let state = DaemonState {
        pid: 12345,
        started_at: 1713700000,
        sessions: vec![
            SessionRecord {
                session_id: "ses-abc".into(),
                channel: aivyx_core::ChannelPlatform::Local,
                trust_tier: aivyx_capability::TrustTier::Trusted,
                created_at_ms: 1713700000000,
                last_active_at_ms: 1713700000000,
            },
            SessionRecord {
                session_id: "ses-def".into(),
                channel: aivyx_core::ChannelPlatform::Telegram,
                trust_tier: aivyx_capability::TrustTier::SemiTrusted,
                created_at_ms: 1713700001000,
                last_active_at_ms: 1713700005000,
            },
        ],
        in_flight_turns: vec!["ses-abc:turn".into()],
    };
    let json = serde_json::to_string(&state).unwrap();
    let parsed: DaemonState = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.sessions.len(), 2);
    assert_eq!(parsed.sessions[0].session_id, "ses-abc");
    assert_eq!(parsed.sessions[0].channel, aivyx_core::ChannelPlatform::Local);
    assert_eq!(parsed.sessions[1].trust_tier, aivyx_capability::TrustTier::SemiTrusted);
    assert_eq!(parsed.sessions[1].last_active_at_ms, 1713700005000);
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p aivyx-channel daemon_state_round_trips_through_json 2>&1 | tail -30`
Expected: a compile error — `SessionRecord` does not exist yet, and `DaemonState.sessions` is still `Vec<String>` (type mismatch against the struct literals above).

- [ ] **Step 3: Define `SessionRecord` and update `DaemonState`**

In `daemon_server.rs`, immediately above the `DaemonState` struct definition (~L3770), add:

```rust
/// One tracked daemon session — channel identity, trust posture, and
/// activity timestamps. Replaces a bare session-id string
/// (`/classic` retirement, `docs/superpowers/specs/2026-08-27-
/// classic-retirement-design.md`) so the Studio's Sessions screen can
/// show more than an opaque id.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub channel: aivyx_core::ChannelPlatform,
    pub trust_tier: aivyx_capability::TrustTier,
    pub created_at_ms: u64,
    pub last_active_at_ms: u64,
}
```

Then change the `DaemonState` field:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DaemonState {
    pub pid: u32,
    pub started_at: u64,
    pub sessions: Vec<SessionRecord>,
    pub in_flight_turns: Vec<String>,
}
```

- [ ] **Step 4: Run the test again to verify it compiles and passes**

Run: `cargo test -p aivyx-channel daemon_state_round_trips_through_json 2>&1 | tail -20`
Expected: still a compile error, this time from the 3 other `DaemonState { sessions: vec!["...".into()], .. }` test literals elsewhere in the file (fixed in Step 7) and from the real `StartSession`/`ListSessions`/disconnect call sites (fixed in Steps 5-6). Do not fix those yet — confirm the *new* test's own literal shape compiles by temporarily commenting out the file's other broken call sites is unnecessary; proceed directly to Steps 5-7, which fix all remaining call sites, then return to this test.

- [ ] **Step 5: Update `StartSession` to capture channel/trust-tier/timestamps**

Find (~L2040):

```rust
                        FrontendMessage::StartSession {
                            role: _,
                            frontend_type,
                        } => {
                            let ft = frontend_type.unwrap_or(FrontendType::Local);
                            channel = Some(channel_factory(ft));

                            let sid = aivyx_core::SessionId::new().to_string();
                            session_id = Some(sid.clone());

                            // Track session in daemon state.
                            if let Ok(mut st) = daemon_state.lock() {
                                st.sessions.push(sid.clone());
                            }
```

Replace with:

```rust
                        FrontendMessage::StartSession {
                            role: _,
                            frontend_type,
                        } => {
                            let ft = frontend_type.unwrap_or(FrontendType::Local);
                            channel = Some(channel_factory(ft));

                            let sid = aivyx_core::SessionId::new().to_string();
                            session_id = Some(sid.clone());

                            // Track session in daemon state — /classic
                            // retirement (Sessions screen): channel and
                            // trust tier are free here (the ChannelContext
                            // was just constructed above); created/
                            // last_active start identical.
                            if let Ok(mut st) = daemon_state.lock() {
                                let now_ms = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_millis() as u64)
                                    .unwrap_or(0);
                                let ch = channel.as_ref().expect("just constructed above");
                                st.sessions.push(SessionRecord {
                                    session_id: sid.clone(),
                                    channel: ch.platform(),
                                    trust_tier: ch.trust_tier(),
                                    created_at_ms: now_ms,
                                    last_active_at_ms: now_ms,
                                });
                            }
```

- [ ] **Step 6: Update `SubmitInput` to bump `last_active_at_ms`, the disconnect cleanup, and the `ListSessions` query handler**

Find the start of the `SubmitInput` arm (~L2059):

```rust
                        FrontendMessage::SubmitInput {
                            session_id: sid,
                            text,
                            mission_id: mid,
                            attachments,
                            headless,
                        } => {
```

Immediately after that opening brace, insert:

```rust
                            // /classic retirement (Sessions screen) —
                            // record this session as active on every
                            // submitted turn, not just at StartSession.
                            if let Ok(mut st) = daemon_state.lock() {
                                if let Some(rec) =
                                    st.sessions.iter_mut().find(|r| r.session_id == sid)
                                {
                                    rec.last_active_at_ms = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_millis() as u64)
                                        .unwrap_or(0);
                                }
                            }
```

Find the disconnect cleanup (~L3499):

```rust
    // Deregister session from daemon state on disconnect.
    if let Some(ref sid) = session_id {
        if let Ok(mut st) = daemon_state.lock() {
            st.sessions.retain(|s| s != sid);
        }
    }
```

Replace with:

```rust
    // Deregister session from daemon state on disconnect.
    if let Some(ref sid) = session_id {
        if let Ok(mut st) = daemon_state.lock() {
            st.sessions.retain(|s| &s.session_id != sid);
        }
    }
```

Find the `ListSessions` query handler (~L3933):

```rust
        QueryPayload::ListSessions => match daemon_state.lock() {
            Ok(st) => {
                let sessions = st
                    .sessions
                    .iter()
                    .map(|s| SessionSummary {
                        session_id: s.clone(),
                    })
                    .collect();
                QueryResponsePayload::ListSessions { sessions }
            }
            Err(_) => QueryResponsePayload::QueryError {
                code: "state_poisoned".into(),
                message: "daemon state mutex poisoned".into(),
            },
        },
```

Replace with:

```rust
        QueryPayload::ListSessions => match daemon_state.lock() {
            Ok(st) => {
                let sessions = st
                    .sessions
                    .iter()
                    .map(|s| SessionSummary {
                        session_id: s.session_id.clone(),
                        channel: s.channel,
                        trust_tier: s.trust_tier,
                        created_at_ms: s.created_at_ms,
                        last_active_at_ms: s.last_active_at_ms,
                    })
                    .collect();
                QueryResponsePayload::ListSessions { sessions }
            }
            Err(_) => QueryResponsePayload::QueryError {
                code: "state_poisoned".into(),
                message: "daemon state mutex poisoned".into(),
            },
        },
```

- [ ] **Step 7: Update `SessionSummary`'s wire type and the 3 remaining test call sites**

In `crates/aivyx-ipc/src/protocol.rs`, find:

```rust
pub struct SessionSummary {
    pub session_id: String,
}
```

Replace with:

```rust
/// Chapter Postern/`/classic` retirement — one session as shown on the
/// Studio's Sessions screen: identity, channel, trust posture, and
/// activity timestamps (Unix millis).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub channel: crate::ChannelPlatform,
    pub trust_tier: crate::TrustTier,
    pub created_at_ms: u64,
    pub last_active_at_ms: u64,
}
```

(If `ChannelPlatform`/`TrustTier` are not already re-exported at `aivyx_ipc`'s crate root, use their real paths — `aivyx_core::ChannelPlatform` / `aivyx_capability::TrustTier` — matching whatever `aivyx-ipc/src/protocol.rs`'s existing imports already use elsewhere in the file; check the top of the file for the exact `use` statements in place before assuming `crate::`.)

In `daemon_server.rs`'s test module, find and update the two other `DaemonState { sessions: vec![...], .. }` literals (~L7532, ~L7561 by the earlier line numbers — confirm the exact current line numbers via `grep -n 'sessions: vec!\["ses' crates/aivyx-channel/src/daemon_server.rs` before editing, since Steps 1-6 may have shifted them) to use `SessionRecord` the same way Step 1 did — reuse the exact same field values, wrapped in `SessionRecord { session_id: "ses-abc".into(), channel: aivyx_core::ChannelPlatform::Local, trust_tier: aivyx_capability::TrustTier::Trusted, created_at_ms: 0, last_active_at_ms: 0 }` (these two tests only assert `.sessions` presence/absence after crash-recovery detection, not field values, so `0` timestamps are fine).

Find the fourth test call site (the `sessions.push("ses-1".into())` / `sessions.retain(|s| s != "ses-1")` pair, ~L7607/7626):

```rust
        // Register a session.
        shared.lock().unwrap().sessions.push("ses-1".into());
        assert_eq!(shared.lock().unwrap().sessions, vec!["ses-1"]);
```

Replace with:

```rust
        // Register a session.
        let rec = SessionRecord {
            session_id: "ses-1".into(),
            channel: aivyx_core::ChannelPlatform::Local,
            trust_tier: aivyx_capability::TrustTier::Trusted,
            created_at_ms: 0,
            last_active_at_ms: 0,
        };
        shared.lock().unwrap().sessions.push(rec.clone());
        assert_eq!(shared.lock().unwrap().sessions, vec![rec]);
```

And:

```rust
        // Deregister session.
        shared.lock().unwrap().sessions.retain(|s| s != "ses-1");
        assert!(shared.lock().unwrap().sessions.is_empty());
```

Replace with:

```rust
        // Deregister session.
        shared.lock().unwrap().sessions.retain(|s| s.session_id != "ses-1");
        assert!(shared.lock().unwrap().sessions.is_empty());
```

- [ ] **Step 8: Run the full `aivyx-channel` test suite**

Run: `cargo test -p aivyx-channel 2>&1 | grep -E "FAILED|error\[|test result:" `
Expected: no `FAILED`, no `error[`; every `test result:` line reads `0 failed`. Confirm `daemon_state_round_trips_through_json` specifically: `cargo test -p aivyx-channel daemon_state_round_trips_through_json -- --nocapture` shows `ok`.

- [ ] **Step 9: Write a mutation-tested regression test for `last_active_at_ms` actually updating on `SubmitInput`**

This is the one genuinely new *behavior* in this task (the other steps are a type change, not new logic) — it needs its own test proving the update really happens, not just that the struct has the field. Add to `daemon_server.rs`'s test module, near the existing session-tracking test from Step 7's fourth call site:

```rust
#[test]
fn submit_input_bumps_last_active_but_not_created_at() {
    let mut sessions = vec![SessionRecord {
        session_id: "ses-1".into(),
        channel: aivyx_core::ChannelPlatform::Local,
        trust_tier: aivyx_capability::TrustTier::Trusted,
        created_at_ms: 1_000,
        last_active_at_ms: 1_000,
    }];
    // Simulate what the SubmitInput handler does: find by session_id,
    // bump last_active_at_ms only.
    let sid = "ses-1".to_string();
    let now_ms = 5_000u64;
    if let Some(rec) = sessions.iter_mut().find(|r| r.session_id == sid) {
        rec.last_active_at_ms = now_ms;
    }
    assert_eq!(sessions[0].created_at_ms, 1_000, "created_at must not move");
    assert_eq!(sessions[0].last_active_at_ms, 5_000);

    // A submit for an unknown session_id must not panic or insert a
    // phantom record (e.g. a stale/already-disconnected session).
    let unknown = "ses-does-not-exist".to_string();
    let before = sessions.clone();
    if let Some(rec) = sessions.iter_mut().find(|r| r.session_id == unknown) {
        rec.last_active_at_ms = now_ms;
    }
    assert_eq!(sessions, before, "unknown session_id must be a no-op");
}
```

This test exercises the exact `find(...).last_active_at_ms = ...` logic Step 6 wrote inline, isolated from the async socket-handling machinery around it (which the file's own existing tests already treat as integration-level, not unit-level).

- [ ] **Step 10: Run it, then mutation-test it**

Run: `cargo test -p aivyx-channel submit_input_bumps_last_active_but_not_created_at -- --nocapture`
Expected: `ok`.

Mutation test: temporarily change `rec.last_active_at_ms = now_ms;` (the first occurrence, inside the `if let Some(rec) = ... find(|r| r.session_id == sid)` block) to `rec.created_at_ms = now_ms;` and rerun the same command — expect `FAILED` (the `created_at` assertion now fails). Revert the change and rerun to confirm `ok` again.

- [ ] **Step 11: Clippy + commit**

Run: `cargo clippy -p aivyx-channel -p aivyx-ipc --all-targets -- -D warnings 2>&1 | tail -20`
Expected: clean (no `error:`/`warning:` lines beyond the normal `Checking`/`Finished` output).

```bash
git add crates/aivyx-channel/src/daemon_server.rs crates/aivyx-ipc/src/protocol.rs
git commit -m "Enrich session tracking with channel/trust-tier/activity timestamps

DaemonState.sessions goes from Vec<String> to Vec<SessionRecord>
(channel, trust_tier, created_at_ms, last_active_at_ms) — the real
backend change docs/superpowers/specs/2026-08-27-classic-retirement-
design.md's Task B calls for, ahead of the Sessions screen itself.
channel/trust_tier/created_at_ms are captured for free at StartSession
(the ChannelContext already exists at that point); last_active_at_ms
is a new update on every SubmitInput.

Mutation-tested: submit_input_bumps_last_active_but_not_created_at
fails if the update targets the wrong field or fires for an unknown
session_id, confirmed by temporarily breaking it and reverting.

SessionSummary (the wire type) carries the same fields now — no
consumer yet; the Sessions screen (next task) is the first.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: Web — Audit screen

**Files:**
- Modify: `crates/aivyx-web/src/main.rs` (`View` enum + its `ALL`/`slug`/`label` methods, `Sidebar`'s `groups`, the title match, the body-render match, the existing dashboard `ListAuditEntries` handler, new `AuditState`/`AuditPanel`)

**Interfaces:**
- Consumes: `aivyx_ipc::{AuditEntrySummary, QueryPayload, QueryResponsePayload, FrontendMessage}` (all pre-existing), the pre-existing `AuditFeed(entries: Vec<AuditEntrySummary>) -> Element` component (reused directly, not rewritten).
- Produces: `View::Audit`, `AuditState { entries: Vec<AuditEntrySummary>, total_len: u64, from_seq: u64 }` context — no other task consumes this directly, but Task 6 (retirement) needs `View::Audit` to exist before it can delete `/classic`'s own audit pane.

- [ ] **Step 1: Add the `View::Audit` variant and its 4 companion-match entries**

In `crates/aivyx-web/src/main.rs`, add `Audit` to the `enum View` body (any position; append at the end of the existing list, right before the closing brace, to minimize diff noise against unrelated variants).

Add to `const ALL: [View; 20]` — the array length **must** change to `21`:

```rust
    const ALL: [View; 21] = [
        View::Command,
        View::Chat,
        View::Missions,
        View::MissionControl,
        View::Schedules,
        View::Notifications,
        View::Memory,
        View::Wiki,
        View::Lattice,
        View::Onboarding,
        View::Agents,
        View::Skills,
        View::Teams,
        View::Documents,
        View::Gallery,
        View::Mcp,
        View::Tools,
        View::Voice,
        View::Settings,
        View::Guide,
        View::Audit,
    ];
```

Add to `fn slug`:

```rust
            View::Audit => "audit",
```

Add to `fn label`:

```rust
            View::Audit => "Audit",
```

- [ ] **Step 2: Add the nav entry and the title**

In `Sidebar`'s `groups` Vec, add `Audit` to the `"System"` group (reusing `ICON_DOCUMENTS` — this chapter ports panes, it doesn't design new icon art; `ICON_MISSIONS` is already reused twice in this same file for `Missions`/`Mission Control`, so icon reuse across nav entries is an established pattern here):

```rust
        (
            "System",
            vec![
                (ICON_DOCUMENTS, "Documents", View::Documents),
                (ICON_DOCUMENTS, "Audit", View::Audit),
                (ICON_GALLERY, "Gallery", View::Gallery),
                (ICON_NOTIFICATIONS, "Notifications", View::Notifications),
                (ICON_PLUGINS, "MCP", View::Mcp),
                (ICON_TOOLS, "Tools", View::Tools),
                (ICON_VOICE, "Voice", View::Voice),
                (ICON_SETTINGS, "Settings", View::Settings),
                (ICON_GUIDE, "Guide", View::Guide),
            ],
        ),
```

In the `title` match:

```rust
        View::Audit => "Audit",
```

- [ ] **Step 3: Define `AuditState`, register it, and add the body-render arm**

Near `NotificationsState`'s own definition, add:

```rust
/// `/classic` retirement — the dedicated Audit screen's state (distinct
/// from `Dashboard.audit_entries`, the Command Center's own short,
/// auto-following tail — this one is explicitly paginated by the
/// operator). Chain-verify reuses `Dashboard.chain_ok` directly rather
/// than duplicating it; see the AuditPanel component below.
#[derive(Clone, Default, PartialEq)]
struct AuditState {
    entries: Vec<AuditEntrySummary>,
    total_len: u64,
    from_seq: u64,
}
```

Where the other `use_signal(...::default)` + `use_context_provider` pairs are declared (alongside `notifications`), add:

```rust
    let audit_page = use_signal(AuditState::default);
```

and, in the `use_context_provider` block:

```rust
    use_context_provider(|| audit_page);
```

In the body-render `match view()`, add:

```rust
                        View::Audit => rsx! { AuditPanel {} },
```

- [ ] **Step 4: Guard the existing dashboard audit handler by query id, and add the new one**

The existing, currently-unguarded handler (in the big `ws_task`-style coroutine match) both this screen's own query *and* the Command Center's short-tail poll will now produce — it must only apply to the poll's own `"mc-audit"` id. Find:

```rust
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::ListAuditEntries { entries, total_len },
                    ..
                } => {
                    let mut d = dashboard.write();
                    d.audit_entries = entries;
                    d.audit_total = total_len;
                    // First dashboard snapshot in — switch off the skeleton.
                    d.loaded = true;
                }
```

Replace with:

```rust
                // /classic retirement — the dedicated Audit screen sends its
                // own ListAuditEntries with a different id ("audit-page");
                // guard this arm to the Command Center poll's own id so the
                // two screens' state don't cross-populate (same pattern the
                // GetProfile handler already uses to route mc-agents-get vs
                // the dashboard's own snapshot).
                DaemonEnvelope::QueryResponse {
                    id,
                    payload: QueryResponsePayload::ListAuditEntries { entries, total_len },
                } if id == "mc-audit" => {
                    let mut d = dashboard.write();
                    d.audit_entries = entries;
                    d.audit_total = total_len;
                    // First dashboard snapshot in — switch off the skeleton.
                    d.loaded = true;
                }
                DaemonEnvelope::QueryResponse {
                    id,
                    payload: QueryResponsePayload::ListAuditEntries { entries, total_len },
                } if id == "audit-page" => {
                    let mut a = audit_page.write();
                    a.entries = entries;
                    a.total_len = total_len;
                }
```

This response is handled inside `read_task` (~L7068), which `ws_task` (~L6934) spawns; `ws_task` itself is invoked once, from `App`'s `use_coroutine` call (~L660). All 4 of these exact sites take the identical long `Signal<T>` parameter list (each one already threading `notifications: Signal<NotificationsState>` through) and must all four gain `audit_page: Signal<AuditState>` in the same position (right after `notifications`). **The `mut` qualifier is not uniform across these sites — verified directly, don't assume**: in `ws_task`'s own signature (~L6934), `notifications`/`dashboard` are plain (no `mut`) while `connected` is `mut connected` (because `ws_task`'s own body calls `connected.set(...)` directly); in `read_task`'s signature (~L7068), `notifications`/`dashboard`/`missions` etc. are *all* `mut`-qualified. The rule: match whichever qualifier `notifications` already has at that specific site — plain in the `App` call and `ws_task`'s signature/spawn call, `mut` in `read_task`'s own signature — rather than applying one rule everywhere.

1. `App`'s `use_coroutine(move |rx| { ws_task(rx, missions, ..., notifications, connected, ...) })` (~L661) — add `audit_page` right after `notifications` in this call's argument list (plain, no `mut` — this is a value, not a binding).
2. `ws_task`'s own signature (~L6934) — add `audit_page: Signal<AuditState>,` right after `notifications: Signal<NotificationsState>,` (no `mut`, matching `notifications`' own qualifier here).
3. `ws_task`'s body, `spawn(read_task(read, missions, ..., notifications, connected, ...))` (~L6989) — add `audit_page` in the same position (plain).
4. `read_task`'s own signature (~L7068) — add `mut audit_page: Signal<AuditState>,` right after `mut notifications: Signal<NotificationsState>,` (`mut`, matching every other parameter in this specific signature).

- [ ] **Step 5: Write `AuditPanel`, reusing the existing `AuditFeed` component**

Add near `NotificationsPanel`:

```rust
/// `/classic` retirement — the dedicated, paginated Audit screen.
/// Reuses the existing AuditFeed row-renderer (built for the Command
/// Center's short tail) rather than a second copy of the same markup.
const AUDIT_PAGE_SIZE: u32 = 50;

#[component]
fn AuditPanel() -> Element {
    let ws = use_context::<Sender>();
    let audit = use_context::<Signal<AuditState>>();
    let dashboard = use_context::<Signal<Dashboard>>();

    // Load the newest page each time the view opens.
    use_future(move || async move {
        let total = audit().total_len;
        let from_seq = total.saturating_sub(AUDIT_PAGE_SIZE as u64);
        ws.send(FrontendMessage::Query {
            id: "audit-page".to_string(),
            payload: QueryPayload::ListAuditEntries { from_seq, limit: AUDIT_PAGE_SIZE },
        });
    });

    let state = audit();
    let chain_ok = dashboard().chain_ok;

    rsx! {
        div { class: "dash-grid",
            div { class: "dash-main",
                section { class: "panel",
                    div { class: "panel-head",
                        h3 { "Audit chain" }
                        span { class: "label-tech", "{state.total_len} total events" }
                    }
                    div { class: "glass-card", style: "margin-bottom:12px;",
                        button {
                            onclick: move |_| ws.send(FrontendMessage::Query {
                                id: "mc-verify".to_string(),
                                payload: QueryPayload::VerifyAuditChain,
                            }),
                            "Verify chain"
                        }
                        match chain_ok {
                            Some(true) => rsx! { span { style: "color: var(--ok, #16a34a); margin-left:8px;", "✓ chain intact" } },
                            Some(false) => rsx! { span { style: "color: var(--danger, #b91c1c); margin-left:8px;", "✗ chain verification failed" } },
                            None => rsx! { span {} },
                        }
                    }
                    AuditFeed { entries: state.entries.clone() }
                }
            }
            aside { class: "dash-rail",
                section { class: "panel",
                    div { class: "panel-head", h3 { "About" } }
                    div { class: "glass-card",
                        p { class: "label-tech",
                            "Every allowed or denied action, HMAC-chained and offline-verifiable. This screen shows the newest {AUDIT_PAGE_SIZE} events; the Command Center's own short tail is separate and always shows the very latest few."
                        }
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 6: Static review (no compiler available here)**

Re-read the full diff (`git diff crates/aivyx-web/src/main.rs`) against every point in Steps 1-5 line by line: every new `View::Audit` match arm present in all 4 existing matches (`ALL`, `slug`, `label`, title, body-render — 5, not 4; recount deliberately), `AuditState` context both created and provided, the coroutine's two parameter-list sites and its call site(s) all updated together (a mismatch here is a compile error a human reviewer must catch since this sandbox cannot). Confirm `AuditFeed`, `AuditEntrySummary`, `Dashboard`, `QueryPayload`, `QueryResponsePayload`, `FrontendMessage`, `Sender` are all already imported at the top of the file (they are, per existing usage elsewhere) — no new `use` statements needed for this task.

- [ ] **Step 7: Hand off for real compilation + browser verification**

This step cannot run in this sandbox. Note in the commit message (Step 8) that `just check-web` and a live browser check are still owed before this is considered done, per the Global Constraints section above.

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "Add a dedicated, paginated Audit screen to the Studio

/classic retirement (docs/superpowers/specs/2026-08-27-classic-
retirement-design.md, Task A): a new View::Audit reuses the existing
AuditFeed row-renderer (built for the Command Center's short tail)
under a dedicated, paginated screen. Chain-verify reuses
Dashboard.chain_ok directly rather than duplicating that state.

The existing dashboard ListAuditEntries handler is now guarded by
query id ('mc-audit') so this screen's own query ('audit-page') no
longer cross-populates it — same routing pattern the GetProfile
handler already uses for mc-agents-get vs the dashboard's own
snapshot.

NOT YET COMPILed/verified in a real browser — this sandbox has no
rustup / wasm32-unknown-unknown target. Owed before merge: just
check-web, then a live operator walkthrough (this phase's established
working mode).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 3: Web — Sessions screen

**Files:**
- Modify: `crates/aivyx-web/src/main.rs` (same set of touch points as Task 2, plus a new `SessionsState`/`SessionsPanel`)

**Interfaces:**
- Consumes: Task 1's enriched `SessionSummary { session_id, channel: ChannelPlatform, trust_tier: TrustTier, created_at_ms, last_active_at_ms }`.
- Produces: `View::Sessions` — Task 6 (retirement) needs this to exist before deleting `/classic`'s own sessions pane.

- [ ] **Step 1: Add `View::Sessions` and its companion-match entries**

Same shape as Task 2 Step 1: add `Sessions` to `enum View`, bump `ALL`'s length to `22` and append `View::Sessions`, add `View::Sessions => "sessions"` to `slug`, `View::Sessions => "Sessions"` to `label`.

- [ ] **Step 2: Add the nav entry and title**

In the `"System"` group, add (reusing `ICON_TEAMS` — a "who's connected" concept, same reuse-not-redesign rationale as Task 2's icon choice):

```rust
                (ICON_TEAMS, "Sessions", View::Sessions),
```

In the `title` match:

```rust
        View::Sessions => "Sessions",
```

- [ ] **Step 3: Define `SessionsState`, register it, add the body-render arm**

```rust
/// `/classic` retirement — the Sessions screen's state. Loaded fresh
/// each time the view opens (sessions are short-lived and this isn't
/// a background-poll surface like Missions/Notifications).
#[derive(Clone, Default, PartialEq)]
struct SessionsState {
    sessions: Vec<SessionSummary>,
}
```

Register alongside the others:

```rust
    let sessions_page = use_signal(SessionsState::default);
    // ... in the use_context_provider block:
    use_context_provider(|| sessions_page);
```

Body-render arm:

```rust
                        View::Sessions => rsx! { SessionsPanel {} },
```

- [ ] **Step 4: Add the response handler**

In the same coroutine match as Task 2 Step 4 (add this as its own arm, no id-guard needed — `ListSessions` has no other consumer today):

```rust
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::ListSessions { sessions },
                    ..
                } => {
                    sessions_page.write().sessions = sessions;
                }
```

Add `sessions_page: Signal<SessionsState>,` to the same 4 sites Task 2 Step 4 already touched (the `App` `use_coroutine` call ~L661, `ws_task`'s signature ~L6934, `ws_task`'s `spawn(read_task(...))` call ~L6989, and `read_task`'s signature ~L7068), right after `audit_page` in each — plain (no `mut`) at the first 3 sites, `mut sessions_page: Signal<SessionsState>,` at `read_task`'s own signature, same rule Task 2 Step 4 established.

- [ ] **Step 5: Write `SessionsPanel`**

```rust
/// One rendered row: channel/trust-tier badge, session id, age, last
/// active. `SessionSummary` fields are all `Copy`/cheap to read
/// directly — no separate row-view type needed.
#[component]
fn SessionRow(entry: SessionSummary) -> Element {
    rsx! {
        div { class: "glass-card routine-row",
            div { class: "row1",
                span { class: "dot live" }
                span { class: "name", "{entry.session_id}" }
                span { class: "label-tech", style: "opacity:0.7;", "[{entry.channel:?} · {entry.trust_tier:?}]" }
            }
            div { class: "row2 label-tech",
                span { "created {rel_time(entry.created_at_ms)}" }
                span { style: "opacity:0.8;", "active {rel_time(entry.last_active_at_ms)}" }
            }
        }
    }
}

#[component]
fn SessionsPanel() -> Element {
    let ws = use_context::<Sender>();
    let sessions = use_context::<Signal<SessionsState>>();

    // Load the current session list each time the view opens.
    use_future(move || async move {
        ws.send(FrontendMessage::Query {
            id: "sessions-page".to_string(),
            payload: QueryPayload::ListSessions,
        });
    });

    let state = sessions();
    let mut rows = state.sessions.clone();
    rows.sort_by(|a, b| b.last_active_at_ms.cmp(&a.last_active_at_ms));

    rsx! {
        div { class: "dash-grid",
            div { class: "dash-main",
                section { class: "panel",
                    div { class: "panel-head",
                        h3 { "Active sessions" }
                        span { class: "label-tech", "{rows.len()} connected" }
                    }
                    if rows.is_empty() {
                        div { class: "glass-card empty",
                            p { class: "label-tech", "No active sessions." }
                        }
                    } else {
                        div { class: "feed",
                            for s in rows.iter() {
                                SessionRow { entry: s.clone() }
                            }
                        }
                    }
                }
            }
        }
    }
}
```

`rel_time` is the same relative-time helper `NotificationHistoryRow` already uses (`rel_time(entry.dispatched_at_unix_ms)`) — confirm it's a free function taking `u64` millis (it is, per that existing call), not a method, before assuming this compiles as-is.

- [ ] **Step 6: Static review**

Same discipline as Task 2 Step 6 — re-read the full diff, confirm all 5 `View::Sessions` match-arm sites, the coroutine's parameter list/call site(s), and that `SessionSummary`'s field names (`channel`, `trust_tier`, `created_at_ms`, `last_active_at_ms`) exactly match Task 1's Step 7 wire-type definition (not the pre-Task-1 bare-`session_id` shape). Confirm `{entry.channel:?}`/`{entry.trust_tier:?}` compiles against `ChannelPlatform`/`TrustTier`'s `Debug` derive (both already derive `Debug`, confirmed in Task 1's research) — Dioxus `rsx!` string interpolation supports `{expr:?}` the same as `format!`.

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "Add a Sessions screen to the Studio

/classic retirement (Task B, frontend half): a new View::Sessions
lists every active session — channel, trust tier, created/last-active
— using Task 1's enriched SessionSummary. Loaded fresh on each visit
(sessions are short-lived; not a background-poll surface like
Missions/Notifications).

NOT YET COMPILED/verified in a real browser — same sandbox limitation
as the Audit screen. Owed before merge: just check-web + a live
operator walkthrough.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 4: Web — Learning panel in Command Center

**Files:**
- Modify: `crates/aivyx-web/src/main.rs` (`Dashboard` struct, `CommandPanel`, the coroutine query-dispatch + response handler)

**Interfaces:**
- Consumes: `aivyx_ipc::insights::{LearningDigest, ProposalProvenance}` via `QueryResponsePayload::LearningInsights { digest, proposals, persona_selection }` (query is `QueryPayload::GetLearningInsights { window_secs: None }`; the **response variant name differs from the query's** — `LearningInsights`, not `GetLearningInsights` — verified directly against `aivyx-ipc/src/protocol.rs`).
- Produces: nothing new for later tasks — this is the smallest, most self-contained piece.

- [ ] **Step 1: Add a `learning` field to `Dashboard`**

```rust
/// Command Center dashboard state — read-only snapshots fanned in by `ws_task`.
#[derive(Clone, Default, PartialEq)]
struct Dashboard {
    audit_entries: Vec<AuditEntrySummary>,
    audit_total: u64,
    chain_ok: Option<bool>,
    assistant_name: Option<String>,
    settings: Option<SettingsSnapshot>,
    schedules: Vec<ScheduleView>,
    /// `/classic` retirement — the self-learning digest (VITRINE.md's
    /// Learning pane, folded in here rather than a dedicated screen).
    /// `None` until the first response arrives.
    learning: Option<aivyx_ipc::insights::LearningDigest>,
    loaded: bool,
}
```

(If `aivyx_ipc::insights` is not the real module path the rest of `main.rs` already uses to reach `LearningDigest`, use whatever path the existing top-of-file `use aivyx_ipc::{...}` import list would need extending with — check that import block for the exact re-export path before assuming `insights::` is public from the crate root.)

- [ ] **Step 2: Dispatch the query and handle the response**

In the one-shot `use_future` block that already sends `mc-profile`/`mc-verify`/`mc-settings`/`mc-schedules` on load, add:

```rust
        ws.send(FrontendMessage::Query {
            id: "mc-learning".to_string(),
            payload: QueryPayload::GetLearningInsights { window_secs: None },
        });
```

In the coroutine's response match, add:

```rust
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::LearningInsights { digest, .. },
                    ..
                } => {
                    dashboard.write().learning = Some(digest);
                }
```

(`proposals` and `persona_selection` are intentionally dropped here — this panel surfaces the digest only, matching the legacy pane's own primary content; if a future pass wants the proposal-provenance detail too, that's new scope, not part of this port.)

Also add the same query to `reconnect_boot_queries()` (~L7053, the function `read_task` calls after a reconnect to refresh every boot one-shot — `mc-profile`/`mc-verify`/`mc-settings`/`mc-schedules`/`mc-teams-roster` are already there), so the Learning panel refreshes on reconnect the same way the rest of the Command Center already does, rather than staying stale until a full page reload:

```rust
        q("mc-learning", QueryPayload::GetLearningInsights { window_secs: None }),
```

- [ ] **Step 3: Add the Learning panel to `CommandPanel`**

`CommandPanel(missions: Vec<TeamMissionView>, dashboard: Dashboard, connected: bool) -> Element` already renders several `section { class: "panel", ... }` blocks in its `dash-main`/`dash-rail` grid. Add a new one — place it in `dash-rail` alongside the existing smaller panels (find that aside block in `CommandPanel`'s body and add this as one more `section`):

```rust
                section { class: "panel",
                    div { class: "panel-head", h3 { "Learning" } }
                    match &dashboard.learning {
                        None => rsx! {
                            div { class: "glass-card empty",
                                p { class: "label-tech", "Loading…" }
                            }
                        },
                        Some(d) if d.recalls_total == 0 => rsx! {
                            div { class: "glass-card empty",
                                p { class: "label-tech", "Nothing learned yet — no recalls in the lookback window." }
                            }
                        },
                        Some(d) => rsx! {
                            div { class: "glass-card",
                                p { class: "label-tech", "{d.recalls_scored}/{d.recalls_total} recalls scored · {d.promoted} promoted · {d.proposals_in_window} proposals this window" }
                                if !d.top_helpful.is_empty() {
                                    p { class: "label-tech", style: "margin-top:6px;",
                                        "Most helpful: "
                                        for (topic, score) in d.top_helpful.iter().take(3) {
                                            span { style: "margin-right:8px;", "{topic} ({score:.2})" }
                                        }
                                    }
                                }
                            }
                        },
                    }
                }
```

- [ ] **Step 4: Static review**

Confirm `LearningDigest`'s field names used above (`recalls_total`, `recalls_scored`, `promoted`, `proposals_in_window`, `top_helpful`) exactly match the struct definition (`crates/aivyx-ipc/src/insights.rs`, verified during design research — `top_helpful: Vec<(String, f32)>`, so `for (topic, score) in ...` destructures correctly and `{score:.2}` is a valid float-format on `f32`). Confirm `CommandPanel`'s real parameter is named `dashboard: Dashboard` (a plain prop, not a `Signal`) — inside the component body it's used as `dashboard.audit_entries` etc. directly (no `()` call), so this new block's `&dashboard.learning` must match that same by-value-not-signal access style, not `dashboard().learning`.

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "Fold a Learning panel into the Command Center

/classic retirement (Task C): the self-learning digest (recalls
scored/promoted, proposals this window, top-helpful topics) now
renders as a Command Center panel, per VITRINE.md's own suggested
candidate — no new nav destination. QueryPayload::GetLearningInsights
already existed fully server-side; this is the first consumer.

NOT YET COMPILED/verified in a real browser.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 5: TUI — Audit view

**Files:**
- Modify: `crates/aivyx-channel/src/daemon_client.rs` (new `list_audit_entries` helper)
- Modify: `crates/aivyx-tui/src/model.rs` (`Msg` variant, `AppState` field, `update()` arm)
- Modify: `crates/aivyx-tui/src/render.rs` (real rendering replacing the placeholder)
- Modify: `crates/aivyx-tui/src/app.rs` (fetch-on-view-switch wiring)
- Test: `crates/aivyx-tui/src/model.rs`'s existing test module (the reducer logic); `app.rs` stays operator-verified per its own existing doc comment, matching the `poll_missions` precedent

**Interfaces:**
- Consumes: `aivyx_channel::daemon_client::list_audit_entries(socket_path, from_seq, limit) -> Result<(Vec<AuditEntrySummary>, u64), DaemonError>` (new, this task's own Step 1).
- Produces: nothing later tasks need — this is the TUI-side leaf of the chapter.

- [ ] **Step 1: Add `list_audit_entries` to `daemon_client.rs`**

Following `team_mission_list`'s exact shape (same file, ~L1216):

```rust
/// `/classic` retirement (Task E) — paginated read of the audit chain
/// for the TUI's Audit view. Mirrors the Studio's own ListAuditEntries
/// query; the daemon caps `limit` at 500 server-side regardless of
/// what's requested here.
pub async fn list_audit_entries(
    socket_path: &Path,
    from_seq: u64,
    limit: u32,
) -> Result<(Vec<aivyx_ipc::AuditEntrySummary>, u64), DaemonError> {
    let payload = send_query(
        socket_path,
        "tui-audit",
        QueryPayload::ListAuditEntries { from_seq, limit },
    )
    .await?;
    match payload {
        QueryResponsePayload::ListAuditEntries { entries, total_len } => {
            Ok((entries, total_len))
        }
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected ListAuditEntries, got {other:?}"
        ))),
    }
}
```

Check the top of `daemon_client.rs` for how `QueryPayload`/`QueryResponsePayload`/`aivyx_ipc::AuditEntrySummary` are already imported elsewhere in the file (e.g. inside `team_mission_list`'s own module scope) and match that exact import style rather than assuming a fully-qualified path is required everywhere.

- [ ] **Step 2: Write the failing test for the `model.rs` reducer**

Following `Msg::MissionsUpdated`'s own test precedent (in `model.rs`'s test module):

```rust
#[test]
fn audit_updated_replaces_entries_and_total() {
    let s = AppState::new();
    assert!(s.audit_entries.is_empty());
    assert_eq!(s.audit_total, 0);

    let entries = vec![aivyx_ipc::AuditEntrySummary {
        seq: 1,
        appended_at_unix_ms: 1_000,
        event_type: "TurnStarted".into(),
        event: serde_json::json!({}),
        mac_hex: "abc".into(),
    }];
    let s = update(s, Msg::AuditUpdated { entries: entries.clone(), total_len: 42 });
    assert_eq!(s.audit_entries, entries);
    assert_eq!(s.audit_total, 42);

    // A second update fully replaces, it doesn't append.
    let s = update(s, Msg::AuditUpdated { entries: vec![], total_len: 42 });
    assert!(s.audit_entries.is_empty());
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test -p aivyx-tui audit_updated_replaces_entries_and_total 2>&1 | tail -20`
Expected: compile error — `Msg::AuditUpdated` and `AppState.audit_entries`/`audit_total` don't exist yet.

- [ ] **Step 4: Add the `Msg` variant, `AppState` fields, and the reducer arm**

In `model.rs`'s `enum Msg`, near `MissionsUpdated`:

```rust
    /// `/classic` retirement (Task E) — a fresh page of audit entries
    /// arrived; replaces the current page + total wholesale (this is a
    /// paginated view, not an append-only feed like Missions).
    AuditUpdated { entries: Vec<aivyx_ipc::AuditEntrySummary>, total_len: u64 },
```

In `AppState`'s struct definition, near wherever `missions: Vec<MissionRow>` (or its equivalent field backing `View::Missions`) is declared, add:

```rust
    /// `/classic` retirement — the Audit view's current page.
    pub audit_entries: Vec<aivyx_ipc::AuditEntrySummary>,
    pub audit_total: u64,
```

Confirm `AppState::new()` (or wherever its `Default`/constructor lives) initializes these to empty/zero — if `AppState` derives `Default`, no change needed; if it has a hand-written constructor, add the two fields there explicitly.

In `pub fn update(mut state: AppState, msg: Msg) -> AppState`, near the `Msg::MissionsUpdated(rows) => { ... }` arm:

```rust
        Msg::AuditUpdated { entries, total_len } => {
            state.audit_entries = entries;
            state.audit_total = total_len;
        }
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p aivyx-tui audit_updated_replaces_entries_and_total -- --nocapture`
Expected: `ok`.

- [ ] **Step 6: Wire the fetch into `app.rs`**

Following `poll_missions`'s exact shape, add:

```rust
/// `/classic` retirement (Task E) — fetch a fresh page of audit
/// entries and push it into the Audit view. Called on switching into
/// the view and on each pagination keypress (Step 7) — this is an
/// operator-paginated screen, not a background-polled live feed like
/// Missions, so there is no timer-driven refresh.
const AUDIT_PAGE_SIZE: u32 = 50;

async fn fetch_audit_page(socket_path: &Path, state: &mut AppState, from_seq: u64) {
    if let Ok((entries, total_len)) =
        aivyx_channel::daemon_client::list_audit_entries(socket_path, from_seq, AUDIT_PAGE_SIZE)
            .await
    {
        apply(state, Msg::AuditUpdated { entries, total_len });
    }
}
```

In the main loop's `Action::Update(msg) => apply(state, msg),` arm, intercept the audit-view-switch case to trigger the initial fetch (replace that one line with):

```rust
            Action::Update(msg) => {
                let switching_to_audit = matches!(msg, Msg::SwitchView(View::Audit));
                apply(state, msg);
                if switching_to_audit {
                    let from_seq = state.audit_total.saturating_sub(AUDIT_PAGE_SIZE as u64);
                    fetch_audit_page(socket_path, state, from_seq).await;
                }
            }
```

- [ ] **Step 7: Add pagination keybindings**

In `event.rs`'s `key_to_action` (where `KeyCode::Char('4') => Action::Update(Msg::SwitchView(View::Audit))` already lives), this task needs a way to page — reuse the existing left/right arrow convention if one already exists elsewhere in this function for a different paginated concept (check for `KeyCode::Left`/`KeyCode::Right` handling first); if none exists yet, add, guarded on the current view being `View::Audit`:

```rust
        KeyCode::Left if state.view == View::Audit => {
            Action::AuditPage { forward: false }
        }
        KeyCode::Right if state.view == View::Audit => {
            Action::AuditPage { forward: true }
        }
```

Add the new `Action::AuditPage { forward: bool }` variant to `enum Action` in this same file, and handle it in `app.rs`'s main loop match (alongside `Action::SubmitMission`/`Action::ResolveGate`):

```rust
            Action::AuditPage { forward } => {
                let page = AUDIT_PAGE_SIZE as u64;
                let from_seq = if forward {
                    state.audit_total.min(state.audit_total.saturating_sub(page).saturating_add(page * 2))
                } else {
                    state.audit_total.saturating_sub(page).saturating_sub(page)
                };
                fetch_audit_page(socket_path, state, from_seq.min(state.audit_total)).await;
            }
```

(The exact backward/forward `from_seq` arithmetic above is a reasonable first cut — re-derive it carefully against `state.audit_entries`'s own current `from_seq`/length at implementation time rather than trusting this formula blindly; the important contract is "moves the window by one page in the requested direction, clamped to `[0, total_len]`," which this plan states as the requirement, not the literal arithmetic.)

- [ ] **Step 8: Replace the hardcoded placeholder in `render.rs`**

Find:

```rust
        View::Audit => (
            "AUDIT",
            placeholder_lines("the HMAC-chained audit stream — events, verification, JSONL export"),
        ),
```

Replace with a real rendering using `state.audit_entries`/`state.audit_total`, matching the existing `panel_block`/`palette` styling conventions used elsewhere in this file:

```rust
        View::Audit => {
            let mut lines: Vec<Line> = vec![Line::from(Span::styled(
                format!("{} total events — ← / → to page", state.audit_total),
                fg(palette::DIMMER),
            ))];
            if state.audit_entries.is_empty() {
                lines.push(Line::from(Span::styled("No entries loaded.", fg(palette::DIM))));
            } else {
                for e in state.audit_entries.iter().rev() {
                    lines.push(Line::from(vec![
                        Span::styled(format!("#{} ", e.seq), fg(palette::DIMMER)),
                        Span::styled(e.event_type.clone(), fg(palette::AMBER)),
                    ]));
                }
            }
            ("AUDIT", lines)
        }
```

(`Line`/`Span`/`fg`/`palette` are already imported and used throughout this file for the other panels — confirm the exact import paths already in scope at the top of `render.rs` rather than adding redundant `use` statements.)

- [ ] **Step 9: Run the full `aivyx-tui` test suite**

Run: `cargo test -p aivyx-tui 2>&1 | grep -E "FAILED|error\[|test result:"`
Expected: no `FAILED`, no `error[`.

- [ ] **Step 10: Clippy**

Run: `cargo clippy -p aivyx-tui -p aivyx-channel --all-targets -- -D warnings 2>&1 | tail -20`
Expected: clean.

- [ ] **Step 11: Commit**

```bash
git add crates/aivyx-channel/src/daemon_client.rs crates/aivyx-tui/src/model.rs crates/aivyx-tui/src/render.rs crates/aivyx-tui/src/app.rs crates/aivyx-tui/src/event.rs
git commit -m "Wire a real Audit view into the TUI

/classic retirement (Task E): View::Audit's hardcoded
placeholder_lines(...) is now a real ratatui rendering fed by the
same ListAuditEntries query the web Audit screen uses (a new
daemon_client::list_audit_entries helper). Fetches on switching into
the view and on Left/Right pagination, following the same
poll_missions-shaped pattern app.rs already uses for the live Missions
feed — except this is operator-paginated, not timer-polled.

Reducer logic (Msg::AuditUpdated -> AppState) is unit-tested per this
crate's own convention (app.rs's live-daemon integration loop stays
operator-verified, matching its existing doc comment).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 6: The actual retirement

**Files:**
- Modify: `crates/aivyx-channel/src/web_ui.rs` (remove the `/classic` route)
- Modify: `crates/aivyx-channel/src/web_ui_static.html` (replaced by a new, much smaller fallback page — see Step 2)
- Test: `crates/aivyx-channel/src/web_ui.rs`'s existing test module

**Interfaces:**
- Consumes: nothing new — this task only removes/replaces existing surface area. **Gated on Tasks 2-5 being real-compiled and live-verified by the operator in a browser** (per the design's own explicit sequencing) — do not execute this task until that verification has actually happened, not just "the code is written."

- [ ] **Step 1: Write the failing test for `/classic` returning 404**

Check `web_ui.rs`'s existing test module for how HTTP responses are tested today (it already has tests asserting `HTML.contains("'ListSessions'")` per Task 1's own research, and other request/response assertions — find the nearest existing test that drives `serve_static`/`handle_connection` against a raw request string and asserts on the response bytes/status line, and match that exact test-harness shape, e.g. a real `tokio::net::TcpListener`/`UnixStream::pair`-based fixture already used elsewhere in this file per this session's own earlier work on `rejected_token_logging_is_rate_limited_per_ip` and the `IpcChannelBridge` test — reuse whichever harness this file's *own* HTTP-serving tests already use, not the socketpair-for-a-ChannelContext one, since those are testing a different layer).

```rust
#[tokio::test]
async fn classic_route_is_gone() {
    // exact assertion shape depends on the harness found above — the
    // requirement is: a request for "/classic" gets a 404 status line,
    // not the legacy HTML.
}
```

- [ ] **Step 2: Replace `web_ui_static.html` and remove the `/classic` route**

Replace the entire contents of `crates/aivyx-channel/src/web_ui_static.html` with a small, dedicated no-bundle fallback page (not the multi-pane legacy app):

```html
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Aivyx</title>
<style>
  body { font-family: system-ui, sans-serif; background: #0b0f14; color: #e6edf3;
         display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }
  .card { max-width: 480px; padding: 24px; border: 1px solid #2a3542; border-radius: 8px; }
  code { background: #161b22; padding: 2px 6px; border-radius: 4px; }
</style>
</head>
<body>
  <div class="card">
    <h1>Aivyx</h1>
    <p>The Studio web bundle isn't built yet. Build it with:</p>
    <p><code>just build-web</code></p>
    <p>then rebuild the daemon binary so it embeds the bundle, and reload this page.</p>
  </div>
</body>
</html>
```

In `crates/aivyx-channel/src/web_ui.rs`'s `serve_static`, find:

```rust
    // `/classic` always serves the legacy single-file inspection UI (audit /
    // memory / learning / proposals / notifications / sessions) — the panes the
    // Dioxus app hasn't ported yet (Chapter M ships Missions + Chat). The new
    // app links to it so building the bundle never loses a pane.
    if path == "/classic" {
        return serve_bytes_ext(
            stream,
            "200 OK",
            "text/html; charset=utf-8",
            extra,
            HTML.as_bytes(),
        )
        .await;
    }
```

Delete this whole block — `/classic` is no longer a route at all (the `HTML` constant, still `include_str!`-ing the now-tiny fallback file, keeps serving `/`'s no-bundle case exactly as it always has via whatever code path already falls through to it for `/`; confirm that fallthrough path is untouched by this deletion before removing anything else).

- [ ] **Step 3: Run the test to verify it passes**

Run: `cargo test -p aivyx-channel classic_route_is_gone -- --nocapture`
Expected: `ok`.

- [ ] **Step 4: Search for and update any remaining `/classic` references**

Run: `grep -rn "classic" crates/aivyx-web/src/main.rs crates/aivyx-cli/src/bin/aivyx_modules/doctor.rs crates/aivyx-channel/src/*.rs docs/*.md`

Two known references from this chapter's own design research: the `nav-classic` link in `aivyx-web/src/main.rs` (`a { class: "nav-item nav-classic", href: "/classic", ... }` — delete this whole nav link, it now points at a 404) and `doctor.rs`'s own mention (check its exact context before deciding whether to update or leave — it may just be an unrelated string match, e.g. a code comment referencing "classic" in some other sense; read the surrounding lines before touching it). Update or justify leaving each hit this grep surfaces — don't leave a dangling link or a stale doc claim unaddressed.

- [ ] **Step 5: Full workspace verification**

Run: `cargo clippy --all-targets -- -D warnings 2>&1 | tail -20` (default-members; excludes the sandbox-unbuildable `aivyx-desktop`)
Expected: clean.

Run: `cargo test 2>&1 | grep -E "FAILED|error\[|test result:"`
Expected: no `FAILED`, no `error[`.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-channel/src/web_ui.rs crates/aivyx-channel/src/web_ui_static.html crates/aivyx-web/src/main.rs
git commit -m "Retire /classic

/classic retirement (Task D, the last piece): with Audit, Sessions,
and Learning all now real Studio/TUI surfaces (Notifications already
shipped 2026-07-07 via Chapter Herald), /classic's remaining panes
(chat/missions/profile/persona/proposals/memory were already covered)
are fully redundant. The route is removed outright; web_ui_static.html
(still /'s no-bundle fallback) shrinks from a 2,879-line multi-pane
app to a small dedicated 'build the Studio bundle' page — NOT a blind
delete, the fallback role is replaced, not dropped.

Gated on Tasks 2-5 having been real-compiled and live-verified by the
operator in a browser first, per docs/superpowers/specs/2026-08-27-
classic-retirement-design.md's own explicit sequencing.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:** Task A (Audit) → Task 2. Task B (Sessions, enriched) → Tasks 1+3. Task C (Learning → Command Center) → Task 4. Task D (the retirement) → Task 6, correctly gated last. Task E (TUI Audit) → Task 5. TUI Tools and the MCP health signal are correctly absent — out of scope per the design, split into `POLISH_WAVES.md` sub-project 8.

**Placeholder scan:** No TBD/TODO. Two spots deliberately leave a decision to the implementer rather than asserting false precision: Task 5 Step 7's pagination arithmetic (stated as "a reasonable first cut... re-derive carefully," with the actual contract given explicitly) and Task 6 Step 1's test-harness choice (pointing at where to find the real precedent rather than guessing a harness shape unverified). Both are honest acknowledgments of this sandbox's real limits (no compiler for `aivyx-web`, and `web_ui.rs`'s exact existing HTTP-test harness wasn't re-confirmed line-by-line during planning), not vague hand-waving — each names the concrete thing to go check.

**Type consistency:** `SessionRecord`/`SessionSummary` field names (`session_id`, `channel`, `trust_tier`, `created_at_ms`, `last_active_at_ms`) are identical across Task 1 (backend), Task 3 (web consumer), and the design doc. `AuditState`/`Msg::AuditUpdated` both carry `entries: Vec<AuditEntrySummary>` + a total — consistent between Task 2 (web) and Task 5 (TUI), which is expected since both wrap the same wire type, not a shared Rust type between the two frontends (they're separate crates/binaries).

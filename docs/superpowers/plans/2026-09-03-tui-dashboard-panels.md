# Phase 186 — TUI Dashboard panels — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `crates/aivyx-tui`'s `View::Dashboard` stub with real
loop-status, reminders, missions, and audit summary panels — the last gap
Phase 185 left open.

**Architecture:** One new backend query (`GetReminders`, since reminders
has no frontend IPC surface anywhere today) threaded through
`daemon_server.rs`'s existing `DaemonConfig`/`ConnectionContext`/
`handle_query` plumbing pattern; loop-status reuses the already-existing
`QueryPayload::LoopStatus` query; missions/audit summaries reuse
`aivyx-tui`'s own already-live state and fetch functions. Dashboard fetches
on switching into the view (matching Audit/Tools), then loop-status and
reminders alone ride the existing Missions poll tick while Dashboard stays
active.

**Tech Stack:** Rust, tokio, ratatui (`aivyx-tui`), the existing daemon
Unix-socket IPC (`aivyx-ipc`/`aivyx-channel`). No new dependencies, no
`aivyx-web`/wasm surface touched.

## Global Constraints

- Zero clippy warnings: `cargo clippy --all-targets -- -D warnings` (from
  the `aivyx` repo root — this touches only default-members crates, no
  `-p`/`--target` needed since `aivyx-tui` isn't wasm-only).
- `cargo test` (no `-p`, no `--workspace`) must stay green throughout —
  matches this repo's own default-members convention (`aivyx-desktop`
  requires system GTK libs this sandbox may lack; don't add `--workspace`).
- No new crate dependencies anywhere in this plan.
- Reminder due-time offsets use plain integer-second arithmetic — no new
  date/time crate.
- `aivyx-ipc` (`crates/aivyx-ipc`) must stay free of any dependency on
  `aivyx-channel` — new wire types for reminders are defined natively in
  `aivyx-ipc/src/protocol.rs`, not reused from `aivyx-channel`'s internal
  `reminder_store::Reminder` type. `daemon_server.rs` converts between them
  with a plain function (an `impl From<Reminder> for ReminderView` would
  violate the orphan rule — both types are foreign to `aivyx-channel`).

---

### Task 1: `GetReminders` backend query, end-to-end

**Files:**
- Modify: `crates/aivyx-ipc/src/protocol.rs` (add `ReminderView` struct,
  `QueryPayload::GetReminders`, `QueryResponsePayload::Reminders`)
- Modify: `crates/aivyx-channel/src/daemon_server.rs` (add
  `reminder_to_view`, `reminders_query_response`, the `GetReminders` match
  arm, and thread a new `reminder_store` field through `DaemonConfig`,
  `ConnectionContext`, `handle_connection`, and `handle_query`)
- Modify: `crates/aivyx-channel/src/daemon_client.rs` (add `get_reminders`)
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` (wire the daemon's existing
  `reminder_store` local into the new `DaemonConfig` field)
- Test: `crates/aivyx-channel/src/daemon_server.rs` (new `#[cfg(test)]`
  module tests, alongside `reminders_query_response`)

**Interfaces:**
- Consumes: `aivyx_channel::reminder_store::{Reminder, ReminderStore,
  ReminderStoreError}` (unchanged, already shipped — `id: String,
  due_unix: i64, message: String, notify_targets: Vec<String>,
  created_unix: i64`; `ReminderStore::list(&self) -> Result<Vec<Reminder>,
  ReminderStoreError>`, soonest-first); `aivyx_channel::reminder_tool::
  SharedReminderStore` (= `Arc<ReminderStore>`, unchanged).
- Produces: `aivyx_ipc::protocol::ReminderView` (`id: String, due_unix:
  i64, message: String, notify_targets: Vec<String>, created_unix: i64`);
  `QueryPayload::GetReminders` (unit variant); `QueryResponsePayload::
  Reminders { reminders: Vec<ReminderView> }`;
  `aivyx_channel::daemon_client::get_reminders(socket_path: &Path) ->
  Result<Vec<ReminderView>, DaemonError>` — Task 2 calls this directly.

- [ ] **Step 1: Add the wire types to `aivyx-ipc`**

In `crates/aivyx-ipc/src/protocol.rs`, find the end of the `QueryPayload`
enum — the last variant is `GetMcpServerCallStats`, closing the enum at:

```rust
    GetMcpServerCallStats {
        #[serde(default)]
        window_secs: Option<u64>,
    },
}
```

Insert a new variant right before the closing `}`:

```rust
    GetMcpServerCallStats {
        #[serde(default)]
        window_secs: Option<u64>,
    },
    /// Phase 186 — read-only reminders query for the TUI Dashboard's
    /// reminders panel. No frontend surface existed for `remind.*`
    /// before this: the feature was agent-tool-only
    /// (`crate::reminder_tool::RemindListTool`). Always "all pending" —
    /// no parameters, matching `ReminderStore::list`'s own shape.
    GetReminders,
}
```

Now find the end of the `QueryResponsePayload` enum — the last variant is
`McpServerCallStats`:

```rust
    /// Response to [`QueryPayload::GetMcpServerCallStats`].
    McpServerCallStats { servers: Vec<McpServerCallStats> },
}
```

Insert a new variant right before the closing `}`:

```rust
    /// Response to [`QueryPayload::GetMcpServerCallStats`].
    McpServerCallStats { servers: Vec<McpServerCallStats> },
    /// Response to [`QueryPayload::GetReminders`].
    Reminders { reminders: Vec<ReminderView> },
}
```

Now add the `ReminderView` struct. Find the `McpServerCallStats` struct
definition (search for `pub struct McpServerCallStats`) and insert a new
struct directly after its closing `}`:

```rust
/// Phase 186 — a wasm-clean mirror of `aivyx_channel::reminder_store::
/// Reminder`, carried on the wire by [`QueryResponsePayload::Reminders`].
/// `aivyx-ipc` cannot depend on `aivyx-channel` (the wasm-clean
/// boundary), so this is a plain field-for-field copy, not a shared type
/// — `daemon_server.rs` converts between them with a free function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderView {
    pub id: String,
    /// When the reminder fires, unix seconds.
    pub due_unix: i64,
    pub message: String,
    #[serde(default)]
    pub notify_targets: Vec<String>,
    pub created_unix: i64,
}
```

- [ ] **Step 2: Run a compile check to confirm the new types build**

Run: `cargo check -p aivyx-ipc`
Expected: compiles clean (no consumers reference the new variants/struct
yet, so nothing else should break).

- [ ] **Step 3: Write the failing test for `reminders_query_response`**

In `crates/aivyx-channel/src/daemon_server.rs`, find the `#[cfg(test)] mod
tests` block (search for `mod tests {` near the end of the file — the same
module `fold_mcp_server_stats_buckets_by_server_not_by_shared_base` lives
in). Add these tests near the other `GetMcpServerCallStats`-era tests:

```rust
    async fn open_reminder_store() -> aivyx_channel::reminder_tool::SharedReminderStore {
        // Mirrors `reminder_store.rs`'s own `open_store()` test fixture.
        let dir = std::env::temp_dir()
            .join(format!("aivyx-dashboard-reminders-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage: Arc<dyn aivyx_storage::Storage> = aivyx_storage::RedbStorage::open(
            aivyx_storage::StorageConfig::new(dir.join("store.redb")),
            aivyx_crypto::MasterKey::from_raw([201u8; 32]),
        )
        .await
        .unwrap();
        Arc::new(crate::reminder_store::ReminderStore::new(
            storage.domain(aivyx_storage::KeyDomain::Reminders),
        ))
    }

    #[tokio::test]
    async fn reminders_query_response_lists_pending_soonest_first() {
        let store = open_reminder_store().await;
        store
            .set(&crate::reminder_store::Reminder {
                id: "r1".into(),
                due_unix: 300,
                message: "call mom".into(),
                notify_targets: vec![],
                created_unix: 0,
            })
            .await
            .unwrap();
        store
            .set(&crate::reminder_store::Reminder {
                id: "r2".into(),
                due_unix: 100,
                message: "standup".into(),
                notify_targets: vec!["telegram:123".into()],
                created_unix: 0,
            })
            .await
            .unwrap();

        let resp = reminders_query_response(Some(&store)).await;
        let QueryResponsePayload::Reminders { reminders } = resp else {
            panic!("expected Reminders, got {resp:?}");
        };
        assert_eq!(reminders.len(), 2);
        assert_eq!(reminders[0].id, "r2"); // due 100, soonest first
        assert_eq!(reminders[0].notify_targets, vec!["telegram:123".to_string()]);
        assert_eq!(reminders[1].message, "call mom");
    }

    #[tokio::test]
    async fn reminders_query_response_none_store_is_empty_not_an_error() {
        let resp = reminders_query_response(None).await;
        let QueryResponsePayload::Reminders { reminders } = resp else {
            panic!("expected Reminders, got {resp:?}");
        };
        assert!(reminders.is_empty());
    }
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p aivyx-channel reminders_query_response`
Expected: FAIL — `reminders_query_response` is not defined yet.

- [ ] **Step 5: Implement `reminder_to_view` + `reminders_query_response`**

In `crates/aivyx-channel/src/daemon_server.rs`, find `fn
fold_mcp_server_stats(` (the sibling function this mirrors) and add these
two functions directly above it:

```rust
/// Phase 186 — `aivyx-ipc` cannot depend on `aivyx-channel` (the
/// wasm-clean boundary), so this is a plain field-for-field copy, not a
/// `From` impl (`impl From<Reminder> for ReminderView` would violate the
/// orphan rule: both types are foreign to this crate's own local types).
fn reminder_to_view(r: crate::reminder_store::Reminder) -> ReminderView {
    ReminderView {
        id: r.id,
        due_unix: r.due_unix,
        message: r.message,
        notify_targets: r.notify_targets,
        created_unix: r.created_unix,
    }
}

/// Phase 186 — the `GetReminders` query's actual logic, extracted so it's
/// testable without hand-constructing the giant `handle_query` parameter
/// list (matches `fold_tool_stats`/`fold_mcp_server_stats`'s own
/// extracted-pure-function precedent). `None` (no `[reminders]` /
/// reminder store configured) degrades to an empty list, not an error —
/// matches every other `Option<&...>` arm in `handle_query`.
async fn reminders_query_response(
    reminder_store: Option<&crate::reminder_tool::SharedReminderStore>,
) -> QueryResponsePayload {
    let Some(store) = reminder_store else {
        return QueryResponsePayload::Reminders { reminders: Vec::new() };
    };
    match store.list().await {
        Ok(reminders) => QueryResponsePayload::Reminders {
            reminders: reminders.into_iter().map(reminder_to_view).collect(),
        },
        Err(e) => QueryResponsePayload::QueryError {
            code: "reminders_failed".into(),
            message: e.to_string(),
        },
    }
}
```

- [ ] **Step 6: Add the `GetReminders` match arm in `handle_query`**

In `crates/aivyx-channel/src/daemon_server.rs`, find the
`QueryPayload::GetMcpServerCallStats { window_secs } => { ... }` arm
inside `handle_query` and add a new arm directly after its closing `}`:

```rust
        QueryPayload::GetReminders => reminders_query_response(reminder_store).await,
```

- [ ] **Step 7: Thread `reminder_store` through `DaemonConfig`**

In `crates/aivyx-channel/src/daemon_server.rs`, find the `DaemonConfig`
struct's last field:

```rust
    /// Chapter Z — the canonical roots the read-only Documents browser may reach
    /// (`fs` = the access-scoped `fs_root`, `workspace` = the agent's workspace).
    pub document_roots: DocumentRoots,
}
```

Add a new field before the closing `}`:

```rust
    /// Chapter Z — the canonical roots the read-only Documents browser may reach
    /// (`fs` = the access-scoped `fs_root`, `workspace` = the agent's workspace).
    pub document_roots: DocumentRoots,
    /// Phase 186 — the reminder store for the `GetReminders` query (the
    /// TUI Dashboard's reminders panel). `None` ⇒ the `GetReminders`
    /// query returns an empty list rather than erroring — matches the
    /// existing `remind.*` tools' own degrade-gracefully posture.
    pub reminder_store: Option<crate::reminder_tool::SharedReminderStore>,
}
```

- [ ] **Step 8: Thread it through `run_daemon`'s destructure and the real `ConnectionContext` construction**

In `run_daemon`, find `let DaemonConfig { ... document_roots, ...` (the
destructure near the top of the function) and add `reminder_store,`
directly after `document_roots,`.

Then find the real `ConnectionContext { ... }` construction inside
`run_daemon`'s accept loop — the one ending with `document_roots:
document_roots.clone(),` — and add directly after it:

```rust
            document_roots: document_roots.clone(),
            reminder_store: reminder_store.clone(),
```

- [ ] **Step 9: Thread it through the `ConnectionContext` struct and `handle_connection`'s destructure**

Find the `ConnectionContext` struct's last field, `document_roots:
DocumentRoots,`, and add directly after it:

```rust
    document_roots: DocumentRoots,
    /// Phase 186 — see `DaemonConfig::reminder_store`'s own doc comment.
    reminder_store: Option<crate::reminder_tool::SharedReminderStore>,
```

Find `handle_connection`'s `let ConnectionContext { ... document_roots,
...` destructure and add `reminder_store,` directly after `document_roots,`.

- [ ] **Step 10: Thread it through `handle_query`'s call site and signature**

Find the `handle_query(` call inside `handle_connection`'s `FrontendMessage
::Query` arm — the argument list ending `comfyui_base_url.as_deref(),` —
and add directly after it:

```rust
                                comfyui_base_url.as_deref(),
                                reminder_store.as_ref(),
```

Find `handle_query`'s own signature — the parameter list ending
`comfyui_base_url: Option<&str>,` right before `) -> QueryResponsePayload
{` — and add directly after it:

```rust
    comfyui_base_url: Option<&str>,
    // Phase 186 — see `DaemonConfig::reminder_store`'s own doc comment.
    reminder_store: Option<&crate::reminder_tool::SharedReminderStore>,
) -> QueryResponsePayload {
```

- [ ] **Step 11: Satisfy the two other `ConnectionContext`/`DaemonConfig` construction sites**

These are test/POC helpers, not the real daemon boot path — both take
`None` since neither builds a real reminder store.

In `run_single_connection_daemon`, find the `handle_connection
(ConnectionContext { ... document_roots: Default::default(), ...` call and
add directly after it:

```rust
        document_roots: Default::default(),
        reminder_store: None,
```

In `run_daemon_compat`, find the `run_daemon(DaemonConfig { ...
document_roots: Default::default(), ...` call and add directly after it:

```rust
        document_roots: Default::default(),
        reminder_store: None,
```

- [ ] **Step 12: Run the tests to verify they pass**

Run: `cargo test -p aivyx-channel reminders_query_response`
Expected: PASS (2 tests).

Run: `cargo build -p aivyx-channel`
Expected: compiles clean (confirms every threading site above was found
and updated — a missed site fails this build with a "missing field"
error naming the exact struct literal to fix).

- [ ] **Step 13: Add the `daemon_client::get_reminders` helper**

In `crates/aivyx-channel/src/daemon_client.rs`, find `pub async fn
get_tool_stats(` and add a new function directly after its closing `}`:

```rust
/// Phase 186 — fetch every pending reminder for the TUI Dashboard's
/// reminders panel. Mirrors `get_tool_stats`'s shape.
pub async fn get_reminders(
    socket_path: &Path,
) -> Result<Vec<crate::daemon_ipc::ReminderView>, DaemonError> {
    let payload =
        send_query(socket_path, "reminders", QueryPayload::GetReminders).await?;
    match payload {
        QueryResponsePayload::Reminders { reminders } => Ok(reminders),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected Reminders, got {other:?}"
        ))),
    }
}
```

- [ ] **Step 14: Wire the daemon boot's existing `reminder_store` into `DaemonConfig`**

In `crates/aivyx-cli/src/bin/aivyx.rs`, find the `DaemonConfig { ... }`
construction (search `run_daemon(DaemonConfig {`) and find the line
`loop_config: config_loop.clone(),`. Add directly after it:

```rust
            loop_config: config_loop.clone(),
            // Phase 186 — the reminder store this same function already
            // built above (used to wire the `remind.*` tools + spawn the
            // reminder driver) is also the `GetReminders` query's source.
            reminder_store: Some(Arc::clone(&reminder_store)),
```

- [ ] **Step 15: Run the full build + test + clippy for the touched crates**

Run: `cargo build -p aivyx-cli -p aivyx-channel -p aivyx-ipc`
Expected: compiles clean.

Run: `cargo test -p aivyx-channel -p aivyx-ipc`
Expected: all pass, including the 2 new tests.

Run: `cargo clippy -p aivyx-cli -p aivyx-channel -p aivyx-ipc --all-targets -- -D warnings`
Expected: zero warnings.

- [ ] **Step 16: Commit**

```bash
git add crates/aivyx-ipc/src/protocol.rs crates/aivyx-channel/src/daemon_server.rs crates/aivyx-channel/src/daemon_client.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat(Phase 186): GetReminders query, threaded end-to-end

Reminders had no frontend IPC surface at all before this — remind.*
was agent-tool-only. New QueryPayload::GetReminders /
QueryResponsePayload::Reminders(ReminderView) pair, a
reminders_query_response handler (mirrors fold_tool_stats/
fold_mcp_server_stats's extracted-pure-function precedent), and a new
DaemonConfig.reminder_store field threaded through
ConnectionContext/handle_query. daemon_client::get_reminders is the
TUI-facing entry point Task 2 calls."
```

---

### Task 2: Dashboard model + fetch wiring in `aivyx-tui`

**Files:**
- Modify: `crates/aivyx-tui/src/model.rs` (add `LoopStatusView` struct,
  `AppState.loop_status`/`AppState.reminders` fields, `Msg::
  LoopStatusUpdated`/`Msg::RemindersUpdated`, reducer arms, fix the
  `View` enum's stale doc comment)
- Modify: `crates/aivyx-tui/src/app.rs` (add `fetch_loop_status`,
  `fetch_reminders`, `fetch_latest_audit_page` (extracted from the
  existing `switching_to_audit` body), the `switching_to_dashboard` gate,
  and the Dashboard branch on the existing Missions poll tick)

**Interfaces:**
- Consumes: `aivyx_channel::daemon_client::{loop_status, get_reminders}`
  (the second is Task 1's new function; `loop_status` already exists,
  returns `(LoopRunState, usize, bool, bool, Option<u64>, Option<u64>,
  Option<f64>, u32)`); `aivyx_channel::daemon_ipc::ReminderView` (Task 1);
  `aivyx_channel::loop_driver::LoopRunState`.
- Produces: `crate::model::LoopStatusView` (named-field wrapper around the
  `loop_status` tuple) and the `AppState.loop_status: Option<
  LoopStatusView>` / `AppState.reminders: Vec<ReminderView>` fields — Task
  3's render code reads both directly.

- [ ] **Step 1: Write the failing model tests**

In `crates/aivyx-tui/src/model.rs`, find the `#[cfg(test)] mod tests`
block and add:

```rust
    #[test]
    fn loop_status_updated_replaces_state() {
        let s = AppState::new();
        assert!(s.loop_status.is_none());
        let view = LoopStatusView {
            state: aivyx_channel::loop_driver::LoopRunState {
                active: true,
                iteration: 3,
                max_iterations: 10,
                ..Default::default()
            },
            remaining: 2,
            armed: true,
            gate_enabled: false,
            max_run_secs: None,
            max_run_tokens: None,
            max_run_usd: None,
            max_idle_iterations: 0,
        };
        let s = update(s, Msg::LoopStatusUpdated(view.clone()));
        assert_eq!(s.loop_status, Some(view));
    }

    #[test]
    fn reminders_updated_replaces_list() {
        let s = AppState::new();
        assert!(s.reminders.is_empty());
        let reminders = vec![aivyx_channel::daemon_ipc::ReminderView {
            id: "r1".into(),
            due_unix: 100,
            message: "call mom".into(),
            notify_targets: vec![],
            created_unix: 0,
        }];
        let s = update(s, Msg::RemindersUpdated(reminders.clone()));
        assert_eq!(s.reminders, reminders);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-tui loop_status_updated_replaces_state reminders_updated_replaces_list`
Expected: FAIL to compile — `LoopStatusView`, `AppState.loop_status`,
`AppState.reminders`, `Msg::LoopStatusUpdated`, `Msg::RemindersUpdated`
don't exist yet.

- [ ] **Step 3: Add `LoopStatusView` and the two `AppState` fields**

In `crates/aivyx-tui/src/model.rs`, find `pub struct AppState {` and its
last field, `tool_stats`:

```rust
    /// POLISH_WAVES.md sub-project 8 item B — the Tools view's data,
    /// fetched once on switching into the view (no background poll,
    /// same posture as `audit_entries` before pagination).
    pub tool_stats: Vec<aivyx_channel::daemon_ipc::ToolStat>,
}
```

Replace with:

```rust
    /// POLISH_WAVES.md sub-project 8 item B — the Tools view's data,
    /// fetched once on switching into the view (no background poll,
    /// same posture as `audit_entries` before pagination).
    pub tool_stats: Vec<aivyx_channel::daemon_ipc::ToolStat>,
    /// Phase 186 — the Dashboard's loop-status panel. `None` until the
    /// first fetch (switching onto Dashboard) resolves; re-fetched on
    /// the Missions poll tick while Dashboard stays the active view (see
    /// `app.rs`'s `run_loop`) so a running loop's iteration count is
    /// visibly live, not a one-shot snapshot like Audit/Tools.
    pub loop_status: Option<LoopStatusView>,
    /// Phase 186 — the Dashboard's reminders panel, soonest-due-first
    /// (matches `ReminderStore::list`'s own order). Same fetch posture
    /// as `loop_status`.
    pub reminders: Vec<aivyx_channel::daemon_ipc::ReminderView>,
}

/// Phase 186 — a named-field wrapper around `daemon_client::
/// loop_status`'s 8-tuple response, so the Dashboard's render code
/// doesn't index into a tuple.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopStatusView {
    pub state: aivyx_channel::loop_driver::LoopRunState,
    pub remaining: usize,
    pub armed: bool,
    pub gate_enabled: bool,
    pub max_run_secs: Option<u64>,
    pub max_run_tokens: Option<u64>,
    pub max_run_usd: Option<f64>,
    pub max_idle_iterations: u32,
}
```

- [ ] **Step 4: Add the two `Msg` variants**

Find `Msg::ToolStatsUpdated(Vec<aivyx_channel::daemon_ipc::ToolStat>),`
inside the `Msg` enum and add directly after it:

```rust
    ToolStatsUpdated(Vec<aivyx_channel::daemon_ipc::ToolStat>),
    /// Phase 186 — a fresh loop-status fetch resolved.
    LoopStatusUpdated(LoopStatusView),
    /// Phase 186 — a fresh reminders fetch resolved.
    RemindersUpdated(Vec<aivyx_channel::daemon_ipc::ReminderView>),
```

- [ ] **Step 5: Add the two reducer arms**

Find `Msg::ToolStatsUpdated(tools) => { state.tool_stats = tools; }` inside
`update` and add directly after it:

```rust
        Msg::ToolStatsUpdated(tools) => {
            state.tool_stats = tools;
        }

        Msg::LoopStatusUpdated(view) => {
            state.loop_status = Some(view);
        }

        Msg::RemindersUpdated(reminders) => {
            state.reminders = reminders;
        }
```

- [ ] **Step 6: Fix the `View` enum's stale doc comment**

Find:

```rust
/// A top-level view in the TUI. `Chat` is the shipped interactive
/// surface; the others are read-only panels (live-data wiring is the
/// Phase 186 follow-on). The order is the tab order.
```

Replace with:

```rust
/// A top-level view in the TUI. `Chat` is the shipped interactive
/// surface; the others are read-only panels. Missions/Audit/Tools were
/// already live-data-wired by Phase 185 and POLISH_WAVES.md sub-project
/// 8 — Dashboard was Phase 186's one remaining stub. The order is the
/// tab order.
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p aivyx-tui loop_status_updated_replaces_state reminders_updated_replaces_list`
Expected: PASS.

- [ ] **Step 8: Commit the model changes**

```bash
git add crates/aivyx-tui/src/model.rs
git commit -m "feat(Phase 186): Dashboard AppState fields + LoopStatusView"
```

- [ ] **Step 9: Write the failing test for `should_poll_dashboard`**

`run_loop`'s real terminal/socket/`tokio::select` loop isn't practically
unit-testable, so the tick's view-gating condition is extracted as its
own pure function first, specifically so it's testable — this is the
design's own testing requirement. Add a test module at the bottom of
`crates/aivyx-tui/src/app.rs` (there isn't one yet — this is the first):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_poll_dashboard_only_when_dashboard_is_active() {
        assert!(should_poll_dashboard(View::Dashboard));
        assert!(!should_poll_dashboard(View::Chat));
        assert!(!should_poll_dashboard(View::Missions));
        assert!(!should_poll_dashboard(View::Audit));
        assert!(!should_poll_dashboard(View::Tools));
    }
}
```

- [ ] **Step 10: Run the test to verify it fails**

Run: `cargo test -p aivyx-tui should_poll_dashboard`
Expected: FAIL — `should_poll_dashboard` is not defined yet.

- [ ] **Step 11: Extract `fetch_latest_audit_page`, add the Dashboard fetch functions, and `should_poll_dashboard`**

In `crates/aivyx-tui/src/app.rs`, find `async fn fetch_tool_stats(` and add
four new functions directly after its closing `}`:

```rust
/// Phase 186 — seed the newest audit page, self-correcting the guessed
/// window against the real total. Shared by `switching_to_audit` and
/// `switching_to_dashboard` (Dashboard's audit summary shows the same
/// "newest page," just fewer lines of it) — extracted from what was
/// previously `switching_to_audit`'s own inline body so both call sites
/// share one implementation.
async fn fetch_latest_audit_page(socket_path: &Path, state: &mut AppState) {
    let guessed_from_seq = state.audit_total.saturating_sub(AUDIT_PAGE_SIZE as u64);
    fetch_audit_page(socket_path, state, guessed_from_seq).await;
    if let Some(corrected_from_seq) = audit_initial_fetch_correction(
        guessed_from_seq,
        state.audit_total,
        AUDIT_PAGE_SIZE as u64,
    ) {
        fetch_audit_page(socket_path, state, corrected_from_seq).await;
    }
}

/// Phase 186 — fetch loop status for the Dashboard's loop panel. Called
/// on switching onto Dashboard, and again on every Missions poll tick
/// while Dashboard stays the active view (see `run_loop`) so a running
/// loop's iteration count is visibly live.
async fn fetch_loop_status(socket_path: &Path, state: &mut AppState) {
    if let Ok((
        rs_state,
        remaining,
        armed,
        gate_enabled,
        max_run_secs,
        max_run_tokens,
        max_run_usd,
        max_idle_iterations,
    )) = aivyx_channel::daemon_client::loop_status(socket_path).await
    {
        apply(
            state,
            Msg::LoopStatusUpdated(crate::model::LoopStatusView {
                state: rs_state,
                remaining,
                armed,
                gate_enabled,
                max_run_secs,
                max_run_tokens,
                max_run_usd,
                max_idle_iterations,
            }),
        );
    }
}

/// Phase 186 — fetch reminders for the Dashboard's reminders panel.
/// Same refresh posture as `fetch_loop_status`.
async fn fetch_reminders(socket_path: &Path, state: &mut AppState) {
    if let Ok(reminders) = aivyx_channel::daemon_client::get_reminders(socket_path).await {
        apply(state, Msg::RemindersUpdated(reminders));
    }
}

/// Phase 186 — pure gate for the Missions-poll-tick branch below:
/// loop-status/reminders only re-fetch while Dashboard is the active
/// view. See the Step 9 test above for why this is its own function.
fn should_poll_dashboard(view: View) -> bool {
    matches!(view, View::Dashboard)
}
```

- [ ] **Step 12: Run the test to verify it passes**

Run: `cargo test -p aivyx-tui should_poll_dashboard`
Expected: PASS.

- [ ] **Step 13: Replace `switching_to_audit`'s inline body with the extracted function, and add the `switching_to_dashboard` gate**

Find this block inside `run_loop`'s `Action::Update(msg)` arm:

```rust
                let switching_to_audit = matches!(msg, Msg::SwitchView(View::Audit));
                // POLISH_WAVES.md sub-project 8 item B — same posture for
                // the Tools view: seed on switch, no background refresh.
                let switching_to_tools = matches!(msg, Msg::SwitchView(View::Tools));
                apply(state, msg);
                if switching_to_audit {
                    // The `from_seq` here is a *guess* — `state.audit_total`
                    // is whatever happened to be cached before this fetch
                    // (`0` on the session's first-ever visit, or possibly
                    // stale if the chain grew since a previous visit). Once
                    // the fetch reveals the real `total_len`, check the
                    // guess against it and, if wrong, fetch again with the
                    // corrected window — all synchronously, before control
                    // returns to the render loop, so the operator never
                    // sees the wrong page. Runs on every switch (no latch),
                    // so a chain that grew between visits self-corrects
                    // every time, not just once per session.
                    let guessed_from_seq =
                        state.audit_total.saturating_sub(AUDIT_PAGE_SIZE as u64);
                    fetch_audit_page(socket_path, state, guessed_from_seq).await;
                    if let Some(corrected_from_seq) = audit_initial_fetch_correction(
                        guessed_from_seq,
                        state.audit_total,
                        AUDIT_PAGE_SIZE as u64,
                    ) {
                        fetch_audit_page(socket_path, state, corrected_from_seq).await;
                    }
                }
                if switching_to_tools {
                    fetch_tool_stats(socket_path, state).await;
                }
```

Replace with:

```rust
                let switching_to_audit = matches!(msg, Msg::SwitchView(View::Audit));
                // POLISH_WAVES.md sub-project 8 item B — same posture for
                // the Tools view: seed on switch, no background refresh.
                let switching_to_tools = matches!(msg, Msg::SwitchView(View::Tools));
                // Phase 186 — same posture again for Dashboard: seed all
                // three of its panels on switch (loop/reminders also ride
                // the Missions poll tick below while Dashboard stays
                // active; the audit summary does not, matching Audit's
                // own static-until-paginate behavior).
                let switching_to_dashboard = matches!(msg, Msg::SwitchView(View::Dashboard));
                apply(state, msg);
                if switching_to_audit {
                    // See `fetch_latest_audit_page`'s own doc comment for
                    // the guess-then-correct rationale.
                    fetch_latest_audit_page(socket_path, state).await;
                }
                if switching_to_tools {
                    fetch_tool_stats(socket_path, state).await;
                }
                if switching_to_dashboard {
                    fetch_loop_status(socket_path, state).await;
                    fetch_reminders(socket_path, state).await;
                    fetch_latest_audit_page(socket_path, state).await;
                }
```

- [ ] **Step 14: Ride the existing Missions poll tick while Dashboard is active**

Find this block inside `run_loop`:

```rust
        let key = tokio::select! {
            k = wait_for_key() => k,
            _ = tokio::time::sleep(MISSION_POLL) => {
                poll_missions(socket_path, state).await;
                continue;
            }
        };
```

Replace with:

```rust
        let key = tokio::select! {
            k = wait_for_key() => k,
            _ = tokio::time::sleep(MISSION_POLL) => {
                poll_missions(socket_path, state).await;
                // Phase 186 — piggyback loop-status/reminders refresh on
                // the same tick, but only while Dashboard is the active
                // view (unlike Missions, which polls unconditionally) so
                // an operator parked on Dashboard sees a running loop's
                // iteration count climb without a second timer, and
                // nothing is queried while Dashboard isn't on screen.
                // Audit deliberately does not ride this tick — see
                // `switching_to_dashboard`'s own comment above.
                if should_poll_dashboard(state.view) {
                    fetch_loop_status(socket_path, state).await;
                    fetch_reminders(socket_path, state).await;
                }
                continue;
            }
        };
```

- [ ] **Step 15: Run the full `aivyx-tui` test suite**

Run: `cargo test -p aivyx-tui`
Expected: all pass (no behavior change yet for existing Audit/Tools
tests — `fetch_latest_audit_page` is a pure extraction).

- [ ] **Step 16: Clippy**

Run: `cargo clippy -p aivyx-tui --all-targets -- -D warnings`
Expected: zero warnings.

- [ ] **Step 17: Commit**

```bash
git add crates/aivyx-tui/src/app.rs
git commit -m "feat(Phase 186): Dashboard fetch wiring (switch + poll-tick)

Extracts fetch_latest_audit_page out of switching_to_audit's inline
body so Dashboard's audit summary reuses it. Adds fetch_loop_status/
fetch_reminders, fetched on switching onto Dashboard and again on
every Missions poll tick while Dashboard stays active (audit does
not ride the tick, matching its own tab's static-until-paginate
behavior)."
```

---

### Task 3: Dashboard render — real content + stale-comment cleanup

**Files:**
- Modify: `crates/aivyx-tui/src/render.rs` (rewrite `dashboard_lines`,
  fix `render_panel`'s stale doc comment, add a `format_due_offset`
  helper, update the existing Dashboard test, add new ones)

**Interfaces:**
- Consumes: `AppState.loop_status: Option<LoopStatusView>`,
  `AppState.reminders: Vec<ReminderView>`, `AppState.missions.rows: Vec<
  MissionRow>` (already live), `AppState.audit_total: u64` /
  `AppState.audit_entries: Vec<AuditEntrySummary>` (both from Task 2 /
  pre-existing).
- Produces: nothing further downstream — this is the render leaf.

- [ ] **Step 1: Write the failing render tests**

In `crates/aivyx-tui/src/render.rs`'s test module, find
`dashboard_view_renders_panel_not_chat_input` and replace it (the
`"Phase 186"` placeholder assertion no longer holds once the stub is
replaced with real content):

```rust
    #[test]
    fn dashboard_view_renders_panel_not_chat_input() {
        let backend = TestBackend::new(72, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut state = AppState::new();
        state.view = View::Dashboard;
        state.status.daemon_connected = true;
        state.status.role = Some("researcher".into());

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        // The panel renders (not the chat input line).
        assert!(text.contains("DASHBOARD"), "panel titled");
        assert!(text.contains("researcher"), "role shown in panel");
        assert!(!text.contains(" Input "), "no chat input in a panel view");
        // Tab bar still present.
        assert!(text.contains("Audit"), "tab bar lists Audit");
    }

    #[test]
    fn dashboard_shows_idle_loop_and_no_reminders_by_default() {
        let backend = TestBackend::new(72, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Dashboard;

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("idle"), "no loop_status fetched yet reads as idle/unknown");
        assert!(text.contains("none pending"), "no reminders fetched yet");
    }

    #[test]
    fn dashboard_shows_running_loop_status() {
        let backend = TestBackend::new(72, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Dashboard;
        state.loop_status = Some(crate::model::LoopStatusView {
            state: aivyx_channel::loop_driver::LoopRunState {
                active: true,
                iteration: 3,
                max_iterations: 10,
                spent_cents: 250,
                tokens_used: 4_000,
                ..Default::default()
            },
            remaining: 2,
            armed: true,
            gate_enabled: false,
            max_run_secs: None,
            max_run_tokens: None,
            max_run_usd: None,
            max_idle_iterations: 0,
        });

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("3/10"), "iteration/max shown");
        assert!(text.contains("running"), "active loop reads as running");
    }

    #[test]
    fn dashboard_shows_stalled_loop_status() {
        let backend = TestBackend::new(72, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Dashboard;
        state.loop_status = Some(crate::model::LoopStatusView {
            state: aivyx_channel::loop_driver::LoopRunState {
                active: true,
                consecutive_idle: 4,
                ..Default::default()
            },
            remaining: 0,
            armed: true,
            gate_enabled: false,
            max_run_secs: None,
            max_run_tokens: None,
            max_run_usd: None,
            max_idle_iterations: 5,
        });

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("stalled"), "consecutive_idle > 0 reads as stalled");
    }

    #[test]
    fn dashboard_shows_next_reminders_soonest_first() {
        let backend = TestBackend::new(72, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Dashboard;
        state.reminders = vec![
            aivyx_channel::daemon_ipc::ReminderView {
                id: "r1".into(),
                due_unix: 300,
                message: "call mom".into(),
                notify_targets: vec![],
                created_unix: 0,
            },
            aivyx_channel::daemon_ipc::ReminderView {
                id: "r2".into(),
                due_unix: 100,
                message: "standup".into(),
                notify_targets: vec![],
                created_unix: 0,
            },
        ];

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("2 pending"), "count shown");
        assert!(text.contains("standup"), "soonest reminder shown");
        let standup_pos = text.find("standup").unwrap();
        let call_mom_pos = text.find("call mom").unwrap();
        assert!(standup_pos < call_mom_pos, "soonest (standup, due 100) listed before due 300");
    }

    #[test]
    fn dashboard_summarizes_missions_by_phase() {
        use crate::model::{MissionPhase, MissionRow};
        let backend = TestBackend::new(72, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Dashboard;
        state.missions.rows = vec![
            MissionRow {
                id: "m1".into(),
                title: "t1".into(),
                lead: "aria".into(),
                phase: MissionPhase::Executing,
                progress: 40,
                steps: vec![],
                pending_gate: None,
            },
            MissionRow {
                id: "m2".into(),
                title: "t2".into(),
                lead: "aria".into(),
                phase: MissionPhase::Done,
                progress: 100,
                steps: vec![],
                pending_gate: None,
            },
        ];

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("1 active"), "one Executing mission counted");
        assert!(text.contains("1 done"), "one Done mission counted");
    }

    #[test]
    fn dashboard_summarizes_audit_total_and_recent() {
        use aivyx_channel::daemon_ipc::AuditEntrySummary;
        let backend = TestBackend::new(72, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Dashboard;
        state.audit_total = 42;
        state.audit_entries = vec![AuditEntrySummary {
            seq: 42,
            appended_at_unix_ms: 1_000,
            event_type: "ToolCall".into(),
            event: serde_json::json!({}),
            mac_hex: "aaa".into(),
        }];

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("42"), "audit total shown");
        assert!(text.contains("ToolCall"), "most recent event type shown");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-tui dashboard_`
Expected: FAIL — `dashboard_shows_idle_loop_and_no_reminders_by_default`
and the others don't find their expected text yet (the stub is still in
place), and the first test's `"Phase 186"` assertion is gone so that one
now only fails on the new-content assertions once you get there.

- [ ] **Step 3: Add a `format_due_offset` helper**

In `crates/aivyx-tui/src/render.rs`, find `fn kv<'a>(` and add a new
function directly after its closing `}`:

```rust
/// Phase 186 — a reminder's due time as a short relative offset. Plain
/// integer-second arithmetic (no date/time crate, matching this crate's
/// existing style — see the Global Constraints in this phase's plan).
fn format_due_offset(due_unix: i64, now_unix: i64) -> String {
    let delta = due_unix - now_unix;
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

- [ ] **Step 4: Rewrite `dashboard_lines`**

Replace the entire existing `dashboard_lines` function body with:

```rust
fn dashboard_lines(state: &AppState) -> Vec<Line<'_>> {
    let role = state.status.role.as_deref().unwrap_or("—");
    let daemon = if state.status.daemon_connected {
        Span::styled("connected ✓", fg(palette::OK))
    } else {
        Span::styled("offline ✗", fg(palette::ERR))
    };
    let status = if state.status.working {
        Span::styled("working…", fg(palette::LAV))
    } else {
        Span::styled("idle", fg(palette::FG))
    };

    let mut lines = vec![
        kv("role", Span::styled(role.to_string(), fg(palette::FG))),
        kv("daemon", daemon),
        kv("status", status),
        kv(
            "session",
            Span::styled(format!("{} lines", state.history.len()), fg(palette::FG)),
        ),
        Line::from(""),
    ];

    // --- Loop ---
    lines.push(Line::from(Span::styled("LOOP", bold(palette::AMBER))));
    match &state.loop_status {
        None => lines.push(Line::from(Span::styled("idle — not yet fetched", fg(palette::DIM)))),
        Some(ls) => {
            if ls.state.consecutive_idle > 0 && ls.state.active {
                lines.push(Line::from(Span::styled(
                    format!("stalled ({} consecutive idle)", ls.state.consecutive_idle),
                    fg(palette::ERR),
                )));
            } else if ls.state.active {
                lines.push(Line::from(Span::styled(
                    format!(
                        "running (iter {}/{}, ${:.2}, {}k tokens)",
                        ls.state.iteration,
                        ls.state.max_iterations,
                        ls.state.spent_cents as f64 / 100.0,
                        ls.state.tokens_used / 1_000,
                    ),
                    fg(palette::LAV),
                )));
            } else {
                let reason = ls.state.last_stop_reason.as_deref().unwrap_or("never run");
                lines.push(Line::from(Span::styled(format!("idle ({reason})"), fg(palette::FG))));
            }
        }
    }
    lines.push(Line::from(""));

    // --- Reminders ---
    lines.push(Line::from(Span::styled("REMINDERS", bold(palette::AMBER))));
    if state.reminders.is_empty() {
        lines.push(Line::from(Span::styled("none pending", fg(palette::DIM))));
    } else {
        lines.push(Line::from(Span::styled(
            format!("{} pending", state.reminders.len()),
            fg(palette::FG),
        )));
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        for r in state.reminders.iter().take(3) {
            lines.push(Line::from(vec![
                Span::styled(format!("{:<12}", format_due_offset(r.due_unix, now_unix)), fg(palette::DIMMER)),
                Span::styled(r.message.clone(), fg(palette::FG)),
            ]));
        }
    }
    lines.push(Line::from(""));

    // --- Missions ---
    lines.push(Line::from(Span::styled("MISSIONS", bold(palette::AMBER))));
    if state.missions.rows.is_empty() {
        lines.push(Line::from(Span::styled("none", fg(palette::DIM))));
    } else {
        let active = state
            .missions
            .rows
            .iter()
            .filter(|m| !matches!(m.phase, crate::model::MissionPhase::Done))
            .count();
        let done = state.missions.rows.len() - active;
        lines.push(Line::from(Span::styled(
            format!("{active} active, {done} done"),
            fg(palette::FG),
        )));
    }
    lines.push(Line::from(""));

    // --- Audit ---
    lines.push(Line::from(Span::styled("AUDIT", bold(palette::AMBER))));
    lines.push(Line::from(Span::styled(
        format!("{} total events", state.audit_total),
        fg(palette::FG),
    )));
    for e in state.audit_entries.iter().rev().take(3) {
        lines.push(Line::from(vec![
            Span::styled(format!("#{} ", e.seq), fg(palette::DIMMER)),
            Span::styled(e.event_type.clone(), fg(palette::AMBER)),
        ]));
    }

    lines
}
```

- [ ] **Step 5: Fix `render_panel`'s stale doc comment**

Find:

```rust
/// Render the active read-only panel (Dashboard / Audit / Tools). These
/// show their frame + the state the model already holds. Audit is wired
/// to live daemon data (`/classic` retirement, Task 5) and scrolls via
/// [`audit_scroll_offset`] (the Audit-page counterpart of
/// [`chat_scroll_offset`], which `render_chat` uses for the same
/// purpose); Dashboard's mission/loop/reminders detail and Tools'
/// capability/call-stats detail remain the Phase 186 follow-on.
```

Replace with:

```rust
/// Render the active read-only panel (Dashboard / Audit / Tools). These
/// show their frame + the state the model already holds. Audit is wired
/// to live daemon data (`/classic` retirement, Task 5) and scrolls via
/// [`audit_scroll_offset`] (the Audit-page counterpart of
/// [`chat_scroll_offset`], which `render_chat` uses for the same
/// purpose); Tools' capability/call-stats detail shipped in
/// POLISH_WAVES.md sub-project 8. Dashboard's loop/reminders/missions/
/// audit summaries (`dashboard_lines`) shipped in Phase 186 — the last
/// of this comment's own original follow-on list.
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p aivyx-tui dashboard_`
Expected: PASS (7 tests: the updated existing one + 6 new).

- [ ] **Step 7: Run the full `aivyx-tui` suite + clippy**

Run: `cargo test -p aivyx-tui`
Expected: all pass.

Run: `cargo clippy -p aivyx-tui --all-targets -- -D warnings`
Expected: zero warnings.

- [ ] **Step 8: Confirm no stale "Phase 186 follow-on" text remains**

Run: `grep -rn "Phase 186" crates/aivyx-tui/src/`
Expected: only forward-looking/historical references remain (e.g. "shipped
in Phase 186", doc comments on the new code itself) — no more "follow-on"/
"land in Phase 186" placeholder language.

- [ ] **Step 9: Commit**

```bash
git add crates/aivyx-tui/src/render.rs
git commit -m "feat(Phase 186): real Dashboard content — loop/reminders/missions/audit

Replaces the dashboard_lines stub with four sections: loop status
(idle/running/stalled derived from LoopRunState), next 3 reminders
soonest-first, a mission phase count, and an audit total + 3 most
recent entries. Also fixes render_panel's stale doc comment (still
named Tools' call-stats as a Phase 186 follow-on — sub-project 8
shipped that)."
```

---

### Task 4: Final sweep

**Files:** none new — verification only, plus the phase-closing doc note.

- [ ] **Step 1: Full default-members build, test, and clippy sweep**

Run: `cargo build`
Expected: compiles clean.

Run: `cargo test`
Expected: all pass, zero failures (this repo's default-members set —
matches `CLAUDE.md`'s own build/test convention; do not add `--workspace`,
see this plan's Global Constraints).

Run: `cargo clippy --all-targets -- -D warnings`
Expected: zero warnings.

- [ ] **Step 2: Confirm the full `GetReminders` round trip compiles across every crate that touches it**

Run: `cargo build -p aivyx-ipc -p aivyx-channel -p aivyx-cli -p aivyx-tui`
Expected: compiles clean (this is a redundant re-check of Task 1–3's own
per-task builds, run once more here as the whole-branch confirmation).

- [ ] **Step 3: Confirm no leftover Dashboard placeholder text anywhere in the crate**

Run: `grep -rn "land in Phase 186\|wired to the live daemon state the IPC already serves" crates/aivyx-tui/src/`
Expected: no matches (the exact placeholder sentence from the original
stub is fully gone).

- [ ] **Step 4: Manual read-through of the final diff**

Run: `git diff main --stat` (from the feature branch, once one exists —
see the finishing-a-development-branch skill for how this plan's commits
get merged)
Expected: only the files this plan named are touched.

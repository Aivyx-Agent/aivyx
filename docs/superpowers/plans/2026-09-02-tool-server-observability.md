# Tool/Server Call-Stat Observability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** POLISH_WAVES.md sub-project 8 — a real per-MCP-server health signal on the Studio's MCP panel (item C), and a working `aivyx-tui` Tools view backed by the already-shipped tool-observability query (item B).

**Architecture:** Item C adds one new audit-chain aggregation function (`fold_mcp_server_stats`, a sibling to the existing `fold_tool_stats`) that recovers the MCP server name from the `mcp.call:<server>:<tool>` scope's qualifier — a dimension the existing aggregator collapses away — behind a new `GetMcpServerCallStats` query, surfaced as a new health chip on `McpServerCard`. Item B needs **no new backend at all**: it wires the already-shipped `QueryPayload::GetToolStats` (already backing the `aivyx tools` CLI) into `aivyx-tui`'s placeholder `View::Tools`, following the exact pattern `View::Audit` already established in this same TUI.

**Tech Stack:** Rust, the existing `QueryPayload`/`QueryResponsePayload` IPC protocol, ratatui (TUI), Dioxus 0.6/wasm32 (Studio).

## Global Constraints

- `fold_tool_stats` (`crates/aivyx-channel/src/daemon_server.rs`) stays untouched — it already does its job for every non-MCP tool. The MCP-specific dimension is a new sibling function, not a retrofit.
- The MCP server name lives in `scope_used.qualifier()` as `<server>:<tool>` — split from the **right** (`rsplit_once(':')`), not the left, so a server name that itself contains a colon (no validation forbids this in `write_mcp_server_section` today) still recovers correctly, since the tool name (the last segment) comes from the connected server's own tool definitions and is the part guaranteed colon-free by the qualifier's own construction site (`crates/aivyx-mcp/src/proxy.rs`'s `format!("mcp.call:{server_name}:{tool_name}")`).
- `GetMcpServerCallStats` is a **new, separate query** from `GetMcpStatus` — `McpServerStatusView`/`GetMcpStatus` documents itself as a boot-time file snapshot ("Snapshot semantics are 'as of the last daemon start' — a file, not live daemon memory," `crates/aivyx-channel/src/mcp_status.rs`); do not add rolling/live fields to that type or query.
- The Studio's new health chip is **additive**, not a replacement: `McpServerCard`'s existing `connected`/`failed` pill and `error`/`stderr_tail` rendering are unchanged. A server can show `connected` (it answered the boot-time handshake) alongside a red call-stat chip (its tools have been failing since) — that combination is the exact finding this plan exists to surface.
- Chip palette: reuse the existing 3-tier classes already in `crates/aivyx-web/src/main.rs` — `chip sage` (healthy), `chip amber` (some failures), `chip error` (majority failures), bare `chip` (no recent activity/neutral). No new CSS.
- Health-chip window is a fixed 24h (`window_secs: Some(86_400)`, matching `[proactive]`'s own `DEFAULT_PROACTIVE_WINDOW_SECS`) — no operator-configurable picker (YAGNI).
- TUI: `aivyx-tui`'s `View::Tools` reuses the **already-shipped** `crate::daemon_client::get_tool_stats(socket_path, window_secs)` (used by the `aivyx tools` CLI) — no new `daemon_client` wrapper function.
- TUI rendering follows `View::Audit`'s own established shape exactly: a flat `Vec<Line>` inside a bordered `Paragraph` (not a `ratatui::widgets::Table` — this codebase's convention for these panels is styled lines, not a literal table widget), with the SAME shared `state.scroll` counter and `audit_scroll_offset` helper Audit already uses (PageUp/PageDn/Up/Down are already view-agnostic keys in `event.rs`; only Left/Right pagination is Audit-specific).
- `Msg::ScrollUp`'s clamp match (`crates/aivyx-tui/src/model.rs:540`-543) must gain a `View::Tools => state.tool_stats.len()` arm — its own doc comment states the rule explicitly ("its ceiling must match whichever view is currently reading it"); leaving `View::Tools` in the `_ => state.history.len()` fallback would clamp scrolling to the wrong length.
- Full sweep before merge: `cargo clippy --workspace --exclude aivyx-desktop --all-targets -- -D warnings` and `cargo test --workspace --exclude aivyx-desktop` (fall back to the `default-members`-only invocations plus explicit `-p aivyx-web --target wasm32-unknown-unknown` if the exclusion flag doesn't apply cleanly — this environment has needed that fallback before), plus a `dist/` rebuild in the final task.

---

## Task 1: `fold_mcp_server_stats` aggregation + `GetMcpServerCallStats` query

**Files:**
- Modify: `crates/aivyx-ipc/src/protocol.rs`
- Modify: `crates/aivyx-channel/src/daemon_server.rs`

**Interfaces:**
- Consumes: nothing new — `aivyx_audit::SignedEntry`, `aivyx_audit::AuditEvent::ToolCall`, `aivyx_core::ToolOutcomeSummary`, and the `audit_log`/`config_toml_path`-style variables already in scope inside `handle_query` (same as every existing arm, e.g. `QueryPayload::GetToolStats`).
- Produces: `pub struct McpServerCallStats { pub server_name: String, pub calls: u64, pub outcomes: BTreeMap<String, u64>, pub total_duration_ms: u64 }` (in `aivyx-ipc`), `QueryPayload::GetMcpServerCallStats { window_secs: Option<u64> }`, `QueryResponsePayload::McpServerCallStats { servers: Vec<McpServerCallStats> }` — Task 2 sends/receives these.

- [ ] **Step 1: Add the wire type**

In `crates/aivyx-ipc/src/protocol.rs`, add right after `pub struct ToolStat { ... }` (search for that struct to find the spot):

```rust
/// POLISH_WAVES.md sub-project 8 item C — per-MCP-server call
/// statistics, derived from the audit chain the same way [`ToolStat`]
/// is, but grouped by MCP server name (recovered from the `mcp.call`
/// scope's qualifier, `<server>:<tool>`) instead of by capability
/// base. Closes the gap where `ToolStat`'s aggregation — keyed on
/// `Scope::base()` — collapses every configured `[[mcp_server]]`'s
/// tool calls into one shared `"mcp.call"` row, making it impossible
/// to tell which server is actually failing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerCallStats {
    pub server_name: String,
    /// Total `AuditEvent::ToolCall` events for this server within the
    /// requested window.
    pub calls: u64,
    /// Per-outcome counts, keyed by the same stable outcome labels
    /// `ToolStat::outcomes` uses (`completed`, `failed`, `denied`,
    /// `not_in_role`, `requires_escalation`, `rate_limited`). A key is
    /// absent when its count is zero.
    pub outcomes: std::collections::BTreeMap<String, u64>,
    /// Total wall-clock duration across all `calls`, in milliseconds.
    pub total_duration_ms: u64,
}
```

- [ ] **Step 2: Add the `QueryPayload` variant**

In the same file, insert right before the closing `}` of `pub enum QueryPayload` (the enum ends right after `SetSlackConfig { ... },` — search for that variant if this plan runs before any other plan has since extended the enum further; in that case, add these right after whatever is currently the last variant):

```rust
    /// POLISH_WAVES.md sub-project 8 item C — read-only per-MCP-server
    /// observability query, closing the gap [`QueryPayload::
    /// GetToolStats`] leaves for MCP-bridged tools (which all share
    /// the single `"mcp.call"` scope base). `window_secs = None`
    /// scopes the answer to the whole audit chain, matching
    /// `GetToolStats`'s own convention. Distinct from [`QueryPayload::
    /// GetMcpStatus`], which is a boot-time file snapshot, not live
    /// audit-derived data — do not merge these two queries.
    GetMcpServerCallStats {
        #[serde(default)]
        window_secs: Option<u64>,
    },
```

- [ ] **Step 3: Add the `QueryResponsePayload` variant**

In the same file, insert right before the closing `}` of `pub enum QueryResponsePayload`:

```rust
    /// Response to [`QueryPayload::GetMcpServerCallStats`].
    McpServerCallStats { servers: Vec<McpServerCallStats> },
```

- [ ] **Step 4: Add `fold_mcp_server_stats`**

In `crates/aivyx-channel/src/daemon_server.rs`, find `fn fold_tool_stats` (search for it) and add this new function right after its closing `}`:

```rust
/// POLISH_WAVES.md sub-project 8 item C — fold `mcp.call`-scoped audit
/// `ToolCall` events into per-MCP-server statistics. Shares
/// `fold_tool_stats`'s own audit-walking/cutoff-window shape, but
/// groups by the server name recovered from the scope's qualifier
/// (`mcp.call:<server>:<tool>`) instead of by `scope_used.base()`
/// (which is the literal string `"mcp.call"` for every MCP-bridged
/// tool call, regardless of server — the exact gap this function
/// closes). No registry join (unlike `fold_tool_stats`, which joins
/// against `tool_descriptors`): a server with zero calls in the
/// window simply has no row here, and the caller (the Studio's
/// `McpPanel`) joins this list against its own already-fetched
/// `GetMcpStatus` server list client-side to render a "no recent
/// activity" state for a configured-but-unused server.
fn fold_mcp_server_stats(
    entries: &[aivyx_audit::SignedEntry],
    cutoff: Option<std::time::SystemTime>,
) -> Vec<crate::daemon_ipc::McpServerCallStats> {
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Acc {
        calls: u64,
        outcomes: BTreeMap<String, u64>,
        total_duration_ms: u64,
    }
    let mut acc: BTreeMap<String, Acc> = BTreeMap::new();

    for entry in entries {
        if let Some(cut) = cutoff {
            if entry.appended_at < cut {
                continue;
            }
        }
        let aivyx_audit::AuditEvent::ToolCall {
            scope_used,
            outcome,
            duration,
            ..
        } = &entry.event
        else {
            continue;
        };
        if scope_used.base() != "mcp.call" {
            continue;
        }
        let Some(qualifier) = scope_used.qualifier() else {
            continue;
        };
        // The qualifier is "<server>:<tool>" (proxy.rs's own
        // construction: `format!("mcp.call:{server_name}:{tool_name}")`).
        // Split from the RIGHT so a server name that itself contains a
        // colon (no validation forbids this today) still recovers
        // correctly, as long as the tool name segment (the last one)
        // has no colon of its own — guaranteed by that construction site.
        let Some((server_name, _tool_name)) = qualifier.rsplit_once(':') else {
            continue;
        };
        let a = acc.entry(server_name.to_string()).or_default();
        a.calls += 1;
        a.total_duration_ms += duration.as_millis() as u64;
        let label = match outcome {
            aivyx_core::ToolOutcomeSummary::Completed { .. } => "completed",
            aivyx_core::ToolOutcomeSummary::Denied => "denied",
            aivyx_core::ToolOutcomeSummary::NotInRole => "not_in_role",
            aivyx_core::ToolOutcomeSummary::RateLimited => "rate_limited",
            aivyx_core::ToolOutcomeSummary::RequiresEscalation => "requires_escalation",
            aivyx_core::ToolOutcomeSummary::Failed => "failed",
        };
        *a.outcomes.entry(label.to_string()).or_insert(0) += 1;
    }

    let mut servers: Vec<crate::daemon_ipc::McpServerCallStats> = acc
        .into_iter()
        .map(|(server_name, a)| crate::daemon_ipc::McpServerCallStats {
            server_name,
            calls: a.calls,
            outcomes: a.outcomes,
            total_duration_ms: a.total_duration_ms,
        })
        .collect();
    servers.sort_by(|x, y| y.calls.cmp(&x.calls).then_with(|| x.server_name.cmp(&y.server_name)));
    servers
}
```

- [ ] **Step 5: Add the daemon handler**

In `handle_query`, find the `QueryPayload::GetToolStats { window_secs } => { ... }` arm (search for it) and add this new arm right after its closing `}`:

```rust
        QueryPayload::GetMcpServerCallStats { window_secs } => {
            let Some(log) = audit_log else {
                return QueryResponsePayload::QueryError {
                    code: "no_audit_log".into(),
                    message: "daemon has no audit log configured".into(),
                };
            };
            let cutoff = window_secs.and_then(|secs| {
                std::time::SystemTime::now().checked_sub(std::time::Duration::from_secs(secs))
            });
            match log.entries_range(0, log.len()) {
                Ok(rows) => QueryResponsePayload::McpServerCallStats {
                    servers: fold_mcp_server_stats(&rows, cutoff),
                },
                Err(e) => QueryResponsePayload::QueryError {
                    code: "mcp_server_call_stats_failed".into(),
                    message: e.to_string(),
                },
            }
        }
```

- [ ] **Step 6: Write the tests**

**Reuse the existing fixture helpers, don't write new ones.** This test module already has `fn tc_entry(seq: u64, scope: &str, outcome: aivyx_core::ToolOutcomeSummary, duration_ms: u64, appended_at: std::time::SystemTime) -> aivyx_audit::SignedEntry` and `fn completed_outcome() -> aivyx_core::ToolOutcomeSummary` (search for `// ---- Phase 102: fold_tool_stats` to find them) — built for `fold_tool_stats`'s own tests, and `tc_entry` already takes a full scope *string*, so it works verbatim for an `mcp.call:<server>:<tool>` scope with no changes needed. (`SignedEntry`'s real fields, for reference, are `seq`, `appended_at`, `event`, `mac: [u8; 32]`, and `prev_mac: [u8; 32]` — `tc_entry` already fills both MAC fields with `[0u8; 32]`, which is fine for these tests since `fold_mcp_server_stats` never reads either.)

```rust
    #[test]
    fn fold_mcp_server_stats_buckets_by_server_not_by_shared_base() {
        let now = std::time::SystemTime::now();
        let entries = vec![
            tc_entry(0, "mcp.call:comfyui:generate_image", completed_outcome(), 10, now),
            tc_entry(
                1,
                "mcp.call:duckduckgo-search:search",
                aivyx_core::ToolOutcomeSummary::Failed,
                10,
                now,
            ),
            tc_entry(
                2,
                "mcp.call:duckduckgo-search:search",
                aivyx_core::ToolOutcomeSummary::Failed,
                10,
                now,
            ),
        ];
        let servers = fold_mcp_server_stats(&entries, None);
        assert_eq!(servers.len(), 2, "two distinct servers, not one shared mcp.call bucket");
        let ddg = servers.iter().find(|s| s.server_name == "duckduckgo-search").unwrap();
        assert_eq!(ddg.calls, 2);
        assert_eq!(ddg.outcomes.get("failed"), Some(&2));
        let comfy = servers.iter().find(|s| s.server_name == "comfyui").unwrap();
        assert_eq!(comfy.calls, 1);
        assert_eq!(comfy.outcomes.get("completed"), Some(&1));
    }

    #[test]
    fn fold_mcp_server_stats_ignores_non_mcp_tool_calls() {
        let now = std::time::SystemTime::now();
        let entries = vec![tc_entry(0, "fs.read", completed_outcome(), 5, now)];
        let servers = fold_mcp_server_stats(&entries, None);
        assert!(servers.is_empty(), "a non-mcp.call entry must not produce a row");
    }

    #[test]
    fn fold_mcp_server_stats_respects_the_cutoff() {
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        let recent = std::time::SystemTime::now();
        let entries = vec![
            tc_entry(0, "mcp.call:comfyui:generate_image", completed_outcome(), 10, old),
            tc_entry(1, "mcp.call:comfyui:generate_image", completed_outcome(), 10, recent),
        ];
        let cutoff = recent - std::time::Duration::from_secs(60);
        let servers = fold_mcp_server_stats(&entries, Some(cutoff));
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].calls, 1, "the entry before cutoff must be excluded");
    }

    #[test]
    fn fold_mcp_server_stats_splits_a_colon_containing_server_name_from_the_right() {
        let now = std::time::SystemTime::now();
        // A server name with a colon in it (no validation forbids this
        // today) — the tool name is guaranteed colon-free by the
        // qualifier's own construction site, so splitting from the
        // right must still recover the full server name correctly.
        let entries = vec![tc_entry(0, "mcp.call:my:weird:server:generate", completed_outcome(), 1, now)];
        let servers = fold_mcp_server_stats(&entries, None);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].server_name, "my:weird:server");
    }
```

Also add a protocol round-trip test in `crates/aivyx-ipc/src/protocol.rs`'s `#[cfg(test)] mod tests` block (search for an existing `_round_trips_over_json` test for the pattern):

```rust
    #[test]
    fn mcp_server_call_stats_round_trips_over_json() {
        let mut outcomes = std::collections::BTreeMap::new();
        outcomes.insert("completed".to_string(), 3u64);
        outcomes.insert("failed".to_string(), 1u64);
        let stats = McpServerCallStats {
            server_name: "comfyui".to_string(),
            calls: 4,
            outcomes,
            total_duration_ms: 400,
        };
        let json = serde_json::to_string(&stats).unwrap();
        let back: McpServerCallStats = serde_json::from_str(&json).unwrap();
        assert_eq!(stats, back);
    }

    #[test]
    fn get_mcp_server_call_stats_round_trips_over_json() {
        let msg = QueryPayload::GetMcpServerCallStats { window_secs: Some(86_400) };
        let json = serde_json::to_string(&msg).unwrap();
        let back: QueryPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }
```

- [ ] **Step 7: Run the tests**

Run: `cargo test -p aivyx-channel fold_mcp_server_stats -- --nocapture` and `cargo test -p aivyx-ipc mcp_server_call_stats -- --nocapture` and `cargo test -p aivyx-ipc get_mcp_server_call_stats -- --nocapture`.
Expected: all new tests pass.

- [ ] **Step 8: Run the full crate suites and clippy**

Run: `cargo test -p aivyx-ipc -p aivyx-channel` (confirm nothing else broke) and `cargo clippy -p aivyx-ipc -p aivyx-channel --all-targets -- -D warnings`.
Expected: all pass, zero warnings.

- [ ] **Step 9: Commit**

```bash
git add crates/aivyx-ipc/src/protocol.rs crates/aivyx-channel/src/daemon_server.rs
git commit -m "feat(ipc,channel): per-MCP-server call-stat aggregation

fold_mcp_server_stats + GetMcpServerCallStats/McpServerCallStats -- a
sibling to the existing fold_tool_stats/GetToolStats (Phase 102), but
grouped by MCP server name (recovered from the mcp.call:<server>:<tool>
scope's qualifier) instead of by Scope::base(), which collapses every
MCP server's tool calls into one shared \"mcp.call\" row today. A new,
separate query from GetMcpStatus (a documented boot-time file snapshot),
not a change to it.

POLISH_WAVES.md sub-project 8, Task 1.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 2: Studio — `McpServerCard` health chip

**Files:**
- Modify: `crates/aivyx-web/src/main.rs`

**Interfaces:**
- Consumes: Task 1's `QueryPayload::GetMcpServerCallStats`, `QueryResponsePayload::McpServerCallStats`, `McpServerCallStats`.
- Produces: nothing further tasks depend on — this is the UI leaf for item C.

- [ ] **Step 1: Import the new wire type**

Extend the existing `use aivyx_ipc::protocol::{ ... };` block (near the top of the file) to add `McpServerCallStats` alphabetically (next to `McpServerConfigView`/`McpServerStatusView`).

- [ ] **Step 2: Add the `call_stats` field to `McpState`**

Find `struct McpState { ... }` (search for `struct McpState`) and add a new field right after `configs: Vec<McpServerConfigView>,`:

```rust
    /// POLISH_WAVES.md sub-project 8 item C — per-server call stats
    /// from `GetMcpServerCallStats`, distinct from `servers` above
    /// (`GetMcpStatus`'s boot-time snapshot). Joined against `servers`
    /// by `server_name == name` when rendering `McpServerCard`.
    call_stats: Vec<McpServerCallStats>,
```

(`McpState` already derives `Default`, so this defaults to an empty `Vec` — no other struct change needed.)

- [ ] **Step 3: Add the query builder**

Find `fn mcp_query() -> FrontendMessage { ... }` (search for it) and add right after it:

```rust
/// POLISH_WAVES.md sub-project 8 item C — the rolling per-server
/// health query, fixed to a 24h window (matching `[proactive]`'s own
/// `DEFAULT_PROACTIVE_WINDOW_SECS` — no operator-configurable picker,
/// YAGNI). Shares the `"mc-mcp-status"` id with `mcp_query()` — both
/// are read-only status fetches for the same panel, and neither is
/// expected to error under normal operation (only `GetMcpServerCall
/// Stats`'s "no_audit_log" case would, which is as rare as `GetMcpStatus`
/// itself failing).
fn mcp_server_call_stats_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-mcp-status".to_string(),
        payload: QueryPayload::GetMcpServerCallStats { window_secs: Some(86_400) },
    }
}
```

- [ ] **Step 4: Extend `McpPanel`'s data loading**

In `fn McpPanel() -> Element { ... }` (search for it), change the `use_future` from:

```rust
    use_future(move || async move {
        ws.send(mcp_query());
        ws.send(mcp_server_configs_query());
    });
```

to:

```rust
    use_future(move || async move {
        ws.send(mcp_query());
        ws.send(mcp_server_configs_query());
        ws.send(mcp_server_call_stats_query());
    });
```

And the "Refresh" button's `onclick` from:

```rust
                    onclick: move |_| { ws.send(mcp_query()); ws.send(mcp_server_configs_query()); },
```

to:

```rust
                    onclick: move |_| {
                        ws.send(mcp_query());
                        ws.send(mcp_server_configs_query());
                        ws.send(mcp_server_call_stats_query());
                    },
```

- [ ] **Step 5: Join call stats onto each `McpServerCard` call site**

In the same function, find:

```rust
                div { class: "mcp-grid",
                    for sv in m.servers.iter() {
                        { rsx! { McpServerCard { key: "{sv.name}", view: sv.clone() } } }
                    }
                }
```

and change it to look up the matching stats row by name before rendering:

```rust
                div { class: "mcp-grid",
                    for sv in m.servers.iter() {
                        {
                            let stats = m.call_stats.iter().find(|s| s.server_name == sv.name).cloned();
                            rsx! { McpServerCard { key: "{sv.name}", view: sv.clone(), call_stats: stats } }
                        }
                    }
                }
```

- [ ] **Step 6: Add the pure chip-selection helper**

Find `fn McpServerCard(view: McpServerStatusView) -> Element { ... }` (search for it) and add this new function right before it:

```rust
/// POLISH_WAVES.md sub-project 8 item C — the health-chip class + label
/// for one server's rolling call stats. A pure function (no `Element`,
/// no context) so it's directly unit-testable, mirroring this file's
/// existing `phase_class`/`eff_class`-style small helpers.
///
/// `stats: None` means no `mcp.call` audit entries for this server in
/// the window — a configured-but-unused server is not itself unhealthy,
/// so this renders neutral, not amber/red. `"failed"` and `"denied"`
/// outcomes both count toward the unhealthy tally: a denial is still a
/// call that didn't do what the operator configured it to do.
fn mcp_health_chip(stats: Option<&McpServerCallStats>) -> (&'static str, String) {
    let Some(s) = stats else {
        return ("chip", "no recent activity".to_string());
    };
    let bad = s.outcomes.get("failed").copied().unwrap_or(0)
        + s.outcomes.get("denied").copied().unwrap_or(0);
    let ok = s.calls.saturating_sub(bad);
    if bad == 0 {
        ("chip sage", format!("{ok} ok"))
    } else if bad.saturating_mul(2) > s.calls {
        ("chip error", format!("{ok} ok / {bad} failed"))
    } else {
        ("chip amber", format!("{ok} ok / {bad} failed"))
    }
}
```

- [ ] **Step 7: Wire the chip into `McpServerCard`**

Change the component's signature and body. Find:

```rust
#[component]
fn McpServerCard(view: McpServerStatusView) -> Element {
    let (pill_class, pill_label) = if view.connected {
        ("chip sage", "connected")
    } else {
        ("chip error", "failed")
    };
    rsx! {
        div { class: "glass-card mcp-card",
            div { class: "mcp-card-head",
                span { class: "mcp-name", "{view.name}" }
                span { class: "label-tech", "{view.transport}" }
                span { class: pill_class, "{pill_label}" }
            }
```

and change it to:

```rust
#[component]
fn McpServerCard(view: McpServerStatusView, call_stats: Option<McpServerCallStats>) -> Element {
    let (pill_class, pill_label) = if view.connected {
        ("chip sage", "connected")
    } else {
        ("chip error", "failed")
    };
    // POLISH_WAVES.md sub-project 8 item C — additive to the boot-time
    // pill above, not a replacement: a server can be `connected` (it
    // answered the startup handshake) while this chip is red (its
    // tools have been failing since) -- that combination is the exact
    // finding this item exists to surface.
    let (health_class, health_label) = mcp_health_chip(call_stats.as_ref());
    rsx! {
        div { class: "glass-card mcp-card",
            div { class: "mcp-card-head",
                span { class: "mcp-name", "{view.name}" }
                span { class: "label-tech", "{view.transport}" }
                span { class: pill_class, "{pill_label}" }
                span { class: health_class, title: "last 24h", "{health_label}" }
            }
```

(Everything below the `div { class: "mcp-card-head", ... }` block — the `if view.connected { ... } else { ... }` tool-count/error/stderr rendering — is unchanged; only add the new `span` line inside `mcp-card-head` and update the function signature/the two new `let` bindings above it.)

- [ ] **Step 8: Wire the `read_task` response handler**

In `async fn read_task(...)`, find the existing `DaemonEnvelope::QueryResponse { payload: QueryResponsePayload::GetMcpStatus { captured_unix, servers }, .. } => { ... }` arm (search for `QueryResponsePayload::GetMcpStatus`) and add a new arm right after its closing `}`:

```rust
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::McpServerCallStats { servers },
                    ..
                } => {
                    // POLISH_WAVES.md sub-project 8 item C.
                    mcp.write().call_stats = servers;
                }
```

- [ ] **Step 9: Write the pure-function unit tests**

This file's convention (confirmed by `grep -n "^#\[cfg(test)\]" crates/aivyx-web/src/main.rs`) is several small test modules colocated near the code they cover, not one big block at the end. Add a new one right after `McpServerCard`'s closing `}`, with all 5 tests below inside it:

```rust
#[cfg(test)]
mod mcp_health_chip_tests {
    use super::*;

    #[test]
    fn mcp_health_chip_no_stats_is_neutral() {
        let (class, label) = mcp_health_chip(None);
        assert_eq!(class, "chip");
        assert_eq!(label, "no recent activity");
    }

    #[test]
    fn mcp_health_chip_all_ok_is_sage() {
        let mut outcomes = std::collections::BTreeMap::new();
        outcomes.insert("completed".to_string(), 5u64);
        let stats = McpServerCallStats {
            server_name: "comfyui".to_string(),
            calls: 5,
            outcomes,
            total_duration_ms: 500,
        };
        let (class, label) = mcp_health_chip(Some(&stats));
        assert_eq!(class, "chip sage");
        assert_eq!(label, "5 ok");
    }

    #[test]
    fn mcp_health_chip_minority_failures_is_amber() {
        let mut outcomes = std::collections::BTreeMap::new();
        outcomes.insert("completed".to_string(), 8u64);
        outcomes.insert("failed".to_string(), 2u64);
        let stats = McpServerCallStats {
            server_name: "duckduckgo-search".to_string(),
            calls: 10,
            outcomes,
            total_duration_ms: 1000,
        };
        let (class, label) = mcp_health_chip(Some(&stats));
        assert_eq!(class, "chip amber");
        assert_eq!(label, "8 ok / 2 failed");
    }

    #[test]
    fn mcp_health_chip_majority_failures_is_error() {
        let mut outcomes = std::collections::BTreeMap::new();
        outcomes.insert("completed".to_string(), 2u64);
        outcomes.insert("failed".to_string(), 8u64);
        let stats = McpServerCallStats {
            server_name: "duckduckgo-search".to_string(),
            calls: 10,
            outcomes,
            total_duration_ms: 1000,
        };
        let (class, label) = mcp_health_chip(Some(&stats));
        assert_eq!(class, "chip error");
        assert_eq!(label, "2 ok / 8 failed");
    }

    #[test]
    fn mcp_health_chip_counts_denied_as_unhealthy_too() {
        let mut outcomes = std::collections::BTreeMap::new();
        outcomes.insert("completed".to_string(), 3u64);
        outcomes.insert("denied".to_string(), 1u64);
        let stats = McpServerCallStats {
            server_name: "comfyui".to_string(),
            calls: 4,
            outcomes,
            total_duration_ms: 400,
        };
        let (class, label) = mcp_health_chip(Some(&stats));
        assert_eq!(class, "chip amber");
        assert_eq!(label, "3 ok / 1 failed");
    }
}
```

- [ ] **Step 10: Run the tests**

Run: `cargo test -p aivyx-web mcp_health_chip -- --nocapture` (plain `cargo test -p aivyx-web`, never `--target wasm32-unknown-unknown` — this crate's `#[cfg(test)]` blocks run natively).
Expected: all 5 new tests pass.

- [ ] **Step 11: Compile-check and clippy (wasm target)**

Run (needs the wasm32 toolchain on `PATH`):

```bash
PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH" cargo build -p aivyx-web --target wasm32-unknown-unknown
PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH" cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings
```

Expected: both clean, zero warnings.

- [ ] **Step 12: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "feat(web): MCP per-server rolling health chip

McpServerCard gains a second chip (health, distinct from the existing
connected/failed pill) sourced from the new GetMcpServerCallStats query
-- reuses the file's existing chip sage/amber/error palette, a fixed
24h window, additive to the existing boot-time status, not a replacement.
mcp_health_chip is a pure, directly-unit-tested function.

POLISH_WAVES.md sub-project 8, Task 2.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 3: `aivyx-tui`'s `View::Tools`

**Files:**
- Modify: `crates/aivyx-tui/src/model.rs`
- Modify: `crates/aivyx-tui/src/app.rs`
- Modify: `crates/aivyx-tui/src/render.rs`

**Interfaces:**
- Consumes: the already-shipped `aivyx_channel::daemon_client::get_tool_stats(socket_path: &Path, window_secs: Option<u64>) -> Result<Vec<ToolStat>, DaemonError>` and `aivyx_channel::daemon_ipc::ToolStat` — no new backend, this task only adds a TUI-side consumer.
- Produces: nothing further tasks depend on — this is item B's complete implementation, independent of Tasks 1-2.

- [ ] **Step 1: Add `Msg::ToolStatsUpdated` and the `tool_stats` field**

In `crates/aivyx-tui/src/model.rs`, find `pub struct AppState { ... }` (search for it) and add a new field right after `pub audit_total: u64,`:

```rust
    /// POLISH_WAVES.md sub-project 8 item B — the Tools view's data,
    /// fetched once on switching into the view (no background poll,
    /// same posture as `audit_entries` before pagination).
    pub tool_stats: Vec<aivyx_channel::daemon_ipc::ToolStat>,
```

Find `pub enum Msg { ... }` and add a new variant right after the existing `AuditUpdated { entries: Vec<AuditEntrySummary>, total_len: u64 },` variant (search for `AuditUpdated` to find its exact declaration and match its style):

```rust
    /// POLISH_WAVES.md sub-project 8 item B — a fresh `GetToolStats`
    /// snapshot, pushed on switching into `View::Tools`.
    ToolStatsUpdated(Vec<aivyx_channel::daemon_ipc::ToolStat>),
```

- [ ] **Step 2: Wire the reducer arm**

Find the `Msg::AuditUpdated { entries, total_len } => { state.audit_entries = entries; state.audit_total = total_len; }` arm inside the `update` function (search for it) and add a new arm right after it:

```rust
        Msg::ToolStatsUpdated(tools) => {
            state.tool_stats = tools;
        }
```

- [ ] **Step 3: Fix the scroll-clamp match**

Find the `Msg::ScrollUp(n) => { ... }` arm's `let max = match state.view { View::Audit => state.audit_entries.len(), _ => state.history.len(), };` (search for `View::Audit => state.audit_entries.len()`) and add a new match arm right after it:

```rust
                View::Audit => state.audit_entries.len(),
                View::Tools => state.tool_stats.len(),
                _ => state.history.len(),
```

- [ ] **Step 4: Add the fetch function and wire the view-switch trigger**

In `crates/aivyx-tui/src/app.rs`, find `async fn fetch_audit_page(...)` (search for it) and add a new function right after its closing `}`:

```rust
/// POLISH_WAVES.md sub-project 8 item B — fetch a fresh `GetToolStats`
/// snapshot and push it into the Tools view. Called once on switching
/// into the view, mirroring `fetch_audit_page`'s own "no background
/// refresh" posture — `window_secs: None` (whole audit chain), matching
/// the `aivyx tools` CLI's own default. Best-effort: a fetch error
/// (e.g. no audit log configured) leaves the panel as-is.
async fn fetch_tool_stats(socket_path: &Path, state: &mut AppState) {
    if let Ok(tools) = aivyx_channel::daemon_client::get_tool_stats(socket_path, None).await {
        apply(state, Msg::ToolStatsUpdated(tools));
    }
}
```

Find the `Action::Update(msg) => { ... }` block (search for `let switching_to_audit = matches!(msg, Msg::SwitchView(View::Audit));`) and extend it:

```rust
            Action::Update(msg) => {
                // `/classic` retirement (Task E) — switching into the Audit
                // view seeds it with the newest page; the view itself has
                // no background refresh, so this is the only fetch until
                // the operator pages (Action::AuditPage, below).
                let switching_to_audit = matches!(msg, Msg::SwitchView(View::Audit));
                // POLISH_WAVES.md sub-project 8 item B — same posture for
                // the Tools view: seed on switch, no background refresh.
                let switching_to_tools = matches!(msg, Msg::SwitchView(View::Tools));
                apply(state, msg);
                if switching_to_audit {
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
            }
```

(This replaces the existing `Action::Update(msg) => { ... }` block in place — the `if switching_to_audit { ... }` body's contents are unchanged from what's already there; only the new `switching_to_tools` binding and its own `if` block are additions.)

- [ ] **Step 5: Render the Tools view**

In `crates/aivyx-tui/src/render.rs`, find `fn render_panel(...)`'s `match state.view { ... }` and replace the `View::Tools => ( "TOOLS", placeholder_lines(...) ),` arm with:

```rust
        View::Tools => {
            let mut lines: Vec<Line> = vec![Line::from(Span::styled(
                format!("{} tool(s) — whole audit chain", state.tool_stats.len()),
                fg(palette::DIMMER),
            ))];
            if state.tool_stats.is_empty() {
                lines.push(Line::from(Span::styled("No tools loaded.", fg(palette::DIM))));
            } else {
                for t in state.tool_stats.iter() {
                    let avg_ms = t.total_duration_ms.checked_div(t.calls).unwrap_or(0);
                    let marker = if t.registered { "" } else { " [unregistered]" };
                    lines.push(Line::from(vec![
                        Span::styled(format!("{}{marker} ", t.name), fg(palette::AMBER)),
                        Span::styled(format!("calls={} avg={avg_ms}ms", t.calls), fg(palette::DIMMER)),
                    ]));
                    if t.calls > 0 {
                        let parts: Vec<String> = t
                            .outcomes
                            .iter()
                            .map(|(label, count)| format!("{label}={count}"))
                            .collect();
                        lines.push(Line::from(Span::styled(
                            format!("  {}", parts.join(" ")),
                            fg(palette::DIM),
                        )));
                    }
                }
            }
            ("TOOLS", lines)
        }
```

Then update the scroll-application condition right below the `match` (search for `if state.view == View::Audit { ... para = para.scroll((top, 0)); }`) to also cover Tools:

```rust
    if state.view == View::Audit || state.view == View::Tools {
        let viewport = area.height.saturating_sub(2) as usize;
        let top = audit_scroll_offset(line_count, viewport, state.scroll);
        para = para.scroll((top, 0));
    }
```

Check whether `placeholder_lines` is still used anywhere else in this file after this change (search for other call sites) — if this was its only caller, leave the function defined but unused-check via `cargo clippy` in Step 7 below; if clippy flags it as dead code, remove the function in this same task (it would then be genuinely orphaned, not a decision to defer).

- [ ] **Step 6: Write the tests**

In `crates/aivyx-tui/src/render.rs`'s `#[cfg(test)] mod tests` block, find `fn audit_view_renders_entries_newest_first` (search for it) for the fixture-building convention, then add:

```rust
    #[test]
    fn tools_view_renders_placeholder_copy_is_gone() {
        let backend = TestBackend::new(72, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Tools;

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("No tools loaded"), "empty state shown");
        assert!(!text.contains("capability scope"), "placeholder copy is gone");
    }

    #[test]
    fn tools_view_renders_call_stats() {
        use aivyx_channel::daemon_ipc::ToolStat;
        let backend = TestBackend::new(72, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Tools;
        let mut outcomes = std::collections::BTreeMap::new();
        outcomes.insert("completed".to_string(), 3u64);
        state.tool_stats = vec![ToolStat {
            name: "fs.read".to_string(),
            description: "read a file".to_string(),
            scope_base: "fs.read".to_string(),
            registered: true,
            calls: 3,
            outcomes,
            total_duration_ms: 30,
        }];

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("fs.read"), "tool name rendered");
        assert!(text.contains("calls=3"), "call count rendered");
        assert!(text.contains("completed=3"), "outcome breakdown rendered");
    }

    #[test]
    fn tools_view_marks_unregistered_tools() {
        use aivyx_channel::daemon_ipc::ToolStat;
        let backend = TestBackend::new(72, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Tools;
        state.tool_stats = vec![ToolStat {
            name: "old.removed.tool".to_string(),
            description: "(no registered tool)".to_string(),
            scope_base: "old.removed.tool".to_string(),
            registered: false,
            calls: 1,
            outcomes: std::collections::BTreeMap::new(),
            total_duration_ms: 5,
        }];

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("[unregistered]"), "unregistered marker rendered");
    }
```

Also in `crates/aivyx-tui/src/model.rs`'s `#[cfg(test)] mod tests` block, find the existing `AuditUpdated`-based test (search for `Msg::AuditUpdated { entries: entries.clone(), total_len: 42 }`) and add:

```rust
    #[test]
    fn tool_stats_updated_replaces_state() {
        use aivyx_channel::daemon_ipc::ToolStat;
        let s = AppState::new();
        let tools = vec![ToolStat {
            name: "fs.read".to_string(),
            description: "read a file".to_string(),
            scope_base: "fs.read".to_string(),
            registered: true,
            calls: 1,
            outcomes: std::collections::BTreeMap::new(),
            total_duration_ms: 10,
        }];
        let s = update(s, Msg::ToolStatsUpdated(tools.clone()));
        assert_eq!(s.tool_stats, tools);
    }

    #[test]
    fn scroll_up_clamps_to_tool_stats_length_in_tools_view() {
        use aivyx_channel::daemon_ipc::ToolStat;
        let mut s = AppState::new();
        s.view = View::Tools;
        s.tool_stats = vec![
            ToolStat {
                name: "a".to_string(),
                description: String::new(),
                scope_base: "a".to_string(),
                registered: true,
                calls: 0,
                outcomes: std::collections::BTreeMap::new(),
                total_duration_ms: 0,
            },
            ToolStat {
                name: "b".to_string(),
                description: String::new(),
                scope_base: "b".to_string(),
                registered: true,
                calls: 0,
                outcomes: std::collections::BTreeMap::new(),
                total_duration_ms: 0,
            },
        ];
        s = update(s, Msg::ScrollUp(10));
        assert_eq!(s.scroll, 2, "clamps to tool_stats length, not history length");
    }
```

- [ ] **Step 7: Run the tests and clippy**

Run: `cargo test -p aivyx-tui`
Expected: all tests pass, including the 5 new ones.

Run: `cargo clippy -p aivyx-tui --all-targets -- -D warnings`
Expected: zero warnings (this is also where Step 5's `placeholder_lines`-still-used check gets its real answer — if it fires as dead code, remove the function and re-run this command to confirm clean).

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-tui/src/model.rs crates/aivyx-tui/src/app.rs crates/aivyx-tui/src/render.rs
git commit -m "feat(tui): wire the already-shipped GetToolStats query into View::Tools

No new backend -- View::Tools was a hardcoded placeholder; this replaces
it with a real fetch-on-switch (mirroring View::Audit's own posture) of
the Phase 102 GetToolStats query already backing the aivyx tools CLI.
Msg::ScrollUp's clamp match gains a View::Tools arm (its own doc comment
already states the rule: the ceiling must match whichever view is
currently reading it).

POLISH_WAVES.md sub-project 8, Task 3.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 4: Full workspace sweep + `dist/` rebuild

**Files:**
- None created; verification + a `dist/` rebuild only.

**Interfaces:**
- Consumes: everything from Tasks 1-3.
- Produces: nothing — this is the plan's final task, gating the whole-branch review.

- [ ] **Step 1: Full clippy sweep**

Run: `cargo clippy --workspace --exclude aivyx-desktop --all-targets -- -D warnings`
Expected: zero warnings. If `aivyx-desktop` cannot be excluded this way in this checkout, fall back to `cargo clippy --all-targets -- -D warnings` (no `--workspace`, touches only `default-members`) plus explicit `cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings` and `cargo clippy -p aivyx-tui --all-targets -- -D warnings`.

- [ ] **Step 2: Full test sweep**

Run: `cargo test --workspace --exclude aivyx-desktop`
Expected: zero failures. Same fallback as Step 1 if needed: `cargo test` (default-members only) plus `cargo test -p aivyx-web` and `cargo test -p aivyx-tui` explicitly.

- [ ] **Step 3: Rebuild `dist/`**

```bash
cd crates/aivyx-web
PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH" \
  dx bundle --release --platform web
rm -rf dist && mkdir -p dist
cp -r target/dx/aivyx-web/release/web/public/. dist/
find dist -name '*.br' -delete
cd ../..
```

- [ ] **Step 4: Verify the `dist/` diff is a clean rename, and check freshness by content**

Run: `git status --porcelain crates/aivyx-web/dist/assets/`
Expected: exactly one added and one deleted `.wasm` file (a rename by content hash), plus the usual `.js`/`index.html` churn.

Then confirm the new bundle actually contains Task 2's UI text (not a stale/cached build):

```bash
strings crates/aivyx-web/dist/assets/*.wasm | grep -F "no recent activity"
```

Expected: at least one match.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "chore: workspace sweep + dist/ rebuild for sub-project 8

cargo clippy/test clean across the workspace (aivyx-desktop excluded --
missing system webkit2gtk libs in this environment, pre-existing and
unrelated to this branch). dist/ rebuilt and verified fresh to serve the
MCP per-server health chip from this branch's HEAD.

POLISH_WAVES.md sub-project 8, Task 4.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## After all tasks: whole-branch review

Dispatch the final whole-branch code review on the most capable available model. Point it at:

- The design spec: `docs/superpowers/specs/2026-09-02-tool-server-observability-design.md`.
- This plan file.
- A `scripts/review-package MERGE_BASE HEAD` diff package (`MERGE_BASE = git merge-base main HEAD`).

Specifically ask the reviewer to verify:

1. `fold_mcp_server_stats`'s qualifier-splitting is actually exercised against a real `Scope` value end-to-end (not just a hand-constructed string) — build a real MCP tool's `required_scope()` output (`crates/aivyx-mcp/src/proxy.rs`) and confirm `fold_mcp_server_stats` recovers the right server name from it, not just from this plan's own test fixtures (which reconstruct the qualifier by hand and could drift from the real construction site without either side's tests catching it).
2. The health chip's `bad = failed + denied` semantics actually match what an operator would want flagged as "this server is having a problem" — specifically, could a `not_in_role`/`requires_escalation`/`rate_limited` outcome ever indicate a genuine server-side issue that this counting scheme is currently blind to (this plan's own design spec only classifies `failed`/`denied` as unhealthy; confirm that's still the right call once real MCP outcome distributions are considered, not just asserted).
3. `McpPanel`'s client-side join (`m.call_stats.iter().find(|s| s.server_name == sv.name)`) is O(servers × call_stats) — confirm this is fine at any realistic scale (a handful of configured MCP servers) and isn't a latent performance concern worth flagging even if not worth fixing.
4. `dist/` in the final commit is actually fresh relative to `main.rs`'s HEAD (this sub-project's own established process gap) — verified by checking the compiled `.wasm` for the new UI string, not just by trusting the rebuild step ran.

After the review (and any fix wave + re-review it triggers), invoke `superpowers:finishing-a-development-branch` for the feature branch, then update `docs/POLISH_WAVES.md` and `aivyx-ecosystem/ROADMAP.md` to record sub-project 8 as shipped.

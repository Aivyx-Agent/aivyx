//! `aivyx tools` CLI — Phase 102.
//!
//! Read-only tool-observability view: every registered tool
//! annotated with audit-derived call statistics (counts, the
//! outcome breakdown, average duration), with an optional
//! `--window` filter. IPC-backed — the same daemon-query shape
//! as `aivyx learning`. The render helper is a pure function so
//! unit tests drive it against fixtures without IPC.

use std::path::Path;

use aivyx_channel::daemon_client::{daemon_is_running, get_tool_stats};
use aivyx_channel::daemon_ipc::{ToolStat, default_socket_path};

/// `aivyx tools [--window <secs>]`
pub async fn run_tools(window_secs: Option<u64>) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    let tools = get_tool_stats(&socket_path, window_secs)
        .await
        .map_err(|e| format!("failed to fetch tool stats: {e}"))?;
    print!("{}", render_tool_stats(&tools, window_secs));
    Ok(())
}

async fn require_daemon_running(socket_path: &Path) -> Result<(), String> {
    if daemon_is_running(socket_path).await {
        return Ok(());
    }
    Err(format!(
        "aivyx tools: no daemon running on socket {} — \
         start the daemon first with `aivyx daemon run`",
        socket_path.display(),
    ))
}

/// Pure renderer — a flat-text table, one stanza per tool.
///
/// Rows arrive already ordered by the daemon (call count
/// descending, then name ascending). A tool with zero calls in
/// the window still renders — a registered-but-unused tool is
/// itself a signal — but its outcome line is omitted. An
/// `[unregistered]` marker flags a base with audit history but
/// no currently registered tool.
fn render_tool_stats(tools: &[ToolStat], window_secs: Option<u64>) -> String {
    let mut out = String::new();
    match window_secs {
        Some(s) => out.push_str(&format!("Tool usage (last {s}s)\n")),
        None => out.push_str("Tool usage (whole audit chain)\n"),
    }
    if tools.is_empty() {
        out.push_str("  (no tools)\n");
        return out;
    }
    for t in tools {
        let avg_ms = t.total_duration_ms.checked_div(t.calls).unwrap_or(0);
        let marker = if t.registered { "" } else { " [unregistered]" };
        out.push_str(&format!(
            "  {}{marker}\n    base={}  calls={}  avg={avg_ms}ms\n",
            t.name, t.scope_base, t.calls,
        ));
        if t.calls > 0 {
            // `outcomes` is a BTreeMap — iteration is sorted, so
            // the rendered breakdown is deterministic.
            let parts: Vec<String> = t
                .outcomes
                .iter()
                .map(|(label, count)| format!("{label}={count}"))
                .collect();
            out.push_str(&format!("    outcomes: {}\n", parts.join(" ")));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn stat(
        name: &str,
        scope_base: &str,
        registered: bool,
        calls: u64,
        outcomes: &[(&str, u64)],
        total_duration_ms: u64,
    ) -> ToolStat {
        ToolStat {
            name: name.to_string(),
            description: format!("{name} description"),
            scope_base: scope_base.to_string(),
            registered,
            calls,
            outcomes: outcomes
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect::<BTreeMap<_, _>>(),
            total_duration_ms,
        }
    }

    #[test]
    fn render_empty_is_graceful() {
        let out = render_tool_stats(&[], None);
        assert!(out.contains("Tool usage (whole audit chain)"));
        assert!(out.contains("(no tools)"));
    }

    #[test]
    fn render_window_header_reflects_the_filter() {
        assert!(render_tool_stats(&[], Some(3600)).contains("last 3600s"));
        assert!(render_tool_stats(&[], None).contains("whole audit chain"));
    }

    #[test]
    fn render_called_tool_shows_counts_avg_and_outcomes() {
        let tools = vec![stat(
            "fs.read",
            "fs.read",
            true,
            12,
            &[("completed", 11), ("failed", 1)],
            36,
        )];
        let out = render_tool_stats(&tools, None);
        assert!(out.contains("fs.read"));
        assert!(out.contains("base=fs.read  calls=12  avg=3ms"));
        assert!(out.contains("outcomes: completed=11 failed=1"));
        assert!(!out.contains("[unregistered]"));
    }

    #[test]
    fn render_uncalled_tool_omits_the_outcome_line() {
        let tools = vec![stat("web.fetch", "net.fetch", true, 0, &[], 0)];
        let out = render_tool_stats(&tools, None);
        // A registered-but-unused tool still appears...
        assert!(out.contains("web.fetch"));
        assert!(out.contains("base=net.fetch  calls=0  avg=0ms"));
        // ...but with no outcome breakdown.
        assert!(!out.contains("outcomes:"));
    }

    #[test]
    fn render_unregistered_base_is_marked() {
        let tools = vec![stat(
            "fs.delete",
            "fs.delete",
            false,
            2,
            &[("completed", 2)],
            4,
        )];
        let out = render_tool_stats(&tools, None);
        assert!(
            out.contains("fs.delete [unregistered]"),
            "a base with history but no registered tool must be marked: {out}"
        );
    }
}

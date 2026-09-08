//! Operator-facing `aivyx-pa tool-relevance` CLI surface — Phase 119
//! Task 6.
//!
//! Closes the Phase 116 deferred inspection surface: the
//! `KeyDomain::ToolRelevanceLedger` is encrypted at rest, so
//! operators had no read path into it before this CLI landed. With
//! `aivyx-pa tool-relevance dump [--keyword-key <key>]`, the operator
//! can see what the Phase 116 relevance ledger has accumulated
//! across turns and decide whether the per-keyword-key outcome rows
//! match what the self-learning loop is reinforcing.
//!
//! Future `aivyx-pa tool-relevance` subcommands (e.g. `clear`,
//! `forget <key>`) land additively under the same parser.

/// Entry point for `aivyx-pa tool-relevance dump [--keyword-key <key>]`.
/// Talks to the daemon over IPC; renders the returned per-row table
/// in a stable column layout the operator can scan.
pub async fn run_tool_relevance_dump(keyword_key_filter: Option<&str>) -> Result<(), String> {
    use aivyx_channel::daemon_client::{daemon_is_running, dump_tool_relevance};
    use aivyx_channel::daemon_ipc::default_socket_path;

    let socket_path = default_socket_path()?;
    if !daemon_is_running(&socket_path).await {
        return Err(format!(
            "aivyx-pa tool-relevance dump: daemon must be running \
             (socket {}). Start it with `aivyx-pa`.",
            socket_path.display(),
        ));
    }

    let rows = dump_tool_relevance(&socket_path, keyword_key_filter)
        .await
        .map_err(|e| format!("failed to fetch tool-relevance dump: {e}"))?;

    print!("{}", render_dump_table(&rows));
    Ok(())
}

/// Pure renderer: format the dump rows as a human-readable table.
/// Extracted so the column layout is testable without IPC
/// scaffolding.
///
/// Empty input renders the "(ledger empty)" sentinel so the
/// operator distinguishes "daemon answered with zero rows" from
/// "daemon failed" (the daemon-failure path errors before this
/// function is called).
pub fn render_dump_table(rows: &[aivyx_channel::daemon_ipc::ToolRelevanceDumpRow]) -> String {
    if rows.is_empty() {
        return "(ledger empty — no per-keyword-key outcomes recorded yet)\n".to_string();
    }

    // Column widths sized to the longest value in each column.
    // The header row participates in the width computation so the
    // header never overflows.
    let mut w_key = "keyword_key".len();
    let mut w_surface = "surface".len();
    let mut w_id = "identifier".len();
    let mut w_succ = "success".len();
    let mut w_fail = "failure".len();
    let w_last_seen = "last_seen_unix_ms".len();
    for row in rows {
        w_key = w_key.max(row.keyword_key.chars().count());
        w_surface = w_surface.max(row.surface_kind.chars().count());
        w_id = w_id.max(row.identifier.chars().count());
        w_succ = w_succ.max(row.success_count.to_string().len());
        w_fail = w_fail.max(row.failure_count.to_string().len());
    }

    let mut out = String::new();
    // Header.
    out.push_str(&format!(
        "{:<w_key$}  {:<w_surface$}  {:<w_id$}  {:>w_succ$}  {:>w_fail$}  {:<w_last_seen$}\n",
        "keyword_key",
        "surface",
        "identifier",
        "success",
        "failure",
        "last_seen_unix_ms",
        w_key = w_key,
        w_surface = w_surface,
        w_id = w_id,
        w_succ = w_succ,
        w_fail = w_fail,
        w_last_seen = w_last_seen,
    ));
    // Separator.
    out.push_str(&format!(
        "{:-<w_key$}  {:-<w_surface$}  {:-<w_id$}  {:-<w_succ$}  {:-<w_fail$}  {:-<w_last_seen$}\n",
        "",
        "",
        "",
        "",
        "",
        "",
        w_key = w_key,
        w_surface = w_surface,
        w_id = w_id,
        w_succ = w_succ,
        w_fail = w_fail,
        w_last_seen = w_last_seen,
    ));
    // Rows.
    for row in rows {
        out.push_str(&format!(
            "{:<w_key$}  {:<w_surface$}  {:<w_id$}  {:>w_succ$}  {:>w_fail$}  {:<w_last_seen$}\n",
            row.keyword_key,
            row.surface_kind,
            row.identifier,
            row.success_count,
            row.failure_count,
            row.last_seen_unix_ms,
            w_key = w_key,
            w_surface = w_surface,
            w_id = w_id,
            w_succ = w_succ,
            w_fail = w_fail,
            w_last_seen = w_last_seen,
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_channel::daemon_ipc::ToolRelevanceDumpRow;

    fn row(
        keyword_key: &str,
        surface_kind: &str,
        identifier: &str,
        success_count: u32,
        failure_count: u32,
        last_seen_unix_ms: u64,
    ) -> ToolRelevanceDumpRow {
        ToolRelevanceDumpRow {
            keyword_key: keyword_key.into(),
            surface_kind: surface_kind.into(),
            identifier: identifier.into(),
            success_count,
            failure_count,
            last_seen_unix_ms,
        }
    }

    #[test]
    fn render_empty_input_emits_sentinel() {
        let out = render_dump_table(&[]);
        assert!(out.contains("ledger empty"));
        // Sentinel ends in newline so terminal prompt lands cleanly.
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn render_single_row_includes_header_separator_and_value() {
        let rows = vec![row(
            "research+deploy",
            "tool",
            "fs.read",
            7,
            1,
            1_715_000_000_000,
        )];
        let out = render_dump_table(&rows);
        // Header row carries the column labels.
        assert!(out.contains("keyword_key"));
        assert!(out.contains("surface"));
        assert!(out.contains("identifier"));
        assert!(out.contains("success"));
        assert!(out.contains("failure"));
        assert!(out.contains("last_seen_unix_ms"));
        // Separator row of dashes.
        assert!(out.contains("---"));
        // Data row carries the values.
        assert!(out.contains("research+deploy"));
        assert!(out.contains("fs.read"));
        assert!(out.contains("1715000000000"));
    }

    #[test]
    fn render_multiple_rows_orders_as_caller_supplied() {
        // The daemon-side handler sorts rows ascending by
        // (keyword_key, surface_kind, identifier) so the table
        // renders in a stable order. The renderer trusts the
        // caller's order — it doesn't re-sort.
        let rows = vec![
            row("a+b", "skill", "summarize-pdf", 3, 0, 100),
            row("a+b", "tool", "fs.read", 7, 1, 200),
            row("c+d", "tool", "web.fetch", 2, 0, 300),
        ];
        let out = render_dump_table(&rows);
        let pos_skill = out.find("summarize-pdf").unwrap();
        let pos_tool = out.find("fs.read").unwrap();
        let pos_fetch = out.find("web.fetch").unwrap();
        // summarize-pdf appears before fs.read (caller order).
        assert!(pos_skill < pos_tool);
        // fs.read appears before web.fetch (different keyword key).
        assert!(pos_tool < pos_fetch);
    }

    #[test]
    fn render_column_widths_size_to_longest_value() {
        // A long keyword key must not get truncated; the column
        // grows to fit. (No-truncation guarantee: the operator
        // should see the full key for forensic reading.)
        let long_key = "this+is+a+very+long+keyword+key+with+many+tokens";
        let rows = vec![row(long_key, "tool", "fs.read", 1, 0, 100)];
        let out = render_dump_table(&rows);
        assert!(out.contains(long_key));
    }
}

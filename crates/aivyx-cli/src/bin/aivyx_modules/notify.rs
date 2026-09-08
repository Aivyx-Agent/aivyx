//! `aivyx-pa notify history` CLI — Phase 73.
//!
//! Terminal parity with the Web UI Notifications pane.
//! IPC-backed (the audit chain lives in encrypted storage; the
//! daemon's `ListNotificationHistory` handler walks the chain and
//! returns the paginated page). Renders as a flat-text table for
//! readability; pipe to `column -t` if you want fixed widths.

use std::path::Path;

use aivyx_channel::daemon_client::{daemon_is_running, list_notification_history};
use aivyx_channel::daemon_ipc::{NotificationHistoryEntry, default_socket_path};

/// Entry point for `aivyx-pa notify history [--target NAME] [--limit N]`.
pub async fn run_notify_history(target: Option<&str>, limit: u32) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    let (entries, total_len) = list_notification_history(&socket_path, 0, limit, target)
        .await
        .map_err(|e| format!("failed to list notification history: {e}"))?;
    print!("{}", render_history(target, &entries, total_len));
    Ok(())
}

async fn require_daemon_running(socket_path: &Path) -> Result<(), String> {
    if daemon_is_running(socket_path).await {
        return Ok(());
    }
    Err(format!(
        "aivyx-pa notify: no daemon running on socket {} — \
         start the daemon first with `aivyx-pa daemon run`",
        socket_path.display(),
    ))
}

/// Pure render — split out so unit tests can drive against
/// fixtures without IPC.
fn render_history(
    target_filter: Option<&str>,
    entries: &[NotificationHistoryEntry],
    total_len: u64,
) -> String {
    let mut out = String::new();
    let header = match target_filter {
        Some(name) => format!("Notification history (target = {name})\n"),
        None => "Notification history (all targets)\n".to_string(),
    };
    out.push_str(&header);
    out.push_str("======================================\n\n");
    if entries.is_empty() {
        out.push_str(&match target_filter {
            Some(name) => {
                format!("No history for target `{name}`.\n")
            }
            None => "No notification history yet.\n".into(),
        });
        return out;
    }
    for e in entries {
        let mut line = format!(
            "[#{seq}] {ts}ms  {target:<20}  {outcome:<24}  {trigger_kind}/{trigger_id}",
            seq = e.seq,
            ts = e.dispatched_at_unix_ms,
            target = e.target_name,
            outcome = e.outcome_kind,
            trigger_kind = e.trigger_kind,
            trigger_id = e.trigger_id,
        );
        if !e.outcome_detail.is_empty() {
            line.push_str(&format!("  · {}", e.outcome_detail));
        }
        line.push('\n');
        out.push_str(&line);
    }
    if total_len as usize > entries.len() {
        out.push_str(&format!(
            "\n(showing first {} of {} matching events)\n",
            entries.len(),
            total_len,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_delivered() -> NotificationHistoryEntry {
        NotificationHistoryEntry {
            seq: 42,
            dispatched_at_unix_ms: 1_715_000_000_000,
            session_id: "ses-abc".into(),
            trigger_kind: "Cron".into(),
            trigger_id: "morning-summary".into(),
            target_name: "phone".into(),
            outcome_kind: "delivered".into(),
            outcome_detail: String::new(),
        }
    }

    fn fixture_failed() -> NotificationHistoryEntry {
        NotificationHistoryEntry {
            seq: 43,
            dispatched_at_unix_ms: 1_715_000_001_000,
            session_id: "ses-def".into(),
            trigger_kind: "Webhook".into(),
            trigger_id: "github-push".into(),
            target_name: "desktop".into(),
            outcome_kind: "failed".into(),
            outcome_detail: "[transport] dns lookup failed".into(),
        }
    }

    #[test]
    fn render_empty_with_no_filter_explains_no_history() {
        let s = render_history(None, &[], 0);
        assert!(s.contains("Notification history (all targets)"));
        assert!(s.contains("No notification history yet."));
    }

    #[test]
    fn render_empty_with_target_filter_names_target() {
        let s = render_history(Some("phone"), &[], 0);
        assert!(s.contains("target = phone"));
        assert!(s.contains("No history for target `phone`"));
    }

    #[test]
    fn render_delivered_entry_omits_detail_column() {
        let s = render_history(None, &[fixture_delivered()], 1);
        assert!(s.contains("[#42]"));
        assert!(s.contains("phone"));
        assert!(s.contains("delivered"));
        assert!(s.contains("Cron/morning-summary"));
        // delivered has empty detail — no trailing `· …`.
        assert!(!s.contains(" · "));
    }

    #[test]
    fn render_failed_entry_includes_detail_column() {
        let s = render_history(None, &[fixture_failed()], 1);
        assert!(s.contains("desktop"));
        assert!(s.contains("failed"));
        assert!(s.contains("Webhook/github-push"));
        assert!(s.contains("· [transport] dns lookup failed"));
    }

    #[test]
    fn render_pagination_note_appears_when_total_exceeds_returned() {
        let s = render_history(None, &[fixture_delivered()], 5);
        assert!(s.contains("showing first 1 of 5 matching events"));
    }
}

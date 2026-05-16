//! `aivyx memory` CLI — Phase 74.
//!
//! Terminal parity with the Web UI Memory pane. IPC-backed
//! (the substrate lives in encrypted storage; the daemon's
//! memory queries walk it). Render helpers are split out as
//! pure functions so unit tests drive them against fixtures
//! without IPC.

use std::io::Write;
use std::path::Path;

use aivyx_channel::daemon_client::{
    daemon_is_running, evict_memory_topic, get_memory_topic_entries,
    list_memory_topics, search_memory,
};
use aivyx_channel::daemon_ipc::{default_socket_path, MemoryEntrySummary};

/// `aivyx memory list`
pub async fn run_memory_list() -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    let topics = list_memory_topics(&socket_path)
        .await
        .map_err(|e| format!("failed to list memory topics: {e}"))?;
    print!("{}", render_topics(&topics));
    Ok(())
}

/// `aivyx memory show <topic> [--limit N]`
pub async fn run_memory_show(topic: &str, limit: u32) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    let entries = get_memory_topic_entries(&socket_path, topic, limit)
        .await
        .map_err(|e| format!("failed to fetch memory topic: {e}"))?;
    print!("{}", render_entries(&format!("topic `{topic}`"), &entries));
    Ok(())
}

/// `aivyx memory search <query> [--semantic] [--limit N]`
pub async fn run_memory_search(
    query: &str,
    limit: u32,
    semantic: bool,
) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    let (matches, fell_back) =
        search_memory(&socket_path, query, limit, semantic)
            .await
            .map_err(|e| format!("failed to search memory: {e}"))?;
    let label = if semantic && !fell_back {
        format!("semantic search \"{query}\"")
    } else {
        format!("search \"{query}\"")
    };
    if semantic && fell_back {
        eprintln!(
            "aivyx memory search: semantic unavailable \
             (no [embedding] config, provider error, or empty \
             vector index) — showing keyword results"
        );
    }
    print!("{}", render_entries(&label, &matches));
    Ok(())
}

/// `aivyx memory evict <topic> [--yes]`
pub async fn run_memory_evict(topic: &str, yes: bool) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    if !yes {
        print!(
            "Delete every memory entry under topic `{topic}`? \
             This cannot be undone. [y/N] "
        );
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| format!("failed to read confirmation: {e}"))?;
        let answer = line.trim().to_lowercase();
        if answer != "y" && answer != "yes" {
            eprintln!("aivyx memory evict: aborted (no confirmation)");
            return Ok(());
        }
    }
    let deleted = evict_memory_topic(&socket_path, topic)
        .await
        .map_err(|e| format!("evict failed: {e}"))?;
    eprintln!(
        "aivyx memory evict: ok — deleted {deleted} entries from `{topic}`"
    );
    Ok(())
}

async fn require_daemon_running(socket_path: &Path) -> Result<(), String> {
    if daemon_is_running(socket_path).await {
        return Ok(());
    }
    Err(format!(
        "aivyx memory: no daemon running on socket {} — \
         start the daemon first with `aivyx daemon run`",
        socket_path.display(),
    ))
}

fn render_topics(topics: &[String]) -> String {
    let mut out = String::from("Memory topics\n");
    out.push_str("=============\n\n");
    if topics.is_empty() {
        out.push_str("No memory topics yet.\n");
        return out;
    }
    for t in topics {
        out.push_str(&format!("  {t}\n"));
    }
    out.push_str(&format!("\n({} topic(s))\n", topics.len()));
    out
}

fn render_entries(title: &str, entries: &[MemoryEntrySummary]) -> String {
    let mut out = format!("Memory: {title}\n");
    out.push_str("======================================\n\n");
    if entries.is_empty() {
        out.push_str("No entries.\n");
        return out;
    }
    for e in entries {
        let read = if e.last_read_at_secs == 0 {
            "never".to_string()
        } else {
            format!("{}s", e.last_read_at_secs)
        };
        out.push_str(&format!(
            "[#{seq}] {topic}  written={created}s  last_read={read}\n  {body}\n",
            seq = e.seq,
            topic = e.topic,
            created = e.created_at_secs,
            read = read,
            body = e.body,
        ));
    }
    out.push_str(&format!("\n({} entry(ies))\n", entries.len()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(seq: u64, topic: &str, body: &str, read: u64) -> MemoryEntrySummary {
        MemoryEntrySummary {
            topic: topic.into(),
            body: body.into(),
            seq,
            created_at_secs: 1_715_000_000,
            last_read_at_secs: read,
        }
    }

    #[test]
    fn render_topics_empty_explains_no_topics() {
        let s = render_topics(&[]);
        assert!(s.contains("No memory topics yet."));
    }

    #[test]
    fn render_topics_lists_each_with_count() {
        let s = render_topics(&[
            "notes".to_string(),
            "project/x".to_string(),
        ]);
        assert!(s.contains("  notes"));
        assert!(s.contains("  project/x"));
        assert!(s.contains("(2 topic(s))"));
    }

    #[test]
    fn render_entries_empty_says_no_entries() {
        let s = render_entries("topic `notes`", &[]);
        assert!(s.contains("Memory: topic `notes`"));
        assert!(s.contains("No entries."));
    }

    #[test]
    fn render_entries_shows_seq_topic_body_and_never_read() {
        let s = render_entries(
            "topic `notes`",
            &[fixture(3, "notes", "remember the milk", 0)],
        );
        assert!(s.contains("[#3]"));
        assert!(s.contains("notes"));
        assert!(s.contains("remember the milk"));
        assert!(s.contains("last_read=never"));
        assert!(s.contains("(1 entry(ies))"));
    }

    #[test]
    fn render_entries_shows_read_timestamp_when_set() {
        let s = render_entries(
            "search \"foo\"",
            &[fixture(9, "project/x", "the foo subsystem", 1_715_000_500)],
        );
        assert!(s.contains("last_read=1715000500s"));
        assert!(s.contains("Memory: search \"foo\""));
    }
}

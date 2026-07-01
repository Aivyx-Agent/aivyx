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
    daemon_is_running, evict_memory_topic, get_knowledge_graph, get_memory_conflicts,
    get_memory_topic_entries, get_wiki_page, list_memory_topics, list_wiki_pages,
    resolve_memory_conflict, search_memory,
};
use aivyx_channel::contradiction::MemoryConflict;
use aivyx_channel::knowledge_graph::{GraphEntity, GraphTriple};
use aivyx_channel::knowledge_wiki::{WikiPage, WikiPageSummary};
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

/// `aivyx memory wiki [topic]` — list synthesized knowledge-wiki pages, or show
/// one topic's consolidated page (summary + backlinks). CLI parity with the
/// Studio Wiki screen.
pub async fn run_memory_wiki(topic: Option<String>) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    match topic {
        None => {
            let pages = list_wiki_pages(&socket_path)
                .await
                .map_err(|e| format!("failed to list wiki pages: {e}"))?;
            print!("{}", render_wiki_list(&pages));
        }
        Some(t) => {
            let page = get_wiki_page(&socket_path, t.clone())
                .await
                .map_err(|e| format!("failed to get wiki page: {e}"))?;
            print!("{}", render_wiki_page(&t, page.as_ref()));
        }
    }
    Ok(())
}

/// `aivyx memory graph [entity]` — show the typed knowledge graph (entity →
/// predicate → entity), optionally filtered to triples touching `entity`.
pub async fn run_memory_graph(entity: Option<String>) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    let (entities, triples) = get_knowledge_graph(&socket_path, 200)
        .await
        .map_err(|e| format!("failed to get knowledge graph: {e}"))?;
    print!("{}", render_graph(&entities, &triples, entity.as_deref()));
    Ok(())
}

/// `aivyx memory conflicts` — run the on-demand contradiction pass.
pub async fn run_memory_conflicts() -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    let conflicts = get_memory_conflicts(&socket_path)
        .await
        .map_err(|e| format!("failed to detect memory conflicts: {e}"))?;
    print!("{}", render_conflicts(&conflicts));
    Ok(())
}

/// `aivyx memory resolve <topic> --archive <seq>`
pub async fn run_memory_resolve(topic: &str, archive_seq: u64) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    let removed = resolve_memory_conflict(&socket_path, topic, archive_seq)
        .await
        .map_err(|e| format!("failed to resolve conflict: {e}"))?;
    if removed {
        println!("Resolved: archived entry seq {archive_seq} under `{topic}`.");
    } else {
        println!(
            "No entry seq {archive_seq} under `{topic}` — nothing to archive \
             (already resolved?)."
        );
    }
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

fn render_conflicts(conflicts: &[MemoryConflict]) -> String {
    let mut out = String::from("Memory conflicts\n================\n\n");
    if conflicts.is_empty() {
        out.push_str(
            "No contradictions detected. (Detection is an on-demand LLM pass; \
             it needs a configured model and at least two entries under a topic.)\n",
        );
        return out;
    }
    for c in conflicts {
        out.push_str(&format!("⚠ {}  —  {}\n", c.topic, c.reason.trim()));
        out.push_str(&format!(
            "  [a] {}      (older, seq {})\n",
            c.a.body.trim().replace('\n', " "),
            c.a.seq,
        ));
        out.push_str(&format!(
            "  [b] {}      (newer, seq {})\n",
            c.b.body.trim().replace('\n', " "),
            c.b.seq,
        ));
        out.push_str(&format!(
            "  keep b: aivyx memory resolve {} --archive {}\n",
            c.topic, c.a.seq,
        ));
        out.push_str(&format!(
            "  keep a: aivyx memory resolve {} --archive {}\n\n",
            c.topic, c.b.seq,
        ));
    }
    out.push_str(&format!(
        "({} conflict(s)) — `resolve` deletes the entry you DON'T keep\n",
        conflicts.len()
    ));
    out
}

fn render_wiki_list(pages: &[WikiPageSummary]) -> String {
    let mut out = String::from("Knowledge wiki pages\n====================\n\n");
    if pages.is_empty() {
        out.push_str(
            "No wiki pages yet. The agent consolidates a topic into a page when \
             `[memory] profile = \"smart\"` (or `[wiki] enabled = true`).\n",
        );
        return out;
    }
    for p in pages {
        out.push_str(&format!(
            "  {}  ({} entr{})\n    {}\n",
            p.topic,
            p.entry_count,
            if p.entry_count == 1 { "y" } else { "ies" },
            p.snippet.trim(),
        ));
    }
    out.push_str(&format!("\n({} page(s)) — `aivyx memory wiki <topic>` for the full page\n", pages.len()));
    out
}

fn render_wiki_page(topic: &str, page: Option<&WikiPage>) -> String {
    let mut out = format!("Wiki page: {topic}\n");
    out.push_str(&"=".repeat(11 + topic.len()));
    out.push_str("\n\n");
    match page {
        None => {
            out.push_str(&format!(
                "No wiki page for `{topic}` yet (no entries consolidated, or the \
                 wiki sweep hasn't run).\n"
            ));
        }
        Some(p) => {
            out.push_str(p.summary.trim());
            out.push_str("\n\n");
            if !p.backlinks.is_empty() {
                out.push_str("Related topics:\n");
                for b in &p.backlinks {
                    out.push_str(&format!("  → {} (affinity {:.2})\n", b.topic, b.affinity));
                }
                out.push('\n');
            }
            out.push_str(&format!("(consolidated from {} entr{})\n",
                p.entry_count, if p.entry_count == 1 { "y" } else { "ies" }));
        }
    }
    out
}

fn render_graph(
    entities: &[GraphEntity],
    triples: &[GraphTriple],
    filter: Option<&str>,
) -> String {
    let mut out = String::from("Knowledge graph\n===============\n\n");
    if entities.is_empty() && triples.is_empty() {
        out.push_str(
            "No knowledge graph yet. The agent extracts entity relations when \
             `[memory] profile = \"smart\"` (or `[graph] enabled = true`).\n",
        );
        return out;
    }
    let shown: Vec<&GraphTriple> = triples
        .iter()
        .filter(|t| {
            filter.is_none_or(|f| {
                t.subject.eq_ignore_ascii_case(f) || t.object.eq_ignore_ascii_case(f)
            })
        })
        .collect();
    if let Some(f) = filter {
        out.push_str(&format!("Relations touching `{f}`:\n"));
    }
    if shown.is_empty() {
        out.push_str("  (no matching relations)\n");
    } else {
        for t in &shown {
            out.push_str(&format!(
                "  {} --[{}]--> {}  ({}\u{00d7})\n",
                t.subject, t.predicate, t.object, t.mentions,
            ));
        }
    }
    out.push_str(&format!(
        "\n({} entit{}, {} relation(s){})\n",
        entities.len(),
        if entities.len() == 1 { "y" } else { "ies" },
        shown.len(),
        if filter.is_some() { format!(" of {}", triples.len()) } else { String::new() },
    ));
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
    fn render_conflicts_empty_and_populated() {
        use aivyx_channel::contradiction::{ConflictSide, MemoryConflict};
        let empty = render_conflicts(&[]);
        assert!(empty.contains("No contradictions detected"));

        let c = MemoryConflict {
            id: MemoryConflict::make_id("operator-note", 3, 7),
            topic: "operator-note".into(),
            a: ConflictSide {
                seq: 3,
                body: "Home airport: YPPH (Perth)".into(),
                created_at_secs: 100,
            },
            b: ConflictSide {
                seq: 7,
                body: "Home airport: Sydney, YSSY".into(),
                created_at_secs: 200,
            },
            reason: "two different home airports".into(),
        };
        let s = render_conflicts(std::slice::from_ref(&c));
        assert!(s.contains("operator-note"));
        assert!(s.contains("two different home airports"));
        assert!(s.contains("Perth"));
        assert!(s.contains("Sydney"));
        // keep-b archives the older (seq 3); keep-a archives the newer (seq 7).
        assert!(s.contains("keep b: aivyx memory resolve operator-note --archive 3"));
        assert!(s.contains("keep a: aivyx memory resolve operator-note --archive 7"));
        assert!(s.contains("(1 conflict(s))"));
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

    #[test]
    fn render_wiki_list_empty_and_populated() {
        assert!(render_wiki_list(&[]).contains("No wiki pages yet"));
        let pages = vec![WikiPageSummary {
            topic: "aviation".into(),
            snippet: "VFR means visual flight rules".into(),
            entry_count: 3,
            updated_at: 1,
        }];
        let s = render_wiki_list(&pages);
        assert!(s.contains("aviation"));
        assert!(s.contains("3 entries"));
        assert!(s.contains("VFR means visual flight rules"));
    }

    #[test]
    fn render_wiki_page_none_and_full() {
        assert!(render_wiki_page("ghost", None).contains("No wiki page for `ghost`"));
        let page = WikiPage {
            topic: "aviation".into(),
            summary: "A consolidated summary of aviation notes.".into(),
            source_seqs: vec![1, 2],
            entry_count: 2,
            backlinks: vec![aivyx_channel::knowledge_wiki::WikiBacklink {
                topic: "coffee".into(),
                affinity: 0.42,
                hops: 1,
            }],
            updated_at: 1,
            source_fingerprint: 9,
        };
        let s = render_wiki_page("aviation", Some(&page));
        assert!(s.contains("consolidated summary of aviation"));
        assert!(s.contains("→ coffee (affinity 0.42)"));
        assert!(s.contains("consolidated from 2 entries"));
    }

    #[test]
    fn render_graph_empty_full_and_filtered() {
        assert!(render_graph(&[], &[], None).contains("No knowledge graph yet"));
        let entities = vec![
            GraphEntity { name: "aviation".into(), degree: 2, kind: String::new() },
            GraphEntity { name: "YPPH".into(), degree: 1, kind: "airport".into() },
        ];
        let triples = vec![
            GraphTriple {
                subject: "aviation".into(),
                predicate: "relates-to".into(),
                object: "YPPH".into(),
                source_seqs: vec![1],
                mentions: 2,
                updated_at: 1,
            },
            GraphTriple {
                subject: "coffee".into(),
                predicate: "is-a".into(),
                object: "beverage".into(),
                source_seqs: vec![2],
                mentions: 1,
                updated_at: 1,
            },
        ];
        let all = render_graph(&entities, &triples, None);
        assert!(all.contains("aviation --[relates-to]--> YPPH"));
        assert!(all.contains("coffee --[is-a]--> beverage"));
        assert!(all.contains("2 entities, 2 relation(s)"));
        // Filter to one entity (case-insensitive).
        let filtered = render_graph(&entities, &triples, Some("AVIATION"));
        assert!(filtered.contains("aviation --[relates-to]--> YPPH"));
        assert!(!filtered.contains("coffee --[is-a]--> beverage"));
        assert!(filtered.contains("1 relation(s) of 2"));
    }
}

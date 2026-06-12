//! Phase 80 — the structural proactive detector (the crux).
//!
//! Pure, no LLM, no I/O. Given the memory entries, the
//! Phase 77 recall-feedback tally, the `[proactive]` signal
//! toggles, the global TTL, and `now`, it returns zero or more
//! [`ProactiveItem`]s — each carrying a concrete,
//! human-readable `reason`. The assistant interrupts the
//! operator unprompted **only** when it can point to one of
//! these structural facts; it never asks a model "is this
//! worth surfacing?" (the Phase 77 no-self-judgement ethos
//! applied to the highest-trust-stakes action).

use serde::{Deserialize, Serialize};

// moved to the wasm-clean aivyx-ipc crate (Chapter M.2d-2); re-exported here.
pub use aivyx_ipc::insights::{ProactiveKind, ProactiveStat, ProactiveSurfaced};

use aivyx_config::ProactiveSignals;
use aivyx_memory::MemoryEntry;

use crate::recall_feedback::HelpfulnessTally;

/// An entry expiring within this window of TTL eviction is
/// "about to be lost" — worth one heads-up. One day.
pub const TTL_WARN_WINDOW_SECS: u64 = 86_400;

/// Per-topic net recall helpfulness a cluster must exceed to be
/// surfaced. Strictly higher than the Phase 77
/// Persona-proposal bar: proactively interrupting the operator
/// is a bigger ask than queuing a gated proposal, so it takes
/// stronger evidence.
pub const PROACTIVE_CLUSTER_THRESHOLD: f32 =
    2.0 * crate::recall_feedback::PROPOSAL_TOPIC_THRESHOLD;

/// Body prefix marking a reminder-shaped memory:
/// `@due:<unix_secs> <text>`. A strict, deterministic
/// convention — no date parsing, no NLP.
const DUE_MARKER: &str = "@due:";

/// Max body characters echoed into a surfacing summary.
const SUMMARY_CHARS: usize = 120;


/// One thing the assistant wants to surface unprompted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProactiveItem {
    /// Deterministic id `(kind:topic[:seq])` — stable across
    /// cycles so the dedup log suppresses re-surfacing.
    pub id: String,
    pub kind: ProactiveKind,
    pub topic: String,
    /// Short, operator-facing body of the surfacing.
    pub summary: String,
    /// Why this fired — concrete provenance shown to the
    /// operator (and recorded in the Phase 78 surface).
    pub reason: String,
}



/// Shared handle the proactive pass writes and the
/// `GetLearningInsights` handler reads. `None` inside = the
/// proactive pass has not run this daemon lifetime.
pub type SharedProactiveStat =
    std::sync::Arc<std::sync::RwLock<Option<ProactiveStat>>>;

/// Construct an empty shared proactive-stat handle.
pub fn shared_proactive_stat() -> SharedProactiveStat {
    std::sync::Arc::new(std::sync::RwLock::new(None))
}

fn snippet(body: &str) -> String {
    let one_line = body.replace('\n', " ");
    if one_line.chars().count() > SUMMARY_CHARS {
        let s: String =
            one_line.chars().take(SUMMARY_CHARS).collect();
        format!("{s}…")
    } else {
        one_line
    }
}

fn humanize_secs(secs: u64) -> String {
    if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// Parse a leading `@due:<digits>` marker, returning
/// `(due_secs, remaining_text)`. `None` if not reminder-shaped.
fn parse_due(body: &str) -> Option<(u64, &str)> {
    let rest = body.strip_prefix(DUE_MARKER)?;
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let due: u64 = rest[..end].parse().ok()?;
    Some((due, rest[end..].trim_start()))
}

/// Run every enabled signal class over the inputs. Output is
/// id-sorted for deterministic dedup + rendering.
pub fn detect(
    entries: &[MemoryEntry],
    tally: &HelpfulnessTally,
    signals: &ProactiveSignals,
    memory_ttl_secs: Option<u64>,
    now_secs: u64,
) -> Vec<ProactiveItem> {
    let mut out: Vec<ProactiveItem> = Vec::new();

    // --- TtlExpiry: an entry within the warn window of TTL ---
    if signals.ttl_expiry {
        if let Some(ttl) = memory_ttl_secs {
            for e in entries {
                let expiry = e.created_at_secs.saturating_add(ttl);
                if expiry > now_secs
                    && expiry - now_secs <= TTL_WARN_WINDOW_SECS
                {
                    let age = now_secs
                        .saturating_sub(e.created_at_secs);
                    out.push(ProactiveItem {
                        id: format!("ttl:{}:{}", e.topic, e.seq),
                        kind: ProactiveKind::TtlExpiry,
                        topic: e.topic.clone(),
                        summary: snippet(&e.body),
                        reason: format!(
                            "memory in '{}' written {} ago expires \
                             in {} — surface before it is lost",
                            e.topic,
                            humanize_secs(age),
                            humanize_secs(expiry - now_secs),
                        ),
                    });
                }
            }
        }
    }

    // --- RecallCluster: a strongly-net-helpful topic ---------
    if signals.recall_cluster {
        use std::collections::BTreeMap;
        let mut by_topic: BTreeMap<String, (f32, usize)> =
            BTreeMap::new();
        for (topic, _seq, score) in tally.ranked() {
            let e = by_topic.entry(topic).or_insert((0.0, 0));
            e.0 += score;
            e.1 += 1;
        }
        for (topic, (net, n)) in by_topic {
            if net >= PROACTIVE_CLUSTER_THRESHOLD {
                out.push(ProactiveItem {
                    id: format!("cluster:{topic}"),
                    kind: ProactiveKind::RecallCluster,
                    topic: topic.clone(),
                    summary: format!(
                        "Your '{topic}' memories keep proving \
                         useful."
                    ),
                    reason: format!(
                        "topic '{topic}' net recall-helpfulness \
                         {net:+.0} across {n} entr{} — a \
                         consistently valuable cluster",
                        if n == 1 { "y" } else { "ies" },
                    ),
                });
            }
        }
    }

    // --- DueReminder: a `@due:` whose time has come ----------
    if signals.due_reminder {
        for e in entries {
            if let Some((due, text)) = parse_due(&e.body) {
                if due <= now_secs {
                    out.push(ProactiveItem {
                        id: format!("due:{}:{}", e.topic, e.seq),
                        kind: ProactiveKind::DueReminder,
                        topic: e.topic.clone(),
                        summary: snippet(if text.is_empty() {
                            &e.body
                        } else {
                            text
                        }),
                        reason: format!(
                            "reminder in '{}' was due {} ago",
                            e.topic,
                            humanize_secs(
                                now_secs.saturating_sub(due)
                            ),
                        ),
                    });
                }
            }
        }
    }

    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        topic: &str,
        seq: u64,
        body: &str,
        created: u64,
    ) -> MemoryEntry {
        MemoryEntry {
            topic: topic.into(),
            body: body.into(),
            seq,
            created_at_secs: created,
            last_read_at_secs: 0,
        }
    }

    fn all_on() -> ProactiveSignals {
        ProactiveSignals::default()
    }

    #[test]
    fn ttl_expiry_fires_only_near_the_boundary() {
        let now = 1_000_000;
        let ttl = 100_000;
        // expires at created+ttl. "near" = within 86_400 of now.
        let near = entry("notes", 1, "important note", 950_000);
        // expiry = 1_050_000; now+86_400 = 1_086_400 → in window.
        let far = entry("notes", 2, "fresh", 990_000);
        // expiry = 1_090_000 > 1_086_400 → NOT in window.
        let expired = entry("notes", 3, "stale", 800_000);
        // expiry = 900_000 <= now → already gone, not "expiring".
        let items = detect(
            &[near, far, expired],
            &HelpfulnessTally::default(),
            &all_on(),
            Some(ttl),
            now,
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, ProactiveKind::TtlExpiry);
        assert_eq!(items[0].id, "ttl:notes:1");
        assert!(items[0].reason.contains("expires in"));
    }

    #[test]
    fn ttl_expiry_skipped_without_ttl_or_toggle() {
        let now = 1_000_000;
        let e = entry("notes", 1, "x", 950_000);
        // No global TTL configured.
        assert!(detect(
            std::slice::from_ref(&e),
            &HelpfulnessTally::default(),
            &all_on(),
            None,
            now
        )
        .is_empty());
        // TTL set but the class toggled off.
        let off = ProactiveSignals {
            ttl_expiry: false,
            ..ProactiveSignals::default()
        };
        assert!(detect(
            &[e],
            &HelpfulnessTally::default(),
            &off,
            Some(100_000),
            now
        )
        .is_empty());
    }

    #[test]
    fn recall_cluster_needs_to_clear_the_high_bar() {
        // Threshold = 2 * 3.0 = 6.0. "deploy" sums to 7
        // (clears); "misc" sums to 2 (does not).
        let t = HelpfulnessTally::from_triples(&[
            ("deploy", 1, 4.0),
            ("deploy", 2, 3.0),
            ("misc", 1, 2.0),
        ]);
        let items = detect(&[], &t, &all_on(), None, 1_000);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, ProactiveKind::RecallCluster);
        assert_eq!(items[0].id, "cluster:deploy");
        assert!(items[0].reason.contains("net recall-helpfulness"));
    }

    #[test]
    fn due_reminder_fires_when_past_due_only() {
        let now = 5_000;
        let due_past =
            entry("rem", 1, "@due:4000 call the vet", 1);
        let due_future =
            entry("rem", 2, "@due:9000 file taxes", 1);
        let not_reminder =
            entry("rem", 3, "just a normal note", 1);
        let items = detect(
            &[due_past, due_future, not_reminder],
            &HelpfulnessTally::default(),
            &all_on(),
            None,
            now,
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, ProactiveKind::DueReminder);
        assert_eq!(items[0].id, "due:rem:1");
        assert_eq!(items[0].summary, "call the vet");
        assert!(items[0].reason.contains("was due"));
    }

    #[test]
    fn empty_inputs_produce_nothing() {
        assert!(detect(
            &[],
            &HelpfulnessTally::default(),
            &all_on(),
            Some(100),
            1_000
        )
        .is_empty());
    }

    #[test]
    fn output_is_id_sorted_and_deterministic() {
        let now = 5_000;
        let a = entry("z-topic", 1, "@due:1 a", 1);
        let b = entry("a-topic", 2, "@due:1 b", 1);
        let i1 = detect(
            &[a.clone(), b.clone()],
            &HelpfulnessTally::default(),
            &all_on(),
            None,
            now,
        );
        let i2 = detect(
            &[b, a],
            &HelpfulnessTally::default(),
            &all_on(),
            None,
            now,
        );
        assert_eq!(i1, i2, "deterministic regardless of input order");
        assert_eq!(i1[0].id, "due:a-topic:2");
        assert_eq!(i1[1].id, "due:z-topic:1");
    }
}

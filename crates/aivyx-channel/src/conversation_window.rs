//! Phase 86 — per-session recent-turns buffer used by
//! auto-recall (Phase 76) and adaptive Persona selection
//! (Phase 79) so their relevance query is the *conversation
//! window*, not just the latest single message.
//!
//! For 85 phases both consumers judged relevance off one line.
//! In real multi-turn conversation the topic drifts, the
//! operator's intent spans several turns, and a single line is
//! a lossy proxy. Phase 86 fixes that with the smallest
//! possible substrate: a bounded recency ring of `(Role, text)`
//! turns keyed by `SessionId`, written by the daemon turn loop
//! when each turn completes (user + assistant pair) and read
//! by the providers via a shared handle injected at daemon
//! startup (the Phase 82/84 shared-handle precedent).
//!
//! **Ephemeral.** The buffer lives only as long as the daemon
//! process — a restart starts fresh. Durable per-session
//! windows are a deliberate deferral (recall context is
//! re-derivable from the persisted memory + the durable
//! ledgers; the *transcript* of recent chatter is intentionally
//! not persisted).
//!
//! **Opt-in.** `assemble_for` returns `None` when
//! `recall_window_turns <= 1` so the providers fall through to
//! exact pre-Phase-86 single-message embedding — the project's
//! behaviour-change-is-opt-in discipline (recall context feeds
//! model output).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};

use aivyx_core::SessionId;

/// The two roles the buffer records. Trigger-fired
/// non-conversational turns (cron / webhook / file-watch) are
/// deliberately NOT recorded — the window is for conversational
/// relevance, not synthetic events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    fn label(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

/// Hard cap on how many turns a single session's window may
/// store before the oldest is evicted. The operator-facing
/// `recall_window_turns` config knob is bounded by this — we
/// can never assemble more than we stored. Small on purpose:
/// the window is a *recency* signal, not a transcript.
pub const WINDOW_MAX_TURNS: usize = 16;

/// Hard char budget on the assembled output string. The
/// assembled prefix is trimmed (oldest first) to fit; the
/// current message is never truncated (it is the actual query).
pub const WINDOW_CHAR_BUDGET: usize = 4_000;

/// A bounded recency ring of `(Role, text)` turns for one
/// session. Push appends and evicts the oldest once
/// `max_turns` is exceeded.
#[derive(Debug, Clone)]
pub struct ConversationWindow {
    turns: VecDeque<(Role, String)>,
    max_turns: usize,
    char_budget: usize,
}

impl ConversationWindow {
    pub fn new() -> Self {
        Self::with_caps(WINDOW_MAX_TURNS, WINDOW_CHAR_BUDGET)
    }

    pub fn with_caps(max_turns: usize, char_budget: usize) -> Self {
        Self {
            turns: VecDeque::with_capacity(
                max_turns.min(64),
            ),
            max_turns: max_turns.max(1),
            char_budget,
        }
    }

    pub fn push(&mut self, role: Role, text: String) {
        self.turns.push_back((role, text));
        while self.turns.len() > self.max_turns {
            self.turns.pop_front();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    pub fn len(&self) -> usize {
        self.turns.len()
    }

    /// Assemble the relevance query: the last
    /// `window_turns - 1` prior turns (oldest → newest), then
    /// `current` LAST so it dominates the embedding. Output is
    /// trimmed to `char_budget` by dropping the *oldest* prior
    /// turns first; `current` is sacrosanct and never
    /// truncated. `window_turns == 0 | 1` → just `current`.
    pub fn assemble(
        &self,
        window_turns: usize,
        current: &str,
    ) -> String {
        if window_turns <= 1 || self.turns.is_empty() {
            return current.to_string();
        }
        // The last (window_turns - 1) stored entries, in their
        // natural (oldest → newest) order.
        let want_prior = window_turns - 1;
        let take = want_prior.min(self.turns.len());
        let start = self.turns.len() - take;
        let mut prior: Vec<&(Role, String)> =
            self.turns.range(start..).collect();
        // Char-budget trim: keep dropping the OLDEST prior
        // entries until the assembled prefix + the current
        // tail fits. `current` is never truncated.
        let current_tail = format!(
            "\n{}: {current}",
            Role::User.label()
        );
        loop {
            let mut out = String::new();
            for (role, text) in &prior {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(role.label());
                out.push_str(": ");
                out.push_str(text);
            }
            let total = out.len() + current_tail.len();
            if total <= self.char_budget || prior.is_empty() {
                if !out.is_empty() {
                    out.push_str(&current_tail);
                } else {
                    // No prior survived the trim — just
                    // current, unlabelled (byte-identical to
                    // the pre-Phase-86 single-message path).
                    out.push_str(current);
                }
                return out;
            }
            prior.remove(0); // drop oldest, retry
        }
    }
}

impl Default for ConversationWindow {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-session windows keyed by `SessionId`. The daemon builds
/// one of these at startup and threads it (via builders) into
/// both relevance consumers + into the turn-completion write
/// site.
pub type SharedConversationWindows =
    Arc<RwLock<HashMap<SessionId, ConversationWindow>>>;

/// Construct an empty shared windows map.
pub fn shared_conversation_windows() -> SharedConversationWindows
{
    Arc::new(RwLock::new(HashMap::new()))
}

/// Record a completed turn (user input → assistant final) into
/// the session's window. Best-effort: a lock failure / empty
/// text is silently skipped (a missed write costs one cycle of
/// signal, never the turn).
pub fn record_turn(
    shared: &SharedConversationWindows,
    session_id: SessionId,
    user_text: &str,
    assistant_text: &str,
) {
    if user_text.is_empty() && assistant_text.is_empty() {
        return;
    }
    let Ok(mut map) = shared.write() else {
        return;
    };
    let w = map
        .entry(session_id)
        .or_insert_with(ConversationWindow::new);
    if !user_text.is_empty() {
        w.push(Role::User, user_text.to_string());
    }
    if !assistant_text.is_empty() {
        w.push(Role::Assistant, assistant_text.to_string());
    }
}

/// Read-only assembly helper for the relevance providers.
/// `None` when `window_turns <= 1` (the Phase 86 opt-in floor),
/// no handle, no per-session entry, or an empty window — every
/// such case falls through to byte-identical pre-Phase-86
/// single-message embedding.
pub fn assemble_for(
    shared: Option<&SharedConversationWindows>,
    session_id: SessionId,
    window_turns: usize,
    current: &str,
) -> Option<String> {
    if window_turns <= 1 {
        return None;
    }
    let map = shared?.read().ok()?;
    let w = map.get(&session_id)?;
    if w.is_empty() {
        return None;
    }
    Some(w.assemble(window_turns, current))
}

// ---------------------------------------------------------------------------
// Chapter Thread — conversation-history replay over the shared windows
// ---------------------------------------------------------------------------

/// [`ConversationSeeder`] implementation over the same
/// [`SharedConversationWindows`] the daemon's turn-completion hook
/// already writes. The planner calls `prior_turns` once per turn; we
/// hand back the session's most recent `max_messages` window entries
/// (oldest first), trimmed oldest-first to [`WINDOW_CHAR_BUDGET`] so a
/// long-winded prior answer can't flood the prompt. Sessions with no
/// window (first turn, trigger-fired turns — never recorded) return
/// empty, which the planner treats as the fresh-context no-op.
pub struct WindowConversationSeeder {
    windows: SharedConversationWindows,
    max_messages: usize,
}

impl WindowConversationSeeder {
    pub fn new(
        windows: SharedConversationWindows,
        max_messages: usize,
    ) -> Self {
        Self { windows, max_messages }
    }
}

#[async_trait::async_trait]
impl aivyx_core::llm_planner::ConversationSeeder
    for WindowConversationSeeder
{
    async fn prior_turns(
        &self,
        session_id: SessionId,
    ) -> Vec<aivyx_core::llm_planner::PriorTurn> {
        use aivyx_core::llm_planner::PriorTurn;
        if self.max_messages == 0 {
            return Vec::new();
        }
        let Ok(map) = self.windows.read() else {
            return Vec::new();
        };
        let Some(w) = map.get(&session_id) else {
            return Vec::new();
        };
        // Newest-first accumulation under the char budget, then
        // reverse back to oldest-first — dropping from the OLD end
        // keeps the most recent context when trimming.
        let mut picked: Vec<PriorTurn> = Vec::new();
        let mut budget = WINDOW_CHAR_BUDGET;
        for (role, text) in
            w.turns.iter().rev().take(self.max_messages)
        {
            let cost = text.chars().count();
            if cost > budget {
                break;
            }
            budget -= cost;
            picked.push(PriorTurn {
                is_user: matches!(role, Role::User),
                text: text.clone(),
            });
        }
        picked.reverse();
        picked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_evicts_oldest_past_cap() {
        let mut w = ConversationWindow::with_caps(3, 10_000);
        w.push(Role::User, "a".into());
        w.push(Role::Assistant, "b".into());
        w.push(Role::User, "c".into());
        w.push(Role::Assistant, "d".into()); // evicts a
        assert_eq!(w.len(), 3);
        // The oldest survivor is "b" now.
        let out = w.assemble(4, "current");
        assert!(out.starts_with("assistant: b\n"));
        assert!(out.ends_with("\nuser: current"));
    }

    #[test]
    fn assemble_recency_order_with_current_last() {
        let mut w = ConversationWindow::with_caps(8, 10_000);
        w.push(Role::User, "u1".into());
        w.push(Role::Assistant, "a1".into());
        w.push(Role::User, "u2".into());
        w.push(Role::Assistant, "a2".into());
        // window_turns = 3 → 2 prior + current.
        let out = w.assemble(3, "q");
        assert_eq!(
            out,
            "user: u2\nassistant: a2\nuser: q"
        );
    }

    #[test]
    fn window_one_returns_just_current() {
        let mut w = ConversationWindow::new();
        w.push(Role::User, "prior".into());
        assert_eq!(w.assemble(1, "now"), "now");
        assert_eq!(w.assemble(0, "now"), "now");
    }

    #[test]
    fn empty_window_returns_just_current() {
        let w = ConversationWindow::new();
        assert_eq!(w.assemble(8, "now"), "now");
    }

    #[test]
    fn char_budget_drops_oldest_prior_first() {
        // budget 25 chars; current "q" (8 chars w/ tail) +
        // assistant: yyy (14) = 22 fits; +user: xxx (10) → 32
        // doesn't. So "user: xxx" (the oldest prior) is dropped.
        let mut w = ConversationWindow::with_caps(8, 25);
        w.push(Role::User, "xxx".into());
        w.push(Role::Assistant, "yyy".into());
        let out = w.assemble(3, "q");
        assert!(
            !out.contains("xxx"),
            "oldest prior must be dropped to fit: {out}"
        );
        assert!(out.contains("yyy"));
        assert!(out.ends_with("\nuser: q"));
        assert!(out.len() <= 25);
    }

    #[test]
    fn current_never_truncated_even_if_over_budget() {
        // budget smaller than current alone → just current,
        // unlabelled (pre-Phase-86 byte-identical path).
        let mut w = ConversationWindow::with_caps(8, 4);
        w.push(Role::User, "anything".into());
        let big = "the operator's whole message here";
        assert_eq!(w.assemble(8, big), big);
    }

    #[test]
    fn record_turn_skips_blank_and_writes_pair() {
        let shared = shared_conversation_windows();
        let sid = SessionId::new();
        record_turn(&shared, sid, "hello", "hi there");
        record_turn(&shared, sid, "", ""); // blank → skipped
        let map = shared.read().unwrap();
        let w = map.get(&sid).unwrap();
        assert_eq!(w.len(), 2);
    }

    #[tokio::test]
    async fn seeder_returns_prior_turns_oldest_first() {
        use aivyx_core::llm_planner::ConversationSeeder;
        let shared = shared_conversation_windows();
        let sid = SessionId::new();
        record_turn(&shared, sid, "q1", "a1");
        record_turn(&shared, sid, "q2", "a2");
        let seeder = WindowConversationSeeder::new(shared, 8);
        let prior = seeder.prior_turns(sid).await;
        let texts: Vec<(bool, &str)> = prior
            .iter()
            .map(|t| (t.is_user, t.text.as_str()))
            .collect();
        assert_eq!(
            texts,
            vec![(true, "q1"), (false, "a1"), (true, "q2"), (false, "a2")]
        );
    }

    #[tokio::test]
    async fn seeder_caps_at_max_messages_keeping_newest() {
        use aivyx_core::llm_planner::ConversationSeeder;
        let shared = shared_conversation_windows();
        let sid = SessionId::new();
        record_turn(&shared, sid, "q1", "a1");
        record_turn(&shared, sid, "q2", "a2");
        let seeder = WindowConversationSeeder::new(shared, 2);
        let prior = seeder.prior_turns(sid).await;
        let texts: Vec<&str> =
            prior.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, vec!["q2", "a2"]);
    }

    #[tokio::test]
    async fn seeder_unknown_session_and_zero_max_are_empty() {
        use aivyx_core::llm_planner::ConversationSeeder;
        let shared = shared_conversation_windows();
        let sid = SessionId::new();
        let seeder =
            WindowConversationSeeder::new(shared.clone(), 8);
        assert!(seeder.prior_turns(sid).await.is_empty());
        record_turn(&shared, sid, "q", "a");
        let disabled = WindowConversationSeeder::new(shared, 0);
        assert!(disabled.prior_turns(sid).await.is_empty());
    }

    #[tokio::test]
    async fn seeder_char_budget_drops_from_the_old_end() {
        use aivyx_core::llm_planner::ConversationSeeder;
        let shared = shared_conversation_windows();
        let sid = SessionId::new();
        let long = "x".repeat(WINDOW_CHAR_BUDGET);
        record_turn(&shared, sid, &long, "short answer");
        record_turn(&shared, sid, "recent q", "recent a");
        let seeder = WindowConversationSeeder::new(shared, 8);
        let prior = seeder.prior_turns(sid).await;
        // The budget-blowing OLD entry is dropped; the newest
        // survive.
        assert!(prior.iter().all(|t| t.text != long));
        assert!(prior.iter().any(|t| t.text == "recent a"));
    }

    #[test]
    fn assemble_for_returns_none_in_fallback_cases() {
        let shared = shared_conversation_windows();
        let sid = SessionId::new();
        // window=1 → None (opt-in floor).
        assert!(
            assemble_for(Some(&shared), sid, 1, "q").is_none()
        );
        // No handle → None.
        assert!(assemble_for(None, sid, 5, "q").is_none());
        // Unknown session → None.
        assert!(
            assemble_for(Some(&shared), sid, 5, "q").is_none()
        );
        // Populated session → Some(assembled).
        record_turn(&shared, sid, "prior u", "prior a");
        let out =
            assemble_for(Some(&shared), sid, 3, "q").unwrap();
        assert!(out.contains("prior u"));
        assert!(out.contains("prior a"));
        assert!(out.ends_with("\nuser: q"));
    }
}

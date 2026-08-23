//! Piece C (2026-08-23) — client-side (channel-adapter) state for
//! `/team run <goal>`'s confirm-first flow and per-channel rate
//! limiting. Deliberately client-side, not daemon-side — see the
//! Piece C plan's own Global Constraints for why (rate-limiting and
//! confirm-tracking are UX/abuse-prevention, not the security
//! boundary; the actual authorization check lives in
//! `daemon_server.rs`, enforced independently of any of this state).
//!
//! No confirm-first/pending-state precedent existed anywhere in this
//! codebase before this module (re-verified via a repo-wide grep
//! during planning) — this is a fresh, deliberately minimal pattern:
//! one `Option<PendingTrigger>` and one `Vec<Instant>` per chat,
//! held locally in each channel's own per-chat daemon-frontend loop
//! (no cross-restart persistence, no shared/global state).

use std::time::{Duration, Instant};

/// How long a `/team run` confirmation prompt stays valid before it
/// must be re-asked. Matches the design's own "e.g. 5 minutes."
pub const PENDING_TRIGGER_TTL: Duration = Duration::from_secs(300);

/// An outstanding "start '<goal>' on the default team? Reply
/// yes/no." prompt for one chat, awaiting resolution.
#[derive(Debug, Clone)]
pub struct PendingTrigger {
    pub goal: String,
    pub created_at: Instant,
}

impl PendingTrigger {
    pub fn new(goal: impl Into<String>) -> Self {
        PendingTrigger {
            goal: goal.into(),
            created_at: Instant::now(),
        }
    }

    /// Whether this prompt is too old to honor a late "yes"/"no" for.
    pub fn is_expired(&self, now: Instant) -> bool {
        now.duration_since(self.created_at) >= PENDING_TRIGGER_TTL
    }
}

/// A bare "yes" / "no" reply, recognized only while a
/// [`PendingTrigger`] is outstanding for that chat — never as a
/// general-purpose command, since that would swallow ordinary
/// conversation. Case-insensitive, whitespace-trimmed, exact match
/// only (no "yeah"/"yep"/"sure" fuzziness, matching this codebase's
/// established preference for unambiguous, exact command parsing
/// over natural-language guessing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmReply {
    Yes,
    No,
}

pub fn parse_confirm_reply(text: &str) -> Option<ConfirmReply> {
    match text.trim().to_ascii_lowercase().as_str() {
        "yes" => Some(ConfirmReply::Yes),
        "no" => Some(ConfirmReply::No),
        _ => None,
    }
}

/// A rolling-hour sliding-window rate limiter. `history` holds the
/// timestamp of every *allowed* attempt; entries older than one hour
/// are pruned before checking. Returns `true` (and records `now`) iff
/// `history.len() < limit` after pruning; returns `false` (and does
/// not record) otherwise — a denied attempt never counts against a
/// future one.
pub fn check_and_record_trigger(history: &mut Vec<Instant>, limit: u32, now: Instant) -> bool {
    const WINDOW: Duration = Duration::from_secs(3600);
    history.retain(|&t| now.duration_since(t) < WINDOW);
    if history.len() as u32 >= limit {
        false
    } else {
        history.push(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn pending_trigger_is_not_expired_immediately() {
        let p = PendingTrigger::new("close the books");
        assert!(!p.is_expired(Instant::now()));
    }

    #[test]
    fn pending_trigger_expires_after_ttl() {
        let mut p = PendingTrigger::new("close the books");
        p.created_at = Instant::now() - PENDING_TRIGGER_TTL - Duration::from_secs(1);
        assert!(p.is_expired(Instant::now()));
    }

    #[test]
    fn pending_trigger_not_yet_expired_just_under_ttl() {
        let mut p = PendingTrigger::new("close the books");
        p.created_at = Instant::now() - PENDING_TRIGGER_TTL + Duration::from_secs(1);
        assert!(!p.is_expired(Instant::now()));
    }

    #[test]
    fn parse_confirm_reply_recognizes_yes_and_no_case_insensitively() {
        assert_eq!(parse_confirm_reply("yes"), Some(ConfirmReply::Yes));
        assert_eq!(parse_confirm_reply("Yes"), Some(ConfirmReply::Yes));
        assert_eq!(parse_confirm_reply("YES"), Some(ConfirmReply::Yes));
        assert_eq!(parse_confirm_reply("  yes  "), Some(ConfirmReply::Yes));
        assert_eq!(parse_confirm_reply("no"), Some(ConfirmReply::No));
        assert_eq!(parse_confirm_reply("No"), Some(ConfirmReply::No));
    }

    #[test]
    fn parse_confirm_reply_rejects_anything_else() {
        assert_eq!(parse_confirm_reply("yeah"), None);
        assert_eq!(parse_confirm_reply("nope"), None);
        assert_eq!(parse_confirm_reply(""), None);
        assert_eq!(parse_confirm_reply("/team status"), None);
    }

    #[test]
    fn check_and_record_trigger_allows_under_the_limit() {
        let mut history = Vec::new();
        let now = Instant::now();
        assert!(check_and_record_trigger(&mut history, 3, now));
        assert!(check_and_record_trigger(&mut history, 3, now));
        assert!(check_and_record_trigger(&mut history, 3, now));
        assert_eq!(history.len(), 3);
    }

    #[test]
    fn check_and_record_trigger_denies_at_the_limit() {
        let mut history = Vec::new();
        let now = Instant::now();
        for _ in 0..3 {
            assert!(check_and_record_trigger(&mut history, 3, now));
        }
        assert!(!check_and_record_trigger(&mut history, 3, now));
        assert_eq!(history.len(), 3, "a denied attempt is not recorded");
    }

    #[test]
    fn check_and_record_trigger_forgets_attempts_older_than_the_window() {
        let mut history = vec![Instant::now() - Duration::from_secs(3601)];
        let now = Instant::now();
        // The one stale entry ages out, so this new attempt is allowed
        // even at a limit of 1.
        assert!(check_and_record_trigger(&mut history, 1, now));
        assert_eq!(history.len(), 1, "the stale entry was pruned, not kept");
    }
}

//! Phase 111 — shared mission-gate text-command parser.
//!
//! Extracted at Phase 111 from `telegram_daemon_frontend.rs`
//! where it landed at Phase 21 as the Telegram-side
//! `/approve` / `/reject` parser. The Discord daemon-frontend
//! (Phase 111 Task 3) and Slack daemon-frontend (Phase 111
//! Task 5) need byte-identical parsing semantics — the Q-block
//! at Phase 111 Task 2 sign-off (Q2a — mirror Phase 19
//! exactly) confirmed the parser is genuinely identical
//! across all three adapters, so extracting it into a shared
//! helper is the right two-data-point-becomes-three move.
//!
//! ## What this parses
//!
//! Inbound text from a SemiTrusted adapter that exactly
//! matches `/approve <mission_id> <gate_id>` or
//! `/reject <mission_id> <gate_id>` (whitespace-trimmed,
//! exactly three space-separated tokens). Anything else
//! returns `None` and the caller treats it as a regular
//! turn-prompt message.
//!
//! ## What this does NOT parse
//!
//! - `/cancel` — that's the per-turn cancellation signal,
//!   not a gate resolution. Each daemon-frontend handles
//!   it separately (cancellation goes through `CancelTurn`
//!   on the IPC, not through gate resolution).
//! - Multi-word mission or gate ids. Mission and gate ids
//!   in Aivyx are always single tokens.
//! - Mention forms like `@my-bot /approve ...` for Slack
//!   or Discord. Adapters that need mention-stripping do
//!   it before calling [`parse`].

/// Parsed `/approve` / `/reject` gate resolution command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateCommand {
    pub mission_id: String,
    pub gate_id: String,
    /// `true` for `/approve`, `false` for `/reject`.
    pub approved: bool,
}

/// Parse `/approve <mission_id> <gate_id>` or
/// `/reject <mission_id> <gate_id>`. Returns `None` for any
/// other text. Input is expected to already be
/// whitespace-trimmed; adapters that need mention-stripping
/// (Slack `<@U...>`, Discord `<@!...>`) do so before calling.
pub fn parse(text: &str) -> Option<GateCommand> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() != 3 {
        return None;
    }
    let approved = match parts[0] {
        "/approve" => true,
        "/reject" => false,
        _ => return None,
    };
    Some(GateCommand {
        mission_id: parts[1].to_string(),
        gate_id: parts[2].to_string(),
        approved,
    })
}

#[cfg(test)]
mod gate_command_tests {
    use super::*;

    #[test]
    fn parse_approve_canonical() {
        let cmd = parse("/approve m-001 g-abc").unwrap();
        assert!(cmd.approved);
        assert_eq!(cmd.mission_id, "m-001");
        assert_eq!(cmd.gate_id, "g-abc");
    }

    #[test]
    fn parse_reject_canonical() {
        let cmd = parse("/reject m-002 g-xyz").unwrap();
        assert!(!cmd.approved);
        assert_eq!(cmd.mission_id, "m-002");
        assert_eq!(cmd.gate_id, "g-xyz");
    }

    #[test]
    fn parse_rejects_two_tokens() {
        assert!(parse("/approve m-001").is_none());
    }

    #[test]
    fn parse_rejects_four_tokens() {
        assert!(parse("/approve m-001 g-abc extra").is_none());
    }

    #[test]
    fn parse_rejects_unknown_command() {
        assert!(parse("/cancel").is_none());
        assert!(parse("/help m g").is_none());
        assert!(parse("hello world here").is_none());
    }

    #[test]
    fn parse_accepts_multiple_spaces_between_tokens() {
        // split_whitespace collapses runs.
        let cmd = parse("/approve   m-001   g-abc").unwrap();
        assert_eq!(cmd.mission_id, "m-001");
        assert_eq!(cmd.gate_id, "g-abc");
    }

    #[test]
    fn parse_empty_returns_none() {
        assert!(parse("").is_none());
        assert!(parse("   ").is_none());
    }
}

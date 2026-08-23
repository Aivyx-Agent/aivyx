//! Piece B (2026-08-23) — shared team-mission text-command parser.
//!
//! Mirrors `gate_command.rs`'s own shape and scope exactly: pure parsing
//! only, no I/O, no daemon dispatch (dispatch lives in `team_dispatch.rs`).
//! Telegram, Discord, and Slack daemon-frontends all call `parse` with the
//! same whitespace-trimmed message text and treat `None` as "fall through
//! to the normal chat-turn path."
//!
//! ## What this parses
//!
//! `/team `-prefixed commands, deliberately namespaced so they never
//! collide with the existing bare `/approve` / `/reject` (handled by
//! `gate_command.rs`, which targets the *old* single-agent Mission system,
//! not Nonagon team missions) or `/cancel` (per-turn cancellation, handled
//! separately by each daemon-frontend):
//!
//! - `/team status` — every mission.
//! - `/team status <mission_id>` — one mission's detail.
//! - `/team approve <mission_id> <step>` / `/team reject <mission_id> <step>`
//! - `/team pause <mission_id>` / `/team resume <mission_id>`
//! - `/team abort <mission_id>`
//!
//! Anything not starting with the `/team` token — bare `/status`, `/approve`
//! without the `team` token, unrelated text — returns `None`, so the caller
//! falls through to the normal chat-turn path. Anything that *does* start
//! with `/team` but doesn't match a known form (wrong argument count,
//! unknown subcommand, bare `/team`) returns `Some(TeamCommand::Usage)`
//! instead, so a typo'd team command never silently burns an LLM turn.

/// A parsed `/team ...` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamCommand {
    /// `/team status` (`None`) or `/team status <id>` (`Some(id)`).
    Status(Option<String>),
    /// `/team approve <id> <step>` (`approve: true`) or
    /// `/team reject <id> <step>` (`approve: false`).
    ResolveGate {
        mission_id: String,
        step: String,
        approve: bool,
    },
    /// `/team pause <id>`.
    Pause { mission_id: String },
    /// `/team resume <id>`.
    Resume { mission_id: String },
    /// `/team abort <id>`.
    Abort { mission_id: String },
    /// `/team run <goal>` — Piece C. `goal` is everything after `run`,
    /// re-joined with single spaces (multi-word goals are the norm;
    /// unlike every other variant's single-token ids).
    Run { goal: String },
    /// Recognized as a `/team` command but didn't match any known form —
    /// wrong argument count or unknown subcommand. Renders a usage hint
    /// instead of falling through to the chat-turn path.
    Usage,
}

/// Parse a `/team ...` command. Returns `None` for text that isn't
/// `/team`-prefixed at all (falls through to the chat-turn path), or
/// `Some(TeamCommand::Usage)` for `/team`-prefixed text that doesn't match
/// any known form. Input is expected to already be whitespace-trimmed;
/// adapters that need mention-stripping (Slack `<@U...>`, Discord `<@!...>`)
/// do so before calling, same contract as `gate_command::parse`.
pub fn parse(text: &str) -> Option<TeamCommand> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.first() != Some(&"/team") {
        return None;
    }
    if parts.get(1) == Some(&"run") {
        return if parts.len() >= 3 {
            Some(TeamCommand::Run {
                goal: parts[2..].join(" "),
            })
        } else {
            Some(TeamCommand::Usage)
        };
    }
    match parts.as_slice() {
        ["/team", "status"] => Some(TeamCommand::Status(None)),
        ["/team", "status", id] => Some(TeamCommand::Status(Some((*id).to_string()))),
        ["/team", "approve", id, step] => Some(TeamCommand::ResolveGate {
            mission_id: (*id).to_string(),
            step: (*step).to_string(),
            approve: true,
        }),
        ["/team", "reject", id, step] => Some(TeamCommand::ResolveGate {
            mission_id: (*id).to_string(),
            step: (*step).to_string(),
            approve: false,
        }),
        ["/team", "pause", id] => Some(TeamCommand::Pause {
            mission_id: (*id).to_string(),
        }),
        ["/team", "resume", id] => Some(TeamCommand::Resume {
            mission_id: (*id).to_string(),
        }),
        ["/team", "abort", id] => Some(TeamCommand::Abort {
            mission_id: (*id).to_string(),
        }),
        _ => Some(TeamCommand::Usage),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_bare_lists_all() {
        assert_eq!(parse("/team status"), Some(TeamCommand::Status(None)));
    }

    #[test]
    fn parse_status_with_id() {
        assert_eq!(
            parse("/team status m-001"),
            Some(TeamCommand::Status(Some("m-001".to_string())))
        );
    }

    #[test]
    fn parse_approve_canonical() {
        assert_eq!(
            parse("/team approve m-001 approve-step"),
            Some(TeamCommand::ResolveGate {
                mission_id: "m-001".to_string(),
                step: "approve-step".to_string(),
                approve: true,
            })
        );
    }

    #[test]
    fn parse_reject_canonical() {
        assert_eq!(
            parse("/team reject m-002 review"),
            Some(TeamCommand::ResolveGate {
                mission_id: "m-002".to_string(),
                step: "review".to_string(),
                approve: false,
            })
        );
    }

    #[test]
    fn parse_pause_canonical() {
        assert_eq!(
            parse("/team pause m-003"),
            Some(TeamCommand::Pause { mission_id: "m-003".to_string() })
        );
    }

    #[test]
    fn parse_resume_canonical() {
        assert_eq!(
            parse("/team resume m-004"),
            Some(TeamCommand::Resume { mission_id: "m-004".to_string() })
        );
    }

    #[test]
    fn parse_abort_canonical() {
        assert_eq!(
            parse("/team abort m-005"),
            Some(TeamCommand::Abort { mission_id: "m-005".to_string() })
        );
    }

    #[test]
    fn parse_rejects_missing_team_token() {
        assert!(parse("/status").is_none());
        assert!(parse("/approve m-001 g-abc").is_none());
        assert!(parse("status").is_none());
    }

    #[test]
    fn parse_wrong_arg_counts_return_usage() {
        assert_eq!(parse("/team approve m-001"), Some(TeamCommand::Usage));
        assert_eq!(
            parse("/team approve m-001 step extra"),
            Some(TeamCommand::Usage)
        );
        assert_eq!(parse("/team pause"), Some(TeamCommand::Usage));
        assert_eq!(
            parse("/team pause m-001 extra"),
            Some(TeamCommand::Usage)
        );
        assert_eq!(parse("/team status a b"), Some(TeamCommand::Usage));
    }

    #[test]
    fn parse_unknown_subcommand_returns_usage() {
        assert_eq!(parse("/team frobnicate m-001"), Some(TeamCommand::Usage));
        assert_eq!(parse("/team"), Some(TeamCommand::Usage));
    }

    #[test]
    fn parse_is_case_sensitive() {
        // "/TEAM" doesn't match the "/team" token at all, so it falls
        // through to the chat-turn path.
        assert!(parse("/TEAM status").is_none());
        // "/team" is present but the subcommand casing doesn't match any
        // known form, so these are Usage, not None.
        assert_eq!(parse("/team STATUS"), Some(TeamCommand::Usage));
        assert_eq!(
            parse("/team Approve m-001 s"),
            Some(TeamCommand::Usage)
        );
    }

    #[test]
    fn parse_accepts_multiple_spaces_between_tokens() {
        let cmd = parse("/team   approve   m-001   g-abc").unwrap();
        assert_eq!(
            cmd,
            TeamCommand::ResolveGate {
                mission_id: "m-001".to_string(),
                step: "g-abc".to_string(),
                approve: true,
            }
        );
    }

    #[test]
    fn parse_empty_and_unrelated_text_returns_none() {
        assert!(parse("").is_none());
        assert!(parse("   ").is_none());
        assert!(parse("hello team status").is_none());
    }

    #[test]
    fn parse_run_canonical_single_word_goal() {
        assert_eq!(
            parse("/team run close-the-books"),
            Some(TeamCommand::Run {
                goal: "close-the-books".to_string()
            })
        );
    }

    #[test]
    fn parse_run_joins_multi_word_goal() {
        assert_eq!(
            parse("/team run close the books"),
            Some(TeamCommand::Run {
                goal: "close the books".to_string()
            })
        );
    }

    #[test]
    fn parse_run_collapses_internal_multiple_spaces() {
        // split_whitespace collapses runs — matches this file's own
        // existing parse_accepts_multiple_spaces_between_tokens precedent.
        assert_eq!(
            parse("/team run close   the   books"),
            Some(TeamCommand::Run {
                goal: "close the books".to_string()
            })
        );
    }

    #[test]
    fn parse_run_with_no_goal_is_usage() {
        assert_eq!(parse("/team run"), Some(TeamCommand::Usage));
    }
}

//! Chapter Candor — detect "claimed-but-didn't" in a completed turn.
//!
//! A tougher dogfood pass (#12) found the agent telling the operator it had
//! *saved something to memory* when no `memory.write` call happened that turn —
//! a quiet shortfall the operator only catches by chance, and exactly what the
//! agent's own honesty contract ("never hide what you did or paper over a
//! failure") forbids. Interactive turns have no acceptance verification (that's
//! Chapter Verdict, loop-only), so this is a lightweight backstop: after a turn
//! completes, compare what the final message *claims* against the tools it
//! actually *called*, and append an honest note when a concrete action was
//! claimed but its tool was never invoked.
//!
//! **Deliberately conservative.** Each rule is a set of past-tense completion
//! phrases bound to the one concrete tool that fulfils them; a rule fires only
//! when a claim phrase is present AND no fulfilling tool was called. It does NOT
//! block the turn — it only annotates — so a rare false positive costs a stray
//! "I may not have…" line, not a broken turn. Categories are data; add more as
//! the patterns prove out.

/// One claim→action rule.
struct ClaimRule {
    /// Lowercased past-tense completion phrases that assert the action was done.
    phrases: &'static [&'static str],
    /// Tool name(s) whose invocation would actually fulfil the claim.
    fulfilling_tools: &'static [&'static str],
    /// First-person note appended when the claim is unfulfilled.
    note: &'static str,
}

const RULES: &[ClaimRule] = &[
    ClaimRule {
        phrases: &[
            "saved to memory",
            "saved it to memory",
            "saved that to memory",
            "saved this to memory",
            "added to memory",
            "added it to memory",
            "stored in memory",
            "recorded in memory",
            "to memory under",
            "saved to your memory",
            "i've remembered",
            "i have remembered",
            "noted that in memory",
        ],
        fulfilling_tools: &["memory.write"],
        note: "I mentioned saving to memory, but I didn't actually record it this \
               turn — please ask me again if you want it saved.",
    },
    ClaimRule {
        phrases: &[
            "i've scheduled",
            "i have scheduled",
            "scheduled a",
            "created a routine",
            "set up a routine",
            "set a reminder",
            "added a schedule",
        ],
        fulfilling_tools: &["schedule.create"],
        note: "I mentioned scheduling something, but I didn't actually create a \
               schedule this turn.",
    },
    ClaimRule {
        phrases: &[
            "sent you a notification",
            "sent a notification",
            "i've notified you",
            "i have notified you",
            "sent the notification",
        ],
        fulfilling_tools: &["notify.send"],
        note: "I mentioned sending a notification, but I didn't actually send one \
               this turn.",
    },
];

/// Compare a completed turn's `final_message` against the `called_tools` (tool
/// names invoked during the turn) and return an honest note for each concrete
/// action that was claimed but whose fulfilling tool was never called. Empty
/// when everything claimed was backed by a real call (the common case). Pure.
pub fn detect_unfulfilled_claims(final_message: &str, called_tools: &[String]) -> Vec<String> {
    let msg = final_message.to_lowercase();
    let mut notes = Vec::new();
    for rule in RULES {
        let claimed = rule.phrases.iter().any(|p| msg.contains(p));
        if !claimed {
            continue;
        }
        let fulfilled = called_tools
            .iter()
            .any(|t| rule.fulfilling_tools.contains(&t.as_str()));
        if !fulfilled {
            notes.push(rule.note.to_string());
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_a_memory_claim_with_no_memory_write() {
        // The #12 case: claims a save, but only called a read/search tool.
        let notes = detect_unfulfilled_claims(
            "I researched it and saved that to memory under aviation-notes.",
            &tools(&["web.search", "memory.read"]),
        );
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("saving to memory"));
    }

    #[test]
    fn no_flag_when_the_claim_is_fulfilled() {
        let notes = detect_unfulfilled_claims(
            "Done — saved to memory.",
            &tools(&["memory.write"]),
        );
        assert!(notes.is_empty());
    }

    #[test]
    fn no_flag_without_a_claim() {
        let notes = detect_unfulfilled_claims(
            "VFR means Visual Flight Rules. Wind is 230 at 12 knots.",
            &tools(&["aviation.get_metar"]),
        );
        assert!(notes.is_empty());
    }

    #[test]
    fn does_not_flag_a_future_tense_offer() {
        // "I can save this to memory if you'd like" is an offer, not a claim —
        // it doesn't match the past-tense completion phrases.
        let notes = detect_unfulfilled_claims(
            "I can save this to memory if you'd like me to.",
            &tools(&[]),
        );
        assert!(notes.is_empty());
    }

    #[test]
    fn flags_schedule_and_notify_claims() {
        let s = detect_unfulfilled_claims("I've scheduled a daily review.", &tools(&[]));
        assert_eq!(s.len(), 1);
        assert!(s[0].contains("scheduling"));
        let n = detect_unfulfilled_claims(
            "All set — I've notified you of the change.",
            &tools(&["memory.write"]),
        );
        assert_eq!(n.len(), 1);
        assert!(n[0].contains("notification"));
    }

    #[test]
    fn multiple_unfulfilled_claims_each_flag() {
        let notes = detect_unfulfilled_claims(
            "I saved to memory and scheduled a follow-up.",
            &tools(&[]),
        );
        assert_eq!(notes.len(), 2);
    }
}

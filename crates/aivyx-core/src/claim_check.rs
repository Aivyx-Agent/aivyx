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
/// Chapter Strop (ST.1) — whether `final_message` carries a Candor
/// annotation (the turn loop appends "⚠ {note}" for each unfulfilled
/// claim before building `TurnOutcome::Completed`). Downstream consumers
/// (the skill-effectiveness fold) read the verdict from the message
/// instead of re-deriving it: the turn loop computed it with
/// registry-accurate tool names, which the audit slice can't reconstruct
/// (`ToolCall` entries carry `tool_id`, not the name). Matches the exact
/// finite note strings from `RULES`, so organic model text can't
/// false-positive. Pure.
pub fn has_unfulfilled_claim_annotation(final_message: &str) -> bool {
    RULES
        .iter()
        .any(|r| final_message.contains(&format!("⚠ {}", r.note)))
}

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

/// POLISH_WAVES.md sub-project 4, item E — a Candor-adjacent identifier-
/// fidelity check. Distinct from `detect_unfulfilled_claims`'s phrase-
/// matching `RULES` above: this compares REPLY TOKENS against
/// identifiers the turn's own tool calls surfaced, flagging a token
/// that's a single-character slip away from the source (three
/// independent live repros: an aircraft registration, a METAR wind
/// group, and an ICAO-code transposition family — see
/// `docs/VITRINE.md` §2b).
///
/// Turn-scoped only — `source_texts` is this turn's own tool-result
/// text (via `TurnPlanner::tool_result_texts`), never global memory —
/// which bounds both cost and false-positive surface. Conservative:
/// only an exact single-edit mismatch flags (not "similar"), and only
/// identifier-shaped tokens ever enter either pool. Does not block the
/// turn — same non-blocking posture as `detect_unfulfilled_claims`.
pub fn detect_identifier_drift(final_message: &str, source_texts: &[String]) -> Vec<String> {
    let source_pool: std::collections::HashSet<String> = source_texts
        .iter()
        .flat_map(|t| identifier_tokens(t))
        .collect();
    if source_pool.is_empty() {
        return Vec::new();
    }
    let mut notes = Vec::new();
    let mut flagged: std::collections::HashSet<String> = std::collections::HashSet::new();
    for token in identifier_tokens(final_message) {
        if source_pool.contains(&token) || flagged.contains(&token) {
            continue;
        }
        if let Some(closest) = source_pool
            .iter()
            .find(|candidate| edit_distance_is_one(&token, candidate))
        {
            notes.push(format!(
                "I wrote '{token}' but the source said '{closest}' — please double-check this identifier."
            ));
            flagged.insert(token);
        }
    }
    notes
}

/// Identifier-shaped tokens: alphanumeric-and-hyphen runs, 4-64
/// characters, that look like a registration/code rather than an
/// ordinary word — a hyphen (with at least one digit or letter), a
/// digit-and-letter mix, or being exactly 4 characters and fully
/// uppercase all qualify. Covers "VH-EZT" (hyphen), "22012KT"
/// (digit+letter), and 4-letter ICAO codes like "YPJT" (uppercase) —
/// no single shared shape covers all three, so the conditions are
/// combined with OR. Pure numbers ("3000") and pure punctuation runs
/// ("-----") are deliberately excluded — see the inline comments below
/// for why. Pure.
fn identifier_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            current.push(ch);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
        .into_iter()
        .filter(|t| {
            let len = t.chars().count();
            // POLISH_WAVES.md sub-project 4 final-review fix — the old
            // "all-uppercase" check was vacuously true for tokens with
            // NO alphabetic characters at all (nothing to violate the
            // uppercase rule), so pure numbers like "3000"/"2025" were
            // wrongly treated as identifiers, producing false
            // "double-check this identifier" warnings on correct
            // arithmetic. Also caps identifier length: real
            // identifiers are short, and an unbounded length here fed
            // an O(n*m) edit-distance check with no upper bound.
            if !(4..=64).contains(&len) {
                return false;
            }
            let has_digit = t.chars().any(|c| c.is_ascii_digit());
            let has_alpha = t.chars().any(|c| c.is_ascii_alphabetic());
            // Re-review fix — the hyphen branch was unguarded, so a
            // token with NO alphanumeric characters at all (a markdown
            // table separator like "-----", a horizontal rule) still
            // qualified as an "identifier" whenever its dash count
            // differed from a look-alike in the source pool. Requiring
            // at least one digit or letter still admits real
            // hyphenated identifiers (VH-EZT) and dates
            // (2026-08-28 vs 2026-08-27, where flagging a drifted date
            // is arguably desirable) while excluding pure punctuation.
            let is_hyphenated = t.contains('-') && (has_digit || has_alpha);
            // Exactly-4-char all-uppercase is ICAO/tail-number-code
            // shaped (e.g. "YPJT"); restricting to length 4 (rather
            // than "any all-uppercase run") excludes ordinary longer
            // acronyms/words like "HTTPS" while still catching real
            // 4-letter codes. A residual collision with genuine
            // 4-letter uppercase acronyms (e.g. "HTTP") is an accepted,
            // documented tradeoff — the same class of risk as the
            // "TODO" acronym case already noted in this function's own
            // design doc.
            let is_icao_like = len == 4
                && has_alpha
                && t.chars()
                    .all(|c| !c.is_ascii_alphabetic() || c.is_ascii_uppercase());
            (has_digit && has_alpha) || is_hyphenated || is_icao_like
        })
        .collect()
}

/// `true` iff `a` and `b` differ by exactly one single-character edit
/// (insertion, deletion, or substitution) — Levenshtein distance == 1.
/// Identical strings return `false` (distance 0, not 1). Full DP rather
/// than a hand-rolled early-exit: identifier tokens here are a handful
/// of characters, so O(len(a) * len(b)) is negligible, and DP is less
/// error-prone than enumerating edit cases by hand.
fn edit_distance_is_one(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut curr = vec![0usize; b.len() + 1];
        curr[0] = i;
        for j in 1..=b.len() {
            curr[j] = if a[i - 1] == b[j - 1] {
                prev[j - 1]
            } else {
                1 + prev[j - 1].min(prev[j]).min(curr[j - 1])
            };
        }
        prev = curr;
    }
    prev[b.len()] == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    // ---- Chapter Strop (ST.1) — annotation detection ----------------

    #[test]
    fn annotation_detection_keys_on_candors_verdict_not_raw_claims() {
        // A raw unfulfilled-claim SENTENCE without the annotation is not
        // a verdict — the fold must key on what Candor concluded (with
        // registry-accurate tool names), not re-guess from prose.
        let raw_claim_only = "I researched it and saved that to memory.";
        let notes = detect_unfulfilled_claims(raw_claim_only, &tools(&["web.search"]));
        assert_eq!(notes.len(), 1);
        // Build the annotated message exactly the way the turn loop
        // does ("\n⚠ {note}" appended), so this test tracks the real
        // append shape rather than a hardcoded copy of the note text.
        let annotated = format!("{raw_claim_only}\n\n⚠ {}", notes[0]);
        // An organic warning glyph with non-RULES text is not a verdict.
        let organic_warning = "⚠ the disk is 90% full — consider a cleanup.";

        assert!(has_unfulfilled_claim_annotation(&annotated));
        assert!(!has_unfulfilled_claim_annotation(raw_claim_only));
        assert!(!has_unfulfilled_claim_annotation(organic_warning));
        assert!(!has_unfulfilled_claim_annotation(""));
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
        let notes = detect_unfulfilled_claims("Done — saved to memory.", &tools(&["memory.write"]));
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
        let notes =
            detect_unfulfilled_claims("I saved to memory and scheduled a follow-up.", &tools(&[]));
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn identifier_drift_flags_single_character_slip() {
        let sources = vec!["Aircraft VH-EZT is currently on the ramp.".to_string()];
        let notes = detect_identifier_drift("The aircraft in question is VH-EQT.", &sources);
        assert_eq!(notes.len(), 1);
        assert!(
            notes[0].contains("VH-EQT") && notes[0].contains("VH-EZT"),
            "{notes:?}"
        );
    }

    #[test]
    fn identifier_drift_does_not_flag_exact_match() {
        let sources = vec!["Aircraft VH-EZT is currently on the ramp.".to_string()];
        let notes = detect_identifier_drift("The aircraft in question is VH-EZT.", &sources);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn identifier_drift_does_not_flag_unrelated_tokens() {
        let sources = vec!["Aircraft VH-EZT is currently on the ramp.".to_string()];
        let notes = detect_identifier_drift("Everything checks out fine today.", &sources);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn identifier_drift_ignores_short_and_lowercase_words() {
        // Nothing identifier-shaped in either pool — no false positive
        // from ordinary lowercase words even ones that are a
        // single-character edit apart ("cat"/"car" are both filtered
        // out: lowercase, no digit, no hyphen).
        let sources = vec!["the cat sat on the mat".to_string()];
        let notes = detect_identifier_drift("the car sat on the mat", &sources);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn identifier_drift_is_turn_scoped_only() {
        // An empty source pool (no tool calls this turn) never flags,
        // regardless of what final_message contains.
        let notes = detect_identifier_drift("VH-EQT departed on schedule.", &[]);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn identifier_tokens_excludes_ordinary_mixed_case_words() {
        // "This" is 4 letters but mixed-case — never an identifier
        // candidate.
        assert!(identifier_tokens("This is a test").is_empty());
    }

    #[test]
    fn identifier_tokens_excludes_pure_punctuation_runs() {
        // Re-review fix — a markdown table separator/horizontal rule
        // has no alphanumeric characters at all and must never qualify
        // as an "identifier," even though it contains a hyphen.
        assert!(identifier_tokens("-----").is_empty());
        assert!(identifier_tokens("----").is_empty());
    }

    #[test]
    fn identifier_drift_does_not_flag_markdown_table_separators() {
        let sources = vec!["| col |\n| ---- |".to_string()];
        let notes = detect_identifier_drift("| col |\n| ----- |", &sources);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn identifier_tokens_admits_hyphenated_dates() {
        // A hyphenated run with digits (a date) still qualifies —
        // flagging a drifted date is desirable, unlike pure punctuation.
        assert_eq!(
            identifier_tokens("filed on 2026-08-28"),
            vec!["2026-08-28".to_string()]
        );
    }

    #[test]
    fn identifier_tokens_admits_icao_style_codes() {
        assert_eq!(
            identifier_tokens("departing YPJT today"),
            vec!["YPJT".to_string()]
        );
    }

    #[test]
    fn identifier_drift_does_not_flag_pure_number_arithmetic() {
        let sources = vec!["Revenue was 1000 and costs were 2000.".to_string()];
        let notes = detect_identifier_drift("Total revenue was 3000 dollars.", &sources);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn identifier_tokens_excludes_pure_digit_runs() {
        assert!(identifier_tokens("the total was 3000 today").is_empty());
    }

    #[test]
    fn identifier_tokens_excludes_long_uppercase_acronyms() {
        // Only exactly-4-char uppercase runs are ICAO-code-shaped;
        // longer ones (ordinary acronyms/words) don't qualify.
        assert!(identifier_tokens("connect over HTTPS please").is_empty());
    }

    #[test]
    fn identifier_tokens_length_capped() {
        let long_run = "A".repeat(100);
        assert!(identifier_tokens(&long_run).is_empty());
    }

    #[test]
    fn edit_distance_is_one_matches_substitution_insertion_and_deletion() {
        assert!(edit_distance_is_one("VH-EZT", "VH-EQT")); // substitution
        assert!(edit_distance_is_one("YPJT", "YPJ")); // deletion
        assert!(edit_distance_is_one("YPJ", "YPJT")); // insertion
        assert!(!edit_distance_is_one("YPJT", "YPJT")); // identical -> distance 0
        assert!(!edit_distance_is_one("YPJT", "YSSY")); // distance > 1
    }
}

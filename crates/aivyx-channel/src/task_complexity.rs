//! Chapter Foreman — a deterministic task-complexity heuristic for the
//! autonomous loop's auto-delegation gate.
//!
//! The loop→team path already exists (the loop prompt offers `team.run`), but it
//! relies on the *model* choosing to delegate — which small local models
//! reliably won't (the dogfood finding). This scores a backlog story by pure
//! structural signals so the loop driver can decide to hand a clearly multi-part
//! story to the team **without** asking the model. Opt-in
//! (`[loop] delegate_above`); a story scoring `>= threshold` is delegated.
//!
//! **Honest scope:** this is a coarse heuristic over text, not an understanding
//! of the work. A high score means "looks like several independent pieces," not
//! "needs a team." It's deliberately conservative (default off; operator sets
//! the threshold) — a wrong *delegate* spends a mission, a wrong *solo* is just a
//! normal iteration. The `signals` are surfaced so the operator sees *why* a
//! story scored as it did.

/// The action verbs that, when several distinct ones appear, suggest the task
/// bundles independent pieces of work.
const ACTION_VERBS: &[&str] = &[
    "build", "create", "write", "implement", "add", "test", "deploy", "research",
    "analyze", "design", "refactor", "fix", "document", "review", "integrate",
    "migrate", "investigate", "benchmark", "audit", "compare", "summarize",
];

/// Clause joiners that hint a task strings several actions together.
const JOINERS: &[&str] = &[" and ", " then ", "; ", ", and ", " after ", " plus ", " as well as "];

/// The verdict for one story: a score and the human-readable signals behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplexityAssessment {
    pub score: u32,
    pub signals: Vec<String>,
}

impl ComplexityAssessment {
    /// Whether this story should be delegated given the operator's threshold.
    pub fn should_delegate(&self, threshold: u32) -> bool {
        self.score >= threshold
    }

    /// A one-line, operator-readable explanation (for loop notes / logs).
    pub fn explain(&self) -> String {
        if self.signals.is_empty() {
            format!("complexity {} (no multi-part signals)", self.score)
        } else {
            format!("complexity {} — {}", self.score, self.signals.join(", "))
        }
    }
}

/// Score a backlog story (`title` + `body`/acceptance criteria) by structural
/// signals. Pure + deterministic. Weights:
/// - each enumerated list item in the body: **2** (explicit separate pieces),
/// - each clause joiner ("and"/"then"/";"/…): **1** (chained actions),
/// - each *distinct* action verb beyond the first: **1** (multiple kinds of work),
/// - a long body (>400 chars): **+2**, (>800): **+4** total (sizeable scope).
pub fn assess(title: &str, body: &str) -> ComplexityAssessment {
    let mut score = 0u32;
    let mut signals = Vec::new();
    let hay = format!("{title}\n{body}").to_lowercase();

    // 1. Enumerated list items in the body — the clearest "N separate pieces".
    let list_items = body
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("- ")
                || t.starts_with("* ")
                || t.starts_with("• ")
                || t.chars().next().is_some_and(|c| c.is_ascii_digit())
                    && t.trim_start_matches(|c: char| c.is_ascii_digit())
                        .starts_with(['.', ')'])
        })
        .count() as u32;
    if list_items >= 2 {
        score += list_items * 2;
        signals.push(format!("{list_items} enumerated item(s)"));
    }

    // 2. Clause joiners stringing actions together.
    let joiners: u32 = JOINERS.iter().map(|j| hay.matches(j).count() as u32).sum();
    if joiners >= 1 {
        score += joiners;
        signals.push(format!("{joiners} clause joiner(s)"));
    }

    // 3. Distinct action verbs — multiple *kinds* of work. The first is free
    //    (every task has at least one verb); extras add to the score.
    let words: Vec<&str> = hay.split(|c: char| !c.is_ascii_alphabetic()).collect();
    let distinct_verbs = ACTION_VERBS
        .iter()
        .filter(|v| {
            // Match the verb or a simple inflection (test/tests/tested/testing),
            // on a word boundary — avoids "addition" matching "add".
            words.iter().any(|w| {
                *w == **v
                    || *w == format!("{v}s")
                    || *w == format!("{v}d")
                    || *w == format!("{v}ed")
                    || *w == format!("{v}ing")
            })
        })
        .count() as u32;
    if distinct_verbs >= 2 {
        let extra = distinct_verbs - 1;
        score += extra;
        signals.push(format!("{distinct_verbs} distinct action verbs"));
    }

    // 4. Sizeable scope by body length.
    let len = body.len();
    if len > 800 {
        score += 4;
        signals.push("large body (>800 chars)".to_string());
    } else if len > 400 {
        score += 2;
        signals.push("long body (>400 chars)".to_string());
    }

    ComplexityAssessment { score, signals }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trivial_one_liner_scores_low() {
        let a = assess("Fix a typo in the README", "");
        assert!(a.score <= 1, "got {} ({:?})", a.score, a.signals);
        assert!(!a.should_delegate(5));
    }

    #[test]
    fn a_chained_multi_verb_task_scores_high() {
        let a = assess(
            "Build the export API, write integration tests, and deploy it to staging",
            "",
        );
        // verbs: build/write/deploy/test → distinct; joiners: " and ", ", and "
        assert!(a.should_delegate(5), "score {} signals {:?}", a.score, a.signals);
    }

    #[test]
    fn an_enumerated_acceptance_list_scores_high() {
        let body = "Acceptance:\n- research the options\n- write a design doc\n- \
                    implement the chosen one\n- add tests";
        let a = assess("Improve caching", body);
        assert!(a.should_delegate(5), "score {} signals {:?}", a.score, a.signals);
        assert!(a.signals.iter().any(|s| s.contains("enumerated")));
    }

    #[test]
    fn explain_lists_the_signals() {
        let a = assess("Build X and test Y", "");
        assert!(a.explain().starts_with("complexity "));
    }

    #[test]
    fn threshold_is_respected() {
        let a = ComplexityAssessment { score: 4, signals: vec![] };
        assert!(!a.should_delegate(5));
        assert!(a.should_delegate(4));
        assert!(a.should_delegate(3));
    }
}

//! `MissionPlan` — the mission **DAG** the lead decomposes a goal into
//! (J.4.1).
//!
//! A mission is a set of [`Step`]s with explicit dependencies. Each step is
//! either a [`StepKind::Delegate`] (run a specialist on a prompt) or a
//! [`StepKind::Gate`] (a reviewer quality-checks the upstream outputs before
//! its dependents may proceed). The plan is a **DAG from day one** — cycle
//! detection + a [`ready`](MissionPlan::ready)-set drive the concurrent
//! [`TeamRuntime`](crate::runtime::TeamRuntime) (J.4.2): every step whose
//! deps are all complete is runnable *now*, and independent branches run
//! together. Early missions may be linear, but widening to parallel is a
//! flip, not a redesign.
//!
//! This module is pure + synchronous — building and checking the plan never
//! runs an agent.

use std::collections::{HashMap, HashSet};

use crate::config::TeamError;

/// What a [`Step`] does when the runtime reaches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepKind {
    /// Run the named specialist on `prompt` (the doc's Execute/Delegate).
    Delegate { specialist: String, prompt: String },
    /// A quality gate: `reviewer` checks the upstream step outputs against
    /// `criteria`; a failing verdict aborts the mission so the gate's
    /// dependents never run (the doc's Reflect/Gate).
    Gate { reviewer: String, criteria: String },
}

impl StepKind {
    /// The team member this step runs (specialist or reviewer).
    pub fn member(&self) -> &str {
        match self {
            StepKind::Delegate { specialist, .. } => specialist,
            StepKind::Gate { reviewer, .. } => reviewer,
        }
    }
}

/// One node in the mission DAG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Unique step id within the plan (`a-z A-Z 0-9 _ -`).
    pub id: String,
    pub kind: StepKind,
    /// Ids of steps that must complete before this one is ready.
    pub deps: Vec<String>,
}

impl Step {
    /// A `Delegate` step with no dependencies.
    pub fn delegate(id: impl Into<String>, specialist: impl Into<String>, prompt: impl Into<String>) -> Self {
        Step {
            id: id.into(),
            kind: StepKind::Delegate {
                specialist: specialist.into(),
                prompt: prompt.into(),
            },
            deps: Vec::new(),
        }
    }

    /// A `Gate` step with no dependencies.
    pub fn gate(id: impl Into<String>, reviewer: impl Into<String>, criteria: impl Into<String>) -> Self {
        Step {
            id: id.into(),
            kind: StepKind::Gate {
                reviewer: reviewer.into(),
                criteria: criteria.into(),
            },
            deps: Vec::new(),
        }
    }

    /// Builder: set this step's dependencies.
    pub fn after(mut self, deps: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.deps = deps.into_iter().map(Into::into).collect();
        self
    }
}

/// A validated mission: a goal plus a DAG of steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissionPlan {
    pub goal: String,
    pub steps: Vec<Step>,
}

impl MissionPlan {
    pub fn new(goal: impl Into<String>, steps: Vec<Step>) -> Self {
        MissionPlan {
            goal: goal.into(),
            steps,
        }
    }

    /// Look up a step by id.
    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.id == id)
    }

    /// Validate the plan: a non-empty goal, ≥1 step, unique non-empty step
    /// ids, every dep + member name present and well-formed, and — the
    /// load-bearing check — **no cycle** (it must be a DAG).
    pub fn validate(&self) -> Result<(), TeamError> {
        if self.goal.trim().is_empty() {
            return Err(TeamError::Config("mission goal must not be empty".into()));
        }
        if self.steps.is_empty() {
            return Err(TeamError::Config("mission has no steps".into()));
        }

        let mut ids = HashSet::new();
        for s in &self.steps {
            validate_step_id(&s.id)?;
            if !ids.insert(s.id.as_str()) {
                return Err(TeamError::Config(format!("duplicate step id {:?}", s.id)));
            }
            if s.kind.member().trim().is_empty() {
                return Err(TeamError::Config(format!("step {:?} names no member", s.id)));
            }
        }

        // Every dependency must reference a real step (and not itself).
        for s in &self.steps {
            for dep in &s.deps {
                if dep == &s.id {
                    return Err(TeamError::Config(format!("step {:?} depends on itself", s.id)));
                }
                if !ids.contains(dep.as_str()) {
                    return Err(TeamError::Config(format!(
                        "step {:?} depends on unknown step {dep:?}",
                        s.id
                    )));
                }
            }
        }

        self.ensure_acyclic()
    }

    /// Kahn's algorithm: repeatedly remove steps whose deps are all already
    /// removed. If any remain, they form a cycle.
    fn ensure_acyclic(&self) -> Result<(), TeamError> {
        let mut remaining: HashMap<&str, usize> =
            self.steps.iter().map(|s| (s.id.as_str(), s.deps.len())).collect();
        // dependents[x] = steps that depend on x (so removing x frees them).
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
        for s in &self.steps {
            for dep in &s.deps {
                dependents.entry(dep.as_str()).or_default().push(s.id.as_str());
            }
        }

        let mut ready: Vec<&str> = remaining
            .iter()
            .filter(|&(_, &indeg)| indeg == 0)
            .map(|(&id, _)| id)
            .collect();
        let mut removed = 0usize;
        while let Some(id) = ready.pop() {
            remaining.remove(id);
            removed += 1;
            for &dependent in dependents.get(id).into_iter().flatten() {
                if let Some(indeg) = remaining.get_mut(dependent) {
                    *indeg -= 1;
                    if *indeg == 0 {
                        ready.push(dependent);
                    }
                }
            }
        }

        if removed != self.steps.len() {
            let mut cycle: Vec<&str> = remaining.keys().copied().collect();
            cycle.sort_unstable();
            return Err(TeamError::Config(format!(
                "mission plan has a dependency cycle among steps {cycle:?}"
            )));
        }
        Ok(())
    }

    /// The steps that are runnable **now**: not yet completed, with every
    /// dependency already in `completed`. Independent branches surface
    /// together, so the runtime can run them concurrently (J.4.2).
    pub fn ready<'a>(&'a self, completed: &HashSet<String>) -> Vec<&'a Step> {
        self.steps
            .iter()
            .filter(|s| !completed.contains(&s.id))
            .filter(|s| s.deps.iter().all(|d| completed.contains(d)))
            .collect()
    }
}

/// Step ids key the mission report + appear in logs — keep them safe.
fn validate_step_id(id: &str) -> Result<(), TeamError> {
    if id.is_empty() || id.len() > 128 {
        return Err(TeamError::Config(format!(
            "step id must be 1-128 characters, got {}",
            id.len()
        )));
    }
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(TeamError::Config(format!(
            "step id {id:?} has invalid characters (a-z A-Z 0-9 _ - only)"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }
    fn ready_ids(plan: &MissionPlan, completed: &[&str]) -> Vec<String> {
        let mut ids: Vec<String> = plan.ready(&done(completed)).iter().map(|s| s.id.clone()).collect();
        ids.sort();
        ids
    }

    #[test]
    fn delegate_and_gate_expose_their_member() {
        assert_eq!(Step::delegate("a", "coder", "build").kind.member(), "coder");
        assert_eq!(Step::gate("g", "reviewer", "ok?").kind.member(), "reviewer");
    }

    #[test]
    fn a_valid_linear_plan_passes() {
        let plan = MissionPlan::new(
            "ship it",
            vec![
                Step::delegate("research", "researcher", "gather"),
                Step::delegate("write", "writer", "draft").after(["research"]),
            ],
        );
        plan.validate().expect("a linear plan is a DAG");
    }

    #[test]
    fn rejects_empty_goal_and_no_steps() {
        let no_goal = MissionPlan::new("  ", vec![Step::delegate("a", "x", "p")]);
        assert!(matches!(no_goal.validate(), Err(TeamError::Config(m)) if m.contains("goal")));

        let no_steps = MissionPlan::new("g", vec![]);
        assert!(matches!(no_steps.validate(), Err(TeamError::Config(m)) if m.contains("no steps")));
    }

    #[test]
    fn rejects_duplicate_step_ids() {
        let plan = MissionPlan::new(
            "g",
            vec![Step::delegate("dup", "x", "p"), Step::delegate("dup", "y", "q")],
        );
        assert!(matches!(plan.validate(), Err(TeamError::Config(m)) if m.contains("duplicate step id")));
    }

    #[test]
    fn rejects_a_dependency_on_an_unknown_step() {
        let plan = MissionPlan::new("g", vec![Step::delegate("a", "x", "p").after(["ghost"])]);
        assert!(matches!(plan.validate(), Err(TeamError::Config(m)) if m.contains("unknown step")));
    }

    #[test]
    fn rejects_a_self_dependency() {
        let plan = MissionPlan::new("g", vec![Step::delegate("a", "x", "p").after(["a"])]);
        assert!(matches!(plan.validate(), Err(TeamError::Config(m)) if m.contains("depends on itself")));
    }

    #[test]
    fn rejects_a_step_with_no_member() {
        let plan = MissionPlan::new("g", vec![Step::delegate("a", "  ", "p")]);
        assert!(matches!(plan.validate(), Err(TeamError::Config(m)) if m.contains("names no member")));
    }

    #[test]
    fn rejects_an_unsafe_step_id() {
        let plan = MissionPlan::new("g", vec![Step::delegate("bad id!", "x", "p")]);
        assert!(matches!(plan.validate(), Err(TeamError::Config(m)) if m.contains("invalid characters")));
    }

    #[test]
    fn detects_a_two_node_cycle() {
        // a -> b -> a is not a DAG.
        let plan = MissionPlan::new(
            "g",
            vec![
                Step::delegate("a", "x", "p").after(["b"]),
                Step::delegate("b", "y", "q").after(["a"]),
            ],
        );
        let err = plan.validate().unwrap_err();
        assert!(matches!(&err, TeamError::Config(m) if m.contains("cycle")), "got {err:?}");
        // Both cyclic nodes are named.
        let TeamError::Config(m) = err else { unreachable!() };
        assert!(m.contains("\"a\"") && m.contains("\"b\""));
    }

    #[test]
    fn detects_a_longer_cycle_but_passes_a_diamond() {
        // Diamond a -> {b, c} -> d: a DAG, must pass.
        let diamond = MissionPlan::new(
            "g",
            vec![
                Step::delegate("a", "x", "p"),
                Step::delegate("b", "x", "p").after(["a"]),
                Step::delegate("c", "x", "p").after(["a"]),
                Step::gate("d", "reviewer", "ok?").after(["b", "c"]),
            ],
        );
        diamond.validate().expect("a diamond is a DAG");

        // a -> b -> c -> a is a cycle even with an extra acyclic tail.
        let cyclic = MissionPlan::new(
            "g",
            vec![
                Step::delegate("a", "x", "p").after(["c"]),
                Step::delegate("b", "x", "p").after(["a"]),
                Step::delegate("c", "x", "p").after(["b"]),
            ],
        );
        assert!(matches!(cyclic.validate(), Err(TeamError::Config(m)) if m.contains("cycle")));
    }

    #[test]
    fn ready_set_advances_as_steps_complete() {
        let plan = MissionPlan::new(
            "g",
            vec![
                Step::delegate("a", "x", "p"),
                Step::delegate("b", "x", "p").after(["a"]),
                Step::delegate("c", "x", "p").after(["a"]),
                Step::gate("d", "r", "ok?").after(["b", "c"]),
            ],
        );
        // Nothing done → only the root is ready.
        assert_eq!(ready_ids(&plan, &[]), vec!["a"]);
        // a done → b and c are independent, both ready (concurrency surface).
        assert_eq!(ready_ids(&plan, &["a"]), vec!["b", "c"]);
        // Only b done → d still blocked on c; nothing new ready.
        assert_eq!(ready_ids(&plan, &["a", "b"]), vec!["c"]);
        // b and c done → the gate is ready.
        assert_eq!(ready_ids(&plan, &["a", "b", "c"]), vec!["d"]);
        // All done → nothing ready.
        assert!(ready_ids(&plan, &["a", "b", "c", "d"]).is_empty());
    }

    #[test]
    fn two_independent_roots_are_both_ready_at_once() {
        let plan = MissionPlan::new(
            "g",
            vec![Step::delegate("a", "x", "p"), Step::delegate("b", "y", "q")],
        );
        plan.validate().unwrap();
        assert_eq!(ready_ids(&plan, &[]), vec!["a", "b"], "disjoint branches run together");
    }

    #[test]
    fn step_lookup_by_id() {
        let plan = MissionPlan::new("g", vec![Step::delegate("a", "x", "p")]);
        assert!(plan.step("a").is_some());
        assert!(plan.step("missing").is_none());
    }
}

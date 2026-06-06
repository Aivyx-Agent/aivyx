//! Phase 184 — pure builders for conversational skill editing.
//!
//! A skill is a [`crate::persona::LearnedSkill`] JSON list-entry
//! under `PersonaDeltaCategory::LearnedSkill`. Teaching, updating,
//! and forgetting one all reduce to a `PersonaDeltaOp` on that
//! category:
//!
//! - **teach** → `AppendList { LearnedSkill-json }`
//! - **forget** → `RemoveList { existing-LearnedSkill-json }`
//! - **update** → `RemoveList { old }` + `AppendList { new }`
//!
//! Keeping these pure (no chain, no I/O) makes the edit logic
//! unit-testable; the `skills.{teach,update,forget}` tools wrap
//! them with the persona-chain append + the effective-persona
//! recompute.

use crate::persona::{LearnedSkill, PersonaDeltaOp};

/// Max characters for a skill name — long enough for a
/// dot-namespaced kebab slug, short enough to stay an identifier.
pub const SKILL_NAME_MAX: usize = 64;

/// Validate a skill name: non-empty, within length, and limited
/// to `[a-z0-9._-]` (kebab-case, optionally dot-namespaced — the
/// convention `skills.invoke` lookups rely on). Returns a
/// human-readable reason on rejection.
pub fn validate_skill_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("skill name must not be empty".into());
    }
    if n.chars().count() > SKILL_NAME_MAX {
        return Err(format!(
            "skill name must be ≤ {SKILL_NAME_MAX} characters"
        ));
    }
    if !n
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_'))
    {
        return Err(
            "skill name must be kebab-case: lowercase letters, \
             digits, '.', '-', '_' only (e.g. `code.review-checklist`)"
                .into(),
        );
    }
    Ok(())
}

/// Find a skill by exact name in the current set.
pub fn find_skill_by_name<'a>(
    skills: &'a [LearnedSkill],
    name: &str,
) -> Option<&'a LearnedSkill> {
    let target = name.trim();
    skills.iter().find(|s| s.name == target)
}

/// The op that teaches (adds) a new skill.
pub fn teach_op(skill: &LearnedSkill) -> PersonaDeltaOp {
    PersonaDeltaOp::AppendList {
        value: skill.to_json_value(),
    }
}

/// The op that forgets (removes) an existing skill. The value
/// must byte-match the stored entry, so it is built from the
/// existing [`LearnedSkill`], never from a reconstruction.
pub fn forget_op(existing: &LearnedSkill) -> PersonaDeltaOp {
    PersonaDeltaOp::RemoveList {
        value: existing.to_json_value(),
    }
}

/// The two ops that update a skill: remove the old entry, append
/// the new one. Order matters at apply time only in that both
/// land; the chain keeps the old entry recoverable.
pub fn update_ops(
    old: &LearnedSkill,
    new: &LearnedSkill,
) -> [PersonaDeltaOp; 2] {
    [forget_op(old), teach_op(new)]
}

/// Merge an update onto an existing skill: any of trigger /
/// procedure that is `Some` replaces the existing field; `None`
/// keeps it. The name is immutable for an update (use forget +
/// teach to rename).
pub fn merged_skill(
    existing: &LearnedSkill,
    new_trigger: Option<&str>,
    new_procedure: Option<&str>,
) -> LearnedSkill {
    LearnedSkill {
        name: existing.name.clone(),
        trigger: new_trigger
            .map(|t| t.trim().to_string())
            .unwrap_or_else(|| existing.trigger.clone()),
        procedure: new_procedure
            .map(|p| p.trim().to_string())
            .unwrap_or_else(|| existing.procedure.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str, trig: &str, proc: &str) -> LearnedSkill {
        LearnedSkill {
            name: name.into(),
            trigger: trig.into(),
            procedure: proc.into(),
        }
    }

    #[test]
    fn validate_name_accepts_kebab_rejects_others() {
        assert!(validate_skill_name("code.review-checklist").is_ok());
        assert!(validate_skill_name("deploy_v2").is_ok());
        assert!(validate_skill_name("").is_err());
        assert!(validate_skill_name("Has Caps").is_err());
        assert!(validate_skill_name("spaces here").is_err());
        assert!(validate_skill_name(&"x".repeat(65)).is_err());
    }

    #[test]
    fn find_by_name_exact() {
        let set = vec![skill("a", "", ""), skill("b.c", "", "")];
        assert_eq!(find_skill_by_name(&set, "b.c").unwrap().name, "b.c");
        assert!(find_skill_by_name(&set, "b").is_none()); // exact only
        assert!(find_skill_by_name(&set, "nope").is_none());
    }

    #[test]
    fn teach_op_appends_skill_json() {
        let s = skill("greet", "when greeting", "say hi warmly");
        match teach_op(&s) {
            PersonaDeltaOp::AppendList { value } => {
                let back = LearnedSkill::from_json_value(&value).unwrap();
                assert_eq!(back, s);
            }
            other => panic!("expected AppendList, got {other:?}"),
        }
    }

    #[test]
    fn forget_op_removes_exact_stored_json() {
        let s = skill("greet", "w", "p");
        let teach = teach_op(&s);
        let forget = forget_op(&s);
        // The forget value byte-matches the teach value — so the
        // RemoveList will find the AppendList'd entry.
        match (teach, forget) {
            (
                PersonaDeltaOp::AppendList { value: a },
                PersonaDeltaOp::RemoveList { value: r },
            ) => assert_eq!(a, r),
            _ => panic!("op shapes"),
        }
    }

    #[test]
    fn update_ops_remove_old_add_new() {
        let old = skill("greet", "old when", "old steps");
        let new = skill("greet", "new when", "new steps");
        let [forget, teach] = update_ops(&old, &new);
        match forget {
            PersonaDeltaOp::RemoveList { value } => {
                assert_eq!(
                    LearnedSkill::from_json_value(&value).unwrap(),
                    old
                );
            }
            _ => panic!("first op is remove"),
        }
        match teach {
            PersonaDeltaOp::AppendList { value } => {
                assert_eq!(
                    LearnedSkill::from_json_value(&value).unwrap(),
                    new
                );
            }
            _ => panic!("second op is append"),
        }
    }

    #[test]
    fn merged_keeps_name_replaces_provided_fields() {
        let existing = skill("greet", "old when", "old steps");
        // Replace only the procedure.
        let m = merged_skill(&existing, None, Some("new steps"));
        assert_eq!(m.name, "greet");
        assert_eq!(m.trigger, "old when"); // kept
        assert_eq!(m.procedure, "new steps"); // replaced
        // Replace only the trigger.
        let m2 = merged_skill(&existing, Some("new when"), None);
        assert_eq!(m2.trigger, "new when");
        assert_eq!(m2.procedure, "old steps");
    }
}

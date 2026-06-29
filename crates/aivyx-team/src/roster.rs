//! The default general-purpose Nonagon — the 9 roles that ship in the free
//! core, used when no vertical supplies its own [`TeamConfig`]. A vertical
//! pack overrides this with a domain-shaped roster (e.g. the kitchen BOH
//! team). Ported from the archive's `NONAGON_ROLES`, re-grounded on
//! new-core scopes.

use aivyx_capability::TrustTier;

use crate::config::{DialogueConfig, TeamConfig, TeamMember};

fn member(
    name: &str,
    role: &str,
    soul: &str,
    tools: &[&str],
    scopes: &[&str],
) -> TeamMember {
    // Every member can talk on the team bus, so `team.message` is granted to
    // all (the lead holds it too, so attenuation keeps it for specialists).
    // The send/read_message tools are injected per member at assembly (J.5).
    let mut capability_scopes: Vec<String> = scopes.iter().map(|s| s.to_string()).collect();
    capability_scopes.push("team.message".to_string());
    TeamMember {
        name: name.to_string(),
        role: role.to_string(),
        soul: soul.to_string(),
        tool_allowlist: tools.iter().map(|s| s.to_string()).collect(),
        capability_scopes,
        trust_ceiling: TrustTier::Trusted,
        model: None,
        base_url: None,
    }
}

/// The default 9-role Nonagon: a coordinator lead + 8 specialists.
pub fn default_nonagon() -> TeamConfig {
    let members = vec![
        member(
            "coordinator",
            "Lead",
            "You are the team coordinator — the orchestrator of a multi-agent team. \
             You decompose goals into targeted subtasks, delegate each to the best-fit \
             specialist, verify their output against the goal, and synthesize the results \
             into one coherent deliverable. You never execute domain work directly: you \
             plan, delegate, verify, and synthesize.",
            &["delegate_task", "query_agent"], // + decompose/verify injected in J.4+
            &["memory.read", "memory.write", "team.delegate"],
        ),
        member(
            "researcher",
            "Researcher",
            "You gather information from the web and project files and distill it into \
             structured, cited findings. You flag contradictions and note confidence per \
             claim rather than picking one interpretation prematurely.",
            // web.fetch ships in the daemon; web.search arrives with the toolkit pack.
            &["web.fetch", "web.search", "fs.read", "memory.write"],
            &["web.search", "net.fetch", "fs.read", "memory.write"],
        ),
        member(
            "analyst",
            "Analyst",
            "You ingest data and code, surface patterns and anomalies, and produce clear \
             quantified findings. You separate what the data shows from what you infer.",
            &["fs.read", "web.search"],
            &["fs.read", "web.search"],
        ),
        member(
            "coder",
            "Coder",
            "You implement focused changes, reach for tests first, and keep diffs minimal. \
             You explain WHY before WHAT and never leave the tree in a broken state.",
            &["fs.read", "fs.write", "shell.exec"],
            &["fs.read", "fs.write", "shell.exec"],
        ),
        member(
            "writer",
            "Writer",
            "You turn raw material into clear prose — docs, summaries, release notes — \
             matching the requested audience and voice without inventing facts.",
            &["fs.read", "fs.write"],
            &["fs.read", "fs.write"],
        ),
        member(
            "reviewer",
            "Reviewer",
            "You critically review another specialist's output against the goal and quality \
             criteria, returning concrete, actionable findings — not a rubber stamp.",
            &["fs.read"],
            &["fs.read"],
        ),
        member(
            "planner",
            "Planner",
            "You break ambiguous goals into ordered, dependency-aware steps and keep the \
             team's plan current as work completes or reveals new needs.",
            &["memory.read", "memory.write"],
            &["memory.read", "memory.write"],
        ),
        member(
            "ops",
            "Operations",
            "You handle execution-environment work — running commands, inspecting state — \
             within a tightly bounded capability scope, reporting results precisely.",
            &["shell.exec", "fs.read"],
            &["shell.exec", "fs.read"],
        ),
        member(
            "archivist",
            "Archivist",
            "You persist significant findings to memory so they outlive the session, and \
             retrieve prior context the team needs. You keep the record clean and findable.",
            &["memory.read", "memory.write", "fs.read"],
            &["memory.read", "memory.write", "fs.read"],
        ),
    ];

    TeamConfig {
        name: "default-nonagon".to_string(),
        description: "The default general-purpose 9-role Nonagon.".to_string(),
        lead: "coordinator".to_string(),
        members,
        dialogue: DialogueConfig::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_nonagon_is_valid() {
        let team = default_nonagon();
        team.validate().expect("the shipped default team must be valid");
    }

    #[test]
    fn has_a_coordinator_lead_and_eight_specialists() {
        let team = default_nonagon();
        assert_eq!(team.members.len(), 9);
        assert_eq!(team.lead, "coordinator");
        assert_eq!(team.specialists().count(), 8);
        assert!(team.lead_member().is_some());
    }

    #[test]
    fn every_member_has_a_soul_and_parseable_scopes() {
        for m in &default_nonagon().members {
            assert!(!m.soul.trim().is_empty(), "{} has no soul", m.name);
            // Unknown scope bases would error here.
            m.parsed_scopes().unwrap_or_else(|e| panic!("{}: {e}", m.name));
        }
    }

    #[test]
    fn ships_the_nine_named_roles() {
        let team = default_nonagon();
        let names: Vec<&str> = team.members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "coordinator", "researcher", "analyst", "coder", "writer", "reviewer",
                "planner", "ops", "archivist"
            ]
        );
    }

    #[test]
    fn every_member_can_talk_on_the_bus() {
        use aivyx_capability::Scope;
        let msg = Scope::parse("team.message").unwrap();
        for m in &default_nonagon().members {
            assert!(
                m.declared_capabilities().unwrap().grants(&msg),
                "{} should hold team.message for peer dialogue",
                m.name
            );
        }
    }

    #[test]
    fn roundtrips_through_toml() {
        let team = default_nonagon();
        let back = TeamConfig::from_toml(&team.to_toml().unwrap()).unwrap();
        assert_eq!(back, team);
    }
}

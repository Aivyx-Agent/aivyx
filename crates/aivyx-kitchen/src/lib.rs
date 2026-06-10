//! `aivyx-kitchen` — the **Kitchen / Back-of-House vertical pack** (Chapter
//! J.6).
//!
//! A vertical pack is *config over the free engine*, never a fork. This crate
//! ships the pack's **Nonagon team**: a customised [`TeamConfig`] (Aria the
//! BOH manager leading four least-privileged specialists over the `kitchen.*`
//! scopes) plus the canonical **overnight-close** [`MissionPlan`]. The engine
//! ([`aivyx_team`]) is free; this domain-expert team is the product. See
//! `docs/VERTICAL_PACKS.md` + `docs/NONAGON.md` §9.
//!
//! The same roster ships as a TOML asset ([`KITCHEN_BOH_TOML`]) so the pack is
//! literally config — `aivyx team run --config <path>` loads it, and the
//! [`kitchen_boh_team`] constructor exists for tests + programmatic use.
//!
//! Safety (NT-02): every specialist is attenuated to a subset of Aria's
//! authority. HACCP holds **only** `kitchen.haccp.log` — it physically cannot
//! send a PO or read inventory. Purchasing holds `kitchen.order.send`, so the
//! `po.send` confirm-first gate fires at the holder.

use aivyx_capability::TrustTier;
use aivyx_team::config::{DialogueConfig, TeamConfig, TeamMember};
use aivyx_team::mission::{MissionPlan, Step};

/// The kitchen BOH team as committed TOML — the literal "pack supplies a
/// `TeamConfig`" artifact. Round-trips with [`kitchen_boh_team`].
pub const KITCHEN_BOH_TOML: &str = include_str!("../assets/kitchen-boh.toml");

fn member(name: &str, role: &str, soul: &str, tools: &[&str], scopes: &[&str]) -> TeamMember {
    TeamMember {
        name: name.to_string(),
        role: role.to_string(),
        soul: soul.to_string(),
        tool_allowlist: tools.iter().map(|s| s.to_string()).collect(),
        capability_scopes: scopes.iter().map(|s| s.to_string()).collect(),
        trust_ceiling: TrustTier::Trusted,
    }
}

/// The Kitchen / BOH Nonagon: **Aria** (lead) + stocktake / inventory /
/// purchasing / HACCP — each least-privileged over the `kitchen.*` scopes.
pub fn kitchen_boh_team() -> TeamConfig {
    let members = vec![
        member(
            "aria",
            "BOH Manager",
            "You coordinate back-of-house. You never touch stock directly — you decompose the \
             shift's goals into targeted subtasks, delegate each to the best-fit specialist, \
             verify their output, and synthesize the close-down report. You hold the authority \
             to send purchase orders and log food-safety records, but you delegate the work.",
            &["decompose_task", "delegate_task", "query_agent", "verify_output", "synthesize_results"],
            &[
                "kitchen.read",
                "kitchen.write",
                "kitchen.order.send",
                "kitchen.haccp.log",
                "team.delegate",
                "team.message",
            ],
        ),
        member(
            "stocktake",
            "Stocktake",
            "You count closing stock accurately and record the counts. You report exactly what \
             is on the shelf — never an estimate dressed up as a count.",
            &["inventory.count"],
            &["kitchen.read", "kitchen.write", "team.message"],
        ),
        member(
            "inventory",
            "Inventory Analyst",
            "You read the recorded counts and compute what is below par, with the shortfall per \
             item. You separate what the counts show from what you infer.",
            &["inventory.low_stock"],
            &["kitchen.read", "team.message"],
        ),
        member(
            "purchasing",
            "Purchasing",
            "You turn the low-stock list into per-supplier purchase orders, grouped by supplier \
             and respecting pack sizes and minimum orders. You draft; sending a PO spends money, \
             so it stays confirm-first.",
            &["po.draft", "po.send"],
            &["kitchen.read", "kitchen.order.send", "team.message"],
        ),
        member(
            "haccp",
            "Food-Safety / Compliance",
            "You log the closing temperature round (fridges, freezers, hot-hold) and flag any \
             out-of-limit reading with its corrective action. You can ONLY log food-safety \
             records — nothing else.",
            &["haccp.log"],
            &["kitchen.haccp.log", "team.message"],
        ),
    ];

    TeamConfig {
        name: "kitchen-boh".to_string(),
        description: "Back-of-House Nonagon: Aria leads stocktake, inventory, purchasing, and HACCP."
            .to_string(),
        lead: "aria".to_string(),
        members,
        dialogue: DialogueConfig::default(),
    }
}

/// The canonical **overnight-close** mission as a DAG:
///
/// ```text
///   count ─▶ lowstock ─▶ draft_po
///   fridge_log                       (independent — runs alongside the chain)
/// ```
///
/// Stocktake counts → Inventory computes low-stock → Purchasing drafts the
/// POs; HACCP's closing fridge round is an independent branch the runtime
/// runs concurrently. Aria then verifies + synthesizes the close-down report
/// from the collected outputs (her orchestration tools, post-mission).
pub fn overnight_close_mission() -> MissionPlan {
    MissionPlan::new(
        "Run end-of-day BOH close: count stock, reorder what's low, and log the closing fridge round.",
        vec![
            Step::delegate("count", "stocktake", "Count tonight's closing stock and record the counts."),
            Step::delegate(
                "lowstock",
                "inventory",
                "From the recorded counts, compute every item below par and its shortfall.",
            )
            .after(["count"]),
            Step::delegate(
                "draft_po",
                "purchasing",
                "Draft per-supplier purchase orders for the low-stock items. Do not send them.",
            )
            .after(["lowstock"]),
            Step::delegate(
                "fridge_log",
                "haccp",
                "Log the closing temperature round for all fridges, freezers, and hot-hold; flag \
                 any out-of-limit reading with a corrective action.",
            ),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_capability::Scope;
    use aivyx_team::attenuate_for_member;

    fn scope(s: &str) -> Scope {
        Scope::parse(s).expect("known base")
    }

    #[test]
    fn team_is_valid_with_aria_leading_four_specialists() {
        let team = kitchen_boh_team();
        team.validate().expect("kitchen team must be valid");
        assert_eq!(team.lead, "aria");
        assert_eq!(team.specialists().count(), 4);
        let names: Vec<&str> = team.members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["aria", "stocktake", "inventory", "purchasing", "haccp"]);
    }

    #[test]
    fn haccp_is_least_privileged() {
        // HACCP declares only kitchen.haccp.log (+ team.message): it can log
        // food-safety records and talk, nothing else.
        let team = kitchen_boh_team();
        let haccp = team.members.iter().find(|m| m.name == "haccp").unwrap();
        assert_eq!(haccp.capability_scopes, ["kitchen.haccp.log", "team.message"]);
    }

    #[test]
    fn nt02_haccp_cannot_exceed_aria_and_cannot_order() {
        // Attenuate HACCP's declared scopes against Aria's authority: the
        // result is exactly {haccp.log, team.message} — and crucially does NOT
        // grant kitchen.order.send (HACCP physically cannot send a PO).
        let team = kitchen_boh_team();
        let aria = team.lead_member().unwrap();
        let aria_caps = aria.declared_capabilities().unwrap();
        let haccp = team.members.iter().find(|m| m.name == "haccp").unwrap();

        let attenuated = attenuate_for_member(&aria_caps, &haccp.parsed_scopes().unwrap());
        assert!(attenuated.grants(&scope("kitchen.haccp.log")));
        assert!(!attenuated.grants(&scope("kitchen.order.send")), "HACCP cannot order");
        assert!(!attenuated.grants(&scope("kitchen.read")), "HACCP cannot read inventory");
    }

    #[test]
    fn only_purchasing_can_send_a_po_and_only_aria_leads() {
        let team = kitchen_boh_team();
        let aria = team.lead_member().unwrap().declared_capabilities().unwrap();
        let order = scope("kitchen.order.send");
        for m in team.specialists() {
            let caps =
                attenuate_for_member(&aria, &m.parsed_scopes().unwrap());
            let can_order = caps.grants(&order);
            assert_eq!(can_order, m.name == "purchasing", "{} order authority", m.name);
            // No specialist inherits the lead's orchestration authority.
            assert!(!caps.grants(&scope("team.delegate")), "{} cannot convene a team", m.name);
        }
    }

    #[test]
    fn overnight_close_is_a_valid_dag_with_one_independent_branch() {
        let mission = overnight_close_mission();
        mission.validate().expect("overnight close must be a DAG");
        // Only the two roots (count + fridge_log) are ready up front; the
        // fridge round runs independently of the count→reorder chain.
        let ready: Vec<&str> = mission
            .ready(&std::collections::HashSet::new())
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert!(ready.contains(&"count"));
        assert!(ready.contains(&"fridge_log"));
        assert!(!ready.contains(&"draft_po"), "PO draft waits on the chain");
    }

    #[test]
    fn every_mission_step_targets_a_real_team_member() {
        let team = kitchen_boh_team();
        let names: std::collections::HashSet<&str> =
            team.members.iter().map(|m| m.name.as_str()).collect();
        for step in &overnight_close_mission().steps {
            let target = step.kind.member();
            assert!(names.contains(target), "mission targets unknown member {target:?}");
            assert_ne!(target, team.lead, "a mission step never delegates to the lead");
        }
    }

    #[test]
    fn toml_asset_round_trips_with_the_constructor() {
        let from_asset = TeamConfig::from_toml(KITCHEN_BOH_TOML)
            .expect("the shipped kitchen-boh.toml must parse + validate");
        assert_eq!(from_asset, kitchen_boh_team(), "asset drifted from the constructor");
    }
}

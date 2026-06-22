//! Chapter Brigade (BG.4) — drift guard between the BOH vertical pack and the
//! kitchen toolkit. Every `kitchen.*` tool a `kitchen-boh.toml` specialist names
//! in its `tool_allowlist` must actually be provided by this toolkit; otherwise
//! the daemon would attenuate the specialist down to nothing and the brigade
//! would stall (the exact gap Chapter Brigade exists to close). This test fails
//! loudly if the pack and the toolkit ever drift apart again.

use std::collections::HashSet;
use std::sync::Arc;

use aivyx_kitchen::{kitchen_boh_team, overnight_close_mission};
use aivyx_kitchen_toolkit::{all_tools, KitchenClient};

/// The set of kitchen.* tool names this toolkit provides.
fn provided_kitchen_tools() -> HashSet<String> {
    let client = Arc::new(KitchenClient::new(reqwest::Client::new(), "http://x", "k", "o"));
    all_tools(client).iter().map(|t| t.name().to_string()).collect()
}

#[test]
fn boh_pack_kitchen_tools_are_all_provided_by_the_toolkit() {
    let provided = provided_kitchen_tools();

    let team = kitchen_boh_team();
    let mut referenced = 0usize;
    for m in &team.members {
        for tool in &m.tool_allowlist {
            // Only the kitchen.* tools come from this toolkit; orchestration /
            // dialogue tools (decompose_task, delegate_task, …) come from the
            // team engine and are out of scope here.
            if tool.starts_with("kitchen.") {
                referenced += 1;
                assert!(
                    provided.contains(tool),
                    "BOH member {:?} references `{tool}`, which the kitchen toolkit does not provide. \
                     Provided: {provided:?}",
                    m.name,
                );
            }
        }
    }
    // Sanity: the pack actually exercises the toolkit (guards against a future
    // edit silently dropping every kitchen.* tool from the allowlists).
    assert!(referenced >= 5, "expected the BOH pack to reference several kitchen.* tools, got {referenced}");
}

/// Chapter Lockup (LK.2) — no dead step. Every delegate step of the flagship
/// `overnight_close_mission()` must target a specialist who actually holds a
/// `kitchen.*` tool the toolkit provides — otherwise the lead would delegate
/// work no specialist can perform (the `draft_po`-with-no-draft-tool gap Lockup
/// closes). Fails loudly if a future edit reintroduces a dead step.
#[test]
fn overnight_close_mission_has_no_dead_step() {
    let provided = provided_kitchen_tools();
    let team = kitchen_boh_team();
    let mission = overnight_close_mission();

    for step in &mission.steps {
        let who = step.kind.member();
        let member = team
            .members
            .iter()
            .find(|m| m.name == who)
            .unwrap_or_else(|| panic!("step {:?} targets unknown member {who}", step.id));
        let has_usable_kitchen_tool = member
            .tool_allowlist
            .iter()
            .any(|t| t.starts_with("kitchen.") && provided.contains(t));
        assert!(
            has_usable_kitchen_tool,
            "dead step {:?}: delegates to {who}, who holds no kitchen.* tool the toolkit provides \
             (allowlist: {:?})",
            step.id, member.tool_allowlist,
        );
    }
}

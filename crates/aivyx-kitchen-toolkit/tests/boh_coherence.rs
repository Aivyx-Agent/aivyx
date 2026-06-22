//! Chapter Brigade (BG.4) — drift guard between the BOH vertical pack and the
//! kitchen toolkit. Every `kitchen.*` tool a `kitchen-boh.toml` specialist names
//! in its `tool_allowlist` must actually be provided by this toolkit; otherwise
//! the daemon would attenuate the specialist down to nothing and the brigade
//! would stall (the exact gap Chapter Brigade exists to close). This test fails
//! loudly if the pack and the toolkit ever drift apart again.

use std::collections::HashSet;
use std::sync::Arc;

use aivyx_kitchen::kitchen_boh_team;
use aivyx_kitchen_toolkit::{all_tools, KitchenClient};

#[test]
fn boh_pack_kitchen_tools_are_all_provided_by_the_toolkit() {
    let client = Arc::new(KitchenClient::new(reqwest::Client::new(), "http://x", "k", "o"));
    let provided: HashSet<String> =
        all_tools(client).iter().map(|t| t.name().to_string()).collect();

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

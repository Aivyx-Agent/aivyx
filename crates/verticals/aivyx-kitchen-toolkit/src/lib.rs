//! `aivyx-kitchen-toolkit` — the kitchen vertical's tool process (Chapter
//! Brigade).
//!
//! A [Chapter F/G](../../../docs/VERTICAL_PACKS.md) substrate-pattern tool
//! process: the daemon spawns it via `[[tool_process]]` in `aivyx-pa.toml`, and it
//! registers the `kitchen.*` tools through the multi-tool harness
//! ([`run_multi_tool_subprocess`]). The tools call the operator's **KitchenDB**
//! (Postgres + PostgREST) — the system of record — via [`KitchenClient`]; the
//! agent never reimplements domain logic.
//!
//! The `kitchen.*` capability bases already live in `aivyx-capability`'s
//! `KNOWN_BASES`, so this crate adds none and is not a P10 amendment — it is
//! the third-party tool-process tier.
//!
//! BG.1 ships the read surface (`kitchen.read`): inventory list / low-stock /
//! value, recipe search, supplier list. Gated writes, `kitchen.order.send`
//! (confirm-first), and `kitchen.haccp.log` land in later phases.

pub mod client;
pub mod config;
pub mod tools;

use std::sync::Arc;

use aivyx_vertical_sdk::tool::Tool;

pub use aivyx_vertical_sdk::tool::run_multi_tool_subprocess;
pub use client::{KitchenClient, KitchenError};
pub use config::{default_config_path, load_config, KitchenConfig, KitchenDbConfig};

/// The full `kitchen.*` tool surface, in one place so the binary and the
/// coherence tests register the identical set. BG.1 read + BG.2 write + BG.3
/// order.send + BG.4 HACCP + LK.1 order.draft — eleven tools across four scope
/// bases (`kitchen.read`, `kitchen.write`, `kitchen.order.send`,
/// `kitchen.haccp.log`).
pub fn all_tools(client: Arc<KitchenClient>) -> Vec<Arc<dyn Tool>> {
    vec![
        // kitchen.read
        Arc::new(tools::InventoryList::new(Arc::clone(&client))),
        Arc::new(tools::InventoryLowStock::new(Arc::clone(&client))),
        Arc::new(tools::InventoryValue::new(Arc::clone(&client))),
        Arc::new(tools::RecipeSearch::new(Arc::clone(&client))),
        Arc::new(tools::SupplierList::new(Arc::clone(&client))),
        // kitchen.write
        Arc::new(tools::InventoryAdjust::new(Arc::clone(&client))),
        Arc::new(tools::BatchStart::new(Arc::clone(&client))),
        Arc::new(tools::BatchComplete::new(Arc::clone(&client))),
        Arc::new(tools::OrderDraft::new(Arc::clone(&client))),
        // kitchen.order.send (confirm-first)
        Arc::new(tools::OrderSend::new(Arc::clone(&client))),
        // kitchen.haccp.log (append-only)
        Arc::new(tools::HaccpLog::new(Arc::clone(&client))),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tools_registers_the_expected_surface() {
        let client = Arc::new(KitchenClient::new(reqwest::Client::new(), "http://x", "k", "o"));
        let tools = all_tools(client);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(
            names,
            vec![
                "kitchen.inventory.list",
                "kitchen.inventory.low_stock",
                "kitchen.inventory.value",
                "kitchen.recipe.search",
                "kitchen.supplier.list",
                "kitchen.inventory.adjust",
                "kitchen.batch.start",
                "kitchen.batch.complete",
                "kitchen.order.draft",
                "kitchen.order.send",
                "kitchen.haccp.log",
            ]
        );
        // Every tool's required scope is a kitchen.* base.
        for t in &tools {
            let scope = t.required_scope(&serde_json::json!({})).to_string();
            assert!(scope.starts_with("kitchen."), "{} -> {scope}", t.name());
        }
    }
}

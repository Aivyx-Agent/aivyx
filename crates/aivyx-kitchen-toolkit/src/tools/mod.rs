//! The `kitchen.*` tool surface.
//!
//! Chapter Brigade. Each tool is a thin wrapper over [`KitchenClient`]: it
//! names a KitchenDB RPC, maps its JSON input to the RPC's params, and shapes
//! the response. The domain logic lives in KitchenDB — the tools never compute
//! inventory math or PO rules themselves.
//!
//! BG.1 ships the read surface (`kitchen.read`). Later phases add the gated
//! write tools, `kitchen.order.send`, and `kitchen.haccp.log`.

use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, ToolContext, ToolId, ToolOutcome, Verification};

use crate::client::KitchenClient;

mod batch;
mod inventory;
mod order;
mod recipe;
mod supplier;

pub use batch::{BatchComplete, BatchStart};
pub use inventory::{InventoryAdjust, InventoryList, InventoryLowStock, InventoryValue};
pub use order::OrderSend;
pub use recipe::RecipeSearch;
pub use supplier::SupplierList;

/// The shared read scope for the BG.1 read tools. `kitchen.read` is already in
/// `aivyx-capability`'s `KNOWN_BASES`, so this always parses.
pub(crate) fn kitchen_read_scope() -> Scope {
    Scope::parse("kitchen.read").expect("kitchen.read is in KNOWN_BASES")
}

/// The shared write scope for the BG.2 mutate tools (stock adjustment, batch
/// lifecycle). `kitchen.write` is already in `KNOWN_BASES`.
pub(crate) fn kitchen_write_scope() -> Scope {
    Scope::parse("kitchen.write").expect("kitchen.write is in KNOWN_BASES")
}

/// The scope for `kitchen.order.send` (BG.3) — the confirm-first PO dispatch.
/// Its own base (separate from `kitchen.write`) so a roster can grant stock
/// edits without granting the power to place orders. Already in `KNOWN_BASES`.
pub(crate) fn kitchen_order_send_scope() -> Scope {
    Scope::parse("kitchen.order.send").expect("kitchen.order.send is in KNOWN_BASES")
}

/// Run a write RPC and build the tool outcome. The KitchenDB response (the
/// affected row / status) is wrapped as `{ <result_key>: value }`. Marked
/// [`Verification::Unverified`] — KitchenDB returned Ok, but the tool does not
/// issue a separate confirming read.
pub(crate) async fn run_write(
    client: &KitchenClient,
    tool_id: ToolId,
    function: &str,
    params: Value,
    result_key: &str,
    _ctx: &ToolContext<'_>,
) -> ToolOutcome {
    match client.call_rpc(function, params).await {
        Ok(value) => ToolOutcome::Completed {
            output: json!({ result_key: value }),
            verified: Verification::Unverified,
        },
        Err(e) => ToolOutcome::Failed(AivyxError::Tool {
            tool: tool_id,
            detail: format!("kitchen: {e}"),
        }),
    }
}

/// Run a read RPC and build the tool outcome. A successful array response is
/// wrapped as `{ <collection_key>: [...], "count": n }`; any other JSON value
/// is wrapped as `{ <collection_key>: value }`. Failures map to a `Failed`
/// outcome carrying the [`KitchenError`](crate::client::KitchenError) message.
pub(crate) async fn run_read(
    client: &KitchenClient,
    tool_id: ToolId,
    function: &str,
    params: Value,
    collection_key: &str,
    _ctx: &ToolContext<'_>,
) -> ToolOutcome {
    match client.call_rpc(function, params).await {
        Ok(value) => ToolOutcome::Completed {
            output: shape_rows(value, collection_key),
            verified: Verification::NotApplicable,
        },
        Err(e) => ToolOutcome::Failed(AivyxError::Tool {
            tool: tool_id,
            detail: format!("kitchen: {e}"),
        }),
    }
}

/// Wrap a KitchenDB RPC result for the LLM. An array → `{ key: rows, count }`;
/// any other value (scalar / object — e.g. an aggregate) → `{ key: value }`.
pub(crate) fn shape_rows(value: Value, key: &str) -> Value {
    match value {
        Value::Array(rows) => {
            let count = rows.len();
            json!({ key: rows, "count": count })
        }
        other => json!({ key: other }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_rows_wraps_an_array_with_count() {
        let shaped = shape_rows(json!([{"sku": "A"}, {"sku": "B"}]), "items");
        assert_eq!(shaped["count"], 2);
        assert_eq!(shaped["items"], json!([{"sku": "A"}, {"sku": "B"}]));
    }

    #[test]
    fn shape_rows_wraps_a_scalar_without_count() {
        let shaped = shape_rows(json!(1234.5), "value");
        assert_eq!(shaped["value"], 1234.5);
        assert!(shaped.get("count").is_none());
    }

    #[test]
    fn kitchen_read_scope_parses() {
        assert_eq!(kitchen_read_scope().to_string(), "kitchen.read");
    }
}

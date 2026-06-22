//! `kitchen.inventory.*` read tools — Chapter Brigade BG.1.
//!
//! Three reads over KitchenDB inventory RPCs: the full stock list (optionally
//! filtered to a location), the low-stock report, and the total inventory
//! value. All `kitchen.read`. RPC function names are the KitchenDB convention
//! (`get_*`, org-scoped); confirmed against the live schema in-phase (OQ-3).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{Tool, ToolContext, ToolId, ToolOutcome};

use super::{kitchen_read_scope, run_read};
use crate::client::KitchenClient;

const INVENTORY_LIST_FN: &str = "get_inventory";
const LOW_STOCK_FN: &str = "get_low_stock_items";
const INVENTORY_VALUE_FN: &str = "get_inventory_value";

/// `kitchen.inventory.list` — the current stock list, optionally filtered to a
/// storage location.
pub struct InventoryList {
    id: ToolId,
    schema: Value,
    client: Arc<KitchenClient>,
}

impl InventoryList {
    pub fn new(client: Arc<KitchenClient>) -> Self {
        Self {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "Optional storage location to filter to (e.g. \"walk-in\", \"dry-store\"). Omit for all locations."
                    }
                },
                "additionalProperties": false
            }),
            client,
        }
    }
}

#[async_trait]
impl Tool for InventoryList {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "kitchen.inventory.list"
    }
    fn description(&self) -> &str {
        "List current kitchen inventory from KitchenDB. Optional `location` \
         (string) filters to one storage area; omit for all. Returns \
         `{ items: [...], count }`. Read-only."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        kitchen_read_scope()
    }
    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        let params = match inventory_list_params(&input) {
            Ok(p) => p,
            Err(detail) => return invalid(self.id, &detail),
        };
        run_read(&self.client, self.id, INVENTORY_LIST_FN, params, "items", ctx).await
    }
}

/// `kitchen.inventory.low_stock` — items at or below their reorder threshold.
pub struct InventoryLowStock {
    id: ToolId,
    schema: Value,
    client: Arc<KitchenClient>,
}

impl InventoryLowStock {
    pub fn new(client: Arc<KitchenClient>) -> Self {
        Self { id: ToolId::new(), schema: empty_object_schema(), client }
    }
}

#[async_trait]
impl Tool for InventoryLowStock {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "kitchen.inventory.low_stock"
    }
    fn description(&self) -> &str {
        "List inventory items at or below their reorder threshold (the reorder \
         candidates). No input. Returns `{ items: [...], count }`. Read-only."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        kitchen_read_scope()
    }
    async fn execute(&self, _input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        run_read(&self.client, self.id, LOW_STOCK_FN, json!({}), "items", ctx).await
    }
}

/// `kitchen.inventory.value` — the total valuation of current stock.
pub struct InventoryValue {
    id: ToolId,
    schema: Value,
    client: Arc<KitchenClient>,
}

impl InventoryValue {
    pub fn new(client: Arc<KitchenClient>) -> Self {
        Self { id: ToolId::new(), schema: empty_object_schema(), client }
    }
}

#[async_trait]
impl Tool for InventoryValue {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "kitchen.inventory.value"
    }
    fn description(&self) -> &str {
        "Get the total valuation of current kitchen inventory. No input. \
         Returns `{ value: <KitchenDB aggregate> }`. Read-only."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        kitchen_read_scope()
    }
    async fn execute(&self, _input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        run_read(&self.client, self.id, INVENTORY_VALUE_FN, json!({}), "value", ctx).await
    }
}

/// Map `kitchen.inventory.list` input → RPC params. An optional `location`
/// becomes `p_location`; an empty/whitespace value is rejected (omit instead).
fn inventory_list_params(input: &Value) -> Result<Value, String> {
    match input.get("location") {
        None | Some(Value::Null) => Ok(json!({})),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                return Err("`location` must be a non-empty string (omit it for all locations)".to_string());
            }
            Ok(json!({ "p_location": t }))
        }
        Some(_) => Err("`location` must be a string".to_string()),
    }
}

fn empty_object_schema() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

fn invalid(id: ToolId, detail: &str) -> ToolOutcome {
    ToolOutcome::Failed(aivyx_core::AivyxError::Tool {
        tool: id,
        detail: format!("kitchen.inventory.list: {detail}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_params_omits_filter_when_absent() {
        assert_eq!(inventory_list_params(&json!({})).unwrap(), json!({}));
        assert_eq!(inventory_list_params(&json!({"location": null})).unwrap(), json!({}));
    }

    #[test]
    fn list_params_maps_location_to_p_location_trimmed() {
        let p = inventory_list_params(&json!({"location": "  walk-in "})).unwrap();
        assert_eq!(p, json!({"p_location": "walk-in"}));
    }

    #[test]
    fn list_params_rejects_blank_and_non_string() {
        assert!(inventory_list_params(&json!({"location": "  "})).is_err());
        assert!(inventory_list_params(&json!({"location": 7})).is_err());
    }

    #[test]
    fn names_and_scopes_are_kitchen_read() {
        let c = Arc::new(KitchenClient::new(reqwest::Client::new(), "http://x", "k", "o"));
        assert_eq!(InventoryList::new(c.clone()).name(), "kitchen.inventory.list");
        assert_eq!(InventoryLowStock::new(c.clone()).name(), "kitchen.inventory.low_stock");
        assert_eq!(InventoryValue::new(c.clone()).name(), "kitchen.inventory.value");
        assert_eq!(
            InventoryLowStock::new(c).required_scope(&json!({})).to_string(),
            "kitchen.read"
        );
    }
}

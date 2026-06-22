//! `kitchen.inventory.*` read tools — Chapter Brigade BG.1.
//!
//! Three reads over KitchenDB inventory RPCs: the full stock list (optionally
//! filtered to a location), the low-stock report, and the total inventory
//! value. All `kitchen.read`. RPC function names are the KitchenDB convention
//! (`get_*`, org-scoped); confirmed against the live schema in-phase (OQ-3).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_vertical_sdk::capability::Scope;
use aivyx_vertical_sdk::tool::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome};

use super::{kitchen_read_scope, kitchen_write_scope, run_read, run_write};
use crate::client::KitchenClient;

const INVENTORY_LIST_FN: &str = "get_inventory";
const LOW_STOCK_FN: &str = "get_low_stock_items";
const INVENTORY_VALUE_FN: &str = "get_inventory_value";
const INVENTORY_ADJUST_FN: &str = "adjust_inventory";

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

/// `kitchen.inventory.adjust` — change an item's on-hand count (a stock count
/// correction, waste, or receipt). `kitchen.write`. KitchenDB applies the
/// adjustment + records the movement; the tool does not compute stock math.
pub struct InventoryAdjust {
    id: ToolId,
    schema: Value,
    client: Arc<KitchenClient>,
}

impl InventoryAdjust {
    pub fn new(client: Arc<KitchenClient>) -> Self {
        Self {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "sku": {
                        "type": "string",
                        "minLength": 1,
                        "description": "The item's SKU / identifier in KitchenDB."
                    },
                    "delta": {
                        "type": "number",
                        "description": "Signed change to the on-hand count (negative for waste/usage, positive for a receipt/correction)."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Optional human reason recorded with the movement (e.g. \"spoilage\", \"stock-count correction\")."
                    }
                },
                "required": ["sku", "delta"],
                "additionalProperties": false
            }),
            client,
        }
    }
}

#[async_trait]
impl Tool for InventoryAdjust {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "kitchen.inventory.adjust"
    }
    fn description(&self) -> &str {
        "Adjust an inventory item's on-hand count in KitchenDB. Requires `sku` \
         (string) and `delta` (signed number; negative for waste/usage); \
         optional `reason` (string). KitchenDB applies the change and records \
         the stock movement. Returns `{ adjustment: <KitchenDB row> }`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        kitchen_write_scope()
    }
    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        let params = match inventory_adjust_params(&input) {
            Ok(p) => p,
            Err(detail) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("kitchen.inventory.adjust: {detail}"),
                });
            }
        };
        run_write(&self.client, self.id, INVENTORY_ADJUST_FN, params, "adjustment", ctx).await
    }
}

/// Map `kitchen.inventory.adjust` input → RPC params: `sku`→`p_sku` (trimmed,
/// required), `delta`→`p_quantity_delta` (required number), `reason`→`p_reason`
/// (optional, trimmed).
fn inventory_adjust_params(input: &Value) -> Result<Value, String> {
    let sku = input
        .get("sku")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "`sku` is required (a non-empty string)".to_string())?;
    let delta = input
        .get("delta")
        .and_then(Value::as_f64)
        .ok_or_else(|| "`delta` is required (a signed number)".to_string())?;
    let mut params = json!({ "p_sku": sku, "p_quantity_delta": delta });
    if let Some(reason) = input.get("reason").and_then(|v| v.as_str()) {
        let r = reason.trim();
        if !r.is_empty() {
            params["p_reason"] = json!(r);
        }
    }
    Ok(params)
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
    ToolOutcome::Failed(aivyx_vertical_sdk::tool::AivyxError::Tool {
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

    #[test]
    fn adjust_params_maps_required_fields_and_optional_reason() {
        let p = inventory_adjust_params(&json!({"sku": " TOM-01 ", "delta": -3.5})).unwrap();
        assert_eq!(p, json!({"p_sku": "TOM-01", "p_quantity_delta": -3.5}));
        let p2 = inventory_adjust_params(
            &json!({"sku": "X", "delta": 2, "reason": " spoilage "}),
        )
        .unwrap();
        assert_eq!(p2["p_reason"], "spoilage");
        assert_eq!(p2["p_quantity_delta"], 2.0);
    }

    #[test]
    fn adjust_params_rejects_missing_sku_or_delta() {
        assert!(inventory_adjust_params(&json!({"delta": 1})).is_err());
        assert!(inventory_adjust_params(&json!({"sku": "X"})).is_err());
        assert!(inventory_adjust_params(&json!({"sku": "  ", "delta": 1})).is_err());
        assert!(inventory_adjust_params(&json!({"sku": "X", "delta": "lots"})).is_err());
    }

    #[test]
    fn adjust_is_kitchen_write() {
        let c = Arc::new(KitchenClient::new(reqwest::Client::new(), "http://x", "k", "o"));
        let t = InventoryAdjust::new(c);
        assert_eq!(t.name(), "kitchen.inventory.adjust");
        assert_eq!(t.required_scope(&json!({})).to_string(), "kitchen.write");
    }
}

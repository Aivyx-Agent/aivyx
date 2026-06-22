//! `kitchen.supplier.list` — Chapter Brigade BG.1. A read over the KitchenDB
//! suppliers RPC. `kitchen.read`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{Tool, ToolContext, ToolId, ToolOutcome};

use super::{kitchen_read_scope, run_read};
use crate::client::KitchenClient;

const SUPPLIER_LIST_FN: &str = "get_suppliers";

/// `kitchen.supplier.list` — the kitchen's suppliers (for sourcing / POs).
pub struct SupplierList {
    id: ToolId,
    schema: Value,
    client: Arc<KitchenClient>,
}

impl SupplierList {
    pub fn new(client: Arc<KitchenClient>) -> Self {
        Self {
            id: ToolId::new(),
            schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            client,
        }
    }
}

#[async_trait]
impl Tool for SupplierList {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "kitchen.supplier.list"
    }
    fn description(&self) -> &str {
        "List the kitchen's suppliers from KitchenDB (for sourcing and purchase \
         orders). No input. Returns `{ suppliers: [...], count }`. Read-only."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        kitchen_read_scope()
    }
    async fn execute(&self, _input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        run_read(&self.client, self.id, SUPPLIER_LIST_FN, json!({}), "suppliers", ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_scope() {
        let c = Arc::new(KitchenClient::new(reqwest::Client::new(), "http://x", "k", "o"));
        let t = SupplierList::new(c);
        assert_eq!(t.name(), "kitchen.supplier.list");
        assert_eq!(t.required_scope(&json!({})).to_string(), "kitchen.read");
    }
}

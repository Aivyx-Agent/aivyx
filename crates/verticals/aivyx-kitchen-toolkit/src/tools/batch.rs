//! `kitchen.batch.*` write tools — Chapter Brigade BG.2. Production-batch
//! lifecycle over KitchenDB RPCs: start a batch from a recipe, complete it with
//! an actual yield. Both `kitchen.write`. KitchenDB owns the batch state
//! machine + inventory consumption; the tools only name the RPC + map params.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome};

use super::{kitchen_write_scope, run_write};
use crate::client::KitchenClient;

const BATCH_START_FN: &str = "start_production_batch";
const BATCH_COMPLETE_FN: &str = "complete_production_batch";

/// `kitchen.batch.start` — open a production batch for a recipe + quantity.
pub struct BatchStart {
    id: ToolId,
    schema: Value,
    client: Arc<KitchenClient>,
}

impl BatchStart {
    pub fn new(client: Arc<KitchenClient>) -> Self {
        Self {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "recipe_id": {
                        "type": "string",
                        "minLength": 1,
                        "description": "The KitchenDB recipe identifier to produce."
                    },
                    "quantity": {
                        "type": "number",
                        "exclusiveMinimum": 0,
                        "description": "How many to produce (in the recipe's batch unit)."
                    },
                    "notes": {
                        "type": "string",
                        "description": "Optional notes recorded on the batch."
                    }
                },
                "required": ["recipe_id", "quantity"],
                "additionalProperties": false
            }),
            client,
        }
    }
}

#[async_trait]
impl Tool for BatchStart {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "kitchen.batch.start"
    }
    fn description(&self) -> &str {
        "Start a production batch in KitchenDB. Requires `recipe_id` (string) \
         and `quantity` (positive number); optional `notes` (string). KitchenDB \
         opens the batch and consumes ingredients per the recipe. Returns \
         `{ batch: <KitchenDB row> }`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        kitchen_write_scope()
    }
    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        let params = match batch_start_params(&input) {
            Ok(p) => p,
            Err(detail) => return invalid(self.id, "kitchen.batch.start", &detail),
        };
        run_write(&self.client, self.id, BATCH_START_FN, params, "batch", ctx).await
    }
}

/// `kitchen.batch.complete` — close an open batch with its actual yield.
pub struct BatchComplete {
    id: ToolId,
    schema: Value,
    client: Arc<KitchenClient>,
}

impl BatchComplete {
    pub fn new(client: Arc<KitchenClient>) -> Self {
        Self {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "batch_id": {
                        "type": "string",
                        "minLength": 1,
                        "description": "The open batch's KitchenDB identifier."
                    },
                    "actual_yield": {
                        "type": "number",
                        "minimum": 0,
                        "description": "Optional actual yield produced (defaults to the planned quantity in KitchenDB if omitted)."
                    }
                },
                "required": ["batch_id"],
                "additionalProperties": false
            }),
            client,
        }
    }
}

#[async_trait]
impl Tool for BatchComplete {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "kitchen.batch.complete"
    }
    fn description(&self) -> &str {
        "Complete an open production batch in KitchenDB. Requires `batch_id` \
         (string); optional `actual_yield` (non-negative number). KitchenDB \
         closes the batch and books the finished product. Returns \
         `{ batch: <KitchenDB row> }`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        kitchen_write_scope()
    }
    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        let params = match batch_complete_params(&input) {
            Ok(p) => p,
            Err(detail) => return invalid(self.id, "kitchen.batch.complete", &detail),
        };
        run_write(&self.client, self.id, BATCH_COMPLETE_FN, params, "batch", ctx).await
    }
}

/// `recipe_id`→`p_recipe_id` (trimmed, required), `quantity`→`p_quantity`
/// (required, > 0), `notes`→`p_notes` (optional, trimmed).
fn batch_start_params(input: &Value) -> Result<Value, String> {
    let recipe_id = input
        .get("recipe_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "`recipe_id` is required (a non-empty string)".to_string())?;
    let quantity = input
        .get("quantity")
        .and_then(Value::as_f64)
        .ok_or_else(|| "`quantity` is required (a number)".to_string())?;
    if quantity <= 0.0 {
        return Err("`quantity` must be greater than 0".to_string());
    }
    let mut params = json!({ "p_recipe_id": recipe_id, "p_quantity": quantity });
    if let Some(notes) = input.get("notes").and_then(|v| v.as_str()) {
        let n = notes.trim();
        if !n.is_empty() {
            params["p_notes"] = json!(n);
        }
    }
    Ok(params)
}

/// `batch_id`→`p_batch_id` (trimmed, required), `actual_yield`→
/// `p_actual_yield` (optional, >= 0).
fn batch_complete_params(input: &Value) -> Result<Value, String> {
    let batch_id = input
        .get("batch_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "`batch_id` is required (a non-empty string)".to_string())?;
    let mut params = json!({ "p_batch_id": batch_id });
    if let Some(y) = input.get("actual_yield") {
        let n = y
            .as_f64()
            .ok_or_else(|| "`actual_yield` must be a number".to_string())?;
        if n < 0.0 {
            return Err("`actual_yield` must be >= 0".to_string());
        }
        params["p_actual_yield"] = json!(n);
    }
    Ok(params)
}

fn invalid(id: ToolId, tool: &str, detail: &str) -> ToolOutcome {
    ToolOutcome::Failed(AivyxError::Tool { tool: id, detail: format!("{tool}: {detail}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_params_maps_required_plus_optional_notes() {
        let p = batch_start_params(&json!({"recipe_id": " R1 ", "quantity": 12})).unwrap();
        assert_eq!(p, json!({"p_recipe_id": "R1", "p_quantity": 12.0}));
        let p2 =
            batch_start_params(&json!({"recipe_id": "R", "quantity": 1, "notes": " rush "})).unwrap();
        assert_eq!(p2["p_notes"], "rush");
    }

    #[test]
    fn start_params_rejects_missing_or_nonpositive() {
        assert!(batch_start_params(&json!({"quantity": 1})).is_err());
        assert!(batch_start_params(&json!({"recipe_id": "R"})).is_err());
        assert!(batch_start_params(&json!({"recipe_id": "R", "quantity": 0})).is_err());
        assert!(batch_start_params(&json!({"recipe_id": "R", "quantity": -2})).is_err());
    }

    #[test]
    fn complete_params_requires_batch_id_and_optional_yield() {
        assert_eq!(
            batch_complete_params(&json!({"batch_id": " B1 "})).unwrap(),
            json!({"p_batch_id": "B1"})
        );
        let p = batch_complete_params(&json!({"batch_id": "B", "actual_yield": 9.5})).unwrap();
        assert_eq!(p["p_actual_yield"], 9.5);
    }

    #[test]
    fn complete_params_rejects_bad_yield_and_missing_id() {
        assert!(batch_complete_params(&json!({})).is_err());
        assert!(batch_complete_params(&json!({"batch_id": "B", "actual_yield": -1})).is_err());
        assert!(batch_complete_params(&json!({"batch_id": "B", "actual_yield": "lots"})).is_err());
    }

    #[test]
    fn names_and_scopes_are_kitchen_write() {
        let c = Arc::new(KitchenClient::new(reqwest::Client::new(), "http://x", "k", "o"));
        let s = BatchStart::new(c.clone());
        let f = BatchComplete::new(c);
        assert_eq!(s.name(), "kitchen.batch.start");
        assert_eq!(f.name(), "kitchen.batch.complete");
        assert_eq!(s.required_scope(&json!({})).to_string(), "kitchen.write");
        assert_eq!(f.required_scope(&json!({})).to_string(), "kitchen.write");
    }
}

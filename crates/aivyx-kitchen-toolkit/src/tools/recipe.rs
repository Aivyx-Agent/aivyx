//! `kitchen.recipe.search` — Chapter Brigade BG.1. A read over the KitchenDB
//! recipe search RPC. `kitchen.read`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome};

use super::{kitchen_read_scope, run_read};
use crate::client::KitchenClient;

const RECIPE_SEARCH_FN: &str = "search_recipes";

/// `kitchen.recipe.search` — find recipes by a free-text query.
pub struct RecipeSearch {
    id: ToolId,
    schema: Value,
    client: Arc<KitchenClient>,
}

impl RecipeSearch {
    pub fn new(client: Arc<KitchenClient>) -> Self {
        Self {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Free-text recipe search (name / ingredient / tag). KitchenDB handles matching."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            client,
        }
    }
}

#[async_trait]
impl Tool for RecipeSearch {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "kitchen.recipe.search"
    }
    fn description(&self) -> &str {
        "Search KitchenDB recipes by a free-text `query` (required string). \
         Returns `{ recipes: [...], count }`. Read-only."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        kitchen_read_scope()
    }
    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        let params = match recipe_search_params(&input) {
            Ok(p) => p,
            Err(detail) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("kitchen.recipe.search: {detail}"),
                });
            }
        };
        run_read(&self.client, self.id, RECIPE_SEARCH_FN, params, "recipes", ctx).await
    }
}

/// Map input → RPC params: the required `query` becomes `p_query` (trimmed).
fn recipe_search_params(input: &Value) -> Result<Value, String> {
    let q = input
        .get("query")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| "input must include a `query` string".to_string())?;
    if q.is_empty() {
        return Err("`query` must not be empty".to_string());
    }
    Ok(json!({ "p_query": q }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_params_maps_query_to_p_query_trimmed() {
        assert_eq!(
            recipe_search_params(&json!({"query": "  tomato sauce "})).unwrap(),
            json!({"p_query": "tomato sauce"})
        );
    }

    #[test]
    fn search_params_rejects_missing_or_empty_query() {
        assert!(recipe_search_params(&json!({})).is_err());
        assert!(recipe_search_params(&json!({"query": "   "})).is_err());
    }

    #[test]
    fn name_and_scope() {
        let c = Arc::new(KitchenClient::new(reqwest::Client::new(), "http://x", "k", "o"));
        let t = RecipeSearch::new(c);
        assert_eq!(t.name(), "kitchen.recipe.search");
        assert_eq!(t.required_scope(&json!({})).to_string(), "kitchen.read");
    }
}

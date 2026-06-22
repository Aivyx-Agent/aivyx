//! `kitchen.haccp.log` — Chapter Brigade BG.4. Append-only food-safety
//! (HACCP) logging. Each call writes one record to KitchenDB **and** lands as
//! exactly one row on the daemon's HMAC audit chain — so the food-safety log is
//! tamper-evident end to end (the EHO-export differentiator), with no new
//! `AuditEvent` variant (the per-call audit row carries it). Its own scope base
//! (`kitchen.haccp.log`) so the compliance specialist can hold ONLY this.
//!
//! Not confirm-first: logging a reading is a safe, expected, append-only act —
//! the opposite of `kitchen.order.send`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_vertical_sdk::capability::Scope;
use aivyx_vertical_sdk::tool::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome};

use super::{kitchen_haccp_scope, run_write};
use crate::client::KitchenClient;

const HACCP_LOG_FN: &str = "log_haccp_record";

/// `kitchen.haccp.log` — append a food-safety record (e.g. a fridge-temperature
/// reading) to KitchenDB.
pub struct HaccpLog {
    id: ToolId,
    schema: Value,
    client: Arc<KitchenClient>,
}

impl HaccpLog {
    pub fn new(client: Arc<KitchenClient>) -> Self {
        Self {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "check_type": {
                        "type": "string",
                        "minLength": 1,
                        "description": "The food-safety check (e.g. \"fridge_temperature\", \"cooking_temp\", \"cleaning\")."
                    },
                    "value": {
                        "type": "number",
                        "description": "Optional measured reading (e.g. a temperature in °C)."
                    },
                    "unit": {
                        "type": "string",
                        "description": "Optional unit for `value` (e.g. \"C\")."
                    },
                    "location": {
                        "type": "string",
                        "description": "Optional equipment / location the check applies to (e.g. \"walk-in fridge\")."
                    },
                    "passed": {
                        "type": "boolean",
                        "description": "Optional pass/fail verdict; pair an out-of-limit reading with the corrective action in `notes`."
                    },
                    "notes": {
                        "type": "string",
                        "description": "Optional free text (corrective action taken, observations)."
                    }
                },
                "required": ["check_type"],
                "additionalProperties": false
            }),
            client,
        }
    }
}

#[async_trait]
impl Tool for HaccpLog {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "kitchen.haccp.log"
    }
    fn description(&self) -> &str {
        "Append a food-safety (HACCP) record to KitchenDB. Requires `check_type` \
         (string); optional `value` (number), `unit` (string), `location` \
         (string), `passed` (bool), `notes` (string). Append-only and recorded \
         on the tamper-evident audit chain — log an out-of-limit reading with \
         its corrective action in `notes`. Returns `{ record: <KitchenDB row> }`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        kitchen_haccp_scope()
    }
    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        let params = match haccp_params(&input) {
            Ok(p) => p,
            Err(detail) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("kitchen.haccp.log: {detail}"),
                });
            }
        };
        run_write(&self.client, self.id, HACCP_LOG_FN, params, "record", ctx).await
    }
}

/// Map input → RPC params: `check_type`→`p_check_type` (trimmed, required);
/// optional `value`/`unit`/`location`/`passed`/`notes` → `p_*` when present.
fn haccp_params(input: &Value) -> Result<Value, String> {
    let check_type = input
        .get("check_type")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "`check_type` is required (a non-empty string)".to_string())?;
    let mut params = json!({ "p_check_type": check_type });
    if let Some(v) = input.get("value") {
        let n = v.as_f64().ok_or_else(|| "`value` must be a number".to_string())?;
        params["p_value"] = json!(n);
    }
    for (field, key) in [("unit", "p_unit"), ("location", "p_location"), ("notes", "p_notes")] {
        if let Some(s) = input.get(field).and_then(|v| v.as_str()) {
            let t = s.trim();
            if !t.is_empty() {
                params[key] = json!(t);
            }
        }
    }
    if let Some(p) = input.get("passed") {
        let b = p.as_bool().ok_or_else(|| "`passed` must be a boolean".to_string())?;
        params["p_passed"] = json!(b);
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haccp_params_requires_check_type() {
        assert!(haccp_params(&json!({})).is_err());
        assert!(haccp_params(&json!({"check_type": "  "})).is_err());
        assert_eq!(
            haccp_params(&json!({"check_type": " fridge_temperature "})).unwrap(),
            json!({"p_check_type": "fridge_temperature"})
        );
    }

    #[test]
    fn haccp_params_maps_optional_structured_fields() {
        let p = haccp_params(&json!({
            "check_type": "fridge_temperature",
            "value": 3.5,
            "unit": "C",
            "location": " walk-in ",
            "passed": false,
            "notes": " moved stock to backup fridge "
        }))
        .unwrap();
        assert_eq!(p["p_value"], 3.5);
        assert_eq!(p["p_unit"], "C");
        assert_eq!(p["p_location"], "walk-in");
        assert_eq!(p["p_passed"], false);
        assert_eq!(p["p_notes"], "moved stock to backup fridge");
    }

    #[test]
    fn haccp_params_rejects_wrong_types() {
        assert!(haccp_params(&json!({"check_type": "x", "value": "hot"})).is_err());
        assert!(haccp_params(&json!({"check_type": "x", "passed": "yes"})).is_err());
    }

    #[test]
    fn name_and_scope() {
        let c = Arc::new(KitchenClient::new(reqwest::Client::new(), "http://x", "k", "o"));
        let t = HaccpLog::new(c);
        assert_eq!(t.name(), "kitchen.haccp.log");
        assert_eq!(t.required_scope(&json!({})).to_string(), "kitchen.haccp.log");
    }
}

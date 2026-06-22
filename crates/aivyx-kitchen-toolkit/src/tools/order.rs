//! `kitchen.order.send` — Chapter Brigade BG.3. Dispatch a (drafted) purchase
//! order to its supplier. **This places a real order — money leaves the
//! building** — so it is **confirm-first**: without `confirmed: true` it returns
//! [`ToolOutcome::RequiresEscalation`], which the daemon turns into a human
//! approval gate and which [Chapter H](../../../docs/HEADLESS_MODE.md)
//! auto-blocks under any non-interactive policy. Its own scope base
//! (`kitchen.order.send`, distinct from `kitchen.write`) lets a roster grant
//! stock edits without granting the power to order.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome};

use super::{kitchen_order_send_scope, run_write};
use crate::client::KitchenClient;

const SEND_PO_FN: &str = "send_purchase_order";

/// The confirm-first decision for one `kitchen.order.send` call.
#[derive(Debug, PartialEq)]
enum OrderDecision {
    /// Well-formed but unconfirmed — escalate with this operator-facing reason.
    Escalate(String),
    /// Confirmed + well-formed — dispatch with these RPC params.
    Send(Value),
}

pub struct OrderSend {
    id: ToolId,
    schema: Value,
    client: Arc<KitchenClient>,
}

impl OrderSend {
    pub fn new(client: Arc<KitchenClient>) -> Self {
        Self {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "purchase_order_id": {
                        "type": "string",
                        "minLength": 1,
                        "description": "The drafted purchase order's KitchenDB identifier."
                    },
                    "notes": {
                        "type": "string",
                        "description": "Optional note sent with the order."
                    },
                    "confirmed": {
                        "type": "boolean",
                        "description": "Must be true to actually send. Without it the tool escalates for human approval — sending a PO places a real order with the supplier."
                    }
                },
                "required": ["purchase_order_id"],
                "additionalProperties": false
            }),
            client,
        }
    }
}

#[async_trait]
impl Tool for OrderSend {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "kitchen.order.send"
    }
    fn description(&self) -> &str {
        "Dispatch a drafted purchase order to its supplier in KitchenDB. \
         Requires `purchase_order_id` (string); optional `notes` (string). \
         CONFIRM-FIRST: this places a real order (money leaves the building), so \
         without `confirmed: true` the tool escalates for human approval rather \
         than sending. Returns `{ order: <KitchenDB row> }` once sent."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        kitchen_order_send_scope()
    }
    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        match decide_order(&input) {
            Err(detail) => ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("kitchen.order.send: {detail}"),
            }),
            Ok(OrderDecision::Escalate(reason)) => ToolOutcome::RequiresEscalation { reason },
            Ok(OrderDecision::Send(params)) => {
                run_write(&self.client, self.id, SEND_PO_FN, params, "order", ctx).await
            }
        }
    }
}

/// Validate input, then apply the confirm-first gate. `Err` = malformed input
/// (→ `Failed`); `Escalate` = well-formed but unconfirmed (→ `RequiresEscalation`);
/// `Send` = confirmed + well-formed (→ the RPC). Pure, so the gate is fully
/// unit-tested without a `ToolContext`.
fn decide_order(input: &Value) -> Result<OrderDecision, String> {
    let po = input
        .get("purchase_order_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "`purchase_order_id` is required (a non-empty string)".to_string())?;

    let confirmed = input.get("confirmed").and_then(|v| v.as_bool()) == Some(true);
    if !confirmed {
        return Ok(OrderDecision::Escalate(format!(
            "About to SEND purchase order {po} to its supplier — this places a real \
             order (money leaves the building). Show the operator the PO; on approval, \
             re-call kitchen.order.send with `confirmed: true`."
        )));
    }

    let mut params = json!({ "p_purchase_order_id": po });
    if let Some(notes) = input.get("notes").and_then(|v| v.as_str()) {
        let n = notes.trim();
        if !n.is_empty() {
            params["p_notes"] = json!(n);
        }
    }
    Ok(OrderDecision::Send(params))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfirmed_well_formed_call_escalates() {
        let d = decide_order(&json!({"purchase_order_id": "PO-42"})).unwrap();
        match d {
            OrderDecision::Escalate(reason) => {
                assert!(reason.contains("PO-42"), "{reason}");
                assert!(reason.contains("confirmed: true"), "{reason}");
            }
            other => panic!("expected Escalate; got {other:?}"),
        }
    }

    #[test]
    fn confirmed_false_still_escalates() {
        let d = decide_order(&json!({"purchase_order_id": "P", "confirmed": false})).unwrap();
        assert!(matches!(d, OrderDecision::Escalate(_)));
    }

    #[test]
    fn confirmed_true_sends_with_trimmed_po_and_notes() {
        let d = decide_order(&json!({
            "purchase_order_id": " PO-7 ",
            "confirmed": true,
            "notes": " deliver am "
        }))
        .unwrap();
        match d {
            OrderDecision::Send(params) => {
                assert_eq!(params["p_purchase_order_id"], "PO-7");
                assert_eq!(params["p_notes"], "deliver am");
            }
            other => panic!("expected Send; got {other:?}"),
        }
    }

    #[test]
    fn missing_po_id_is_a_hard_error_even_when_confirmed() {
        assert!(decide_order(&json!({"confirmed": true})).is_err());
        assert!(decide_order(&json!({"purchase_order_id": "  ", "confirmed": true})).is_err());
    }

    #[test]
    fn name_and_scope() {
        let c = Arc::new(KitchenClient::new(reqwest::Client::new(), "http://x", "k", "o"));
        let t = OrderSend::new(c);
        assert_eq!(t.name(), "kitchen.order.send");
        assert_eq!(t.required_scope(&json!({})).to_string(), "kitchen.order.send");
    }
}

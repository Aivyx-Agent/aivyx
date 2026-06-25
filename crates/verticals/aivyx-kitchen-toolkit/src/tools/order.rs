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

use aivyx_vertical_sdk::capability::Scope;
use aivyx_vertical_sdk::tool::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome};

use super::{kitchen_order_send_scope, kitchen_write_scope, run_write};
use crate::client::KitchenClient;

const SEND_PO_FN: &str = "send_purchase_order";
const DRAFT_PO_FN: &str = "draft_purchase_order";

/// `kitchen.order.draft` — Chapter Lockup (LK.1). Turn the low-stock list into
/// per-supplier draft purchase orders. **Not confirm-first** — drafting writes a
/// reversible draft and spends nothing; only `kitchen.order.send` dispatches.
/// Gated on `kitchen.write` (a normal kitchen mutation, not the outbound
/// `order.send` power). KitchenDB owns per-supplier grouping / pack-size rules.
pub struct OrderDraft {
    id: ToolId,
    schema: Value,
    client: Arc<KitchenClient>,
}

impl OrderDraft {
    pub fn new(client: Arc<KitchenClient>) -> Self {
        Self {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "description": "Optional explicit reorder list. Omit to let KitchenDB draft from the current low-stock report.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "sku": { "type": "string", "minLength": 1, "description": "Item SKU to order." },
                                "quantity": { "type": "number", "exclusiveMinimum": 0, "description": "Quantity to order." }
                            },
                            "required": ["sku", "quantity"],
                            "additionalProperties": false
                        }
                    },
                    "notes": { "type": "string", "description": "Optional note recorded on the draft order(s)." }
                },
                "additionalProperties": false
            }),
            client,
        }
    }
}

#[async_trait]
impl Tool for OrderDraft {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "kitchen.order.draft"
    }
    fn description(&self) -> &str {
        "Draft per-supplier purchase orders in KitchenDB. Optional `items` \
         (array of `{sku, quantity}`) gives an explicit reorder list; omit it to \
         draft from the current low-stock report. Optional `notes` (string). \
         KitchenDB groups by supplier and applies pack-size / minimum rules. \
         This DRAFTS only — use kitchen.order.send (confirm-first) to dispatch. \
         Returns `{ draft: <KitchenDB row(s)> }`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        kitchen_write_scope()
    }
    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        let params = match order_draft_params(&input) {
            Ok(p) => p,
            Err(detail) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("kitchen.order.draft: {detail}"),
                });
            }
        };
        run_write(&self.client, self.id, DRAFT_PO_FN, params, "draft", ctx).await
    }
}

/// Map input → RPC params: optional `items` → `p_items` (each validated to a
/// non-empty `sku` + positive `quantity`); optional `notes` → `p_notes`. No
/// items → an empty object (the client injects `p_organization_id`; KitchenDB
/// auto-drafts from low-stock).
fn order_draft_params(input: &Value) -> Result<Value, String> {
    let mut params = json!({});
    if let Some(items) = input.get("items") {
        let arr = items.as_array().ok_or_else(|| "`items` must be an array".to_string())?;
        let mut out = Vec::with_capacity(arr.len());
        for (i, it) in arr.iter().enumerate() {
            let sku = it
                .get("sku")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("items[{i}] needs a non-empty `sku` string"))?;
            let qty = it
                .get("quantity")
                .and_then(Value::as_f64)
                .ok_or_else(|| format!("items[{i}] needs a numeric `quantity`"))?;
            if qty <= 0.0 {
                return Err(format!("items[{i}].quantity must be greater than 0"));
            }
            out.push(json!({ "sku": sku, "quantity": qty }));
        }
        params["p_items"] = json!(out);
    }
    if let Some(notes) = input.get("notes").and_then(|v| v.as_str()) {
        let n = notes.trim();
        if !n.is_empty() {
            params["p_notes"] = json!(n);
        }
    }
    Ok(params)
}

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
            // RN.3 — the parent turn loop stamps the authoritative scope
            // (`kitchen.order.send`) when this crosses the tool-process bridge;
            // the child need not provide it.
            Ok(OrderDecision::Escalate(reason)) => {
                ToolOutcome::RequiresEscalation { reason, scope: None }
            }
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

    // ---- kitchen.order.draft (LK.1) ----

    #[test]
    fn draft_params_empty_when_no_items() {
        assert_eq!(order_draft_params(&json!({})).unwrap(), json!({}));
    }

    #[test]
    fn draft_params_maps_items_and_notes() {
        let p = order_draft_params(&json!({
            "items": [{"sku": " TOM-01 ", "quantity": 6}, {"sku": "OIL", "quantity": 2.5}],
            "notes": " am delivery "
        }))
        .unwrap();
        assert_eq!(p["p_items"][0], json!({"sku": "TOM-01", "quantity": 6.0}));
        assert_eq!(p["p_items"][1], json!({"sku": "OIL", "quantity": 2.5}));
        assert_eq!(p["p_notes"], "am delivery");
    }

    #[test]
    fn draft_params_rejects_bad_items() {
        assert!(order_draft_params(&json!({"items": "nope"})).is_err());
        assert!(order_draft_params(&json!({"items": [{"quantity": 1}]})).is_err());
        assert!(order_draft_params(&json!({"items": [{"sku": "X"}]})).is_err());
        assert!(order_draft_params(&json!({"items": [{"sku": "X", "quantity": 0}]})).is_err());
    }

    #[test]
    fn draft_name_and_scope_is_kitchen_write_not_confirm_first() {
        let c = Arc::new(KitchenClient::new(reqwest::Client::new(), "http://x", "k", "o"));
        let t = OrderDraft::new(c);
        assert_eq!(t.name(), "kitchen.order.draft");
        // Drafting is a normal write, NOT the confirm-first order.send power.
        assert_eq!(t.required_scope(&json!({})).to_string(), "kitchen.write");
    }
}

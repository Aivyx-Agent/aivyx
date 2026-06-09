//! `McpConn` — the shared per-server connection state.
//!
//! One `McpConn` is held via `Arc` by the bridge **and** every proxy it
//! produces, so all callers on a given MCP server share:
//! - the transport,
//! - the JSON-RPC id counter,
//! - the demuxed `*/list_changed` pending set, and
//! - a **round-trip lock** that serializes each whole request→response
//!   exchange.
//!
//! The lock is what makes the live refresh safe: the agent's tool calls
//! and the refresh coordinator's `rediscover()` both read the same
//! transport, and without serializing the send→receive pair they could
//! mis-correlate responses (read each other's replies). Holding the lock
//! across the round-trip — and demuxing notifications inside it — keeps
//! every caller's response its own.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::jsonrpc::{Request, Response};
use crate::notifications::{classify_incoming, Incoming, ListKind};
use crate::transport_trait::McpTransport;

pub struct McpConn {
    transport: Arc<dyn McpTransport>,
    next_id: AtomicU64,
    pending: Mutex<BTreeSet<ListKind>>,
    call_lock: tokio::sync::Mutex<()>,
}

impl McpConn {
    pub(crate) fn new(transport: Arc<dyn McpTransport>) -> Arc<Self> {
        Arc::new(McpConn {
            transport,
            next_id: AtomicU64::new(1),
            pending: Mutex::new(BTreeSet::new()),
            call_lock: tokio::sync::Mutex::new(()),
        })
    }

    /// The underlying transport (used for transport-specific shutdown).
    pub(crate) fn transport(&self) -> &Arc<dyn McpTransport> {
        &self.transport
    }

    /// Peek at the primitives whose `*/list_changed` fired since the
    /// last drain.
    pub(crate) fn pending(&self) -> Vec<ListKind> {
        self.pending.lock().unwrap().iter().copied().collect()
    }

    /// Drain the pending-refresh set.
    pub(crate) fn take_pending(&self) -> Vec<ListKind> {
        std::mem::take(&mut *self.pending.lock().unwrap())
            .into_iter()
            .collect()
    }

    /// A locked, notification-demuxing JSON-RPC round-trip. Any
    /// `*/list_changed` that interleaves is recorded into `pending`;
    /// other server-initiated notifications are skipped; the matching
    /// response is returned.
    pub(crate) async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<T, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = Request::new(id, method, params);
        let mut line =
            serde_json::to_string(&req).map_err(|e| format!("serialize request: {e}"))?;
        line.push('\n');

        let resp_line = {
            let _guard = self.call_lock.lock().await;
            self.transport.send(&line).await?;
            loop {
                let inbound = self.transport.receive().await?;
                match classify_incoming(&inbound) {
                    Incoming::ListChanged(kind) => {
                        self.pending.lock().unwrap().insert(kind);
                    }
                    Incoming::OtherNotification => {}
                    Incoming::Response | Incoming::Unknown => break inbound,
                }
            }
        };

        let resp: Response =
            serde_json::from_str(&resp_line).map_err(|e| format!("parse MCP response: {e}"))?;
        if resp.id != id {
            return Err(format!(
                "MCP response id mismatch: expected {id}, got {}",
                resp.id
            ));
        }
        if let Some(err) = resp.error {
            return Err(format!("MCP server error: {err}"));
        }
        let result = resp
            .result
            .ok_or_else(|| "MCP response has no result".to_string())?;
        serde_json::from_value(result).map_err(|e| format!("deserialize MCP result: {e}"))
    }

    /// Send a fire-and-forget notification (no response expected),
    /// serialized against in-flight round-trips.
    pub(crate) async fn send_notification(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Notification<'a> {
            jsonrpc: &'static str,
            method: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            params: Option<Value>,
        }
        let notif = Notification {
            jsonrpc: "2.0",
            method,
            params,
        };
        let mut line =
            serde_json::to_string(&notif).map_err(|e| format!("serialize notification: {e}"))?;
        line.push('\n');
        let _guard = self.call_lock.lock().await;
        self.transport.send(&line).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::MockTransport;

    #[tokio::test]
    async fn call_demuxes_list_changed_and_records_pending() {
        let conn = McpConn::new(Arc::new(MockTransport::new(vec![
            r#"{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}"#.to_string(),
            r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#.to_string(),
        ])));
        let v: Value = conn.call("anything", None).await.unwrap();
        assert_eq!(v["ok"], serde_json::json!(true));
        assert_eq!(conn.take_pending(), vec![ListKind::Tools]);
        assert!(conn.take_pending().is_empty(), "drained");
    }

    #[tokio::test]
    async fn call_surfaces_id_mismatch() {
        let conn = McpConn::new(Arc::new(MockTransport::new(vec![
            r#"{"jsonrpc":"2.0","id":99,"result":{}}"#.to_string(),
        ])));
        let out: Result<Value, String> = conn.call("m", None).await;
        assert!(out.unwrap_err().contains("id mismatch"));
    }
}

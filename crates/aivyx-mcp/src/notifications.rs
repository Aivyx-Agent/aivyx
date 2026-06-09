//! Inbound JSON-RPC message classification — distinguishes a response
//! (a reply to one of our requests) from a server-initiated
//! notification, and recognizes the three `*/list_changed`
//! notifications that mean "re-discover this primitive".
//!
//! The bridge's `call` loop uses this to stay robust when a server
//! interleaves notifications with responses (previously an id-mismatch
//! error), and to record which primitives need re-discovery.

use serde_json::Value;

/// Which primitive's list a `notifications/<x>/list_changed` refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ListKind {
    Tools,
    Resources,
    Prompts,
}

impl ListKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ListKind::Tools => "tools",
            ListKind::Resources => "resources",
            ListKind::Prompts => "prompts",
        }
    }
}

/// The classification of one inbound JSON-RPC message line.
#[derive(Debug, PartialEq, Eq)]
pub enum Incoming {
    /// No `method` field — a reply to one of our requests.
    Response,
    /// A `notifications/<kind>/list_changed` — re-discover that primitive.
    ListChanged(ListKind),
    /// Some other server-initiated notification or request we don't act
    /// on (logging, progress, sampling, …) — safe to skip.
    OtherNotification,
    /// Not valid JSON — let the caller surface a parse error.
    Unknown,
}

/// Classify one inbound message line. Pure.
pub fn classify_incoming(line: &str) -> Incoming {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Incoming::Unknown;
    };
    match v.get("method").and_then(Value::as_str) {
        // A JSON-RPC response carries no `method`.
        None => Incoming::Response,
        Some("notifications/tools/list_changed") => Incoming::ListChanged(ListKind::Tools),
        Some("notifications/resources/list_changed") => Incoming::ListChanged(ListKind::Resources),
        Some("notifications/prompts/list_changed") => Incoming::ListChanged(ListKind::Prompts),
        Some(_) => Incoming::OtherNotification,
    }
}

/// Read the `listChanged` flag from a capability object (e.g.
/// `"tools": { "listChanged": true }`). Absent / non-bool → false.
pub(crate) fn list_changed_flag(capability: &Option<Value>) -> bool {
    capability
        .as_ref()
        .and_then(|v| v.get("listChanged"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_has_no_method() {
        assert_eq!(
            classify_incoming(r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}"#),
            Incoming::Response
        );
    }

    #[test]
    fn recognizes_each_list_changed() {
        assert_eq!(
            classify_incoming(r#"{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}"#),
            Incoming::ListChanged(ListKind::Tools)
        );
        assert_eq!(
            classify_incoming(r#"{"jsonrpc":"2.0","method":"notifications/resources/list_changed"}"#),
            Incoming::ListChanged(ListKind::Resources)
        );
        assert_eq!(
            classify_incoming(r#"{"jsonrpc":"2.0","method":"notifications/prompts/list_changed"}"#),
            Incoming::ListChanged(ListKind::Prompts)
        );
    }

    #[test]
    fn other_notification_is_skippable() {
        assert_eq!(
            classify_incoming(r#"{"jsonrpc":"2.0","method":"notifications/message","params":{}}"#),
            Incoming::OtherNotification
        );
    }

    #[test]
    fn garbage_is_unknown() {
        assert_eq!(classify_incoming("not json"), Incoming::Unknown);
    }

    #[test]
    fn list_changed_flag_reads_capability() {
        assert!(list_changed_flag(&Some(serde_json::json!({ "listChanged": true }))));
        assert!(!list_changed_flag(&Some(serde_json::json!({}))));
        assert!(!list_changed_flag(&None));
    }
}

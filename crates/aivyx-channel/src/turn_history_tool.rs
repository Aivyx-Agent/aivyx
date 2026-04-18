//! `TurnHistoryTool` — Phase 28 Task 5: first Reflection Layer primitive.
//!
//! A read-only tool that queries the persistent audit chain and returns
//! recent turn outcomes. The agent uses this to observe its own recent
//! behavior — the observation substrate that future reflection loops
//! compose against.
//!
//! ## Input shape
//!
//! ```json
//! { "limit": 10, "since_ms": 1713400000000 }
//! ```
//!
//! Both fields are optional. `limit` defaults to 10; `since_ms` filters
//! to turns started at or after the given epoch-millisecond timestamp.
//!
//! ## Output
//!
//! ```json
//! {
//!   "turns": [
//!     {
//!       "turn_id": "...",
//!       "session_id": "...",
//!       "channel": "Local",
//!       "outcome": "Completed",
//!       "tool_calls_made": 3,
//!       "duration_ms": 1420,
//!       "started_at": 1713400012345
//!     }
//!   ]
//! }
//! ```

use std::sync::OnceLock;
use std::time::{Duration, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_audit::{AuditEvent, AuditLog};
use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

pub struct TurnHistoryTool {
    id: ToolId,
    schema: Value,
    audit_log: OnceLock<std::sync::Arc<dyn AuditLog + Send + Sync>>,
}

impl std::fmt::Debug for TurnHistoryTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnHistoryTool")
            .field("id", &self.id)
            .field("has_audit_log", &self.audit_log.get().is_some())
            .finish()
    }
}

impl Default for TurnHistoryTool {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnHistoryTool {
    pub fn new() -> Self {
        TurnHistoryTool {
            id: ToolId::new(),
            schema: turn_history_input_schema(),
            audit_log: OnceLock::new(),
        }
    }

    pub fn set_audit_log(
        &self,
        log: std::sync::Arc<dyn AuditLog + Send + Sync>,
    ) -> Result<(), std::sync::Arc<dyn AuditLog + Send + Sync>> {
        self.audit_log.set(log)
    }
}

#[async_trait]
impl Tool for TurnHistoryTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "turn.history"
    }

    fn description(&self) -> &str {
        "Query recent turn outcomes from the audit chain. Returns an array \
         of turn summaries with outcome, tool call count, duration, and \
         timestamps. Use this to reflect on your own recent behavior."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("audit.read").expect("known base")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(log) = self.audit_log.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "turn.history: no audit log configured".to_string(),
            });
        };

        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;

        let since_ms = input
            .get("since_ms")
            .and_then(|v| v.as_u64());

        let since_time = since_ms.map(|ms| UNIX_EPOCH + Duration::from_millis(ms));

        // Walk the chain backwards collecting TurnStarted/TurnEnded pairs.
        let total = log.len();
        let mut turns: Vec<Value> = Vec::new();

        // Build a map of turn_id → (started_entry, ended_entry) by scanning
        // from the end. We collect TurnEnded first, then match with TurnStarted.
        let mut ended_map: std::collections::HashMap<String, (String, usize, Duration)> =
            std::collections::HashMap::new();

        // Scan backwards to find recent turns efficiently.
        let mut seq = total as u64;
        while seq > 0 && turns.len() < limit {
            seq -= 1;
            let Some(entry) = log.get(seq) else { break };

            match &entry.event {
                AuditEvent::TurnStarted {
                    turn_id,
                    session_id,
                    channel,
                    ..
                } => {
                    let turn_id_str = format!("{turn_id:?}");
                    if let Some((outcome, tool_calls_made, duration)) =
                        ended_map.remove(&turn_id_str)
                    {
                        // Apply since_ms filter on the start time.
                        if let Some(threshold) = since_time {
                            if entry.appended_at < threshold {
                                break;
                            }
                        }

                        let started_at_ms = entry
                            .appended_at
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;

                        turns.push(json!({
                            "turn_id": turn_id_str,
                            "session_id": format!("{session_id:?}"),
                            "channel": format!("{channel:?}"),
                            "outcome": outcome,
                            "tool_calls_made": tool_calls_made,
                            "duration_ms": duration.as_millis() as u64,
                            "started_at": started_at_ms,
                        }));
                    }
                }
                AuditEvent::TurnEnded {
                    turn_id,
                    outcome,
                    tool_calls_made,
                    duration,
                    ..
                } => {
                    let turn_id_str = format!("{turn_id:?}");
                    ended_map.insert(
                        turn_id_str,
                        (format!("{outcome:?}"), *tool_calls_made, *duration),
                    );
                }
                _ => {}
            }
        }

        // Reverse so oldest is first (chronological order).
        turns.reverse();

        ToolOutcome::Completed {
            output: json!({ "turns": turns }),
            verified: Verification::NotApplicable,
        }
    }
}

fn turn_history_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "limit": {
                "type": "integer",
                "description": "Maximum number of recent turns to return (default 10)"
            },
            "since_ms": {
                "type": "integer",
                "description": "Only return turns started at or after this epoch-millisecond timestamp"
            }
        },
        "required": []
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_scope_is_audit_read() {
        let tool = TurnHistoryTool::new();
        let scope = tool.required_scope(&json!({}));
        assert_eq!(scope.as_str(), "audit.read");
    }

    #[test]
    fn name_and_schema() {
        let tool = TurnHistoryTool::new();
        assert_eq!(tool.name(), "turn.history");
        let schema = tool.input_schema();
        assert!(schema["properties"]["limit"].is_object());
        assert!(schema["properties"]["since_ms"].is_object());
    }
}

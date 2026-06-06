//! Phase 183 — the `remind.*` channel-tier tools.
//!
//! Daemon-native (the `loop.*` / `mission.*` pattern), because a
//! reminder must *push* a notification at a time — the daemon's
//! scheduler cadence + notify dispatcher own that. The reminder
//! driver fires due reminders; these tools just CRUD the store.
//!
//! `remind.set` takes an **absolute** time (`at`) as either unix
//! seconds or an RFC3339 string. Natural language ("6pm", "in 2
//! hours") is resolved to an absolute time by the **agent** using
//! the turn's clock context — no date-parsing burden on the tool
//! beyond accepting the two stable formats.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

use crate::reminder_store::{Reminder, ReminderStore};

/// Shared handle injected into the tools after storage opens.
pub type SharedReminderStore = Arc<ReminderStore>;

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse the `at` field: a JSON number (unix seconds) or a string
/// (numeric unix seconds, or RFC3339). Pure + testable.
pub fn parse_at(value: &Value) -> Result<i64, String> {
    if let Some(n) = value.as_i64() {
        return Ok(n);
    }
    if let Some(s) = value.as_str() {
        let s = s.trim();
        if let Ok(n) = s.parse::<i64>() {
            return Ok(n);
        }
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
            return Ok(dt.timestamp());
        }
        return Err(format!(
            "`at` string {s:?} is neither unix seconds nor RFC3339 \
             (e.g. \"2026-06-07T18:00:00Z\")"
        ));
    }
    Err("`at` must be unix seconds (number) or an RFC3339 string"
        .to_string())
}

fn missing_store(id: ToolId) -> ToolOutcome {
    ToolOutcome::Failed(AivyxError::Tool {
        tool: id,
        detail: "reminder tool invoked without a store; the session \
                 layer must inject the ReminderStore after opening \
                 storage"
            .to_string(),
    })
}

// ---------------------------------------------------------------------------
// remind.set
// ---------------------------------------------------------------------------

pub struct RemindSetTool {
    id: ToolId,
    schema: Value,
    store: OnceLock<SharedReminderStore>,
}

impl std::fmt::Debug for RemindSetTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemindSetTool").finish()
    }
}

impl Default for RemindSetTool {
    fn default() -> Self {
        Self::new()
    }
}

impl RemindSetTool {
    pub fn new() -> Self {
        RemindSetTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "at": {
                        "description": "When to fire — unix seconds \
                         (number) or an RFC3339 timestamp. Resolve \
                         natural language ('6pm', 'in 2 hours') to an \
                         absolute time yourself using the current time.",
                    },
                    "message": {
                        "type": "string",
                        "description": "What to remind the operator."
                    },
                    "notify_targets": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional notify target names; \
                         omit to use the operator's defaults."
                    }
                },
                "required": ["at", "message"]
            }),
            store: OnceLock::new(),
        }
    }

    pub fn set_store(
        &self,
        store: SharedReminderStore,
    ) -> Result<(), SharedReminderStore> {
        self.store.set(store)
    }
}

#[async_trait]
impl Tool for RemindSetTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "remind.set"
    }
    fn description(&self) -> &str {
        "Set a one-shot reminder. Input: `{ \"at\": <unix seconds or \
         RFC3339>, \"message\": <text>, \"notify_targets\"?: [..] }`. \
         At the time, the operator is notified with the message. \
         Resolve natural-language times to an absolute `at` yourself. \
         Returns `{ \"id\", \"due_unix\" }`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("remind.write").expect("known base")
    }

    async fn execute(
        &self,
        input: Value,
        _ctx: &ToolContext<'_>,
    ) -> ToolOutcome {
        let Some(store) = self.store.get() else {
            return missing_store(self.id);
        };
        let due_unix = match input.get("at").map(parse_at) {
            Some(Ok(t)) => t,
            Some(Err(e)) => return fail(self.id, &e),
            None => return fail(self.id, "`at` is required"),
        };
        let message = match input.get("message").and_then(|v| v.as_str()) {
            Some(m) if !m.trim().is_empty() => m.trim().to_string(),
            _ => return fail(self.id, "`message` must be a non-empty string"),
        };
        let notify_targets: Vec<String> = input
            .get("notify_targets")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let reminder = Reminder {
            id: uuid::Uuid::new_v4().to_string(),
            due_unix,
            message,
            notify_targets,
            created_unix: now_unix(),
        };
        match store.set(&reminder).await {
            Ok(()) => ToolOutcome::Completed {
                output: json!({
                    "id": reminder.id,
                    "due_unix": reminder.due_unix,
                }),
                verified: Verification::NotApplicable,
            },
            Err(e) => fail(self.id, &format!("failed to store reminder: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// remind.list
// ---------------------------------------------------------------------------

pub struct RemindListTool {
    id: ToolId,
    schema: Value,
    store: OnceLock<SharedReminderStore>,
}

impl std::fmt::Debug for RemindListTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemindListTool").finish()
    }
}

impl Default for RemindListTool {
    fn default() -> Self {
        Self::new()
    }
}

impl RemindListTool {
    pub fn new() -> Self {
        RemindListTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object", "properties": {}, "required": []
            }),
            store: OnceLock::new(),
        }
    }
    pub fn set_store(
        &self,
        store: SharedReminderStore,
    ) -> Result<(), SharedReminderStore> {
        self.store.set(store)
    }
}

#[async_trait]
impl Tool for RemindListTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "remind.list"
    }
    fn description(&self) -> &str {
        "List pending reminders, soonest first. Returns \
         `{ \"reminders\": [ { \"id\", \"due_unix\", \"message\" } ] }`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("remind.read").expect("known base")
    }

    async fn execute(
        &self,
        _input: Value,
        _ctx: &ToolContext<'_>,
    ) -> ToolOutcome {
        let Some(store) = self.store.get() else {
            return missing_store(self.id);
        };
        match store.list().await {
            Ok(reminders) => {
                let items: Vec<Value> = reminders
                    .iter()
                    .map(|r| {
                        json!({
                            "id": r.id,
                            "due_unix": r.due_unix,
                            "message": r.message,
                        })
                    })
                    .collect();
                ToolOutcome::Completed {
                    output: json!({ "reminders": items }),
                    verified: Verification::NotApplicable,
                }
            }
            Err(e) => fail(self.id, &format!("failed to list reminders: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// remind.cancel
// ---------------------------------------------------------------------------

pub struct RemindCancelTool {
    id: ToolId,
    schema: Value,
    store: OnceLock<SharedReminderStore>,
}

impl std::fmt::Debug for RemindCancelTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemindCancelTool").finish()
    }
}

impl Default for RemindCancelTool {
    fn default() -> Self {
        Self::new()
    }
}

impl RemindCancelTool {
    pub fn new() -> Self {
        RemindCancelTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string",
                            "description": "The reminder id to cancel." }
                },
                "required": ["id"]
            }),
            store: OnceLock::new(),
        }
    }
    pub fn set_store(
        &self,
        store: SharedReminderStore,
    ) -> Result<(), SharedReminderStore> {
        self.store.set(store)
    }
}

#[async_trait]
impl Tool for RemindCancelTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "remind.cancel"
    }
    fn description(&self) -> &str {
        "Cancel a pending reminder by id. Returns \
         `{ \"canceled\": <bool> }` (false if no reminder had that id)."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("remind.write").expect("known base")
    }

    async fn execute(
        &self,
        input: Value,
        _ctx: &ToolContext<'_>,
    ) -> ToolOutcome {
        let Some(store) = self.store.get() else {
            return missing_store(self.id);
        };
        let id = match input.get("id").and_then(|v| v.as_str()) {
            Some(i) if !i.trim().is_empty() => i.trim(),
            _ => return fail(self.id, "`id` must be a non-empty string"),
        };
        match store.cancel(id).await {
            Ok(canceled) => ToolOutcome::Completed {
                output: json!({ "canceled": canceled }),
                verified: Verification::NotApplicable,
            },
            Err(e) => fail(self.id, &format!("failed to cancel reminder: {e}")),
        }
    }
}

fn fail(id: ToolId, detail: &str) -> ToolOutcome {
    ToolOutcome::Failed(AivyxError::Tool {
        tool: id,
        detail: detail.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_core::{
        AgentId, CancellationToken, ChannelContext, ChannelError,
        ChannelPlatform, SessionId, StreamEvent, TurnId, TurnOutcome,
    };
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{
        KeyDomain, RedbStorage, Storage, StorageConfig,
    };

    struct NoopChannel {
        session: SessionId,
        token: CancellationToken,
    }
    #[async_trait]
    impl ChannelContext for NoopChannel {
        fn session_id(&self) -> SessionId {
            self.session
        }
        fn platform(&self) -> ChannelPlatform {
            ChannelPlatform::Local
        }
        fn channel_name(&self) -> &str {
            "test"
        }
        fn trust_tier(&self) -> aivyx_capability::TrustTier {
            aivyx_capability::TrustTier::Trusted
        }
        async fn stream_event(
            &self,
            _event: StreamEvent<'_>,
        ) -> Result<(), ChannelError> {
            Ok(())
        }
        async fn finalize(
            &self,
            _outcome: &TurnOutcome,
        ) -> Result<(), ChannelError> {
            Ok(())
        }
        fn cancellation_token(&self) -> CancellationToken {
            self.token.clone()
        }
    }
    struct NoopAudit;
    impl aivyx_core::AuditHook for NoopAudit {
        fn on_event(&self, _tag: aivyx_core::AuditTag) {}
    }
    fn ctx_parts() -> (NoopChannel, NoopAudit) {
        (
            NoopChannel {
                session: SessionId::new(),
                token: CancellationToken::new(),
            },
            NoopAudit,
        )
    }
    fn make_ctx<'a>(
        ch: &'a NoopChannel,
        audit: &'a dyn aivyx_core::AuditHook,
    ) -> ToolContext<'a> {
        ToolContext {
            agent_id: AgentId::new(),
            session_id: ch.session,
            turn_id: TurnId::new(),
            channel: ch,
            audit,
            cancellation: &ch.token,
        }
    }

    #[test]
    fn parse_at_accepts_unix_and_rfc3339() {
        assert_eq!(parse_at(&json!(1_700_000_000)).unwrap(), 1_700_000_000);
        assert_eq!(parse_at(&json!("1700000000")).unwrap(), 1_700_000_000);
        let t = parse_at(&json!("2026-06-07T18:00:00Z")).unwrap();
        assert!(t > 1_700_000_000); // a real future-ish timestamp
        assert!(parse_at(&json!("not a time")).is_err());
        assert!(parse_at(&json!(true)).is_err());
    }

    fn scopes_correct() {
        assert_eq!(
            RemindSetTool::new().required_scope(&json!({})).base(),
            "remind.write"
        );
        assert_eq!(
            RemindListTool::new().required_scope(&json!({})).base(),
            "remind.read"
        );
        assert_eq!(
            RemindCancelTool::new().required_scope(&json!({})).base(),
            "remind.write"
        );
    }

    #[test]
    fn required_scopes_are_correct() {
        scopes_correct();
    }

    async fn store() -> SharedReminderStore {
        let dir = std::env::temp_dir()
            .join(format!("aivyx-remtool-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let s: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("s.redb")),
            MasterKey::from_raw([7u8; 32]),
        )
        .await
        .unwrap();
        Arc::new(ReminderStore::new(s.domain(KeyDomain::Reminders)))
    }

    #[tokio::test]
    async fn set_list_cancel_tools_round_trip() {
        let st = store().await;
        let set = RemindSetTool::new();
        set.set_store(Arc::clone(&st)).unwrap();
        let list = RemindListTool::new();
        list.set_store(Arc::clone(&st)).unwrap();
        let cancel = RemindCancelTool::new();
        cancel.set_store(Arc::clone(&st)).unwrap();
        let (ch, audit) = ctx_parts();
        let ctx = make_ctx(&ch, &audit);

        // set
        let out = set
            .execute(
                json!({ "at": 1_700_000_000_i64, "message": "call mom" }),
                &ctx,
            )
            .await;
        let id = match out {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["due_unix"], 1_700_000_000_i64);
                output["id"].as_str().unwrap().to_string()
            }
            other => panic!("expected Completed, got {other:?}"),
        };
        // list shows it
        match list.execute(json!({}), &ctx).await {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["reminders"].as_array().unwrap().len(), 1);
                assert_eq!(output["reminders"][0]["message"], "call mom");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
        // cancel it
        match cancel.execute(json!({ "id": id }), &ctx).await {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["canceled"], true);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
        // list empty
        match list.execute(json!({}), &ctx).await {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["reminders"].as_array().unwrap().len(), 0);
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn set_rejects_bad_input() {
        let st = store().await;
        let set = RemindSetTool::new();
        set.set_store(st).unwrap();
        let (ch, audit) = ctx_parts();
        let ctx = make_ctx(&ch, &audit);
        // missing message
        assert!(matches!(
            set.execute(json!({ "at": 100 }), &ctx).await,
            ToolOutcome::Failed(_)
        ));
        // bad time
        assert!(matches!(
            set.execute(json!({ "at": "soon", "message": "x" }), &ctx).await,
            ToolOutcome::Failed(_)
        ));
    }

    #[tokio::test]
    async fn tools_without_store_fail_cleanly() {
        let (ch, audit) = ctx_parts();
        let ctx = make_ctx(&ch, &audit);
        assert!(matches!(
            RemindListTool::new().execute(json!({}), &ctx).await,
            ToolOutcome::Failed(_)
        ));
    }
}

//! Phase 183 — durable one-shot reminder store.
//!
//! A reminder is a *"at `due_unix`, notify with `message`"* row.
//! Operator-set via `remind.set`, fired + cleared by the reminder
//! driver. Backed by [`aivyx_storage::KeyDomain::Reminders`], a
//! dedicated encrypted domain so a corrupt row degrades only
//! reminders — never schedules, missions, or any learning ledger.
//!
//! Not a tamper-evident HMAC chain like the loop backlog:
//! reminders are operator preferences, not security events, so a
//! simple id-keyed CRUD store is the right weight. Few rows in
//! practice, so `list` / `due` scan + sort in memory.

use serde::{Deserialize, Serialize};

use aivyx_storage::DomainHandle;

/// One pending reminder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reminder {
    /// Stable id (uuid). The storage key.
    pub id: String,
    /// When to fire, unix seconds.
    pub due_unix: i64,
    /// The message to deliver.
    pub message: String,
    /// Notify targets; empty → the operator's default targets
    /// (resolved by the driver).
    #[serde(default)]
    pub notify_targets: Vec<String>,
    /// When the reminder was set, unix seconds.
    pub created_unix: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum ReminderStoreError {
    #[error("reminder store storage error: {0}")]
    Storage(String),
    #[error("reminder store encode error: {0}")]
    Encode(String),
}

/// Pure selector — the reminders that are due at `now_unix`
/// (`due_unix <= now`), soonest first. Lives apart from the store
/// so the driver's decision is unit-testable without I/O.
pub fn due_now(reminders: &[Reminder], now_unix: i64) -> Vec<&Reminder> {
    let mut due: Vec<&Reminder> =
        reminders.iter().filter(|r| r.due_unix <= now_unix).collect();
    due.sort_by_key(|r| (r.due_unix, r.id.clone()));
    due
}

/// Durable reminder store over
/// [`aivyx_storage::KeyDomain::Reminders`]. Keyed by `id`.
pub struct ReminderStore {
    storage: DomainHandle,
}

impl ReminderStore {
    pub fn new(storage: DomainHandle) -> Self {
        Self { storage }
    }

    /// Insert (or replace) a reminder.
    pub async fn set(
        &self,
        reminder: &Reminder,
    ) -> Result<(), ReminderStoreError> {
        let value = serde_json::to_vec(reminder)
            .map_err(|e| ReminderStoreError::Encode(e.to_string()))?;
        self.storage
            .put(reminder.id.as_bytes(), &value)
            .await
            .map_err(|e| ReminderStoreError::Storage(e.to_string()))
    }

    /// All pending reminders, soonest first.
    pub async fn list(
        &self,
    ) -> Result<Vec<Reminder>, ReminderStoreError> {
        let rows = self
            .storage
            .scan_prefix(&[])
            .await
            .map_err(|e| ReminderStoreError::Storage(e.to_string()))?;
        let mut out = Vec::with_capacity(rows.len());
        for (_k, v) in &rows {
            let r: Reminder = serde_json::from_slice(v)
                .map_err(|e| ReminderStoreError::Encode(e.to_string()))?;
            out.push(r);
        }
        out.sort_by_key(|r| (r.due_unix, r.id.clone()));
        Ok(out)
    }

    /// Cancel by id. `Ok(true)` if a reminder was removed,
    /// `Ok(false)` if none had that id.
    pub async fn cancel(
        &self,
        id: &str,
    ) -> Result<bool, ReminderStoreError> {
        let existed = self
            .storage
            .get(id.as_bytes())
            .await
            .map_err(|e| ReminderStoreError::Storage(e.to_string()))?
            .is_some();
        if existed {
            self.storage
                .delete(id.as_bytes())
                .await
                .map_err(|e| ReminderStoreError::Storage(e.to_string()))?;
        }
        Ok(existed)
    }

    /// The reminders due at `now_unix`, soonest first.
    pub async fn due(
        &self,
        now_unix: i64,
    ) -> Result<Vec<Reminder>, ReminderStoreError> {
        Ok(self
            .list()
            .await?
            .into_iter()
            .filter(|r| r.due_unix <= now_unix)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{
        KeyDomain, RedbStorage, Storage, StorageConfig,
    };
    use std::sync::Arc;

    fn reminder(id: &str, due: i64, msg: &str) -> Reminder {
        Reminder {
            id: id.into(),
            due_unix: due,
            message: msg.into(),
            notify_targets: vec![],
            created_unix: 0,
        }
    }

    #[test]
    fn due_now_selects_past_and_exact_not_future() {
        let rs = vec![
            reminder("a", 100, "past"),
            reminder("b", 200, "exact"),
            reminder("c", 300, "future"),
        ];
        let due = due_now(&rs, 200);
        let ids: Vec<&str> = due.iter().map(|r| r.id.as_str()).collect();
        // 100 (past) + 200 (exact) fire; 300 (future) doesn't.
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn due_now_is_soonest_first() {
        let rs = vec![
            reminder("z", 50, ""),
            reminder("a", 10, ""),
            reminder("m", 30, ""),
        ];
        let due = due_now(&rs, 100);
        let order: Vec<i64> = due.iter().map(|r| r.due_unix).collect();
        assert_eq!(order, vec![10, 30, 50]);
    }

    async fn open_store() -> ReminderStore {
        let dir = std::env::temp_dir()
            .join(format!("aivyx-reminders-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([183u8; 32]),
        )
        .await
        .unwrap();
        ReminderStore::new(store.domain(KeyDomain::Reminders))
    }

    #[tokio::test]
    async fn set_list_cancel_round_trip() {
        let store = open_store().await;
        assert!(store.list().await.unwrap().is_empty());
        store.set(&reminder("r1", 300, "call mom")).await.unwrap();
        store.set(&reminder("r2", 100, "standup")).await.unwrap();
        // list is soonest-first.
        let listed = store.list().await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "r2"); // due 100 first
        assert_eq!(listed[1].message, "call mom");
        // cancel a present + an absent id.
        assert!(store.cancel("r2").await.unwrap());
        assert!(!store.cancel("nope").await.unwrap());
        let after = store.list().await.unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].id, "r1");
    }

    #[tokio::test]
    async fn due_returns_only_past_or_now() {
        let store = open_store().await;
        store.set(&reminder("past", 100, "")).await.unwrap();
        store.set(&reminder("future", 9_999, "")).await.unwrap();
        let due = store.due(500).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "past");
    }

    #[tokio::test]
    async fn set_replaces_same_id() {
        let store = open_store().await;
        store.set(&reminder("r", 100, "first")).await.unwrap();
        store.set(&reminder("r", 200, "second")).await.unwrap();
        let listed = store.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].message, "second");
        assert_eq!(listed[0].due_unix, 200);
    }
}

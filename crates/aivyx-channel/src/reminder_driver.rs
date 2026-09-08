//! Phase 183 — the reminder driver.
//!
//! A re-arming background task (sibling of `reflection_scheduler`
//! / `loop_driver`): each tick, find reminders whose `due_unix <=
//! now`, **push** them through the notify dispatcher, and clear
//! them from the store. This is the half a separate tool process
//! can't do — pushing a message *at a time* needs the daemon's
//! cadence + notify, which is exactly what lives here.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::notify_dispatcher::NotifyDispatcher;
use crate::reminder_store::ReminderStore;

/// Default tick when `[reminders].check_interval_secs` is absent.
/// 30s keeps reminders punctual without busy-polling.
pub const DEFAULT_CHECK_INTERVAL_SECS: u64 = 30;

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Delivery seam so the driver's fire logic is testable without a
/// live notify backend.
#[async_trait]
pub trait ReminderNotifier: Send + Sync {
    /// Deliver `message` to `target`. Best-effort — an error is
    /// logged by the driver, never fatal.
    async fn notify(&self, target: &str, message: &str)
        -> Result<(), String>;
    /// Targets to use when a reminder named none.
    fn default_targets(&self) -> Vec<String>;
}

/// Production adapter over the [`NotifyDispatcher`].
pub struct DispatcherNotifier {
    dispatcher: Arc<NotifyDispatcher>,
    defaults: Vec<String>,
}

impl DispatcherNotifier {
    pub fn new(
        dispatcher: Arc<NotifyDispatcher>,
        defaults: Vec<String>,
    ) -> Self {
        Self {
            dispatcher,
            defaults,
        }
    }
}

#[async_trait]
impl ReminderNotifier for DispatcherNotifier {
    async fn notify(
        &self,
        target: &str,
        message: &str,
    ) -> Result<(), String> {
        self.dispatcher
            .dispatch(target, message, Some("Reminder"))
            .await
            .map_err(|e| e.to_string())
    }
    fn default_targets(&self) -> Vec<String> {
        self.defaults.clone()
    }
}

/// Fire every reminder due at `now`: deliver to its targets (or
/// the notifier's defaults) and remove it from the store. Returns
/// how many were fired. Pure of timing so a test can drive it
/// directly.
pub async fn fire_due(
    store: &ReminderStore,
    notifier: &dyn ReminderNotifier,
    now: i64,
) -> usize {
    let due = store.due(now).await.unwrap_or_default();
    let defaults = notifier.default_targets();
    for r in &due {
        let targets: Vec<String> = if r.notify_targets.is_empty() {
            defaults.clone()
        } else {
            r.notify_targets.clone()
        };
        for t in &targets {
            if let Err(e) = notifier.notify(t, &r.message).await {
                eprintln!(
                    "aivyx-pa reminder: deliver to {t:?} failed: {e}"
                );
            }
        }
        // Clear the fired reminder (best-effort; a failed cancel
        // means it re-fires next tick — at-least-once, never lost).
        if let Err(e) = store.cancel(&r.id).await {
            eprintln!("aivyx-pa reminder: clear {:?} failed: {e}", r.id);
        }
    }
    due.len()
}

/// The re-arming driver loop. Runs until the task is dropped (a
/// daemon shutdown). On each tick fires due reminders.
pub async fn run_reminder_driver(
    store: Arc<ReminderStore>,
    notifier: Arc<dyn ReminderNotifier>,
    interval: Duration,
) {
    loop {
        tokio::time::sleep(interval).await;
        let fired = fire_due(&store, notifier.as_ref(), now_unix()).await;
        if fired > 0 {
            eprintln!("aivyx-pa reminder: fired {fired} reminder(s)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reminder_store::Reminder;
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{
        KeyDomain, RedbStorage, Storage, StorageConfig,
    };
    use std::sync::Mutex;

    struct FakeNotifier {
        sent: Mutex<Vec<(String, String)>>,
        defaults: Vec<String>,
    }
    #[async_trait]
    impl ReminderNotifier for FakeNotifier {
        async fn notify(
            &self,
            target: &str,
            message: &str,
        ) -> Result<(), String> {
            self.sent
                .lock()
                .unwrap()
                .push((target.to_string(), message.to_string()));
            Ok(())
        }
        fn default_targets(&self) -> Vec<String> {
            self.defaults.clone()
        }
    }

    async fn store() -> ReminderStore {
        let dir = std::env::temp_dir()
            .join(format!("aivyx-remdrv-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let s: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("s.redb")),
            MasterKey::from_raw([9u8; 32]),
        )
        .await
        .unwrap();
        ReminderStore::new(s.domain(KeyDomain::Reminders))
    }

    fn reminder(id: &str, due: i64, msg: &str, targets: &[&str]) -> Reminder {
        Reminder {
            id: id.into(),
            due_unix: due,
            message: msg.into(),
            notify_targets: targets.iter().map(|t| t.to_string()).collect(),
            created_unix: 0,
        }
    }

    #[tokio::test]
    async fn fires_due_delivers_and_clears_leaves_future() {
        let store = store().await;
        store
            .set(&reminder("past", 100, "standup", &["webui"]))
            .await
            .unwrap();
        store
            .set(&reminder("future", 9_999, "later", &["webui"]))
            .await
            .unwrap();
        let notifier = FakeNotifier {
            sent: Mutex::new(vec![]),
            defaults: vec!["email".into()],
        };
        let n = fire_due(&store, &notifier, 500).await;
        assert_eq!(n, 1);
        // Delivered the past one to its explicit target.
        let sent = notifier.sent.lock().unwrap().clone();
        assert_eq!(sent, vec![("webui".to_string(), "standup".to_string())]);
        // Past cleared, future remains.
        let left = store.list().await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, "future");
    }

    #[tokio::test]
    async fn no_targets_uses_defaults() {
        let store = store().await;
        store.set(&reminder("r", 100, "hi", &[])).await.unwrap();
        let notifier = FakeNotifier {
            sent: Mutex::new(vec![]),
            defaults: vec!["email".into(), "telegram".into()],
        };
        fire_due(&store, &notifier, 500).await;
        let sent = notifier.sent.lock().unwrap().clone();
        // Delivered to BOTH default targets.
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().any(|(t, _)| t == "email"));
        assert!(sent.iter().any(|(t, _)| t == "telegram"));
    }

    #[tokio::test]
    async fn nothing_due_fires_nothing() {
        let store = store().await;
        store.set(&reminder("future", 9_999, "x", &["a"])).await.unwrap();
        let notifier = FakeNotifier {
            sent: Mutex::new(vec![]),
            defaults: vec![],
        };
        assert_eq!(fire_due(&store, &notifier, 100).await, 0);
        assert!(notifier.sent.lock().unwrap().is_empty());
        assert_eq!(store.list().await.unwrap().len(), 1);
    }
}

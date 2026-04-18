//! Unified trigger dispatch — Phase 27 Task 2.
//!
//! All daemon trigger types (cron schedules, webhooks, file watchers)
//! converge on the same turn-dispatch path: construct a `Message`,
//! acquire the turn lock, run `agent.turn()`, and log the outcome.
//! This module owns the shared dispatch logic so that each trigger
//! source only needs to decide *when* to fire and *what* prompt to
//! send.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use aivyx_core::{Agent, Message, SessionId, TurnOutcome};

use aivyx_storage::DomainHandle;

use crate::daemon_ipc::FrontendType;
use crate::daemon_server::ChannelFactory;
use crate::mission;

// ---------------------------------------------------------------------------
// Trigger source tag — carried through dispatch for logging / audit.
// ---------------------------------------------------------------------------

/// Identifies the origin of a triggered turn. Carried through the
/// dispatch path for logging; will also inform audit attribution in
/// future phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerSource {
    Cron,
    Webhook,
    FileWatch,
}

impl std::fmt::Display for TriggerSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TriggerSource::Cron => write!(f, "cron"),
            TriggerSource::Webhook => write!(f, "webhook"),
            TriggerSource::FileWatch => write!(f, "file-watch"),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared turn-dispatch context.
// ---------------------------------------------------------------------------

/// Shared state for trigger dispatch. Created once at daemon startup
/// and cloned into each trigger subsystem (scheduler, webhook listener,
/// file watcher).
#[derive(Clone)]
pub struct TriggerDispatch {
    agent: Arc<dyn Agent>,
    channel_factory: ChannelFactory,
    /// Serializes triggered turns so concurrent fires don't interleave.
    turn_lock: Arc<Mutex<()>>,
    /// Optional mission store for automatic mission wrapping.
    mission_store: Option<DomainHandle>,
}

impl TriggerDispatch {
    pub fn new(agent: Arc<dyn Agent>, channel_factory: ChannelFactory) -> Self {
        Self {
            agent,
            channel_factory,
            turn_lock: Arc::new(Mutex::new(())),
            mission_store: None,
        }
    }

    /// Attach a mission store so that triggers with `wrap_mission = true`
    /// can create missions automatically.
    pub fn with_mission_store(mut self, store: DomainHandle) -> Self {
        self.mission_store = Some(store);
        self
    }

    /// Fire a single triggered turn through the agent.
    ///
    /// Acquires the turn lock, constructs a `Message` from the prompt,
    /// dispatches through `agent.turn()`, and returns the wall-clock
    /// duration of the turn (for logging / metrics).
    ///
    /// When `wrap_mission` is true and a mission store is configured,
    /// a `MissionRecord` is created before the turn (state Created →
    /// Running) and completed or failed after the turn finishes.
    pub async fn fire(
        &self,
        source: TriggerSource,
        trigger_id: &str,
        prompt: &str,
        wrap_mission: bool,
    ) -> Duration {
        eprintln!(
            "aivyx trigger: firing {source} {trigger_id:?} (prompt={prompt:?}, mission={wrap_mission})",
        );

        let channel = (self.channel_factory)(FrontendType::Local);
        let msg = Message::text(SessionId::new(), prompt.to_owned());

        // Create mission if requested.
        let mission_id = if wrap_mission {
            if let Some(store) = &self.mission_store {
                let mid = format!("trg-{}", uuid::Uuid::new_v4().as_simple());
                let description = format!(
                    "{source} trigger {trigger_id}: {prompt}",
                );
                let mut record = mission::MissionRecord::new(
                    mid.clone(),
                    "default".into(),
                    description,
                );
                if let Err(e) = mission::create_mission(store, &record).await {
                    eprintln!("aivyx trigger: failed to create mission for {trigger_id}: {e}");
                    None
                } else {
                    // Transition to Running immediately.
                    let _ = mission::transition_to_running(&mut record);
                    if let Err(e) = mission::update_mission(store, &record).await {
                        eprintln!("aivyx trigger: failed to start mission {mid}: {e}");
                    }
                    Some(mid)
                }
            } else {
                eprintln!(
                    "aivyx trigger: wrap_mission requested for {trigger_id} but no mission store configured"
                );
                None
            }
        } else {
            None
        };

        let start = std::time::Instant::now();
        let _guard = self.turn_lock.lock().await;
        let outcome = self.agent.turn(msg, channel.as_ref()).await;
        drop(_guard);
        let elapsed = start.elapsed();

        eprintln!(
            "aivyx trigger: {source} {trigger_id:?} turn outcome: {outcome:?} ({elapsed:.1?})",
        );

        // Complete or fail the mission based on turn outcome.
        if let (Some(mid), Some(store)) = (&mission_id, &self.mission_store) {
            let result = async {
                let mut record = mission::get_mission(store, mid)
                    .await
                    .map_err(|e| format!("get mission: {e}"))?
                    .ok_or_else(|| format!("mission {mid} not found"))?;

                match &outcome {
                    TurnOutcome::Completed { .. } => {
                        mission::complete_mission(&mut record)
                            .map_err(|e| format!("complete mission: {e}"))?;
                    }
                    TurnOutcome::Failed(_) | TurnOutcome::Cancelled { .. } | TurnOutcome::TimedOut { .. } => {
                        // Cancel rather than fail — the mission itself didn't
                        // hit a gate rejection, the turn just didn't succeed.
                        mission::cancel_mission(&mut record)
                            .map_err(|e| format!("cancel mission: {e}"))?;
                    }
                    TurnOutcome::Escalated { .. } => {
                        // Leave in Running state — escalation means a gate
                        // will be created by the normal gate path.
                    }
                }

                mission::update_mission(store, &record)
                    .await
                    .map_err(|e| format!("update mission: {e}"))?;
                Ok::<(), String>(())
            }
            .await;

            if let Err(e) = result {
                eprintln!("aivyx trigger: mission lifecycle error for {mid}: {e}");
            }
        }

        elapsed
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_source_display() {
        assert_eq!(TriggerSource::Cron.to_string(), "cron");
        assert_eq!(TriggerSource::Webhook.to_string(), "webhook");
        assert_eq!(TriggerSource::FileWatch.to_string(), "file-watch");
    }

    #[test]
    fn trigger_source_eq() {
        assert_eq!(TriggerSource::Cron, TriggerSource::Cron);
        assert_ne!(TriggerSource::Cron, TriggerSource::Webhook);
        assert_ne!(TriggerSource::Webhook, TriggerSource::FileWatch);
    }
}

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

use aivyx_core::{Agent, Message, SessionId};

use crate::daemon_ipc::FrontendType;
use crate::daemon_server::ChannelFactory;

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
}

impl TriggerDispatch {
    pub fn new(agent: Arc<dyn Agent>, channel_factory: ChannelFactory) -> Self {
        Self {
            agent,
            channel_factory,
            turn_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Fire a single triggered turn through the agent.
    ///
    /// Acquires the turn lock, constructs a `Message` from the prompt,
    /// dispatches through `agent.turn()`, and returns the wall-clock
    /// duration of the turn (for logging / metrics).
    pub async fn fire(&self, source: TriggerSource, trigger_id: &str, prompt: &str) -> Duration {
        eprintln!(
            "aivyx trigger: firing {source} {trigger_id:?} (prompt={prompt:?})",
        );

        let channel = (self.channel_factory)(FrontendType::Local);
        let msg = Message::text(SessionId::new(), prompt.to_owned());

        let start = std::time::Instant::now();
        let _guard = self.turn_lock.lock().await;
        let outcome = self.agent.turn(msg, channel.as_ref()).await;
        drop(_guard);
        let elapsed = start.elapsed();

        eprintln!(
            "aivyx trigger: {source} {trigger_id:?} turn outcome: {outcome:?} ({elapsed:.1?})",
        );

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

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
use crate::notify_dispatcher::NotifyDispatcher;

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
    /// Optional notify dispatcher for Phase 63 auto-notify sugar.
    /// When set, a trigger with `notify_target = Some(name)` fires
    /// the named target's backend after the turn completes.
    notify_dispatcher: Option<Arc<NotifyDispatcher>>,
}

impl TriggerDispatch {
    pub fn new(agent: Arc<dyn Agent>, channel_factory: ChannelFactory) -> Self {
        Self {
            agent,
            channel_factory,
            turn_lock: Arc::new(Mutex::new(())),
            mission_store: None,
            notify_dispatcher: None,
        }
    }

    /// Attach a mission store so that triggers with `wrap_mission = true`
    /// can create missions automatically.
    pub fn with_mission_store(mut self, store: DomainHandle) -> Self {
        self.mission_store = Some(store);
        self
    }

    /// Phase 63 Task 3 — attach a `NotifyDispatcher` so triggers
    /// with `notify_target = Some(name)` can auto-dispatch their
    /// turn's final response after completion.
    pub fn with_notify_dispatcher(mut self, dispatcher: Arc<NotifyDispatcher>) -> Self {
        self.notify_dispatcher = Some(dispatcher);
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
    ///
    /// Phase 63 Task 3: when `notify_target` is `Some` and a notify
    /// dispatcher is configured, after the turn completes the
    /// agent's final response is auto-pushed to the named target.
    /// Q2(a): empty agent response skips the dispatch.
    /// Q3(a): failed turns dispatch a synthesized body.
    /// Q4(a): subject is `<kind>: <trigger-id>`.
    pub async fn fire(
        &self,
        source: TriggerSource,
        trigger_id: &str,
        prompt: &str,
        wrap_mission: bool,
        notify_target: Option<&str>,
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
                    TurnOutcome::Escalated { reason, .. } => {
                        // Phase 35: create a gate on the mission so the
                        // operator can approve/reject and resume the turn.
                        let gate_id = format!(
                            "gate-{}",
                            uuid::Uuid::new_v4().as_hyphenated()
                        );
                        mission::add_gate(
                            &mut record,
                            gate_id.clone(),
                            reason.clone(),
                            None,
                        )
                        .map_err(|e| format!("add gate: {e}"))?;
                        eprintln!(
                            "aivyx trigger: escalation gate {gate_id} created on mission {mid}",
                        );
                        // Mission stays in GatePending (set by add_gate).
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

        // ---- Phase 63 Task 3: auto-notify ------------------------
        // If the trigger declared a notify_target AND a dispatcher
        // is configured, push the turn's outcome to the named
        // target. Failure surfaces as eprintln; one attempt, no
        // retry (Phase 63 sign-off). Audit-chain integration is
        // deferred — see Phase 63 deferrals.
        if let (Some(target), Some(dispatcher)) = (notify_target, &self.notify_dispatcher) {
            let body = render_notify_body(&outcome);
            let subject = format!("{source}: {trigger_id}");
            if body.is_empty() {
                // Q2(a) — skip empty responses.
                eprintln!(
                    "aivyx trigger: auto-notify skipped (empty response) for \
                     {source} {trigger_id:?} → target `{target}`",
                );
            } else {
                match dispatcher.dispatch(target, &body, Some(&subject)).await {
                    Ok(()) => {
                        eprintln!(
                            "aivyx trigger: auto-notify dispatched for \
                             {source} {trigger_id:?} → target `{target}`",
                        );
                    }
                    Err(e) => {
                        eprintln!(
                            "aivyx trigger: auto-notify FAILED for \
                             {source} {trigger_id:?} → target `{target}`: {e}",
                        );
                    }
                }
            }
        }

        elapsed
    }
}

/// Render the body of an auto-notify message from a `TurnOutcome`.
/// Public for testing.
///
/// - `Completed` → the agent's `final_message` text.
/// - `Escalated` → "Turn escalated: <reason>" so the operator
///   sees the agent needed approval.
/// - `Failed` → "Turn failed: <error>" per Q3(a) at sign-off.
/// - `TimedOut` → "Turn timed out after <duration>".
/// - `Cancelled` → "Turn cancelled".
///
/// Empty string → caller should skip the dispatch (Q2(a)).
pub fn render_notify_body(outcome: &TurnOutcome) -> String {
    match outcome {
        TurnOutcome::Completed { final_message, .. } => final_message.clone(),
        TurnOutcome::Escalated { reason, .. } => {
            format!("Turn escalated: {reason}")
        }
        TurnOutcome::Failed(e) => format!("Turn failed: {e}"),
        TurnOutcome::TimedOut { elapsed, .. } => {
            format!("Turn timed out after {elapsed:.1?}")
        }
        TurnOutcome::Cancelled { .. } => "Turn cancelled".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_core::{AivyxError, ToolId};
    use std::time::Duration;

    #[test]
    fn render_completed_returns_final_message_verbatim() {
        let outcome = TurnOutcome::Completed {
            final_message: "Daily summary: 3 commits, 2 PRs reviewed.".into(),
            tool_calls_made: 0,
            duration: Duration::from_secs(2),
        };
        assert_eq!(
            render_notify_body(&outcome),
            "Daily summary: 3 commits, 2 PRs reviewed."
        );
    }

    #[test]
    fn render_completed_empty_message_returns_empty_string() {
        // Caller (TriggerDispatch::fire) checks for empty body
        // and skips the dispatch per Q2(a).
        let outcome = TurnOutcome::Completed {
            final_message: String::new(),
            tool_calls_made: 0,
            duration: Duration::from_secs(1),
        };
        assert!(render_notify_body(&outcome).is_empty());
    }

    #[test]
    fn render_failed_returns_turn_failed_prefix() {
        let outcome = TurnOutcome::Failed(AivyxError::Channel(
            "provider unreachable".into(),
        ));
        let body = render_notify_body(&outcome);
        assert!(body.starts_with("Turn failed:"), "body: {body}");
        assert!(body.contains("provider unreachable"), "body: {body}");
    }

    #[test]
    fn render_escalated_returns_escalation_summary() {
        let outcome = TurnOutcome::Escalated {
            reason: "destructive shell command refused".into(),
            pending_tool: ToolId::new(),
            tool_calls_made: 1,
        };
        let body = render_notify_body(&outcome);
        assert!(body.starts_with("Turn escalated:"), "body: {body}");
        assert!(body.contains("destructive"), "body: {body}");
    }

    #[test]
    fn render_timed_out_includes_elapsed() {
        let outcome = TurnOutcome::TimedOut {
            tool_calls_made: 5,
            elapsed: Duration::from_secs(120),
        };
        let body = render_notify_body(&outcome);
        assert!(body.starts_with("Turn timed out"), "body: {body}");
    }

    #[test]
    fn render_cancelled_returns_cancelled_marker() {
        let outcome = TurnOutcome::Cancelled { tool_calls_made: 2 };
        assert_eq!(render_notify_body(&outcome), "Turn cancelled");
    }

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

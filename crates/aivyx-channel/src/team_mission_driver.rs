//! Chapter L (L.4) — the daemon-side driver for Nonagon team missions.
//!
//! [`crate::team_mission`] is the durable *store*; this module is the live
//! *engine driver* over it. It owns [`SharedMissionState`] — the in-memory
//! registry (active + recent missions) backed by `KeyDomain::TeamMissions`,
//! mirroring `SharedLoopState` — and the two daemon operations the IPC layer
//! (L.5) exposes:
//!
//! - [`team_run`] assembles the team over the daemon's **real tool list**
//!   ([`TeamAssembly::build`], NT-02 attenuation intact) and drives the plan
//!   through [`TeamRuntime::run_until_pause`] with an observer that updates the
//!   snapshot and persists every checkpoint, on the one HMAC chain.
//! - [`resolve_team_gate`] resumes (approve) or aborts (reject) a mission
//!   paused at a human-approval gate.
//!
//! Per the L.4 entry decision, `team_run` takes an **already-built**
//! [`MissionPlan`]; turning a free-text goal into a plan via the lead LLM is a
//! later increment. The engine stays pure — the checkpoint/resume durability
//! lives entirely here, at the daemon boundary. See `docs/DAEMON_TEAMS.md` §5.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use aivyx_capability::TrustTier;
use aivyx_core::{
    AuditHook, CancellationToken, ChannelContext, ChannelError, ChannelPlatform, SessionId,
    StreamEvent, Tool, TurnOutcome,
};
use aivyx_llm::LlmProvider;
use aivyx_storage::StorageError;
use aivyx_team::{
    MissionObserver, MissionPlan, MissionStatus, RunYield, TeamAssembly, TeamConfig, TeamError,
    TeamRuntime,
};
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::team_mission::{
    list_team_missions, save_team_mission, TeamMissionPhase, TeamMissionRecord,
};
use aivyx_storage::DomainHandle;

/// Everything wrong a mission run can hit at the daemon boundary.
#[derive(Debug, thiserror::Error)]
pub enum MissionDriverError {
    /// A storage read/write failed.
    #[error("mission store error: {0}")]
    Store(#[from] StorageError),
    /// Assembling or running the team failed.
    #[error("team error: {0}")]
    Team(#[from] TeamError),
    /// The named mission isn't in the registry.
    #[error("no such mission: {0}")]
    NotFound(String),
    /// `resolve_team_gate` was called on a mission that isn't awaiting one.
    #[error("mission {0} is not awaiting a gate decision (phase {1:?})")]
    NotAwaiting(String, TeamMissionPhase),
    /// `resolve_team_gate` named a step that isn't the pending gate.
    #[error("mission {0} is awaiting gate {1:?}, not {2:?}")]
    WrongGate(String, String, String),
    /// The mission-drive task panicked.
    #[error("mission run task failed: {0}")]
    Join(String),
}

/// The daemon's shared deps for assembling a team — the same live provider,
/// model, audit chain, and tool set every other daemon turn runs on.
#[derive(Clone)]
pub struct TeamRunDeps {
    pub provider: Arc<dyn LlmProvider>,
    pub model: String,
    pub max_tokens: u32,
    /// The persistent HMAC audit chain — specialist sub-turns append to it.
    pub audit: Arc<dyn AuditHook>,
    /// The daemon's full tool set; each specialist gets exactly the subset its
    /// `tool_allowlist` names, capability-attenuated against the lead (NT-02).
    pub base_tools: Vec<Arc<dyn Tool>>,
}

/// In-memory registry of daemon-run team missions, backed by the encrypted
/// `KeyDomain::TeamMissions` store. Cloned into every IPC handler and the
/// drive task (mirrors `SharedLoopState`).
#[derive(Clone)]
pub struct SharedMissionState {
    store: DomainHandle,
    registry: Arc<RwLock<BTreeMap<String, TeamMissionRecord>>>,
}

impl SharedMissionState {
    /// A registry over `store` (a `KeyDomain::TeamMissions` handle). Empty
    /// until [`reload`](Self::reload) hydrates it from the store.
    pub fn new(store: DomainHandle) -> Self {
        SharedMissionState {
            store,
            registry: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    /// Reload-on-startup: hydrate the in-memory registry from the store.
    /// Returns the number of missions loaded (doc §4). `AwaitingApproval`
    /// missions are resumable; `Executing` ones interrupted by a crash are
    /// re-drivable from their last checkpoint.
    pub async fn reload(&self) -> Result<usize, StorageError> {
        let records = list_team_missions(&self.store).await?;
        let mut reg = self.registry.write().expect("mission registry lock");
        reg.clear();
        for r in &records {
            reg.insert(r.id.clone(), r.clone());
        }
        Ok(records.len())
    }

    /// One mission's current snapshot, or `None` if unknown.
    pub fn snapshot(&self, id: &str) -> Option<TeamMissionRecord> {
        self.registry
            .read()
            .expect("mission registry lock")
            .get(id)
            .cloned()
    }

    /// Every known mission, id-ordered (the poll-feed primitive).
    pub fn list(&self) -> Vec<TeamMissionRecord> {
        self.registry
            .read()
            .expect("mission registry lock")
            .values()
            .cloned()
            .collect()
    }

    /// Persist a record to the store **and** the in-memory registry — the
    /// single transition primitive, called on every state change. Stamps
    /// `updated_at`.
    pub async fn put(&self, mut record: TeamMissionRecord) -> Result<(), StorageError> {
        record.touch();
        save_team_mission(&self.store, &record).await?;
        self.registry
            .write()
            .expect("mission registry lock")
            .insert(record.id.clone(), record);
        Ok(())
    }

    /// Apply `f` to the in-memory record (sync — for the observer's live
    /// step-progress updates). Persistence is the driver's job at the next
    /// checkpoint ping.
    fn touch_in_memory(&self, id: &str, f: impl FnOnce(&mut TeamMissionRecord)) {
        let mut reg = self.registry.write().expect("mission registry lock");
        if let Some(r) = reg.get_mut(id) {
            f(r);
            r.touch();
        }
    }

    /// Persist the registry's current in-memory record for `id` to the store
    /// (the checkpoint write behind each step-completion ping).
    async fn persist_current(&self, id: &str) -> Result<(), StorageError> {
        if let Some(record) = self.snapshot(id) {
            save_team_mission(&self.store, &record).await?;
        }
        Ok(())
    }
}

/// Start a daemon-run team mission from an already-built plan: register it,
/// assemble the team over the real tool list, and drive it to its first pause
/// or terminal state. Returns the mission id.
///
/// The drive blocks until the mission reaches a human gate (`AwaitingApproval`)
/// or finishes (`Done`/`Rejected`); the daemon IPC handler wraps this in a
/// `tokio::spawn` (L.5) so the call site returns the id immediately.
pub async fn team_run(
    shared: &SharedMissionState,
    deps: &TeamRunDeps,
    config: TeamConfig,
    plan: MissionPlan,
    id: impl Into<String>,
) -> Result<String, MissionDriverError> {
    let id = register_mission(shared, plan, id).await?;
    drive_registered(shared, deps, config, &id).await?;
    Ok(id)
}

/// Validate and persist a fresh mission in the `Planning` phase, returning its
/// id. The daemon registers synchronously (so a `TeamMissionStatus` poll sees
/// it immediately) then spawns [`drive_registered`].
pub async fn register_mission(
    shared: &SharedMissionState,
    plan: MissionPlan,
    id: impl Into<String>,
) -> Result<String, MissionDriverError> {
    let id = id.into();
    // Validate before we register anything the operator would have to clean up.
    plan.validate()?;
    let goal = plan.goal.clone();
    shared.put(TeamMissionRecord::new(&id, goal, plan)).await?;
    Ok(id)
}

/// Assemble the team and drive an **already-registered** mission from its
/// checkpoint to the next pause / terminal state. The long-running half of a
/// mission run; the daemon `tokio::spawn`s it.
pub async fn drive_registered(
    shared: &SharedMissionState,
    deps: &TeamRunDeps,
    config: TeamConfig,
    id: &str,
) -> Result<TeamMissionPhase, MissionDriverError> {
    let runtime = assemble_runtime(deps, config)?;
    drive(shared, runtime, id).await
}

/// Resume (`approve`) or abort (`!approve`) a mission paused at a human gate.
/// On approval the gate is recorded as passed and the runtime is re-driven
/// from the checkpoint; on rejection the gate's dependents never run and the
/// mission ends `Rejected` with its partial outputs preserved.
pub async fn resolve_team_gate(
    shared: &SharedMissionState,
    deps: &TeamRunDeps,
    config: TeamConfig,
    id: &str,
    step: &str,
    approve: bool,
) -> Result<TeamMissionPhase, MissionDriverError> {
    match prepare_gate_resolution(shared, id, step, approve).await? {
        // Approve flipped the mission to `Executing` — drive the resume.
        TeamMissionPhase::Executing => drive_registered(shared, deps, config, id).await,
        // Reject is terminal; nothing left to drive.
        terminal => Ok(terminal),
    }
}

/// The **synchronous** half of a gate decision: validate the mission is paused
/// at `step`, then apply the immediate state change — `Rejected` (reject) or
/// `Executing` with the gate recorded passed (approve). Returns the new phase;
/// the caller drives the resume when it's `Executing`. Splitting this out lets
/// the daemon flip the persisted state before it spawns the (long) drive.
pub async fn prepare_gate_resolution(
    shared: &SharedMissionState,
    id: &str,
    step: &str,
    approve: bool,
) -> Result<TeamMissionPhase, MissionDriverError> {
    let mut record = shared
        .snapshot(id)
        .ok_or_else(|| MissionDriverError::NotFound(id.to_string()))?;
    if record.phase != TeamMissionPhase::AwaitingApproval {
        return Err(MissionDriverError::NotAwaiting(id.to_string(), record.phase));
    }
    match &record.pending_gate {
        Some(pending) if pending == step => {}
        Some(pending) => {
            return Err(MissionDriverError::WrongGate(
                id.to_string(),
                pending.clone(),
                step.to_string(),
            ))
        }
        None => return Err(MissionDriverError::NotAwaiting(id.to_string(), record.phase)),
    }

    record.pending_gate = None;
    if approve {
        // Record the gate as passed in the checkpoint, ready to re-drive.
        record
            .outputs
            .insert(step.to_string(), "PASS (approved by operator)".to_string());
        record.phase = TeamMissionPhase::Executing;
    } else {
        // Reject: the gate's dependents never run; preserve the checkpoint.
        record
            .outputs
            .insert(step.to_string(), "rejected by operator".to_string());
        record.phase = TeamMissionPhase::Rejected;
    }
    let phase = record.phase;
    shared.put(record).await?;
    Ok(phase)
}

/// The daemon's team-mission surface: the [`SharedMissionState`] registry, the
/// shared [`TeamRunDeps`], and the team [`TeamConfig`] to assemble. Cloned into
/// every IPC handler (it's the one handle the `TeamRun` / `TeamMissionList` /
/// `TeamMissionStatus` / `ResolveTeamGate` arms touch). `start` / `resolve`
/// spawn the long drive and return immediately, so the daemon never blocks a
/// connection on a running mission.
#[derive(Clone)]
pub struct TeamMissionService {
    state: SharedMissionState,
    deps: TeamRunDeps,
    config: TeamConfig,
}

impl TeamMissionService {
    /// A service over `state`, assembling `config` with `deps` per run. (For
    /// L.5 the daemon passes the default Nonagon; vertical-pack configs are a
    /// later increment.)
    pub fn new(state: SharedMissionState, deps: TeamRunDeps, config: TeamConfig) -> Self {
        TeamMissionService { state, deps, config }
    }

    /// The underlying registry — the read path (`reload`, `snapshot`, `list`).
    pub fn state(&self) -> &SharedMissionState {
        &self.state
    }

    /// One mission's snapshot.
    pub fn snapshot(&self, id: &str) -> Option<TeamMissionRecord> {
        self.state.snapshot(id)
    }

    /// Every known mission (the poll feed).
    pub fn list(&self) -> Vec<TeamMissionRecord> {
        self.state.list()
    }

    /// Register a mission from an explicit plan and **spawn** its drive,
    /// returning the new id immediately. The drive runs to the first human
    /// gate or terminal state in the background.
    pub async fn start(&self, plan: MissionPlan) -> Result<String, MissionDriverError> {
        let id = register_mission(&self.state, plan, uuid::Uuid::new_v4().to_string()).await?;
        self.spawn_drive(id.clone());
        Ok(id)
    }

    /// Resolve a paused gate, spawning the resume drive on approval. Returns
    /// the immediate phase (`Executing` on approve, `Rejected` on reject).
    pub async fn resolve(
        &self,
        id: &str,
        step: &str,
        approve: bool,
    ) -> Result<TeamMissionPhase, MissionDriverError> {
        let phase = prepare_gate_resolution(&self.state, id, step, approve).await?;
        if phase == TeamMissionPhase::Executing {
            self.spawn_drive(id.to_string());
        }
        Ok(phase)
    }

    /// Spawn the background drive for an already-registered/-resumed mission.
    /// A drive failure leaves the record `Executing` for the operator to
    /// inspect; we log rather than unwind the detached task.
    fn spawn_drive(&self, id: String) {
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) =
                drive_registered(&this.state, &this.deps, this.config.clone(), &id).await
            {
                eprintln!("aivyx team: mission {id} drive failed — {e}");
            }
        });
    }
}

/// Build the resumable runtime for a team. The plan-driven daemon path needs
/// only `assembly.runtime()` (the pool + bus ride along inside the `Arc`); the
/// lead agent + its tools are the CLI's lead-driven path, not ours.
fn assemble_runtime(
    deps: &TeamRunDeps,
    config: TeamConfig,
) -> Result<Arc<TeamRuntime>, MissionDriverError> {
    let lead = config
        .lead_member()
        .ok_or_else(|| TeamError::Config("team has no lead".into()))?
        .clone();
    let lead_caps = lead.declared_capabilities()?;
    let assembly = TeamAssembly::build(
        config,
        Arc::clone(&deps.provider),
        deps.model.clone(),
        deps.max_tokens,
        Arc::clone(&deps.audit),
        deps.base_tools.clone(),
        lead_caps,
    )?;
    Ok(assembly.runtime())
}

/// Drive `id` from its current checkpoint until it pauses at a human gate or
/// reaches a terminal state, persisting every step. Marks the mission
/// `Executing`, then applies the [`RunYield`] outcome.
async fn drive(
    shared: &SharedMissionState,
    runtime: Arc<TeamRuntime>,
    id: &str,
) -> Result<TeamMissionPhase, MissionDriverError> {
    let mut record = shared
        .snapshot(id)
        .ok_or_else(|| MissionDriverError::NotFound(id.to_string()))?;
    let plan = record.plan.clone();
    let checkpoint = record.outputs.clone();

    record.phase = TeamMissionPhase::Executing;
    record.pending_gate = None;
    shared.put(record).await?;

    // The observer pings on each step completion; the drive runs in a task so
    // that when it ends the observer (and its sender) drop, closing the ping
    // channel and ending the drain loop.
    let (tx, mut rx) = mpsc::unbounded_channel();
    let observer = RegistryObserver {
        shared: shared.clone(),
        id: id.to_string(),
        tx,
    };
    let channel = MissionLeadChannel::new();
    let run = tokio::spawn(async move {
        runtime
            .run_until_pause(&plan, checkpoint, &channel, &observer)
            .await
    });

    while rx.recv().await.is_some() {
        shared.persist_current(id).await?;
    }
    let outcome = run
        .await
        .map_err(|e| MissionDriverError::Join(e.to_string()))??;

    let mut record = shared
        .snapshot(id)
        .ok_or_else(|| MissionDriverError::NotFound(id.to_string()))?;
    match outcome {
        RunYield::Done(report) => {
            record.outputs = report.outputs;
            record.pending_gate = None;
            record.phase = match report.status {
                MissionStatus::Completed => TeamMissionPhase::Done,
                MissionStatus::GateRejected { .. } => TeamMissionPhase::Rejected,
            };
        }
        RunYield::AwaitingHuman { step, outputs } => {
            record.outputs = outputs;
            record.phase = TeamMissionPhase::AwaitingApproval;
            record.pending_gate = Some(step);
        }
    }
    let phase = record.phase;
    shared.put(record).await?;
    Ok(phase)
}

/// Feeds step progress into the registry as the DAG is walked. Updates are
/// in-memory + synchronous (the observer's callbacks are sync); a ping on `tx`
/// tells the driver to persist the new checkpoint. Maps onto the TUI's live
/// `MissionStep` states (L.6).
struct RegistryObserver {
    shared: SharedMissionState,
    id: String,
    tx: mpsc::UnboundedSender<()>,
}

impl MissionObserver for RegistryObserver {
    fn on_step_completed(&self, step_id: &str, output: &str) {
        self.shared.touch_in_memory(&self.id, |r| {
            r.outputs.insert(step_id.to_string(), output.to_string());
        });
        let _ = self.tx.send(());
    }

    fn on_gate(&self, step_id: &str, _passed: bool, verdict: &str) {
        self.shared.touch_in_memory(&self.id, |r| {
            r.outputs.insert(step_id.to_string(), verdict.to_string());
        });
        let _ = self.tx.send(());
    }
}

/// A fresh local Trusted channel for a daemon mission run — the lead context
/// `run_until_pause` derives each specialist sub-turn's channel from. Operator
/// progress is the live TUI feed (L.6), not this channel's stream.
struct MissionLeadChannel {
    session: SessionId,
    token: CancellationToken,
}

impl MissionLeadChannel {
    fn new() -> Self {
        MissionLeadChannel {
            session: SessionId::new(),
            token: CancellationToken::new(),
        }
    }
}

#[async_trait]
impl ChannelContext for MissionLeadChannel {
    fn channel_name(&self) -> &str {
        "team-mission"
    }
    fn platform(&self) -> ChannelPlatform {
        ChannelPlatform::Local
    }
    fn trust_tier(&self) -> TrustTier {
        TrustTier::Trusted
    }
    fn session_id(&self) -> SessionId {
        self.session
    }
    async fn stream_event(&self, _event: StreamEvent<'_>) -> Result<(), ChannelError> {
        Ok(())
    }
    async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), ChannelError> {
        Ok(())
    }
    fn cancellation_token(&self) -> CancellationToken {
        self.token.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    use aivyx_core::NullAuditHook;
    use aivyx_crypto::MasterKey;
    use aivyx_llm::{
        LlmError, LlmRequest, LlmStepEnd, LlmStream, LlmStreamEvent, LlmUsage,
    };
    use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
    use aivyx_team::{default_nonagon, MissionPlan, Step};

    // --- a fake provider: every sub-turn completes with one fixed line -------

    struct FakeProvider {
        line: String,
    }

    #[async_trait]
    impl LlmProvider for FakeProvider {
        async fn chat_stream(
            &self,
            _req: LlmRequest<'_>,
            _cancel: &CancellationToken,
        ) -> Result<Box<dyn LlmStream>, LlmError> {
            Ok(Box::new(FakeStream {
                events: vec![LlmStreamEvent::TextChunk(self.line.clone())].into_iter(),
                terminal: Some(LlmStepEnd::FinalMessage {
                    text: self.line.clone(),
                    usage: LlmUsage::default(),
                }),
            }))
        }
    }

    struct FakeStream {
        events: std::vec::IntoIter<LlmStreamEvent>,
        terminal: Option<LlmStepEnd>,
    }

    #[async_trait]
    impl LlmStream for FakeStream {
        async fn next_event(&mut self) -> Result<Option<LlmStreamEvent>, LlmError> {
            Ok(self.events.next())
        }
        async fn finish(mut self: Box<Self>) -> Result<LlmStepEnd, LlmError> {
            Ok(self.terminal.take().expect("finish once"))
        }
    }

    async fn team_domain() -> DomainHandle {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir()
            .join(format!("aivyx-team-driver-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let storage: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([7u8; 32]),
        )
        .await
        .expect("open storage");
        storage.domain(KeyDomain::TeamMissions)
    }

    fn deps(line: &str) -> TeamRunDeps {
        TeamRunDeps {
            provider: Arc::new(FakeProvider { line: line.into() }),
            model: "test-model".into(),
            max_tokens: 1024,
            audit: Arc::new(NullAuditHook),
            base_tools: vec![],
        }
    }

    /// research → [human gate] → write.
    fn gated_plan() -> MissionPlan {
        MissionPlan::new(
            "ship the note",
            vec![
                Step::delegate("research", "researcher", "gather"),
                Step::human_gate("approve", "reviewer", "ok to publish?").after(["research"]),
                Step::delegate("write", "writer", "draft").after(["approve"]),
            ],
        )
    }

    #[tokio::test]
    async fn runs_a_linear_plan_to_done() {
        let shared = SharedMissionState::new(team_domain().await);
        let plan = MissionPlan::new(
            "two-step",
            vec![
                Step::delegate("a", "researcher", "p"),
                Step::delegate("b", "writer", "p").after(["a"]),
            ],
        );
        let id = team_run(&shared, &deps("done-line"), default_nonagon(), plan, "m1")
            .await
            .unwrap();
        let rec = shared.snapshot(&id).unwrap();
        assert_eq!(rec.phase, TeamMissionPhase::Done);
        assert_eq!(rec.outputs["a"], "done-line");
        assert_eq!(rec.outputs["b"], "done-line");
        assert!(rec.pending_gate.is_none());
    }

    #[tokio::test]
    async fn pauses_at_a_human_gate_and_persists_the_checkpoint() {
        let store = team_domain().await;
        let shared = SharedMissionState::new(store.clone());
        let id = team_run(&shared, &deps("x"), default_nonagon(), gated_plan(), "m2")
            .await
            .unwrap();

        let rec = shared.snapshot(&id).unwrap();
        assert_eq!(rec.phase, TeamMissionPhase::AwaitingApproval);
        assert_eq!(rec.pending_gate.as_deref(), Some("approve"));
        assert!(rec.outputs.contains_key("research"), "checkpoint has upstream");
        assert!(!rec.outputs.contains_key("write"), "the gated step hasn't run");

        // The pause is durable: a fresh registry reloads it as AwaitingApproval.
        let reloaded = SharedMissionState::new(store);
        assert_eq!(reloaded.reload().await.unwrap(), 1);
        let got = reloaded.snapshot(&id).unwrap();
        assert_eq!(got.phase, TeamMissionPhase::AwaitingApproval);
        assert_eq!(got.pending_gate.as_deref(), Some("approve"));
    }

    #[tokio::test]
    async fn approve_resumes_the_gated_step_to_done() {
        let shared = SharedMissionState::new(team_domain().await);
        let d = deps("ok");
        let id = team_run(&shared, &d, default_nonagon(), gated_plan(), "m3")
            .await
            .unwrap();
        assert_eq!(shared.snapshot(&id).unwrap().phase, TeamMissionPhase::AwaitingApproval);

        let phase = resolve_team_gate(&shared, &d, default_nonagon(), &id, "approve", true)
            .await
            .unwrap();
        assert_eq!(phase, TeamMissionPhase::Done);
        let rec = shared.snapshot(&id).unwrap();
        assert_eq!(rec.outputs["write"], "ok", "the gated dependent ran on resume");
        assert!(rec.outputs["approve"].contains("approved"));
        assert!(rec.pending_gate.is_none());
    }

    #[tokio::test]
    async fn reject_ends_rejected_and_skips_dependents() {
        let shared = SharedMissionState::new(team_domain().await);
        let d = deps("ok");
        let id = team_run(&shared, &d, default_nonagon(), gated_plan(), "m4")
            .await
            .unwrap();

        let phase = resolve_team_gate(&shared, &d, default_nonagon(), &id, "approve", false)
            .await
            .unwrap();
        assert_eq!(phase, TeamMissionPhase::Rejected);
        let rec = shared.snapshot(&id).unwrap();
        assert!(!rec.outputs.contains_key("write"), "the dependent never ran");
        assert_eq!(rec.outputs["approve"], "rejected by operator");
        assert!(rec.pending_gate.is_none());
    }

    #[tokio::test]
    async fn resolve_rejects_unknown_and_non_awaiting_missions() {
        let shared = SharedMissionState::new(team_domain().await);
        let d = deps("ok");
        // Unknown id.
        assert!(matches!(
            resolve_team_gate(&shared, &d, default_nonagon(), "ghost", "s", true).await,
            Err(MissionDriverError::NotFound(_))
        ));
        // A finished mission isn't awaiting anything.
        let id = team_run(
            &shared,
            &d,
            default_nonagon(),
            MissionPlan::new("one", vec![Step::delegate("a", "writer", "p")]),
            "m5",
        )
        .await
        .unwrap();
        assert!(matches!(
            resolve_team_gate(&shared, &d, default_nonagon(), &id, "a", true).await,
            Err(MissionDriverError::NotAwaiting(_, _))
        ));
    }

    #[tokio::test]
    async fn resolve_rejects_the_wrong_gate_id() {
        let shared = SharedMissionState::new(team_domain().await);
        let d = deps("ok");
        let id = team_run(&shared, &d, default_nonagon(), gated_plan(), "m6")
            .await
            .unwrap();
        assert!(matches!(
            resolve_team_gate(&shared, &d, default_nonagon(), &id, "not-the-gate", true).await,
            Err(MissionDriverError::WrongGate(_, _, _))
        ));
    }

    #[tokio::test]
    async fn reload_is_empty_on_a_fresh_store() {
        let shared = SharedMissionState::new(team_domain().await);
        assert_eq!(shared.reload().await.unwrap(), 0);
        assert!(shared.list().is_empty());
    }

    /// Poll the registry until `id` reaches `want`, or fail after a bound. The
    /// fake provider settles in microseconds; this just yields to the spawned
    /// drive task.
    async fn wait_for(svc: &TeamMissionService, id: &str, want: TeamMissionPhase) {
        for _ in 0..200 {
            if svc.snapshot(id).map(|r| r.phase) == Some(want) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("mission {id} never reached {want:?}: {:?}", svc.snapshot(id));
    }

    #[tokio::test]
    async fn service_start_spawns_a_drive_that_pauses_then_resumes() {
        let svc = TeamMissionService::new(
            SharedMissionState::new(team_domain().await),
            deps("ok"),
            default_nonagon(),
        );
        // start returns immediately with a fresh id; the drive runs in the bg.
        let id = svc.start(gated_plan()).await.unwrap();
        assert_eq!(svc.list().len(), 1);

        wait_for(&svc, &id, TeamMissionPhase::AwaitingApproval).await;
        assert_eq!(svc.snapshot(&id).unwrap().pending_gate.as_deref(), Some("approve"));

        // resolve(approve) flips to Executing synchronously, then drives to Done.
        let phase = svc.resolve(&id, "approve", true).await.unwrap();
        assert_eq!(phase, TeamMissionPhase::Executing);
        wait_for(&svc, &id, TeamMissionPhase::Done).await;
        assert_eq!(svc.snapshot(&id).unwrap().outputs["write"], "ok");
    }
}

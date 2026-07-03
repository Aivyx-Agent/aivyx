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
use std::sync::{Arc, OnceLock, RwLock};

use aivyx_capability::{Scope, TrustTier};
use aivyx_core::{
    AivyxError, AuditHook, AuditTag, CancellationToken, ChannelContext, ChannelError,
    ChannelPlatform, GatePolicy, SessionId, StreamEvent, Tool, ToolContext, ToolId, ToolOutcome,
    TurnOutcome,
    Verification,
};
use serde_json::{json, Value};
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
    /// Chapter Belay — abort was requested on a mission that isn't running.
    #[error("mission {0} cannot be aborted ({1})")]
    NotAbortable(String, String),
}

/// Chapter Ensemble — builds an LLM provider (the daemon's kind) at a given
/// `base_url`, for a team member that declared its own per-role endpoint.
pub type MemberProviderBuilder =
    Arc<dyn Fn(&str) -> Result<Arc<dyn LlmProvider>, String> + Send + Sync>;

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
    /// Chapter Ballast (Opp D) — model pricing used to meter a mission's
    /// spend against `mission_budget`. Local models price at $0.
    pub pricing: Arc<aivyx_cost::Pricing>,
    /// Chapter Ballast (Opp D) — the per-mission aggregate cap (tokens + $).
    /// `MissionBudget::default()` (no caps) ⇒ unbounded: the metering hook +
    /// halt check are skipped entirely and the run is byte-identical to before.
    pub mission_budget: aivyx_cost::MissionBudget,
    /// Chapter Ensemble — builds an LLM provider of the daemon's kind at a
    /// given `base_url`, for a team member that declared a per-role endpoint.
    /// `None` ⇒ per-role `base_url` overrides are ignored (members fall back to
    /// the shared provider; per-role `model` still applies). The binary supplies
    /// it because provider construction lives there.
    pub member_provider_builder: Option<MemberProviderBuilder>,
    /// #17d — the memory substrate, so the delegated completion judge can
    /// ground its verdict on the artifact the team actually wrote (symmetric
    /// with the solo `loop.complete` path). `None` ⇒ the judge is summary-only,
    /// as before.
    pub memory: Option<Arc<dyn aivyx_memory::Memory>>,
    /// #17d — the agent workspace root, so the delegated judge can also ground
    /// on recent FILE artifacts the team wrote (symmetric with the solo path).
    /// `None` ⇒ file evidence is skipped.
    pub workspace_root: Option<std::path::PathBuf>,
    /// Chapter Keystone — when true, a mission that runs to completion is graded
    /// against its goal, GROUNDED on the workspace/memory artifacts it produced,
    /// before it reports `Done`; a mission that claims done but produced no
    /// deliverable is flipped to `Rejected`. `false` (default) ⇒ a mission is
    /// `Done` on step-completion alone (pre-Keystone behavior).
    pub verify_missions: bool,
}

/// In-memory registry of daemon-run team missions, backed by the encrypted
/// `KeyDomain::TeamMissions` store. Cloned into every IPC handler and the
/// drive task (mirrors `SharedLoopState`).
#[derive(Clone)]
pub struct SharedMissionState {
    store: DomainHandle,
    registry: Arc<RwLock<BTreeMap<String, TeamMissionRecord>>>,
    /// Chapter Belay — runtime-only abort flags, keyed by mission id. The drive
    /// arms one when a mission starts executing; the observer reads it at each
    /// wave boundary; `request_abort` sets it. Not persisted (a flag is
    /// meaningless across a restart — an interrupted mission re-drives fresh).
    abort_flags: Arc<RwLock<std::collections::HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
}

impl SharedMissionState {
    /// A registry over `store` (a `KeyDomain::TeamMissions` handle). Empty
    /// until [`reload`](Self::reload) hydrates it from the store.
    pub fn new(store: DomainHandle) -> Self {
        SharedMissionState {
            store,
            registry: Arc::new(RwLock::new(BTreeMap::new())),
            abort_flags: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Chapter Belay — arm a fresh abort flag for an executing mission and
    /// return it (the drive hands the clone to the observer's `should_halt`).
    fn arm_abort(&self, id: &str) -> Arc<std::sync::atomic::AtomicBool> {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.abort_flags
            .write()
            .expect("abort flags lock")
            .insert(id.to_string(), Arc::clone(&flag));
        flag
    }

    /// Chapter Belay — drop a mission's abort flag once its drive ends.
    fn disarm_abort(&self, id: &str) {
        self.abort_flags.write().expect("abort flags lock").remove(id);
    }

    /// Chapter Belay — request that an executing mission halt at its next wave
    /// boundary. Returns `true` if the mission was running (a flag was armed),
    /// `false` if not (already terminal, paused, or unknown).
    pub fn request_abort(&self, id: &str) -> bool {
        match self.abort_flags.read().expect("abort flags lock").get(id) {
            Some(flag) => {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// Reload-on-startup: hydrate the in-memory registry from the store.
    /// Returns the number of missions loaded (doc §4).
    ///
    /// Chapter Reckon — reconcile zombies. A mission left `Executing` when the
    /// daemon stopped has no live drive task in this fresh process, so it would
    /// otherwise sit `Executing` forever in `team list`. There is no resume
    /// machinery for team missions (unlike Chapter Helm for the loop), so mark
    /// each interrupted `Executing` mission `Halted` with a truthful reason and
    /// persist that — the operator sees an honest terminal state (and can
    /// re-run) instead of a permanent zombie. `AwaitingApproval` is a legitimate
    /// pause (an operator gate) and is left untouched.
    pub async fn reload(&self) -> Result<usize, StorageError> {
        let records = list_team_missions(&self.store).await?;
        let mut reconciled: Vec<TeamMissionRecord> = Vec::new();
        {
            let mut reg = self.registry.write().expect("mission registry lock");
            reg.clear();
            for r in &records {
                let mut r = r.clone();
                if r.phase == TeamMissionPhase::Executing {
                    r.phase = TeamMissionPhase::Halted;
                    r.pending_gate = None;
                    r.halt_reason =
                        Some("interrupted by a daemon restart (not resumed)".to_string());
                    reconciled.push(r.clone());
                }
                reg.insert(r.id.clone(), r);
            }
        }
        // Persist the reconciled terminal state so the store agrees with memory.
        for r in reconciled {
            let _ = save_team_mission(&self.store, &r).await;
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
    // Pin the mission to `config` (persisted on the record), then drive it —
    // `config` doubles as the resume fallback.
    let id = register_mission(shared, plan, id, Some(config.clone())).await?;
    drive_registered(shared, deps, config, &id, GatePolicy::Interactive).await?;
    Ok(id)
}

/// Validate and persist a fresh mission in the `Planning` phase, returning its
/// id. `config` pins the team (a vertical pack); `None` ⇒ the daemon default.
/// The daemon registers synchronously (so a `TeamMissionStatus` poll sees it
/// immediately) then spawns [`drive_registered`].
pub async fn register_mission(
    shared: &SharedMissionState,
    plan: MissionPlan,
    id: impl Into<String>,
    config: Option<TeamConfig>,
) -> Result<String, MissionDriverError> {
    let id = id.into();
    // Validate before we register anything the operator would have to clean up.
    plan.validate()?;
    let goal = plan.goal.clone();
    shared
        .put(TeamMissionRecord::new(&id, goal, plan).with_config(config))
        .await?;
    Ok(id)
}

/// Assemble the team and drive an **already-registered** mission from its
/// checkpoint to the next pause / terminal state. The mission runs on the team
/// it was registered with (`record.config`); `default_config` is the fallback
/// for missions started without a pack (and legacy records). The long-running
/// half of a mission run; the daemon `tokio::spawn`s it.
pub async fn drive_registered(
    shared: &SharedMissionState,
    deps: &TeamRunDeps,
    default_config: TeamConfig,
    id: &str,
    policy: GatePolicy,
) -> Result<TeamMissionPhase, MissionDriverError> {
    let config = shared
        .snapshot(id)
        .and_then(|r| r.config)
        .unwrap_or(default_config);
    let (runtime, meter) = assemble_runtime(deps, config)?;
    let budget_guard = meter.map(|m| (m, deps.mission_budget.clone()));
    let phase = drive(shared, runtime, id, policy, &deps.audit, budget_guard).await?;
    // Chapter Keystone — mission-level artifact grounding. A mission is `Done`
    // only if its deliverable actually exists: grade the completed mission
    // against its goal, grounded on the workspace/memory artifacts, and flip a
    // "done but produced nothing" mission to `Rejected`. Opt-in + best-effort.
    if phase == TeamMissionPhase::Done && deps.verify_missions {
        return Ok(verify_mission_artifact(shared, deps, id).await);
    }
    Ok(phase)
}

/// Chapter Keystone — grade a just-completed mission against its goal, grounded
/// on the artifacts it produced. Returns the resulting phase: `Done` if the
/// judge accepts (or can't run — best-effort fails open), `Rejected` if the
/// deliverable isn't there. On a reject it updates the record (phase +
/// halt_reason) and lands the verdict on the audit chain.
async fn verify_mission_artifact(
    shared: &SharedMissionState,
    deps: &TeamRunDeps,
    id: &str,
) -> TeamMissionPhase {
    // Need a snapshot + at least one grounding source; else keep `Done`.
    let Some(record) = shared.snapshot(id) else {
        return TeamMissionPhase::Done;
    };
    if deps.memory.is_none() && deps.workspace_root.is_none() {
        return TeamMissionPhase::Done;
    }
    let mut judge = crate::completion_judge::CompletionJudge::new(
        Arc::clone(&deps.provider),
        deps.model.clone(),
    );
    if let Some(m) = &deps.memory {
        judge = judge.with_memory(Arc::clone(m));
    }
    if let Some(ws) = &deps.workspace_root {
        judge = judge.with_workspace(ws.clone());
    }
    // The synthesized result the mission produced (its step outputs).
    let result = record
        .outputs
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n\n");
    let verdict = judge.verify(&record.goal, &record.goal, &result).await;
    eprintln!(
        "aivyx team: mission {id} artifact verdict — {}: {}",
        if verdict.passed { "ACCEPTED" } else { "REJECTED" },
        verdict.reason,
    );
    if verdict.passed {
        return TeamMissionPhase::Done;
    }
    // Flip to Rejected: the mission claimed done but the deliverable isn't there.
    if let Some(mut rec) = shared.snapshot(id) {
        rec.phase = TeamMissionPhase::Rejected;
        rec.halt_reason = Some(format!("deliverable not verified: {}", verdict.reason));
        let _ = shared.put(rec).await;
    }
    deps.audit.on_event(AuditTag::HeadlessRefusal {
        run_id: id.to_string(),
        step: "<artifact-gate>".to_string(),
        reason: format!("team mission rejected — deliverable not verified: {}", verdict.reason),
    });
    TeamMissionPhase::Rejected
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
        // Approve flipped the mission to `Executing` — drive the resume. A
        // resume after an operator decision is inherently interactive (a human
        // just acted), so the next human gate, if any, parks as usual.
        TeamMissionPhase::Executing => {
            drive_registered(shared, deps, config, id, GatePolicy::Interactive).await
        }
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

/// Chapter Belay — request that a **running** mission stop. Sets the mission's
/// abort flag; its drive halts gracefully at the next wave boundary (in-flight
/// specialist turns finish, completed outputs are preserved), landing the
/// mission in `Halted` with reason "aborted by operator" — the same terminal
/// shape as a tripped budget cap. A mission paused at a human gate isn't running,
/// so it can't be aborted this way — reject its gate instead. Returns a short
/// status message on success.
pub fn abort_mission(
    shared: &SharedMissionState,
    id: &str,
) -> Result<String, MissionDriverError> {
    let record = shared
        .snapshot(id)
        .ok_or_else(|| MissionDriverError::NotFound(id.to_string()))?;
    match record.phase {
        TeamMissionPhase::Executing => {
            if shared.request_abort(id) {
                Ok(format!(
                    "abort requested — mission {id} will halt at its next step boundary"
                ))
            } else {
                // Executing in the record but no armed flag (e.g. a just-finished
                // race): nothing to halt.
                Err(MissionDriverError::NotAbortable(
                    id.to_string(),
                    "the mission is no longer running".to_string(),
                ))
            }
        }
        TeamMissionPhase::AwaitingApproval => Err(MissionDriverError::NotAbortable(
            id.to_string(),
            "it is paused at a human gate — reject the gate instead".to_string(),
        )),
        other => Err(MissionDriverError::NotAbortable(
            id.to_string(),
            format!("it is not currently running (phase {other:?})"),
        )),
    }
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
    /// Chapter H — the gate posture for missions this service drives. A
    /// headless service rejects (rather than parks) at a human gate.
    gate_policy: GatePolicy,
}

impl TeamMissionService {
    /// A service over `state`, assembling `config` with `deps` per run, under
    /// `gate_policy` (the daemon's posture; `Interactive` parks at a human
    /// gate, headless rejects). (For L.5 the daemon passes the default Nonagon;
    /// vertical-pack configs are a later increment.)
    pub fn new(
        state: SharedMissionState,
        deps: TeamRunDeps,
        config: TeamConfig,
        gate_policy: GatePolicy,
    ) -> Self {
        TeamMissionService { state, deps, config, gate_policy }
    }

    /// The underlying registry — the read path (`reload`, `snapshot`, `list`).
    pub fn state(&self) -> &SharedMissionState {
        &self.state
    }

    /// One mission's snapshot.
    pub fn snapshot(&self, id: &str) -> Option<TeamMissionRecord> {
        self.state.snapshot(id)
    }

    /// Verdict for delegated stories — a completion judge over this service's own
    /// provider + model, so the loop driver can hold an auto-delegated mission's
    /// result to the same acceptance bar as a solo `loop.complete`.
    pub fn completion_judge(&self) -> crate::completion_judge::CompletionJudge {
        let judge = crate::completion_judge::CompletionJudge::new(
            Arc::clone(&self.deps.provider),
            self.deps.model.clone(),
        );
        // #17d — ground the delegated verdict on the real memory artifact the
        // team wrote, so a mission that only *claims* completion (terse synth
        // over genuine work OR a hollow "Done" with nothing produced) is judged
        // against what's actually in memory — same bar as solo loop.complete.
        let judge = match &self.deps.memory {
            Some(m) => judge.with_memory(Arc::clone(m)),
            None => judge,
        };
        match &self.deps.workspace_root {
            Some(ws) => judge.with_workspace(ws.clone()),
            None => judge,
        }
    }

    /// Every known mission (the poll feed).
    pub fn list(&self) -> Vec<TeamMissionRecord> {
        self.state.list()
    }

    /// Chapter Y — the active team roster (the `TeamConfig` this service
    /// assembles per run). The Studio's `GetTeamRoster` handler renders it.
    pub fn team_config(&self) -> TeamConfig {
        self.config.clone()
    }

    /// Register a mission from an explicit plan and **spawn** its drive,
    /// returning the new id immediately. `config` pins a vertical pack (`None`
    /// ⇒ the daemon default team). The drive runs to the first human gate or
    /// terminal state in the background.
    pub async fn start(
        &self,
        plan: MissionPlan,
        config: Option<TeamConfig>,
    ) -> Result<String, MissionDriverError> {
        let id =
            register_mission(&self.state, plan, uuid::Uuid::new_v4().to_string(), config).await?;
        self.spawn_drive(id.clone());
        Ok(id)
    }

    /// Decompose a free-text `goal` into a plan (one LLM planning call over the
    /// chosen team's roster), then [`start`](Self::start) it on that team.
    /// `config` pins a vertical pack (`None` ⇒ the daemon default). The
    /// decomposition is awaited (a few seconds) so the returned id belongs to a
    /// registered, validated mission; the drive then runs in the background.
    pub async fn start_from_goal(
        &self,
        goal: &str,
        config: Option<TeamConfig>,
    ) -> Result<String, MissionDriverError> {
        let cancel = aivyx_core::CancellationToken::new();
        let plan = aivyx_team::decompose_goal(
            self.deps.provider.as_ref(),
            &self.deps.model,
            goal,
            config.as_ref().unwrap_or(&self.config),
            &cancel,
            // Interactive `team.run`: the operator can approve human gates.
            true,
        )
        .await?;
        self.start(plan, config).await
    }

    /// Chapter Foreman — decompose `goal`, register the mission, and **drive it
    /// to a terminal state inline** (not spawned), returning the mission id and
    /// its final phase. The autonomous loop's auto-delegation uses this to hand a
    /// complex story to the team and wait for the result. `policy` is passed
    /// explicitly so the loop can run it **headless** (a human gate auto-rejects
    /// rather than parking forever in an unattended run).
    pub async fn run_goal_blocking(
        &self,
        goal: &str,
        config: Option<TeamConfig>,
        policy: GatePolicy,
    ) -> Result<(String, TeamMissionPhase), MissionDriverError> {
        let cancel = aivyx_core::CancellationToken::new();
        let plan = aivyx_team::decompose_goal(
            self.deps.provider.as_ref(),
            &self.deps.model,
            goal,
            config.as_ref().unwrap_or(&self.config),
            &cancel,
            // #15 — a headless drive forbids ALL gates in the plan: no operator
            // to approve a human gate, and a failed auto-gate would abort the
            // whole mission → retry → a doable story gets skipped (work lost).
            !policy.is_headless(),
        )
        .await?;
        let id = register_mission(
            &self.state,
            plan,
            uuid::Uuid::new_v4().to_string(),
            config.clone(),
        )
        .await?;
        let default_config = config.unwrap_or_else(|| self.config.clone());
        let phase = match drive_registered(&self.state, &self.deps, default_config, &id, policy)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                eprintln!("aivyx loop: delegated mission {id} drive ERRORED: {e}");
                return Err(e);
            }
        };
        // Operator-visible: the terminal phase of a loop-delegated mission (so a
        // watcher sees whether auto-delegation completed or was halted/rejected).
        eprintln!("aivyx loop: delegated mission {id} → phase {phase:?}");
        Ok((id, phase))
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

    /// Chapter Belay — request that a running mission halt at its next wave
    /// boundary. Returns a short status message.
    pub fn abort(&self, id: &str) -> Result<String, MissionDriverError> {
        abort_mission(&self.state, id)
    }

    /// Spawn the background drive for an already-registered/-resumed mission.
    /// A drive failure leaves the record `Executing` for the operator to
    /// inspect; we log rather than unwind the detached task.
    fn spawn_drive(&self, id: String) {
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = drive_registered(
                &this.state,
                &this.deps,
                this.config.clone(),
                &id,
                this.gate_policy,
            )
            .await
            {
                eprintln!("aivyx team: mission {id} drive failed — {e}");
            }
        });
    }
}

/// Build the resumable runtime for a team. The plan-driven daemon path needs
/// only `assembly.runtime()` (the pool + bus ride along inside the `Arc`); the
/// lead agent + its tools are the CLI's lead-driven path, not ours.
///
/// Chapter Ballast — when `deps.mission_budget` is bounded, the team's audit
/// hook is wrapped in a [`MeteringAuditHook`] so this mission's spend is tallied
/// (the returned [`MissionMeter`] feeds the driver's wave-boundary halt check).
/// When unbounded, the real audit is used directly and `None` is returned — the
/// run is byte-identical to pre-Ballast.
fn assemble_runtime(
    deps: &TeamRunDeps,
    config: TeamConfig,
) -> Result<(Arc<TeamRuntime>, Option<crate::mission_meter::MissionMeter>), MissionDriverError> {
    let lead = config
        .lead_member()
        .ok_or_else(|| TeamError::Config("team has no lead".into()))?
        .clone();
    let lead_caps = lead.declared_capabilities()?;

    let (audit, meter): (Arc<dyn AuditHook>, Option<crate::mission_meter::MissionMeter>) =
        if deps.mission_budget.is_unbounded() {
            (Arc::clone(&deps.audit), None)
        } else {
            let hook = crate::mission_meter::MeteringAuditHook::new(
                Arc::clone(&deps.audit),
                Arc::clone(&deps.pricing),
            );
            let meter = hook.meter();
            (Arc::new(hook), Some(meter))
        };

    // Chapter Ensemble — resolve per-role backend overrides from the team
    // config (a member's own `model` and/or `base_url`).
    let member_backends = resolve_member_backends(
        &config.members,
        &deps.provider,
        &deps.model,
        deps.member_provider_builder.as_ref(),
    )?;

    let assembly = TeamAssembly::build(
        config,
        Arc::clone(&deps.provider),
        deps.model.clone(),
        deps.max_tokens,
        audit,
        deps.base_tools.clone(),
        lead_caps,
        member_backends,
    )?;
    Ok((assembly.runtime(), meter))
}

/// Chapter Ensemble — resolve per-role backend overrides. A member with its own
/// `model` and/or `base_url` gets a [`SpecialistBackend`]; members without
/// either are absent (they use the shared default). A `base_url` is honoured
/// only when `builder` is wired (else the role falls back to the shared
/// provider, keeping any `model` override). Pure over its inputs so the
/// resolution is unit-testable without a daemon.
#[allow(clippy::type_complexity)]
fn resolve_member_backends(
    members: &[aivyx_team::TeamMember],
    default_provider: &Arc<dyn LlmProvider>,
    default_model: &str,
    builder: Option<&MemberProviderBuilder>,
) -> Result<std::collections::HashMap<String, aivyx_team::SpecialistBackend>, MissionDriverError>
{
    let mut map = std::collections::HashMap::new();
    for m in members {
        if m.model.is_none() && m.base_url.is_none() {
            continue;
        }
        let provider = match (&m.base_url, builder) {
            (Some(url), Some(build)) => build(url).map_err(|e| {
                TeamError::Config(format!(
                    "team member {:?} base_url {url:?}: {e}",
                    m.name
                ))
            })?,
            // base_url set but no builder wired → can't honour the endpoint;
            // fall back to the shared provider (any model override still applies).
            _ => Arc::clone(default_provider),
        };
        let model = m.model.clone().unwrap_or_else(|| default_model.to_string());
        map.insert(
            m.name.clone(),
            aivyx_team::SpecialistBackend { provider, model },
        );
    }
    Ok(map)
}

/// Drive `id` from its current checkpoint until it pauses at a human gate or
/// reaches a terminal state, persisting every step. Marks the mission
/// `Executing`, then applies the [`RunYield`] outcome.
async fn drive(
    shared: &SharedMissionState,
    runtime: Arc<TeamRuntime>,
    id: &str,
    policy: GatePolicy,
    audit: &Arc<dyn AuditHook>,
    // Chapter Ballast — `Some` when a per-mission budget is armed: the meter
    // tracks this mission's spend and the budget says when to halt. `None` ⇒
    // unbounded (the observer's `should_halt` stays the default no-op).
    budget_guard: Option<(crate::mission_meter::MissionMeter, aivyx_cost::MissionBudget)>,
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
    // Chapter Belay — arm this mission's abort flag; the observer halts the run
    // at the next wave boundary if an operator requests an abort.
    let abort = shared.arm_abort(id);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let observer = RegistryObserver {
        shared: shared.clone(),
        id: id.to_string(),
        tx,
        budget_guard,
        abort: Some(abort),
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
            // Chapter Belay — stashed here so it can be set on `record` after the
            // phase assignment (mutating `record` inside `record.phase = match …`
            // would be a borrow conflict).
            let mut halted_reason: Option<String> = None;
            record.phase = match report.status {
                MissionStatus::Completed => TeamMissionPhase::Done,
                MissionStatus::GateRejected { .. } => TeamMissionPhase::Rejected,
                // A halt at a wave boundary — a per-mission budget cap (Ballast)
                // or an operator abort (Belay). Land the reason on the audit
                // chain (same legibility as the headless-refusal path), preserve
                // the partial outputs above, and stash the reason for the record.
                MissionStatus::Halted { reason } => {
                    eprintln!(
                        "aivyx team: mission {id} halted — {reason}"
                    );
                    audit.on_event(AuditTag::HeadlessRefusal {
                        run_id: id.to_string(),
                        step: "<halt>".to_string(),
                        reason: format!("team mission halted: {reason}"),
                    });
                    halted_reason = Some(reason);
                    TeamMissionPhase::Halted
                }
            };
            record.halt_reason = halted_reason;
        }
        RunYield::AwaitingHuman { step, outputs } => {
            record.outputs = outputs;
            record.pending_gate = None;
            if policy.is_headless() {
                // Chapter H — no operator to approve; the human gate is a
                // refusal. Reject the mission (dependents never run, partial
                // outputs preserved) — the same terminal shape as an operator
                // reject, decided by policy.
                //
                // H.6 — land the refusal on the audit chain so the unattended
                // path stays as legible as the attended one (the operator
                // reviewing later sees exactly which step we declined on their
                // behalf, and why).
                let reason = format!(
                    "team mission human-approval gate at step '{step}' refused (headless run, no operator)"
                );
                eprintln!("aivyx team: {reason} — mission {id} rejected");
                audit.on_event(AuditTag::HeadlessRefusal {
                    run_id: id.to_string(),
                    step: step.clone(),
                    reason,
                });
                record
                    .outputs
                    .insert(step, "rejected: headless run (no operator)".to_string());
                record.phase = TeamMissionPhase::Rejected;
            } else {
                record.phase = TeamMissionPhase::AwaitingApproval;
                record.pending_gate = Some(step);
            }
        }
    }
    let phase = record.phase;
    shared.put(record).await?;
    // Chapter Belay — the drive is over; drop the abort flag.
    shared.disarm_abort(id);
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
    /// Chapter Ballast — `Some` when a per-mission budget is armed. The meter
    /// reads this mission's running spend; the budget says when a cap trips.
    budget_guard: Option<(crate::mission_meter::MissionMeter, aivyx_cost::MissionBudget)>,
    /// Chapter Belay — the mission's abort flag. Set by `request_abort`; read at
    /// each wave boundary in `should_halt`.
    abort: Option<Arc<std::sync::atomic::AtomicBool>>,
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

    /// Chapter Ballast — the runtime calls this at each wave boundary. Compare
    /// the mission's metered spend so far against its caps; a breach returns the
    /// reason, halting the mission gracefully before the next wave launches.
    fn should_halt(&self) -> Option<String> {
        // Chapter Belay — an operator abort takes priority over the budget check.
        if let Some(flag) = &self.abort {
            if flag.load(std::sync::atomic::Ordering::SeqCst) {
                return Some("aborted by operator".to_string());
            }
        }
        let (meter, budget) = self.budget_guard.as_ref()?;
        budget.breach(meter.tokens(), meter.usd())
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

// ---------------------------------------------------------------------------
// team.run — the loop ↔ team seam (Chapter L.7)
// ---------------------------------------------------------------------------

/// `team.run` — delegate a free-text goal to a **durable** daemon team mission.
///
/// Mounted in the daemon tool list, so any daemon turn — most importantly an
/// **autonomous-loop iteration** ([`crate::loop_driver`]) — can hand a large,
/// multi-part story to a Nonagon team instead of implementing it single-handed.
/// The daemon decomposes the goal, runs it through the checkpoint/resume engine
/// (gate-pausable, restart-durable, shown in the TUI Missions panel), and the
/// tool returns immediately with the new mission id (fire-and-forget; the
/// caller tracks progress via `aivyx team status <id>`).
///
/// Wired like the loop tools: built into the tool list before the
/// [`TeamMissionService`] exists, then [`set_service`](Self::set_service) is
/// called once storage is up. Without a service (e.g. a no-daemon run) the
/// call fails cleanly.
pub struct TeamRunTool {
    id: ToolId,
    schema: Value,
    service: OnceLock<TeamMissionService>,
}

impl std::fmt::Debug for TeamRunTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TeamRunTool")
            .field("id", &self.id)
            .field("has_service", &self.service.get().is_some())
            .finish()
    }
}

impl Default for TeamRunTool {
    fn default() -> Self {
        Self::new()
    }
}

impl TeamRunTool {
    pub fn new() -> Self {
        TeamRunTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "goal": {
                        "type": "string",
                        "description": "The mission goal — what the team should accomplish. \
                                        The daemon decomposes it into a DAG of specialist steps."
                    }
                },
                "required": ["goal"]
            }),
            service: OnceLock::new(),
        }
    }

    /// Wire the daemon's team-mission service (call once, after storage opens).
    /// Returns `true` if it was set, `false` if it was already set (the value
    /// is dropped — callers don't need it).
    pub fn set_service(&self, service: TeamMissionService) -> bool {
        self.service.set(service).is_ok()
    }
}

#[async_trait]
impl Tool for TeamRunTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "team.run"
    }
    fn description(&self) -> &str {
        "Delegate a goal to a durable, daemon-run agent team (the Nonagon). Use this for a \
         large or multi-part task better handled by several specialists than by you alone: \
         the daemon decomposes the goal into a plan and runs it in the background \
         (gate-pausable, restart-durable, visible in `aivyx team status`). Input: \
         `{ \"goal\": string }`. Returns the new mission id immediately — it does NOT wait \
         for the mission to finish."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("team.run").expect("known base")
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(service) = self.service.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "team.run invoked without a team-mission service; it is available \
                         only on the daemon (start one with `aivyx daemon run`)"
                    .to_string(),
            });
        };
        let goal = match input.get("goal").and_then(Value::as_str) {
            Some(g) if !g.trim().is_empty() => g.trim(),
            _ => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: "team.run requires a non-empty `goal` (string)".to_string(),
                });
            }
        };
        // Default team for now (the Nonagon); per-call pack selection is a
        // later increment, like the TUI new-mission prompt.
        match service.start_from_goal(goal, None).await {
            Ok(mission_id) => ToolOutcome::Completed {
                output: json!({
                    "mission_id": mission_id,
                    "status": "started",
                    "note": "the team mission runs in the background; track it with \
                             `aivyx team status` or the TUI Missions panel",
                }),
                verified: Verification::NotApplicable,
            },
            Err(e) => ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("team.run failed to start a mission: {e}"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    use aivyx_core::NullAuditHook;
    use aivyx_crypto::MasterKey;
    use aivyx_llm::{
        LlmError, LlmMessage, LlmRequest, LlmStepEnd, LlmStream, LlmStreamEvent,
        LlmUsage,
    };
    use aivyx_capability::TrustTier;
    use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
    use aivyx_team::{default_nonagon, MissionPlan, Step, TeamMember};

    // --- a fake provider: every sub-turn completes with one fixed line -------

    struct FakeProvider {
        line: String,
        /// Usage reported by every sub-turn (Chapter Ballast tests use a
        /// non-zero value so the per-mission meter accumulates).
        usage: LlmUsage,
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
                    usage: self.usage,
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
            provider: Arc::new(FakeProvider { line: line.into(), usage: LlmUsage::default() }),
            model: "test-model".into(),
            max_tokens: 1024,
            audit: Arc::new(NullAuditHook),
            base_tools: vec![],
            pricing: Arc::new(aivyx_cost::Pricing::default()),
            // Unbounded by default — the metering hook + halt check are skipped.
            mission_budget: aivyx_cost::MissionBudget::default(),
            member_provider_builder: None,
            memory: None,
            workspace_root: None,
            verify_missions: false,
        }
    }

    /// Records `HeadlessRefusal` tags so a test can assert the driver lands the
    /// refusal on the audit chain (H.6). Other tags are ignored.
    #[derive(Default)]
    struct CapturingAuditHook {
        refusals: std::sync::Mutex<Vec<(String, String)>>,
    }
    impl AuditHook for CapturingAuditHook {
        fn on_event(&self, tag: AuditTag) {
            if let AuditTag::HeadlessRefusal { step, reason, .. } = tag {
                self.refusals.lock().unwrap().push((step, reason));
            }
        }
    }

    fn deps_with_audit(line: &str, audit: Arc<dyn AuditHook>) -> TeamRunDeps {
        TeamRunDeps {
            provider: Arc::new(FakeProvider { line: line.into(), usage: LlmUsage::default() }),
            model: "test-model".into(),
            max_tokens: 1024,
            audit,
            base_tools: vec![],
            pricing: Arc::new(aivyx_cost::Pricing::default()),
            mission_budget: aivyx_cost::MissionBudget::default(),
            member_provider_builder: None,
            memory: None,
            workspace_root: None,
            verify_missions: false,
        }
    }

    /// #17d — a provider that records the prompt it was handed, so a test can
    /// assert the delegated completion judge actually SAW the memory artifact.
    struct CapturingProvider {
        reply: String,
        seen: Arc<std::sync::Mutex<String>>,
    }
    #[async_trait]
    impl LlmProvider for CapturingProvider {
        async fn chat_stream(
            &self,
            req: LlmRequest<'_>,
            _cancel: &CancellationToken,
        ) -> Result<Box<dyn LlmStream>, LlmError> {
            if let Some(LlmMessage::User { content }) = req.messages.first() {
                let text: String = content
                    .iter()
                    .filter_map(|b| match b {
                        aivyx_llm::ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                *self.seen.lock().unwrap() = text;
            }
            Ok(Box::new(FakeStream {
                events: vec![].into_iter(),
                terminal: Some(LlmStepEnd::FinalMessage {
                    text: self.reply.clone(),
                    usage: LlmUsage::default(),
                }),
            }))
        }
    }

    #[tokio::test]
    async fn delegated_completion_judge_grounds_on_deps_memory() {
        use aivyx_memory::{InMemoryMemory, Memory};
        let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        mem.put("sleep-notes", "Melatonin 1–3 mg shortens sleep onset; CBT-I is durable.")
            .await
            .unwrap();
        let seen = Arc::new(std::sync::Mutex::new(String::new()));
        let mut deps = deps("unused");
        deps.provider = Arc::new(CapturingProvider {
            reply: "PASS — memory shows the required note.".into(),
            seen: Arc::clone(&seen),
        });
        deps.memory = Some(Arc::clone(&mem));
        let service = TeamMissionService::new(
            SharedMissionState::new(team_domain().await),
            deps,
            default_nonagon(),
            GatePolicy::Interactive,
        );
        // A terse mission result still PASSES because the judge sees the artifact.
        let v = service
            .completion_judge()
            .verify("Note sleep tips", "memory has a sleep note", "did it")
            .await;
        assert!(v.passed, "delegated verdict grounded on memory: {v:?}");
        let prompt = seen.lock().unwrap().clone();
        assert!(prompt.contains("ground-truth evidence"), "evidence block present");
        assert!(prompt.contains("Melatonin"), "artifact reached the delegated judge");
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

    // ---- Chapter Keystone — mission-level artifact grounding ----------------

    fn artifact_plan() -> MissionPlan {
        MissionPlan::new(
            "Write foo.md into the workspace",
            vec![Step::delegate("write", "writer", "produce foo.md")],
        )
    }

    #[tokio::test]
    async fn keystone_rejects_a_mission_with_no_deliverable() {
        // The dogfood bug: the mission runs to "done" but the deliverable isn't
        // there. With verify_missions on and the judge returning FAIL, the
        // mission is flipped to Rejected instead of Done.
        let shared = SharedMissionState::new(team_domain().await);
        let mut deps = deps("FAIL — the requested file was never produced.");
        deps.verify_missions = true;
        deps.workspace_root = Some(std::env::temp_dir()); // grounding present
        let id = team_run(&shared, &deps, default_nonagon(), artifact_plan(), "m1")
            .await
            .unwrap();
        let rec = shared.snapshot(&id).unwrap();
        assert_eq!(rec.phase, TeamMissionPhase::Rejected);
        assert!(rec
            .halt_reason
            .as_deref()
            .unwrap_or("")
            .contains("deliverable not verified"));
    }

    #[tokio::test]
    async fn keystone_keeps_a_verified_mission_done() {
        let shared = SharedMissionState::new(team_domain().await);
        let mut deps = deps("PASS — foo.md is present with the requested content.");
        deps.verify_missions = true;
        deps.workspace_root = Some(std::env::temp_dir());
        let id = team_run(&shared, &deps, default_nonagon(), artifact_plan(), "m1")
            .await
            .unwrap();
        assert_eq!(shared.snapshot(&id).unwrap().phase, TeamMissionPhase::Done);
    }

    #[tokio::test]
    async fn keystone_off_is_byte_identical_done() {
        // verify_missions default false ⇒ pre-Keystone behavior (Done on step
        // completion), even with a FAIL-shaped provider line.
        let shared = SharedMissionState::new(team_domain().await);
        let deps = deps("FAIL — would reject if the gate ran");
        assert!(!deps.verify_missions);
        let id = team_run(&shared, &deps, default_nonagon(), artifact_plan(), "m1")
            .await
            .unwrap();
        assert_eq!(shared.snapshot(&id).unwrap().phase, TeamMissionPhase::Done);
    }

    /// Chapter Ballast — a per-mission token cap halts a runaway mission at a
    /// wave boundary, preserving the work already done. Each sub-turn reports
    /// ≥1000 tokens; the 500-token cap is clear after wave 1, so wave 2 never
    /// launches. (The first boundary check, with the meter at 0, lets wave 1
    /// run.)
    #[tokio::test]
    async fn per_mission_token_cap_halts_a_runaway_mission() {
        let shared = SharedMissionState::new(team_domain().await);
        let mut deps = deps("spend");
        deps.provider = Arc::new(FakeProvider {
            line: "spend".into(),
            usage: LlmUsage {
                input_tokens: 600,
                output_tokens: 400,
                ..Default::default()
            },
        });
        deps.mission_budget = aivyx_cost::MissionBudget {
            max_tokens: Some(500),
            max_usd: None,
        };
        let plan = MissionPlan::new(
            "runaway",
            vec![
                Step::delegate("a", "researcher", "p"),
                Step::delegate("b", "writer", "p").after(["a"]),
            ],
        );
        let id = team_run(&shared, &deps, default_nonagon(), plan, "halt1")
            .await
            .unwrap();
        let rec = shared.snapshot(&id).unwrap();
        assert_eq!(
            rec.phase,
            TeamMissionPhase::Halted,
            "mission halted on the token cap"
        );
        assert!(rec.outputs.contains_key("a"), "wave 1 work preserved");
        assert!(!rec.outputs.contains_key("b"), "wave 2 never ran");
        // Chapter Belay / backlog #10 — the halt reason is persisted (so
        // `team status` shows *why*, not a hardcoded "budget").
        assert!(
            rec.halt_reason.is_some(),
            "the halt reason should be persisted on the record"
        );
    }

    /// Chapter Ballast — with no cap set, the same multi-wave mission runs to
    /// completion (byte-identical to pre-Ballast).
    #[tokio::test]
    async fn no_mission_cap_runs_to_completion_even_with_spend() {
        let shared = SharedMissionState::new(team_domain().await);
        let mut deps = deps("spend");
        deps.provider = Arc::new(FakeProvider {
            line: "spend".into(),
            usage: LlmUsage {
                input_tokens: 9_000,
                output_tokens: 9_000,
                ..Default::default()
            },
        });
        // mission_budget left at default (unbounded).
        let plan = MissionPlan::new(
            "unbounded",
            vec![
                Step::delegate("a", "researcher", "p"),
                Step::delegate("b", "writer", "p").after(["a"]),
            ],
        );
        let id = team_run(&shared, &deps, default_nonagon(), plan, "nocap1")
            .await
            .unwrap();
        let rec = shared.snapshot(&id).unwrap();
        assert_eq!(rec.phase, TeamMissionPhase::Done);
        assert!(rec.outputs.contains_key("b"));
    }

    // ---- Chapter Ensemble: per-role backend resolution ----

    fn fake_provider() -> Arc<dyn LlmProvider> {
        Arc::new(FakeProvider { line: "x".into(), usage: LlmUsage::default() })
    }

    // ---- Chapter Belay: abort a running mission ----

    #[tokio::test]
    async fn anchor_request_abort_sets_the_armed_flag() {
        use std::sync::atomic::Ordering;
        let shared = SharedMissionState::new(team_domain().await);
        // No flag armed yet → request is a no-op.
        assert!(!shared.request_abort("m"));
        let flag = shared.arm_abort("m");
        assert!(!flag.load(Ordering::SeqCst));
        // Armed → request sets the flag the observer reads.
        assert!(shared.request_abort("m"));
        assert!(flag.load(Ordering::SeqCst));
        // Disarmed (drive ended) → request is a no-op again.
        shared.disarm_abort("m");
        assert!(!shared.request_abort("m"));
    }

    #[tokio::test]
    async fn anchor_abort_mission_rejects_non_running() {
        let shared = SharedMissionState::new(team_domain().await);
        // Unknown mission.
        assert!(matches!(
            abort_mission(&shared, "nope"),
            Err(MissionDriverError::NotFound(_))
        ));
        // A registered-but-not-executing mission can't be aborted this way.
        let plan = MissionPlan::new("g", vec![Step::delegate("a", "researcher", "p")]);
        register_mission(&shared, plan, "m", None).await.unwrap();
        assert!(matches!(
            abort_mission(&shared, "m"),
            Err(MissionDriverError::NotAbortable(..))
        ));
    }

    #[tokio::test]
    async fn anchor_observer_halts_when_aborted() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let shared = SharedMissionState::new(team_domain().await);
        let (tx, _rx) = mpsc::unbounded_channel();
        let flag = Arc::new(AtomicBool::new(false));
        let obs = RegistryObserver {
            shared,
            id: "m".into(),
            tx,
            budget_guard: None,
            abort: Some(Arc::clone(&flag)),
        };
        // Not aborted, no budget → no halt.
        assert!(obs.should_halt().is_none());
        // Operator abort → the runtime's wave-boundary check halts the mission.
        flag.store(true, Ordering::SeqCst);
        assert_eq!(obs.should_halt(), Some("aborted by operator".to_string()));
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn ensemble_resolves_only_overridden_members() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let default_provider = fake_provider();
        let mut m_model = member("a", "R", &[]);
        m_model.model = Some("fast-model".into());
        let mut m_url = member("b", "R", &[]);
        m_url.base_url = Some("http://gpu-b:11434".into());
        let m_none = member("c", "R", &[]);

        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = Arc::clone(&calls);
        let builder: Arc<
            dyn Fn(&str) -> Result<Arc<dyn LlmProvider>, String> + Send + Sync,
        > = Arc::new(move |_url| {
            calls2.fetch_add(1, Ordering::Relaxed);
            Ok(fake_provider())
        });

        let map = resolve_member_backends(
            &[m_model, m_url, m_none],
            &default_provider,
            "default-model",
            Some(&builder),
        )
        .unwrap();

        // Only the two overridden members are present; the plain one isn't.
        assert_eq!(map.len(), 2);
        assert!(!map.contains_key("c"));
        // model-only → keeps default provider, overrides the model.
        assert_eq!(map["a"].model, "fast-model");
        // base_url-only → built provider (builder called once), default model.
        assert_eq!(map["b"].model, "default-model");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn ensemble_base_url_falls_back_when_no_builder() {
        // base_url set but no builder wired → shared provider, model override kept.
        let default_provider = fake_provider();
        let mut m = member("a", "R", &[]);
        m.base_url = Some("http://x".into());
        m.model = Some("mm".into());
        let map =
            resolve_member_backends(&[m], &default_provider, "dm", None).unwrap();
        assert_eq!(map["a"].model, "mm");
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
    async fn reload_reconciles_an_executing_zombie_to_halted() {
        // Chapter Reckon — a mission left `Executing` when the daemon stopped is
        // reconciled to a truthful `Halted` on reload, not left a permanent
        // zombie; and it stays that way (persisted).
        let store = team_domain().await;
        let shared = SharedMissionState::new(store.clone());
        let plan = MissionPlan::new("goal", vec![Step::delegate("a", "writer", "p")]);
        register_mission(&shared, plan, "z1", Some(default_nonagon()))
            .await
            .unwrap();
        let mut rec = shared.snapshot("z1").unwrap();
        rec.phase = TeamMissionPhase::Executing; // simulate mid-flight
        shared.put(rec).await.unwrap();

        // A fresh process reloads → the zombie becomes Halted with a reason.
        let reloaded = SharedMissionState::new(store.clone());
        assert_eq!(reloaded.reload().await.unwrap(), 1);
        let got = reloaded.snapshot("z1").unwrap();
        assert_eq!(got.phase, TeamMissionPhase::Halted);
        assert!(got
            .halt_reason
            .as_deref()
            .unwrap_or("")
            .contains("interrupted by a daemon restart"));

        // Persisted: a second fresh reload still sees Halted (not re-zombied).
        let again = SharedMissionState::new(store);
        again.reload().await.unwrap();
        assert_eq!(again.snapshot("z1").unwrap().phase, TeamMissionPhase::Halted);
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

    fn member(name: &str, role: &str, scopes: &[&str]) -> TeamMember {
        TeamMember {
            name: name.into(),
            role: role.into(),
            soul: format!("You are the {role}."),
            tool_allowlist: vec![],
            capability_scopes: scopes.iter().map(|s| s.to_string()).collect(),
            trust_ceiling: TrustTier::Trusted,
            model: None,
            base_url: None,
        }
    }

    /// A two-role vertical pack (a stand-in kitchen BOH team).
    fn custom_team() -> TeamConfig {
        TeamConfig {
            name: "boh".into(),
            description: "kitchen back-of-house".into(),
            lead: "chef".into(),
            members: vec![
                member("chef", "Lead", &["team.delegate"]),
                member("line", "Cook", &[]),
            ],
            dialogue: Default::default(),
        }
    }

    #[tokio::test]
    async fn start_pins_persists_and_resumes_a_vertical_pack() {
        let store = team_domain().await;
        // The service default is the Nonagon, but this mission pins a pack.
        let svc = TeamMissionService::new(
            SharedMissionState::new(store.clone()),
            deps("ok"),
            default_nonagon(),
            GatePolicy::Interactive,
        );
        let plan = MissionPlan::new("prep", vec![Step::delegate("a", "line", "chop")]);
        let id = svc.start(plan, Some(custom_team())).await.unwrap();
        wait_for(&svc, &id, TeamMissionPhase::Done).await;

        let rec = svc.snapshot(&id).unwrap();
        assert_eq!(rec.config.as_ref().unwrap().lead, "chef", "pack persisted on the record");
        assert_eq!(rec.outputs["a"], "ok", "ran on the pack's specialist, not the Nonagon");
        assert_eq!(rec.to_view().lead, "chef", "the feed shows the pack's lead");

        // The pin survives a reload — a resume after restart uses the pack.
        let reloaded = SharedMissionState::new(store);
        reloaded.reload().await.unwrap();
        assert_eq!(reloaded.snapshot(&id).unwrap().config.unwrap().lead, "chef");
    }

    #[tokio::test]
    async fn default_team_missions_have_no_pinned_config() {
        let svc = TeamMissionService::new(
            SharedMissionState::new(team_domain().await),
            deps("ok"),
            default_nonagon(),
            GatePolicy::Interactive,
        );
        let plan = MissionPlan::new("g", vec![Step::delegate("a", "writer", "p")]);
        let id = svc.start(plan, None).await.unwrap();
        wait_for(&svc, &id, TeamMissionPhase::Done).await;
        let rec = svc.snapshot(&id).unwrap();
        assert!(rec.config.is_none(), "no pack → no pinned config");
        assert_eq!(rec.to_view().lead, "coordinator", "feed falls back to the Nonagon lead");
    }

    #[tokio::test]
    async fn team_config_accessor_returns_the_active_roster() {
        let svc = TeamMissionService::new(
            SharedMissionState::new(team_domain().await),
            deps("ok"),
            default_nonagon(),
            GatePolicy::Interactive,
        );
        let cfg = svc.team_config();
        assert_eq!(cfg, default_nonagon(), "accessor returns the assembled roster");
        assert!(!cfg.members.is_empty());
        assert!(
            cfg.members.iter().any(|m| m.name == cfg.lead),
            "the lead is one of the members",
        );
    }

    #[tokio::test]
    async fn service_start_spawns_a_drive_that_pauses_then_resumes() {
        let svc = TeamMissionService::new(
            SharedMissionState::new(team_domain().await),
            deps("ok"),
            default_nonagon(),
            GatePolicy::Interactive,
        );
        // start returns immediately with a fresh id; the drive runs in the bg.
        let id = svc.start(gated_plan(), None).await.unwrap();
        assert_eq!(svc.list().len(), 1);

        wait_for(&svc, &id, TeamMissionPhase::AwaitingApproval).await;
        assert_eq!(svc.snapshot(&id).unwrap().pending_gate.as_deref(), Some("approve"));

        // resolve(approve) flips to Executing synchronously, then drives to Done.
        let phase = svc.resolve(&id, "approve", true).await.unwrap();
        assert_eq!(phase, TeamMissionPhase::Executing);
        wait_for(&svc, &id, TeamMissionPhase::Done).await;
        assert_eq!(svc.snapshot(&id).unwrap().outputs["write"], "ok");
    }

    #[tokio::test]
    async fn headless_run_rejects_at_a_human_gate() {
        // Chapter H — a headless service rejects (not parks) at a human gate.
        let svc = TeamMissionService::new(
            SharedMissionState::new(team_domain().await),
            deps("ok"),
            default_nonagon(),
            GatePolicy::RejectAndAbort,
        );
        let id = svc.start(gated_plan(), None).await.unwrap();
        wait_for(&svc, &id, TeamMissionPhase::Rejected).await;

        let rec = svc.snapshot(&id).unwrap();
        assert!(rec.pending_gate.is_none(), "headless never leaves a pending gate");
        assert!(rec.outputs.contains_key("research"), "upstream work still ran");
        assert!(rec.outputs["approve"].starts_with("rejected"), "the human gate auto-rejected");
        assert!(!rec.outputs.contains_key("write"), "the gated dependent never ran");
    }

    #[tokio::test]
    async fn headless_team_refusal_lands_on_the_audit_chain() {
        // H.6 — the headless gate refusal must also reach the audit chain
        // (the unattended path stays as legible as an operator-resolved gate),
        // carrying the gated step + a reason.
        let audit = Arc::new(CapturingAuditHook::default());
        let svc = TeamMissionService::new(
            SharedMissionState::new(team_domain().await),
            deps_with_audit("ok", audit.clone()),
            default_nonagon(),
            GatePolicy::RejectAndAbort,
        );
        let id = svc.start(gated_plan(), None).await.unwrap();
        wait_for(&svc, &id, TeamMissionPhase::Rejected).await;

        let refusals = audit.refusals.lock().unwrap();
        assert_eq!(refusals.len(), 1, "exactly one refusal audited");
        assert_eq!(refusals[0].0, "approve", "the gated step id is recorded");
        assert!(
            refusals[0].1.contains("approve") && refusals[0].1.contains("headless"),
            "the reason names the step and why it was refused"
        );
    }

    // ---- team.run tool (L.7) ------------------------------------------

    /// A one-step plan the fake provider returns for a `team.run` decomposition.
    const TOOL_PLAN_JSON: &str =
        r#"{"goal":"g","steps":[{"id":"a","specialist":"writer","prompt":"draft"}]}"#;

    fn tool_ctx_parts() -> (MissionLeadChannel, CancellationToken) {
        (MissionLeadChannel::new(), CancellationToken::new())
    }

    fn tool_ctx<'a>(
        ch: &'a MissionLeadChannel,
        token: &'a CancellationToken,
        audit: &'a NullAuditHook,
    ) -> ToolContext<'a> {
        ToolContext {
            agent_id: aivyx_core::AgentId::new(),
            session_id: ch.session,
            turn_id: aivyx_core::TurnId::new(),
            channel: ch,
            audit,
            cancellation: token,
        }
    }

    #[test]
    fn team_run_tool_name_and_scope() {
        let tool = TeamRunTool::new();
        assert_eq!(tool.name(), "team.run");
        assert_eq!(tool.required_scope(&json!({})).base(), "team.run");
    }

    #[tokio::test]
    async fn team_run_tool_starts_a_mission_via_the_service() {
        let svc = TeamMissionService::new(
            SharedMissionState::new(team_domain().await),
            deps(TOOL_PLAN_JSON),
            default_nonagon(),
            GatePolicy::Interactive,
        );
        let tool = TeamRunTool::new();
        assert!(tool.set_service(svc.clone()));

        let (ch, token) = tool_ctx_parts();
        let audit = NullAuditHook;
        let ctx = tool_ctx(&ch, &token, &audit);
        let out = tool.execute(json!({ "goal": "do the big thing" }), &ctx).await;
        match out {
            ToolOutcome::Completed { output, .. } => {
                assert!(output["mission_id"].as_str().is_some(), "returns a mission id");
                assert_eq!(output["status"], "started");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
        assert_eq!(svc.list().len(), 1, "the mission was registered on the service");
    }

    #[tokio::test]
    async fn team_run_tool_without_a_service_fails_cleanly() {
        let tool = TeamRunTool::new(); // no set_service
        let (ch, token) = tool_ctx_parts();
        let audit = NullAuditHook;
        let ctx = tool_ctx(&ch, &token, &audit);
        assert!(matches!(
            tool.execute(json!({ "goal": "x" }), &ctx).await,
            ToolOutcome::Failed(_)
        ));
    }

    #[tokio::test]
    async fn team_run_tool_rejects_an_empty_goal() {
        let svc = TeamMissionService::new(
            SharedMissionState::new(team_domain().await),
            deps(TOOL_PLAN_JSON),
            default_nonagon(),
            GatePolicy::Interactive,
        );
        let tool = TeamRunTool::new();
        assert!(tool.set_service(svc));
        let (ch, token) = tool_ctx_parts();
        let audit = NullAuditHook;
        let ctx = tool_ctx(&ch, &token, &audit);
        assert!(matches!(
            tool.execute(json!({ "goal": "  " }), &ctx).await,
            ToolOutcome::Failed(_)
        ));
    }
}

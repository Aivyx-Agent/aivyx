//! `aivyx team` CLI — Chapter J (the Nonagon).
//!
//! Two subcommands:
//!
//! - `aivyx team roster` — render the default Nonagon (the 9 roles + their
//!   scopes/trust). Offline: pure rendering, no provider, no storage.
//! - `aivyx team run "<mission>"` — assemble the team in-process and hand the
//!   mission to the **lead** agent. The lead drives via its orchestration
//!   tools (`decompose_task` → delegate → `verify`/`synthesize`); every
//!   specialist sub-turn is built by the [`SpecialistPool`] over the **same
//!   `AuditHook`**, so the whole run lands on the one HMAC chain. Wired from
//!   `run_async` (which owns the live provider + persistent audit).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aivyx_capability::TrustTier;
use aivyx_core::{
    Agent, AgentId, AuditHook, CancellationToken, ChannelContext, ChannelError, ChannelPlatform,
    ConcreteAgent, LlmPlanner, LlmPlannerConfig, Message, SessionId, StreamEvent, Tool,
    ToolRegistry, TurnOutcome,
};
use aivyx_llm::LlmProvider;
use aivyx_team::{default_nonagon, TeamAssembly, TeamConfig};
use async_trait::async_trait;

/// Render the team roster as an operator-readable block. Pure — the unit of
/// `aivyx team roster`.
pub fn render_roster(config: &TeamConfig) -> String {
    let specialists = config.specialists().count();
    let mut out = format!(
        "Team: {} — {}\n  lead: {} ({} specialist{})\n",
        config.name,
        if config.description.is_empty() { "(no description)" } else { &config.description },
        config.lead,
        specialists,
        if specialists == 1 { "" } else { "s" },
    );
    for m in &config.members {
        let tag = if m.name == config.lead { "lead " } else { "spec " };
        let scopes = if m.capability_scopes.is_empty() {
            "(none)".to_string()
        } else {
            m.capability_scopes.join(", ")
        };
        out.push_str(&format!(
            "  [{tag}] {:<12} {:<24} trust={}\n             scopes: {scopes}\n",
            m.name,
            m.role,
            trust_label(m.trust_ceiling),
        ));
    }
    out
}

fn trust_label(t: TrustTier) -> &'static str {
    match t {
        TrustTier::Untrusted => "Untrusted",
        TrustTier::SemiTrusted => "SemiTrusted",
        TrustTier::Trusted => "Trusted",
        TrustTier::Kernel => "Kernel",
    }
}

/// Load the team to run: a vertical pack's `TeamConfig` from `--config
/// <path.toml>`, or the default 9-role Nonagon when none is given.
fn load_team(config: Option<&str>) -> Result<TeamConfig, String> {
    match config {
        Some(path) => TeamConfig::load(path)
            .map_err(|e| format!("failed to load team config from {path:?}: {e}")),
        None => Ok(default_nonagon()),
    }
}

/// Resolve the daemon's startup team (Chapter Roster RO.1). Resolution order:
///
/// 1. `[team] config_path` set → load that file (a relative path is resolved
///    against `base_dir`, the directory of the loaded `aivyx.toml`).
/// 2. unset → the conventional `team.toml` beside `aivyx.toml`, if it exists.
/// 3. neither → the built-in [`default_nonagon`] (byte-identical to the
///    pre-RO.1 daemon).
///
/// Never errors: a configured-but-broken (or unparseable conventional) file
/// logs a warning and falls back to the default Nonagon so the daemon still
/// boots. The write path ([Chapter U] machinery, RO.2) guarantees a valid
/// file; this reader is the defensive boot half.
pub fn resolve_daemon_team_config(configured: Option<&Path>, base_dir: &Path) -> TeamConfig {
    // Which file to try — the configured path (resolved) or the conventional
    // `team.toml`. An unset-and-absent conventional file means "no file".
    let candidate: Option<PathBuf> = match configured {
        Some(p) if p.is_absolute() => Some(p.to_path_buf()),
        Some(p) => Some(base_dir.join(p)),
        None => {
            let conventional = base_dir.join("team.toml");
            conventional.exists().then_some(conventional)
        }
    };
    let Some(path) = candidate else {
        return default_nonagon();
    };
    match TeamConfig::load(&path) {
        Ok(cfg) => {
            eprintln!("aivyx team: loaded team config from {}", path.display());
            cfg
        }
        Err(e) => {
            eprintln!(
                "aivyx team: WARNING — failed to load team config {} ({e}); \
                 falling back to the default Nonagon",
                path.display()
            );
            default_nonagon()
        }
    }
}

/// `aivyx team roster [--config <path>]` — print a team. Offline.
pub fn run_roster(config: Option<&str>) -> Result<(), String> {
    print!("{}", render_roster(&load_team(config)?));
    Ok(())
}

/// `aivyx team run "<mission>"` — assemble the default team and run the lead
/// over `mission`. Called from `run_async` with the live provider + the
/// persistent `AuditHook`, so specialist sub-turns land on the HMAC chain.
#[allow(clippy::too_many_arguments)]
pub async fn run_mission(
    provider: Arc<dyn LlmProvider>,
    model: &str,
    max_tokens: u32,
    audit: Arc<dyn AuditHook>,
    base_tools: Vec<Arc<dyn Tool>>,
    mission: &str,
    config: Option<&str>,
) -> Result<(), String> {
    let config = load_team(config)?;
    let team_name = config.name.clone();
    let lead = config
        .lead_member()
        .ok_or("team has no lead")?
        .clone();
    // The team runs under the lead's declared authority; every specialist is
    // attenuated to a subset of it (NT-02). It grants team.delegate +
    // team.message, so the lead's orchestration/dialogue tools are callable.
    let lead_caps = lead.declared_capabilities().map_err(|e| e.to_string())?;

    let assembly = TeamAssembly::build(
        config,
        Arc::clone(&provider),
        model,
        max_tokens,
        Arc::clone(&audit),
        // The daemon's full tool set. Each specialist gets exactly the subset
        // its `tool_allowlist` names (least privilege), capability-attenuated
        // against the lead (NT-02). The lead itself stays orchestration-only.
        // (A vertical's *domain* tools — e.g. the kitchen toolkit's RPCs —
        // join this set once that toolkit crate is wired in.)
        base_tools,
        lead_caps.clone(),
    )
    .map_err(|e| format!("failed to assemble team: {e}"))?;

    let registry = Arc::new(ToolRegistry::new(assembly.lead_tools()));
    let planner_provider = Arc::clone(&provider);
    let planner_registry = Arc::clone(&registry);
    let model_owned = model.to_string();
    let soul = lead.soul.clone();
    let agent = ConcreteAgent::new(
        AgentId::new(),
        lead_caps,
        registry,
        audit,
        move || {
            let cfg = LlmPlannerConfig::new(&model_owned)
                .with_system_prompt(&soul)
                .with_max_tokens(max_tokens);
            Box::new(LlmPlanner::new(
                Arc::clone(&planner_provider),
                Arc::clone(&planner_registry),
                cfg,
            ))
        },
    );

    let channel = MissionChannel::new();
    let msg = Message::text(channel.session_id(), mission);
    eprintln!("team: running mission on {} (lead: {})…", team_name, lead.name);
    match agent.turn(msg, &channel).await {
        TurnOutcome::Completed { final_message, .. } => {
            println!("{final_message}");
            Ok(())
        }
        TurnOutcome::Escalated { reason, .. } => {
            Err(format!("team mission escalated for approval: {reason}"))
        }
        TurnOutcome::TimedOut { .. } => Err("team mission timed out".to_string()),
        TurnOutcome::Cancelled { .. } => Err("team mission was cancelled".to_string()),
        _ => Err("team mission did not complete".to_string()),
    }
}

/// A one-shot operator-facing channel for a `team run`: a fresh session at the
/// local Trusted tier. Streaming the lead's progress to the operator is the
/// J.7 Fleet panel; here we just collect the final deliverable.
struct MissionChannel {
    session: SessionId,
    token: CancellationToken,
}

impl MissionChannel {
    fn new() -> Self {
        MissionChannel {
            session: SessionId::new(),
            token: CancellationToken::new(),
        }
    }
}

#[async_trait]
impl ChannelContext for MissionChannel {
    fn channel_name(&self) -> &str {
        "team"
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

    #[test]
    fn roster_renders_the_default_nonagon() {
        let out = render_roster(&default_nonagon());
        // Header: 9 roles → 8 specialists, coordinator lead.
        assert!(out.contains("Team: default-nonagon"));
        assert!(out.contains("lead: coordinator (8 specialists)"));
        // Every role is listed.
        for name in [
            "coordinator", "researcher", "analyst", "coder", "writer", "reviewer", "planner",
            "ops", "archivist",
        ] {
            assert!(out.contains(name), "roster missing {name}");
        }
        // The lead is tagged distinctly and shows its orchestration scope.
        assert!(out.contains("[lead ]"));
        assert!(out.contains("[spec ]"));
        assert!(out.contains("team.delegate"));
        // J.5 roster wiring: every member can talk on the bus.
        assert!(out.contains("team.message"));
    }

    #[test]
    fn load_team_defaults_to_the_nonagon_and_errors_on_a_bad_path() {
        // No --config → the default 9-role Nonagon.
        assert_eq!(load_team(None).unwrap().lead, "coordinator");
        // A missing pack path is a clean error, not a panic.
        let err = load_team(Some("/no/such/team.toml")).unwrap_err();
        assert!(err.contains("failed to load team config"), "error: {err}");
    }

    // --- Chapter Roster (RO.1): the daemon's load-or-default resolver --------

    /// A unique scratch dir under the system temp root (no tempfile dev-dep).
    fn scratch(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "aivyx-roster-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Write a distinguishable (non-default) `[team]`-rooted file. Returns the path.
    fn write_custom_team(path: &Path) {
        use aivyx_team::config::{DialogueConfig, TeamConfig, TeamMember};
        let m = |name: &str| TeamMember {
            name: name.into(),
            role: "R".into(),
            soul: "s".into(),
            tool_allowlist: vec![],
            capability_scopes: vec![],
            trust_ceiling: TrustTier::Trusted,
        };
        let cfg = TeamConfig {
            name: "custom-team".into(),
            description: String::new(),
            lead: "boss".into(),
            members: vec![m("boss"), m("helper")],
            dialogue: DialogueConfig::default(),
        };
        std::fs::write(path, cfg.to_toml().unwrap()).unwrap();
    }

    #[test]
    fn resolve_unset_and_no_file_is_the_default_nonagon() {
        let dir = scratch("none");
        let team = resolve_daemon_team_config(None, &dir);
        assert_eq!(team.name, "default-nonagon");
        assert_eq!(team.lead, "coordinator");
    }

    #[test]
    fn resolve_unset_loads_the_conventional_team_toml() {
        let dir = scratch("conventional");
        write_custom_team(&dir.join("team.toml"));
        let team = resolve_daemon_team_config(None, &dir);
        assert_eq!(team.name, "custom-team");
        assert_eq!(team.lead, "boss");
    }

    #[test]
    fn resolve_configured_relative_path_is_joined_to_base_dir() {
        let dir = scratch("relative");
        write_custom_team(&dir.join("my-team.toml"));
        let team = resolve_daemon_team_config(Some(Path::new("my-team.toml")), &dir);
        assert_eq!(team.name, "custom-team");
    }

    #[test]
    fn resolve_configured_absolute_path_ignores_base_dir() {
        let dir = scratch("absolute");
        let elsewhere = scratch("absolute-target");
        let abs = elsewhere.join("packed.toml");
        write_custom_team(&abs);
        let team = resolve_daemon_team_config(Some(&abs), &dir);
        assert_eq!(team.name, "custom-team");
    }

    #[test]
    fn resolve_configured_broken_path_falls_back_to_default_not_panic() {
        let dir = scratch("broken");
        let team = resolve_daemon_team_config(Some(Path::new("/no/such/team.toml")), &dir);
        assert_eq!(team.name, "default-nonagon");
    }

    #[test]
    fn roster_handles_a_single_specialist_plural() {
        use aivyx_team::config::{DialogueConfig, TeamConfig, TeamMember};
        let m = |name: &str| TeamMember {
            name: name.into(),
            role: "R".into(),
            soul: "s".into(),
            tool_allowlist: vec![],
            capability_scopes: vec![],
            trust_ceiling: TrustTier::Trusted,
        };
        let cfg = TeamConfig {
            name: "duo".into(),
            description: String::new(),
            lead: "lead".into(),
            members: vec![m("lead"), m("helper")],
            dialogue: DialogueConfig::default(),
        };
        let out = render_roster(&cfg);
        assert!(out.contains("1 specialist)"), "singular, not '1 specialists'");
        assert!(out.contains("(no description)"));
        assert!(out.contains("scopes: (none)"));
    }
}

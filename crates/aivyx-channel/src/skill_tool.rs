//! Phase 184 — `skills.teach` / `skills.update` / `skills.forget`.
//!
//! Channel-tier tools that let the End User **author** skills
//! conversationally. Each reads the current `LearnedSkill` set
//! (the injected effective-persona snapshot), builds the
//! appropriate Persona-chain delta(s) via [`crate::skill_edit`],
//! and appends them — saving the skill directly, because the
//! operator chose "draft-in-chat, save on confirmation."
//!
//! The confirmation is a **safe-by-contract** part of the input:
//! every edit requires `confirmed: true`, which the agent is
//! instructed to set only after it has shown the drafted skill to
//! the operator and they approved. Combined with Trusted-tier-only
//! (the `skills.write` base), the HMAC persona-chain audit, and
//! `aivyx persona revert`, a save is a visible, reversible event.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

use crate::persona::{
    recompute_shared_from_entries, synthesize_delta_id, LearnedSkill,
    PersonaDelta, PersonaDeltaCategory, PersonaDeltaOp,
    PersistentPersonaLog, SharedEffectivePersona,
};
use crate::skill_edit::{
    find_skill_by_name, forget_op, merged_skill, teach_op, update_ops,
    validate_skill_name,
};

/// Shared persona-chain handle + effective-persona snapshot the
/// edit tools need.
pub type SharedPersonaLog = Arc<PersistentPersonaLog>;

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The agent must set `confirmed: true` only after the operator
/// approved the drafted skill.
const CONFIRM_HINT: &str =
    "set `confirmed: true` only AFTER you have shown the drafted \
     skill to the operator and they explicitly approved it. Draft \
     it, show name + when-to-use + steps, ask, then call again.";

fn fail(id: ToolId, detail: impl Into<String>) -> ToolOutcome {
    ToolOutcome::Failed(AivyxError::Tool {
        tool: id,
        detail: detail.into(),
    })
}

fn is_confirmed(input: &Value) -> bool {
    input.get("confirmed").and_then(|v| v.as_bool()) == Some(true)
}

/// Parse the current skill set from the effective-persona
/// snapshot (each `learned_skills` entry is a `LearnedSkill` JSON).
///
/// `pub(crate)` so Chapter Tutor's operator-authoring path
/// (`daemon_server::author_skill_live`) reads the same skill set the agent
/// tools do.
pub(crate) fn current_skills(effective: &SharedEffectivePersona) -> Vec<LearnedSkill> {
    effective
        .read()
        .ok()
        .map(|p| {
            p.learned_skills
                .iter()
                .filter_map(|s| LearnedSkill::from_json_value(s))
                .collect()
        })
        .unwrap_or_default()
}

/// Append the ops as a single operator-authored "proposal" worth
/// of `LearnedSkill` deltas, then recompute the effective persona.
///
/// Returns the chain seq of the **last** appended delta. `pub(crate)` so
/// Chapter Tutor's operator-authoring path reuses the exact same append +
/// recompute the agent skill tools use (operator- and agent-authored skills
/// land identically on the chain).
pub(crate) async fn commit_ops(
    persona_log: &SharedPersonaLog,
    effective: &SharedEffectivePersona,
    ops: &[PersonaDeltaOp],
) -> Result<u64, String> {
    let proposal_id = format!("skill-edit:{}", uuid::Uuid::new_v4());
    let now_ms = now_unix_ms();
    let mut last_seq = 0u64;
    for (idx, op) in ops.iter().enumerate() {
        let delta = PersonaDelta {
            delta_id: synthesize_delta_id(
                &proposal_id,
                PersonaDeltaCategory::LearnedSkill,
                op,
                idx as u32,
            ),
            proposed_at_unix_ms: now_ms,
            approved_at_unix_ms: now_ms,
            proposal_id: proposal_id.clone(),
            category: PersonaDeltaCategory::LearnedSkill,
            op: op.clone(),
        };
        last_seq = persona_log
            .append(delta)
            .await
            .map_err(|e| e.to_string())?;
    }
    let entries = persona_log.entries();
    recompute_shared_from_entries(effective, &entries);
    Ok(last_seq)
}

/// Common injection slots — every edit tool needs the chain + the
/// effective snapshot.
struct Deps {
    persona_log: OnceLock<SharedPersonaLog>,
    effective: OnceLock<SharedEffectivePersona>,
}

impl Deps {
    fn new() -> Self {
        Self {
            persona_log: OnceLock::new(),
            effective: OnceLock::new(),
        }
    }
    fn get(&self) -> Option<(&SharedPersonaLog, &SharedEffectivePersona)> {
        Some((self.persona_log.get()?, self.effective.get()?))
    }
}

macro_rules! skill_tool {
    ($ty:ident, $name:literal) => {
        pub struct $ty {
            id: ToolId,
            schema: Value,
            deps: Deps,
        }
        impl std::fmt::Debug for $ty {
            fn fmt(
                &self,
                f: &mut std::fmt::Formatter<'_>,
            ) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish()
            }
        }
        impl Default for $ty {
            fn default() -> Self {
                Self::new()
            }
        }
        impl $ty {
            pub fn set_persona_log(
                &self,
                log: SharedPersonaLog,
            ) -> Result<(), SharedPersonaLog> {
                self.deps.persona_log.set(log)
            }
            pub fn set_effective_persona(
                &self,
                eff: SharedEffectivePersona,
            ) -> Result<(), SharedEffectivePersona> {
                self.deps.effective.set(eff)
            }
        }
    };
}

skill_tool!(SkillTeachTool, "skills.teach");
skill_tool!(SkillUpdateTool, "skills.update");
skill_tool!(SkillForgetTool, "skills.forget");

// ---------------------------------------------------------------------------
// skills.teach
// ---------------------------------------------------------------------------

impl SkillTeachTool {
    pub fn new() -> Self {
        SkillTeachTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description":
                        "kebab-case id, e.g. `code.review-checklist`." },
                    "trigger": { "type": "string", "description":
                        "When this skill applies (one or two sentences)." },
                    "procedure": { "type": "string", "description":
                        "The steps / instructions to follow." },
                    "confirmed": { "type": "boolean", "description":
                        CONFIRM_HINT }
                },
                "required": ["name", "trigger", "procedure", "confirmed"]
            }),
            deps: Deps::new(),
        }
    }
}

#[async_trait]
impl Tool for SkillTeachTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "skills.teach"
    }
    fn description(&self) -> &str {
        "Save a new skill the operator taught you. First draft it \
         and show name + when-to-use + steps to the operator; only \
         call this with `confirmed: true` after they approve. Input: \
         `{ name, trigger, procedure, confirmed }`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    // skills.write is never auto-granted -- the write half of the
    // skills surface (SkillTeachTool/SkillUpdateTool/SkillForgetTool, all
    // three) stays auto-proposer / role-declared, unlike skills.list/
    // skills.invoke (both in the floor's fixed baseline).
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("skills.write").expect("known base")
    }

    async fn execute(
        &self,
        input: Value,
        _ctx: &ToolContext<'_>,
    ) -> ToolOutcome {
        let Some((log, eff)) = self.deps.get() else {
            return fail(self.id, "skills.teach has no persona chain injected");
        };
        if !is_confirmed(&input) {
            return fail(self.id, CONFIRM_HINT);
        }
        let name = input.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if let Err(e) = validate_skill_name(name) {
            return fail(self.id, e);
        }
        let name = name.trim();
        let trigger = input.get("trigger").and_then(|v| v.as_str()).unwrap_or("").trim();
        let procedure = input.get("procedure").and_then(|v| v.as_str()).unwrap_or("").trim();
        if trigger.is_empty() || procedure.is_empty() {
            return fail(self.id, "`trigger` and `procedure` must be non-empty");
        }
        let skills = current_skills(eff);
        if find_skill_by_name(&skills, name).is_some() {
            return fail(
                self.id,
                format!("skill {name:?} already exists — use skills.update"),
            );
        }
        let skill = LearnedSkill {
            name: name.to_string(),
            trigger: trigger.to_string(),
            procedure: procedure.to_string(),
            ..Default::default()
        };
        match commit_ops(log, eff, &[teach_op(&skill)]).await {
            Ok(_) => ToolOutcome::Completed {
                output: json!({ "taught": name }),
                verified: Verification::NotApplicable,
            },
            Err(e) => fail(self.id, format!("failed to save skill: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// skills.update
// ---------------------------------------------------------------------------

impl SkillUpdateTool {
    pub fn new() -> Self {
        SkillUpdateTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description":
                        "The existing skill to refine." },
                    "trigger": { "type": "string", "description":
                        "New when-to-use (omit to keep)." },
                    "procedure": { "type": "string", "description":
                        "New steps (omit to keep)." },
                    "confirmed": { "type": "boolean", "description":
                        CONFIRM_HINT }
                },
                "required": ["name", "confirmed"]
            }),
            deps: Deps::new(),
        }
    }
}

#[async_trait]
impl Tool for SkillUpdateTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "skills.update"
    }
    fn description(&self) -> &str {
        "Refine an existing skill (trigger and/or procedure), \
         keeping its name. Draft the change, show it, and only call \
         with `confirmed: true` after the operator approves. Input: \
         `{ name, trigger?, procedure?, confirmed }`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("skills.write").expect("known base")
    }

    async fn execute(
        &self,
        input: Value,
        _ctx: &ToolContext<'_>,
    ) -> ToolOutcome {
        let Some((log, eff)) = self.deps.get() else {
            return fail(self.id, "skills.update has no persona chain injected");
        };
        if !is_confirmed(&input) {
            return fail(self.id, CONFIRM_HINT);
        }
        let name = input.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
        let new_trigger = input.get("trigger").and_then(|v| v.as_str());
        let new_procedure = input.get("procedure").and_then(|v| v.as_str());
        if new_trigger.is_none() && new_procedure.is_none() {
            return fail(self.id, "provide a new `trigger` and/or `procedure`");
        }
        let skills = current_skills(eff);
        let Some(old) = find_skill_by_name(&skills, name) else {
            return fail(self.id, not_found(name, &skills));
        };
        let new = merged_skill(old, new_trigger, new_procedure);
        match commit_ops(log, eff, &update_ops(old, &new)).await {
            Ok(_) => ToolOutcome::Completed {
                output: json!({ "updated": name }),
                verified: Verification::NotApplicable,
            },
            Err(e) => fail(self.id, format!("failed to update skill: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// skills.forget
// ---------------------------------------------------------------------------

impl SkillForgetTool {
    pub fn new() -> Self {
        SkillForgetTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description":
                        "The skill to forget." },
                    "confirmed": { "type": "boolean", "description":
                        CONFIRM_HINT }
                },
                "required": ["name", "confirmed"]
            }),
            deps: Deps::new(),
        }
    }
}

#[async_trait]
impl Tool for SkillForgetTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "skills.forget"
    }
    fn description(&self) -> &str {
        "Forget (remove) a skill. Confirm with the operator first; \
         call with `confirmed: true` only after they approve. Input: \
         `{ name, confirmed }`. Recoverable via `aivyx persona`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("skills.write").expect("known base")
    }

    async fn execute(
        &self,
        input: Value,
        _ctx: &ToolContext<'_>,
    ) -> ToolOutcome {
        let Some((log, eff)) = self.deps.get() else {
            return fail(self.id, "skills.forget has no persona chain injected");
        };
        if !is_confirmed(&input) {
            return fail(self.id, CONFIRM_HINT);
        }
        let name = input.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
        let skills = current_skills(eff);
        let Some(old) = find_skill_by_name(&skills, name) else {
            return fail(self.id, not_found(name, &skills));
        };
        match commit_ops(log, eff, &[forget_op(old)]).await {
            Ok(_) => ToolOutcome::Completed {
                output: json!({ "forgot": name }),
                verified: Verification::NotApplicable,
            },
            Err(e) => fail(self.id, format!("failed to forget skill: {e}")),
        }
    }
}

fn not_found(name: &str, skills: &[LearnedSkill]) -> String {
    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    if names.is_empty() {
        format!("no skill named {name:?} (none are defined yet)")
    } else {
        format!(
            "no skill named {name:?}. Defined skills: {}",
            names.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::{shared_effective_persona, EffectivePersona};
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
            _e: StreamEvent<'_>,
        ) -> Result<(), ChannelError> {
            Ok(())
        }
        async fn finalize(
            &self,
            _o: &TurnOutcome,
        ) -> Result<(), ChannelError> {
            Ok(())
        }
        fn cancellation_token(&self) -> CancellationToken {
            self.token.clone()
        }
    }
    struct NoopAudit;
    impl aivyx_core::AuditHook for NoopAudit {
        fn on_event(&self, _t: aivyx_core::AuditTag) {}
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

    async fn persona_log() -> SharedPersonaLog {
        let dir = std::env::temp_dir()
            .join(format!("aivyx-skilltool-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let s: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("s.redb")),
            MasterKey::from_raw([5u8; 32]),
        )
        .await
        .unwrap();
        Arc::new(
            PersistentPersonaLog::open(
                s.domain(KeyDomain::Persona),
                b"k".to_vec(),
            )
            .await
            .unwrap(),
        )
    }

    fn eff() -> SharedEffectivePersona {
        shared_effective_persona(EffectivePersona::default())
    }

    fn wire<T>(tool: &T, log: &SharedPersonaLog, e: &SharedEffectivePersona)
    where
        T: SkillToolWiring,
    {
        let _ = tool.wire_log(Arc::clone(log));
        let _ = tool.wire_eff(Arc::clone(e));
    }
    // tiny shim so the test can wire all three uniformly
    trait SkillToolWiring {
        fn wire_log(&self, l: SharedPersonaLog) -> Result<(), SharedPersonaLog>;
        fn wire_eff(
            &self,
            e: SharedEffectivePersona,
        ) -> Result<(), SharedEffectivePersona>;
    }
    macro_rules! impl_wiring {
        ($t:ty) => {
            impl SkillToolWiring for $t {
                fn wire_log(
                    &self,
                    l: SharedPersonaLog,
                ) -> Result<(), SharedPersonaLog> {
                    self.set_persona_log(l)
                }
                fn wire_eff(
                    &self,
                    e: SharedEffectivePersona,
                ) -> Result<(), SharedEffectivePersona> {
                    self.set_effective_persona(e)
                }
            }
        };
    }
    impl_wiring!(SkillTeachTool);
    impl_wiring!(SkillUpdateTool);
    impl_wiring!(SkillForgetTool);

    #[tokio::test]
    async fn teach_update_forget_round_trip() {
        let log = persona_log().await;
        let e = eff();
        let teach = SkillTeachTool::new();
        wire(&teach, &log, &e);
        let update = SkillUpdateTool::new();
        wire(&update, &log, &e);
        let forget = SkillForgetTool::new();
        wire(&forget, &log, &e);
        let (ch, audit) = ctx_parts();
        let ctx = make_ctx(&ch, &audit);

        // teach
        let out = teach
            .execute(
                json!({ "name": "greet", "trigger": "on hello",
                        "procedure": "say hi", "confirmed": true }),
                &ctx,
            )
            .await;
        assert!(matches!(out, ToolOutcome::Completed { .. }));
        assert_eq!(current_skills(&e).len(), 1);
        assert_eq!(current_skills(&e)[0].procedure, "say hi");

        // update the procedure
        update
            .execute(
                json!({ "name": "greet", "procedure": "say hi warmly",
                        "confirmed": true }),
                &ctx,
            )
            .await;
        let now = current_skills(&e);
        assert_eq!(now.len(), 1);
        assert_eq!(now[0].procedure, "say hi warmly");
        assert_eq!(now[0].trigger, "on hello"); // kept

        // forget
        forget
            .execute(json!({ "name": "greet", "confirmed": true }), &ctx)
            .await;
        assert_eq!(current_skills(&e).len(), 0);
    }

    #[tokio::test]
    async fn unconfirmed_is_rejected() {
        let log = persona_log().await;
        let e = eff();
        let teach = SkillTeachTool::new();
        wire(&teach, &log, &e);
        let (ch, audit) = ctx_parts();
        let ctx = make_ctx(&ch, &audit);
        // confirmed omitted → rejected, nothing saved.
        let out = teach
            .execute(
                json!({ "name": "x", "trigger": "t", "procedure": "p" }),
                &ctx,
            )
            .await;
        assert!(matches!(out, ToolOutcome::Failed(_)));
        assert_eq!(current_skills(&e).len(), 0);
    }

    #[tokio::test]
    async fn teach_rejects_duplicate_update_and_forget_reject_missing() {
        let log = persona_log().await;
        let e = eff();
        let teach = SkillTeachTool::new();
        wire(&teach, &log, &e);
        let update = SkillUpdateTool::new();
        wire(&update, &log, &e);
        let forget = SkillForgetTool::new();
        wire(&forget, &log, &e);
        let (ch, audit) = ctx_parts();
        let ctx = make_ctx(&ch, &audit);

        teach
            .execute(
                json!({ "name": "a", "trigger": "t", "procedure": "p",
                        "confirmed": true }),
                &ctx,
            )
            .await;
        // duplicate teach → fail
        assert!(matches!(
            teach
                .execute(
                    json!({ "name": "a", "trigger": "t2",
                            "procedure": "p2", "confirmed": true }),
                    &ctx
                )
                .await,
            ToolOutcome::Failed(_)
        ));
        // update / forget a missing skill → fail
        assert!(matches!(
            update
                .execute(
                    json!({ "name": "nope", "procedure": "x",
                            "confirmed": true }),
                    &ctx
                )
                .await,
            ToolOutcome::Failed(_)
        ));
        assert!(matches!(
            forget
                .execute(json!({ "name": "nope", "confirmed": true }), &ctx)
                .await,
            ToolOutcome::Failed(_)
        ));
    }

    #[test]
    fn required_scope_is_skills_write() {
        assert_eq!(
            SkillTeachTool::new().required_scope(&json!({})).base(),
            "skills.write"
        );
        assert_eq!(
            SkillForgetTool::new().required_scope(&json!({})).base(),
            "skills.write"
        );
    }
}

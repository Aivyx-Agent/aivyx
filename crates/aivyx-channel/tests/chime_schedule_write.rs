//! Chapter Chime — the schedule WRITE half's governance rules, end to
//! end against a real (temp) store:
//!
//! - agent creations follow the Reins growth gradient (disabled pending
//!   approval below `policy_auto`; armed at `policy_auto`+),
//! - the 15-minute fire floor and the 10-schedule cap hold,
//! - the agent may only update/delete its own (`agt-`) schedules, and an
//!   update below `policy_auto` re-disables an approved schedule,
//! - the operator mutation rules: reserved prefixes, collisions, and the
//!   config-routine delete refusal.

use std::sync::Arc;

use aivyx_channel::schedule::{
    self, operator_create_schedule, operator_delete_schedule,
    operator_update_schedule, ScheduleProvenance, ScheduleRecord,
};
use aivyx_channel::schedule_tool::{
    ScheduleCreateTool, ScheduleDeleteTool, ScheduleUpdateTool,
};
use aivyx_config::GrowthAdoption;
use aivyx_core::{
    AgentId, AuditTag, ChannelPlatform, SessionId, StreamEvent, Tool,
    ToolContext, ToolOutcome, TurnId, TurnOutcome,
};
use aivyx_storage::{
    KeyDomain, RedbStorage, Storage, StorageConfig,
};
use serde_json::json;
use aivyx_core::CancellationToken;

async fn open_store() -> Arc<dyn Storage> {
    use aivyx_crypto::MasterKey;
    let base = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
    let dir = std::path::PathBuf::from(base)
        .join(format!("aivyx-chime-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    RedbStorage::open(
        StorageConfig::new(dir.join("store.redb")),
        MasterKey::from_raw([42u8; 32]),
    )
    .await
    .unwrap()
}

struct NoopChannel;
#[async_trait::async_trait]
impl aivyx_core::ChannelContext for NoopChannel {
    fn channel_name(&self) -> &str {
        "test"
    }
    fn platform(&self) -> ChannelPlatform {
        ChannelPlatform::Local
    }
    fn trust_tier(&self) -> aivyx_capability::TrustTier {
        aivyx_capability::TrustTier::Trusted
    }
    fn session_id(&self) -> SessionId {
        SessionId::new()
    }
    async fn stream_event(
        &self,
        _: StreamEvent<'_>,
    ) -> Result<(), aivyx_core::ChannelError> {
        Ok(())
    }
    async fn finalize(
        &self,
        _: &TurnOutcome,
    ) -> Result<(), aivyx_core::ChannelError> {
        Ok(())
    }
    fn cancellation_token(&self) -> CancellationToken {
        CancellationToken::new()
    }
}

struct NoopAudit;
impl aivyx_core::AuditHook for NoopAudit {
    fn on_event(&self, _: AuditTag) {}
}

/// A daily 09:00 cron in the 7-field form the `cron` crate parses.
const DAILY: &str = "0 0 9 * * * *";
/// Fires every minute — must trip the 15-minute floor.
const EVERY_MINUTE: &str = "0 * * * * * *";

fn ctx_parts() -> (NoopChannel, NoopAudit, CancellationToken) {
    (NoopChannel, NoopAudit, CancellationToken::new())
}

macro_rules! ctx {
    ($c:ident, $a:ident, $t:ident) => {
        ToolContext {
            agent_id: AgentId::new(),
            session_id: SessionId::new(),
            turn_id: TurnId::new(),
            channel: &$c,
            audit: &$a,
            cancellation: &$t,
        }
    };
}

#[tokio::test]
async fn agent_create_lands_disabled_below_policy_auto() {
    let store = open_store().await;
    let tool = ScheduleCreateTool::new();
    tool.set_schedule_store(store.domain(KeyDomain::Schedules)).unwrap();
    tool.set_growth(GrowthAdoption::ProposeOnly).unwrap();
    let (c, a, t) = ctx_parts();

    let out = tool
        .execute(json!({"cron": DAILY, "prompt": "check the weather"}), &ctx!(c, a, t))
        .await;
    let ToolOutcome::Completed { output, .. } = out else {
        panic!("expected Completed, got {out:?}");
    };
    assert_eq!(output["status"], "pending_approval");
    let id = output["schedule_id"].as_str().unwrap().to_string();
    assert!(id.starts_with("agt-"));

    let handle = store.domain(KeyDomain::Schedules);
    let rec = schedule::get_schedule(&handle, &id).await.unwrap().unwrap();
    assert!(!rec.enabled, "must land disabled pending approval");
    assert_eq!(rec.created_by, ScheduleProvenance::Agent);
}

#[tokio::test]
async fn agent_create_arms_directly_at_policy_auto() {
    let store = open_store().await;
    let tool = ScheduleCreateTool::new();
    tool.set_schedule_store(store.domain(KeyDomain::Schedules)).unwrap();
    tool.set_growth(GrowthAdoption::PolicyAuto).unwrap();
    let (c, a, t) = ctx_parts();

    let out = tool
        .execute(json!({"cron": DAILY, "prompt": "p"}), &ctx!(c, a, t))
        .await;
    let ToolOutcome::Completed { output, .. } = out else {
        panic!("expected Completed, got {out:?}");
    };
    assert_eq!(output["status"], "armed");
    let handle = store.domain(KeyDomain::Schedules);
    let rec = schedule::get_schedule(
        &handle,
        output["schedule_id"].as_str().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(rec.enabled);
}

#[tokio::test]
async fn growth_none_refuses_and_frequency_floor_holds() {
    let store = open_store().await;
    let (c, a, t) = ctx_parts();

    let refused = ScheduleCreateTool::new();
    refused.set_schedule_store(store.domain(KeyDomain::Schedules)).unwrap();
    refused.set_growth(GrowthAdoption::None).unwrap();
    let out = refused
        .execute(json!({"cron": DAILY, "prompt": "p"}), &ctx!(c, a, t))
        .await;
    assert!(matches!(out, ToolOutcome::Failed(_)), "None must refuse");

    let floor = ScheduleCreateTool::new();
    floor.set_schedule_store(store.domain(KeyDomain::Schedules)).unwrap();
    floor.set_growth(GrowthAdoption::PolicyAuto).unwrap();
    let out = floor
        .execute(json!({"cron": EVERY_MINUTE, "prompt": "p"}), &ctx!(c, a, t))
        .await;
    let ToolOutcome::Failed(e) = out else {
        panic!("every-minute cron must trip the floor, got {out:?}");
    };
    assert!(e.to_string().contains("15"), "floor message names the limit");
}

#[tokio::test]
async fn agent_schedule_cap_is_enforced() {
    let store = open_store().await;
    let handle = store.domain(KeyDomain::Schedules);
    for i in 0..10 {
        let rec = ScheduleRecord::new(
            format!("agt-seed-{i}"),
            DAILY.into(),
            "default".into(),
            "p".into(),
        )
        .unwrap()
        .with_provenance(ScheduleProvenance::Agent);
        schedule::create_schedule(&handle, &rec).await.unwrap();
    }
    let tool = ScheduleCreateTool::new();
    tool.set_schedule_store(store.domain(KeyDomain::Schedules)).unwrap();
    tool.set_growth(GrowthAdoption::PolicyAuto).unwrap();
    let (c, a, t) = ctx_parts();
    let out = tool
        .execute(json!({"cron": DAILY, "prompt": "p"}), &ctx!(c, a, t))
        .await;
    assert!(matches!(out, ToolOutcome::Failed(_)), "11th must be refused");
}

#[tokio::test]
async fn agent_may_only_touch_its_own_schedules() {
    let store = open_store().await;
    let handle = store.domain(KeyDomain::Schedules);
    // A config routine and an operator schedule.
    let cfg = ScheduleRecord::new(
        "cfg-nightly".into(),
        DAILY.into(),
        "default".into(),
        "p".into(),
    )
    .unwrap();
    schedule::create_schedule(&handle, &cfg).await.unwrap();
    let op = ScheduleRecord::new(
        "coffee-run".into(),
        DAILY.into(),
        "default".into(),
        "p".into(),
    )
    .unwrap()
    .with_provenance(ScheduleProvenance::Operator);
    schedule::create_schedule(&handle, &op).await.unwrap();
    // And the agent's own.
    let own = ScheduleRecord::new(
        "agt-own".into(),
        DAILY.into(),
        "default".into(),
        "p".into(),
    )
    .unwrap()
    .with_provenance(ScheduleProvenance::Agent);
    schedule::create_schedule(&handle, &own).await.unwrap();

    let del = ScheduleDeleteTool::new();
    del.set_schedule_store(store.domain(KeyDomain::Schedules)).unwrap();
    let upd = ScheduleUpdateTool::new();
    upd.set_schedule_store(store.domain(KeyDomain::Schedules)).unwrap();
    upd.set_growth(GrowthAdoption::PolicyAuto).unwrap();
    let (c, a, t) = ctx_parts();

    for foreign in ["cfg-nightly", "coffee-run"] {
        let out = del
            .execute(json!({"schedule_id": foreign}), &ctx!(c, a, t))
            .await;
        assert!(
            matches!(out, ToolOutcome::Failed(_)),
            "delete of {foreign} must be refused"
        );
        let out = upd
            .execute(json!({"schedule_id": foreign, "prompt": "x"}), &ctx!(c, a, t))
            .await;
        assert!(
            matches!(out, ToolOutcome::Failed(_)),
            "update of {foreign} must be refused"
        );
    }

    // Its own deletes fine.
    let out = del
        .execute(json!({"schedule_id": "agt-own"}), &ctx!(c, a, t))
        .await;
    let ToolOutcome::Completed { output, .. } = out else {
        panic!("own delete must succeed");
    };
    assert_eq!(output["deleted"], true);
}

#[tokio::test]
async fn agent_edit_below_policy_auto_re_disables_an_approved_schedule() {
    let store = open_store().await;
    let handle = store.domain(KeyDomain::Schedules);
    // An agent schedule the operator has approved (enabled).
    let mut own = ScheduleRecord::new(
        "agt-approved".into(),
        DAILY.into(),
        "default".into(),
        "p".into(),
    )
    .unwrap()
    .with_provenance(ScheduleProvenance::Agent);
    own.enabled = true;
    schedule::create_schedule(&handle, &own).await.unwrap();

    let upd = ScheduleUpdateTool::new();
    upd.set_schedule_store(store.domain(KeyDomain::Schedules)).unwrap();
    upd.set_growth(GrowthAdoption::ProposeOnly).unwrap();
    let (c, a, t) = ctx_parts();
    let out = upd
        .execute(
            json!({"schedule_id": "agt-approved", "prompt": "edited"}),
            &ctx!(c, a, t),
        )
        .await;
    let ToolOutcome::Completed { output, .. } = out else {
        panic!("own update must succeed, got {out:?}");
    };
    assert_eq!(output["enabled"], false, "edit invalidates the approval");
    // Self-approval bypass is blocked too: enabled:true from the agent
    // still ends disabled below policy_auto.
    let out = upd
        .execute(
            json!({"schedule_id": "agt-approved", "enabled": true}),
            &ctx!(c, a, t),
        )
        .await;
    let ToolOutcome::Completed { output, .. } = out else {
        panic!("own update must succeed");
    };
    assert_eq!(output["enabled"], false, "no self-approval below policy_auto");
}

#[tokio::test]
async fn operator_rules_reserved_prefixes_collisions_and_config_delete() {
    let store = open_store().await;
    let handle = store.domain(KeyDomain::Schedules);

    // Reserved prefixes refused.
    for reserved in ["cfg-sneak", "agt-sneak"] {
        assert!(
            operator_create_schedule(&handle, reserved, DAILY, "p", true)
                .await
                .is_err(),
            "{reserved} must be refused"
        );
    }
    // Create + collision.
    operator_create_schedule(&handle, "coffee-run", DAILY, "p", true)
        .await
        .unwrap();
    assert!(
        operator_create_schedule(&handle, "coffee-run", DAILY, "p", true)
            .await
            .is_err(),
        "duplicate name must be refused"
    );
    let rec = schedule::get_schedule(&handle, "coffee-run")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rec.created_by, ScheduleProvenance::Operator);
    assert!(rec.wrap_mission);

    // Operator update: enable-toggle works on any provenance (approval
    // gesture), and a bad cron is refused.
    let cfg = ScheduleRecord::new(
        "cfg-nightly".into(),
        DAILY.into(),
        "default".into(),
        "p".into(),
    )
    .unwrap();
    schedule::create_schedule(&handle, &cfg).await.unwrap();
    operator_update_schedule(&handle, "cfg-nightly", Some(false), None, None)
        .await
        .unwrap();
    assert!(
        !schedule::get_schedule(&handle, "cfg-nightly")
            .await
            .unwrap()
            .unwrap()
            .enabled
    );
    assert!(
        operator_update_schedule(
            &handle,
            "coffee-run",
            None,
            Some("not a cron".into()),
            None
        )
        .await
        .is_err()
    );

    // Config routines can't be deleted (boot sync would resurrect them);
    // operator schedules can.
    assert!(operator_delete_schedule(&handle, "cfg-nightly").await.is_err());
    operator_delete_schedule(&handle, "coffee-run").await.unwrap();
    assert!(schedule::get_schedule(&handle, "coffee-run")
        .await
        .unwrap()
        .is_none());
}

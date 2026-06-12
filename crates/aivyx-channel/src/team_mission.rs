//! Chapter L — persistence for daemon-run Nonagon team missions.
//!
//! The daemon drives `aivyx-team` missions (`TeamRuntime::run_until_pause`) and
//! must survive a restart: a mission paused at a **human-approval gate** has to
//! reload and resume, and an interrupted in-flight mission has to re-drive from
//! its last checkpoint. This module is the durable side of that — one encrypted
//! row per mission under [`KeyDomain::TeamMissions`], keyed by mission id.
//!
//! The mission **data** types ([`TeamMissionRecord`] + the [`TeamMissionView`]
//! projection) moved to the wasm-clean [`aivyx_ipc`] crate (Chapter M.2a) so
//! the browser Mission-Control app shares them; this module keeps the
//! **storage** side — the encrypted `serde_json`-over-[`DomainHandle`] CRUD,
//! mirroring the older single-agent [`crate::mission`] store but on its own
//! [`KeyDomain::TeamMissions`] domain. See `docs/DAEMON_TEAMS.md`.

use aivyx_storage::{DomainHandle, KeyDomain, StorageError};

pub use aivyx_ipc::team_mission::{
    TeamMissionPhase, TeamMissionRecord, TeamMissionView, TeamStepState, TeamStepView,
};

fn mission_key(id: &str) -> Vec<u8> {
    id.as_bytes().to_vec()
}

/// Insert or overwrite a mission record (call on every state transition).
pub async fn save_team_mission(
    handle: &DomainHandle,
    record: &TeamMissionRecord,
) -> Result<(), StorageError> {
    debug_assert_eq!(handle.domain(), KeyDomain::TeamMissions);
    let json = serde_json::to_vec(record).map_err(|e| {
        StorageError::Redb(format!("serialize TeamMissionRecord: {e}"))
    })?;
    handle.put(&mission_key(&record.id), &json).await
}

/// Fetch one mission by id, or `None` if absent.
pub async fn get_team_mission(
    handle: &DomainHandle,
    id: &str,
) -> Result<Option<TeamMissionRecord>, StorageError> {
    debug_assert_eq!(handle.domain(), KeyDomain::TeamMissions);
    match handle.get(&mission_key(id)).await? {
        Some(bytes) => {
            let record = serde_json::from_slice(&bytes).map_err(|e| {
                StorageError::Redb(format!("deserialize TeamMissionRecord: {e}"))
            })?;
            Ok(Some(record))
        }
        None => Ok(None),
    }
}

/// Every persisted mission — the daemon's reload-on-startup primitive.
pub async fn list_team_missions(
    handle: &DomainHandle,
) -> Result<Vec<TeamMissionRecord>, StorageError> {
    debug_assert_eq!(handle.domain(), KeyDomain::TeamMissions);
    let rows = handle.scan_prefix(b"").await?;
    let mut out = Vec::with_capacity(rows.len());
    for (_key, bytes) in rows {
        let record: TeamMissionRecord = serde_json::from_slice(&bytes).map_err(|e| {
            StorageError::Redb(format!("deserialize TeamMissionRecord: {e}"))
        })?;
        out.push(record);
    }
    Ok(out)
}

/// Remove a mission record (e.g. operator-cleared history).
pub async fn delete_team_mission(
    handle: &DomainHandle,
    id: &str,
) -> Result<(), StorageError> {
    debug_assert_eq!(handle.domain(), KeyDomain::TeamMissions);
    handle.delete(&mission_key(id)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{RedbStorage, Storage, StorageConfig};
    use aivyx_team::{MissionPlan, Step};
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    async fn team_domain() -> DomainHandle {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir()
            .join(format!("aivyx-team-mission-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let storage: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([3u8; 32]),
        )
        .await
        .expect("open storage");
        storage.domain(KeyDomain::TeamMissions)
    }

    fn sample(id: &str) -> TeamMissionRecord {
        let plan = MissionPlan::new(
            "overnight close",
            vec![
                Step::delegate("count", "inventory", "count stock"),
                Step::human_gate("approve", "manager", "ok to order?").after(["count"]),
                Step::delegate("order", "purchasing", "place PO").after(["approve"]),
            ],
        );
        let mut r = TeamMissionRecord::new(id, "overnight close", plan);
        r.outputs.insert("count".into(), "12 low items".into());
        r.phase = TeamMissionPhase::AwaitingApproval;
        r.pending_gate = Some("approve".into());
        r
    }

    #[tokio::test]
    async fn save_get_round_trips_a_paused_mission() {
        let h = team_domain().await;
        let rec = sample("m1");
        save_team_mission(&h, &rec).await.unwrap();
        let got = get_team_mission(&h, "m1").await.unwrap().expect("present");
        assert_eq!(got, rec, "the full record (plan + checkpoint + phase) survives");
        assert_eq!(got.phase, TeamMissionPhase::AwaitingApproval);
        assert_eq!(got.pending_gate.as_deref(), Some("approve"));
        assert_eq!(got.outputs["count"], "12 low items");
        // The human gate's mode survived serde.
        assert!(got.plan.step("approve").unwrap().is_human_gate());
    }

    #[tokio::test]
    async fn get_missing_is_none() {
        let h = team_domain().await;
        assert!(get_team_mission(&h, "nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_is_the_reload_primitive() {
        let h = team_domain().await;
        save_team_mission(&h, &sample("a")).await.unwrap();
        save_team_mission(&h, &sample("b")).await.unwrap();
        let mut ids: Vec<String> =
            list_team_missions(&h).await.unwrap().into_iter().map(|r| r.id).collect();
        ids.sort();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn save_overwrites_and_delete_removes() {
        let h = team_domain().await;
        let mut rec = sample("m");
        save_team_mission(&h, &rec).await.unwrap();
        rec.phase = TeamMissionPhase::Done;
        rec.touch();
        save_team_mission(&h, &rec).await.unwrap();
        assert_eq!(
            get_team_mission(&h, "m").await.unwrap().unwrap().phase,
            TeamMissionPhase::Done,
            "save overwrites in place"
        );
        delete_team_mission(&h, "m").await.unwrap();
        assert!(get_team_mission(&h, "m").await.unwrap().is_none());
    }
}

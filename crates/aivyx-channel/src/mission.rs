//! Mission primitive — Phase 21.
//!
//! A mission is a long-running work item that survives daemon restarts,
//! runs under a specific role's capability envelope, and emits
//! operator-visible approval gates. Backed by a redb row under
//! `KeyDomain::Missions`.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use aivyx_storage::{DomainHandle, KeyDomain, StorageError};

// ---------------------------------------------------------------------------
// State enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionState {
    Created,
    Running,
    GatePending,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateState {
    Pending,
    Approved,
    Rejected,
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateRecord {
    pub gate_id: String,
    pub reason: String,
    pub scope: Option<String>,
    pub state: GateState,
    pub created_at: u64,
    pub resolved_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionRecord {
    pub mission_id: String,
    pub role_name: String,
    pub description: String,
    pub state: MissionState,
    pub gates: Vec<GateRecord>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl MissionRecord {
    pub fn new(mission_id: String, role_name: String, description: String) -> Self {
        let now = now_millis();
        MissionRecord {
            mission_id,
            role_name,
            description,
            state: MissionState::Created,
            gates: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn pending_gate(&self) -> Option<&GateRecord> {
        self.gates.iter().find(|g| g.state == GateState::Pending)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            MissionState::Completed | MissionState::Failed | MissionState::Cancelled
        )
    }
}

// ---------------------------------------------------------------------------
// Storage CRUD
// ---------------------------------------------------------------------------

fn mission_key(mission_id: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(mission_id.len());
    key.extend_from_slice(mission_id.as_bytes());
    key
}

pub async fn create_mission(
    handle: &DomainHandle,
    record: &MissionRecord,
) -> Result<(), StorageError> {
    assert_eq!(handle.domain(), KeyDomain::Missions);
    let json = serde_json::to_vec(record).map_err(|e| {
        StorageError::Redb(format!("serialize MissionRecord: {e}"))
    })?;
    handle.put(&mission_key(&record.mission_id), &json).await
}

pub async fn get_mission(
    handle: &DomainHandle,
    mission_id: &str,
) -> Result<Option<MissionRecord>, StorageError> {
    assert_eq!(handle.domain(), KeyDomain::Missions);
    match handle.get(&mission_key(mission_id)).await? {
        Some(bytes) => {
            let record: MissionRecord =
                serde_json::from_slice(&bytes).map_err(|e| {
                    StorageError::Redb(format!("deserialize MissionRecord: {e}"))
                })?;
            Ok(Some(record))
        }
        None => Ok(None),
    }
}

pub async fn update_mission(
    handle: &DomainHandle,
    record: &MissionRecord,
) -> Result<(), StorageError> {
    create_mission(handle, record).await
}

pub async fn list_missions(
    handle: &DomainHandle,
) -> Result<Vec<MissionRecord>, StorageError> {
    assert_eq!(handle.domain(), KeyDomain::Missions);
    let rows = handle.scan_prefix(b"").await?;
    let mut missions = Vec::with_capacity(rows.len());
    for (_key, bytes) in rows {
        let record: MissionRecord =
            serde_json::from_slice(&bytes).map_err(|e| {
                StorageError::Redb(format!("deserialize MissionRecord: {e}"))
            })?;
        missions.push(record);
    }
    Ok(missions)
}

pub async fn delete_mission(
    handle: &DomainHandle,
    mission_id: &str,
) -> Result<(), StorageError> {
    assert_eq!(handle.domain(), KeyDomain::Missions);
    handle.delete(&mission_key(mission_id)).await
}

// ---------------------------------------------------------------------------
// State transitions
// ---------------------------------------------------------------------------

pub fn transition_to_running(record: &mut MissionRecord) -> Result<(), String> {
    match record.state {
        MissionState::Created => {
            record.state = MissionState::Running;
            record.updated_at = now_millis();
            Ok(())
        }
        other => Err(format!(
            "cannot transition to Running from {other:?}"
        )),
    }
}

pub fn add_gate(
    record: &mut MissionRecord,
    gate_id: String,
    reason: String,
    scope: Option<String>,
) -> Result<(), String> {
    if record.state != MissionState::Running {
        return Err(format!(
            "cannot add gate in state {:?}",
            record.state
        ));
    }
    record.gates.push(GateRecord {
        gate_id,
        reason,
        scope,
        state: GateState::Pending,
        created_at: now_millis(),
        resolved_at: None,
    });
    record.state = MissionState::GatePending;
    record.updated_at = now_millis();
    Ok(())
}

pub fn resolve_gate(
    record: &mut MissionRecord,
    gate_id: &str,
    approved: bool,
) -> Result<(), String> {
    if record.state != MissionState::GatePending {
        return Err(format!(
            "cannot resolve gate in state {:?}",
            record.state
        ));
    }
    let gate = record
        .gates
        .iter_mut()
        .find(|g| g.gate_id == gate_id && g.state == GateState::Pending)
        .ok_or_else(|| format!("no pending gate with id {gate_id}"))?;

    let now = now_millis();
    gate.state = if approved {
        GateState::Approved
    } else {
        GateState::Rejected
    };
    gate.resolved_at = Some(now);

    record.state = if approved {
        MissionState::Running
    } else {
        MissionState::Failed
    };
    record.updated_at = now;
    Ok(())
}

pub fn complete_mission(record: &mut MissionRecord) -> Result<(), String> {
    if record.state != MissionState::Running {
        return Err(format!(
            "cannot complete in state {:?}",
            record.state
        ));
    }
    record.state = MissionState::Completed;
    record.updated_at = now_millis();
    Ok(())
}

pub fn cancel_mission(record: &mut MissionRecord) -> Result<(), String> {
    if record.is_terminal() {
        return Err(format!(
            "cannot cancel in terminal state {:?}",
            record.state
        ));
    }
    record.state = MissionState::Cancelled;
    record.updated_at = now_millis();
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_mission() -> MissionRecord {
        MissionRecord::new(
            "m-001".into(),
            "coder".into(),
            "Track CI for regressions".into(),
        )
    }

    #[test]
    fn new_mission_is_created_state() {
        let m = sample_mission();
        assert_eq!(m.state, MissionState::Created);
        assert!(m.gates.is_empty());
        assert!(!m.is_terminal());
    }

    #[test]
    fn transition_created_to_running() {
        let mut m = sample_mission();
        transition_to_running(&mut m).unwrap();
        assert_eq!(m.state, MissionState::Running);
    }

    #[test]
    fn transition_running_to_running_fails() {
        let mut m = sample_mission();
        transition_to_running(&mut m).unwrap();
        assert!(transition_to_running(&mut m).is_err());
    }

    #[test]
    fn add_gate_transitions_to_gate_pending() {
        let mut m = sample_mission();
        transition_to_running(&mut m).unwrap();
        add_gate(&mut m, "g-001".into(), "deploy?".into(), None).unwrap();
        assert_eq!(m.state, MissionState::GatePending);
        assert_eq!(m.gates.len(), 1);
        assert_eq!(m.gates[0].state, GateState::Pending);
    }

    #[test]
    fn add_gate_in_created_state_fails() {
        let mut m = sample_mission();
        assert!(add_gate(&mut m, "g-001".into(), "x".into(), None).is_err());
    }

    #[test]
    fn resolve_gate_approved_resumes_running() {
        let mut m = sample_mission();
        transition_to_running(&mut m).unwrap();
        add_gate(&mut m, "g-001".into(), "deploy?".into(), None).unwrap();
        resolve_gate(&mut m, "g-001", true).unwrap();
        assert_eq!(m.state, MissionState::Running);
        assert_eq!(m.gates[0].state, GateState::Approved);
        assert!(m.gates[0].resolved_at.is_some());
    }

    #[test]
    fn resolve_gate_rejected_fails_mission() {
        let mut m = sample_mission();
        transition_to_running(&mut m).unwrap();
        add_gate(&mut m, "g-001".into(), "deploy?".into(), None).unwrap();
        resolve_gate(&mut m, "g-001", false).unwrap();
        assert_eq!(m.state, MissionState::Failed);
        assert_eq!(m.gates[0].state, GateState::Rejected);
        assert!(m.is_terminal());
    }

    #[test]
    fn resolve_wrong_gate_id_fails() {
        let mut m = sample_mission();
        transition_to_running(&mut m).unwrap();
        add_gate(&mut m, "g-001".into(), "deploy?".into(), None).unwrap();
        assert!(resolve_gate(&mut m, "g-999", true).is_err());
    }

    #[test]
    fn complete_mission_from_running() {
        let mut m = sample_mission();
        transition_to_running(&mut m).unwrap();
        complete_mission(&mut m).unwrap();
        assert_eq!(m.state, MissionState::Completed);
        assert!(m.is_terminal());
    }

    #[test]
    fn complete_from_gate_pending_fails() {
        let mut m = sample_mission();
        transition_to_running(&mut m).unwrap();
        add_gate(&mut m, "g-001".into(), "x".into(), None).unwrap();
        assert!(complete_mission(&mut m).is_err());
    }

    #[test]
    fn cancel_from_any_non_terminal() {
        let mut m = sample_mission();
        cancel_mission(&mut m).unwrap();
        assert_eq!(m.state, MissionState::Cancelled);
        assert!(m.is_terminal());
    }

    #[test]
    fn cancel_from_terminal_fails() {
        let mut m = sample_mission();
        transition_to_running(&mut m).unwrap();
        complete_mission(&mut m).unwrap();
        assert!(cancel_mission(&mut m).is_err());
    }

    #[test]
    fn pending_gate_returns_first_pending() {
        let mut m = sample_mission();
        transition_to_running(&mut m).unwrap();
        add_gate(&mut m, "g-001".into(), "first".into(), None).unwrap();
        let pg = m.pending_gate().unwrap();
        assert_eq!(pg.gate_id, "g-001");
    }

    #[test]
    fn pending_gate_returns_none_when_all_resolved() {
        let mut m = sample_mission();
        transition_to_running(&mut m).unwrap();
        add_gate(&mut m, "g-001".into(), "first".into(), None).unwrap();
        resolve_gate(&mut m, "g-001", true).unwrap();
        assert!(m.pending_gate().is_none());
    }

    #[test]
    fn serde_round_trip() {
        let mut m = sample_mission();
        transition_to_running(&mut m).unwrap();
        add_gate(
            &mut m,
            "g-001".into(),
            "deploy?".into(),
            Some("shell.exec".into()),
        )
        .unwrap();

        let json = serde_json::to_vec(&m).unwrap();
        let recovered: MissionRecord = serde_json::from_slice(&json).unwrap();
        assert_eq!(recovered.mission_id, m.mission_id);
        assert_eq!(recovered.state, MissionState::GatePending);
        assert_eq!(recovered.gates.len(), 1);
        assert_eq!(recovered.gates[0].scope, Some("shell.exec".into()));
    }

    #[test]
    fn multi_gate_lifecycle() {
        let mut m = sample_mission();
        transition_to_running(&mut m).unwrap();

        add_gate(&mut m, "g-001".into(), "first gate".into(), None).unwrap();
        resolve_gate(&mut m, "g-001", true).unwrap();
        assert_eq!(m.state, MissionState::Running);

        add_gate(&mut m, "g-002".into(), "second gate".into(), None).unwrap();
        resolve_gate(&mut m, "g-002", true).unwrap();
        assert_eq!(m.state, MissionState::Running);

        complete_mission(&mut m).unwrap();
        assert_eq!(m.state, MissionState::Completed);
        assert_eq!(m.gates.len(), 2);
    }
}

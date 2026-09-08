//! Chapter Helm (Opp F) — the persisted autonomous-loop run marker.
//!
//! [`crate::loop_driver::SharedLoopState`] is in-memory only, so a daemon
//! restart loses whether a run was active. This tiny marker — a single byte in
//! [`aivyx_storage::KeyDomain::LoopState`] — bridges that gap so an opt-in
//! `[loop] resume_on_boot` can tell a **crash / restart mid-run** (resume)
//! apart from an **operator `loop stop`** (stay stopped).
//!
//! Semantics = operator intent, written at exactly the two operator-driven
//! transitions:
//! - `aivyx-pa loop start` → marker **active**
//! - `aivyx-pa loop stop`  → marker **idle**
//!
//! A crash, panic, or `systemctl restart` never touches it, so the marker
//! stays active across those → the loop resumes. A clean run-end (backlog
//! drained / caps hit) leaves it as-is: a drained backlog has nothing to
//! resume anyway, and a cap-ended run with pending work resumes into a fresh
//! window — matching the "runs for days" intent. Best-effort throughout:
//! persistence failures never break a `loop start`/`stop`.

use aivyx_storage::{DomainHandle, StorageError};

/// The single reserved key under [`aivyx_storage::KeyDomain::LoopState`].
const RUN_ACTIVE_KEY: &[u8] = b"run-active";

/// Persist whether an autonomous-loop run is active.
pub async fn set_run_active(
    store: &DomainHandle,
    active: bool,
) -> Result<(), StorageError> {
    store.put(RUN_ACTIVE_KEY, &[active as u8]).await
}

/// Read the marker. Absent or unreadable → `false` (the safe default: don't
/// resume something we can't confirm was running).
pub async fn run_was_active(store: &DomainHandle) -> bool {
    matches!(
        store.get(RUN_ACTIVE_KEY).await,
        Ok(Some(v)) if v.first() == Some(&1)
    )
}

/// The pure boot-resume decision: resume an autonomous-loop run on daemon boot
/// iff the operator opted in (`resume_on_boot`), a run was active when the
/// daemon last stopped (`marker_active` — a crash/restart, not an explicit
/// stop), AND there is still pending work. Pure so the policy is unit-testable
/// without a daemon.
pub fn should_resume_on_boot(
    resume_on_boot: bool,
    marker_active: bool,
    pending_stories: usize,
) -> bool {
    resume_on_boot && marker_active && pending_stories > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
    use std::sync::Arc;

    async fn loop_state_domain() -> DomainHandle {
        let base = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base)
            .join(format!("aivyx-loop-resume-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([7u8; 32]),
        )
        .await
        .unwrap();
        store.domain(KeyDomain::LoopState)
    }

    #[tokio::test]
    async fn marker_defaults_to_inactive_when_absent() {
        let d = loop_state_domain().await;
        assert!(!run_was_active(&d).await, "absent marker reads inactive");
    }

    #[tokio::test]
    async fn marker_round_trips_and_clears() {
        let d = loop_state_domain().await;
        set_run_active(&d, true).await.unwrap();
        assert!(run_was_active(&d).await, "set active");
        set_run_active(&d, false).await.unwrap();
        assert!(!run_was_active(&d).await, "cleared");
    }

    #[test]
    fn resume_decision_requires_optin_marker_and_pending() {
        // Resume only when ALL three hold.
        assert!(should_resume_on_boot(true, true, 3));
        // opt-out → never
        assert!(!should_resume_on_boot(false, true, 3));
        // not active (e.g. operator stopped it) → never
        assert!(!should_resume_on_boot(true, false, 3));
        // nothing pending → nothing to resume
        assert!(!should_resume_on_boot(true, true, 0));
    }
}

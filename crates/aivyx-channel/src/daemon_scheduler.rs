//! Daemon scheduler loop — Phase 26 Task 3.
//!
//! A background task that evaluates schedule entries against the current
//! time and fires agent turns when a schedule is due. Spawned inside
//! `run_daemon` alongside the accept loop, sharing the same `Agent`,
//! `ChannelFactory`, and `CancellationToken`.
//!
//! The tick cadence is adaptive: the loop sleeps until the earliest
//! next-fire-time across all enabled schedules (capped at 60 s so new
//! schedules created mid-run are picked up promptly).

use std::time::Duration;

use chrono::Utc;

use aivyx_core::CancellationToken;
use aivyx_storage::DomainHandle;

use crate::schedule::{self, ScheduleRecord};
use crate::trigger::{TriggerDispatch, TriggerSource};

const MAX_TICK_INTERVAL: Duration = Duration::from_secs(60);

/// Merge TOML `[[schedule]]` entries into the schedule store. Entries
/// whose `name` already exists in storage are skipped (storage is
/// authoritative after first sync). Called once at daemon startup.
pub async fn sync_config_schedules(
    store: &DomainHandle,
    config_schedules: &[crate::schedule::ScheduleRecord],
) -> Result<usize, String> {
    let mut synced = 0;
    for record in config_schedules {
        let existing = schedule::get_schedule(store, &record.schedule_id)
            .await
            .map_err(|e| format!("sync_config_schedules get: {e}"))?;
        if existing.is_none() {
            schedule::create_schedule(store, record)
                .await
                .map_err(|e| format!("sync_config_schedules create: {e}"))?;
            synced += 1;
        }
    }
    Ok(synced)
}

/// Convert config-layer `ScheduleConfig` entries into storage-layer
/// `ScheduleRecord` values suitable for `sync_config_schedules`.
pub fn config_to_records(
    configs: &[aivyx_config::ScheduleConfig],
) -> Result<Vec<ScheduleRecord>, String> {
    configs
        .iter()
        .map(|c| {
            let mut built = match &c.team_mission {
                Some(tm) => ScheduleRecord::new_team_mission(
                    format!("cfg-{}", c.name),
                    c.cron.clone(),
                    tm.goal.clone(),
                    tm.pack_config.clone(),
                ),
                None => ScheduleRecord::new(
                    format!("cfg-{}", c.name),
                    c.cron.clone(),
                    c.role.clone(),
                    c.prompt.clone(),
                ),
            }?;
            built.enabled = c.enabled;
            built.wrap_mission = c.wrap_mission;
            built.notify_target = c.notify_target.clone();
            built.notify_targets = c.notify_targets.clone();
            built.notify_when = c.notify_when;
            built.report_kind = c.report_kind.clone();
            Ok(built)
        })
        .collect()
}

/// Chapter Ledger — context the scheduler uses to run a `report_kind = "digest"`
/// routine **deterministically** (no LLM). `None` ⇒ digest routines fall back
/// to firing their prompt as a normal LLM turn (so the daemon still works if
/// the builder wasn't wired).
#[derive(Clone)]
pub struct ReportContext {
    pub digest: std::sync::Arc<crate::digest::WeeklyDigestBuilder>,
    pub notify:
        Option<std::sync::Arc<crate::notify_dispatcher::NotifyDispatcher>>,
    /// Chapter Herald — see `TriggerDispatch::default_notify_target`;
    /// same live fallback, for the deterministic digest path which
    /// bypasses `TriggerDispatch::fire` entirely.
    pub default_notify_target: Option<String>,
}

/// Run the scheduler loop. This future never returns normally — it
/// runs until `shutdown` is cancelled.
///
/// On each tick the scheduler:
/// 1. Loads all enabled schedules from `store`.
/// 2. For each schedule whose next fire time is ≤ now and that hasn't
///    already fired in this cron window, fires a turn through the agent.
/// 3. Updates `last_fired_at` on fired schedules.
/// 4. Sleeps until the earliest next-fire-time (capped at 60 s).
pub async fn run_scheduler(
    dispatch: TriggerDispatch,
    store: DomainHandle,
    shutdown: CancellationToken,
    report_ctx: Option<ReportContext>,
    team_missions: Option<crate::team_mission_driver::TeamMissionService>,
) {
    loop {
        if shutdown.is_cancelled() {
            return;
        }

        let sleep_dur = match tick(
            &dispatch,
            &store,
            report_ctx.as_ref(),
            team_missions.as_ref(),
        )
        .await
        {
            Ok(dur) => dur,
            Err(e) => {
                eprintln!("aivyx scheduler: tick error: {e}");
                MAX_TICK_INTERVAL
            }
        };

        tokio::select! {
            _ = tokio::time::sleep(sleep_dur) => {}
            _ = shutdown.cancelled() => return,
        }
    }
}

/// Execute one scheduler tick. Returns the duration to sleep before the
/// next tick.
async fn tick(
    dispatch: &TriggerDispatch,
    store: &DomainHandle,
    report_ctx: Option<&ReportContext>,
    team_missions: Option<&crate::team_mission_driver::TeamMissionService>,
) -> Result<Duration, String> {
    let schedules = schedule::list_schedules(store)
        .await
        .map_err(|e| format!("list schedules: {e}"))?;

    let now = Utc::now();
    let mut earliest_next: Option<Duration> = None;

    for sched in &schedules {
        if !sched.enabled {
            continue;
        }

        let Some(next_fire) = sched.next_fire_time_after(
            last_fired_or_epoch(sched),
        ) else {
            continue;
        };

        if next_fire <= now {
            if !already_fired_in_window(sched, next_fire) {
                fire_schedule(dispatch, store, sched, report_ctx, team_missions).await;
            }
            // Recompute next fire after this one.
            if let Some(after_now) = sched.next_fire_time_after(now) {
                update_earliest(&mut earliest_next, after_now, now);
            }
        } else {
            update_earliest(&mut earliest_next, next_fire, now);
        }
    }

    Ok(earliest_next.unwrap_or(MAX_TICK_INTERVAL).min(MAX_TICK_INTERVAL))
}

/// Compute the anchor for next-fire-time: if the schedule has fired
/// before, use that timestamp; otherwise anchor at **creation time**.
/// The old epoch anchor made every never-fired schedule instantly
/// "overdue", so a schedule created at 15:18 for a 16:00 cron fired a
/// spurious catch-up within seconds (Chime dogfood, 2026-07-06) — the
/// reflection scheduler's boot-fire bug, same family. A `created_at`
/// of 0 (hand-crafted records) still degrades to epoch harmlessly.
fn last_fired_or_epoch(sched: &ScheduleRecord) -> chrono::DateTime<Utc> {
    match sched.last_fired_at {
        Some(ms) => {
            chrono::DateTime::from_timestamp_millis(ms as i64)
                .unwrap_or(chrono::DateTime::UNIX_EPOCH)
        }
        None => chrono::DateTime::from_timestamp_millis(sched.created_at as i64)
            .unwrap_or(chrono::DateTime::UNIX_EPOCH),
    }
}

/// Deduplication: a schedule has "already fired in this window" if
/// `last_fired_at` ≥ the fire time we're considering.
fn already_fired_in_window(sched: &ScheduleRecord, fire_time: chrono::DateTime<Utc>) -> bool {
    match sched.last_fired_at {
        Some(ms) => {
            let last = chrono::DateTime::from_timestamp_millis(ms as i64)
                .unwrap_or(chrono::DateTime::UNIX_EPOCH);
            last >= fire_time
        }
        None => false,
    }
}

fn update_earliest(
    earliest: &mut Option<Duration>,
    fire_time: chrono::DateTime<Utc>,
    now: chrono::DateTime<Utc>,
) {
    let delta = (fire_time - now).to_std().unwrap_or(Duration::ZERO);
    match earliest {
        Some(current) if delta < *current => *earliest = Some(delta),
        None => *earliest = Some(delta),
        _ => {}
    }
}

/// Chapter Muster — the deterministic team-mission half of `fire_schedule`,
/// extracted into its own function so it's testable without constructing a
/// `TriggerDispatch` (which nothing in this codebase does today — see this
/// function's own test for why). Loads `sched.team_mission`'s pack config
/// (if any) fresh at fire time, then starts the mission via
/// `TeamMissionService::start_from_goal_for_schedule`, tagging it with
/// `sched.schedule_id`. Errors are logged, never propagated — matches
/// `fire_schedule`'s own existing fire-and-forget error handling.
async fn fire_team_mission_schedule(
    sched: &ScheduleRecord,
    tm: &crate::schedule::ScheduledTeamMission,
    team_missions: &crate::team_mission_driver::TeamMissionService,
) {
    let config = match &tm.pack_config {
        Some(path) => match aivyx_team::TeamConfig::load(path) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!(
                    "aivyx scheduler: schedule {:?}'s pack_config {path:?} failed to load: {e}",
                    sched.schedule_id
                );
                return;
            }
        },
        None => None,
    };
    match team_missions
        .start_from_goal_for_schedule(&tm.goal, config, &sched.schedule_id)
        .await
    {
        Ok(mission_id) => {
            eprintln!(
                "aivyx scheduler: schedule {:?} started team mission {mission_id}",
                sched.schedule_id
            );
        }
        Err(e) => {
            eprintln!(
                "aivyx scheduler: schedule {:?} failed to start a team mission: {e}",
                sched.schedule_id
            );
        }
    }
}

/// Fire a single scheduled turn through the shared trigger dispatch.
async fn fire_schedule(
    dispatch: &TriggerDispatch,
    store: &DomainHandle,
    sched: &ScheduleRecord,
    report_ctx: Option<&ReportContext>,
    team_missions: Option<&crate::team_mission_driver::TeamMissionService>,
) {
    if let Some(tm) = &sched.team_mission {
        match team_missions {
            Some(svc) => fire_team_mission_schedule(sched, tm, svc).await,
            None => eprintln!(
                "aivyx scheduler: schedule {:?} targets a team mission but no \
                 TeamMissionService is wired — skipping",
                sched.schedule_id
            ),
        }
        update_last_fired(store, sched).await;
        return;
    }

    // Chapter Ledger — a `report_kind = "digest"` routine runs a deterministic
    // daemon-assembled digest instead of an LLM turn, so it cannot confabulate
    // (#6). Falls through to the normal LLM path if no builder is wired.
    if sched.report_kind.as_deref() == Some("digest") {
        if let Some(ctx) = report_ctx {
            run_digest_report(ctx, sched).await;
            update_last_fired(store, sched).await;
            return;
        }
        eprintln!(
            "aivyx scheduler: schedule {:?} is report_kind=digest but no digest \
             builder is wired — falling back to the LLM prompt",
            sched.schedule_id
        );
    }

    dispatch
        .fire(
            TriggerSource::Cron,
            &sched.schedule_id,
            &sched.prompt,
            sched.wrap_mission,
            &sched.notify_targets,
            sched.notify_when,
        )
        .await;

    update_last_fired(store, sched).await;
}

/// Chapter Ledger — build the deterministic digest (since the last fire) +
/// persist it + push it to any notify targets. No LLM in the content path.
async fn run_digest_report(ctx: &ReportContext, sched: &ScheduleRecord) {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Cover everything since the last fire (ms→s); first run → since 0.
    let since_secs = sched.last_fired_at.map(|ms| ms / 1000).unwrap_or(0);

    let text = ctx.digest.run(since_secs, now_secs).await;
    eprintln!(
        "aivyx scheduler: deterministic digest written ({} chars) for {:?}",
        text.len(),
        sched.schedule_id
    );

    // Deliver to notify targets (the digest text is always a real, non-empty,
    // successful completion). `OnFailed` opts out (a digest never fails);
    // `Always` / `OnCompletedNonEmpty` deliver the briefing.
    let targets: Vec<String> = crate::trigger::resolve_notify_targets(
        &sched.notify_targets,
        ctx.default_notify_target.as_deref(),
    );
    if !targets.is_empty() && sched.notify_when != aivyx_config::NotifyWhen::OnFailed {
        if let Some(notify) = &ctx.notify {
            let subject = format!("cron: {}", sched.schedule_id);
            for target in &targets {
                if let Err(e) = notify.dispatch(target, &text, Some(&subject)).await {
                    eprintln!(
                        "aivyx scheduler: digest notify to {target:?} failed: {e}"
                    );
                }
            }
        }
    }
}

/// Stamp `last_fired_at = now` (shared by both fire paths).
async fn update_last_fired(store: &DomainHandle, sched: &ScheduleRecord) {
    // Update last_fired_at regardless of outcome.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let mut updated = sched.clone();
    updated.last_fired_at = Some(now_ms);
    if let Err(e) = schedule::update_schedule(store, &updated).await {
        eprintln!(
            "aivyx scheduler: failed to update last_fired_at for {:?}: {e}",
            sched.schedule_id,
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    fn make_schedule(cron_expr: &str, last_fired: Option<u64>) -> ScheduleRecord {
        let mut r = ScheduleRecord::new(
            "test-sched".into(),
            cron_expr.into(),
            "default".into(),
            "do something".into(),
        )
        .unwrap();
        r.last_fired_at = last_fired;
        r
    }

    #[test]
    fn never_fired_schedule_anchors_at_creation_not_epoch() {
        // A schedule created NOW for a later tick must not be "overdue
        // since 1970" — the epoch anchor fired a 16:00 schedule seconds
        // after its 15:18 creation (Chime dogfood, 2026-07-06).
        let s = make_schedule("0 * * * * * *", None);
        let anchor = last_fired_or_epoch(&s);
        assert_eq!(anchor.timestamp_millis(), s.created_at as i64);
        let next = s.next_fire_time_after(anchor).expect("has a next fire");
        assert!(
            next > anchor,
            "first fire must be the next tick after creation"
        );
    }

    #[test]
    fn last_fired_or_epoch_returns_timestamp() {
        let ts: u64 = 1_700_000_000_000; // 2023-11-14
        let s = make_schedule("0 * * * * * *", Some(ts));
        let dt = last_fired_or_epoch(&s);
        assert_eq!(dt.timestamp_millis(), ts as i64);
    }

    #[test]
    fn already_fired_in_window_prevents_double_fire() {
        let fire_time = DateTime::parse_from_rfc3339("2026-06-01T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // last_fired_at is AT the fire time — should be considered fired.
        let s = make_schedule(
            "0 0 9 * * * *",
            Some(fire_time.timestamp_millis() as u64),
        );
        assert!(already_fired_in_window(&s, fire_time));
    }

    #[test]
    fn not_fired_when_last_fired_is_before_window() {
        let fire_time = DateTime::parse_from_rfc3339("2026-06-01T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // last_fired is one hour before — not in this window.
        let earlier = fire_time - chrono::Duration::hours(1);
        let s = make_schedule(
            "0 0 9 * * * *",
            Some(earlier.timestamp_millis() as u64),
        );
        assert!(!already_fired_in_window(&s, fire_time));
    }

    #[test]
    fn not_fired_when_no_last_fired() {
        let fire_time = DateTime::parse_from_rfc3339("2026-06-01T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let s = make_schedule("0 0 9 * * * *", None);
        assert!(!already_fired_in_window(&s, fire_time));
    }

    #[test]
    fn update_earliest_picks_minimum() {
        let now = Utc::now();
        let mut earliest: Option<Duration> = None;

        let t1 = now + chrono::Duration::seconds(120);
        update_earliest(&mut earliest, t1, now);
        assert!(earliest.is_some());
        let first = earliest.unwrap();

        let t2 = now + chrono::Duration::seconds(30);
        update_earliest(&mut earliest, t2, now);
        assert!(earliest.unwrap() < first);

        // A later time shouldn't replace the earlier one.
        let t3 = now + chrono::Duration::seconds(300);
        update_earliest(&mut earliest, t3, now);
        assert!(earliest.unwrap() < Duration::from_secs(60));
    }

    #[test]
    fn config_to_records_converts_correctly() {
        let configs = vec![aivyx_config::ScheduleConfig {
            name: "daily-check".into(),
            cron: "0 0 9 * * * *".into(),
            role: "ops".into(),
            prompt: "check system health".into(),
            enabled: true,
            wrap_mission: false,
            notify_target: None,
            notify_targets: Vec::new(),
            notify_when: aivyx_config::NotifyWhen::Always,
            report_kind: None,
            team_mission: None,
        }];
        let records = config_to_records(&configs).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].schedule_id, "cfg-daily-check");
        assert_eq!(records[0].role_name, "ops");
        assert!(records[0].enabled);
    }

    #[test]
    fn config_to_records_rejects_invalid_cron() {
        let configs = vec![aivyx_config::ScheduleConfig {
            name: "bad".into(),
            cron: "not valid".into(),
            role: "default".into(),
            prompt: "test".into(),
            enabled: true,
            wrap_mission: false,
            notify_target: None,
            notify_targets: Vec::new(),
            notify_when: aivyx_config::NotifyWhen::Always,
            report_kind: None,
            team_mission: None,
        }];
        assert!(config_to_records(&configs).is_err());
    }

    #[test]
    fn disabled_config_produces_disabled_record() {
        let configs = vec![aivyx_config::ScheduleConfig {
            name: "off".into(),
            cron: "0 0 9 * * * *".into(),
            role: "default".into(),
            prompt: "test".into(),
            enabled: false,
            wrap_mission: false,
            notify_target: None,
            notify_targets: Vec::new(),
            notify_when: aivyx_config::NotifyWhen::Always,
            report_kind: None,
            team_mission: None,
        }];
        let records = config_to_records(&configs).unwrap();
        assert!(!records[0].enabled);
    }

    #[test]
    fn config_to_records_propagates_wrap_mission() {
        let configs = vec![aivyx_config::ScheduleConfig {
            name: "wrapped".into(),
            cron: "0 0 9 * * * *".into(),
            role: "default".into(),
            prompt: "test".into(),
            enabled: true,
            wrap_mission: true,
            notify_target: None,
            notify_targets: Vec::new(),
            notify_when: aivyx_config::NotifyWhen::Always,
            report_kind: None,
            team_mission: None,
        }];
        let records = config_to_records(&configs).unwrap();
        assert!(records[0].wrap_mission);
    }

    #[tokio::test]
    async fn fire_team_mission_schedule_starts_a_tagged_mission() {
        let sched = ScheduleRecord::new_team_mission(
            "cfg-nightly-boh-close".to_string(),
            "0 0 2 * * * *".to_string(),
            "run the overnight close".to_string(),
            None,
        )
        .expect("valid");
        let svc = crate::team_mission_driver::TeamMissionService::new(
            crate::team_mission_driver::SharedMissionState::new(
                crate::team_mission_driver::tests::team_domain().await,
            ),
            crate::team_mission_driver::tests::deps(crate::team_mission_driver::tests::TOOL_PLAN_JSON),
            aivyx_team::default_nonagon(),
            aivyx_core::GatePolicy::Interactive,
        );
        let tm = sched.team_mission.as_ref().expect("set");
        fire_team_mission_schedule(&sched, tm, &svc).await;
        let missions = svc.list();
        assert_eq!(missions.len(), 1);
        assert_eq!(missions[0].triggered_by.as_deref(), Some("cfg-nightly-boh-close"));
    }

    #[test]
    fn config_to_records_builds_a_team_mission_record() {
        let cfg = aivyx_config::ScheduleConfig {
            name: "nightly-boh-close".to_string(),
            cron: "0 0 2 * * * *".to_string(),
            role: "default".to_string(),
            prompt: String::new(),
            enabled: true,
            wrap_mission: false,
            notify_target: None,
            notify_targets: vec![],
            notify_when: aivyx_config::NotifyWhen::Always,
            report_kind: None,
            team_mission: Some(aivyx_config::ScheduledTeamMissionConfig {
                goal: "run the overnight close".to_string(),
                pack_config: None,
            }),
        };
        let records = config_to_records(&[cfg]).expect("valid");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].schedule_id, "cfg-nightly-boh-close");
        assert_eq!(records[0].role_name, "");
        assert_eq!(records[0].prompt, "");
        let tm = records[0].team_mission.as_ref().expect("team_mission set");
        assert_eq!(tm.goal, "run the overnight close");
    }
}

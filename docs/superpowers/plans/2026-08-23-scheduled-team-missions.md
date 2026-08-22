# Scheduled Team Missions (Piece A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an operator (via `aivyx.toml`, or conversationally by asking
the agent) schedule a **team mission** to fire on a cron cadence — closing
the gap where `docs/NONAGON.md`'s own "overnight loop" worked example has no
first-party way to actually run unattended.

**Architecture:** `ScheduleRecord` gains an optional team-mission target,
mutually exclusive with its existing single-agent `role_name`/`prompt`
fields. `fire_schedule` gains a new, fully deterministic dispatch branch
that calls `TeamMissionService` directly — no LLM tool-call in the loop,
unlike the already-possible-but-unreliable path of granting a scheduled
role the `team.run` tool. The resulting `TeamMissionRecord` is tagged with
its originating `schedule_id` for traceability, and the mission's own
existing lifecycle (interactive gate-parking, same as a manually-run
mission) is reused unchanged — no new headless mode. `notify_mission_result`
(the existing Chapter Herald mechanism) is extended to also fire when a
mission parks at a gate (today it only fires on terminal phases) and, for
schedule-triggered missions specifically, to use that schedule's own
configured `notify_targets`/`notify_when` instead of the operator's global
default.

**Tech Stack:** Rust, `serde`/`toml`, the existing `aivyx-channel`/
`aivyx-config`/`aivyx-ipc` crates. No new external dependencies.

## Global Constraints

- **Piece A is scheduling-only.** No changes to channel adapters
  (`aivyx-telegram`/`aivyx-discord`/`aivyx-slack`) or any new capability
  base — those are Piece B/C, each getting its own plan written after this
  piece ships, against the real code as it exists then (not assumed now).
- **No new headless/auto-reject gate mode.** A scheduled mission that hits
  a human gate parks in `AwaitingApproval` exactly like a manually-run one
  — reuse `TeamMissionService::start_from_goal`'s existing interactive
  model, not `run_goal_blocking`'s headless `GatePolicy` (that's a
  different mechanism, built for the autonomous loop, not reused here).
- **Every existing `ScheduleRecord`/`ScheduleConfig`/`TeamMissionRecord`
  field, constructor, and call site not explicitly named in a task below
  stays untouched.** In particular: `register_mission`, `TeamMissionService::
  start`/`start_from_goal`, and every one of their existing callers/tests
  keep their exact current signatures — this plan adds new, additive
  sibling functions rather than changing them, specifically to avoid
  touching the 10+ existing test call sites that construct missions via
  `register_mission`/`svc.start(...)`.
- **`#[serde(default)]` on every new field**, matching this codebase's own
  established pattern for additive fields on `ScheduleRecord`/
  `TeamMissionRecord` (both files use it extensively today) — every
  pre-existing persisted record and every pre-existing TOML config must
  keep deserializing unchanged.
- **The Studio's own dedicated "create a team-mission schedule" GUI form is
  out of scope for this plan.** Task 6 gives the operator a real, complete,
  working path today (conversationally, by asking the agent to schedule a
  team mission — the same way an operator already creates any other
  schedule via `schedule.create`) — the Studio's create-schedule form
  keeps working exactly as it does today (single-agent-turn schedules
  only) and is a reasonable fast-follow, not part of this plan.

---

### Task 1: The team-mission target on the whole config→storage chain

**Files:**
- Modify: `crates/aivyx-config/src/lib.rs:1809-1842` (`ScheduleConfig`)
- Modify: `crates/aivyx-config/src/lib.rs:4087-4114` (`RawSchedule`)
- Modify: `crates/aivyx-config/src/lib.rs:6710-6736` (the RawSchedule→ScheduleConfig loader loop)
- Modify: `crates/aivyx-channel/src/schedule.rs:42-118` (`ScheduleRecord` + `ScheduleRecord::new`)
- Modify: `crates/aivyx-channel/src/daemon_scheduler.rs:46-71` (`config_to_records`)
- Test: `crates/aivyx-channel/src/schedule.rs`'s own `#[cfg(test)]` module (inline, matches this file's existing convention — check the bottom of the file for it)
- Test: `crates/aivyx-config/src/lib.rs`'s own `#[cfg(test)]` module

**Interfaces:**
- Produces: `ScheduledTeamMission { goal: String, pack_config: Option<String> }` (new pub struct, `aivyx-channel`'s `schedule` module) and `aivyx_config::ScheduledTeamMissionConfig { goal: String, pack_config: Option<String> }` (new pub struct, `aivyx-config` crate — kept as a **separate** type from the `aivyx-channel` one, matching how `ScheduleConfig`/`ScheduleRecord` are already two separate types for the same conceptual thing across the config/storage boundary, not one shared type). `ScheduleRecord.team_mission: Option<ScheduledTeamMission>` and `ScheduleConfig.team_mission: Option<ScheduledTeamMissionConfig>`, both `#[serde(default)]`.
- Consumes: nothing new from other tasks (this is the foundational task everything else depends on).

- [ ] **Step 1: Write the failing tests for `ScheduleRecord`'s new mutual-exclusivity rule**

Add to `crates/aivyx-channel/src/schedule.rs`'s existing `#[cfg(test)] mod tests` block (find it — the file already has one, given `create_schedule`/`get_schedule`/etc. are tested there):

```rust
#[test]
fn new_team_mission_builds_a_record_with_no_role_or_prompt() {
    let record = ScheduleRecord::new_team_mission(
        "sched-1".to_string(),
        "0 0 2 * * *".to_string(),
        "run the overnight close".to_string(),
        None,
    )
    .expect("valid cron");
    assert!(record.team_mission.is_some());
    let tm = record.team_mission.as_ref().unwrap();
    assert_eq!(tm.goal, "run the overnight close");
    assert_eq!(tm.pack_config, None);
    assert_eq!(record.role_name, "");
    assert_eq!(record.prompt, "");
}

#[test]
fn new_team_mission_rejects_an_empty_goal() {
    let err = ScheduleRecord::new_team_mission(
        "sched-1".to_string(),
        "0 0 2 * * *".to_string(),
        "".to_string(),
        None,
    );
    assert!(err.is_err());
}

#[test]
fn new_team_mission_still_validates_cron() {
    let err = ScheduleRecord::new_team_mission(
        "sched-1".to_string(),
        "not a cron expression".to_string(),
        "run the overnight close".to_string(),
        None,
    );
    assert!(err.is_err());
}

#[test]
fn a_role_prompt_record_has_no_team_mission() {
    let record = ScheduleRecord::new(
        "sched-2".to_string(),
        "0 0 7 * * *".to_string(),
        "default".to_string(),
        "check system health".to_string(),
    )
    .expect("valid cron");
    assert_eq!(record.team_mission, None);
}

#[test]
fn a_team_mission_record_round_trips_through_json() {
    let record = ScheduleRecord::new_team_mission(
        "sched-3".to_string(),
        "0 0 2 * * *".to_string(),
        "run the overnight close".to_string(),
        Some("crates/verticals/aivyx-kitchen/assets/kitchen-boh.toml".to_string()),
    )
    .expect("valid cron");
    let json = serde_json::to_vec(&record).expect("serialize");
    let back: ScheduleRecord = serde_json::from_slice(&json).expect("deserialize");
    assert_eq!(back.team_mission, record.team_mission);
}

#[test]
fn a_pre_existing_role_prompt_json_record_deserializes_with_no_team_mission() {
    // No "team_mission" key at all -- simulates a record persisted before
    // this field existed.
    let json = br#"{
        "schedule_id": "old-1",
        "cron_expr": "0 0 7 * * *",
        "role_name": "default",
        "prompt": "check system health",
        "enabled": true,
        "wrap_mission": false,
        "created_at": 1000,
        "last_fired_at": null,
        "notify_target": null,
        "notify_targets": [],
        "notify_when": "always",
        "report_kind": null,
        "created_by": "config"
    }"#;
    let record: ScheduleRecord = serde_json::from_slice(json).expect("deserialize old record");
    assert_eq!(record.team_mission, None);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-channel new_team_mission -- --test-threads=1`
Expected: FAIL with `no function or associated item named 'new_team_mission' found` (and `no field 'team_mission' on type 'ScheduleRecord'`) — the struct/constructor don't exist yet.

- [ ] **Step 3: Add `ScheduledTeamMission` + the `team_mission` field + `new_team_mission` to `ScheduleRecord`**

In `crates/aivyx-channel/src/schedule.rs`, add right before `ScheduleRecord`'s own definition (before line 42):

```rust
/// Chapter Muster — a schedule targets EITHER a single-agent turn
/// (`role_name` + `prompt`, the original shape) OR a team mission (this
/// struct, via `ScheduleRecord::new_team_mission`). The two are mutually
/// exclusive: a team-mission record's `role_name`/`prompt` are always
/// empty strings, never read by `fire_schedule`'s team-mission branch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduledTeamMission {
    pub goal: String,
    /// A path to a vertical-pack `TeamConfig` TOML file (e.g.
    /// `crates/verticals/aivyx-kitchen/assets/kitchen-boh.toml`).
    /// `None` -> the daemon's default team. Stored as a path, not a
    /// pre-loaded `TeamConfig`, so `fire_schedule` always loads the
    /// pack's current contents at fire time, not whatever it was when
    /// the schedule was created.
    pub pack_config: Option<String>,
}
```

Then add the new field to `ScheduleRecord` (right after `pub created_by: ScheduleProvenance,`, before the struct's closing brace):

```rust
    /// Chapter Muster — mutually exclusive with `role_name`/`prompt`
    /// (which are empty strings on a team-mission record). `None` ->
    /// this is an ordinary single-agent-turn schedule (every record
    /// before this field existed). `#[serde(default)]` so every
    /// pre-existing persisted record deserializes unchanged.
    #[serde(default)]
    pub team_mission: Option<ScheduledTeamMission>,
```

Update `ScheduleRecord::new` to set the new field to `None` (in the struct literal, alongside the other fields):

```rust
    pub fn new(
        schedule_id: String,
        cron_expr: String,
        role_name: String,
        prompt: String,
    ) -> Result<Self, String> {
        validate_cron(&cron_expr)?;
        Ok(ScheduleRecord {
            schedule_id,
            cron_expr,
            role_name,
            prompt,
            enabled: true,
            wrap_mission: false,
            created_at: now_millis(),
            last_fired_at: None,
            notify_target: None,
            notify_targets: Vec::new(),
            notify_when: aivyx_config::NotifyWhen::Always,
            report_kind: None,
            created_by: ScheduleProvenance::Config,
            team_mission: None,
        })
    }

    /// Chapter Muster — a schedule whose fire target is a team mission,
    /// not a single-agent turn. `role_name`/`prompt` are set to empty
    /// strings (never read by `fire_schedule`'s team-mission branch, and
    /// deliberately not `Option` themselves -- see the file's own
    /// mutual-exclusivity note on `team_mission`).
    pub fn new_team_mission(
        schedule_id: String,
        cron_expr: String,
        goal: String,
        pack_config: Option<String>,
    ) -> Result<Self, String> {
        validate_cron(&cron_expr)?;
        if goal.trim().is_empty() {
            return Err("team-mission schedule requires a non-empty goal".to_string());
        }
        Ok(ScheduleRecord {
            schedule_id,
            cron_expr,
            role_name: String::new(),
            prompt: String::new(),
            enabled: true,
            wrap_mission: false,
            created_at: now_millis(),
            last_fired_at: None,
            notify_target: None,
            notify_targets: Vec::new(),
            notify_when: aivyx_config::NotifyWhen::Always,
            report_kind: None,
            created_by: ScheduleProvenance::Config,
            team_mission: Some(ScheduledTeamMission { goal, pack_config }),
        })
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p aivyx-channel new_team_mission a_role_prompt_record a_team_mission_record a_pre_existing -- --test-threads=1`
Expected: PASS (6 tests).

- [ ] **Step 5: Extend the config-layer types (`RawSchedule`, `ScheduleConfig`) the same way**

In `crates/aivyx-config/src/lib.rs`, add right before `ScheduleConfig`'s own definition (before line 1809):

```rust
/// Chapter Muster — the TOML-layer mirror of `aivyx_channel::schedule::
/// ScheduledTeamMission`. Kept as a separate type (this crate doesn't
/// depend on `aivyx-channel`), same relationship as `ScheduleConfig`/
/// `ScheduleRecord` already have for the rest of a schedule's fields.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledTeamMissionConfig {
    pub goal: String,
    pub pack_config: Option<String>,
}
```

Add the new field to `ScheduleConfig` (after `pub report_kind: Option<String>,`, before the struct's closing brace):

```rust
    /// Chapter Muster — mutually exclusive with `role`/`prompt`. `None`
    /// -> an ordinary single-agent-turn schedule.
    pub team_mission: Option<ScheduledTeamMissionConfig>,
```

Add the matching field to `RawSchedule` (after `report_kind: Option<String>,`, before its closing brace), plus a nested raw struct for the TOML sub-table:

```rust
    /// Chapter Muster — `[schedule.team_mission]` sub-table. Mutually
    /// exclusive with `role`/`prompt` at the loader level (Step 6).
    #[serde(default)]
    team_mission: Option<RawScheduledTeamMission>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct RawScheduledTeamMission {
    goal: String,
    #[serde(default)]
    pack_config: Option<String>,
```

(Note: `RawSchedule`'s own closing `}` moves to right after the new `team_mission` field — the `RawScheduledTeamMission` struct is a new, separate top-level item declared immediately after it, matching this file's existing convention of small private `Raw*` structs living next to the `[[table]]` they parse, e.g. `RawReflectionSchedule` right after `RawSchedule` today.)

Also make `prompt` on `RawSchedule` genuinely optional (today it's a bare `String` with no `#[serde(default)]`, meaning a `[[schedule]]` TOML table that omits `prompt` fails to parse at all — a team-mission-only schedule has no prompt). Find:

```rust
    #[serde(default = "default_role_name")]
    role: String,
    prompt: String,
```

Replace with:

```rust
    #[serde(default = "default_role_name")]
    role: String,
    #[serde(default)]
    prompt: String,
```

- [ ] **Step 6: Validate mutual exclusivity at load time, in the RawSchedule→ScheduleConfig loop**

In the `for r in toml.schedules.unwrap_or_default() { ... }` loop (`crates/aivyx-config/src/lib.rs:6712-6736`), after the existing `resolve_trigger_notify_fields` call and before pushing to `schedules`, add the validation and the new field:

```rust
            let team_mission = match &r.team_mission {
                Some(tm) => {
                    if !r.prompt.trim().is_empty() {
                        return Err(ConfigError::Invalid {
                            field: "schedule.team_mission",
                            reason: format!(
                                "schedule {:?} sets both `prompt` and `[schedule.team_mission]` \
                                 -- a schedule targets one or the other, never both",
                                r.name
                            ),
                        });
                    }
                    if tm.goal.trim().is_empty() {
                        return Err(ConfigError::Invalid {
                            field: "schedule.team_mission.goal",
                            reason: format!(
                                "schedule {:?}'s [schedule.team_mission] needs a non-empty goal",
                                r.name
                            ),
                        });
                    }
                    Some(ScheduledTeamMissionConfig {
                        goal: tm.goal.clone(),
                        pack_config: tm.pack_config.clone(),
                    })
                }
                None => {
                    if r.prompt.trim().is_empty() {
                        return Err(ConfigError::Invalid {
                            field: "schedule.prompt",
                            reason: format!(
                                "schedule {:?} has neither a `prompt` nor a \
                                 `[schedule.team_mission]` -- it needs one or the other",
                                r.name
                            ),
                        });
                    }
                    None
                }
            };
            schedules.push(ScheduleConfig {
                name: r.name,
                cron: r.cron,
                role: r.role,
                prompt: r.prompt,
                enabled: true,
                wrap_mission: r.wrap_mission,
                notify_target: r.notify_target,
                notify_targets,
                notify_when,
                report_kind: r.report_kind,
                team_mission,
```

(the closing `});` and whatever field(s) originally followed `report_kind` stay exactly as they are today — this only inserts the new validation block and the new `team_mission` field into the existing literal.)

- [ ] **Step 7: Write the failing config-loader tests**

Add to `crates/aivyx-config/src/lib.rs`'s own `#[cfg(test)]` module (search for an existing test that constructs a `[[schedule]]` TOML string, e.g. one asserting on `schedules[0].role`, and add these alongside it):

```rust
#[test]
fn schedule_team_mission_loads_with_no_prompt() {
    let toml_str = r#"
        [[schedule]]
        name = "nightly-boh-close"
        cron = "0 0 2 * * * *"
        [schedule.team_mission]
        goal = "run the overnight close"
        pack_config = "crates/verticals/aivyx-kitchen/assets/kitchen-boh.toml"
    "#;
    let settings = Settings::from_toml_str(toml_str).expect("valid config");
    assert_eq!(settings.schedules.len(), 1);
    let tm = settings.schedules[0].team_mission.as_ref().expect("team_mission set");
    assert_eq!(tm.goal, "run the overnight close");
    assert_eq!(
        tm.pack_config.as_deref(),
        Some("crates/verticals/aivyx-kitchen/assets/kitchen-boh.toml")
    );
}

#[test]
fn schedule_rejects_both_prompt_and_team_mission() {
    let toml_str = r#"
        [[schedule]]
        name = "bad"
        cron = "0 0 2 * * * *"
        prompt = "do a thing"
        [schedule.team_mission]
        goal = "run the overnight close"
    "#;
    let err = Settings::from_toml_str(toml_str).unwrap_err();
    assert!(matches!(err, ConfigError::Invalid { field: "schedule.team_mission", .. }));
}

#[test]
fn schedule_rejects_neither_prompt_nor_team_mission() {
    let toml_str = r#"
        [[schedule]]
        name = "bad"
        cron = "0 0 2 * * * *"
    "#;
    let err = Settings::from_toml_str(toml_str).unwrap_err();
    assert!(matches!(err, ConfigError::Invalid { field: "schedule.prompt", .. }));
}
```

Before writing these, **check the real name of the existing helper this file's own schedule-loading tests already use to parse a TOML string into `Settings`** (search the test module for an existing `[[schedule]]`-parsing test and copy its exact helper call — `Settings::from_toml_str` above is illustrative of the shape, not guaranteed to be the real function name; use whatever the file's own existing schedule tests actually call).

- [ ] **Step 8: Run the config tests to verify they fail, then pass**

Run: `cargo test -p aivyx-config schedule_team_mission schedule_rejects -- --test-threads=1`
Expected: FAIL first (missing field/type), then PASS after Step 5/6's changes (3 tests).

- [ ] **Step 9: Update `config_to_records` to carry the new field through**

In `crates/aivyx-channel/src/daemon_scheduler.rs`, `config_to_records` (lines 48-71) branches on whether the config is a team-mission schedule or a role/prompt one — it must call `ScheduleRecord::new_team_mission` for the former (since `ScheduleRecord::new` would reject an empty-string cron-only construction path with no real prompt, and more importantly a team-mission record's `role_name`/`prompt` must stay empty, not silently take whatever `c.role`/`c.prompt` happen to be). Replace the whole function body:

```rust
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
```

- [ ] **Step 10: Write the failing test for `config_to_records`**

Add to `daemon_scheduler.rs`'s own `#[cfg(test)]` module (find the existing tests around `config_to_records` — the module already has some, given the function is old):

```rust
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
```

(If constructing a bare `ScheduleConfig`/`aivyx_config::ScheduledTeamMissionConfig` literal like this doesn't compile because either struct isn't `pub` with all-`pub` fields from this crate's perspective, check the real visibility `aivyx-config` already exports for `ScheduleConfig` — the existing `config_to_records` function already constructs records from `&[aivyx_config::ScheduleConfig]` today, so this exact literal-construction pattern is already proven reachable from `aivyx-channel`'s own test module elsewhere in this file; mirror whatever that existing precedent does.)

- [ ] **Step 11: Run all of Task 1's tests**

Run: `cargo test -p aivyx-channel -p aivyx-config -- --test-threads=1`
Expected: all pass, including the new ones (10 new tests total across both crates). No regressions in either crate's existing suite.

- [ ] **Step 12: Commit**

```bash
git add crates/aivyx-config/src/lib.rs crates/aivyx-channel/src/schedule.rs crates/aivyx-channel/src/daemon_scheduler.rs
git commit -m "feat: ScheduleRecord/ScheduleConfig gain an optional team-mission target

Mutually exclusive with the existing role/prompt single-agent fields,
validated at TOML-load time (schedule.team_mission vs. prompt -- exactly
one, never both, never neither). ScheduleRecord::new_team_mission mirrors
new(), leaving role_name/prompt as empty strings on a team-mission record
rather than making them Option (avoids touching the 10+ existing call
sites that read them as plain strings). config_to_records threads the
new field through the TOML->storage sync path.

No dispatch wiring yet -- fire_schedule doesn't know about this field
until the next task."
```

---

### Task 2: `TeamMissionRecord` gains schedule traceability

**Files:**
- Modify: `crates/aivyx-ipc/src/team_mission.rs` (`TeamMissionRecord` struct + `impl TeamMissionRecord`)
- Test: same file's own `#[cfg(test)]` module

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: `TeamMissionRecord.triggered_by: Option<String>` (`#[serde(default)]`) and `TeamMissionRecord::with_triggered_by(mut self, source: impl Into<String>) -> Self` — a builder mirroring the existing `with_config` exactly, so Task 3 can chain `TeamMissionRecord::new(&id, goal, plan).with_config(config).with_triggered_by(schedule_id)`.

- [ ] **Step 1: Write the failing test**

Add to `crates/aivyx-ipc/src/team_mission.rs`'s own `#[cfg(test)]` module (search for existing tests on `TeamMissionRecord::new`/`with_config` and add alongside them):

```rust
#[test]
fn with_triggered_by_sets_the_field_and_leaves_everything_else_unchanged() {
    let plan = MissionPlan { goal: "g".to_string(), steps: vec![] };
    let record = TeamMissionRecord::new("m1", "goal", plan.clone())
        .with_triggered_by("cfg-nightly-boh-close");
    assert_eq!(record.triggered_by.as_deref(), Some("cfg-nightly-boh-close"));
    assert_eq!(record.id, "m1");
    assert_eq!(record.phase, TeamMissionPhase::Planning);
}

#[test]
fn a_record_with_no_triggered_by_call_has_none() {
    let plan = MissionPlan { goal: "g".to_string(), steps: vec![] };
    let record = TeamMissionRecord::new("m1", "goal", plan);
    assert_eq!(record.triggered_by, None);
}

#[test]
fn a_pre_existing_json_record_deserializes_with_no_triggered_by() {
    // Simulates a record persisted before this field existed -- no
    // "triggered_by" key at all. Use the real current field set (check
    // the struct definition for anything this literal is missing before
    // trusting it -- TeamMissionRecord has grown fields across several
    // chapters, most recently spend_tokens/spend_usd).
    let json = br#"{
        "id": "m1",
        "goal": "goal",
        "plan": {"goal": "goal", "steps": []},
        "outputs": {},
        "phase": "planning",
        "started_at_unix_ms": 1000,
        "updated_at_unix_ms": 1000
    }"#;
    let record: TeamMissionRecord = serde_json::from_slice(json).expect("deserialize old record");
    assert_eq!(record.triggered_by, None);
}
```

(The third test's JSON literal must satisfy every field on the *real* struct that lacks `#[serde(default)]` — re-check `TeamMissionRecord`'s full current field list before trusting the fields listed above; every field shown as `#[serde(default)]` in the earlier investigation, `pending_gate`/`halt_reason`/`config`/`verify_attempts`/`spend_tokens`/`spend_usd`, can be safely omitted from this literal, but `id`/`goal`/`plan`/`phase`/`started_at_unix_ms`/`updated_at_unix_ms` are not `#[serde(default)]` and must be present.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-ipc triggered_by -- --test-threads=1`
Expected: FAIL — `no method named 'with_triggered_by'`, `no field 'triggered_by'`.

- [ ] **Step 3: Add the field and builder**

Add the new field to `TeamMissionRecord` (right after `pub spend_usd: f64,`, before `pub started_at_unix_ms: u64,`):

```rust
    /// Chapter Muster — the id of the `[[schedule]]` entry that started
    /// this mission, if any. `None` for every mission started any other
    /// way (manual `aivyx team run`, the Studio, the autonomous loop's
    /// auto-delegation, ...). `#[serde(default)]` keeps pre-existing
    /// records decoding.
    #[serde(default)]
    pub triggered_by: Option<String>,
```

Add the builder to `impl TeamMissionRecord` (right after `with_config`'s own definition):

```rust
    /// Chapter Muster — tag this mission with the schedule that started
    /// it. Mirrors `with_config`'s exact shape.
    pub fn with_triggered_by(mut self, source: impl Into<String>) -> Self {
        self.triggered_by = Some(source.into());
        self
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p aivyx-ipc triggered_by -- --test-threads=1`
Expected: PASS (3 tests).

- [ ] **Step 5: Run the full `aivyx-ipc` suite to confirm no regressions**

Run: `cargo test -p aivyx-ipc -- --test-threads=1`
Expected: all pass, same total plus 3.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-ipc/src/team_mission.rs
git commit -m "feat: TeamMissionRecord gains triggered_by schedule traceability

A new, purely additive field + builder mirroring with_config exactly.
Nothing constructs it yet -- that's the next task."
```

---

### Task 3: `TeamMissionService` gains a schedule-aware start method

**Files:**
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs`
- Test: same file's own `#[cfg(test)]` module

**Interfaces:**
- Consumes: `TeamMissionRecord::with_triggered_by` (Task 2).
- Produces: `TeamMissionService::start_from_goal_for_schedule(&self, goal: &str, config: Option<TeamConfig>, schedule_id: &str) -> Result<String, MissionDriverError>` — Task 4 calls this from `fire_schedule`.

- [ ] **Step 1: Write the failing test**

Add to `team_mission_driver.rs`'s own `#[cfg(test)]` module, right next to the
existing `team_run_tool_starts_a_mission_via_the_service` test (it already
exercises `start_from_goal` via `TeamRunTool` — mirror its exact
`TeamMissionService::new(...)` construction, shown below):

```rust
#[tokio::test]
async fn start_from_goal_for_schedule_tags_the_record_with_the_schedule_id() {
    let svc = TeamMissionService::new(
        SharedMissionState::new(team_domain().await),
        deps(TOOL_PLAN_JSON),
        default_nonagon(),
        GatePolicy::Interactive,
    );
    let id = svc
        .start_from_goal_for_schedule("do the nightly close", None, "cfg-nightly-boh-close")
        .await
        .expect("starts");
    let record = svc.list().into_iter().find(|r| r.id == id).expect("registered");
    assert_eq!(record.triggered_by.as_deref(), Some("cfg-nightly-boh-close"));
}
```

`team_domain()` (an `async fn team_domain() -> DomainHandle` opening a
throwaway temp-dir `redb` store under `KeyDomain::TeamMissions`), `deps()`
(the `FakeProvider`-backed `TeamRunDeps` fixture), `TOOL_PLAN_JSON` (the
canned plan JSON `deps()` needs so `decompose_goal`'s one LLM call
succeeds), and `default_nonagon()` all already exist in this same test
module — no new fixtures needed for this task.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p aivyx-channel start_from_goal_for_schedule -- --test-threads=1`
Expected: FAIL with `no method named 'start_from_goal_for_schedule'`.

- [ ] **Step 3: Add `register_mission_for_schedule` (free function) and `start_from_goal_for_schedule` (service method)**

Add right after `register_mission`'s own definition (after line 504's closing brace):

```rust
/// Chapter Muster — like [`register_mission`], but tags the resulting
/// record with the schedule that started it. A separate function rather
/// than a new parameter on `register_mission` itself, specifically to
/// avoid touching that function's own 10+ existing call sites (production
/// and test) that have no schedule to tag.
pub async fn register_mission_for_schedule(
    shared: &SharedMissionState,
    plan: MissionPlan,
    id: impl Into<String>,
    config: Option<TeamConfig>,
    schedule_id: &str,
) -> Result<String, MissionDriverError> {
    let id = id.into();
    plan.validate()?;
    let goal = plan.goal.clone();
    shared
        .put(
            TeamMissionRecord::new(&id, goal, plan)
                .with_config(config)
                .with_triggered_by(schedule_id),
        )
        .await?;
    Ok(id)
}
```

Add right after `start_from_goal`'s own definition (`impl TeamMissionService` block, after its closing brace around line 1026):

```rust
    /// Chapter Muster — like [`start_from_goal`], but the resulting
    /// mission is tagged with the schedule that started it (`triggered_by`)
    /// so Mission Control / `aivyx team status` can show "started by
    /// schedule: <id>", and so the notify-on-gate/terminal-phase hook
    /// (Task 5) can look up that schedule's own `notify_targets`.
    pub async fn start_from_goal_for_schedule(
        &self,
        goal: &str,
        config: Option<TeamConfig>,
        schedule_id: &str,
    ) -> Result<String, MissionDriverError> {
        let cancel = aivyx_core::CancellationToken::new();
        let plan = aivyx_team::decompose_goal(
            self.deps.provider.as_ref(),
            &self.deps.model,
            goal,
            config.as_ref().unwrap_or(&self.config),
            &cancel,
            true,
        )
        .await?;
        let id = register_mission_for_schedule(
            &self.state,
            plan,
            uuid::Uuid::new_v4().to_string(),
            config,
            schedule_id,
        )
        .await?;
        self.spawn_drive(id.clone());
        Ok(id)
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p aivyx-channel start_from_goal_for_schedule -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Run the full `aivyx-channel` suite**

Run: `cargo test -p aivyx-channel -- --test-threads=1`
Expected: all pass, no regressions (confirms `register_mission`/`start_from_goal` and their existing callers/tests are genuinely untouched).

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-channel/src/team_mission_driver.rs
git commit -m "feat: TeamMissionService::start_from_goal_for_schedule

New, additive sibling to start_from_goal (same decompose-then-register-
then-spawn shape) that tags the resulting TeamMissionRecord with the
originating schedule id. register_mission/start/start_from_goal and
every existing caller are unchanged -- register_mission_for_schedule is
a new free function, not a new parameter on the existing one.

Nothing calls this yet -- the scheduler doesn't know about it until the
next task."
```

---

### Task 4: `fire_schedule` dispatches team-mission schedules deterministically

**Files:**
- Modify: `crates/aivyx-channel/src/daemon_scheduler.rs`
- Test: same file's own `#[cfg(test)]` module

**Interfaces:**
- Consumes: `ScheduleRecord.team_mission` (Task 1), `TeamMissionService::start_from_goal_for_schedule` (Task 3).
- Produces: `fire_schedule` now takes an additional dependency — a way to reach the daemon's `TeamMissionService`. Task 5 (notify) and any future caller of `fire_schedule`/`run_scheduler` needs this same handle.

**Before writing code — a real finding from this plan's own research, not
assumed:** `fire_schedule` itself has **no existing unit-test precedent
anywhere in this codebase** — `daemon_scheduler.rs`'s own `#[cfg(test)]`
module (confirmed by reading it in full) only tests pure helpers
(`last_fired_or_epoch`, `already_fired_in_window`, `update_earliest`,
`config_to_records`); `trigger.rs`'s own tests likewise only cover pure
functions (`resolve_notify_targets`, `condition_gate_passes`) — nothing
anywhere constructs a real `TriggerDispatch` for a test (it needs a full
`Arc<dyn Agent>` + `ChannelFactory`, `TriggerDispatch::new`'s own real
signature). So: **don't inline the new team-mission logic directly into
`fire_schedule`'s body** (which would make it untestable without being the
first thing in this codebase to build a `TriggerDispatch` test fixture).
Extract it into its own function instead, matching this file's own
established pattern of pure/semi-pure, directly-testable helpers.

- [ ] **Step 1: Write the failing test**

Add to `daemon_scheduler.rs`'s own `#[cfg(test)]` module, next to
`config_to_records_converts_correctly`:

```rust
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
        crate::team_mission_driver::tests::default_nonagon(),
        crate::team_mission_driver::GatePolicy::Interactive,
    );
    let tm = sched.team_mission.as_ref().expect("set");
    fire_team_mission_schedule(&sched, tm, &svc).await;
    let missions = svc.list();
    assert_eq!(missions.len(), 1);
    assert_eq!(missions[0].triggered_by.as_deref(), Some("cfg-nightly-boh-close"));
}
```

**Before this compiles**, `team_domain`/`deps`/`TOOL_PLAN_JSON`/
`default_nonagon` (all private to `team_mission_driver.rs`'s own
`#[cfg(test)]` module today) need to be reachable from
`daemon_scheduler.rs`'s test module — check whether that module is
already `pub(crate)` or its items already `pub(crate)`; if not, the
minimal fix is marking just those four items `pub(crate)` (not the whole
module) in `team_mission_driver.rs`, which is itself worth being its own
tiny first sub-step here since it's a real, necessary, easily-isolated
change — verify by attempting the reference and reading the exact
visibility error, don't guess at what's already exported.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p aivyx-channel fire_team_mission_schedule_starts_a_tagged_mission -- --test-threads=1`
Expected: FAIL — `fire_team_mission_schedule` doesn't exist yet (plus whatever visibility errors Step 1's note above anticipates, fixed first).

- [ ] **Step 3: Add `fire_team_mission_schedule` and call it from `fire_schedule`**

Add a new function right before `fire_schedule`'s own definition:

```rust
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
        Some(path) => match aivyx_team_types::TeamConfig::load(path) {
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
```

**`aivyx_team_types::TeamConfig::load`'s exact import path**: confirm
`aivyx-team-types` is already a dependency of `aivyx-channel` (it almost
certainly is, given `team_mission_driver.rs` in the same crate already
uses `TeamConfig` throughout) — check the top of `daemon_scheduler.rs`
for its current `use` block and add whatever import is missing.

Now wire it into `fire_schedule` itself — add the check as the *first*
thing in the function body (before the existing `report_kind == "digest"`
check, since a team-mission schedule has no `report_kind`/`prompt` to
consider at all), and add the new `team_missions` parameter:

```rust
async fn fire_schedule(
    dispatch: &TriggerDispatch,
    store: &DomainHandle,
    sched: &ScheduleRecord,
    report_ctx: Option<&ReportContext>,
    team_missions: &crate::team_mission_driver::TeamMissionService,
) {
    if let Some(tm) = &sched.team_mission {
        fire_team_mission_schedule(sched, tm, team_missions).await;
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
```

This keeps `fire_schedule`'s own remaining body (the digest/dispatch path)
exactly as untested-at-the-unit-level as it already is today — not a
regression, just not a new problem this task needs to solve either.

Now find `fire_schedule`'s one call site (inside `run_scheduler`, the tick
loop) and thread a `TeamMissionService` handle through `run_scheduler`'s
own signature the same way — **re-read `run_scheduler`'s real current
signature and body first** (not reproduced in this plan; find it in
`daemon_scheduler.rs` directly). Also find and update whatever daemon
startup code in `crates/aivyx-cli/src/bin/aivyx.rs` calls `run_scheduler`
(wherever `run_daemon` spawns the scheduler task) to pass the
already-constructed `TeamMissionService` through, mirroring how
`notify_dispatcher`/`default_notify_target` or similar already-threaded
dependencies reach `run_scheduler` today.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p aivyx-channel fire_team_mission_schedule_starts_a_tagged_mission -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Run the full `aivyx-channel` suite, then the full workspace build**

Run: `cargo test -p aivyx-channel -- --test-threads=1`
Expected: all pass.

Run: `cargo build --workspace --exclude aivyx-desktop --exclude aivyx-web`
Expected: clean — confirms `run_scheduler`'s new parameter didn't break its real caller in `aivyx-cli`'s daemon startup path.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-channel/src/daemon_scheduler.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat: fire_schedule dispatches a team-mission schedule deterministically

New branch, checked first: a schedule with team_mission set calls the new
fire_team_mission_schedule helper (TeamMissionService::
start_from_goal_for_schedule directly -- no LLM tool-call in the loop,
unlike the already-possible-but-unreliable path of granting a scheduled
role the team.run tool), extracted into its own function specifically so
it's unit-testable without constructing a TriggerDispatch (nothing in
this codebase does that today -- see the new test's own comment). A
role/prompt schedule's existing dispatch path (the report_kind=="digest"
check and dispatch.fire(...) call) is byte-identical to before this
change, just reached after the new early-return -- confirmed by diff,
not by a new dedicated test (fire_schedule itself has no unit-test
precedent in this codebase to extend either way).

run_scheduler/fire_schedule both gained a new TeamMissionService
parameter, threaded from the daemon's existing startup wiring."
```

---

### Task 5: Notify on gate-parked and terminal scheduled missions

**Files:**
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs` (`TeamRunDeps`, `notify_mission_result`)
- Test: same file's own `#[cfg(test)]` module

**Interfaces:**
- Consumes: `TeamMissionRecord.triggered_by` (Task 2).
- Produces: `TeamRunDeps.schedule_store: Option<DomainHandle>` (new field, `#[serde]` not applicable — this struct isn't serialized, it's an in-process dependency bag, confirm this against the struct's real current derives before assuming `#[serde(default)]` even applies here).

**Before writing code:** re-read `TeamRunDeps`'s real current derives (does it derive `Clone`/`Default`, or is it hand-constructed everywhere? every field shown during this plan's own research was documented individually with no visible `#[derive(Default)]` — confirm before assuming a `Default` impl exists to lean on) and re-confirm every real call site that *constructs* a `TeamRunDeps` literal (the daemon startup path in `aivyx.rs`, and every test fixture in this file) so the new field can be added to all of them without missing one.

- [ ] **Step 1: Add a local recording notify backend + write the failing tests**

`notify_dispatcher.rs`'s own test module already has an equivalent
(`MockBackend`, records every `send(message, subject)` call and returns a
caller-configurable outcome), but it's private to that file — add a small,
local equivalent to `team_mission_driver.rs`'s own `#[cfg(test)]` module
instead of changing that file's visibility, next to `gated_plan()`:

```rust
/// Chapter Muster — records every `send` call in memory so a test can
/// assert whether a specific registered target was notified. A local,
/// smaller equivalent of `notify_dispatcher.rs`'s own private `MockBackend`
/// (not reused directly -- it's private to that file's own test module).
struct RecordingNotifyBackend {
    calls: std::sync::Mutex<Vec<(String, Option<String>)>>,
}
impl RecordingNotifyBackend {
    fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { calls: std::sync::Mutex::new(Vec::new()) })
    }
}
#[async_trait]
impl crate::notify_dispatcher::NotifyBackend for RecordingNotifyBackend {
    async fn send(
        &self,
        message: &str,
        subject: Option<&str>,
    ) -> Result<(), crate::notify_dispatcher::NotifyError> {
        self.calls.lock().unwrap().push((message.to_string(), subject.map(str::to_string)));
        Ok(())
    }
}
```

Then the tests, next to the backend:

```rust
#[tokio::test]
async fn notify_mission_result_fires_when_a_mission_reaches_awaiting_approval() {
    let backend = RecordingNotifyBackend::new();
    let mut dispatcher = crate::notify_dispatcher::NotifyDispatcher::new();
    dispatcher.register("studio", backend.clone());
    let mut deps = deps("unused");
    deps.notify_dispatcher = Some(std::sync::Arc::new(dispatcher));
    deps.default_notify_target = Some("studio".to_string());

    let mut record = TeamMissionRecord::new("m1", "goal", gated_plan());
    record.phase = TeamMissionPhase::AwaitingApproval;
    record.pending_gate = Some("approve".to_string());
    notify_mission_result(&deps, &record).await;
    assert_eq!(backend.calls.lock().unwrap().len(), 1, "AwaitingApproval now notifies, not just terminal phases");
}

#[tokio::test]
async fn notify_mission_result_still_fires_on_terminal_phases() {
    let backend = RecordingNotifyBackend::new();
    let mut dispatcher = crate::notify_dispatcher::NotifyDispatcher::new();
    dispatcher.register("studio", backend.clone());
    let mut deps = deps("unused");
    deps.notify_dispatcher = Some(std::sync::Arc::new(dispatcher));
    deps.default_notify_target = Some("studio".to_string());

    let mut record = TeamMissionRecord::new("m1", "goal", artifact_plan());
    record.phase = TeamMissionPhase::Done;
    notify_mission_result(&deps, &record).await;
    assert_eq!(backend.calls.lock().unwrap().len(), 1, "no regression on the existing terminal-phase path");
}

#[tokio::test]
async fn notify_mission_result_does_not_fire_on_executing_or_planning() {
    let backend = RecordingNotifyBackend::new();
    let mut dispatcher = crate::notify_dispatcher::NotifyDispatcher::new();
    dispatcher.register("studio", backend.clone());
    let mut deps = deps("unused");
    deps.notify_dispatcher = Some(std::sync::Arc::new(dispatcher));
    deps.default_notify_target = Some("studio".to_string());

    let mut record = TeamMissionRecord::new("m1", "goal", artifact_plan());
    record.phase = TeamMissionPhase::Executing;
    notify_mission_result(&deps, &record).await;
    assert!(backend.calls.lock().unwrap().is_empty(), "still a no-op mid-run");
}

#[tokio::test]
async fn a_schedule_triggered_mission_uses_the_schedules_own_notify_targets() {
    // Two named targets on one dispatcher: "ops-channel" (the schedule's
    // own configured target) and "studio" (the operator's global
    // default). Only the schedule's own target should receive the call.
    let ops_backend = RecordingNotifyBackend::new();
    let studio_backend = RecordingNotifyBackend::new();
    let mut dispatcher = crate::notify_dispatcher::NotifyDispatcher::new();
    dispatcher.register("ops-channel", ops_backend.clone());
    dispatcher.register("studio", studio_backend.clone());

    let store = schedule_domain().await; // Step 1's sibling helper, added alongside this test
    let mut sched = ScheduleRecord::new_team_mission(
        "cfg-nightly-boh-close".to_string(),
        "0 0 2 * * * *".to_string(),
        "run the overnight close".to_string(),
        None,
    )
    .unwrap();
    sched.notify_targets = vec!["ops-channel".to_string()];
    crate::schedule::create_schedule(&store, &sched).await.unwrap();

    let mut deps = deps("unused");
    deps.notify_dispatcher = Some(std::sync::Arc::new(dispatcher));
    deps.default_notify_target = Some("studio".to_string());
    deps.schedule_store = Some(store);

    let mut record = TeamMissionRecord::new("m1", "goal", artifact_plan())
        .with_triggered_by("cfg-nightly-boh-close");
    record.phase = TeamMissionPhase::Done;
    notify_mission_result(&deps, &record).await;

    assert_eq!(ops_backend.calls.lock().unwrap().len(), 1, "used the schedule's own target");
    assert!(studio_backend.calls.lock().unwrap().is_empty(), "not the global default");
}
```

`gated_plan()` and `artifact_plan()` are both existing fixtures in this
same test module (`gated_plan` has a human gate on its `"approve"` step;
`artifact_plan` is a plain linear plan with none — check both bodies
directly before using them, since this plan's own research read
`gated_plan` in full but only confirmed `artifact_plan`'s existence, not
its exact step shape). `deps(line: &str)` is the existing `FakeProvider`
fixture (its `line` argument is irrelevant here since these tests never
drive the mission, only call `notify_mission_result` directly on a
hand-built record — `"unused"` is a deliberately obvious marker, not a
real prompt).

Add one new sibling fixture, next to the existing `team_domain()`:

```rust
async fn schedule_domain() -> DomainHandle {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir()
        .join(format!("aivyx-schedule-notify-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let storage: Arc<dyn Storage> = RedbStorage::open(
        StorageConfig::new(dir.join("store.redb")),
        MasterKey::from_raw([7u8; 32]),
    )
    .await
    .expect("open storage");
    storage.domain(KeyDomain::Schedules)
}
```

(Byte-for-byte `team_domain()`'s own shape, targeting `KeyDomain::Schedules`
instead of `KeyDomain::TeamMissions` — every import it needs is already in
scope in this file, since `team_domain()` uses the same ones.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-channel notify_mission_result_fires_when_a_mission_reaches_awaiting_approval a_schedule_triggered_mission_uses -- --test-threads=1`
Expected: FAIL — `AwaitingApproval` currently no-ops (first test), `schedule_store` field doesn't exist (fourth test won't compile).

- [ ] **Step 3: Add `schedule_store` to `TeamRunDeps` and extend every literal construction site**

Add the field (right after `pub audit_log: Option<Arc<PersistentAuditLog>>,`, before `pub checkpointer`):

```rust
    /// Chapter Muster — the schedule store, so `notify_mission_result` can
    /// look up a schedule-triggered mission's OWN `notify_targets`/
    /// `notify_when` instead of the operator's global default. `None` ⇒
    /// every mission notifies via the global default, same as before this
    /// field existed (harmless on a daemon build that, for whatever
    /// reason, doesn't wire it).
    pub schedule_store: Option<aivyx_storage::DomainHandle>,
```

Update every real `TeamRunDeps { ... }` struct-literal construction site found during this file's own re-verification (the daemon startup path in `aivyx.rs`, and every test fixture in this file that builds one directly rather than via a shared helper) to set `schedule_store: None` — unless a helper function centralizes construction, in which case add the field there once. **Enumerate every real site via `grep -n "TeamRunDeps {" crates/aivyx-channel/src/team_mission_driver.rs crates/aivyx-cli/src/bin/aivyx.rs` before editing**, since this plan's own research didn't exhaustively count them.

- [ ] **Step 4: Extend `notify_mission_result` to fire on `AwaitingApproval` and prefer a schedule's own notify config**

Replace the whole function:

```rust
async fn notify_mission_result(deps: &TeamRunDeps, record: &TeamMissionRecord) {
    if !matches!(
        record.phase,
        TeamMissionPhase::Done
            | TeamMissionPhase::Rejected
            | TeamMissionPhase::Halted
            | TeamMissionPhase::AwaitingApproval
    ) {
        return;
    }
    let Some(dispatcher) = &deps.notify_dispatcher else {
        return;
    };
    // Chapter Muster — a schedule-triggered mission uses that schedule's
    // own configured notify_targets/notify_when instead of the operator's
    // global default, when both a schedule store and a matching record
    // are available. Falls back to the pre-existing global-default
    // behavior for every other mission (manual runs, the autonomous
    // loop's auto-delegation, ...) and for a schedule-triggered mission
    // whose originating schedule was since deleted.
    let (targets, notify_when) = match (&record.triggered_by, &deps.schedule_store) {
        (Some(schedule_id), Some(store)) => {
            match crate::schedule::get_schedule(store, schedule_id).await {
                Ok(Some(sched)) => {
                    let resolved = crate::trigger::resolve_notify_targets(
                        &sched.notify_targets,
                        deps.default_notify_target.as_deref(),
                    );
                    (resolved, sched.notify_when)
                }
                _ => (
                    crate::trigger::resolve_notify_targets(&[], deps.default_notify_target.as_deref()),
                    aivyx_config::NotifyWhen::Always,
                ),
            }
        }
        _ => (
            crate::trigger::resolve_notify_targets(&[], deps.default_notify_target.as_deref()),
            aivyx_config::NotifyWhen::Always,
        ),
    };
    if targets.is_empty() {
        return;
    }
    let _ = notify_when; // Chapter Muster leaves notify_when gating for a later pass -- see Step 5's note.
    let body = render_mission_notify_body(record);
    let subject = format!("mission: {}", record.id);
    let session_id = SessionId::new();
    for target in &targets {
        let outcome = match dispatcher.dispatch(target, &body, Some(&subject)).await {
            Ok(()) => AutoNotifyOutcomeSummary::Delivered,
            Err(e) => crate::trigger::outcome_from_notify_error(&e),
        };
        crate::trigger::emit_auto_notify_audit(
            deps.audit_log.as_deref(),
            session_id,
            crate::trigger::TriggerSource::Mission,
            &record.id,
            target,
            outcome,
        );
    }
}
```

**Note on the `let _ = notify_when;` line**: `condition_gate_passes` (seen in `trigger.rs`, used by `TriggerDispatch::fire`'s own notify path) gates on a single-agent `TurnOutcome`, which has no team-mission equivalent — deciding what "on_failed" / "on_completed_non_empty" *mean* for a team mission (failed = `Rejected`/`Halted`? non-empty = has outputs?) is a real design question this task deliberately does not resolve; every schedule-triggered mission notifies on every one of the four qualifying phases regardless of its own `notify_when` setting for now, which is a conservative, over-notifying default (never silently drops a notification), not a regression from anything that exists today (nothing team-mission-aware existed before this task). Flag this explicitly in the task's own commit message and self-review notes rather than silently deciding it — it's a real, closeable gap, just correctly out of this task's own scope (matches the design doc's own "notify when a mission reaches AwaitingApproval or a terminal phase" requirement without also requiring "and respect notify_when's condition," which the design doc didn't specify precisely enough to implement blind).

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p aivyx-channel notify_mission_result a_schedule_triggered_mission_uses -- --test-threads=1`
Expected: PASS (6 tests: the 4 new ones plus whatever pre-existing `notify_mission_result`-adjacent tests already existed, confirmed still green).

- [ ] **Step 6: Run the full `aivyx-channel` suite**

Run: `cargo test -p aivyx-channel -- --test-threads=1`
Expected: all pass — in particular, confirm no existing test asserted "a mission reaching `AwaitingApproval` never notifies" (if one exists, it needs updating as part of this task, not left contradicting the new intended behavior — check for this specifically, since Step 4 is a deliberate, documented behavior change for every mission, not just schedule-triggered ones).

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-channel/src/team_mission_driver.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat: notify_mission_result fires on AwaitingApproval, prefers a schedule's own notify_targets

A mission parking at a human gate now notifies (previously silent --
only Done/Rejected/Halted fired). Applies to every mission, not just
scheduled ones, since it's the same single call site every mission
already goes through. A schedule-triggered mission's notify dispatch
uses that schedule's own notify_targets/notify_when instead of the
operator's global default, falling back to the global default when no
schedule_store is wired or the originating schedule was deleted.

Known, deliberately out-of-scope gap (documented in the code): every
qualifying phase notifies regardless of notify_when's condition
(always/on_failed/on_completed_non_empty) for team missions specifically
-- deciding what those conditions mean for a team mission (vs. a
single-agent TurnOutcome) is a real design question left for later."
```

---

### Task 6: Author a team-mission schedule conversationally (`schedule.create`)

**Files:**
- Modify: `crates/aivyx-channel/src/schedule_tool.rs` (`ScheduleCreateTool`)
- Test: same file's own `#[cfg(test)]` module

**Interfaces:**
- Consumes: `ScheduleRecord::new_team_mission` (Task 1).
- Produces: nothing new consumed by later tasks — this is the last task in this plan.

**A real finding from this plan's own research, not assumed:** every
existing `ScheduleCreateTool` test in this file only checks `name()`/
`required_scope()`/`input_schema()` — none call `.execute()` today, so
there's no local precedent for constructing a `ToolContext`. The closest
real, working precedent anywhere in this crate is `reminder_tool.rs`'s own
`#[cfg(test)]` module: a `NoopChannel`/`NoopAudit`/`ctx_parts()`/
`make_ctx()` set (`ChannelContext` is a trait object field on
`ToolContext`, so *something* has to implement it — `reminder_tool.rs`'s
version is deliberately minimal, exactly what this task needs, unlike
`team_mission_driver.rs`'s own heavier `MissionLeadChannel`-based one).
Also real and necessary: `schedule_create_name_and_schema`
(`schedule_tool.rs`'s own existing test, asserts `schema["required"]`
contains `"prompt"`) will break once Step 3 makes `prompt` no longer
required — update it as part of this task, not a surprise regression.

- [ ] **Step 1: Add a local test-context fixture + write the failing tests**

Add to `schedule_tool.rs`'s own `#[cfg(test)]` module, mirroring
`reminder_tool.rs`'s exact `NoopChannel`/`NoopAudit`/`ctx_parts`/`make_ctx`
shape (copy that file's real implementation directly rather than
reinventing it — the four functions/structs are small and self-contained):

```rust
// (NoopChannel, NoopAudit, ctx_parts, make_ctx -- copied from
// reminder_tool.rs's own test module, byte-for-byte except the doc
// comment below.)
```

Also add a schedule-store test helper, mirroring `team_mission_driver.rs`'s
own `team_domain()` shape but for `KeyDomain::Schedules`:

```rust
async fn schedule_domain() -> DomainHandle {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir()
        .join(format!("aivyx-schedule-tool-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let storage: Arc<dyn Storage> = RedbStorage::open(
        StorageConfig::new(dir.join("store.redb")),
        MasterKey::from_raw([7u8; 32]),
    )
    .await
    .expect("open storage");
    storage.domain(KeyDomain::Schedules)
}
```

(needs `use aivyx_storage::{Storage, RedbStorage, StorageConfig, KeyDomain};`,
`use aivyx_crypto::MasterKey;`, and `use std::time::{SystemTime, UNIX_EPOCH};`
added to this test module's own `use` block if not already present — check
before assuming, `schedule_tool.rs`'s test module today only tests pure
schema/scope getters and may not have any of these imported yet.)

Then the tests:

```rust
#[tokio::test]
async fn schedule_create_builds_a_team_mission_record() {
    let tool = ScheduleCreateTool::new();
    let store = schedule_domain().await;
    tool.set_schedule_store(store.clone()).unwrap();
    let (ch, audit) = ctx_parts();
    let ctx = make_ctx(&ch, &audit);
    let outcome = tool
        .execute(
            json!({
                "cron": "0 0 2 * * * *",
                "goal": "run the overnight close"
            }),
            &ctx,
        )
        .await;
    let ToolOutcome::Completed { output, .. } = outcome else {
        panic!("expected success, got {outcome:?}");
    };
    let schedule_id = output["schedule_id"].as_str().unwrap().to_string();
    let record = crate::schedule::get_schedule(&store, &schedule_id)
        .await
        .unwrap()
        .unwrap();
    assert!(record.team_mission.is_some());
    assert_eq!(record.team_mission.as_ref().unwrap().goal, "run the overnight close");
}

#[tokio::test]
async fn schedule_create_rejects_both_prompt_and_goal() {
    let tool = ScheduleCreateTool::new();
    let store = schedule_domain().await;
    tool.set_schedule_store(store).unwrap();
    let (ch, audit) = ctx_parts();
    let ctx = make_ctx(&ch, &audit);
    let outcome = tool
        .execute(
            json!({
                "cron": "0 0 2 * * * *",
                "prompt": "check system health",
                "goal": "run the overnight close"
            }),
            &ctx,
        )
        .await;
    assert!(matches!(outcome, ToolOutcome::Failed(_)));
}

#[tokio::test]
async fn schedule_create_rejects_neither_prompt_nor_goal() {
    let tool = ScheduleCreateTool::new();
    let store = schedule_domain().await;
    tool.set_schedule_store(store).unwrap();
    let (ch, audit) = ctx_parts();
    let ctx = make_ctx(&ch, &audit);
    let outcome = tool.execute(json!({"cron": "0 0 2 * * * *"}), &ctx).await;
    assert!(matches!(outcome, ToolOutcome::Failed(_)));
}
```

And fix the now-stale existing test — find `schedule_create_name_and_schema`
and remove its `"prompt"`-is-required assertion (it moves from required to
optional in Step 3 below):

```rust
    #[test]
    fn schedule_create_name_and_schema() {
        let tool = ScheduleCreateTool::new();
        assert_eq!(tool.name(), "schedule.create");
        let schema = tool.input_schema();
        assert!(schema["required"].as_array().unwrap().contains(&json!("cron")));
        assert!(!schema["required"].as_array().unwrap().contains(&json!("prompt")));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-channel schedule_create_builds_a_team_mission schedule_create_rejects -- --test-threads=1`
Expected: FAIL — `execute` currently requires `prompt` unconditionally (Step 1's first test), and neither new-input-shape test has real behavior to check yet.

- [ ] **Step 3: Extend the schema and `execute`**

Update the JSON schema in `ScheduleCreateTool::new` — replace the `properties`/`required` block:

```rust
            schema: json!({
                "type": "object",
                "properties": {
                    "cron": {
                        "type": "string",
                        "description": "Cron expression (7-field: sec min hour dom month dow year). Example: \"0 0 9 * * * *\" for daily at 09:00 UTC."
                    },
                    "role": {
                        "type": "string",
                        "description": "Role name to run the scheduled turn under. Ignored if `goal` is set."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The prompt text submitted as a turn when the schedule fires. Mutually exclusive with `goal` -- set exactly one."
                    },
                    "goal": {
                        "type": "string",
                        "description": "Instead of `prompt`, delegate this goal to a durable team mission (the Nonagon) when the schedule fires, rather than a single-agent turn. Mutually exclusive with `prompt` -- set exactly one."
                    },
                    "pack_config": {
                        "type": "string",
                        "description": "Only meaningful with `goal`: a path to a vertical-pack team config TOML file. Omit to use the daemon's default team."
                    }
                },
                "required": ["cron"]
            }),
```

Update `execute`'s body — replace the input-parsing/validation block (from `let cron = ...` through the `if prompt.is_empty() { ... }` check):

```rust
        let cron = input
            .get("cron")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if cron.is_empty() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "schedule.create requires a non-empty `cron` field".to_string(),
            });
        }

        let prompt = input.get("prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let goal = input.get("goal").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let pack_config = input
            .get("pack_config")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let record = match (prompt.is_empty(), goal.is_empty()) {
            (false, false) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: "schedule.create: set exactly one of `prompt` or `goal`, not both"
                        .to_string(),
                });
            }
            (true, true) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: "schedule.create requires exactly one of `prompt` or `goal`"
                        .to_string(),
                });
            }
            (true, false) => {
                let schedule_id = uuid::Uuid::new_v4().to_string();
                match crate::schedule::ScheduleRecord::new_team_mission(
                    schedule_id,
                    cron,
                    goal,
                    pack_config,
                ) {
                    Ok(r) => r,
                    Err(e) => {
                        return ToolOutcome::Failed(AivyxError::Tool { tool: self.id, detail: e });
                    }
                }
            }
            (false, true) => {
                let role = input
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string();
                let schedule_id = uuid::Uuid::new_v4().to_string();
                match crate::schedule::ScheduleRecord::new(schedule_id, cron, role, prompt) {
                    Ok(r) => r,
                    Err(e) => {
                        return ToolOutcome::Failed(AivyxError::Tool { tool: self.id, detail: e });
                    }
                }
            }
        };
```

**Before finalizing this replacement**, re-read the rest of the original `execute` body past the removed block (it presumably does more — growth-gating via `self.growth`, `with_provenance(ScheduleProvenance::Agent)`, persisting via `create_schedule`, building the `ToolOutcome::Completed` response) and keep every bit of that logic, adjusting only the now-already-constructed `record` variable's origin (previously built inline from `cron`/`role`/`prompt` directly via `ScheduleRecord::new` inside this same block — now built above via the match) rather than removing any of it. The exact original code past this point isn't reproduced here since this plan's own research didn't need to change it — read it directly before editing.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p aivyx-channel schedule_create -- --test-threads=1`
Expected: PASS, including every pre-existing `schedule_create_*` test (the `prompt`-only path must still work byte-identically for an operator/agent that doesn't know about `goal` at all).

- [ ] **Step 5: Run the full `aivyx-channel` suite, then clippy**

Run: `cargo test -p aivyx-channel -- --test-threads=1`
Expected: all pass.

Run: `cargo clippy -p aivyx-channel --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-channel/src/schedule_tool.rs
git commit -m "feat: schedule.create can target a team mission (goal + optional pack_config)

Mutually exclusive with prompt, same rule as the ScheduleRecord/
ScheduleConfig layers below it. An operator can now ask the agent
conversationally (\"schedule the overnight close every night at 2am\")
and get a real, working, deterministically-dispatched scheduled team
mission -- the Studio's own dedicated create-schedule form keeps working
exactly as it does today (out of scope for this plan, a reasonable
fast-follow)."
```

---

## Final Verification (whole plan)

- [ ] `cargo build --workspace --exclude aivyx-desktop --exclude aivyx-web` — clean.
- [ ] `cargo test --workspace --exclude aivyx-desktop --exclude aivyx-web -- --test-threads=1` — all green; report the exact total vs. the pre-plan baseline (re-run `cargo test --workspace --exclude aivyx-desktop --exclude aivyx-web -- --test-threads=1` on `main` before Task 1 starts and record that number, so the delta is real, not assumed).
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — clean (the pre-commit hook already enforces this; confirm it explicitly here too).
- [ ] Manually confirm (by reading, since this can't be driven live in this environment) the full path: a `[[schedule]]` TOML entry with `[schedule.team_mission]` loads without error → the daemon syncs it into the store on boot (`sync_config_schedules`) → `run_scheduler`'s tick fires it at the right time → `fire_schedule`'s new branch calls `start_from_goal_for_schedule` → a `TeamMissionRecord` appears tagged `triggered_by` → if it hits a gate, `notify_mission_result` fires using the schedule's own `notify_targets`.
- [ ] Confirm this plan's own Global Constraints held: `git diff --stat main..HEAD` should show changes confined to `crates/aivyx-config/src/lib.rs`, `crates/aivyx-channel/src/{schedule.rs,daemon_scheduler.rs,team_mission_driver.rs,schedule_tool.rs}`, `crates/aivyx-ipc/src/team_mission.rs`, and `crates/aivyx-cli/src/bin/aivyx.rs` (the daemon-startup wiring touched by Tasks 4/5) — no channel-adapter crate, no new capability base, no Studio (`aivyx-web`) changes.

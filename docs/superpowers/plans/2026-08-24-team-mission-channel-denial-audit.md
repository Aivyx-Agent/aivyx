# Team-Mission Channel Denial Audit Event Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Write a denied channel-triggered `/team run` attempt to the
persistent audit chain (not just `eprintln!`), closing the last deferred
finding from Piece C of Team-Mission Triggers' own final review.

**Architecture:** One new `AuditEvent::TeamMissionChannelDenied` variant,
appended in `handle_run_team_mission_channel`'s existing denial branch
using the exact same pattern its own success branch already uses for
`TeamMissionChannelTriggered`.

**Tech Stack:** Rust, `aivyx-audit`/`aivyx-channel` crates already in the
workspace. No new dependencies.

## Global Constraints

- The existing `eprintln!` warning in the denial branch stays — the new
  audit append is additive, not a replacement.
- The two existing `handle_run_team_mission_channel` tests
  (`handle_run_team_mission_channel_denies_before_ever_checking_the_service`,
  `handle_run_team_mission_channel_reports_no_service_when_authorized_but_absent`)
  must be re-run and confirmed passing **unchanged**.
- The new test must be a genuine mutation-proof, shown to actually fail if
  the new `log.append(...)` call is removed.
- `cargo build -p aivyx-audit -p aivyx-channel` clean. Do **not** run
  `cargo build --workspace`/`cargo test --workspace` — the full workspace
  has an unrelated, pre-existing, out-of-scope build failure
  (`javascriptcoregtk-4.1` missing system library in a GUI crate).
- **Environment note for whoever executes this**: this session has hit a
  severe, intermittent `/tmp` scratch-space exhaustion that can break Bash
  commands unpredictably, including silently dropping Bash heredoc
  content while still reporting success. If any command's output looks
  empty/wrong, or a heredoc-written file doesn't contain what you expect,
  re-read the file to confirm before trusting the command's own exit
  code. Prefer the `Write`/`Edit` tools over heredocs for file content if
  this recurs, and redirect command output to a file under `/home` (then
  `Read` it) if the harness's own output capture fails.

---

### Task 1: Add `TeamMissionChannelDenied` and wire it into the denial branch

**Files:**
- Modify: `crates/aivyx-audit/src/lib.rs` — `AuditEvent` enum (new
  variant), the display-name match arm's counterpart in
  `aivyx-channel` (see below), a new round-trip test in this crate's own
  test module.
- Modify: `crates/aivyx-channel/src/daemon_server.rs` —
  `handle_run_team_mission_channel`'s denial branch, the exhaustive
  `AuditEvent` display-name match statement (search for
  `aivyx_audit::AuditEvent::TeamMissionChannelTriggered { .. } =>
  "TeamMissionChannelTriggered",` — currently at line ~6593), a new
  mutation-proof test in this file's own test module.

Re-run these greps before editing — line numbers were current as of this
plan's writing but may have drifted:

```bash
cd /home/julian/Projects/Rust/aivyx
grep -n "pub enum AuditEvent\|TeamMissionChannelTriggered {" crates/aivyx-audit/src/lib.rs
grep -n "team_mission_channel_triggered_round_trips" crates/aivyx-audit/src/lib.rs
grep -n "async fn handle_run_team_mission_channel\|Finding I3(b)\|TeamMissionChannelTriggered { .. } =>" crates/aivyx-channel/src/daemon_server.rs
grep -n "handle_run_team_mission_channel_denies_before_ever_checking_the_service" crates/aivyx-channel/src/daemon_server.rs
```

Confirm the shapes below still match; use whatever's actually current for
every step if they've drifted.

**Interfaces:**
- Produces: `AuditEvent::TeamMissionChannelDenied { platform: String,
  goal: String, reason: String }` — a new variant on the existing,
  already-`pub` `AuditEvent` enum in `aivyx-audit`.
- Consumes: nothing new from elsewhere — this task is self-contained
  across the two files.

- [ ] **Step 1: Write the failing tests**

In `crates/aivyx-audit/src/lib.rs`'s own test module, immediately after
the existing `team_mission_channel_triggered_round_trips` test (search
for it — inside the `// ---- Piece C (2026-08-23) —
TeamMissionChannelTriggered variant ----` section), add:

```rust
    #[test]
    fn team_mission_channel_denied_round_trips() {
        let event = AuditEvent::TeamMissionChannelDenied {
            platform: "telegram".to_string(),
            goal: "close the books".to_string(),
            reason: "channel not authorized via team_run_channel in aivyx.toml".to_string(),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        let back: AuditEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(event, back);
    }
```

In `crates/aivyx-channel/src/daemon_server.rs`'s own test module,
immediately after
`handle_run_team_mission_channel_denies_before_ever_checking_the_service`
(search for it), add:

```rust
    #[tokio::test]
    async fn handle_run_team_mission_channel_denial_is_audited() {
        use aivyx_crypto::MasterKey;
        use aivyx_storage::{RedbStorage, StorageConfig};

        let dir = std::env::temp_dir()
            .join(format!("aivyx-team-run-denial-audit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([7u8; 32]),
        )
        .await
        .expect("open storage");
        let log = PersistentAuditLog::open(storage, [9u8; 32])
            .await
            .expect("open chain");

        let authz = ChannelTriggerAuthz::default(); // all false
        let resp = handle_run_team_mission_channel(
            None,
            &authz,
            Some(aivyx_core::ChannelPlatform::Telegram),
            Some(&log),
            "close the books".to_string(),
        )
        .await;
        match resp {
            DaemonMessage::Error { code, .. } => assert_eq!(code, "team_run_channel_denied"),
            other => panic!("expected Error(team_run_channel_denied), got {other:?}"),
        }

        let entries = log.entries().expect("read chain");
        assert_eq!(entries.len(), 1, "the denial must append exactly one entry");
        match &entries[0].event {
            aivyx_audit::AuditEvent::TeamMissionChannelDenied { platform, goal, reason } => {
                assert_eq!(platform, "telegram");
                assert_eq!(goal, "close the books");
                assert!(
                    reason.contains("team_run_channel"),
                    "reason should name the config key an operator needs to set: {reason}"
                );
            }
            other => panic!("expected TeamMissionChannelDenied, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd /home/julian/Projects/Rust/aivyx
cargo test -p aivyx-audit --lib team_mission_channel_denied_round_trips -- --test-threads=1
cargo test -p aivyx-channel --lib handle_run_team_mission_channel_denial_is_audited -- --test-threads=1
```

Expected: both FAIL — compile errors, `AuditEvent::TeamMissionChannelDenied`
doesn't exist yet.

- [ ] **Step 3: Add the new variant to `AuditEvent`**

In `crates/aivyx-audit/src/lib.rs`, immediately after
`TeamMissionChannelTriggered`'s closing `},` (still inside the `AuditEvent`
enum, before its own closing `}`), add:

```rust
    /// Piece C follow-up (2026-08-24) — a channel's native `/team run
    /// <goal>` command was refused (the channel isn't opted into
    /// `team_run_channel`). Sibling of `TeamMissionChannelTriggered` —
    /// same shape minus `mission_id` (nothing was created), plus
    /// `reason` for the refusal.
    TeamMissionChannelDenied {
        /// Same convention as `TeamMissionChannelTriggered::platform`.
        platform: String,
        goal: String,
        reason: String,
    },
```

- [ ] **Step 4: Wire the denial branch and the display-name match arm**

In `crates/aivyx-channel/src/daemon_server.rs`, change
`handle_run_team_mission_channel`'s denial branch from:

```rust
    if !channel_trigger_authorized(authz, platform) {
        // Finding I3(b) — a denied `/team run` otherwise leaves zero
        // forensic trace (only successful starts are audited via
        // `TeamMissionChannelTriggered`). No new `AuditEvent` variant
        // here (out of scope for this fix); a startup-log-style
        // eprintln is the narrowest fix.
        eprintln!(
            "aivyx daemon: /team run denied for channel {} (not authorized via \
             team_run_channel in aivyx.toml)",
            channel_trigger_audit_platform(platform)
        );
        return DaemonMessage::Error {
            code: "team_run_channel_denied".into(),
            message: "this channel is not authorized to start team missions (operator \
                      opt-in required via team_run_channel in aivyx.toml)"
                .into(),
        };
    }
```

to:

```rust
    if !channel_trigger_authorized(authz, platform) {
        // Finding I3(b), closed 2026-08-24 — a denied `/team run` used to
        // leave zero forensic trace beyond this eprintln. Now also
        // appended to the persistent audit chain, mirroring the success
        // branch's own TeamMissionChannelTriggered pattern below.
        eprintln!(
            "aivyx daemon: /team run denied for channel {} (not authorized via \
             team_run_channel in aivyx.toml)",
            channel_trigger_audit_platform(platform)
        );
        if let Some(log) = audit_log {
            if let Err(e) = log.append(aivyx_audit::AuditEvent::TeamMissionChannelDenied {
                platform: channel_trigger_audit_platform(platform),
                goal: goal.clone(),
                reason: "channel not authorized via team_run_channel in aivyx.toml".into(),
            }) {
                eprintln!("aivyx daemon: failed to audit denied channel team trigger: {e}");
            }
        }
        return DaemonMessage::Error {
            code: "team_run_channel_denied".into(),
            message: "this channel is not authorized to start team missions (operator \
                      opt-in required via team_run_channel in aivyx.toml)"
                .into(),
        };
    }
```

Then find the exhaustive `AuditEvent` display-name match statement
(search for `aivyx_audit::AuditEvent::TeamMissionChannelTriggered { .. }
=> "TeamMissionChannelTriggered",`) and add immediately after it:

```rust
        aivyx_audit::AuditEvent::TeamMissionChannelDenied { .. } => "TeamMissionChannelDenied",
```

This match has no wildcard arm — the compiler will refuse to build until
this arm exists, which is the intended safety net for never forgetting a
variant here again.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd /home/julian/Projects/Rust/aivyx
cargo test -p aivyx-audit --lib team_mission_channel_denied_round_trips -- --test-threads=1
cargo test -p aivyx-channel --lib handle_run_team_mission_channel_denial_is_audited -- --test-threads=1
```

Expected: both PASS.

- [ ] **Step 6: Perform the mutation-proof**

Temporarily comment out the new `if let Some(log) = audit_log { ... }`
block you added in Step 4 (leave the `eprintln!` above it and the
`return DaemonMessage::Error { ... }` below it untouched), re-run:

```bash
cargo test -p aivyx-channel --lib handle_run_team_mission_channel_denial_is_audited -- --test-threads=1
```

Expected: FAILS — `entries.len()` is `0`, not `1` (the assertion
`assert_eq!(entries.len(), 1, ...)` fails). This confirms the test
genuinely detects the bug it's named for. Then restore the block and
confirm the test passes again. Record both outputs in your report.

- [ ] **Step 7: Run the full test suites and builds**

```bash
cd /home/julian/Projects/Rust/aivyx
cargo test -p aivyx-audit --lib -- --test-threads=1
cargo test -p aivyx-channel --lib -- --test-threads=1
cargo build -p aivyx-audit -p aivyx-channel
cargo clippy -p aivyx-audit --lib -- -D warnings
cargo clippy -p aivyx-channel --lib -- -D warnings
```

Expected: `aivyx-audit` passes at baseline + 1 new test; `aivyx-channel`
passes at baseline + 1 new test, with both pre-existing
`handle_run_team_mission_channel_*` tests unchanged and passing; clean
builds; clippy clean except the pre-existing, unrelated `trigger.rs:229`
finding.

- [ ] **Step 8: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx
git add crates/aivyx-audit/src/lib.rs crates/aivyx-channel/src/daemon_server.rs
git commit -m "Audit denied channel-triggered /team run attempts

Closes the last deferred finding from Piece C's own final review: a
denied trigger left zero forensic trace beyond an eprintln. Adds
AuditEvent::TeamMissionChannelDenied, appended in
handle_run_team_mission_channel's existing denial branch alongside
the eprintln (both stay -- live warning plus permanent record),
mirroring the success branch's own TeamMissionChannelTriggered
pattern exactly.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

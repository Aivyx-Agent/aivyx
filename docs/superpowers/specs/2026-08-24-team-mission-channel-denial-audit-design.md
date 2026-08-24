# Team-Mission Channel Denial Audit Event Design

## Motivation

Piece C of Team-Mission Triggers (shipped 2026-08-23) shipped
channel-triggered `/team run <goal>`, gated by a per-channel
`team_run_channel` opt-in the daemon enforces. Its own final review found
a real, deliberately-scoped-out gap: a denied attempt is logged via
`eprintln!` but never written to the persistent audit chain — only
successful starts get an `AuditEvent::TeamMissionChannelTriggered`. For a
security-gated feature, denial forensics arguably belong in the audit
trail too — an operator investigating "did someone try to run something
unauthorized" today has only a live stderr stream to check, not a durable
record.

## Re-verified framing (2026-08-24)

- `crates/aivyx-channel/src/daemon_server.rs`'s `handle_run_team_mission_channel`
  is the **single** real authorization + start decision for this feature —
  confirmed fresh: all three channel adapters (Telegram, Discord, Slack)
  send the identical `FrontendMessage::RunTeamMissionChannel { goal }`,
  dispatched through exactly one shared match arm that calls this
  function. There is no separate per-channel denial path to update.
- The success path, in the same function, already establishes the exact
  pattern to mirror: `if let Some(log) = audit_log { if let Err(e) =
  log.append(AuditEvent::TeamMissionChannelTriggered { .. }) {
  eprintln!(...) } }`.
- Recording the denied attempt's `goal` text is safe and consistent with
  the existing precedent: the audit log is an operator-only forensic
  record (read via the Studio's `ListAuditEntries`/`VerifyAuditChain`
  queries), never surfaced back to the sender — the same reasoning that
  already justifies recording `goal` for successful triggers applies
  identically to denied ones.
- No existing `AuditEvent` variant is a clean fit to reuse.
  `HeadlessRefusal` is the closest sibling but a structurally different
  concept (which run-path hit a human-approval gate, not who was denied
  authorization to start something).

## Architecture

Add one new variant to `crates/aivyx-audit/src/lib.rs`'s `AuditEvent`
enum, immediately after its sibling `TeamMissionChannelTriggered`, with a
doc comment in the same style:

```rust
/// Piece C follow-up (2026-08-24) — a channel's native `/team run <goal>`
/// command was refused (the channel isn't opted into `team_run_channel`).
/// Sibling of `TeamMissionChannelTriggered` — same shape minus
/// `mission_id` (nothing was created), plus `reason` for the refusal.
TeamMissionChannelDenied {
    /// Same convention as `TeamMissionChannelTriggered::platform`.
    platform: String,
    goal: String,
    reason: String,
},
```

In `handle_run_team_mission_channel`'s denial branch, add the append call
immediately alongside the existing `eprintln!` (both stay — the live
stderr warning is what an operator watching the daemon's own logs sees
immediately; the audit entry is the permanent forensic record; neither
replaces the other):

```rust
if let Some(log) = audit_log {
    if let Err(e) = log.append(aivyx_audit::AuditEvent::TeamMissionChannelDenied {
        platform: channel_trigger_audit_platform(platform),
        goal: goal.clone(),
        reason: "channel not authorized via team_run_channel in aivyx.toml".into(),
    }) {
        eprintln!("aivyx daemon: failed to audit denied channel team trigger: {e}");
    }
}
```

Delete the now-outdated "Finding I3(b)... out of scope for this fix"
comment — the gap it describes is what this design closes.

Add the corresponding display-name match arm wherever
`TeamMissionChannelTriggered` is mapped to a human-readable name (the
`daemon_server.rs` match arm found during research, used by whatever
audit-query/display surface consumes it).

## Testing

- New test: a denied trigger genuinely appends
  `AuditEvent::TeamMissionChannelDenied` to the audit chain, with the
  correct `platform`/`goal`/`reason` fields. Mutation-proof: must fail if
  the new `log.append(...)` call is removed.
- The two existing `handle_run_team_mission_channel` tests
  (`..._denies_before_ever_checking_the_service`,
  `..._reports_no_service_when_authorized_but_absent`) must keep passing
  unchanged.
- A round-trip serialization test for the new variant, matching the
  existing pattern `aivyx-audit`'s own test module already uses for its
  sibling variants (parse → serialize → deserialize → equal).

## Out of scope

- No change to the existing `eprintln!` warning's own text or the
  `DaemonMessage::Error` response shape returned to the denied sender.
- No new query/display surface for reading denial events specifically —
  they land on the same audit chain existing queries already read.
- No change to `channel_trigger_authorized`'s own authorization logic.

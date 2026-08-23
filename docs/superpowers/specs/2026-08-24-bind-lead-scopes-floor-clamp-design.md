# `bind_lead_scopes` Floor-Clamp Design

## Motivation

Piece A of the Team-Mission Triggers initiative (shipped 2026-08-23,
`docs/superpowers/specs/2026-08-23-team-mission-triggers-design.md`)
shipped with a real Critical security finding at its own final review:
`schedule.create`'s model-facing `pack_config` parameter let the model
point a scheduled mission at an arbitrary `TeamConfig` file, whose own
declared lead-role `capability_scopes` got unioned — not intersected —
into the daemon's real, operator-authorized capability floor via
`bind_lead_scopes` (`crates/aivyx-channel/src/team_mission_driver.rs`).
Fixed at the time by removing `pack_config` from the model-facing tool
only; the deeper root cause in `bind_lead_scopes` itself was
deliberately left unfixed and logged to the ecosystem backlog. This
design closes it.

## Re-verified framing (2026-08-24)

The concrete exploit path from Piece A's own finding is confirmed
**closed**: `schedule_tool.rs`'s own test
(`schedule_create_schema_has_no_pack_config`) and its real current JSON
schema confirm the model-facing `schedule.create` tool has no
`pack_config` parameter at all. The model-facing `team.run` tool
(`TeamRunTool`) was also checked fresh — its schema is `{"goal":
string}` only, and its `execute` hard-codes
`service.start_from_goal(goal, None)`, so the model has no path to name
a pack there either.

Today, the only real call sites that can reach `bind_lead_scopes` with a
non-default `TeamConfig` are all operator-controlled: the CLI's own
`aivyx team run --config <pack.toml>` flag, `aivyx.toml`'s own
`[schedule.team_mission] pack_config` (Piece A's own operator-authored
path, unaffected by that piece's fix), and the Studio's own
team-roster-editor re-read (`team_roster_applied` in
`daemon_server.rs`). None are model-reachable or otherwise
untrusted-actor-reachable.

**This is therefore not a live exploit — it's a genuine defense-in-depth
gap** for a real, plausible threat model this codebase's own "vertical
pack" extensibility design invites: an operator who installs a
**third-party-authored** vertical pack without having fully audited its
own declared `capability_scopes`. `TeamConfig::load` only validates that
scopes *parse*, never that they're authorized against anything a real
operator config established — so a pack's own file could silently grant
its lead role more than the operator's real trust-tier floor intended,
and the operator would have no easy way to notice without reading the
pack's own scope declarations line by line.

## What already exists (verified against real code)

`bind_lead_scopes` (`team_mission_driver.rs:1357-1405`) takes
`lead_scopes: &[String]` — confirmed, via the real call site in
`crates/aivyx-cli/src/bin/aivyx.rs:8909`
(`lead_scopes: backcompat_floor...`), to be **the daemon's own real,
operator-configured authority** (its own comment: *"the daemon's real
authority, so the team lead can grant specialists the concrete scopes
their tools need"*) — not anything pack-derived. This confirms "the
floor" genuinely represents the operator's own authorized ceiling, and
clamping a pack's own declarations to it is architecturally sound.

The function already has two branches, and **the specialist branch
already does the right thing**:

```rust
fn bind_lead_scopes(config: &mut TeamConfig, lead_scopes: &[String]) {
    if lead_scopes.is_empty() {
        return;
    }
    let lead_name = config.lead.clone();
    for m in &mut config.members {
        if m.name == lead_name {
            // BUG: unconditional union — the pack's own declared scopes
            // for the lead role can add anything, not just orchestration
            // markers, regardless of whether the floor grants it.
            let mut caps: Vec<String> = lead_scopes.to_vec();
            caps.extend(m.capability_scopes.iter().cloned());
            caps.sort();
            caps.dedup();
            m.capability_scopes = caps;
        } else {
            // Specialists already do this correctly: floor scopes
            // filtered to the roles this specialist's own declared
            // scope bases cover, PLUS only team.message/team.delegate/
            // qualified-mcp markers from the pack's own declarations —
            // never an arbitrary domain scope beyond the floor.
            let mut bases: std::collections::HashSet<&str> =
                m.capability_scopes.iter().map(|s| scope_base(s)).collect();
            if m.tool_allowlist.iter().any(|t| t.starts_with("workspace.")) {
                bases.insert("workspace");
            }
            let mut caps: Vec<String> = lead_scopes
                .iter()
                .filter(|s| bases.contains(scope_base(s)))
                .cloned()
                .collect();
            for s in &m.capability_scopes {
                let b = scope_base(s);
                if b == "team.message"
                    || b == "team.delegate"
                    || (b.starts_with("mcp.") && s.contains(':'))
                {
                    caps.push(s.clone());
                }
            }
            caps.sort();
            caps.dedup();
            m.capability_scopes = caps;
        }
    }
}
```

Existing tests (`bind_lead_scopes_grants_per_role_and_stays_least_privilege`,
`bind_lead_scopes_flows_qualified_mcp_grants_to_declaring_roles`) encode
real, intentional, currently-tested behavior that must not break: *"Lead
holds the FULL floor (so it can grant) + its own orchestration
scopes."* Neither test exercises a pack declaring an out-of-floor domain
scope for the lead role specifically, so neither is expected to change.

## Architecture

Change the lead branch to mirror the specialist branch's own filtering:
the lead still receives the **entire** real floor unconditionally (that
property is correct and required — the lead needs full floor authority
to grant onward to specialists) — but the pack's own declared
`capability_scopes` for the lead role are now filtered to the same
narrow allowlist the specialist branch already uses (`team.message`,
`team.delegate`, qualified `mcp.call:<server>:*`), instead of being
unioned in wholesale. Any domain scope (`fs.*`, `shell.*`, `net.*`,
etc.) a pack declares for the lead beyond what's already in the floor
is dropped.

**No other files change.** The floor-clamping decision belongs where
the floor is actually visible — `bind_lead_scopes` is the one place
that has both the pack's own declarations and the real floor in scope
at once. `TeamConfig::load`'s own job stays parsing only, matching its
existing, correct separation of concerns.

**Observability:** when the clamp actually removes a scope the pack
declared (i.e. the pack asked for something the floor doesn't already
grant), `bind_lead_scopes` logs a clear warning (`eprintln!`, matching
this codebase's own established convention for daemon-side warnings)
naming exactly which scope(s) were dropped and why — firing only when a
real clamp happens, not on every call, so an operator installing an
unaudited third-party pack gets a genuine signal rather than log noise.

## Testing

- Existing tests must keep passing unchanged.
- New test: a pack's lead role declares a domain scope the floor never
  granted (e.g. `shell.exec:cwd:/whatever/**`) — confirm it's absent
  from the lead's final `capability_scopes`, while floor scopes and
  legitimate orchestration markers still flow through exactly as
  before.
- New test: the warning fires when (and only when) a clamp actually
  removes something — not on every call, and not when the pack's own
  declarations are already a subset of what the floor grants.

## Out of scope

- No changes to `TeamConfig::load`'s own parsing/validation.
- No new audit-event/protocol surface for the clamp — a warning log is
  sufficient given this is a defense-in-depth fix, not an active
  exploit closure.
- No broader capability-system redesign. The specialist branch's own
  filtering pattern already proves this narrow shape is sufficient; this
  design doesn't invent a new mechanism, it extends an existing correct
  one to a branch that never got it.

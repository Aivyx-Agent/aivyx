# Extend TurnSafety to Close the Team-Mission Injection-Scan Gap

**Status: approved, ready for implementation planning.**

## Context

Phase 199 shipped `[agent] injection_scan_enabled`/`injection_scan_exempt`,
gating Chapter Picket's active injection scan, and threaded them into
the two `ConcreteAgent::new(...)` sites its own plan named
(`daemon_agent`, `child_agent` in `crates/aivyx-cli/src/bin/aivyx.rs`).
Its final review found team missions build agents through a separate
path this branch never touched, and PHASE_199.md's own retrospective
logged the gap as "`SpecialistFactory::new` is constructed from 4
separate files across `aivyx-team`."

That estimate was wrong. Fresh, exhaustive grepping this session found
3 of those 4 files (`pool.rs`, `runtime.rs`, `testutil.rs`) are
test-only fixtures inside `#[cfg(test)]`/`mod tests` blocks — not
production code. The real gap, confirmed by tracing every production
`ConcreteAgent::new(...)` call site in the whole codebase, is exactly
**three** construction paths:

1. `crates/aivyx-channel/src/session.rs`'s `build_agent_stack()` — a
   shared function feeding multiple channel-session construction
   sites (confirmed real callers: `session.rs` itself and
   `crates/aivyx-cli/src/bin/aivyx.rs`, both via an `AgentStackSpec`
   struct literal). Already calls `turn_safety.apply(agent)` before
   returning.
2. `crates/aivyx-cli/src/bin/aivyx_modules/team.rs`'s `run_mission()`
   — the CLI's own `aivyx team run` lead agent, calling
   `TurnSafety::autonomous().apply(agent)` with zero parameters today.
3. `crates/aivyx-team/src/factory.rs`'s `SpecialistFactory::build()` —
   every team specialist, reached from both `team.rs`'s CLI path (via
   `crates/aivyx-team/src/assembly.rs`'s `TeamAssembly::build()`) and
   the daemon's team-mission path
   (`crates/aivyx-channel/src/team_mission_driver.rs`, which has zero
   `ConcreteAgent::new`/`TurnSafety::` references of its own — it only
   calls `TeamAssembly::build()`). Also calls
   `TurnSafety::autonomous().apply(agent)` with zero parameters.

`TeamAssembly::build()` and `run_mission()` both already thread
`checkpointer: Option<Arc<GitCheckpointer>>` as a direct parameter to
solve this exact class of problem for checkpointing — that's the
template this spec follows for the two new values.

## Approach

Extend `TurnSafety` (`crates/aivyx-core/src/agent.rs`) — the
codebase's own existing choke point, whose doc comment already states
its purpose is preventing exactly this class of bug ("that ad-hoc
duplication is exactly what previously left the daemon, the
role-switch child, and the team agents unprotected") — rather than
adding direct per-site builder calls at each of the three gaps. Per
the user's explicit choice, Phase 199's two already-covered sites
(`daemon_agent`, `child_agent`) are also converted to route through
the extended `TurnSafety` constructors, so all five real construction
sites in the system end up going through the exact same mechanism —
no special cases, and Phase 199's now-redundant direct `.with_*` calls
are removed.

## 1. `TurnSafety` itself (`crates/aivyx-core/src/agent.rs`)

```rust
#[derive(Clone, Debug, Default)]
pub struct TurnSafety {
    turn_timeout: Option<Duration>,
    cycle_config: Option<CycleConfig>,
    injection_scan_enabled: bool,
    injection_scan_exempt: std::collections::BTreeSet<String>,
}

impl TurnSafety {
    pub fn interactive(
        turn_timeout_secs: Option<u64>,
        cycle_detection: Option<bool>,
        injection_scan_enabled: bool,
        injection_scan_exempt: std::collections::BTreeSet<String>,
    ) -> Self {
        Self {
            turn_timeout: turn_timeout_secs.map(Duration::from_secs),
            cycle_config: cycle_detection
                .unwrap_or(false)
                .then(CycleConfig::default_enabled),
            injection_scan_enabled,
            injection_scan_exempt,
        }
    }

    pub fn autonomous(
        injection_scan_enabled: bool,
        injection_scan_exempt: std::collections::BTreeSet<String>,
    ) -> Self {
        Self {
            turn_timeout: None,
            cycle_config: Some(CycleConfig::default_enabled()),
            injection_scan_enabled,
            injection_scan_exempt,
        }
    }

    pub fn apply(&self, agent: ConcreteAgent) -> ConcreteAgent {
        let agent = agent.with_cycle_detection(self.cycle_config.clone());
        let agent = agent
            .with_injection_scan_enabled(self.injection_scan_enabled)
            .with_injection_scan_exempt(self.injection_scan_exempt.clone());
        match self.turn_timeout {
            Some(d) => agent.with_turn_timeout(d),
            None => agent,
        }
    }
}
```

`Default` still derives correctly (`bool::default() == false`,
`BTreeSet::default()` is empty) — only relevant to any test construction
that uses `TurnSafety::default()` directly rather than one of the two
named constructors; no such production call exists today.

## 2. `daemon_agent`/`child_agent` (Phase 199's sites, converted)

Both already call `TurnSafety::interactive(turn_timeout_secs,
cycle_detection).apply(agent)` immediately after their own construction
chain. Add the two new arguments to that existing call
(`injection_scan_enabled`, `injection_scan_exempt` are already local
`bool`/`BTreeSet<String>` bindings from Phase 199's own unwrap), and
remove the now-redundant `.with_injection_scan_enabled(...)`/
`.with_injection_scan_exempt(...)` calls Phase 199 added directly to
each construction chain — `TurnSafety::apply` now does this for both.

## 3. `build_agent_stack`'s two `AgentStackSpec` construction sites

Both already build their `turn_safety` field as
`aivyx_core::TurnSafety::interactive(turn_timeout_secs, cycle_detection)`,
with `turn_timeout_secs`/`cycle_detection` already in local scope at
each site. Add the same two new arguments. `build_agent_stack` itself
needs no changes — it already calls `turn_safety.apply(agent)`.

## 4. `SpecialistFactory` (`crates/aivyx-team/src/factory.rs`)

Two new fields + builder methods, mirroring the existing
`checkpointer: Option<Arc<GitCheckpointer>>` field and its
`with_checkpointer` builder exactly:

```rust
injection_scan_enabled: bool,
injection_scan_exempt: std::collections::BTreeSet<String>,
```

```rust
pub fn with_injection_scan_enabled(mut self, enabled: bool) -> Self {
    self.injection_scan_enabled = enabled;
    self
}

pub fn with_injection_scan_exempt(
    mut self,
    exempt: std::collections::BTreeSet<String>,
) -> Self {
    self.injection_scan_exempt = exempt;
    self
}
```

`SpecialistFactory::new`'s initializer gets `injection_scan_enabled:
true, injection_scan_exempt: BTreeSet::new()` (matching every other
default-preserving field). The `build()` method's
`TurnSafety::autonomous().apply(agent)` call becomes
`TurnSafety::autonomous(self.injection_scan_enabled,
self.injection_scan_exempt.clone()).apply(agent)`.

## 5. `TeamAssembly::build()` (`crates/aivyx-team/src/assembly.rs`)

Two new parameters, positioned next to the existing `checkpointer`
parameter (matching its exact type shape at the call level — plain
`bool`/`BTreeSet<String>`, not `Option`-wrapped, since both already
have real defaults at every caller). Threaded into the
`SpecialistFactory::new(...)` builder chain via
`.with_injection_scan_enabled(...)`/`.with_injection_scan_exempt(...)`,
alongside the existing `.with_checkpointer(...)` call.

## 6. `run_mission()` (`crates/aivyx-cli/src/bin/aivyx_modules/team.rs`)

Two new parameters, mirroring the existing `checkpointer` parameter
exactly. Used for two things: (a) `TurnSafety::autonomous(...)` for the
lead's own `ConcreteAgent` (replacing the current zero-argument call),
and (b) passed straight through to `TeamAssembly::build(...)`'s two new
parameters. The caller (`crates/aivyx-cli/src/bin/aivyx.rs:8180`) is
inside `run_async`'s scope where both values are already unwrapped
locally from Phase 199's own work — passing them is a two-argument
addition to an existing call, nothing new to thread from further back.

## 7. `TeamRunDeps` (`crates/aivyx-channel/src/team_mission_driver.rs`)

Two new fields, mirroring the existing `pub checkpointer:
Option<Arc<GitCheckpointer>>` field. `TeamRunDeps` has exactly one real
production construction site
(`crates/aivyx-cli/src/bin/aivyx.rs:9006`), inside the same `run_async`
scope the two values are already unwrapped in — a two-field addition to
an existing struct literal, no new plumbing. `team_mission_driver.rs`'s
own `TeamAssembly::build(...)` call gets `deps.injection_scan_enabled`/
`deps.injection_scan_exempt.clone()` added alongside its existing
`deps.checkpointer.clone()` argument.

## Testing

- **`aivyx-core`**: update `TurnSafety`'s existing unit tests
  (`crates/aivyx-core/src/agent.rs:~3315-3331`, which call
  `TurnSafety::interactive`/`autonomous` directly) to pass the two new
  arguments; add one new test confirming `TurnSafety::autonomous(false,
  {})` produces a `ConcreteAgent` with the scan disabled (mirroring
  Phase 199's own `injection_scan_disabled_globally_skips_the_scan_but_
  still_fences` test, but going through `TurnSafety` instead of the
  direct builder).
- **`aivyx-team`**: one new test on `SpecialistFactory` confirming
  `with_injection_scan_enabled(false)` produces a specialist agent with
  the scan disabled (mirroring the existing `checkpointer` test
  pattern in the same file, if one exists — ground this at plan time).
- **`aivyx-cli`**: no new unit tests planned (integration is covered by
  the crate-level tests above plus the existing `cargo build`/`cargo
  test` verification every task in this codebase's plans already runs)
  — flag at plan time if grounding finds an existing integration test
  this change should extend instead.

## Self-review

- **Placeholder scan:** none — every struct, field, method signature,
  and call-site change is given concretely, grounded against the real
  current file content for all 3 previously-uncovered sites plus the
  2 already-covered ones being converted.
- **Internal consistency:** all 5 real construction sites end up
  calling the same two `TurnSafety::interactive`/`autonomous`
  constructors with the same two new arguments in the same order,
  and the same `apply()` method applies them identically everywhere.
- **Scope check:** one phase, seven well-bounded touch points across
  3 crates (`aivyx-core`, `aivyx-team`, `aivyx-cli`) plus one shared
  file in a 4th (`aivyx-channel`) — comparable in size to Phase 199
  itself, not larger, now that the real (corrected, smaller) scope is
  known.
- **Ambiguity check:** the "convert Phase 199's already-working sites
  too, for full consistency" question was confirmed with the user
  during brainstorming, not assumed.

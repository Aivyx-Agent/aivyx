# Extend TurnSafety to Close the Team-Mission Injection-Scan Gap — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend `TurnSafety` to carry `injection_scan_enabled`/
`injection_scan_exempt`, and thread it through every real production
`ConcreteAgent` construction site in the system, closing the gap where
team missions (both the CLI's `aivyx team run` and the daemon's
team-mission driver) don't yet receive the operator's injection-scan
config.

**Architecture:** One shared struct (`TurnSafety`) gains two fields and
two new constructor parameters; every one of its 6 real call sites
(4 `interactive`, 2 `autonomous`) is updated to pass the two new
values, which are already available as local `bool`/`BTreeSet<String>`
bindings at every call site (from Phase 199's own config-unwrapping
work) or trivially threaded one hop further via existing parameters
that already carry `checkpointer` the same way (`TeamAssembly::build`,
`run_mission`, `TeamRunDeps`, `SpecialistFactory`).

**Tech Stack:** Rust, existing `aivyx-core`/`aivyx-team`/`aivyx-cli`/
`aivyx-channel` crates, `cargo test`.

## Global Constraints

- Full design: `docs/superpowers/specs/2026-09-06-turnsafety-injection-scan-propagation-design.md`.
- `injection_scan_enabled`/`injection_scan_exempt` are plain
  `bool`/`BTreeSet<String>` everywhere in this propagation — never
  `Option`-wrapped. Every caller already has a real value.
- After this plan, **no direct `.with_injection_scan_enabled(...)`/
  `.with_injection_scan_exempt(...)` calls should exist anywhere** —
  every real construction site applies them exclusively via
  `TurnSafety::interactive(...)`/`TurnSafety::autonomous(...)` +
  `.apply(agent)`. Phase 199's two direct calls (on `daemon_agent`/
  `child_agent`) are removed in Task 2.
- The complete, exhaustively-grepped inventory of every real (non-test)
  `TurnSafety::interactive`/`TurnSafety::autonomous` call site in the
  whole codebase (confirmed via `grep -rn "TurnSafety::interactive(\|
  TurnSafety::autonomous(" --include="*.rs" .` on the real checkout,
  excluding the 4 test-only calls in `crates/aivyx-core/src/agent.rs`'s
  own `mod tests`):
  1. `crates/aivyx-cli/src/bin/aivyx.rs:8619` — `child_agent`
  2. `crates/aivyx-cli/src/bin/aivyx.rs:9119` — `daemon_agent`
  3. `crates/aivyx-cli/src/bin/aivyx.rs:9818` — inside a `SessionConfig
     { ... }` struct literal's `turn_safety` field (flows into
     `AgentStackSpec::from_session_config` → `build_agent_stack`)
  4. `crates/aivyx-cli/src/bin/aivyx.rs:10305` — inside an
     `aivyx_channel::AgentStackSpec { ... }` struct literal's
     `turn_safety` field (flows directly into `build_agent_stack`)
  5. `crates/aivyx-team/src/factory.rs:214` — `SpecialistFactory::build`
  6. `crates/aivyx-cli/src/bin/aivyx_modules/team.rs:308` —
     `run_mission`'s lead agent

---

## Task 1: Extend `TurnSafety` itself (`aivyx-core`)

**Files:**
- Modify: `crates/aivyx-core/src/agent.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `TurnSafety::interactive(turn_timeout_secs: Option<u64>,
  cycle_detection: Option<bool>, injection_scan_enabled: bool,
  injection_scan_exempt: BTreeSet<String>) -> Self` and
  `TurnSafety::autonomous(injection_scan_enabled: bool,
  injection_scan_exempt: BTreeSet<String>) -> Self` — every other task
  in this plan calls one of these two by this exact new signature.
  `TurnSafety::apply` keeps its existing signature
  (`&self, agent: ConcreteAgent) -> ConcreteAgent`) but now also
  applies the two new fields internally.

- [ ] **Step 1: Add the two new fields and update both constructors**

Find this exact block:

```rust
#[derive(Clone, Debug, Default)]
pub struct TurnSafety {
    turn_timeout: Option<Duration>,
    cycle_config: Option<CycleConfig>,
}

impl TurnSafety {
    /// Interactive posture: inherit the operator's `[agent]` settings
    /// (`turn_timeout_secs`, `cycle_detection`). Both unset → the built-in
    /// defaults (120s deadline, no cycle breaker) — i.e. byte-identical to a
    /// bare `ConcreteAgent`. Used by the REPL, voice, daemon, and the
    /// role-switch child (all run under a watching operator).
    pub fn interactive(turn_timeout_secs: Option<u64>, cycle_detection: Option<bool>) -> Self {
        Self {
            turn_timeout: turn_timeout_secs.map(Duration::from_secs),
            cycle_config: cycle_detection
                .unwrap_or(false)
                .then(CycleConfig::default_enabled),
        }
    }

    /// Autonomous posture (team / mission agents): the small-cycle breaker is a
    /// built-in floor (always on) because no human watches each turn to cancel a
    /// runaway. The per-turn deadline keeps the built-in 120s default.
    pub fn autonomous() -> Self {
        Self {
            turn_timeout: None,
            cycle_config: Some(CycleConfig::default_enabled()),
        }
    }

    /// Apply the knobs to a freshly constructed agent — the single choke point.
    /// Every `ConcreteAgent::new(...)` site ends with
    /// `TurnSafety::<posture>(...).apply(agent)`.
    pub fn apply(&self, agent: ConcreteAgent) -> ConcreteAgent {
        let agent = agent.with_cycle_detection(self.cycle_config.clone());
        match self.turn_timeout {
            Some(d) => agent.with_turn_timeout(d),
            None => agent,
        }
    }
}
```

Replace with:

```rust
#[derive(Clone, Debug, Default)]
pub struct TurnSafety {
    turn_timeout: Option<Duration>,
    cycle_config: Option<CycleConfig>,
    injection_scan_enabled: bool,
    injection_scan_exempt: std::collections::BTreeSet<String>,
}

impl TurnSafety {
    /// Interactive posture: inherit the operator's `[agent]` settings
    /// (`turn_timeout_secs`, `cycle_detection`, `injection_scan_enabled`,
    /// `injection_scan_exempt`). All unset → the built-in defaults (120s
    /// deadline, no cycle breaker, scan on, no exemptions) — i.e.
    /// byte-identical to a bare `ConcreteAgent`. Used by the REPL, voice,
    /// daemon, and the role-switch child (all run under a watching
    /// operator).
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

    /// Autonomous posture (team / mission agents): the small-cycle breaker is a
    /// built-in floor (always on) because no human watches each turn to cancel a
    /// runaway. The per-turn deadline keeps the built-in 120s default.
    /// `injection_scan_enabled`/`injection_scan_exempt` still come from the
    /// operator's own `[agent]` config — team missions are exactly the
    /// unattended case Chapter Picket's tripwire exists for, so they honor
    /// the same posture as every other agent, not a hardcoded always-on.
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

    /// Apply the knobs to a freshly constructed agent — the single choke point.
    /// Every `ConcreteAgent::new(...)` site ends with
    /// `TurnSafety::<posture>(...).apply(agent)`.
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

- [ ] **Step 2: Update the two existing tests**

Find this exact block:

```rust
    #[test]
    fn turn_safety_interactive_maps_config() {
        // Unset → built-in defaults (no override, no breaker) = bare agent.
        let off = TurnSafety::interactive(None, None);
        assert_eq!(off.turn_timeout, None);
        assert_eq!(off.cycle_config, None);
        // default() agrees — the no-op posture used by paths without config.
        assert_eq!(TurnSafety::default().turn_timeout, None);
        assert_eq!(TurnSafety::default().cycle_config, None);
        // Set → mapped to Duration + the enabled CycleConfig.
        let on = TurnSafety::interactive(Some(300), Some(true));
        assert_eq!(on.turn_timeout, Some(Duration::from_secs(300)));
        assert_eq!(on.cycle_config, Some(CycleConfig::default_enabled()));
        // cycle_detection = Some(false) is off, like None.
        assert_eq!(TurnSafety::interactive(None, Some(false)).cycle_config, None);
    }

    #[test]
    fn turn_safety_autonomous_forces_the_breaker_floor() {
        let a = TurnSafety::autonomous();
        // Cycle breaker always on (the floor); deadline keeps the 120s default.
        assert_eq!(a.cycle_config, Some(CycleConfig::default_enabled()));
        assert_eq!(a.turn_timeout, None);
    }
```

Replace with:

```rust
    #[test]
    fn turn_safety_interactive_maps_config() {
        // Unset → built-in defaults (no override, no breaker) = bare agent.
        let off = TurnSafety::interactive(None, None, true, std::collections::BTreeSet::new());
        assert_eq!(off.turn_timeout, None);
        assert_eq!(off.cycle_config, None);
        // default() agrees — the no-op posture used by paths without config.
        assert_eq!(TurnSafety::default().turn_timeout, None);
        assert_eq!(TurnSafety::default().cycle_config, None);
        // Set → mapped to Duration + the enabled CycleConfig.
        let on = TurnSafety::interactive(Some(300), Some(true), true, std::collections::BTreeSet::new());
        assert_eq!(on.turn_timeout, Some(Duration::from_secs(300)));
        assert_eq!(on.cycle_config, Some(CycleConfig::default_enabled()));
        // cycle_detection = Some(false) is off, like None.
        assert_eq!(
            TurnSafety::interactive(None, Some(false), true, std::collections::BTreeSet::new())
                .cycle_config,
            None
        );
    }

    #[test]
    fn turn_safety_autonomous_forces_the_breaker_floor() {
        let a = TurnSafety::autonomous(true, std::collections::BTreeSet::new());
        // Cycle breaker always on (the floor); deadline keeps the 120s default.
        assert_eq!(a.cycle_config, Some(CycleConfig::default_enabled()));
        assert_eq!(a.turn_timeout, None);
    }

    #[test]
    fn turn_safety_autonomous_carries_the_injection_scan_posture_through_apply() {
        // Mirrors Phase 199's own
        // `injection_scan_disabled_globally_skips_the_scan_but_still_fences`
        // test, but going through TurnSafety instead of the direct builder
        // — proves the choke point applies the knob, not just stores it.
        let mut exempt = std::collections::BTreeSet::new();
        exempt.insert("test.exempt".to_string());
        let safety = TurnSafety::autonomous(false, exempt.clone());
        assert_eq!(safety.injection_scan_enabled, false);
        assert_eq!(safety.injection_scan_exempt, exempt);
    }
```

- [ ] **Step 3: Run the crate's tests**

Run: `cargo test -p aivyx-core turn_safety`
Expected: compile error before Task 2-7's call sites are updated (every
other real call site still uses the old 2-arg signatures) — this is
expected and resolved as later tasks land. If you want to verify this
step's own test logic is sound before other call sites compile, run
`cargo check -p aivyx-core` in isolation and confirm the only errors
are at call sites this task does not own (`agent.rs`'s own 2 test call
sites should already compile clean at this point; errors should only
appear in other files' code, if `cargo check --workspace` is run
instead of scoping to `-p aivyx-core`).

Run: `cargo check -p aivyx-core`
Expected: clean (this crate owns both the definition and its own
tests; no other crate's call sites live here).

- [ ] **Step 4: Commit**

```bash
git add crates/aivyx-core/src/agent.rs
git commit -m "feat(aivyx-core): extend TurnSafety to carry the injection-scan posture

TurnSafety::interactive/autonomous both gain injection_scan_enabled/
injection_scan_exempt parameters; apply() now calls the two
ConcreteAgent builders Phase 199 already added. This is the single
choke point every real ConcreteAgent::new(...) site in the system will
route through (Tasks 2-7) instead of per-site direct builder calls.

Breaks every other real call site in the workspace until Tasks 2-7
land -- expected, tracked in the plan's dependency graph."
```

---

## Task 2: Convert `daemon_agent`/`child_agent` (`aivyx-cli`)

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs`

**Interfaces:**
- Consumes: `TurnSafety::interactive(..., bool, BTreeSet<String>)` (Task 1).
- Produces: nothing new (removes Phase 199's now-redundant direct calls).

- [ ] **Step 1: Convert `child_agent`**

Find this exact block:

```rust
        .with_tool_allowlist(child_tool_allowlist)
        .with_memory_topic_prefix(child_memory_topic_prefix)
        .with_checkpointer(checkpointer_for_factory.clone())
        .with_injection_scan_enabled(injection_scan_enabled)
        .with_injection_scan_exempt(injection_scan_exempt_for_factory.clone());
        let child_agent = aivyx_core::TurnSafety::interactive(turn_timeout_secs, cycle_detection)
            .apply(child_agent);
```

Replace with:

```rust
        .with_tool_allowlist(child_tool_allowlist)
        .with_memory_topic_prefix(child_memory_topic_prefix)
        .with_checkpointer(checkpointer_for_factory.clone());
        let child_agent = aivyx_core::TurnSafety::interactive(
            turn_timeout_secs,
            cycle_detection,
            injection_scan_enabled,
            injection_scan_exempt_for_factory.clone(),
        )
        .apply(child_agent);
```

- [ ] **Step 2: Convert `daemon_agent`**

Find this exact block:

```rust
        .with_tool_allowlist(daemon_tool_allowlist)
        .with_memory_topic_prefix(memory_topic_prefix)
        .with_budget_gate(daemon_budget_gate)
        .with_rate_gate(daemon_rate_gate)
        .with_checkpointer(checkpointer.clone())
        .with_injection_scan_enabled(injection_scan_enabled)
        .with_injection_scan_exempt(injection_scan_exempt.clone());
        let daemon_agent = aivyx_core::TurnSafety::interactive(turn_timeout_secs, cycle_detection)
            .apply(daemon_agent);
```

Replace with:

```rust
        .with_tool_allowlist(daemon_tool_allowlist)
        .with_memory_topic_prefix(memory_topic_prefix)
        .with_budget_gate(daemon_budget_gate)
        .with_rate_gate(daemon_rate_gate)
        .with_checkpointer(checkpointer.clone());
        let daemon_agent = aivyx_core::TurnSafety::interactive(
            turn_timeout_secs,
            cycle_detection,
            injection_scan_enabled,
            injection_scan_exempt.clone(),
        )
        .apply(daemon_agent);
```

- [ ] **Step 3: Build**

Run: `cargo build -p aivyx-cli --bin aivyx 2>&1 | grep -A5 "TurnSafety::interactive"`
Expected: any remaining error at this point should only reference the
two NOT-yet-converted call sites at lines ~9818/~10305 (Task 3) — not
`child_agent`/`daemon_agent`, which this task already fixed.

- [ ] **Step 4: Commit**

```bash
git add crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat(aivyx-cli): route daemon_agent/child_agent through the extended TurnSafety

Removes Phase 199's now-redundant direct .with_injection_scan_enabled/
.with_injection_scan_exempt calls -- TurnSafety::apply now does this
for both, matching cycle_detection/turn_timeout's existing pattern.
Behavior is unchanged (same values, same effect); only the mechanism
changed, closing the class of gap that left build_agent_stack and team
agents uncovered."
```

---

## Task 3: Convert the two `SessionConfig`/`AgentStackSpec` sites (`aivyx-cli`)

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs`

**Interfaces:**
- Consumes: `TurnSafety::interactive(..., bool, BTreeSet<String>)` (Task 1).
- Produces: nothing new.

- [ ] **Step 1: Convert the `SessionConfig` site (~line 9818)**

Find this exact block:

```rust
                // Chapter Bridle (BR.4) — operator override for the
                // Per-turn safety knobs from `[agent]` (deadline + cycle
                // breaker), applied uniformly in `build_agent_stack`.
                turn_safety: aivyx_core::TurnSafety::interactive(
                    turn_timeout_secs,
                    cycle_detection,
                ),
            };
```

Replace with:

```rust
                // Chapter Bridle (BR.4) — operator override for the
                // Per-turn safety knobs from `[agent]` (deadline + cycle
                // breaker + injection-scan posture), applied uniformly in
                // `build_agent_stack`.
                turn_safety: aivyx_core::TurnSafety::interactive(
                    turn_timeout_secs,
                    cycle_detection,
                    injection_scan_enabled,
                    injection_scan_exempt.clone(),
                ),
            };
```

- [ ] **Step 2: Convert the `AgentStackSpec` site (~line 10305)**

Find this exact block:

```rust
                    // Same per-turn safety knobs as the Local path (deadline +
                    // cycle breaker), applied uniformly in `build_agent_stack`.
                    turn_safety: aivyx_core::TurnSafety::interactive(
                        turn_timeout_secs,
                        cycle_detection,
                    ),
                    checkpointer: checkpointer.clone(),
                };
```

Replace with:

```rust
                    // Same per-turn safety knobs as the Local path (deadline +
                    // cycle breaker + injection-scan posture), applied
                    // uniformly in `build_agent_stack`.
                    turn_safety: aivyx_core::TurnSafety::interactive(
                        turn_timeout_secs,
                        cycle_detection,
                        injection_scan_enabled,
                        injection_scan_exempt.clone(),
                    ),
                    checkpointer: checkpointer.clone(),
                };
```

- [ ] **Step 3: Build**

Run: `cargo build -p aivyx-cli --bin aivyx 2>&1 | grep -A5 "TurnSafety::"`
Expected: any remaining error should only reference `aivyx-team`'s two
`autonomous()` call sites (Tasks 4-6) — every `interactive()` call site
in `aivyx.rs` is now converted.

- [ ] **Step 4: Commit**

```bash
git add crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat(aivyx-cli): thread the injection-scan posture into SessionConfig/AgentStackSpec

Both feed into build_agent_stack's single existing turn_safety.apply()
call -- no changes needed there. Closes the channel-session half of
the team-mission-adjacent coverage gap (Local REPL + voice paths that
go through build_agent_stack rather than the daemon's own agent)."
```

---

## Task 4: Add the two new fields/builders to `SpecialistFactory` (`aivyx-team`)

**Files:**
- Modify: `crates/aivyx-team/src/factory.rs`

**Interfaces:**
- Consumes: `TurnSafety::autonomous(bool, BTreeSet<String>)` (Task 1).
- Produces: `SpecialistFactory::with_injection_scan_enabled(bool) -> Self`
  and `SpecialistFactory::with_injection_scan_exempt(BTreeSet<String>)
  -> Self` — Task 5 calls both by these exact names.

- [ ] **Step 1: Add the two new fields**

Find this exact block:

```rust
    /// `aivyx-checkpoint` — attached to every built specialist so an
    /// fs_root-mutating tool call it makes gets checkpointed, same as the
    /// lead agent. `None` (the default) preserves pre-checkpoint behavior.
    checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
```

Replace with:

```rust
    /// `aivyx-checkpoint` — attached to every built specialist so an
    /// fs_root-mutating tool call it makes gets checkpointed, same as the
    /// lead agent. `None` (the default) preserves pre-checkpoint behavior.
    checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
    /// Chapter Picket team-mission follow-up — threaded into
    /// `TurnSafety::autonomous(...)` for every specialist this factory
    /// builds, so team missions honor the same operator `[agent]`
    /// injection-scan posture as every other agent. `true` (the default)
    /// preserves Chapter Picket's original always-on behavior.
    injection_scan_enabled: bool,
    /// Chapter Picket team-mission follow-up — same as
    /// `injection_scan_enabled`. Empty (the default) preserves Chapter
    /// Picket's original behavior byte-for-byte.
    injection_scan_exempt: std::collections::BTreeSet<String>,
```

- [ ] **Step 2: Initialize both in `SpecialistFactory::new`**

Find this exact block:

```rust
        SpecialistFactory {
            provider,
            model: model.into(),
            max_tokens,
            audit,
            base_tools,
            dialogue: None,
            member_backends: std::collections::HashMap::new(),
            checkpointer: None,
            kv_cache_handles: None,
        }
    }
```

Replace with:

```rust
        SpecialistFactory {
            provider,
            model: model.into(),
            max_tokens,
            audit,
            base_tools,
            dialogue: None,
            member_backends: std::collections::HashMap::new(),
            checkpointer: None,
            kv_cache_handles: None,
            injection_scan_enabled: true,
            injection_scan_exempt: std::collections::BTreeSet::new(),
        }
    }
```

- [ ] **Step 3: Add the two new builder methods**

Find this exact block (the end of `with_checkpointer`, right before
`with_kv_cache`):

```rust
    pub fn with_checkpointer(
        mut self,
        checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
    ) -> Self {
        self.checkpointer = checkpointer;
        self
    }

    /// Attach the shared kvcache pool/store to every specialist this
    /// factory builds. `None` means "no kvcache" (provider isn't
    /// llama-server, or the `/props` probe failed), preserving
    /// pre-kvcache behavior -- same shape as `with_checkpointer`.
    pub fn with_kv_cache(
```

Replace with:

```rust
    pub fn with_checkpointer(
        mut self,
        checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
    ) -> Self {
        self.checkpointer = checkpointer;
        self
    }

    /// Chapter Picket team-mission follow-up — global on/off for the
    /// active injection scan, applied to every specialist this factory
    /// builds. `true` (the default) preserves Chapter Picket's original
    /// behavior byte-for-byte.
    pub fn with_injection_scan_enabled(mut self, enabled: bool) -> Self {
        self.injection_scan_enabled = enabled;
        self
    }

    /// Chapter Picket team-mission follow-up — tool names exempted from
    /// the active scan for every specialist this factory builds. Empty
    /// (the default) preserves Chapter Picket's original behavior
    /// byte-for-byte.
    pub fn with_injection_scan_exempt(
        mut self,
        exempt: std::collections::BTreeSet<String>,
    ) -> Self {
        self.injection_scan_exempt = exempt;
        self
    }

    /// Attach the shared kvcache pool/store to every specialist this
    /// factory builds. `None` means "no kvcache" (provider isn't
    /// llama-server, or the `/props` probe failed), preserving
    /// pre-kvcache behavior -- same shape as `with_checkpointer`.
    pub fn with_kv_cache(
```

- [ ] **Step 4: Update `build()`'s `TurnSafety::autonomous()` call**

Find this exact block:

```rust
        // Team specialists run autonomously inside a mission — no human watches
        // each turn to `/cancel` a runaway — so they take the autonomous safety
        // posture: the small-cycle breaker as a built-in floor (always on, like
        // `MAX_STEPS_PER_TURN`), independent of the interactive `[agent]
        // cycle_detection` knob.
        Ok(TurnSafety::autonomous().apply(agent))
    }
```

Replace with:

```rust
        // Team specialists run autonomously inside a mission — no human watches
        // each turn to `/cancel` a runaway — so they take the autonomous safety
        // posture: the small-cycle breaker as a built-in floor (always on, like
        // `MAX_STEPS_PER_TURN`), independent of the interactive `[agent]
        // cycle_detection` knob. The injection-scan posture, unlike the cycle
        // breaker, is NOT forced -- it carries the operator's own `[agent]`
        // config through, same as every other agent (Chapter Picket's
        // tripwire exists specifically for this unattended case).
        Ok(TurnSafety::autonomous(
            self.injection_scan_enabled,
            self.injection_scan_exempt.clone(),
        )
        .apply(agent))
    }
```

- [ ] **Step 5: Write a new test**

Find `SpecialistFactory`'s existing test module (search for `mod tests`
in this file) and add a new test alongside whatever existing
`checkpointer`-related test is there — if none exists, add this test
directly inside the file's `mod tests` block, right after `use
super::*;`:

```rust
    #[test]
    fn specialist_factory_injection_scan_defaults_preserve_prior_behavior() {
        // Ground this test's exact SpecialistFactory::new(...) construction
        // against whatever fixture/helper this file's existing tests already
        // use before writing the assertion body -- match that pattern
        // exactly rather than inventing a new one.
    }
```

(This step's exact test body depends on grounding this file's existing
test fixtures first — the implementer should read `factory.rs`'s
`mod tests` block, find how an existing test already constructs a
`SpecialistFactory` and calls `.build(...)`, and write a real test
using that same construction that asserts the built agent's
`output_is_untrusted()`-gated behavior is unaffected by default
(`injection_scan_enabled: true`, `injection_scan_exempt: {}` on a
fresh factory matches pre-this-task behavior) — following the same
verification shape as Task 1's Step 2 test additions.)

- [ ] **Step 6: Run the crate's tests**

Run: `cargo test -p aivyx-team`
Expected: all tests pass, including the new one from Step 5.

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-team/src/factory.rs
git commit -m "feat(aivyx-team): thread the injection-scan posture through SpecialistFactory

Mirrors the existing checkpointer field/builder exactly. build()'s
TurnSafety::autonomous() call now carries the factory's own posture
instead of a hardcoded always-on -- team specialists will honor the
operator's real [agent] config once Task 5 threads real values in."
```

---

## Task 5: Add the two new parameters to `TeamAssembly::build()` (`aivyx-team`)

**Files:**
- Modify: `crates/aivyx-team/src/assembly.rs`

**Interfaces:**
- Consumes: `SpecialistFactory::with_injection_scan_enabled`/
  `with_injection_scan_exempt` (Task 4).
- Produces: `TeamAssembly::build(..., injection_scan_enabled: bool,
  injection_scan_exempt: BTreeSet<String>, ...)` — Task 6 and the
  daemon's `team_mission_driver.rs` (Task 7) both call this by the
  exact new parameter position specified below.

- [ ] **Step 1: Add the two new parameters and thread them into `SpecialistFactory`**

Find this exact block:

```rust
    pub fn build(
        config: TeamConfig,
        provider: Arc<dyn LlmProvider>,
        model: impl Into<String>,
        max_tokens: u32,
        audit: Arc<dyn AuditHook>,
        base_tools: Vec<Arc<dyn Tool>>,
        ceiling: CapabilitySet,
        member_backends: std::collections::HashMap<
            String,
            crate::factory::SpecialistBackend,
        >,
        checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
        kv_cache_handles: Option<(
            Arc<aivyx_llm::KvSlotPool>,
            Arc<aivyx_kvcache::LlamaServerSlotStore>,
            String,
        )>,
        message_origin: aivyx_core::MessageOrigin,
    ) -> Result<Self, TeamError> {
        config.validate()?;
        let dialogue = config.dialogue.clone();
        let bus = MessageBus::new(dialogue.message_bus_capacity);

        let factory = SpecialistFactory::new(provider, model, max_tokens, audit, base_tools)
            .with_dialogue(Arc::clone(&bus), dialogue.clone())
            .with_member_backends(member_backends)
            .with_checkpointer(checkpointer)
            .with_kv_cache(kv_cache_handles);
```

Replace with:

```rust
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        config: TeamConfig,
        provider: Arc<dyn LlmProvider>,
        model: impl Into<String>,
        max_tokens: u32,
        audit: Arc<dyn AuditHook>,
        base_tools: Vec<Arc<dyn Tool>>,
        ceiling: CapabilitySet,
        member_backends: std::collections::HashMap<
            String,
            crate::factory::SpecialistBackend,
        >,
        checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
        kv_cache_handles: Option<(
            Arc<aivyx_llm::KvSlotPool>,
            Arc<aivyx_kvcache::LlamaServerSlotStore>,
            String,
        )>,
        message_origin: aivyx_core::MessageOrigin,
        injection_scan_enabled: bool,
        injection_scan_exempt: std::collections::BTreeSet<String>,
    ) -> Result<Self, TeamError> {
        config.validate()?;
        let dialogue = config.dialogue.clone();
        let bus = MessageBus::new(dialogue.message_bus_capacity);

        let factory = SpecialistFactory::new(provider, model, max_tokens, audit, base_tools)
            .with_dialogue(Arc::clone(&bus), dialogue.clone())
            .with_member_backends(member_backends)
            .with_checkpointer(checkpointer)
            .with_kv_cache(kv_cache_handles)
            .with_injection_scan_enabled(injection_scan_enabled)
            .with_injection_scan_exempt(injection_scan_exempt);
```

(The function already has `#[allow(clippy::too_many_arguments)]` one
line above `pub fn build(` in the current file — confirm this exact
attribute is still present immediately above the signature after your
edit; if your edit's `Find` block above didn't already include it,
add it back exactly as shown.)

- [ ] **Step 2: Build**

Run: `cargo build -p aivyx-team`
Expected: a compile error at this task's own two call sites (`team.rs`
and `team_mission_driver.rs`) is expected until Tasks 6-7 land — those
are in a different crate (`aivyx-cli`/`aivyx-channel`), so
`cargo build -p aivyx-team` itself should be clean.

- [ ] **Step 3: Commit**

```bash
git add crates/aivyx-team/src/assembly.rs
git commit -m "feat(aivyx-team): add injection-scan parameters to TeamAssembly::build

Threaded into the SpecialistFactory builder chain, mirroring how
checkpointer/kv_cache_handles already flow through. Breaks both real
callers (team.rs, team_mission_driver.rs) until Tasks 6-7 update them
-- expected, tracked in the plan's dependency graph."
```

---

## Task 6: Add the two new parameters to `run_mission()` (`aivyx-cli`)

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx_modules/team.rs`
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs`

**Interfaces:**
- Consumes: `TurnSafety::autonomous(bool, BTreeSet<String>)` (Task 1);
  `TeamAssembly::build(..., bool, BTreeSet<String>)` (Task 5).
- Produces: nothing new (terminal for the CLI's own `aivyx team run`
  path).

- [ ] **Step 1: Add the two new parameters to `run_mission`'s signature**

Find this exact block:

```rust
pub async fn run_mission(
    provider: Arc<dyn LlmProvider>,
    model: &str,
    max_tokens: u32,
    audit: Arc<dyn AuditHook>,
    checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
    lead_scopes: &[String],
    kv_cache_handles: Option<(
        Arc<aivyx_llm::KvSlotPool>,
        Arc<aivyx_kvcache::LlamaServerSlotStore>,
        String,
    )>,
    base_tools: Vec<Arc<dyn Tool>>,
    mission: &str,
    config: Option<&str>,
) -> Result<(), String> {
```

Replace with:

```rust
#[allow(clippy::too_many_arguments)]
pub async fn run_mission(
    provider: Arc<dyn LlmProvider>,
    model: &str,
    max_tokens: u32,
    audit: Arc<dyn AuditHook>,
    checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
    lead_scopes: &[String],
    kv_cache_handles: Option<(
        Arc<aivyx_llm::KvSlotPool>,
        Arc<aivyx_kvcache::LlamaServerSlotStore>,
        String,
    )>,
    base_tools: Vec<Arc<dyn Tool>>,
    mission: &str,
    config: Option<&str>,
    injection_scan_enabled: bool,
    injection_scan_exempt: std::collections::BTreeSet<String>,
) -> Result<(), String> {
```

(Check whether `#[allow(clippy::too_many_arguments)]` already exists
immediately above `pub async fn run_mission(` in the current file — if
it does, leave it as-is and don't duplicate it; the `Find` block above
may not have included a pre-existing one directly above it.)

- [ ] **Step 2: Thread both values into `TeamAssembly::build(...)`**

Find this exact block:

```rust
        checkpointer.clone(),
        kv_cache_handles.clone(),
        // Interactively started via the CLI -- a real operator, not an
        // unattended trigger, so the recursive-scheduling guard doesn't
        // apply here.
        aivyx_core::MessageOrigin::Operator,
    )
    .map_err(|e| format!("failed to assemble team: {e}"))?;
```

Replace with:

```rust
        checkpointer.clone(),
        kv_cache_handles.clone(),
        // Interactively started via the CLI -- a real operator, not an
        // unattended trigger, so the recursive-scheduling guard doesn't
        // apply here.
        aivyx_core::MessageOrigin::Operator,
        injection_scan_enabled,
        injection_scan_exempt.clone(),
    )
    .map_err(|e| format!("failed to assemble team: {e}"))?;
```

- [ ] **Step 3: Apply both to the lead's own agent**

Find this exact block:

```rust
    .with_checkpointer(checkpointer);
    // The lead orchestrates the mission autonomously (delegating to specialists
    // via team.delegate), so it takes the same autonomous safety posture as the
    // specialists (see SpecialistFactory::build): the small-cycle breaker as a
    // built-in floor, independent of the interactive `[agent] cycle_detection`.
    let agent = TurnSafety::autonomous().apply(agent);
```

Replace with:

```rust
    .with_checkpointer(checkpointer);
    // The lead orchestrates the mission autonomously (delegating to specialists
    // via team.delegate), so it takes the same autonomous safety posture as the
    // specialists (see SpecialistFactory::build): the small-cycle breaker as a
    // built-in floor, independent of the interactive `[agent] cycle_detection`.
    // The injection-scan posture carries the operator's own `[agent]` config
    // through, same as every specialist this mission builds.
    let agent = TurnSafety::autonomous(injection_scan_enabled, injection_scan_exempt).apply(agent);
```

- [ ] **Step 4: Update the one real call site of `run_mission` in `aivyx.rs`**

Find this exact block:

```rust
        return team::run_mission(
            Arc::clone(&provider),
            &model,
            DEFAULT_MAX_TOKENS,
            Arc::clone(&audit),
            checkpointer.clone(),
            &cli_lead_scopes,
            // Task 6 — `aivyx team run` runs inside this SAME `run_async`
            // invocation, after the provider-selection block above already
            // built `kv_cache_handles` (the same one the daemon path below
            // reuses) — no second `/props` probe needed here.
            kv_cache_handles.clone(),
            tool_list,
            mission,
            config.as_deref(),
        )
```

Replace with:

```rust
        return team::run_mission(
            Arc::clone(&provider),
            &model,
            DEFAULT_MAX_TOKENS,
            Arc::clone(&audit),
            checkpointer.clone(),
            &cli_lead_scopes,
            // Task 6 — `aivyx team run` runs inside this SAME `run_async`
            // invocation, after the provider-selection block above already
            // built `kv_cache_handles` (the same one the daemon path below
            // reuses) — no second `/props` probe needed here.
            kv_cache_handles.clone(),
            tool_list,
            mission,
            config.as_deref(),
            injection_scan_enabled,
            injection_scan_exempt.clone(),
        )
```

- [ ] **Step 5: Build**

Run: `cargo build -p aivyx-cli --bin aivyx 2>&1 | grep -A5 "TurnSafety::\|TeamAssembly::build"`
Expected: any remaining error should only reference
`team_mission_driver.rs`'s own `TeamAssembly::build(...)` call (Task 7).

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-cli/src/bin/aivyx_modules/team.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat(aivyx-cli): thread the injection-scan posture through aivyx team run

run_mission's lead agent and every specialist it assembles now honor
the operator's real [agent] injection_scan_enabled/injection_scan_exempt
config, closing the CLI half of the team-mission coverage gap."
```

---

## Task 7: Add the two new fields to `TeamRunDeps` (`aivyx-channel`)

**Files:**
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs`
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs`

**Interfaces:**
- Consumes: `TeamAssembly::build(..., bool, BTreeSet<String>)` (Task 5).
- Produces: nothing new (terminal for the daemon's team-mission path).

- [ ] **Step 1: Add the two new fields to `TeamRunDeps`**

Find this exact block:

```rust
    /// `aivyx-checkpoint` — passed through to every specialist's
    /// `SpecialistFactory` so fs_root-mutating tool calls made during a team
    /// mission are checkpointed, same as every other agent construction path.
    pub checkpointer: Option<std::sync::Arc<aivyx_core::GitCheckpointer>>,
```

Replace with:

```rust
    /// `aivyx-checkpoint` — passed through to every specialist's
    /// `SpecialistFactory` so fs_root-mutating tool calls made during a team
    /// mission are checkpointed, same as every other agent construction path.
    pub checkpointer: Option<std::sync::Arc<aivyx_core::GitCheckpointer>>,
    /// Chapter Picket team-mission follow-up — passed through to
    /// `TeamAssembly::build` so a daemon-driven team mission's lead and
    /// every specialist honor the operator's real `[agent]`
    /// injection-scan posture, same as every other agent construction
    /// path. `true` (the default) preserves Chapter Picket's original
    /// always-on behavior.
    pub injection_scan_enabled: bool,
    /// Chapter Picket team-mission follow-up — same as
    /// `injection_scan_enabled`. Empty (the default) preserves Chapter
    /// Picket's original behavior byte-for-byte.
    pub injection_scan_exempt: std::collections::BTreeSet<String>,
```

- [ ] **Step 2: Thread both into `TeamAssembly::build(...)`**

Find this exact block:

```rust
    let assembly = TeamAssembly::build(
        config,
        Arc::clone(&deps.provider),
        deps.model.clone(),
        deps.max_tokens,
        audit,
        deps.base_tools.clone(),
        ceiling,
        member_backends,
        deps.checkpointer.clone(),
        deps.kv_cache_handles.clone(),
        message_origin,
    )?;
```

Replace with:

```rust
    let assembly = TeamAssembly::build(
        config,
        Arc::clone(&deps.provider),
        deps.model.clone(),
        deps.max_tokens,
        audit,
        deps.base_tools.clone(),
        ceiling,
        member_backends,
        deps.checkpointer.clone(),
        deps.kv_cache_handles.clone(),
        message_origin,
        deps.injection_scan_enabled,
        deps.injection_scan_exempt.clone(),
    )?;
```

- [ ] **Step 3: Update `TeamRunDeps`'s one real production construction site**

Find this exact block in `crates/aivyx-cli/src/bin/aivyx.rs`:

```rust
                schedule_store: Some(storage.domain(KeyDomain::Schedules)),
                checkpointer: checkpointer.clone(),
                // Task 6 — reuse the daemon's own shared kvcache pool/store
                // (Task 5), not a second probe: every specialist sub-turn a
                // team mission runs shares the exact same `KvSlotPool` the
                // daemon's main agent uses.
                kv_cache_handles: kv_cache_handles.clone(),
            };
```

Replace with:

```rust
                schedule_store: Some(storage.domain(KeyDomain::Schedules)),
                checkpointer: checkpointer.clone(),
                // Task 6 — reuse the daemon's own shared kvcache pool/store
                // (Task 5), not a second probe: every specialist sub-turn a
                // team mission runs shares the exact same `KvSlotPool` the
                // daemon's main agent uses.
                kv_cache_handles: kv_cache_handles.clone(),
                injection_scan_enabled,
                injection_scan_exempt: injection_scan_exempt.clone(),
            };
```

- [ ] **Step 4: Update `TeamRunDeps`'s test-only construction sites so the crate still compiles under `cargo test`**

Run: `grep -n "TeamRunDeps {" crates/aivyx-channel/src/team_mission_driver.rs`

This will show the `#[cfg(test)]`-only construction sites (inside
`mod tests`, after the `#[cfg(test)]` line found near the top of that
module — confirm each hit is genuinely inside the test module before
editing it, by checking it appears after the file's `#[cfg(test)]`
line). Add `injection_scan_enabled: true, injection_scan_exempt:
std::collections::BTreeSet::new(),` to each one, matching how
`checkpointer: None` already appears in each of those same test
fixtures.

- [ ] **Step 5: Build and run the full default-members test suite**

Run: `cargo build -p aivyx-cli --bin aivyx`
Expected: clean build — this is the last real call site in the whole
propagation chain.

Run: `cargo test`
Expected: all tests pass across the whole default-members workspace.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean, zero warnings.

- [ ] **Step 6: Confirm no direct `.with_injection_scan_enabled`/`.with_injection_scan_exempt` calls remain anywhere**

Run: `grep -rn "\.with_injection_scan_enabled(\|\.with_injection_scan_exempt(" --include="*.rs" . | grep -v target`

Expected: matches only inside `crates/aivyx-core/src/agent.rs` (where
`TurnSafety::apply` itself calls them — this is the one place they're
supposed to remain) and `crates/aivyx-team/src/assembly.rs` (where
`TeamAssembly::build` calls them on the `SpecialistFactory` it
constructs — also intended, per Task 5). No hits should remain in
`crates/aivyx-cli/src/bin/aivyx.rs` (Task 2 removed both).

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-channel/src/team_mission_driver.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat(aivyx-channel): thread the injection-scan posture through TeamRunDeps

Closes the daemon-side half of the team-mission coverage gap --
daemon-driven team missions (via aivyx_channel::team_mission_driver)
now honor the operator's real [agent] injection_scan_enabled/
injection_scan_exempt config, same as every other agent construction
path in the system. This closes Phase 199's logged follow-up: all 6
real TurnSafety::interactive/autonomous call sites in the codebase now
carry the same operator posture through one shared mechanism."
```

## Self-review notes (for whoever executes this plan)

- **Spec coverage:** Task 1 implements the spec's Section 1
  (`TurnSafety` itself) verbatim. Tasks 2-3 implement Section 2
  (converting the 4 real `interactive()` sites, including the
  previously-mislabeled `SessionConfig` one). Tasks 4-7 implement
  Sections 4-7 (the 2 real `autonomous()` sites and their full
  propagation chain back to config). Nothing in the spec is left
  unimplemented.
- **No placeholders:** every step's before/after text is copied
  directly from the real, current file content (verified via direct
  reads of all 7 touched files before writing this plan) — except
  Task 4 Step 5's test, which is deliberately left as a grounding
  instruction rather than fabricated code, since this session's own
  grounding pass did not find an existing `checkpointer`-equivalent
  test in `factory.rs` to copy the exact construction pattern from;
  the implementer must read that file's real `mod tests` block first
  and write real, compilable code matching its actual existing
  fixtures — this is the one intentional exception to "no
  placeholders," flagged explicitly rather than guessed at.
- **Type/interface consistency:** `injection_scan_enabled: bool` and
  `injection_scan_exempt: BTreeSet<String>` (never `Option`-wrapped)
  are used identically in every task's signatures, matching the
  spec's explicit constraint.

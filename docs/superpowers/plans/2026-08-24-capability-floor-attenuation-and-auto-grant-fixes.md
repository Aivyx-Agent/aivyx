# Capability-Floor Attenuation and Auto-Grant Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the 2 Critical findings from the vertical-pack-aware-capability-
floor branch's own final review: `bind_lead_scopes`' lead/specialist
unification breaks NT-02 attenuation one hop downstream (Task 1), and
`compute_backcompat_floor`'s generic sweep silently re-grants scopes the
codebase deliberately withholds (Task 2).

**Architecture:** Task 1 separates "the ceiling specialists get attenuated
against" (should be the operator's raw, un-narrowed floor) from "the lead's
own operational capabilities" (correctly narrowed already) — both real call
sites already hold the raw floor, they just need to pass it instead of
reusing the lead's own field. Task 2 adds a real per-tool opt-in on the
`Tool` trait (default `false`, fail-closed), overridden `true` on exactly
the 7 types that should flow into the floor today, replacing "is this tool
registered" with "did a maintainer explicitly mark this tool's scope safe
to auto-grant" as the governing question.

**Tech Stack:** Rust, existing `aivyx-team`/`aivyx-core`/`aivyx-tool`/
`aivyx-mcp`/`aivyx-channel`/`aivyx-cli` crates. No new dependencies.

## Global Constraints

- Task 1's fix must not change what the lead's own `ConcreteAgent` is built
  with (still `lead.declared_capabilities()`) — only what `SpecialistPool`'s
  ceiling is built from.
- Task 1's existing test `build_attenuates_capabilities_against_the_lead`
  (`crates/aivyx-team/src/factory.rs`) must be re-run and confirmed passing
  **unchanged** — it tests `attenuate_for_member`'s own logic directly,
  unaffected by which value callers pass.
- Task 2's `EXPLICIT_BASES` exclusion list and its own logic inside
  `compute_backcompat_floor` are **not** touched — it's a separate,
  additional safety layer (scope shape: a bare grant of a path-qualified
  family is dangerous regardless of a tool's own opinion), not replaced by
  the new opt-in mechanism.
- Task 2's `compute_backcompat_floor` function signature and its own two
  existing pinning tests (`compute_backcompat_floor_covers_every_conditional_grant`,
  `compute_backcompat_floor_omits_grants_when_conditions_are_false`) must
  **not** change — the filtering happens at the call site only.
- Every new test claiming to close either finding must be a genuine
  mutation-proof, shown to actually fail against the code as it exists at
  the start of this plan.
- `cargo build -p aivyx-cli -p aivyx-channel -p aivyx-team -p aivyx-core -p
  aivyx-tool -p aivyx-mcp` clean. Do **not** run `cargo build --workspace`/
  `cargo test --workspace` — the full workspace has an unrelated,
  pre-existing, out-of-scope build failure (`javascriptcoregtk-4.1` missing
  system library in a GUI crate).

---

### Task 1: Separate the NT-02 ceiling from the lead's own capabilities

**Files:**
- Modify: `crates/aivyx-team/src/assembly.rs` (`TeamAssembly::build`'s
  parameter/field, currently named `lead_caps`, at lines 39, 47-59, 80, 96;
  its doc comment at lines 45-50).
- Modify: `crates/aivyx-team/src/pool.rs` (`SpecialistPool`'s stored field,
  currently `lead_caps`, at lines 100, 104, 108, 258).
- Modify: `crates/aivyx-team/src/factory.rs` (`SpecialistFactory::build`'s
  parameter, currently `lead_caps: &CapabilitySet`, at lines 145-151 — pure
  rename for consistency, no logic change).
- Modify: `crates/aivyx-cli/src/bin/aivyx_modules/team.rs` (`run_mission`,
  around lines 226-260 — the local `lead_caps` variable stays; a new
  `ceiling` variable is added and passed to `TeamAssembly::build` in its
  place).
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs` (`assemble_runtime`,
  lines 1454-1511 — the local `lead`/`lead_caps` variables are removed
  entirely and replaced by the new `ceiling` variable, since neither has
  any other consumer in this function).
- Test: new test in `crates/aivyx-team/src/factory.rs`'s own test module.

Re-run `grep -n "lead_caps\|struct SpecialistPool\|struct TeamAssembly\|pub fn build\|pub fn new" crates/aivyx-team/src/assembly.rs crates/aivyx-team/src/pool.rs crates/aivyx-team/src/factory.rs` and `grep -n "fn run_mission\|lead_caps" crates/aivyx-cli/src/bin/aivyx_modules/team.rs` and `grep -n "fn assemble_runtime\|lead_caps\|deps.lead_scopes" crates/aivyx-channel/src/team_mission_driver.rs` before editing — confirm these still match the line numbers below; use whatever's actually current if they've drifted.

**Interfaces:**
- Produces: `TeamAssembly::build`'s renamed parameter —
  `pub fn build(config: TeamConfig, provider: ..., ceiling: CapabilitySet,
  ...) -> Result<Self, TeamError>` (same position in the argument list, same
  type, only the name changes — no caller needs to change argument *order*,
  only what value they compute for it).
- Produces: `SpecialistFactory::build`'s renamed parameter —
  `pub fn build(&self, member: &TeamMember, ceiling: &CapabilitySet) ->
  Result<ConcreteAgent, TeamError>`.
- Consumes: nothing from Task 2 (different crate cluster, no shared
  types beyond `Scope`/`CapabilitySet`, already imported).

- [ ] **Step 1: Write the failing mutation-proof test**

Add this test to `crates/aivyx-team/src/factory.rs`'s existing test module
(search for `fn build_attenuates_capabilities_against_the_lead` to find the
right neighborhood; add this test immediately after it):

```rust
    #[test]
    fn build_attenuates_against_the_full_ceiling_not_a_narrow_lead_declaration() {
        // Mirrors a real orchestration-only lead (default_nonagon's own
        // coordinator declares only [memory.read, memory.write,
        // team.delegate] -- "you never execute domain work directly").
        // The specialist declares a domain scope the LEAD itself never
        // asked for, but the operator's real ceiling grants it -- this
        // must still flow through. Before this fix, SpecialistFactory
        // was fed the lead's own narrow declaration as the ceiling,
        // collapsing this to nothing.
        let ceiling = CapabilitySet::from_scopes([
            Scope::parse("memory.read").unwrap(),
            Scope::parse("memory.write").unwrap(),
            Scope::parse("team.delegate").unwrap(),
            Scope::parse("fs.write:/root/**").unwrap(),
        ]);
        let factory = SpecialistFactory::new(
            FakeProvider::always("x"),
            "m",
            4096,
            Arc::new(NullAuditHook),
            vec![],
        );
        let m = member("writer", &["fs.write"], &["a"]);

        let agent = factory.build(&m, &ceiling).unwrap();

        assert!(
            agent.capabilities().grants(&Scope::parse("fs.write:/root/**").unwrap()),
            "a specialist's effective capabilities must be bounded by the \
             real operator ceiling, not a narrower value a caller might \
             mistakenly pass"
        );
    }
```

If `member(...)` or `FakeProvider::always(...)` or `agent.capabilities()`
don't match the exact helper names/shapes already in this test file, use
whatever the real existing helpers in this same test module are named —
search for `fn member(` and the existing `build_attenuates_capabilities_against_the_lead`
test's own body for the exact patterns to copy.

- [ ] **Step 2: Run the test to verify it fails for the right reason**

```bash
cd /home/julian/Projects/Rust/aivyx/.worktrees/cli-team-run-capability-floor
cargo test -p aivyx-team --lib build_attenuates_against_the_full_ceiling -- --test-threads=1
```

Expected at this point: the test should actually PASS already, since
`SpecialistFactory::build`'s own logic (`attenuate_for_member(lead_caps,
...)`) is correct in isolation — this task's bug is about *which value*
callers pass, not this function's own logic. This test alone does not
mutation-prove the bug; it's a regression guard for the function's own
contract. The REAL mutation-proof for this task lives in Step 6 below,
applied to the two real call sites. Do not be alarmed if this test passes
immediately — proceed to Step 3.

- [ ] **Step 3: Rename `SpecialistFactory::build`'s parameter**

In `crates/aivyx-team/src/factory.rs`, change:

```rust
    /// Build an attenuated specialist agent from `member`, with its
    /// capabilities capped at `lead_caps` (NT-02). Sync — no turn runs.
    pub fn build(
        &self,
        member: &TeamMember,
        lead_caps: &CapabilitySet,
    ) -> Result<ConcreteAgent, TeamError> {
        let caps = attenuate_for_member(lead_caps, &member.parsed_scopes()?);
```

to:

```rust
    /// Build an attenuated specialist agent from `member`, with its
    /// capabilities capped at `ceiling` (NT-02) — the operator's real,
    /// un-narrowed authority, not any particular member's own declared
    /// scopes. Sync — no turn runs.
    pub fn build(
        &self,
        member: &TeamMember,
        ceiling: &CapabilitySet,
    ) -> Result<ConcreteAgent, TeamError> {
        let caps = attenuate_for_member(ceiling, &member.parsed_scopes()?);
```

- [ ] **Step 4: Rename `TeamAssembly::build`'s parameter and `SpecialistPool`'s field**

In `crates/aivyx-team/src/assembly.rs`, change the `TeamAssembly` struct's
field:

```rust
pub struct TeamAssembly {
    config: TeamConfig,
    bus: Arc<MessageBus>,
    pool: Arc<SpecialistPool>,
    runtime: Arc<TeamRuntime>,
    lead_caps: CapabilitySet,
```

to:

```rust
pub struct TeamAssembly {
    config: TeamConfig,
    bus: Arc<MessageBus>,
    pool: Arc<SpecialistPool>,
    runtime: Arc<TeamRuntime>,
    ceiling: CapabilitySet,
```

Change `build`'s own doc comment and signature from:

```rust
    /// Validate `config` and wire the team against the daemon's shared deps.
    /// `lead_caps` is the authority the team runs under — every specialist is
    /// attenuated to a subset of it (NT-02), and the lead agent itself is
    /// mounted with it, so it must grant `team.delegate` + `team.message` for
    /// the orchestration/dialogue tools to be callable.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        config: TeamConfig,
        provider: Arc<dyn LlmProvider>,
        model: impl Into<String>,
        max_tokens: u32,
        audit: Arc<dyn AuditHook>,
        base_tools: Vec<Arc<dyn Tool>>,
        lead_caps: CapabilitySet,
```

to:

```rust
    /// Validate `config` and wire the team against the daemon's shared deps.
    /// `ceiling` is the operator's real, un-narrowed authority — every
    /// specialist is attenuated to a subset of it (NT-02). This is
    /// deliberately NOT the lead's own (possibly narrower)
    /// `capability_scopes` field: a purely-orchestration lead that
    /// declares no domain scopes for itself must still be able to grant
    /// its specialists whatever the operator's real floor allows, or
    /// every specialist collapses to near-nothing (the "missions report
    /// done but do nothing" failure `bind_lead_scopes` exists to
    /// prevent). The lead's own `ConcreteAgent`, built separately by
    /// this function's caller, uses the lead's own narrower field — it
    /// must still grant `team.delegate` + `team.message` for the
    /// orchestration/dialogue tools to be callable.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        config: TeamConfig,
        provider: Arc<dyn LlmProvider>,
        model: impl Into<String>,
        max_tokens: u32,
        audit: Arc<dyn AuditHook>,
        base_tools: Vec<Arc<dyn Tool>>,
        ceiling: CapabilitySet,
```

Inside the function body, change:

```rust
        let pool = Arc::new(SpecialistPool::new(factory, config.clone(), lead_caps.clone()));
```

to:

```rust
        let pool = Arc::new(SpecialistPool::new(factory, config.clone(), ceiling.clone()));
```

And change the struct literal at the end of `build`:

```rust
        Ok(TeamAssembly {
            config,
            bus,
            pool,
            runtime,
            lead_caps,
            lead_send,
        })
```

to:

```rust
        Ok(TeamAssembly {
            config,
            bus,
            pool,
            runtime,
            ceiling,
            lead_send,
        })
```

Change the accessor:

```rust
    pub fn lead_caps(&self) -> &CapabilitySet {
        &self.lead_caps
    }
```

to:

```rust
    pub fn ceiling(&self) -> &CapabilitySet {
        &self.ceiling
    }
```

(This accessor has zero callers today — confirmed by `grep -rn
"\.lead_caps()" crates/` returning nothing. Renaming it is for future
clarity, not because anything currently depends on it.)

In `crates/aivyx-team/src/pool.rs`, change the `SpecialistPool` struct:

```rust
pub struct SpecialistPool {
    factory: SpecialistFactory,
    config: TeamConfig,
    lead_caps: CapabilitySet,
}

impl SpecialistPool {
    pub fn new(factory: SpecialistFactory, config: TeamConfig, lead_caps: CapabilitySet) -> Self {
        SpecialistPool {
            factory,
            config,
            lead_caps,
        }
    }
```

to:

```rust
pub struct SpecialistPool {
    factory: SpecialistFactory,
    config: TeamConfig,
    ceiling: CapabilitySet,
}

impl SpecialistPool {
    pub fn new(factory: SpecialistFactory, config: TeamConfig, ceiling: CapabilitySet) -> Self {
        SpecialistPool {
            factory,
            config,
            ceiling,
        }
    }
```

And change the one call site inside `pool.rs` (currently around line 258):

```rust
        let agent = self.factory.build(member, &self.lead_caps)?;
```

to:

```rust
        let agent = self.factory.build(member, &self.ceiling)?;
```

- [ ] **Step 5: Fix the two real callers of `TeamAssembly::build`**

In `crates/aivyx-cli/src/bin/aivyx_modules/team.rs`, add `Scope` and
`CapabilitySet` to the existing `aivyx_capability` import (currently `use
aivyx_capability::TrustTier;`):

```rust
use aivyx_capability::{CapabilitySet, Scope, TrustTier};
```

Then, in `run_mission`, immediately after the existing:

```rust
    let lead_caps = lead.declared_capabilities().map_err(|e| e.to_string())?;
```

add:

```rust
    // The specialist ceiling is the operator's own real authority (the
    // raw floor this function was called with) -- not the lead's own
    // narrowed capability_scopes. Reusing the lead's own field here
    // would silently re-narrow every specialist down to whatever the
    // lead itself declared for its own direct use, defeating
    // bind_lead_scopes' own already-correct per-specialist floor
    // computation one hop downstream.
    let ceiling =
        CapabilitySet::from_scopes(lead_scopes.iter().filter_map(|s| Scope::parse(s)));
```

Then change the `TeamAssembly::build` call's argument from `lead_caps.clone(),`
to `ceiling,` (no `.clone()` needed — `ceiling` isn't used again after this
point, unlike `lead_caps` which is still needed for `ConcreteAgent::new`
below it, unchanged).

In `crates/aivyx-channel/src/team_mission_driver.rs`, in `assemble_runtime`,
change:

```rust
    bind_lead_scopes(&mut config, &deps.lead_scopes);
    let lead = config
        .lead_member()
        .ok_or_else(|| TeamError::Config("team has no lead".into()))?
        .clone();
    let lead_caps = lead.declared_capabilities()?;
```

to:

```rust
    bind_lead_scopes(&mut config, &deps.lead_scopes);
    // The specialist ceiling is the operator's own real authority
    // (deps.lead_scopes) -- not any member's own narrowed
    // capability_scopes. (TeamAssembly::build's own config.validate()
    // already errors on a missing lead, so no separate check is needed
    // here -- the old `lead`/`lead_caps` locals existed only to feed
    // this value, and had no other consumer in this function.)
    let ceiling = CapabilitySet::from_scopes(
        deps.lead_scopes.iter().filter_map(|s| Scope::parse(s)),
    );
```

Then change the `TeamAssembly::build` call's argument from `lead_caps,` to
`ceiling,`. Add `CapabilitySet` to the existing `aivyx_capability` import
(currently `use aivyx_capability::{Scope, TrustTier};`):

```rust
use aivyx_capability::{CapabilitySet, Scope, TrustTier};
```

- [ ] **Step 6: Perform the mutation-proof — confirm the real bug is closed**

This test demonstrates the bug directly, self-contained within
`aivyx-team` (no cross-crate call to the real `bind_lead_scopes` needed —
`lead_narrowed` below is simply what that function's already-shipped
specialist-branch logic would produce for this coordinator, computed by
hand from the same rule: floor ∩ the lead's own declared bases + markers).
Add this second test to the same `factory.rs` test module, after Step 1's
test:

```rust
    #[test]
    fn effective_specialist_caps_come_from_the_real_floor_not_the_orchestration_only_leads_own_narrowed_declaration() {
        // What the operator's real, un-narrowed floor grants.
        let raw_floor = CapabilitySet::from_scopes([
            Scope::parse("memory.read").unwrap(),
            Scope::parse("memory.write").unwrap(),
            Scope::parse("team.delegate").unwrap(),
            Scope::parse("fs.write:/root/**").unwrap(),
        ]);
        // What bind_lead_scopes' own (already-shipped, correct)
        // specialist-branch logic produces for an orchestration-only
        // lead shaped like default_nonagon's coordinator -- which
        // declares only [memory.read, memory.write, team.delegate] for
        // itself, so the lead's OWN capability_scopes field ends up
        // narrower than the raw floor.
        let lead_narrowed = CapabilitySet::from_scopes([
            Scope::parse("memory.read").unwrap(),
            Scope::parse("memory.write").unwrap(),
            Scope::parse("team.delegate").unwrap(),
        ]);
        let factory = SpecialistFactory::new(
            FakeProvider::always("x"),
            "m",
            4096,
            Arc::new(NullAuditHook),
            vec![],
        );
        let writer = member("writer", &["fs.write"], &["a"]);

        // Correct: the ceiling is the raw floor -- the specialist's
        // declared fs.write base is covered.
        let agent_correct = factory.build(&writer, &raw_floor).unwrap();
        assert!(
            agent_correct.capabilities().grants(&Scope::parse("fs.write:/root/**").unwrap()),
            "against the real floor, the specialist must retain fs.write"
        );

        // The bug this task fixes: if a caller mistakenly passes the
        // lead's own narrowed field as the ceiling instead, the
        // specialist's effective capabilities collapse -- reproducing
        // the exact regression the final review found.
        let agent_buggy = factory.build(&writer, &lead_narrowed).unwrap();
        assert!(
            !agent_buggy.capabilities().grants(&Scope::parse("fs.write:/root/**").unwrap()),
            "this assertion documents the bug's own shape: an \
             orchestration-only lead's own narrowed field does NOT cover \
             fs.write, so a caller passing it as the ceiling (the pre-fix \
             behavior at both real call sites) collapses the specialist"
        );
    }
```

The second assertion is deliberately inverted (asserts the OLD, buggy
behavior would fail) — it's not testing this task's own fix directly, it's
documenting the exact failure mode Step 8's real-call-site changes
prevent, so a future reader can see both sides in one place. The real
mutation-proof for Task 1's actual fix is structural: Step 5's changes are
the only thing standing between `run_mission`/`assemble_runtime` and
reproducing the `agent_buggy` case above. Confirm this directly: after
completing Step 5, temporarily revert just the one line in `run_mission`
that passes `ceiling,` back to `lead_caps.clone(),`, run `cargo build -p
aivyx-cli` to confirm it still compiles (proving the bug is a silent logic
error, not a type error — nothing catches it at compile time), then revert
back to the real fix. Record this in your report; no automated test can
exercise the real call sites without a live LLM provider, so this manual
build-time check plus the two unit tests above are the mutation-proof for
this task.

- [ ] **Step 7: Run the full test suites and a clean build**

```bash
cd /home/julian/Projects/Rust/aivyx/.worktrees/cli-team-run-capability-floor
cargo test -p aivyx-team --lib -- --test-threads=1
cargo test -p aivyx-cli --bin aivyx -- --test-threads=1
cargo test -p aivyx-channel --lib -- --test-threads=1
cargo build -p aivyx-team -p aivyx-cli -p aivyx-channel
cargo clippy -p aivyx-team --lib -- -D warnings
cargo clippy -p aivyx-cli --bin aivyx -- -D warnings
cargo clippy -p aivyx-channel --lib -- -D warnings
```

Expected: `aivyx-team` passes (baseline + 2 new tests), `aivyx-cli` 566/566
unchanged, `aivyx-channel` 1305/1305 unchanged (this task doesn't add any
tests to `aivyx-channel` itself), clean builds, clippy clean except the
pre-existing, unrelated `trigger.rs:223` finding.

- [ ] **Step 8: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx/.worktrees/cli-team-run-capability-floor
git add crates/aivyx-team/src/assembly.rs crates/aivyx-team/src/pool.rs \
        crates/aivyx-team/src/factory.rs \
        crates/aivyx-cli/src/bin/aivyx_modules/team.rs \
        crates/aivyx-channel/src/team_mission_driver.rs
git commit -m "Separate the NT-02 attenuation ceiling from the lead's own capabilities

TeamAssembly::build's lead_caps parameter had exactly one real
consumer (SpecialistPool's attenuation ceiling) but was fed the
lead's OWN post-bind_lead_scopes capability_scopes field at both real
call sites -- harmless when the lead unconditionally held the full
floor, but a silent double-narrowing once bind_lead_scopes' lead
branch was correctly narrowed to the lead's own declared bases:
every specialist's effective runtime capabilities collapsed to
whatever the lead itself declared, reproducing the exact
'missions report done but do nothing' bug bind_lead_scopes exists to
prevent. Renamed to `ceiling` throughout and fed the raw operator
floor both call sites already hold, instead of the narrowed field.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: A real per-tool opt-in for the backcompat floor's generic sweep

**Files:**
- Modify: `crates/aivyx-core/src/lib.rs` (`Tool` trait, add the new default
  method near `mutates_fs_root` at line ~947).
- Modify: `crates/aivyx-tool/src/proxy.rs` (`ToolProxy`, override at its
  `impl Tool for ToolProxy` block, near `output_is_untrusted` at line ~119).
- Modify: `crates/aivyx-mcp/src/proxy.rs`, `resource_proxy.rs`,
  `prompt_proxy.rs` (`McpToolProxy`, `McpResourceProxy`, `McpPromptProxy` —
  one override each).
- Modify: `crates/aivyx-channel/src/ollama_tools.rs` (`OllamaListTool`,
  `OllamaShowTool`, `OllamaPullTool` — one override each).
- Modify: `crates/aivyx-core/src/tools/git.rs`,
  `crates/aivyx-channel/src/role_update_tool.rs`,
  `crates/aivyx-channel/src/reflection_tool.rs`,
  `crates/aivyx-channel/src/skill_tool.rs` — no code change, just a short
  restored-rationale comment each (the default `false` already applies;
  these files are touched only for documentation).
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` — extract the call site's
  inline `tool_scope_bases` derivation into a small, directly-testable
  function, add the new filter.
- Test: new tests in `crates/aivyx-cli/src/bin/aivyx.rs`'s existing `mod
  tests` block, plus a doc-comment update to the existing integration test
  `compute_backcompat_floor_flows_a_configured_verticals_domain_scopes_through_bind_lead_scopes`.

Re-run `grep -n "trait Tool\|fn mutates_fs_root" crates/aivyx-core/src/lib.rs`,
`grep -n "impl Tool for ToolProxy\|fn output_is_untrusted" crates/aivyx-tool/src/proxy.rs`,
`grep -n "impl Tool for" crates/aivyx-mcp/src/proxy.rs crates/aivyx-mcp/src/resource_proxy.rs crates/aivyx-mcp/src/prompt_proxy.rs`,
`grep -n "impl Tool for" crates/aivyx-channel/src/ollama_tools.rs`, and
`grep -n "let tool_scope_bases: Vec<Scope> =" crates/aivyx-cli/src/bin/aivyx.rs`
before editing — confirm these still match the line numbers/shapes below.

**Interfaces:**
- Produces: `Tool::auto_grantable_in_backcompat_floor(&self) -> bool`
  (default `false`) on the trait in `aivyx-core`, usable from every crate
  that already implements `Tool`.
- Produces: `fn tool_scope_bases_for_floor(tool_list: &[Arc<dyn Tool>]) ->
  Vec<Scope>` in `crates/aivyx-cli/src/bin/aivyx.rs` — a small private
  helper replacing the inline derivation at the `compute_backcompat_floor`
  call site, directly unit-testable with fake `Tool` implementations.
- Consumes: nothing from Task 1 (different crate cluster).

- [ ] **Step 1: Write the failing tests for the trait mechanism**

Add this to `crates/aivyx-cli/src/bin/aivyx.rs`'s existing `mod tests`
block (search for `fn compute_backcompat_floor_omits_grants_when_conditions_are_false`
and add these tests immediately after it, before the role-envelope tests):

```rust
    /// A minimal fake `Tool` for testing `tool_scope_bases_for_floor`'s
    /// own filtering, without needing a real spawned tool process or MCP
    /// connection.
    struct FakeFloorTool {
        scope: &'static str,
        auto_grantable: bool,
    }

    #[async_trait::async_trait]
    impl aivyx_core::Tool for FakeFloorTool {
        fn id(&self) -> aivyx_core::ToolId {
            aivyx_core::ToolId::new()
        }
        fn name(&self) -> &str {
            "fake"
        }
        fn description(&self) -> &str {
            "fake"
        }
        fn input_schema(&self) -> &serde_json::Value {
            static SCHEMA: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
            SCHEMA.get_or_init(|| serde_json::json!({}))
        }
        fn required_scope(&self, _input: &serde_json::Value) -> Scope {
            Scope::parse(self.scope).unwrap()
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _context: &aivyx_core::ToolContext<'_>,
        ) -> aivyx_core::ToolOutcome {
            unreachable!("not exercised by this test")
        }
        fn auto_grantable_in_backcompat_floor(&self) -> bool {
            self.auto_grantable
        }
    }

    #[test]
    fn tool_scope_bases_for_floor_excludes_tools_that_do_not_opt_in() {
        let tool_list: Vec<Arc<dyn Tool>> = vec![
            Arc::new(FakeFloorTool { scope: "kitchen.read", auto_grantable: true }),
            Arc::new(FakeFloorTool { scope: "git.write", auto_grantable: false }),
        ];
        let bases = tool_scope_bases_for_floor(&tool_list);
        let strings: Vec<&str> = bases.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            strings,
            vec!["kitchen.read"],
            "a tool that does not opt in must never contribute to the floor, \
             even though its scope base isn't in EXPLICIT_BASES either"
        );
    }

    #[test]
    fn tool_scope_bases_for_floor_includes_every_opted_in_tool() {
        let tool_list: Vec<Arc<dyn Tool>> = vec![
            Arc::new(FakeFloorTool { scope: "kitchen.read", auto_grantable: true }),
            Arc::new(FakeFloorTool { scope: "ollama.list", auto_grantable: true }),
        ];
        let bases = tool_scope_bases_for_floor(&tool_list);
        let strings: Vec<&str> = bases.iter().map(|s| s.as_str()).collect();
        assert_eq!(strings, vec!["kitchen.read", "ollama.list"]);
    }
```

`Arc`/`Scope`/`Tool` should already be in scope in this test module via
`use super::*;` (confirm — if not, add the specific imports needed).

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd /home/julian/Projects/Rust/aivyx/.worktrees/cli-team-run-capability-floor
cargo test -p aivyx-cli --bin aivyx tool_scope_bases_for_floor -- --test-threads=1
```

Expected: FAIL — compile error, `tool_scope_bases_for_floor` and
`Tool::auto_grantable_in_backcompat_floor` don't exist yet.

- [ ] **Step 3: Add the trait method**

In `crates/aivyx-core/src/lib.rs`, immediately after the existing
`mutates_fs_root` default method (search for it — it ends with `fn
mutates_fs_root(&self) -> bool { false }` followed by the closing `}` of
the trait), add:

```rust

    /// Whether this tool's required scope may be auto-granted to the
    /// default, floor-only role via the operator's backcompat floor.
    /// Default `false`: a tool must explicitly opt in. Most tools should
    /// NOT override this — third-party/OAuth integrations (Gmail, Drive,
    /// Notion, Obsidian, N8N, Contacts, Calendar, ...) and every
    /// domain-specific toolkit tool stay withheld unless a maintainer has
    /// explicitly reviewed the base and opted it in here.
    ///
    /// Withheld deliberately, with no override anywhere in this codebase
    /// as of this writing: `git.write` (commit rights are role-config-
    /// driven, never auto-granted — an operator declares `git.write:<repo>`
    /// in a custom role), `git.read` (same: an operator declares
    /// `git.read:**` or a per-repo grant explicitly), `role.update`
    /// (self-escalation surface — P8 no-self-escalation), `reflection.apply`
    /// (the propose half is safe to auto-grant since it only ever lands as
    /// Pending behind operator approval; apply is self-modification and
    /// stays role-declared), `skills.write` (the identity-modifying
    /// persona-chain writer — the write half stays auto-proposer /
    /// role-declared, unlike the read half).
    fn auto_grantable_in_backcompat_floor(&self) -> bool {
        false
    }
```

- [ ] **Step 4: Override on the 7 types that should opt in**

In `crates/aivyx-tool/src/proxy.rs`, inside `impl Tool for ToolProxy`,
immediately after the existing `output_is_untrusted` override, add:

```rust

    // Every tool-process-sourced tool (kitchen, `applications`, any
    // future vertical toolkit) is here because the operator explicitly
    // configured a `[[tool_process]]` entry -- that configuration act
    // IS the opt-in. `scope_overrides` (the daemon's own
    // narrower-than-declared override mechanism) is already folded into
    // `required_scope()` by construction, so the floor grant reflects
    // whatever the operator actually authorized, not the toolkit's own
    // raw declaration.
    fn auto_grantable_in_backcompat_floor(&self) -> bool {
        true
    }
```

In `crates/aivyx-mcp/src/proxy.rs`, inside `impl Tool for McpToolProxy`,
add the same override (adjust the comment's first sentence to say "every
MCP-server-sourced tool" and "the operator explicitly configured a
`[[mcp_server]]` entry"):

```rust

    // Every MCP-server-sourced tool is here because the operator
    // explicitly configured a `[[mcp_server]]` entry -- that
    // configuration act IS the opt-in.
    fn auto_grantable_in_backcompat_floor(&self) -> bool {
        true
    }
```

Add the identical override (same comment) to `impl Tool for
McpResourceProxy` in `crates/aivyx-mcp/src/resource_proxy.rs` and `impl
Tool for McpPromptProxy` in `crates/aivyx-mcp/src/prompt_proxy.rs`.

In `crates/aivyx-channel/src/ollama_tools.rs`, inside each of `impl Tool
for OllamaListTool`, `impl Tool for OllamaShowTool`, `impl Tool for
OllamaPullTool`, add:

```rust

    // Registered only when the operator's own [provider] kind = "ollama"
    // -- that configuration choice is the opt-in.
    fn auto_grantable_in_backcompat_floor(&self) -> bool {
        true
    }
```

- [ ] **Step 5: Restore rationale comments at the withheld tools' own sites**

In `crates/aivyx-core/src/tools/git.rs`, immediately above `GitCommitTool`'s
`fn required_scope` (search for `impl Tool for GitCommitTool`, then its
`required_scope` method), add:

```rust
    // git.write is never auto-granted via the backcompat floor's generic
    // sweep (Tool::auto_grantable_in_backcompat_floor's default `false`,
    // not overridden here) -- an operator who wants the agent to commit
    // declares `git.write:<repo>` in a role's own `capability_scopes`.
```

Immediately above `GitStatusTool`'s `fn required_scope` (`impl Tool for
GitStatusTool`), add:

```rust
    // git.read is never auto-granted via the backcompat floor's generic
    // sweep either, for the same reason as git.write -- an operator
    // declares `git.read:**` or a per-repo grant explicitly.
```

In `crates/aivyx-channel/src/role_update_tool.rs`, immediately above
`RoleUpdateTool`'s `fn required_scope`, add:

```rust
    // role.update is never auto-granted -- self-escalation scopes stay
    // out of the default floor (P8 no-self-escalation).
```

In `crates/aivyx-channel/src/reflection_tool.rs`, immediately above
`ReflectionApplyTool`'s `fn required_scope`, add:

```rust
    // reflection.apply is never auto-granted, unlike reflection.propose
    // (which IS in the floor's fixed baseline) -- a proposal only ever
    // lands as Pending behind operator approval, but apply is
    // self-modification and stays role-declared.
```

In `crates/aivyx-channel/src/skill_tool.rs`, immediately above
`SkillTeachTool`'s `fn required_scope` (the first of the three
`skills.write`-declaring types), add:

```rust
    // skills.write is never auto-granted -- the write half of the
    // skills surface (SkillTeachTool/SkillUpdateTool/SkillForgetTool, all
    // three) stays auto-proposer / role-declared, unlike skills.list/
    // skills.invoke (both in the floor's fixed baseline).
```

- [ ] **Step 6: Extract and filter the call site**

In `crates/aivyx-cli/src/bin/aivyx.rs`, add this new function directly
above `compute_backcompat_floor` (search for `fn compute_backcompat_floor(`
to find the insertion point):

```rust
/// The tool-derived slice of `compute_backcompat_floor`'s input: every
/// registered tool's own `required_scope()`, filtered down to only the
/// tools that have explicitly opted in via
/// `Tool::auto_grantable_in_backcompat_floor()`. Extracted as its own
/// function so the filter itself is directly unit-testable without
/// needing a real, fully-wired `tool_list`.
fn tool_scope_bases_for_floor(tool_list: &[Arc<dyn Tool>]) -> Vec<Scope> {
    tool_list
        .iter()
        .filter(|t| t.auto_grantable_in_backcompat_floor())
        .map(|t| t.required_scope(&serde_json::json!({})))
        .collect()
}

```

Then change the call site (search for `let tool_scope_bases: Vec<Scope> =`)
from:

```rust
    let tool_scope_bases: Vec<Scope> =
        tool_list.iter().map(|t| t.required_scope(&serde_json::json!({}))).collect();
```

to:

```rust
    let tool_scope_bases: Vec<Scope> = tool_scope_bases_for_floor(&tool_list);
```

- [ ] **Step 7: Update the existing integration test's doc comment**

In the same file, find
`fn compute_backcompat_floor_flows_a_configured_verticals_domain_scopes_through_bind_lead_scopes`
and its doc comment (search for it). Replace the doc comment's text with:

```rust
    /// Proof that compute_backcompat_floor's own generic sweep, GIVEN a
    /// set of tool-derived scope bases, correctly flows a vertical
    /// toolkit's domain scopes through the real bind_lead_scopes so a
    /// pack's specialists keep them instead of collapsing to
    /// team.message only. This test calls compute_backcompat_floor
    /// directly with hand-picked bases, so it does NOT exercise the
    /// call-site's own auto_grantable_in_backcompat_floor filter
    /// (tool_scope_bases_for_floor_excludes_tools_that_do_not_opt_in
    /// covers that, separately, with a fake Tool). Together the two
    /// tests cover the full real pipeline: filter -> sweep -> bind.
```

Do not change the test's body — it remains valid and useful as written,
just under a corrected description of its own scope.

- [ ] **Step 8: Run the tests to verify they pass**

```bash
cargo test -p aivyx-cli --bin aivyx tool_scope_bases_for_floor -- --test-threads=1
```

Expected: PASS (2/2).

- [ ] **Step 9: Run the full test suites and builds**

```bash
cd /home/julian/Projects/Rust/aivyx/.worktrees/cli-team-run-capability-floor
cargo test -p aivyx-cli --bin aivyx -- --test-threads=1
cargo test -p aivyx-channel --lib -- --test-threads=1
cargo build -p aivyx-cli -p aivyx-channel -p aivyx-core -p aivyx-tool -p aivyx-mcp
cargo clippy -p aivyx-cli --bin aivyx -- -D warnings
cargo clippy -p aivyx-channel --lib -- -D warnings
cargo clippy -p aivyx-core --lib -- -D warnings
cargo clippy -p aivyx-tool --lib -- -D warnings
cargo clippy -p aivyx-mcp --lib -- -D warnings
```

Expected: `aivyx-cli` passes at 566 + 2 new = 568 (the two new
`tool_scope_bases_for_floor_*` tests; `compute_backcompat_floor_covers_every_conditional_grant`/
`compute_backcompat_floor_omits_grants_when_conditions_are_false` unchanged
per the global constraint); `aivyx-channel` 1305 unchanged; clean builds;
clippy clean except the pre-existing, unrelated `trigger.rs:223` finding.
Also run, since this task touches many crates with many `impl Tool`
sites elsewhere in the dependency graph that could theoretically break if
the trait's default method interacts badly with an existing override
(unlikely, since it's a new method with a default, but confirm):

```bash
cargo build -p aivyx-team -p aivyx-memory -p aivyx-toolkit -p aivyx-dataread
```

Expected: clean — these crates have `impl Tool` sites too (per the
earlier `grep -rln "impl Tool for"` survey) and must still compile
unmodified against the trait's new default method.

- [ ] **Step 10: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx/.worktrees/cli-team-run-capability-floor
git add crates/aivyx-core/src/lib.rs \
        crates/aivyx-core/src/tools/git.rs \
        crates/aivyx-tool/src/proxy.rs \
        crates/aivyx-mcp/src/proxy.rs crates/aivyx-mcp/src/resource_proxy.rs crates/aivyx-mcp/src/prompt_proxy.rs \
        crates/aivyx-channel/src/ollama_tools.rs \
        crates/aivyx-channel/src/role_update_tool.rs \
        crates/aivyx-channel/src/reflection_tool.rs \
        crates/aivyx-channel/src/skill_tool.rs \
        crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "Add a real per-tool opt-in for the backcompat floor's generic sweep

compute_backcompat_floor's generic sweep granted the bare scope base
of any registered tool except a small fs/shell/net/workspace
exclusion list -- silently re-granting git.write, git.read,
role.update, reflection.apply, and skills.write, all five previously
deliberately withheld with their own documented rationale (four of
which had that rationale deleted by the same change that reversed
them). Adds Tool::auto_grantable_in_backcompat_floor (default false,
fail-closed), overridden true on exactly the 7 types that should
flow into the floor today (the tool-process proxy, the 3 MCP proxy
types, and the 3 Ollama tools) -- so a future dangerous tool needs an
explicit maintainer opt-in rather than silently slipping through.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

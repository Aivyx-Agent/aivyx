# CLI `team run --config` Capability Floor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the CLI's `aivyx team run --config <pack.toml>` capability-floor
gap — a vertical pack's lead role today gets exactly and unconditionally
whatever `capability_scopes` it declares, with no clamp against the
operator's own real authority.

**Architecture:** Extract `aivyx.rs`'s existing `backcompat_floor`
construction into a standalone `compute_backcompat_floor` function and call
it earlier in `run_async` — before the `aivyx team run` early-return branch
— so the operator's real authority is available at that point. Make
`bind_lead_scopes` (already shipped, already tested, currently private to
`aivyx-channel`) `pub`, and call it from a new `load_and_clamp_team` helper
in `team.rs` that `run_mission` uses in place of its current unclamped
`load_team` call.

**Tech Stack:** Rust, existing `aivyx-cli`/`aivyx-channel`/`aivyx-team-types`
crates. No new dependencies.

## Global Constraints

- `compute_backcompat_floor`'s extracted output must be provably identical
  to the current inline block's behavior for the interactive path — this is
  this plan's primary regression risk, not a formality.
- The existing `empty_role_inherits_backcompat_floor_verbatim` test and its
  neighbors (`crates/aivyx-cli/src/bin/aivyx.rs`, in the `mod tests` block
  starting at line 10237) must be re-run and confirmed passing **unchanged**
  as part of this plan's own verification — not modified to make this
  plan's changes pass.
- No new CLI flags, no confirm-first prompts, no changes to
  `TeamConfig::load`'s parsing/validation, no changes to the Studio's or
  scheduler's own call sites already fixed by the bind_lead_scopes
  Floor-Clamp piece — all explicitly out of scope per the design doc
  (`docs/superpowers/specs/2026-08-24-cli-team-run-capability-floor-design.md`).
- Every new test this plan adds must be a genuine mutation-proof — shown to
  actually fail if the code it protects is reverted or bypassed — not just
  an assertion that happens to pass. This applies with extra weight to
  Task 2's CLI-clamp test, since that test closes the actual security gap
  this whole plan exists to fix.

---

### Task 1: Extract `compute_backcompat_floor` and call it before the CLI team-run branch

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs`
  - Delete the inline construction at lines 7995–8180 (the `---- Capabilities
    ----` comment block through the `if loop_state.is_some() { ... }` closing
    brace at line 8180 — **not** the "Walk the active role's inheritance
    chain..." comment at lines 8182–8186, which documents the *next*
    statement, `assemble_role_envelope`, and must be preserved), replacing
    the deleted range with a call to the new function.
  - Add the new `fn compute_backcompat_floor(...)` as a top-level function
    (place it directly above `async fn run_async(` — search for
    `async fn run_async(` to find the exact current line; Rust doesn't care
    about item order, this is just for readability).
  - Add a new call to `compute_backcompat_floor(...)` immediately before the
    `// Chapter J — \`aivyx team run "<mission>"\`.` comment (currently line
    7860, immediately before `if let CliMode::Team(TeamSubcommand::Run { .. })
    = &mode {` at line 7866 — re-find these exact lines fresh, since Task 1's
    own earlier edits in this same task do not move them, but confirm before
    editing).
  - Add a new test in the existing `mod tests` block (starts at line 10237,
    `use super::*;` already brings `Scope` and `compute_backcompat_floor`
    into scope) — place it near the `assemble_role_envelope` tests (search
    for `fn empty_role_inherits_backcompat_floor_verbatim`).

**Interfaces:**
- Produces: `fn compute_backcompat_floor(fs_read_scope: Scope, fs_write_scope:
  Scope, fs_metadata_scope: Scope, canonical_root: &std::path::Path,
  shell_exec_scope: Option<Scope>, fs_delete_scope: Option<Scope>,
  workspace_scopes: Vec<Scope>, ollama_configured: bool, mcp_server_names:
  &[String], config_tool_processes: &[aivyx_config::ToolProcessConfig],
  loop_armed: bool) -> Vec<Scope>` — Task 2 does not call this directly (it
  consumes `backcompat_floor`, the `Vec<Scope>` this produces, via the local
  variable in `run_async`), but must know its existence and location.
- Produces: the local variable `backcompat_floor: Vec<Scope>` in `run_async`
  is now bound earlier (before line 7866) than before. Task 2's edit to the
  `aivyx team run` call site depends on `backcompat_floor` already being in
  scope at that point.

- [ ] **Step 1: Re-verify the exact current source ranges before editing**

Run, from the repo root:

```bash
grep -n "Chapter J —\|CliMode::Team(TeamSubcommand::Run\|let mut backcompat_floor: Vec<Scope> = vec!\[\|let role_envelope = assemble_role_envelope\|^async fn run_async" crates/aivyx-cli/src/bin/aivyx.rs
```

Expected (as of this plan's writing — confirm the line numbers match before
proceeding; if they've drifted, use the real current numbers for every step
below instead):

```
5257:async fn run_async(
7860:    // Chapter J — `aivyx team run "<mission>"`. We now hold the live provider,
7866:    if let CliMode::Team(TeamSubcommand::Run { mission, config }) = &mode {
8020:    let mut backcompat_floor: Vec<Scope> = vec![
8187:    let role_envelope = assemble_role_envelope(&role_for_envelope, &roles, &backcompat_floor);
```

- [ ] **Step 2: Write the failing test for `compute_backcompat_floor`**

Insert this test into the `mod tests` block in
`crates/aivyx-cli/src/bin/aivyx.rs`, right after the closing brace of
`fn empty_role_inherits_backcompat_floor_verbatim` (search for that
function name to find the insertion point):

```rust
    /// Pins `compute_backcompat_floor`'s exact output — every conditional
    /// grant fires (Ollama configured, one MCP bridge, `applications`
    /// tool process, loop armed) so this doubles as the regression proof
    /// that extracting the block out of `run_async` (previously inline,
    /// with no direct unit coverage — only indirect e2e) didn't silently
    /// change interactive-path behavior.
    #[test]
    fn compute_backcompat_floor_covers_every_conditional_grant() {
        let canonical_root = PathBuf::from("/tmp/proj");
        let floor = compute_backcompat_floor(
            Scope::parse("fs.read:/tmp/proj/**").unwrap(),
            Scope::parse("fs.write:/tmp/proj/**").unwrap(),
            Scope::parse("fs.metadata:/tmp/proj/**").unwrap(),
            &canonical_root,
            Some(Scope::parse("shell.exec:cwd:/tmp/proj/**").unwrap()),
            Some(Scope::parse("fs.delete:/tmp/proj/**").unwrap()),
            vec![
                Scope::parse("workspace:/tmp/proj/ws/**").unwrap(),
                Scope::parse("workspace:/tmp/proj/ws").unwrap(),
            ],
            true, // ollama_configured
            &["websearch".to_string()],
            &[aivyx_config::ToolProcessConfig {
                name: "applications".to_string(),
                command: "aivyx-apps".to_string(),
                args: vec![],
                env: vec![],
                scope_overrides: std::collections::HashMap::new(),
                enabled: true,
                sandbox: None,
                disable_sandbox: false,
            }],
            true, // loop_armed
        );

        let scope_strings: Vec<&str> = floor.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            scope_strings,
            vec![
                "memory.read",
                "memory.write",
                "memory.forget",
                "memory.gc",
                "graph.read",
                "skills.list",
                "skills.invoke",
                "schedule.list",
                "schedule.create",
                "schedule.update",
                "schedule.delete",
                "reflection.propose",
                "persona.propose",
                "fs.read:/tmp/proj/**",
                "fs.write:/tmp/proj/**",
                "fs.metadata:/tmp/proj/**",
                "net.fetch",
                "net.post",
                "fs.read:/tmp/proj",
                "fs.write:/tmp/proj",
                "fs.metadata:/tmp/proj",
                "shell.exec:cwd:/tmp/proj/**",
                "shell.exec:cwd:/tmp/proj",
                "fs.delete:/tmp/proj/**",
                "fs.delete:/tmp/proj",
                "workspace:/tmp/proj/ws/**",
                "workspace:/tmp/proj/ws",
                "ollama.list",
                "ollama.show",
                "ollama.pull",
                "mcp.call:websearch:*",
                "app.read",
                "app.control",
                "app.input",
                "loop.next",
                "loop.complete",
                "loop.note",
                "team.run",
            ]
        );
    }

    /// The floor with every conditional grant OFF (no Ollama, no MCP
    /// bridges, no `applications` process, loop not armed) produces just
    /// the always-on base + fs/workspace grants — pins that each gate is
    /// genuinely conditional, not accidentally always-true.
    #[test]
    fn compute_backcompat_floor_omits_grants_when_conditions_are_false() {
        let canonical_root = PathBuf::from("/tmp/proj");
        let floor = compute_backcompat_floor(
            Scope::parse("fs.read:/tmp/proj/**").unwrap(),
            Scope::parse("fs.write:/tmp/proj/**").unwrap(),
            Scope::parse("fs.metadata:/tmp/proj/**").unwrap(),
            &canonical_root,
            None,
            None,
            vec![],
            false,
            &[],
            &[],
            false,
        );

        let scope_strings: Vec<&str> = floor.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            scope_strings,
            vec![
                "memory.read",
                "memory.write",
                "memory.forget",
                "memory.gc",
                "graph.read",
                "skills.list",
                "skills.invoke",
                "schedule.list",
                "schedule.create",
                "schedule.update",
                "schedule.delete",
                "reflection.propose",
                "persona.propose",
                "fs.read:/tmp/proj/**",
                "fs.write:/tmp/proj/**",
                "fs.metadata:/tmp/proj/**",
                "net.fetch",
                "net.post",
                "fs.read:/tmp/proj",
                "fs.write:/tmp/proj",
                "fs.metadata:/tmp/proj",
            ]
        );
    }
```

- [ ] **Step 3: Run the new tests to verify they fail**

```bash
cargo test -p aivyx-cli --lib compute_backcompat_floor -- --test-threads=1
```

Expected: FAIL — `compute_backcompat_floor` is not yet defined (compile
error `cannot find function`).

- [ ] **Step 4: Extract the function**

In `crates/aivyx-cli/src/bin/aivyx.rs`, delete lines 7995–8180 in full (the
`// ---- Capabilities ----` comment through the closing `}` of the
`if loop_state.is_some() { ... }` block at line 8180 — re-verify these are
still the exact current line numbers via Step 1's grep before deleting).
**Do not delete lines 8182–8186** (the "Walk the active role's inheritance
chain..." comment) — that documents the `assemble_role_envelope` call, not
the block being removed, and must stay immediately above it. Replace the
deleted range with nothing at that location — the construction moves to
Step 5's new call site instead, so after deletion the surviving code reads
straight from wherever `apply_ollama_prompt_strategy`'s call ends into the
blank line, then the preserved 8182–8186 comment, then `let role_envelope =
assemble_role_envelope(&role_for_envelope, &roles, &backcompat_floor);`
(originally line 8187) unchanged.

Add this function as a new top-level item, placed directly above
`async fn run_async(` (search for that string to find the insertion point):

```rust
/// The **backcompat floor** (Phase 13 Task 2, Q6): the capabilities the
/// binary used to grant unconditionally before Phase 13 introduced
/// per-role envelopes. Used as the fallback for any role (or inherited
/// level) whose declared `capability_scopes` is empty — and, as of the
/// CLI `team run --config` capability-floor fix, as the operator's own
/// real authority that a vertical pack's declared lead-role scopes get
/// clamped against (`bind_lead_scopes`), mirroring what the daemon path
/// already does.
///
/// The fs.* scopes are rooted at the canonicalized sandbox path so
/// `FsReadTool::required_scope` lines up exactly with the held
/// capability — that's a per-process anchor, not a per-role decision, so
/// it stays inside the floor. The three `memory.*` scopes remain
/// unqualified (D4 Rule 2 — unqualified held grants any qualified
/// needed). `shell.exec` is appended only when the caller passes a
/// `shell_exec_scope` (absent on the SemiTrusted branch, where the tool
/// itself is absent from the dispatch registry). `net.fetch` is granted
/// unqualified; the turn loop's ceiling intersection narrows it for
/// SemiTrusted via `CEILING_SEMITRUSTED`.
#[allow(clippy::too_many_arguments)]
fn compute_backcompat_floor(
    fs_read_scope: Scope,
    fs_write_scope: Scope,
    fs_metadata_scope: Scope,
    canonical_root: &std::path::Path,
    shell_exec_scope: Option<Scope>,
    fs_delete_scope: Option<Scope>,
    workspace_scopes: Vec<Scope>,
    ollama_configured: bool,
    mcp_server_names: &[String],
    config_tool_processes: &[aivyx_config::ToolProcessConfig],
    loop_armed: bool,
) -> Vec<Scope> {
    let mut backcompat_floor: Vec<Scope> = vec![
        Scope::parse("memory.read").unwrap(),
        Scope::parse("memory.write").unwrap(),
        Scope::parse("memory.forget").unwrap(),
        Scope::parse("memory.gc").unwrap(),
        // Chapter Lattice — the default agent may query its own typed
        // knowledge graph (a read of derived memory, like memory.read).
        Scope::parse("graph.read").unwrap(),
        // Phase 110 skills substrate — enumerate + render the agent's own
        // operator-approved skill set (read-only, same self-knowledge class
        // as graph.read). skills.propose stays out — the write half remains
        // auto-proposer / role-declared.
        Scope::parse("skills.list").unwrap(),
        Scope::parse("skills.invoke").unwrap(),
        // Phase 67 schedule tools — READ half: enumerate the agent's own
        // routines (self-knowledge, the skills.list class).
        Scope::parse("schedule.list").unwrap(),
        // Chapter Chime — the WRITE half. The tools themselves enforce the
        // Reins growth gradient, own-schedules-only authority, a 15-minute
        // fire floor, and a 10-schedule cap, so granting the scopes is
        // governance-safe at every level.
        Scope::parse("schedule.create").unwrap(),
        Scope::parse("schedule.update").unwrap(),
        Scope::parse("schedule.delete").unwrap(),
        // Reflection / persona proposals — both governance-safe to grant:
        // a proposal only ever lands as Pending behind the operator's
        // approval gate, so this is the propose half, not self-modification.
        Scope::parse("reflection.propose").unwrap(),
        Scope::parse("persona.propose").unwrap(),
        fs_read_scope,
        fs_write_scope,
        fs_metadata_scope,
        Scope::parse("net.fetch").unwrap(),
        Scope::parse("net.post").unwrap(),
    ];
    // Chapter N — the `<root>/**` scopes above grant the root's
    // DESCENDANTS only; the glob does not match the bare root path. Also
    // grant the root directory itself so the agent can inspect/operate on
    // its own root.
    let root_str = canonical_root.display().to_string();
    backcompat_floor.push(Scope::parse(&format!("fs.read:{root_str}")).unwrap());
    backcompat_floor.push(Scope::parse(&format!("fs.write:{root_str}")).unwrap());
    backcompat_floor.push(Scope::parse(&format!("fs.metadata:{root_str}")).unwrap());
    if let Some(s) = shell_exec_scope {
        backcompat_floor.push(s);
        // Bare-root shell cwd (the run-from-the-root case).
        backcompat_floor
            .push(Scope::parse(&format!("shell.exec:cwd:{root_str}")).unwrap());
    }
    if let Some(s) = fs_delete_scope {
        backcompat_floor.push(s);
        backcompat_floor.push(Scope::parse(&format!("fs.delete:{root_str}")).unwrap());
    }
    // Chapter O — grant the agent its workspace (`workspace:<wsroot>/**` +
    // bare root). Always-on for the default role, independent of fs_root.
    for s in workspace_scopes {
        backcompat_floor.push(s);
    }
    // Phase 36 — grant ollama model management scopes when the provider is
    // Ollama, so the default role (empty capability_scopes) can use them.
    if ollama_configured {
        backcompat_floor.push(Scope::parse("ollama.list").unwrap());
        backcompat_floor.push(Scope::parse("ollama.show").unwrap());
        backcompat_floor.push(Scope::parse("ollama.pull").unwrap());
    }
    // Grant the default role the scope to call every configured MCP
    // server's tools. `mcp.call:<server>:*` grants exactly that server's
    // tools (least-privilege per server).
    for name in mcp_server_names {
        if let Some(s) = Scope::parse(&format!("mcp.call:{name}:*")) {
            backcompat_floor.push(s);
        }
    }
    // Chapter Deckhand — when the `aivyx-apps` desktop tool process is
    // configured, grant the default role the `app.*` scopes its tools
    // require. The ceiling intersection keeps these Trusted-only and
    // `app.input` stays confirm-first.
    if config_tool_processes.iter().any(|tp| tp.name == "applications") {
        for base in ["app.read", "app.control", "app.input"] {
            if let Some(s) = Scope::parse(base) {
                backcompat_floor.push(s);
            }
        }
    }
    // Phase 173 — when the autonomous loop is armed, grant the default
    // role the `loop.*` scopes its iterations require, plus `team.run`
    // (Chapter Circuit CI.0) so the loop can delegate a large story to a
    // durable Nonagon team mission. `git.write` is intentionally NOT
    // granted here (Forge made committing operator opt-in per-repo).
    if loop_armed {
        backcompat_floor.push(Scope::parse("loop.next").unwrap());
        backcompat_floor.push(Scope::parse("loop.complete").unwrap());
        backcompat_floor.push(Scope::parse("loop.note").unwrap());
        backcompat_floor.push(Scope::parse("team.run").unwrap());
    }
    backcompat_floor
}
```

- [ ] **Step 5: Add the early call site and update the existing one**

Immediately before the `// Chapter J — \`aivyx team run "<mission>"\`.`
comment (found in Step 1), insert:

```rust
    // The operator's own real capability floor — moved here (earlier than
    // the role envelope that used to be its only consumer) because `aivyx
    // team run --config <pack.toml>` needs it before assembling the pack's
    // team, to clamp the pack's own declared lead-role scopes against it
    // (closing the CLI capability-floor gap; mirrors what the daemon path
    // already does via `bind_lead_scopes`). All of this function's inputs
    // are already computed above this point.
    let backcompat_floor: Vec<Scope> = compute_backcompat_floor(
        fs_read_scope,
        fs_write_scope,
        fs_metadata_scope,
        &canonical_root,
        shell_exec_scope,
        fs_delete_scope,
        workspace_scopes,
        ollama_base_url_for_tools.is_some(),
        &mcp_bridges.iter().map(|b| b.server_name().to_string()).collect::<Vec<String>>(),
        &config_tool_processes,
        loop_state.is_some(),
    );

```

Then confirm the old site (now just past where lines 7995–8180 used to be,
per Step 4's deletion) reads exactly:

```rust
    // Walk the active role's inheritance chain, intersecting
    // declared scopes leaf-to-root. Empty levels substitute the
    // floor. This is the new primary code path for any operator
    // who has written a `[[role]]` entry; the floor is consulted
    // only as a per-empty-level fallback.
    let role_envelope = assemble_role_envelope(&role_for_envelope, &roles, &backcompat_floor);
```

— the preserved 8182–8186 comment immediately followed by the unchanged
`assemble_role_envelope` call, with nothing else between the end of the
`apply_ollama_prompt_strategy` call (which used to precede the deleted
block) and this comment.

- [ ] **Step 6: Run the new tests to verify they pass**

```bash
cargo test -p aivyx-cli --lib compute_backcompat_floor -- --test-threads=1
```

Expected: PASS (2/2 — `compute_backcompat_floor_covers_every_conditional_grant`,
`compute_backcompat_floor_omits_grants_when_conditions_are_false`).

- [ ] **Step 7: Re-run the existing role-envelope tests unchanged, and the full crate suite**

```bash
cargo test -p aivyx-cli --lib -- --test-threads=1
```

Expected: PASS, including (unmodified)
`empty_role_inherits_backcompat_floor_verbatim`,
`role_with_declared_scope_runs_only_that_scope_not_floor`, and
`child_attenuates_parents_substituted_floor_at_runtime` — these exercise
`assemble_role_envelope` against the small local `floor()` test stub, not
`compute_backcompat_floor`, so they are unaffected by the extraction; their
continued pass confirms nothing downstream broke.

```bash
cargo build -p aivyx-cli
```

Expected: builds cleanly — this is the proof that every input to
`compute_backcompat_floor` really was already in scope before the new call
site, and that nothing between the new and old call sites still expected
`fs_read_scope`/`fs_write_scope`/`fs_metadata_scope`/`shell_exec_scope`/
`fs_delete_scope`/`workspace_scopes` to be usable after this point (they are
now consumed at the new, earlier site).

- [ ] **Step 8: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx
git add crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "Extract compute_backcompat_floor, compute it before aivyx team run

Moves the operator's real capability-floor construction out of its
inline spot (previously consumed only by assemble_role_envelope) into
a standalone, directly-unit-tested function, and calls it before the
CLI's aivyx team run --config early-return branch instead of after —
so that path will be able to clamp a pack's declared lead-role scopes
against it (Task 2).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: Make `bind_lead_scopes` `pub` and clamp `aivyx team run --config`'s pack against the operator's real floor

**Files:**
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs:1402` (the
  `bind_lead_scopes` function signature only — re-confirm this line number
  is still current before editing, since Task 1 may have shifted nothing in
  this file, but re-verify per this plan's own discipline).
- Modify: `crates/aivyx-cli/src/bin/aivyx_modules/team.rs` — add
  `load_and_clamp_team`, thread `lead_scopes` through `run_mission`, add the
  mutation-proof test.
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` — update the one `run_mission`
  call site to pass the newly-available `backcompat_floor` (from Task 1) as
  `lead_scopes`.

**Interfaces:**
- Consumes: `compute_backcompat_floor` and the early-bound `backcompat_floor`
  local variable in `run_async`, both from Task 1.
- Consumes: `bind_lead_scopes(config: &mut TeamConfig, lead_scopes: &[String])`
  — already implemented and tested in `team_mission_driver.rs`; this task
  only widens its visibility, no logic change.
- Produces: `fn load_and_clamp_team(config: Option<&str>, lead_scopes:
  &[String]) -> Result<TeamConfig, String>` in `team.rs` — private to that
  module, used only by `run_mission` in the same file.
- Produces: `run_mission`'s new signature gains one parameter,
  `lead_scopes: &[String]`, inserted immediately after `checkpointer:
  Option<Arc<aivyx_core::GitCheckpointer>>` and before `kv_cache_handles`.

- [ ] **Step 1: Re-verify the exact current source before editing**

```bash
grep -n "^fn bind_lead_scopes" crates/aivyx-channel/src/team_mission_driver.rs
grep -n "fn load_team\|pub async fn run_mission\|let config = load_team" crates/aivyx-cli/src/bin/aivyx_modules/team.rs
grep -n "return team::run_mission" crates/aivyx-cli/src/bin/aivyx.rs
```

Expected (confirm before proceeding; use real current numbers if they've
drifted):

```
team_mission_driver.rs:1402:fn bind_lead_scopes(config: &mut TeamConfig, lead_scopes: &[String]) {
team.rs:67:fn load_team(config: Option<&str>) -> Result<TeamConfig, String> {
team.rs:183:pub async fn run_mission(
team.rs:197:    let config = load_team(config)?;
aivyx.rs:7867:        return team::run_mission(
```

- [ ] **Step 2: Make `bind_lead_scopes` `pub`**

In `crates/aivyx-channel/src/team_mission_driver.rs`, change:

```rust
fn bind_lead_scopes(config: &mut TeamConfig, lead_scopes: &[String]) {
```

to:

```rust
pub fn bind_lead_scopes(config: &mut TeamConfig, lead_scopes: &[String]) {
```

No other change to this function — its logic, tests, and warning behavior
are untouched.

- [ ] **Step 3: Run `team_mission_driver.rs`'s own tests to confirm no regression**

```bash
cargo test -p aivyx-channel --lib bind_lead_scopes -- --test-threads=1
```

Expected: PASS (5/5 — the 2 original tests plus the 3 added by the
bind_lead_scopes Floor-Clamp piece), unchanged, proving the visibility
change alone didn't affect behavior.

- [ ] **Step 4: Write the failing mutation-proof test in `team.rs`**

Add this test to the `mod tests` block in
`crates/aivyx-cli/src/bin/aivyx_modules/team.rs` (search for
`fn roster_handles_a_single_specialist_plural` — the last existing test —
and add this immediately after its closing brace):

```rust
    #[test]
    fn load_and_clamp_team_strips_a_lead_scope_the_floor_does_not_grant() {
        use aivyx_team::config::{DialogueConfig, TeamConfig, TeamMember};

        let dir = scratch("clamp");
        let path = dir.join("pack.toml");
        let m = |name: &str, scopes: Vec<&str>| TeamMember {
            name: name.into(),
            role: "R".into(),
            soul: "s".into(),
            tool_allowlist: vec![],
            capability_scopes: scopes.into_iter().map(String::from).collect(),
            trust_ceiling: TrustTier::Trusted,
            model: None,
            base_url: None,
        };
        let cfg = TeamConfig {
            name: "unaudited-pack".into(),
            description: String::new(),
            lead: "boss".into(),
            // The lead declares a domain scope well beyond team
            // orchestration — exactly the shape an unaudited third-party
            // pack might ship, and exactly what this fix must strip.
            members: vec![m("boss", vec!["shell.exec:cwd:/etc/**", "team.delegate"])],
            dialogue: DialogueConfig::default(),
        };
        std::fs::write(&path, cfg.to_toml().unwrap()).unwrap();

        // The floor grants only team orchestration markers — no shell.exec
        // at all. This is the operator's own real authority; the pack's
        // file must not be able to exceed it.
        let floor = vec!["team.message".to_string(), "team.delegate".to_string()];
        let clamped = load_and_clamp_team(Some(path.to_str().unwrap()), &floor).unwrap();
        let lead = clamped.lead_member().unwrap();

        assert!(
            !lead.capability_scopes.iter().any(|s| s.starts_with("shell.exec")),
            "lead scopes should not include the out-of-floor domain scope: {:?}",
            lead.capability_scopes
        );
        assert!(
            lead.capability_scopes.contains(&"team.delegate".to_string()),
            "the legitimate orchestration marker must still flow through: {:?}",
            lead.capability_scopes
        );
    }
```

- [ ] **Step 5: Run the test to verify it fails**

```bash
cargo test -p aivyx-cli --lib load_and_clamp_team -- --test-threads=1
```

Expected: FAIL — `load_and_clamp_team` is not yet defined (compile error
`cannot find function`).

- [ ] **Step 6: Add `load_and_clamp_team` and wire it into `run_mission`**

In `crates/aivyx-cli/src/bin/aivyx_modules/team.rs`, add this function
immediately after `load_team` (search for `fn load_team` to find the
insertion point, right after its closing brace):

```rust
/// Load a team config, then clamp the lead's (and every specialist's)
/// pack-declared `capability_scopes` to `lead_scopes` — the CLI-run
/// equivalent of the daemon path's own `bind_lead_scopes` call
/// (`aivyx-channel`'s `team_mission_driver.rs`, used by
/// `TeamMissionService`'s `assemble_runtime`). Closes the CLI team-run
/// capability-floor gap: without this clamp, a vertical pack's own file
/// could grant its lead role — and therefore, via delegation, its
/// specialists — any `capability_scopes` it declares, regardless of the
/// operator's own real, already-configured authority.
fn load_and_clamp_team(config: Option<&str>, lead_scopes: &[String]) -> Result<TeamConfig, String> {
    let mut config = load_team(config)?;
    aivyx_channel::team_mission_driver::bind_lead_scopes(&mut config, lead_scopes);
    Ok(config)
}
```

Then in `run_mission`, add the `lead_scopes: &[String]` parameter (insert it
immediately after the `checkpointer` parameter):

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
) -> Result<(), String> {
    let config = load_and_clamp_team(config, lead_scopes)?;
    let team_name = config.name.clone();
```

(The rest of `run_mission`'s body is unchanged — `let team_name =
config.name.clone();` and everything below it already exist; only the
`let config = ...;` line's right-hand side and the new parameter change.)

- [ ] **Step 7: Update the one call site in `aivyx.rs`**

In `crates/aivyx-cli/src/bin/aivyx.rs`, immediately before the
`return team::run_mission(` call (found in Step 1), add:

```rust
        let cli_lead_scopes: Vec<String> =
            backcompat_floor.iter().map(|s| s.as_str().to_string()).collect();
```

Then add `&cli_lead_scopes` as an argument to the call, in the same position
as the new parameter in `run_mission`'s signature (immediately after
`checkpointer.clone(),`):

```rust
    if let CliMode::Team(TeamSubcommand::Run { mission, config }) = &mode {
        let cli_lead_scopes: Vec<String> =
            backcompat_floor.iter().map(|s| s.as_str().to_string()).collect();
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
        .await;
    }
```

- [ ] **Step 8: Run the mutation-proof test to verify it passes**

```bash
cargo test -p aivyx-cli --lib load_and_clamp_team -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 9: Perform the mutation-proof — confirm the test actually catches the bug**

Temporarily comment out the `bind_lead_scopes` call inside
`load_and_clamp_team` (so it reads `load_team(config)` with no clamp, same
as the original bug), re-run the same test command from Step 8, and confirm
it now FAILS with the `!lead.capability_scopes.iter().any(...)` assertion
failing (the unclamped `shell.exec:cwd:/etc/**` is still present). Then
restore the `bind_lead_scopes` call and re-run once more to confirm it
passes again. Record both outputs in the task report — this is the actual
proof the test detects the vulnerability it's named for, not just an
assertion that happens to pass against working code.

- [ ] **Step 10: Run the full `aivyx-cli` and `aivyx-channel` suites**

```bash
cargo test -p aivyx-cli --lib -- --test-threads=1
cargo test -p aivyx-channel --lib -- --test-threads=1
```

Expected: PASS, 0 failures in both crates.

```bash
cargo build --workspace
cargo clippy -p aivyx-cli -p aivyx-channel --all-targets -- -D warnings
```

Expected: clean build; clippy has no new warnings introduced by this task
(the codebase's own pre-existing `trigger.rs:223` `clippy::result_unit_err`
finding, if still present, predates this work and is out of scope).

- [ ] **Step 11: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx
git add crates/aivyx-channel/src/team_mission_driver.rs \
        crates/aivyx-cli/src/bin/aivyx_modules/team.rs \
        crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "Clamp aivyx team run --config's pack to the operator's real floor

Makes bind_lead_scopes pub (aivyx-channel) and calls it from a new
load_and_clamp_team helper that run_mission now uses in place of the
unclamped load_team — closing the last open item from the
bind_lead_scopes Floor-Clamp piece's own final review: the CLI path
granted a pack's lead role exactly and unconditionally whatever
capability_scopes it declared, worse than the bug that piece fixed.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

# Vertical-Pack-Aware Capability Floor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the Critical, Important, and Important findings from the CLI
capability-floor branch's own final review: `bind_lead_scopes`'s floor has
no concept of vertical-pack domain scopes (collapsing every real pack's
specialists and stripping the lead's domain scopes), the lead branch widens
a conservative pack's declared scopes to the full floor unconditionally,
and `aivyx team roster --config` displays scopes that may not match what's
actually granted at runtime.

**Architecture:** Generalize `compute_backcompat_floor` to derive its
tool-process/MCP/Ollama grants from the actually-registered `tool_list`'s
own `required_scope()` bases instead of three hand-written special cases.
Unify `bind_lead_scopes`'s lead and specialist branches into one path so
the lead is intersected against its own declared bases like any other
member. Add an honest caveat line to `render_roster`'s output.

**Tech Stack:** Rust, `aivyx-cli`/`aivyx-channel`/`aivyx-team`/
`aivyx-capability` crates already in the workspace. No new dependencies.

## Global Constraints

- The exclusion list (`fs.read`, `fs.write`, `fs.metadata`, `fs.delete`,
  `shell.exec`, `net.fetch`, `net.post`, `workspace`) in
  `compute_backcompat_floor`'s generic sweep is a security boundary, not a
  style choice — granting any of these bases bare (unqualified) would be a
  real regression (D4 Rule 2: an unqualified held scope grants any
  qualified need). Any new test covering this sweep must include at least
  one `fs.read`-shaped entry in its `tool_scope_bases` fixture and assert
  the bare `"fs.read"` scope is **not** present in the output, not just
  that the qualified one is.
- `mcp.call` must never appear bare/unqualified in
  `compute_backcompat_floor`'s output — only as `mcp.call:<server>:*` per
  configured server. Any new test must include an
  `mcp.call:<server>:<tool>`-shaped entry and assert only the wildcard form
  appears.
- Narrowing `bind_lead_scopes`' lead branch must not change what a
  specialist receives — specialists are filtered against `lead_scopes`
  (the function's own parameter, the real floor) directly, never against
  the lead's post-processed `capability_scopes` field. The plan's own test
  suite must re-run `bind_lead_scopes_flows_qualified_mcp_grants_to_declaring_roles`
  **byte-identical, unmodified** (it makes zero lead-specific assertions)
  and confirm it still passes, proving the lead-branch change is isolated
  to the lead's own field.
- Every new test this plan adds that claims to close a Critical/Important
  finding must be a genuine mutation-proof, shown to actually fail against
  the code as it exists at the start of this plan (before the fix) — not
  just an assertion that happens to pass. This applies with extra weight
  to the kitchen-shaped specialist-scopes test (the direct proof for the
  Critical finding) and the conservative-lead test (the direct proof for
  the widening finding).
- All of this branch's existing tests must keep passing: 564 in `aivyx-cli`
  (`cargo test -p aivyx-cli --bin aivyx`, no `--lib` — this crate has no
  lib target), `bind_lead_scopes`'s own 4 in `aivyx-channel` (though this
  plan intentionally changes the assertions inside one of them — see
  Task 2), plus the branch's own mutation-proof
  `load_and_clamp_team_strips_a_lead_scope_the_floor_does_not_grant`.
- `cargo build -p aivyx-cli -p aivyx-channel` clean. Do **not** run `cargo
  build --workspace` or `cargo test --workspace` — the full workspace has
  an unrelated, pre-existing, out-of-scope build failure
  (`javascriptcoregtk-4.1` missing system library in a GUI crate).

---

### Task 1: Generalize `compute_backcompat_floor`'s tool-derived grants

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs`
  - `compute_backcompat_floor`'s signature (currently at line 5276) and
    body (through line 5385).
  - Its one call site (currently lines 7998–8010, immediately before the
    `aivyx team run` branch at line 8018).
  - Its two existing tests, `compute_backcompat_floor_covers_every_conditional_grant`
    (line 11051) and `compute_backcompat_floor_omits_grants_when_conditions_are_false`
    (line 11130).

Re-run this file's own `grep -n "^fn compute_backcompat_floor\|^async fn
run_async\|let backcompat_floor: Vec<Scope> = compute_backcompat_floor\|if
let CliMode::Team(TeamSubcommand::Run"` before editing — line numbers may
have drifted from a prior task on this branch; use whatever the grep
reports as ground truth for every step below.

**Interfaces:**
- Produces: `compute_backcompat_floor`'s new signature —
  `fn compute_backcompat_floor(fs_read_scope: Scope, fs_write_scope: Scope,
  fs_metadata_scope: Scope, canonical_root: &std::path::Path,
  shell_exec_scope: Option<Scope>, fs_delete_scope: Option<Scope>,
  workspace_scopes: Vec<Scope>, tool_scope_bases: &[Scope], loop_armed:
  bool) -> Vec<Scope>` — three old parameters (`ollama_configured: bool`,
  `mcp_server_names: &[String]`, `config_tool_processes:
  &[aivyx_config::ToolProcessConfig]`) are replaced by the one new
  `tool_scope_bases: &[Scope]` parameter. Task 2 and Task 3 do not call
  this function; only this task's own call site and tests are affected.

- [ ] **Step 1: Write the failing tests (replace both existing test bodies)**

Replace `compute_backcompat_floor_covers_every_conditional_grant` in full
with:

```rust
    /// Pins `compute_backcompat_floor`'s exact output — every conditional
    /// grant fires, sourced generically from `tool_scope_bases` (an Ollama
    /// tool, two tools on the same MCP server, three `applications` tools,
    /// and — new — a `fs.read`-shaped entry proving the exclusion list
    /// keeps a built-in tool's own qualified scope from also being granted
    /// bare) instead of three hand-written special cases.
    #[test]
    fn compute_backcompat_floor_covers_every_conditional_grant() {
        let canonical_root = PathBuf::from("/tmp/proj");
        let tool_scope_bases: Vec<Scope> = vec![
            Scope::parse("ollama.list").unwrap(),
            Scope::parse("ollama.show").unwrap(),
            Scope::parse("ollama.pull").unwrap(),
            // Two tools on the same MCP server — must collapse to ONE
            // mcp.call:websearch:* grant, never the bare "mcp.call" base.
            Scope::parse("mcp.call:websearch:search").unwrap(),
            Scope::parse("mcp.call:websearch:fetch").unwrap(),
            Scope::parse("app.read").unwrap(),
            Scope::parse("app.control").unwrap(),
            Scope::parse("app.input").unwrap(),
            // A built-in fs tool's own required_scope is qualified, exactly
            // like the one already passed via fs_read_scope below — proves
            // the exclusion list keeps this from ALSO being granted bare.
            Scope::parse("fs.read:/tmp/proj/**").unwrap(),
        ];
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
            &tool_scope_bases,
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
                // The generic sweep's output is a sorted set (BTreeSet),
                // deliberately alphabetical rather than tool_list's own
                // iteration order.
                "app.control",
                "app.input",
                "app.read",
                "mcp.call:websearch:*",
                "ollama.list",
                "ollama.pull",
                "ollama.show",
                "loop.next",
                "loop.complete",
                "loop.note",
                "team.run",
            ]
        );
        // Explicit, redundant-on-purpose per the plan's own global
        // constraint: the bare, unqualified forms must never appear,
        // even though the exact-Vec assertion above already implies it.
        assert!(
            !scope_strings.contains(&"fs.read"),
            "fs.read must never appear bare — only the qualified forms above"
        );
        assert!(
            !scope_strings.contains(&"mcp.call"),
            "mcp.call must never appear bare — only mcp.call:<server>:* above"
        );
    }
```

Replace `compute_backcompat_floor_omits_grants_when_conditions_are_false`
in full with:

```rust
    /// The floor with an empty `tool_scope_bases` and the loop not armed
    /// produces just the always-on base + fs/workspace grants — pins that
    /// the generic sweep and the loop gate are genuinely conditional, not
    /// accidentally always-true.
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

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd /home/julian/Projects/Rust/aivyx/.worktrees/cli-team-run-capability-floor
cargo test -p aivyx-cli --bin aivyx compute_backcompat_floor -- --test-threads=1
```

Expected: FAIL — compile error, `compute_backcompat_floor` still takes the
old 11-parameter signature (`ollama_configured`, `mcp_server_names`,
`config_tool_processes`), not the new 9-parameter one.

- [ ] **Step 3: Change `compute_backcompat_floor`'s signature and body**

Change the function signature from:

```rust
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
```

to:

```rust
fn compute_backcompat_floor(
    fs_read_scope: Scope,
    fs_write_scope: Scope,
    fs_metadata_scope: Scope,
    canonical_root: &std::path::Path,
    shell_exec_scope: Option<Scope>,
    fs_delete_scope: Option<Scope>,
    workspace_scopes: Vec<Scope>,
    tool_scope_bases: &[Scope],
    loop_armed: bool,
) -> Vec<Scope> {
```

Leave the function body completely unchanged from its start through the
`for s in workspace_scopes { backcompat_floor.push(s); }` loop (the fixed
baseline vec, the root-dir pushes, `shell_exec_scope`, `fs_delete_scope`,
`workspace_scopes` — none of that changes).

Replace the three blocks that follow it — the `if ollama_configured { ...
}` block, the `for name in mcp_server_names { ... }` loop, and the `if
config_tool_processes.iter().any(...) { ... }` block — with:

```rust
    // Generic sweep: every distinct domain scope some actually-registered
    // tool (built-in, MCP-discovered, or tool-process-proxied — kitchen,
    // applications, any future vertical toolkit) requires, minus the bases
    // already granted precisely above (path-qualified, so a bare grant
    // here would be a real over-broadening, not a harmless duplicate —
    // D4 Rule 2: an unqualified held scope grants any qualified need), and
    // with mcp.call resolved to a per-server wildcard rather than its bare
    // (D4-Rule-2-violating) base. This subsumes what used to be three
    // hand-written special cases (Ollama, MCP, `applications`) — any
    // future tool-process's own domain scopes are picked up automatically,
    // closing the class of bug this file's own history already hit five
    // times (Chapter Lattice, Phase 110, Phase 67, Chapter Chime, Vitrine)
    // before this fix generalized it.
    const EXPLICIT_BASES: &[&str] = &[
        "fs.read", "fs.write", "fs.metadata", "fs.delete", "shell.exec",
        "net.fetch", "net.post", "workspace",
    ];
    let mut derived: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for scope in tool_scope_bases {
        let base = scope.base();
        if base == "mcp.call" {
            // Qualified per-tool grant like "mcp.call:<server>:<tool>" —
            // extract the server segment and grant the whole server.
            // Never grant "mcp.call" bare, which would unlock every
            // configured server (D4 Rule 2).
            let parts: Vec<&str> = scope.as_str().splitn(3, ':').collect();
            if parts.len() == 3 && !parts[1].is_empty() {
                derived.insert(format!("mcp.call:{}:*", parts[1]));
            }
            continue;
        }
        if EXPLICIT_BASES.contains(&base) {
            continue;
        }
        derived.insert(base.to_string());
    }
    for s in derived {
        if let Some(scope) = Scope::parse(&s) {
            backcompat_floor.push(scope);
        }
    }
```

Leave the trailing `if loop_armed { ... }` block and the final
`backcompat_floor` return unchanged.

- [ ] **Step 4: Update the one call site**

Immediately before the `let backcompat_floor: Vec<Scope> =
compute_backcompat_floor(` line, add:

```rust
    let tool_scope_bases: Vec<Scope> =
        tool_list.iter().map(|t| t.required_scope(&serde_json::json!({}))).collect();
```

Then change the call itself from:

```rust
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

to:

```rust
    let backcompat_floor: Vec<Scope> = compute_backcompat_floor(
        fs_read_scope,
        fs_write_scope,
        fs_metadata_scope,
        &canonical_root,
        shell_exec_scope,
        fs_delete_scope,
        workspace_scopes,
        &tool_scope_bases,
        loop_state.is_some(),
    );
```

Do not remove `ollama_base_url_for_tools`, `mcp_bridges`, or
`config_tool_processes` themselves — they are each still used elsewhere in
`run_async` for unrelated purposes (building the Ollama tools, the MCP
hot-swap watcher, spawning tool processes). Only their use as arguments to
`compute_backcompat_floor` is removed.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p aivyx-cli --bin aivyx compute_backcompat_floor -- --test-threads=1
```

Expected: PASS (2/2).

- [ ] **Step 6: Run the full `aivyx-cli` suite and a clean build**

```bash
cargo test -p aivyx-cli --bin aivyx -- --test-threads=1
cargo build -p aivyx-cli
```

Expected: 564 passed, 0 failed (same count as before this task — this task
changes two existing tests' bodies, not their count); clean build.

- [ ] **Step 7: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx/.worktrees/cli-team-run-capability-floor
git add crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "Generalize compute_backcompat_floor's tool-derived grants

Replaces the three hand-written special cases (Ollama, MCP,
applications) with one generic sweep over tool_list's own registered
required_scope() bases, so any operator-configured tool process's
domain scopes (kitchen.*, or any future vertical) flow into the floor
automatically. Preserves the security boundary around fs/shell/net/
workspace (bare grants excluded) and mcp.call's per-server-wildcard
semantics (never granted bare).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: Unify `bind_lead_scopes`'s lead and specialist branches

**Files:**
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs`
  - `bind_lead_scopes` (currently at line 1402).
  - `bind_lead_scopes_grants_per_role_and_stays_least_privilege` (currently
    at line 2387) — its lead-specific assertions change; its
    writer/researcher/reviewer assertions do not.
  - Two new tests, added after
    `bind_lead_scopes_flows_qualified_mcp_grants_to_declaring_roles`
    (currently ending around line 2480) — which itself is **not** modified
    at all.

Re-run `grep -n "^pub fn bind_lead_scopes\|fn bind_lead_scopes_grants_per_role\|fn bind_lead_scopes_flows_qualified\|fn custom_team\|^    fn member("
crates/aivyx-channel/src/team_mission_driver.rs` before editing to confirm
current line numbers.

**Interfaces:**
- Consumes: nothing from Task 1 (different crate, no shared types beyond
  `Scope`, already imported here).
- Produces: `bind_lead_scopes`'s signature is unchanged
  (`pub fn bind_lead_scopes(config: &mut TeamConfig, lead_scopes:
  &[String])`) — only its internal logic changes. Task 3 does not depend
  on this task.

- [ ] **Step 1: Write the two new failing tests**

Add these after `bind_lead_scopes_flows_qualified_mcp_grants_to_declaring_roles`'s
closing brace (that test itself is not modified):

```rust
    #[test]
    fn bind_lead_scopes_narrows_a_conservative_leads_declared_scopes_not_the_full_floor() {
        // A floor far broader than what this lead actually declared for
        // itself — before the fix, the lead branch grants the WHOLE floor
        // unconditionally regardless of what it declared.
        let floor: Vec<String> = [
            "memory.read",
            "fs.write:/root/**",
            "shell.exec:cwd:/root/**",
            "net.fetch",
            "team.delegate",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let mut config = default_nonagon();
        let lead_name = config.lead.clone();
        for m in &mut config.members {
            if m.name == lead_name {
                // Override the lead's own declared scopes to a
                // deliberately conservative set — simulating a pack whose
                // lead never intended to touch fs/shell/net directly.
                m.capability_scopes =
                    vec!["team.delegate".to_string(), "team.message".to_string()];
            }
        }
        bind_lead_scopes(&mut config, &floor);
        let lead = config.members.iter().find(|m| m.name == lead_name).unwrap();
        assert_eq!(
            lead.capability_scopes,
            vec!["team.delegate".to_string(), "team.message".to_string()],
            "a conservative lead must stay conservative, not widen to the \
             full floor: {:?}",
            lead.capability_scopes
        );
    }

    #[test]
    fn bind_lead_scopes_lets_a_configured_verticals_domain_scopes_flow_through() {
        // Shaped like the real shipped kitchen-boh.toml pack (aria + 4
        // specialists; crates/verticals/aivyx-kitchen/assets/kitchen-boh.toml).
        // This floor is what a generalized compute_backcompat_floor now
        // produces once the kitchen tool-process is configured — it
        // includes the vertical's own domain scope bases, not just the
        // interactive-session ones.
        let floor: Vec<String> = [
            "memory.read",
            "memory.write",
            "team.delegate",
            "kitchen.read",
            "kitchen.write",
            "kitchen.order.send",
            "kitchen.haccp.log",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let mut config = TeamConfig {
            name: "kitchen-boh".into(),
            description: "Back-of-House Nonagon".into(),
            lead: "aria".into(),
            members: vec![
                member(
                    "aria",
                    "BOH Manager",
                    &[
                        "kitchen.read",
                        "kitchen.write",
                        "kitchen.order.send",
                        "kitchen.haccp.log",
                        "team.delegate",
                        "team.message",
                    ],
                ),
                member(
                    "stocktake",
                    "Stocktake",
                    &["kitchen.read", "kitchen.write", "team.message"],
                ),
                member("inventory", "Inventory Analyst", &["kitchen.read", "team.message"]),
                member(
                    "purchasing",
                    "Purchasing",
                    &["kitchen.read", "kitchen.write", "kitchen.order.send", "team.message"],
                ),
                member("haccp", "Food-Safety / Compliance", &["kitchen.haccp.log", "team.message"]),
            ],
            dialogue: Default::default(),
        };
        bind_lead_scopes(&mut config, &floor);
        let caps = |name: &str| -> Vec<String> {
            config.members.iter().find(|m| m.name == name).unwrap().capability_scopes.clone()
        };

        // Every specialist retains its own domain scopes — the direct
        // mutation-proof for the Critical finding: against the current,
        // unfixed bind_lead_scopes, every one of these would collapse to
        // [team.message] only, because the floor before this fix never
        // contains any kitchen.* base.
        assert!(caps("stocktake").contains(&"kitchen.read".to_string()));
        assert!(caps("stocktake").contains(&"kitchen.write".to_string()));
        assert!(caps("inventory").contains(&"kitchen.read".to_string()));
        assert!(caps("purchasing").contains(&"kitchen.order.send".to_string()));
        assert!(caps("haccp").contains(&"kitchen.haccp.log".to_string()));
        // The lead keeps exactly its own declared domain + orchestration
        // scopes (not the full floor, and not stripped either).
        let aria = caps("aria");
        for s in [
            "kitchen.read",
            "kitchen.write",
            "kitchen.order.send",
            "kitchen.haccp.log",
            "team.delegate",
            "team.message",
        ] {
            assert!(aria.contains(&s.to_string()), "aria should hold {s}: {aria:?}");
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cd /home/julian/Projects/Rust/aivyx/.worktrees/cli-team-run-capability-floor
cargo test -p aivyx-channel --lib bind_lead_scopes -- --test-threads=1
```

Expected: FAIL —
`bind_lead_scopes_narrows_a_conservative_leads_declared_scopes_not_the_full_floor`
fails because the lead still receives the full floor;
`bind_lead_scopes_lets_a_configured_verticals_domain_scopes_flow_through`
fails because every specialist collapses to `[team.message]` (the floor
has no `kitchen.*` base yet — `bind_lead_scopes` itself isn't changed
until Step 3).

- [ ] **Step 3: Unify the lead and specialist branches**

Replace `bind_lead_scopes`'s full body (the `if lead_scopes.is_empty() {
return; }` guard stays; everything from `let lead_name = ...` through the
closing brace of the `for m in &mut config.members` loop is replaced):

```rust
pub fn bind_lead_scopes(config: &mut TeamConfig, lead_scopes: &[String]) {
    if lead_scopes.is_empty() {
        return;
    }
    for m in &mut config.members {
        // The scope bases this member's own role declares (every member —
        // lead included — is attenuated to floor ∩ its own declared
        // bases, never the unconditional full floor: a conservative
        // member's deliberately narrow declaration stays narrow, and a
        // member that needs broad authority to delegate onward — the
        // usual lead shape — still gets it, because it declares the
        // matching bases itself).
        let mut bases: std::collections::HashSet<&str> =
            m.capability_scopes.iter().map(|s| scope_base(s)).collect();
        if m.tool_allowlist.iter().any(|t| t.starts_with("workspace.")) {
            bases.insert("workspace");
        }
        // Grant the floor's scopes for those bases…
        let mut caps: Vec<String> = lead_scopes
            .iter()
            .filter(|s| bases.contains(scope_base(s)))
            .cloned()
            .collect();
        // …and keep the non-floor scopes the roster declared: the team
        // bus, plus any QUALIFIED MCP grants a custom roster pinned. The
        // bare "mcp.call" roster entry is a marker only — declaring the
        // base makes the floor's qualified `mcp.call:<server>:*` grants
        // flow through the filter above; pushing the bare form here would
        // grant every server unqualified (D4 Rule 2), which is broader
        // than the operator's configured set.
        let (kept, dropped) = filter_orchestration_markers(&m.capability_scopes);
        let really_exceeds_floor = scopes_exceeding_floor(&dropped, lead_scopes);
        if !really_exceeds_floor.is_empty() {
            eprintln!(
                "aivyx team: {}'s own declared scopes exceed the daemon floor, \
                 clamped: {}",
                m.name,
                really_exceeds_floor.join(", ")
            );
        }
        caps.extend(kept);
        caps.sort();
        caps.dedup();
        m.capability_scopes = caps;
    }
}
```

This deletes the `if m.name == lead_name { ... } else { ... }` split
entirely — every member, lead included, now runs through the same
computation. Also update the function's own doc comment (the block
immediately above `pub fn bind_lead_scopes`) — replace the sentence "This
grants the LEAD the daemon's full floor (so it can grant), and each
specialist the lead's floor scopes whose BASE its role declares" with:
"Every member — lead included — is attenuated to floor ∩ its own declared
scope bases, plus narrow orchestration markers (team bus, qualified MCP).
A lead therefore holds exactly what it declared for itself, not the
daemon's full floor; it can still delegate broad authority to specialists
because specialists are filtered against the real floor parameter
directly, never against the lead's own post-processed field." Leave the
rest of the doc comment (the NT-02 / "missions report done but do nothing"
history) unchanged.

- [ ] **Step 4: Update the lead-specific assertions in the existing per-role test**

In `bind_lead_scopes_grants_per_role_and_stays_least_privilege`, replace
only these three lines:

```rust
        // Lead holds the FULL floor (so it can grant) + its own orchestration.
        let lead = caps("coordinator");
        assert!(lead.contains(&"net.fetch".to_string()));
        assert!(lead.contains(&"fs.write:/root/**".to_string()));
        assert!(lead.iter().any(|s| s == "team.delegate"));
```

with:

```rust
        // Lead holds exactly its own declared scopes (memory.read/write +
        // team.delegate, per default_nonagon's own coordinator role),
        // intersected with the floor, plus team.message — NOT the full
        // floor. default_nonagon's coordinator never declared fs/net/shell
        // for itself (its own soul: "You never execute domain work
        // directly"), so those stay absent even though the floor grants
        // them to roles that DO declare matching bases (see `writer`/
        // `researcher` below, which keep this test's own original intent).
        let lead = caps("coordinator");
        assert_eq!(
            lead,
            vec![
                "memory.read".to_string(),
                "memory.write".to_string(),
                "team.delegate".to_string(),
                "team.message".to_string(),
            ]
        );
```

Leave every other line in this test (the `writer`/`researcher`/`reviewer`
assertions) exactly as they are.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p aivyx-channel --lib bind_lead_scopes -- --test-threads=1
```

Expected: PASS (6/6 — the 4 pre-existing plus the 2 new ones from Step 1).

- [ ] **Step 6: Perform the mutation-proof for both new tests**

For `bind_lead_scopes_narrows_a_conservative_leads_declared_scopes_not_the_full_floor`:
temporarily revert Step 3's change (restore the original `if m.name ==
lead_name { let mut caps: Vec<String> = lead_scopes.to_vec(); ... }
else { ... }` split — check it out from this task's own pre-Step-3 state
via `git diff` or `git stash` rather than retyping it from memory), re-run
the test, confirm it now FAILS (the lead ends up with the full floor,
including `fs.write:/root/**` and `shell.exec:cwd:/root/**`, not just
`team.delegate`/`team.message`), then restore Step 3's real fix and
confirm it passes again.

For `bind_lead_scopes_lets_a_configured_verticals_domain_scopes_flow_through`:
this one's mutation-proof is already established by Step 2 (it failed
before Step 3's fix, using the OLD `bind_lead_scopes` body) — no
additional mutation needed, but re-confirm by running it once more here
alongside the other test.

Record the raw command output for both the reverted (failing) and restored
(passing) runs in the task report.

- [ ] **Step 7: Run the full `aivyx-channel` suite and a clean build**

```bash
cargo test -p aivyx-channel --lib -- --test-threads=1
cargo build -p aivyx-channel
cargo clippy -p aivyx-channel --lib -- -D warnings
```

Expected: all passing (1302 + 2 new = 1304); clean build; clippy clean
except the pre-existing, unrelated `trigger.rs:223` `clippy::result_unit_err`
finding (predates this branch, not touched by this task).

- [ ] **Step 8: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx/.worktrees/cli-team-run-capability-floor
git add crates/aivyx-channel/src/team_mission_driver.rs
git commit -m "Unify bind_lead_scopes' lead and specialist branches

The lead branch previously replaced a pack's declared lead scopes with
the full floor unconditionally, silently widening a conservative
pack's deliberately narrow lead declaration. Every member, lead
included, is now attenuated to floor ∩ its own declared bases plus
orchestration markers, mirroring what the specialist branch already
did correctly. Specialists are unaffected -- they're filtered against
the real floor parameter directly, never the lead's own field. The
clamp-exceeds-floor warning now fires for any member, not just the
lead.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 3: `render_roster` caveat

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx_modules/team.rs`
  - `render_roster` (currently at line 29).
  - Add one new test after `roster_handles_a_single_specialist_plural`
    (currently ending around line 496).

Re-run `grep -n "^pub fn render_roster\|fn roster_handles_a_single_specialist_plural"
crates/aivyx-cli/src/bin/aivyx_modules/team.rs` before editing to confirm
current line numbers.

**Interfaces:**
- Consumes: nothing from Tasks 1 or 2 (`render_roster`'s own signature is
  unchanged — `pub fn render_roster(config: &TeamConfig) -> String`).

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn roster_shows_a_declared_not_guaranteed_caveat() {
        let out = render_roster(&default_nonagon());
        assert!(
            out.contains("declared") && out.contains("aivyx team run"),
            "roster output should caveat that scopes are declared, not \
             guaranteed, and point at where the real grant happens: {out}"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd /home/julian/Projects/Rust/aivyx/.worktrees/cli-team-run-capability-floor
cargo test -p aivyx-cli --bin aivyx roster_shows_a_declared_not_guaranteed_caveat -- --test-threads=1
```

Expected: FAIL — the current output contains neither string.

- [ ] **Step 3: Add the caveat line**

Change `render_roster` from:

```rust
pub fn render_roster(config: &TeamConfig) -> String {
    let specialists = config.specialists().count();
    let mut out = format!(
        "Team: {} — {}\n  lead: {} ({} specialist{})\n",
        config.name,
        if config.description.is_empty() { "(no description)" } else { &config.description },
        config.lead,
        specialists,
        if specialists == 1 { "" } else { "s" },
    );
    for m in &config.members {
        let tag = if m.name == config.lead { "lead " } else { "spec " };
        let scopes = if m.capability_scopes.is_empty() {
            "(none)".to_string()
        } else {
            m.capability_scopes.join(", ")
        };
        out.push_str(&format!(
            "  [{tag}] {:<12} {:<24} trust={}\n             scopes: {scopes}\n",
            m.name,
            m.role,
            trust_label(m.trust_ceiling),
        ));
    }
    out
}
```

to:

```rust
pub fn render_roster(config: &TeamConfig) -> String {
    let specialists = config.specialists().count();
    let mut out = format!(
        "Team: {} — {}\n  lead: {} ({} specialist{})\n",
        config.name,
        if config.description.is_empty() { "(no description)" } else { &config.description },
        config.lead,
        specialists,
        if specialists == 1 { "" } else { "s" },
    );
    for m in &config.members {
        let tag = if m.name == config.lead { "lead " } else { "spec " };
        let scopes = if m.capability_scopes.is_empty() {
            "(none)".to_string()
        } else {
            m.capability_scopes.join(", ")
        };
        out.push_str(&format!(
            "  [{tag}] {:<12} {:<24} trust={}\n             scopes: {scopes}\n",
            m.name,
            m.role,
            trust_label(m.trust_ceiling),
        ));
    }
    out.push_str(
        "\nNote: scopes above are each member's own declared ask. Actual \
         grants are computed when the team runs (`aivyx team run` /\n\
         `aivyx team start`) against your own configured authority, and \
         may be narrower.\n",
    );
    out
}
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
cargo test -p aivyx-cli --bin aivyx roster_shows_a_declared_not_guaranteed_caveat -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 5: Run the full `aivyx-cli` suite and a clean build**

```bash
cargo test -p aivyx-cli --bin aivyx -- --test-threads=1
cargo build -p aivyx-cli
```

Expected: 565 passed (564 + 1 new), 0 failed — including
`roster_renders_the_default_nonagon` and
`roster_handles_a_single_specialist_plural` unchanged (both use
`.contains(...)` substring checks on strings this change doesn't remove or
rename, so they remain valid without modification); clean build.

- [ ] **Step 6: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx/.worktrees/cli-team-run-capability-floor
git add crates/aivyx-cli/src/bin/aivyx_modules/team.rs
git commit -m "Caveat aivyx team roster's displayed scopes as declared, not guaranteed

render_roster dispatches before any capability floor exists in
run_async, so it cannot show what will actually be granted at run/
start time. Rather than an expensive recomputation that would break
its offline contract, the output now says plainly that displayed
scopes are the pack's own ask, not a runtime guarantee.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

# `aivyx-coder` as a Nonagon specialist — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a Nonagon team delegate a bounded coding task to a separate `aivyx-coder --mcp-server` process, by documenting it as a curated `docs/MCP_RECIPES.md` recipe (code + doc, kept in sync in one commit) and a worked `docs/NONAGON.md` example — then prove the whole path live, end to end, on the real rig.

**Architecture:** No new engine code. `aivyx-coder --mcp-server` becomes one more `[[mcp_server]]` entry in `aivyx`'s own config; its bridged `code`/`code_reply` tools land in the daemon's `ToolRegistry` under `mcp.call:aivyx-coder:*` exactly like any other MCP server (`crates/aivyx-mcp`), and `aivyx-team`'s existing `"mcp.call"` allowlist marker (`crates/aivyx-team/src/factory.rs::filter_tools`) admits them into any specialist that declares a matching `capability_scopes` entry. This plan adds: one `Recipe` literal + one doc section (`docs/MCP_RECIPES.md` + its backing `RECIPES` slice), one worked example in `docs/NONAGON.md`, and a live verification pass — nothing in `aivyx-core`, `aivyx-team`, `aivyx-mcp`, or `aivyx-capability` changes.

**Tech Stack:** Rust (workspace `aivyx`), TOML config, no new dependencies.

## Global Constraints

- The recipe's capability-scope guidance MUST read `mcp.call:aivyx-coder:*` — **not** the bare `mcp.call:aivyx-coder` that appeared in the approved design doc. Verified against the real `Scope` parser (`crates/aivyx-capability/src/lib.rs`): a qualified held scope only grants an exactly-matching or glob-matching needed scope (Rule 3/4); `mcp.call` uses `SimpleGlob` dispatch, so the trailing `:*` is required to cover both `mcp.call:aivyx-coder:code` and `mcp.call:aivyx-coder:code_reply`. A bare `mcp.call:aivyx-coder` (no `:*`) would only match a needed scope of exactly `mcp.call:aivyx-coder`, which nothing ever requests.
- A specialist's `tool_allowlist` admits MCP-bridged tools via the **literal marker string `"mcp.call"`** (not a scope, not a tool name) — `crates/aivyx-team/src/factory.rs::filter_tools` special-cases this exact string. `capability_scopes` is the separate list that carries the real, qualified `mcp.call:aivyx-coder:*` scope. Do not conflate the two lists.
- `default_nonagon`'s built-in roster (`crates/aivyx-team/src/roster.rs`) already has a member named `"coder"` (an in-process specialist using `fs.read`/`fs.write`/`shell.exec` directly, unrelated to `aivyx-coder`). The new worked example's specialist MUST use a different name — this plan uses `"remote-coder"` — and MUST NOT modify `default_nonagon` or `roster.rs` at all (explicit non-goal in the approved design).
- Recipes are dual-maintained: `docs/MCP_RECIPES.md` (operator-facing doc) and `crates/aivyx-cli/src/bin/aivyx_modules/mcp_recipes.rs`'s `RECIPES` slice (the `aivyx mcp recipes` CLI's data source) — **note the real path is `aivyx-cli`, not `aivyx-channel`; the doc's own "Adding a recipe" section names the latter, which is stale.** Both must be updated in the same commit per that file's own doc comment. Only the module's internal shape tests (`every_recipe_has_*`) are automated; doc/module sync itself is operator review, not a test — there is no `recipes_doc_and_module_stay_in_sync` test in the codebase today despite the module doc comment's claim, so this plan does not rely on one existing.
- Every recipe's `toml_snippet` must contain both a `[[mcp_server]]` block and a `[mcp_server.sandbox]` block — enforced generically for every `RECIPES` entry by `every_recipe_snippet_includes_an_mcp_server_block` / `..._a_sandbox_block`. Include a sandbox block even though, per the design, it's optional defense-in-depth for this particular recipe (say so in the snippet's own comment) — omitting it would fail those two existing tests.
- `aivyx-coder`'s real `[mcp_server]` config shape (verified byte-accurate against `aivyx-coder/README.md` lines 823-871) is:
  ```toml
  [mcp_server]
  max_access_level = "edit"   # "plan" | "edit" | "execute" -- no default, required
  session_ttl_secs = 1800
  max_concurrent_sessions = 8
  max_iterations = 10
  ```
  This lives in `aivyx-coder`'s own `config.toml`, not in `aivyx`'s `aivyx.toml` — the recipe must say so as a prerequisite, not show it inside the `[[mcp_server]]` TOML block (which is `aivyx`-side config).

---

### Task 1: Add the `aivyx-coder` MCP recipe (doc + code, same commit)

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx_modules/mcp_recipes.rs` (add one `Recipe` entry to `RECIPES`, add one dedicated test)
- Modify: `docs/MCP_RECIPES.md` (add one `## aivyx-coder` section + one row in the "Catalog at a glance" table)

**Interfaces:**
- Consumes: the existing `Recipe { name, description, toml_snippet }` struct and `RECIPES: &[Recipe]` slice (`crates/aivyx-cli/src/bin/aivyx_modules/mcp_recipes.rs:32-46,74`). No signature changes.
- Produces: nothing new is consumed by later tasks — Task 2 and Task 3 reference the recipe's content by value (the `[[mcp_server]]` block shown below), not by importing any Rust symbol from this task.

- [ ] **Step 1: Write the failing test**

Add this test to the `#[cfg(test)] mod tests` block in `crates/aivyx-cli/src/bin/aivyx_modules/mcp_recipes.rs`, next to the existing `render_recipe_known_name_returns_snippet` test (same section, "-- render_recipe --" comment block):

```rust
    #[test]
    fn render_recipe_aivyx_coder_returns_snippet() {
        let out = render_recipe("aivyx-coder").expect("aivyx-coder recipe must exist");
        assert!(out.contains("[[mcp_server]]"));
        assert!(out.contains("aivyx-coder"));
        assert!(out.contains("--mcp-server"));
        assert!(out.contains("mcp.call:aivyx-coder:*"));
        assert!(out.ends_with('\n'), "snippet must end with newline");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p aivyx-cli --lib mcp_recipes::tests::render_recipe_aivyx_coder_returns_snippet -- --test-threads=1`
Expected: FAIL — `render_recipe("aivyx-coder")` returns `Err(RecipeError { message: "unknown recipe: \`aivyx-coder\`", .. })`, so `.expect(...)` panics.

- [ ] **Step 3: Add the `Recipe` entry to `RECIPES`**

Append this entry to the end of the `RECIPES` slice in `crates/aivyx-cli/src/bin/aivyx_modules/mcp_recipes.rs`, immediately before the closing `];` (after the `everything` entry, matching the file's own "ordered by expected operator-touch frequency... auxiliary/experimental servers come last" convention — this one is neither official-npm nor a third-party server, so it goes last):

```rust
    Recipe {
        name: "aivyx-coder",
        description:
            "Delegate bounded coding tasks to a local aivyx-coder process over MCP.",
        toml_snippet: r#"# aivyx-coder MCP server -- delegates bounded coding tasks to a
# local `aivyx-coder --mcp-server` process. Unlike every other
# recipe in this catalog, aivyx-coder is not third-party code: it
# ships its own Landlock+seccomp confinement and its own tiered
# access ceiling, so the sandbox block below is optional
# defense-in-depth here, not the thing actually keeping the
# operator safe -- that's the prerequisite below.
#
# Prerequisite: aivyx-coder's own config.toml must set
# `[mcp_server].max_access_level` ("plan" | "edit" | "execute")
# before this server can start -- there is no default, and it
# refuses to start unconfigured. This ceiling caps every session's
# access regardless of what a specialist's model requests; set it
# no higher than the specialists calling it actually need.
#
# Required env: none.
# Capability scopes the agent gets: mcp.call:aivyx-coder:*

[[mcp_server]]
name = "aivyx-coder"
command = "aivyx-coder"
args = ["--mcp-server"]

[mcp_server.sandbox]
# Optional defense-in-depth (see note above) -- bind only what
# aivyx-coder itself needs to start. Omit this block entirely if
# you'd rather rely solely on aivyx-coder's own confinement.
wrapper = "bwrap"
args = [
    "--ro-bind", "/usr", "/usr",
    "--ro-bind", "/etc", "/etc",
    "--ro-bind", "/home/me/.config/aivyx-coder", "/home/me/.config/aivyx-coder",
    "--dev", "/dev", "--proc", "/proc",
    "--",
]
"#,
    },
```

- [ ] **Step 4: Run the test to verify it passes, then run the whole file's suite**

Run: `cargo test -p aivyx-cli --lib mcp_recipes:: -- --test-threads=1`
Expected: PASS, all tests in the module green (the new test plus every pre-existing generic test, since `every_recipe_has_*`/`every_recipe_snippet_includes_*`/`render_listing_*` all iterate `RECIPES` and automatically cover the new entry with no changes needed to them).

- [ ] **Step 5: Add the matching `docs/MCP_RECIPES.md` section**

Add a new row to the "Catalog at a glance" table (`docs/MCP_RECIPES.md`, after the `everything` row):

```markdown
| [`aivyx-coder`](#aivyx-coder) | Low | Delegate coding tasks to a local aivyx-coder process |
```

(`Low` matches the table's existing vocabulary — High/Medium/Low/First-run — same bracket as `memory`/`puppeteer`: a real capability, invoked only when a team specifically delegates coding work, not an everyday-touch server like `filesystem`/`github`.)

Add a new `## aivyx-coder` section at the end of the file, immediately before the `## Adding a recipe to this catalog` section, matching the exact shape every other recipe section uses (description paragraph, `**Required env:**`, `**Capability scopes the agent gets:**`, fenced ```toml block, "Verify it works:" paragraph, trailing `---`):

```markdown
## aivyx-coder

Delegate bounded coding tasks to a local `aivyx-coder` process (a
separate, sibling Aivyx product — a terminal coding agent for local
LLMs) running as `aivyx-coder --mcp-server`. Unlike every other
recipe in this catalog, this is not third-party code: `aivyx-coder`
ships its own Landlock+seccomp confinement and its own tiered
access ceiling, so the sandbox block below is optional
defense-in-depth, not the thing actually keeping the operator safe.

**Prerequisite:** `aivyx-coder`'s own `config.toml` must set
`[mcp_server].max_access_level` (`"plan"` | `"edit"` | `"execute"`)
before this server can start — there is no default, and it refuses
to start unconfigured. This ceiling caps every session's access
regardless of what a specialist's model requests; set it no higher
than the specialists calling it actually need.

**Required env:** none.
**Capability scopes the agent gets:** `mcp.call:aivyx-coder:*`.

```toml
[[mcp_server]]
name = "aivyx-coder"
command = "aivyx-coder"
args = ["--mcp-server"]

[mcp_server.sandbox]
# Optional defense-in-depth (see prerequisite above) -- bind only
# what aivyx-coder itself needs to start. Omit this block entirely
# if you'd rather rely solely on aivyx-coder's own confinement.
wrapper = "bwrap"
args = [
    "--ro-bind", "/usr", "/usr",
    "--ro-bind", "/etc", "/etc",
    "--ro-bind", "/home/me/.config/aivyx-coder", "/home/me/.config/aivyx-coder",
    "--dev", "/dev", "--proc", "/proc",
    "--",
]
```

Verify it works: after configuring `aivyx-coder`'s
`max_access_level` and restarting the daemon (`aivyx daemon stop &&
aivyx`), run `aivyx mcp status` — `aivyx-coder` should show
connected with 2 tools (`code`, `code_reply`). See
`docs/NONAGON.md` §9 for a worked example wiring this into a
Nonagon specialist.

---
```

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-cli/src/bin/aivyx_modules/mcp_recipes.rs docs/MCP_RECIPES.md
git commit -m "docs+data: add aivyx-coder MCP recipe"
```

---

### Task 2: Add a worked "remote-coder" example to `docs/NONAGON.md`

**Files:**
- Modify: `docs/NONAGON.md` (append a second worked example under `## 9. Worked example — the kitchen BOH Nonagon`)

**Interfaces:**
- Consumes: the recipe from Task 1 (references `mcp.call:aivyx-coder:*` and the server name `"aivyx-coder"` established there — must match exactly).
- Consumes: the real `[[team.member]]` TOML shape confirmed against `crates/verticals/aivyx-kitchen/assets/kitchen-boh.toml` (`name`/`role`/`soul`/`tool_allowlist`/`capability_scopes`/`trust_ceiling`) and the real `filter_tools` marker semantics confirmed against `crates/aivyx-team/src/factory.rs:185-202` (the literal string `"mcp.call"` in `tool_allowlist` admits every bridged tool whose required scope base is `mcp.call`; `capability_scopes` carries the actual attenuated `mcp.call:aivyx-coder:*` grant).
- Produces: nothing later tasks import — this is a doc-only addition. Task 3 uses the same TOML shown here as the basis for its scratch verification config, so the exact field values here (server name, member name, scope string) must match what Task 3 actually runs.

- [ ] **Step 1: Draft and insert the second worked example**

In `docs/NONAGON.md`, immediately after the existing kitchen-BOH callout block and before `## 10. What's reused vs. new` (i.e., appended to the end of `## 9`, not as a new numbered section — keeps the existing section numbering stable), insert:

```markdown

A second example: delegating a bounded coding task to `aivyx-coder`
running out-of-process, bridged in as an MCP server (see
`docs/MCP_RECIPES.md`'s `aivyx-coder` recipe). The specialist below
is named `remote-coder` — deliberately distinct from the default
roster's own in-process `coder` role (`crates/aivyx-team/src/roster.rs`,
which touches `fs.*`/`shell.exec` directly) — to keep "runs in this
process" and "delegates to a separate aivyx-coder binary" visually
unambiguous in any team config that uses both:

```toml
[[team.member]]
name = "remote-coder"
role = "Engineering specialist (out-of-process)"
soul = "You delegate coding tasks to a separate aivyx-coder process over MCP rather than touching files yourself. Every call needs an access_level: \"plan\" for read-only investigation, \"edit\" when the task needs a file changed but no commands or git actions, \"execute\" when it needs to run tests, commands, or commit. Pick the lowest tier that gets the task done -- the ceiling aivyx-coder's own operator configured wins regardless of what you request, so asking for more than you need only risks an unnecessary rejection, never gets you more than what's configured."
tool_allowlist = ["mcp.call"]
capability_scopes = ["mcp.call:aivyx-coder:*"]
trust_ceiling = "Trusted"
```

`tool_allowlist` carries the literal marker `"mcp.call"` — not a
tool name, not a scope — which `filter_tools`
(`crates/aivyx-team/src/factory.rs`) expands to every bridged tool
whose required scope base is `mcp.call` (here: `code` and
`code_reply`, `aivyx-coder`'s only two). `capability_scopes` is the
separate list carrying the real, attenuated grant the specialist
actually gets. No `default_nonagon` change: this member is
opt-in, added to a custom `TeamConfig` the same way the kitchen BOH
roster is — never part of the free-core default roster.
```

- [ ] **Step 2: Proofread anchors and rendering**

Confirm the inserted content doesn't break any existing markdown anchor links elsewhere in the doc (the section heading `## 9. Worked example — the kitchen BOH Nonagon` is unchanged, so its `#9-worked-example-...` anchor still resolves) and that the nested triple-backtick fences render correctly (the outer prose is not itself fenced, only the `toml` block is — verify by rendering the file locally or checking indentation matches the file's existing style for embedded code blocks, e.g. the `[[mcp_server]]` blocks in `docs/MCP_RECIPES.md`).

- [ ] **Step 3: Commit**

```bash
git add docs/NONAGON.md
git commit -m "docs: add remote-coder worked example to NONAGON.md"
```

---

### Task 3: Live end-to-end verification

**Files:** none modified — this task produces a verification report, not a code change. If verification fails, fix whatever Task 1/2 content is wrong and re-run this task before proceeding.

**Interfaces:**
- Consumes: `aivyx-coder`'s `--mcp-server` flag and `[mcp_server]` config (`aivyx-coder/README.md` lines 823-871, `aivyx-coder/crates/aivyx-config/src/lib.rs`'s `McpServerSettings`), `aivyx`'s `aivyx team run "<mission>" [--config <path>]` CLI (`crates/aivyx-cli/src/bin/aivyx.rs:1955-1959`, executed via `team::run_mission` at `crates/aivyx-cli/src/bin/aivyx.rs:7802-7814`), and the exact recipe/worked-example content from Task 1 and Task 2.
- Produces: a PASS/FAIL verification note appended to this task's own report — no interface other tasks depend on.

**Note for whoever executes this task:** unlike Task 1/2, this needs a real, already-configured local LLM backend (Ollama/vLLM/llama-server, per `aivyx-coder/README.md`'s "Serving" section and `aivyx`'s own provider config) and two real built binaries running side by side. If you are a subagent without access to the operator's already-running model backend, report `NEEDS_CONTEXT` rather than guessing at credentials or starting a model server yourself — this task is expected to run under the controller's direct supervision on the real rig, matching the design's own "Testing / verification" section.

- [ ] **Step 1: Build both binaries in release mode**

```bash
cd /home/julian/Projects/Rust/aivyx-coder && cargo build --release -p aivyx
cd /home/julian/Projects/Rust/aivyx && cargo build --release -p aivyx-cli
```
Expected: both build clean. Note the resulting binary paths (`aivyx-coder/target/release/aivyx-coder`, `aivyx/target/release/aivyx`).

- [ ] **Step 2: Put `aivyx-coder` on `PATH` and configure its `[mcp_server]` ceiling**

```bash
export PATH="/home/julian/Projects/Rust/aivyx-coder/target/release:$PATH"
mkdir -p ~/.config/aivyx-coder
```

Ensure `~/.config/aivyx-coder/config.toml` contains (add if missing — this file may already exist with other settings from normal `aivyx-coder` use; only the `[mcp_server]` section needs adding):

```toml
[mcp_server]
max_access_level = "edit"
```

- [ ] **Step 3: Create a scratch working directory, `aivyx.toml`, and team pack TOML**

```bash
mkdir -p /tmp/claude-1000/-home-julian-Projects-Rust-aivyx-coder/ae597f07-76af-4bd8-973a-5bf6b76e1d4b/scratchpad/nonagon-live-verify
cd /tmp/claude-1000/-home-julian-Projects-Rust-aivyx-coder/ae597f07-76af-4bd8-973a-5bf6b76e1d4b/scratchpad/nonagon-live-verify
```

Write `aivyx.toml` in this directory with whatever provider/model config the operator's existing setup already uses (copy the relevant `[provider]`/`[model]` section from an already-working `aivyx.toml` — this plan does not specify provider credentials), plus the recipe's `[[mcp_server]]` block from Task 1:

```toml
[[mcp_server]]
name = "aivyx-coder"
command = "aivyx-coder"
args = ["--mcp-server"]
```

Write `team.toml` in the same directory — a minimal pack with a lead plus the `remote-coder` specialist from Task 2:

```toml
[team]
name = "live-verify"
description = "Minimal pack to verify the remote-coder MCP bridge end to end."
lead = "lead"

[[team.member]]
name = "lead"
role = "Coordinator"
soul = "You decompose the goal into one targeted subtask and delegate it to remote-coder, then verify the result and report what actually happened."
tool_allowlist = ["decompose_task", "delegate_task", "query_agent", "verify_output", "synthesize_results"]
capability_scopes = ["team.delegate", "team.message", "mcp.call:aivyx-coder:*"]
trust_ceiling = "Trusted"

[[team.member]]
name = "remote-coder"
role = "Engineering specialist (out-of-process)"
soul = "You delegate coding tasks to a separate aivyx-coder process over MCP rather than touching files yourself. Use access_level \"edit\" for this task."
tool_allowlist = ["mcp.call"]
capability_scopes = ["mcp.call:aivyx-coder:*", "team.message"]
trust_ceiling = "Trusted"
```

**Live-verification finding (not knowable before running this task):**
the lead's `capability_scopes` MUST also carry a matching `mcp.call`
entry, or NT-02 attenuation (`crates/aivyx-team/src/attenuation.rs`'s
`attenuate_for_member`, floored against `lead.declared_capabilities()`
in `crates/aivyx-cli/src/bin/aivyx_modules/team.rs`) silently strips
`remote-coder`'s grant to nothing — the tools still appear in its
registry, every call to them is capability-denied, and the specialist
falls back to describing a shell command instead of running it. This
is why the lead's `capability_scopes` above includes
`mcp.call:aivyx-coder:*`, not just `team.delegate`/`team.message`.
`docs/NONAGON.md` §9's worked example carries the same fix and
explains it in full.

- [ ] **Step 4: Run the mission**

```bash
/home/julian/Projects/Rust/aivyx/target/release/aivyx team run \
  "Delegate to remote-coder: create a file named hello.txt in the current directory containing exactly the text 'hello from aivyx-coder' (no trailing content), using access_level \"edit\"." \
  --config team.toml
```

Expected: the run completes without error; the lead's synthesized output describes a file having been written.

- [ ] **Step 5: Verify against real output**

```bash
cat /tmp/claude-1000/-home-julian-Projects-Rust-aivyx-coder/ae597f07-76af-4bd8-973a-5bf6b76e1d4b/scratchpad/nonagon-live-verify/hello.txt
```
Expected: the file exists and contains `hello from aivyx-coder` — not just a claim in the mission's text output. If it's missing or wrong, this is a real gap in either the recipe (Task 1) or the worked example (Task 2), not a task to mark PASS with a caveat — fix the content and re-run from Step 4.

Also run `aivyx mcp status` (against the same scratch `aivyx.toml`, if the daemon path is exercised) or check the mission's own tool-call trace to confirm the call actually routed through `mcp.call:aivyx-coder:code` (or `code_reply`) — not, e.g., the lead silently doing the work itself with a different tool because the allowlist wiring was wrong.

- [ ] **Step 6: Record the result**

No commit for this task (nothing was modified in either repo). Report PASS with the observed tool-call trace and file contents, or FAIL with the exact error/output — to the controller, for inclusion in the project's closing summary and memory write-up.

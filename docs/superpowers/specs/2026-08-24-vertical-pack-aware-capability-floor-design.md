# Vertical-Pack-Aware Capability Floor Design

## Motivation

The final whole-branch review of the "CLI `team run --config` Capability
Floor" branch (`docs/superpowers/specs/2026-08-24-cli-team-run-capability-floor-design.md`,
branch `worktree-cli-team-run-capability-floor`, currently unmerged at
`32b51351`) found a Critical, empirically-verified regression: clamping a
pack's declared `capability_scopes` against `compute_backcompat_floor`'s
output breaks every real vertical pack. `compute_backcompat_floor` is the
**operator's interactive-session floor** — `memory.*`, `graph.read`,
`skills.*`, `schedule.*`, `reflection.propose`, `persona.propose`, `fs.*`,
`net.*`, `shell.exec`, `workspace`, `ollama.*`, `mcp.call:*`, `app.*`,
`loop.*`, `team.run` — with zero concept of a vertical toolkit's own domain
scopes. Run through the real shipped
`crates/verticals/aivyx-kitchen/assets/kitchen-boh.toml` pack, the reviewer
found every specialist collapsed to `team.message` only and the lead's four
`kitchen.*` scopes stripped — `aivyx team run --config` can no longer
execute a pack's actual domain work, on the exact command
`docs/VERTICAL_PACKS.md:58` documents as standard.

This defect **pre-exists in `bind_lead_scopes` itself**, shipped the same
day in the earlier "bind_lead_scopes Floor-Clamp" piece — the daemon's own
`aivyx team start --config <pack.toml>` path (`assemble_runtime`) has the
identical bug today, independent of this branch. The CLI branch didn't
create it; it exposed it, because the CLI path had zero clamping before (so
kitchen packs happened to work via `aivyx team run --config` despite
already being broken via the daemon's `aivyx team start --config`).

The same review also flagged two Important findings in the same function
and its CLI-side neighbor: `bind_lead_scopes`'s lead branch *widens* a
conservative pack's declared lead scopes to the full floor unconditionally
(never intersects), and `aivyx team roster --config <pack.toml>` displays a
pack's raw declared scopes with no indication they may not match what's
actually granted at runtime. The user chose to resolve all three in this
one design, continuing on the same branch before it merges.

## Root-cause finding: the floor and the filter are both fine on their own — they just never learned about vertical packs

`bind_lead_scopes`'s specialist branch already does the right thing in
principle: `caps = lead_scopes.iter().filter(|s| bases.contains(scope_base(s))).cloned().collect()`
— it filters the **real floor** (the `lead_scopes` parameter) down to the
scope bases the specialist's own declared roster names. The bug is not in
this filtering logic; it's that `lead_scopes` (produced by
`compute_backcompat_floor`) never contains a `kitchen.*`-based scope in the
first place, so the intersection is always empty for any vertical pack.

Two facts make the fix tractable without inventing new machinery:

1. **`backcompat_floor` is one shared value.** Task 1 of the CLI branch
   already extracted its construction into `compute_backcompat_floor` and
   moved the call earlier in `run_async` — the exact same `Vec<Scope>` now
   feeds the interactive agent's own role envelope, the CLI's
   `load_and_clamp_team`, and the daemon's `TeamMissionService` (`lead_scopes:
   backcompat_floor.iter()...` at `aivyx.rs:8876`, confirmed unaffected by
   this branch and reading the same variable). **One fix at this level closes
   all three exposures at once** — the CLI gap this whole initiative targets,
   the pre-existing daemon gap, and (as a bonus, previously undiscovered)
   the same "registered-but-unauthorized" dead-on-arrival failure mode for
   the plain interactive agent, which this file's own comments document
   recurring five times already (Chapter Lattice, Phase 110, Phase 67,
   Chapter Chime, Vitrine) before this piece.
2. **The tool list already knows the answer.** By the point
   `compute_backcompat_floor` runs, `tool_list` is fully populated —
   built-in tools, MCP-discovered tools, and tool-process-proxied tools
   (kitchen, applications, and any future vertical toolkit spawned via
   `[[tool_process]]`) alike — because `run_mission` (and the interactive
   agent, and the daemon) all consume this same, already-finished list.
   Every tool exposes `required_scope(&json!({}))` (already used
   side-effect-free elsewhere in this file for the `ToolDescriptor`
   snapshot), and tool-process proxies already carry the *effective* scope
   — the operator's own `scope_overrides`, if any, already applied. The
   floor's domain grants can be **derived** from what's actually registered,
   instead of hand-enumerated per integration.

## Architecture

### 1. Generalize `compute_backcompat_floor`'s tool-derived grants

Replace the three hand-written special cases — the `ollama_configured`
conditional, the `mcp_server_names` loop, and the `config_tool_processes`
"applications" name check — with one generic sweep over `tool_list`'s own
registered tools. The `ollama_configured: bool`, `mcp_server_names:
&[String]`, and `config_tool_processes: &[aivyx_config::ToolProcessConfig]`
parameters are replaced by one new parameter, `tool_scope_bases: &[Scope]`
(the caller passes `tool_list.iter().map(|t|
t.required_scope(&serde_json::json!({}))).collect::<Vec<Scope>>()` — the
same snapshot pattern already used for `ToolDescriptor`, reused rather than
duplicated).

Inside `compute_backcompat_floor`, for each scope in `tool_scope_bases`:

- If its base is `mcp.call` and it's qualified (contains `:`), extract the
  server segment and grant `mcp.call:<server>:*` (deduplicated — one grant
  per unique server, reproducing today's exact per-server-wildcard
  semantics, just sourced from the tool list instead of `mcp_bridges`
  directly).
- Otherwise, if its base is **not** one of `fs.read`, `fs.write`,
  `fs.metadata`, `fs.delete`, `shell.exec`, `net.fetch`, `net.post`,
  `workspace` (the explicit, path-qualified grants that stay exactly as
  they are — granting their bare base would be a real security
  regression, not a no-op: an unqualified `fs.read` grants every file on
  disk, per D4 Rule 2), grant `Scope::parse(base)`.

This reproduces `ollama.list/show/pull` and `app.read/control/input`
exactly as before (since those tools are already in `tool_list`, gated on
the same conditions that used to gate the hand-written pushes), and now
also produces `kitchen.read`/`kitchen.write`/`kitchen.order.send`/
`kitchen.haccp.log` whenever the kitchen tool-process is configured — with
zero toolkit-specific code, so any future vertical pack's tool-process
gets this for free.

**Boundary, stated explicitly so it isn't re-litigated later:** this
generalization does not touch `fs.*`/`shell.exec`/`net.*`/`workspace` —
those keep their existing explicit, path-rooted parameters unchanged. The
exclusion list above is the enforcement point; a future edit that adds a
new *built-in* fs-adjacent tool must not accidentally let its bare base
leak through this sweep.

### 2. Unify `bind_lead_scopes`'s lead and specialist branches

Factor the specialist branch's own logic — compute `bases` from the
member's declared `capability_scopes` (plus the `workspace.`-tool-allowlist
special case), filter `lead_scopes` down to those bases, union in
`filter_orchestration_markers`'s `kept` — into one path applied to **every**
member, lead included. The lead no longer receives `lead_scopes.to_vec()`
unconditionally; it receives exactly what its own declared bases unlock
from the floor, same as a specialist.

This is safe without weakening specialist grants: specialists are already
filtered against `lead_scopes` (the function's own parameter, the real
floor) directly, never against the lead's post-processed
`capability_scopes` field — confirmed by reading the current specialist
branch, which never reads `m.capability_scopes` for the lead member at all.
Narrowing the lead's own field cannot change what a specialist receives.

The clamp-exceeds-floor warning (`scopes_exceeding_floor` +
`eprintln!`) generalizes to fire for **any** member whose declared,
non-marker scopes are genuinely absent from the floor — not just the lead
— closing the final review's own observation that the specialist branch
silently discarded its own `dropped` scopes with no operator-visible
signal.

### 3. `run_roster --config`: an honest caveat, not a full recomputation

`run_roster` dispatches before `run_async`'s provider/sandbox/tool_process
machinery exists at all — computing the real floor there would mean
spawning tool processes and connecting MCP servers just to render a
preview, breaking its "offline, no side effects" contract stated in its own
doc comment. Instead, `render_roster`'s output gains one line making clear
the printed `capability_scopes` are the pack's own **declared** ask, not a
runtime guarantee: actual grants are computed by `bind_lead_scopes` at
`run`/`start` time against the operator's real, configured authority.

## Testing

- `compute_backcompat_floor`'s two existing pinning tests (added by the
  CLI branch's own Task 1) are updated to pass a `tool_scope_bases` fixture
  instead of the three removed parameters, including at least one
  `mcp.call:<server>:<tool>`-shaped entry (proving the wildcard-not-bare
  resolution) and one `fs.read:...`-shaped entry (proving the exclusion
  list actually excludes it, not just happens not to collide).
- New test: a pack's lead declaring only a conservative set of scopes
  (e.g. `["team.delegate"]`) keeps exactly that plus orchestration markers
  after `bind_lead_scopes` — not the full floor. Mutation-proof: fails
  against the current unfixed lead branch.
- New test, using a synthetic fixture shaped like `kitchen-boh.toml`'s real
  roster (not the file itself, to keep the unit test hermetic and not
  filesystem-dependent): given a floor that includes the four `kitchen.*`
  bases (as the generalized `compute_backcompat_floor` now produces for a
  configured kitchen tool-process), every specialist retains its domain
  scopes instead of collapsing to `team.message`. This is the direct
  mutation-proof for the Critical finding — it must fail against the
  current, unfixed `bind_lead_scopes`.
- New test: a member whose declared scopes exceed the floor (not just the
  lead) triggers the clamp warning.
- `render_roster`'s new caveat line gets a rendering test.
- All of Task 1 and Task 2's existing tests from the CLI branch (563 in
  `aivyx-cli`, `bind_lead_scopes`'s own 4 in `aivyx-channel`, plus the
  branch's own mutation-proof `load_and_clamp_team_strips_a_lead_scope...`)
  must keep passing — this design changes `compute_backcompat_floor`'s
  signature and `bind_lead_scopes`'s body, both already covered.

## Out of scope

- No change to `TeamConfig::load`'s parsing/validation.
- No change to the fs/shell/net/workspace grant mechanism itself — only
  the exclusion boundary around the new generic sweep, stated above.
- No attempt to make `run_roster` compute the real floor — the caveat is
  the deliberately-chosen, lower-cost fix given its offline contract.
- No broader redesign of the capability-scope system beyond what's needed
  to close these three findings.

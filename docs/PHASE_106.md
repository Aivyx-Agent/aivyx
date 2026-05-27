# Phase 106 — MCP Server Breadth (Curated Recipes)

The second item of Chapter D — Substrate Breadth. Phase 105
closed the trajectory-logging gap; Phase 106 closes the MCP-
server-breadth gap on the recipes side.

Phase 46 shipped the first bundled MCP server
(`aivyx mcp-server web-search`); Phase 24 shipped the
external `[[mcp_server]]` TOML surface that lets the operator
plug in any MCP-compatible binary. What's missing today is a
**curated catalog**: an operator who knows MCP exists has
nowhere to look for "which MCP servers should I actually
enable, and what do their `[[mcp_server]]` blocks look like
with the right sandbox config?" `examples/aivyx.toml` shows
two commented snippets (`github`, `filesystem`); a dozen more
official servers exist that an operator coming from Hermes
expects to find documented.

Phase 106 ships two pieces — a doc and a CLI surface — both
purely additive:

- **`docs/MCP_RECIPES.md`** — a single reference document
  catalogging ~10–12 well-supported MCP servers (the official
  `@modelcontextprotocol/server-*` family, mostly), each with
  a paste-able `[[mcp_server]]` block, an inline
  `[mcp_server.sandbox]` block per Q3a (the Phase 55
  sandbox-layer story is load-bearing for any server that
  reads files or hits the network), required env vars, and
  capability-scope notes for the resulting
  `mcp.call:<server>:<tool>` qualifiers.
- **`aivyx mcp recipes [<name>]`** — a new CLI subcommand per
  Q2a. Bare form lists every recipe name with a one-line
  description so the operator can discover what's documented
  without opening the doc; `aivyx mcp recipes <name>` prints
  the worked snippet for a specific recipe so the operator
  can pipe it into `aivyx.toml`. Mirrors Phase 103's
  `aivyx tool init` pattern (CLI surface that lives next to
  a markdown reference).

## Why this, why now

- **It's the second-lowest-risk Chapter D item.** Recipes are
  documentation; the CLI subcommand emits embedded strings.
  Zero substrate change; the existing `[[mcp_server]]`
  loader, the existing Phase 55 sandbox layer, and the
  existing capability machinery all do the actual work — the
  recipes just point at them.
- **It closes the Hermes-comparison gap on MCP breadth.**
  Hermes ships an `optional-mcps/` directory of curated
  integrations; Aivyx's substrate is just as capable, but
  the discovery story is "go find an MCP server, write your
  own TOML." Phase 106 makes the discovery surface match.
- **It builds on the Phase 55 sandbox layer.** Every recipe
  shows the sandbox block alongside the server block by
  default (Q3a), so an operator who copies a recipe gets a
  sandboxed config out of the gate — the substrate-default
  posture, not an afterthought.
- **It pairs cleanly with the held Phase 61 publication.**
  When `v0.1.0` ships, the recipes doc is one of the
  highest-leverage things to point new operators at — the
  difference between "Aivyx supports MCP" and "here are
  twelve specific things Aivyx can do today, copy-paste this
  block."
- **It does not commit to substrate growth.** Phase 106 ships
  zero new bundled-server code paths. A later phase can ship
  one or two new bundled servers (the original Chapter D
  framing left room for both halves); the recipes-first
  posture means the bundled-server half is operator-feedback-
  gated rather than speculative.

## Streak predictions

- **DESIGN.md** — **Will hold.** A new doc + a new CLI
  subcommand emitting embedded strings touches no locked
  technical-contract decision and adds no daemon-IPC
  variant. Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to fifty-three**.

- **PRODUCT.md** — **Will hold.** No P-* commitment touched;
  MCP recipes are third-party-tool ergonomics on top of the
  existing P11 SDK + Phase 24 / 32 / 55 MCP substrate.
  Hash at entry:
  `9f0a515c9076544866aa955d6835763ee15beb0d009b4c335f5710b6d9ba61d3`.
  Prediction: streak **extends to six** (was 5).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold.** All of Phase 106 lives in `aivyx-channel`'s binary
  (new `CliMode::Mcp` variant + parse block + new
  `aivyx_modules/mcp_recipes.rs` with embedded recipe data
  and render functions). `aivyx-core` is untouched.
  Hash at entry:
  `ab3f9730c692917023239bbdd7c375497459e2a7fb3bbf08c007b5c945c6210d`.
  Prediction: streak **extends to six** (was 5).

- **New workspace deps** — Zero. The recipes module uses
  `serde_json` for the `[mcp_server.sandbox]` JSON args
  arrays the recipes embed; both `serde` and `serde_json` are
  already workspace deps. Rendering recipe text is
  string-concatenation against `const &str` literals.

- **Test count** — Positive. New tests cover: the recipes
  registry shape (every recipe has a non-empty name +
  description + snippet), `render_recipe(name)` lookup
  (known recipe round-trips, unknown errors with a candidate
  list), and CLI parse paths. Rough prediction: **+8 to +12**.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_106.md` + `docs/ROADMAP.md` Chapter D entry
refinement (flip Phase 106 row from scheduled to Active and
add a per-phase `## Phase 106` section) + `docs/README.md`
status row.

### Task 2 — `aivyx mcp recipes` subcommand + recipes module

- **CLI shape.** New `CliMode::Mcp(McpSubcommand)` with one
  sub-subcommand `Recipes { name: Option<String> }`. The
  pre-existing `aivyx mcp-server <name>` subcommand
  (Phase 46) stays untouched — `mcp-server` is the *runner*,
  `mcp recipes` is the *catalog*. Two different surfaces
  with adjacent names; the namespacing is the same logic
  Phase 103 used to add `aivyx tool init` next to the
  existing `aivyx tools` introspection (Phase 102).
- **`aivyx_modules/mcp_recipes.rs`.** Embedded registry as a
  `&'static [Recipe]` slice; each `Recipe` carries `name`,
  `description`, `toml_snippet`, optional `env_vars` notes,
  and optional `notes` for capability-scope guidance. Two
  pure functions: `list_recipes() -> &'static [Recipe]`
  (returns the slice) and
  `render_recipe(name: &str) -> Result<String, RecipeError>`
  (looks up by name, emits the worked snippet to a `String`
  the dispatch printer can write to stdout).
- **Dispatch.** Bare `aivyx mcp recipes` prints each
  recipe's `name <padding> description` on its own line.
  `aivyx mcp recipes <name>` prints the snippet for that
  recipe. Unknown name errors with the candidate list (same
  shape as Phase 13's `UnknownRole` error).

### Task 3 — `docs/MCP_RECIPES.md` + tests + INSTALL.md + exit

- **`docs/MCP_RECIPES.md`.** The canonical reference for the
  curated catalog. Each recipe section carries the
  `[[mcp_server]]` block, the `[mcp_server.sandbox]` block
  by default per Q3a, required env vars, capability-scope
  notes, and a "verify it works" line. Recipes ordered by
  expected operator-touch frequency (filesystem first,
  experimental servers last).
- **Recipe coverage.** Initial set: `filesystem`, `github`,
  `gitlab`, `sqlite`, `postgres`, `time`, `fetch`,
  `brave-search`, `slack`, `memory`, `puppeteer`,
  `everything`. The official `@modelcontextprotocol/server-*`
  family covers the first eight; `puppeteer` and `everything`
  also live in the official org. The exact set may shift one
  or two slots at implementation time as I check each
  server's current state — the doc is a living one.
- **Tests.** Three unit-test clusters:
  - `recipes_registry_is_non_empty`,
    `every_recipe_has_a_unique_name`,
    `every_recipe_has_a_non_empty_snippet`.
  - `render_recipe_known_name` (round-trips a sample
    snippet), `render_recipe_unknown_name_errors_with_candidates`.
  - CLI parse: `aivyx mcp recipes` parses; `aivyx mcp
    recipes filesystem` parses; bare `aivyx mcp` errors
    with a usable hint; unknown subcommand errors.
- **`docs/INSTALL.md`.** First-run checklist gains a line
  pointing at `docs/MCP_RECIPES.md` next to the existing
  Phase 46 web-search note.
- **Exit.** ROADMAP frozen entry, docs/README status flip,
  prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Scope:** (a) **Recipes-only.** Phase 106 ships
  zero new bundled-server code paths. The recipes catalog
  is the operator-visible surface change; new in-tree
  bundled servers stay a deliberate deferral pending
  operator pressure. Chosen over recipes + 1 (would put
  substrate-design work into Phase 106 — refactoring
  `run_mcp_server` for a generic dispatch — that the
  easy-wins-first chapter ordering argues against) and
  recipes + 2 (same argument, larger).
- **Q2 — Surface:** (a) **Doc + `aivyx mcp recipes`
  subcommand.** The canonical reference is
  `docs/MCP_RECIPES.md`; the CLI is the discovery surface
  for operators who haven't opened the doc yet. Bare
  `aivyx mcp recipes` lists every recipe; `aivyx mcp
  recipes <name>` prints the worked snippet. Mirrors
  Phase 103's `aivyx tool init` (CLI lives next to a
  markdown reference). Chosen over doc-only (loses
  in-shell discoverability) and over a wizard interactive
  multi-select (highest UX cost; the wizard would need a
  new primitive — the small recipes set today doesn't
  warrant it).
- **Q3 — Sandbox guidance:** (a) **Sandbox snippet inline
  per recipe.** Every recipe shows a
  `[mcp_server.sandbox]` block alongside the
  `[[mcp_server]]` block by default. Encodes the Phase 55
  "sandbox by default" posture; copy-paste produces a
  sandboxed config out of the gate. Chosen over
  show-only-when-needed (operators forget the recipes that
  need it) and over pointer-to-sandbox-docs (copy-paste
  produces an unsandboxed config until the operator
  follows the link).

## Exit criteria

- [ ] `docs/PHASE_106.md` + ROADMAP Chapter D Phase 106
  entry flip + docs/README status row — Task 1 (this
  commit).
- [ ] `aivyx mcp recipes [<name>]` subcommand wired through
  `CliMode::Mcp(McpSubcommand::Recipes)` — Task 2.
- [ ] `aivyx_modules/mcp_recipes.rs` module with embedded
  recipe registry + `list_recipes` + `render_recipe`
  pure helpers — Task 2.
- [ ] Recipes-registry shape tests (non-empty, unique
  names, non-empty snippets) — Task 3.
- [ ] `render_recipe` lookup tests (known + unknown with
  candidate list) — Task 3.
- [ ] CLI parse tests (`mcp recipes`, `mcp recipes <name>`,
  bare `mcp` error, unknown subcommand error) — Task 3.
- [ ] `docs/MCP_RECIPES.md` (new) with 10–12 worked
  recipes, each with sandbox block + env-var notes +
  capability scope notes — Task 3.
- [ ] `docs/INSTALL.md` mention — Task 3.
- [ ] ROADMAP + docs/README refreshed at exit — Task 3.
- [ ] All three Q-block questions resolved with operator
  sign-off pre-Task 2 (recorded above).
- [ ] DESIGN.md streak extends to fifty-three.
- [ ] PRODUCT.md streak extends to six.
- [ ] Production-core `lib.rs` streak extends to six.
- [ ] Zero new workspace dependencies.
- [ ] Test count delta positive — predicted `+8` to `+12`.
- [ ] Zero clippy warnings.

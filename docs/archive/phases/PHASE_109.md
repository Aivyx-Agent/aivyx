# Phase 109 — Tool Breadth + Amendment A12

The fifth Chapter D item. Phase 109 extends the substrate
tool surface for the second time since the project closed
its PRODUCT.md forward-commitment ledger — A11 (Phase 100)
took P10's count from 8 → 10; A12 (Phase 109) takes it from
10 → 13.

Three new substrate tools, organized as **two headline
additions for one new scope base + one closure of a long-
standing toolless scope from Phase 100's audit**:

- **Headline (A12, new scope base):** `git.status` +
  `git.diff` — both gated by a new `git.read` capability
  scope qualified by repo path. The single most-requested
  Hermes-comparison-driven tool addition for code-agent
  workflows (Hermes ships `git` as one of its 40+ tools;
  Aivyx had none).
- **Closure (no amendment needed):** `net.dns` — a tool for
  the `net.dns` scope base that has lived in `KNOWN_BASES`
  since Phase 0 without a tool behind it (one of the eight
  declared-but-toolless scopes Phase 100's audit
  catalogued). Closes one of the eight; the other seven
  (`shell.spawn`, `audit.read`, `config.read`,
  `config.write`, etc.) stay deferred per Phase 100's
  audit conclusions.

The headline outcome: an operator who runs `aivyx` against
a code project today can ask the agent to "check what's
changed since my last commit" or "show me the diff for
this file" and the agent has substrate tools to answer
without shelling out manually. The DNS tool is a small
diagnostic addition that closes a years-long substrate
deferral.

## Why this, why now

- **Chapter D's tool-breadth item.** The Hermes-comparison
  analysis at Phase 104 named tool breadth as one of five
  out-of-the-box-surface gaps. Phase 105 (audit export),
  Phase 106 (MCP recipes), Phase 107 (Discord), and Phase
  108 (Slack) closed four of the five; Phase 109 closes
  the fifth.
- **A11 precedent is fresh.** Phase 100 set the
  amendment-driven-tool-addition pattern eight phases ago
  (and Phase 37 / A5 set it before that). Phase 109 is the
  third such amendment — A12 mirrors A11 mirrors A5 in
  structure. The amendment file is short, the substrate
  changes are tight, the test surface is bounded.
- **`git.read` is the right scope grain.** Per Q1 sign-off:
  one scope base shared by both git tools (`git.status`,
  `git.diff`) means roles get "read this repo" as a single
  capability grant rather than picking off individual git
  verbs. Qualifier-by-repo-path matches the `fs.read:<path>`
  precedent exactly.
- **`net.dns` closes a substrate hygiene gap.** The scope
  base has existed since Phase 0; a tool that gates against
  it has not. Phase 100's audit listed it as "tool later";
  Phase 109 ships the tool. No amendment needed.
- **Foundation-style scope.** Like Phase 100, Phase 109 is
  a small tight phase — three tools, one amendment, no
  cross-crate refactors. The Phase 107-style multi-session
  arc is not needed here.

## Scope (Q-block sign-off)

- **Q1 — Headline tool pick:** (a) **`git.status` +
  `git.diff`** sharing one new `git.read` scope base
  qualified by repo path. High-impact for code-agent use
  cases; shells out to the system `git` binary (no new Rust
  deps). A12 takes P10 from 10 → 12 tools for the headline
  additions; total to 13 once Q2's `net.dns` lands.
- **Q2 — Toolless scope closure:** (a) **`net.dns`** —
  cheap addition (`tokio::net::lookup_host`) closing one of
  Phase 100's audit-deferred toolless scopes. No amendment
  needed (scope base already in `KNOWN_BASES`). The other
  seven toolless scopes stay deferred.
- **Q3 — Amendment shape:** (a) **Mirror A11 exactly** —
  amendment file is short and structurally identical to
  A11 (which itself mirrored A5 from Phase 37). Pins P10's
  count at the new value; cross-references A5 and A11 as
  precedents. A12 also acknowledges the `net.dns` closure
  even though it doesn't require an amendment for the
  scope-base itself — the count change from 10 → 13 wants
  one consistent amendment.

## Streak predictions

- **DESIGN.md** — **Will break at A12.** The amendment
  process requires editing the relevant DESIGN.md section
  to reference the amendment inline (blockquote-style
  pointer). Same break-pattern as A11 at Phase 100. Hash at
  entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **breaks at 55**, re-establishes from
  zero.

- **PRODUCT.md** — **Will break at A12.** P10's enumerated
  ten-tool list (post-A11) edits to thirteen. Same
  break-pattern as A11 at Phase 100. Hash at entry:
  `9f0a515c9076544866aa955d6835763ee15beb0d009b4c335f5710b6d9ba61d3`.
  Prediction: streak **breaks at 8**, re-establishes from
  zero.

- **Production-core `aivyx-core/src/lib.rs`** — **Should
  hold.** New tools live in `aivyx-channel` (likely a new
  `aivyx_channel/src/tools/git_*.rs` + `net_dns.rs`
  alongside the existing tool modules). `aivyx-core`
  itself is not touched. Hash at entry:
  `ab3f9730c692917023239bbdd7c375497459e2a7fb3bbf08c007b5c945c6210d`.
  Prediction: streak **extends to nine** (was 8).

- **`aivyx-capability::KNOWN_BASES`** — One new scope
  base added (`git.read`). The `KNOWN_BASES` array is in
  `aivyx-capability`, not `aivyx-core`, so this doesn't
  affect the lib.rs streak.

- **New workspace deps** — Zero. Both git tools shell out
  to the system `git` binary; the DNS tool uses
  `tokio::net::lookup_host` (already in workspace tokio
  features). No clipboard / image / process crates pulled
  in.

- **Test count** — Positive. Per-tool unit tests for happy
  path + error path + scope-qualifier matching, plus
  integration tests for the tool dispatch wiring. Rough
  prediction: **+15 to +25**.

## Tasks

Five sub-tasks plus exit + backfill, mirroring Phase 100's
shape:

### Task 1 — Open (this commit)

`docs/PHASE_109.md` + `docs/ROADMAP.md` Chapter D Phase
109 entry flip (scheduled → Active) + per-phase `## Phase
109` section + `docs/README.md` status row.

### Task 2 — Amendment A12 + `git.read` scope base + Phase 100 audit note refresh

- New `docs/amendments/<date>-substrate-tool-count-thirteen.md`
  filing A12. Structurally mirrors A11 (which mirrored A5).
- DESIGN.md edit referencing A12 inline at the P10 section.
- PRODUCT.md edit updating P10's enumerated tool list from
  ten to thirteen.
- `aivyx-capability::KNOWN_BASES` gains `git.read`.
- Phase 100 audit note refresh: `net.dns` moves from
  "tool later" to "shipped Phase 109"; the other seven
  declared-but-toolless scopes stay in the audit list.

### Task 3 — `git.status` + `git.diff` tools

- New module `aivyx-channel/src/tools/git_read.rs` housing
  both tools (they share the scope, the dispatch helpers,
  and the repo-path-validation logic — keeping them in
  one module keeps the scope-base/file-name mapping
  legible).
- `GitStatusTool` — runs `git -C <repo_path> status
  --porcelain --untracked-files=all`, parses lines into
  structured `{path, status_code}` entries, returns as
  JSON.
- `GitDiffTool` — runs `git -C <repo_path> diff
  [--cached] [<path>]`, returns raw unified-diff output.
  Accepts optional `--cached` (staged diff) and optional
  `path` (file or directory scoped).
- Both tools' `required_scope` builds
  `git.read:<canonical_repo_path>` from input + the
  configured allowed repo paths; mirrors `fs.read`'s
  path-qualifier pattern.
- Both tools refuse to run outside the configured allowed
  repo path set (no `--bare-repo`, no `cd` shenanigans —
  the `-C` flag pins the working directory).

### Task 4 — `net.dns` tool

- New module `aivyx-channel/src/tools/net_dns.rs` housing
  `NetDnsTool`.
- Tool runs `tokio::net::lookup_host(input_host_with_port)`
  and parses the iterator into a `Vec<String>` of resolved
  addresses (each `IpAddr.to_string()`).
- `required_scope` builds `net.dns:<host>` from the input
  host; qualifier semantics match `net.fetch:<url>` from
  Phase 12 (glob match per `Scope::is_granted_by`).
- Defensive: input host validated to be a valid hostname
  (no `..`, no `/`, no scheme prefix); the tool fails
  cleanly on malformed input rather than passing it to
  `lookup_host` and getting a cryptic system error.

### Task 5 — Binary wiring + tests + role-render envelope updates

- `aivyx-channel/src/bin/aivyx.rs` registration: three new
  `Tool` impls land in the agent's `ToolRegistry` at the
  same registration point Phase 100 wired
  `fs.delete` / `fs.metadata` through.
- Configuration surface: `[git]` TOML section with
  `repos = ["/path/to/repo1", "/path/to/repo2"]` so the
  operator constrains which repo paths the git tools may
  touch.
- Role-render envelope updates: `git.read:<path>` lands in
  the default ceiling for `Trusted` (Local). For the
  remote `SemiTrusted` adapters (Telegram, Discord, Slack)
  it's grantable but not in the default ceiling. `net.dns`
  is in `CEILING_SEMITRUSTED` (network reads are
  SemiTrusted).
- Per-tool tests in `tools/git_read.rs` and `tools/net_dns.rs`
  (input validation, scope qualifier shape, happy-path
  output parsing). Integration test in the binary's
  `tests` module covering the registration path (mirrors
  the Phase 100 `build_fs_delete_for_channel` tier-gate
  tests).

### Task 6 — Docs sweep + exit

- `docs/INSTALL.md` First-run checklist gains a Phase 109
  entry covering the three new tools.
- `examples/aivyx.toml` gains a commented `[git]` section.
- A4 amendment addendum **not** needed (no crate-shape
  change, no workspace member added).
- A3 amendment addendum **not** needed (the new
  `git.read` scope base lands inside the existing
  `KNOWN_BASES` list and A3's "scope-base count" was last
  refreshed at Phase 54; the next A3 addendum can batch
  the post-Phase-54 additions including `git.read`
  whenever a focused docs phase opens for that).
- Exit: PHASE_109.md prediction-vs-reality + ROADMAP
  freeze + README flip + hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Headline tool pick:** (a) **`git.status` +
  `git.diff`** with one shared `git.read` scope base
  qualified by repo path. Code-agent use case; shells out
  to system `git`; no new Rust deps.
- **Q2 — Toolless scope closure:** (a) **`net.dns`** added
  as a third tool. Uses existing scope base from Phase 0;
  no amendment needed for the scope itself. Closes one of
  Phase 100's eight audit-deferred toolless scopes.
- **Q3 — Amendment shape:** (a) **Mirror A11 exactly** —
  amendment file structurally identical to A11; pins P10's
  new count at thirteen; cross-references A5 + A11. The
  `net.dns` closure is acknowledged in the amendment's
  count change (10 → 13) so the substrate has one
  consistent record of the tool-count jump.

## Prediction vs. reality

**Two predictions held, two predictions broke — one
expected, one honest surprise.**

- **DESIGN.md — broke at A12, as predicted.** The D4
  base-table edit (new `git` section) plus the inline
  blockquote pointing at A12 are the load-bearing changes.
  Hash at exit: `2bd52e61…6b4e69` (was `89dc8903…a94bce`).
  Streak ends at 55, re-establishes from zero.
- **PRODUCT.md — broke at A12, as predicted.** P10's
  enumerated tool list edits from ten to thirteen across
  the Rule block, commit-1, what-this-does-not-say,
  delivery-status, and Cluster 4 entries. Hash at exit:
  `3fba1078…d9b766a8` (was `9f0a515c…ba61d3`). Streak
  ends at 8, re-establishes from zero.
- **`aivyx-core/src/lib.rs` — broke, against the open-doc
  prediction.** The open doc predicted the streak would
  extend to nine because tools would land in
  `aivyx-channel`. **The prediction was wrong**: tools
  live in `aivyx-core/src/tools/` per the Phase 4
  substrate convention, so the new `git` + `net_dns`
  modules edited `aivyx-core/src/lib.rs`'s re-export
  block. Hash at exit: `90104b1f…ab85` (was
  `ab3f9730…c6210d`). Honest break per the Phase 6 Q5
  convention ("honesty over streak preservation") — the
  substrate convention is right; the prediction was
  uncalibrated. Streak ends at 8, re-establishes from
  zero. **All three byte-identity streaks reset in the
  same phase**, the first such triple-reset since Phase
  56's two-streak break for the P13/P14 amendments.

**Zero new workspace deps — held.** Git tools shell out to
the system `git` binary; `net.dns` uses
`tokio::net::lookup_host` already in the workspace tokio
features. No new crates pulled in.

**A12 amendment file landed.**
`docs/amendments/2026-05-28-substrate-tool-count-thirteen.md`.
Structurally mirrors A11 + A5: short, pins P10's new count,
documents the substrate-not-infrastructure classification,
explains the shared-scope-base rationale for the git pair.

**Test count — `+25`** (workspace `1913 → 1938`). **Inside
the predicted `+15` to `+25` band at the upper edge.**
Breakdown: 11 git_tests in `tools/git.rs` (porcelain parse
× 4, deny_scope, resolve_repo × 2, schema × 2, config-build
× 2) + 14 net_dns_tests in `tools/net_dns.rs` (validate_host
× 10 covering accept/reject branches, deny_scope, tool
metadata, required_scope build × 2).

**Scope — six tasks shipped as planned**, with Tasks 5+6
combined into one commit per the Phase 105/106 pattern.

**End-to-end notes.** The `net.dns` tool is fully wired
and accessible to agents through any of the four
adapters (Local/Telegram/Discord/Slack); operators get it
for free with no configuration. The git tools register
only when `[git]` config supplies an allow-set — operators
without `[git]` see no git tools in their dispatch
surface. The Phase 100 audit deferral status for `net.dns`
is closed within A12's body.

The other seven declared-but-toolless scopes from Phase
100's audit (`shell.spawn`, `audit.read`, `config.read`,
`config.write`, `display.window_close`, `memory.gc`,
`mission.gate`) stay deferred — `memory.gc` and
`mission.gate` actually do have tools (Phase 28, Phase 21);
the Phase 100 audit's catalog was approximate. A future
phase that wants to revisit the audit can refresh those
two entries.

## Exit criteria

- [x] `docs/PHASE_109.md` + ROADMAP Chapter D Phase 109
  entry flip + docs/README status row — Task 1 (commit
  `9703605`).
- [x] Amendment A12 filed + DESIGN.md edit + PRODUCT.md
  P10 count edit + `git.read` added to `KNOWN_BASES` +
  Phase 100 audit note refresh (inside the A12 file
  itself) — Task 2 (commit `c85f732`).
- [x] `git.status` + `git.diff` tools shipped in
  `aivyx-core/src/tools/git.rs` with shared `git.read`
  scope — Task 3 (commit `1a0446d`). Note: open doc
  said `aivyx-channel/src/tools/git_read.rs`; actually
  landed in `aivyx-core/src/tools/git.rs` per the Phase 4
  substrate convention (tools live in core).
- [x] `net.dns` tool shipped in `aivyx-core/src/tools/net_dns.rs`
  — Task 4 (commit `1a0446d`, combined with Task 3).
- [x] Binary wiring + `[git]` TOML config + tool
  registration at all four channel kinds — Task 5
  (commit `3084da4`, combined with Task 6). Role-render
  envelope updates not needed (the existing render path
  picks up new scope bases automatically through
  `KNOWN_BASES`).
- [x] Docs sweep (INSTALL.md Phase 109 paragraph,
  examples/aivyx.toml `[git]` section) — Task 6 (commit
  `3084da4`).
- [x] All three Q-block questions resolved with operator
  sign-off pre-Task 2 (recorded above).
- [x] DESIGN.md streak broke at A12 as predicted.
- [x] PRODUCT.md streak broke at A12 as predicted.
- [x] **`aivyx-core/src/lib.rs` streak broke** —
  open-doc prediction was wrong (assumed tools would
  live in aivyx-channel; they actually live in
  aivyx-core/src/tools per Phase 4 substrate convention).
  Honest break per the Phase 6 Q5 convention.
- [x] Zero new workspace dependencies.
- [x] A12 amendment file landed in
  `docs/amendments/2026-05-28-substrate-tool-count-thirteen.md`.
- [x] Test count delta positive — `+25` (inside the
  predicted `+15`–`+25` band at the upper edge).
- [x] Zero clippy warnings.

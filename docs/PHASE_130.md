# Phase 130 — Notion + Obsidian Knowledge-Management Bundle (Chapter F #5)

**Largest Chapter F phase yet.** Operator-picked at the
Phase 130 direction question: bundle Notion (Chapter F
#5) and Obsidian (Chapter F #6) into one phase under a
"knowledge management" theme rather than sequencing them
across two phases. Two new binaries, two distinct
substrate patterns (REST API + token vs local
filesystem + markdown), thirteen tools total.

## Chapter F context

Chapter F's pattern locked at Phase 123 (Gmail) and held
through three Google integrations
(Gmail → Calendar → Drive). Phase 130 is the chapter's
first non-Google + first non-OAuth phase, proving the
substrate generalizes beyond Google's OAuth dialect.

- **Notion** is a REST API service with **Integration
  token (API key) auth** — simpler than OAuth (no
  callback flow, no token refresh). Operators create
  an "internal integration" in Notion's settings, paste
  the token into config. **Notion-specific UX quirk:**
  operators must explicitly share each page/database
  with the integration in Notion's UI before the
  integration can see them. INSTALL.md documents this
  clearly.

- **Obsidian** is a **local Markdown vault** — no REST
  API. Integration is filesystem operations against
  the operator's vault directory (e.g.,
  `~/Documents/MyVault/`). Substrate-wise it's closer
  to `fs.*` substrate tools than to the Google
  integrations, but with markdown-aware operations
  (frontmatter, `[[wikilinks]]`, `#tags`). Path-
  traversal protection (no `..` escape) is load-bearing.

Two crates ship under the same Chapter F binary-per-
service pattern:

- `aivyx-notion` — REST API client; Notion's `api.notion.com/v1`
  with the `Authorization: Bearer <integration-token>`
  header + `Notion-Version: 2022-06-28` date pin.
- `aivyx-obsidian` — vault-rooted filesystem client;
  read/write markdown files under a configured vault
  path with frontmatter parsing and wikilink/tag
  extraction.

## Why this, why now

- **Operator-pressure-driven.** Knowledge-management
  picked over Sheets / Docs / Photos / Tasks at the
  Phase 130 direction question. Both Notion and
  Obsidian are common operator stacks; covering both
  in one phase maximizes the "agent can search +
  modify my knowledge base" value across the operator
  population.

- **Substrate generalization test.** Three consecutive
  Google integrations (Phases 123, 128, 129) might give
  the impression that Chapter F is "Google integrations
  only." Phase 130 proves the chapter pattern works for:
  - **Non-OAuth auth** (Notion's bearer-token).
  - **No-external-API integrations** (Obsidian's
    filesystem-only model).

- **Test substrate for the auth_cli per-binary
  decision.** Phase 129 left auth_cli as a per-binary
  duplicate (each Chapter F crate copies it with bulk
  identifier swap). Notion's auth is trivially simpler
  than OAuth (no init flow; just a config.toml token
  field), so its auth_cli is much smaller. This
  empirically tests whether the per-binary auth_cli
  posture is sustainable or whether the auth_cli lift
  becomes necessary at N=4. If Notion's slim auth_cli
  is awkward to bolt onto the existing OAuth-shaped
  helpers, that's a signal for a substrate phase.

## Q-block sign-off (all four Recommended — fifth all-Recommended phase if you count Phase 127's six)

- **Q1a — Notion 7-tool surface** (Recommended).
  `notion.search`, `notion.get_page`,
  `notion.list_database`, `notion.create_page`,
  `notion.append_blocks`,
  `notion.update_page_properties`,
  `notion.archive_page`. Trusted-gated for the four
  write tools. Mirrors Drive's 7-tool Q2b pattern.

- **Q2a — Obsidian 6-tool surface** (Recommended).
  `obsidian.search`, `obsidian.get_note`,
  `obsidian.list_folder`, `obsidian.create_note`,
  `obsidian.update_note`, `obsidian.delete_note`.
  Trusted-gated for the three write tools.

- **Q3a — Single vault in config.toml** (Recommended).
  Operator configures one `vault_path = "..."` in
  config.toml; all tools operate on that vault.
  Operators with multiple vaults run multiple
  `aivyx-obsidian` binaries with distinct configs.
  Simpler substrate; matches the per-binary-per-account
  pattern.

- **Q4a — Operator-discretionary live verification**
  (Recommended). Matches the Phase 127/128/129
  precedent. Auth flow integration-tested with fake
  transport for Notion; vault ops integration-tested
  with tempfile-vault for Obsidian.

**All four Recommended.** No non-Recommended picks.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract amendment.
  Chapter F pattern accommodates non-Google services
  per its original framing (Phase 123 entry doc:
  "GitHub / others picked later as operator pressure
  dictates"). Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to twenty-one** (was 20
  after Phase 129).

- **PRODUCT.md** — **Will hold.** G6 + P10 + P11 + P12
  cover this case. P10 names "browser, LSP, code
  search, email, calendar, anything domain-specific"
  as third-party territory; Notion and Obsidian fall
  squarely in "anything domain-specific." Hash:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to twenty-one**.

- **`aivyx-core/src/lib.rs`** — **Will hold.** All
  Phase 130 work lives in `aivyx-notion` (new),
  `aivyx-obsidian` (new), and `aivyx-capability` (four
  new bases). NO core changes. Hash:
  `9692e5d102ca0721f1a6a958fda1d24f287e0b1295a09193db92ad82eb5cde35`.
  Prediction: streak **extends from 3 to 4**.

- **New workspace deps** — Zero anticipated. Notion
  uses reqwest + serde (existing). Obsidian uses
  tokio::fs + serde (existing). YAML frontmatter
  parsing in Obsidian uses a hand-written minimal
  parser for the common Obsidian frontmatter shape
  (flat key/value with optional list values) — avoids
  pulling `serde_yaml` as a new workspace dep. Same
  posture as Phase 127's hand-written Python-call
  parser.

- **Test count** — Two crates × ~7 tools per crate ×
  ~15 tests per tool plus client + capability bases +
  vault path-traversal substrate. Prediction: **`+200`
  to `+280`**. Largest phase to date by test count;
  bundled scope is the load-bearing driver.

## Tasks

Seventeen sub-tasks plus exit + backfill:

### Task 1 — Open (this commit)

Entry doc + ROADMAP + README rotation. Documents
operator framing + Q-block resolutions + bundled-scope
honesty.

### Task 2 — `aivyx-notion` crate skeleton + capability bases

- New `crates/aivyx-notion/` with Cargo.toml + lib.rs +
  main.rs + `notion_client.rs` skeleton + `auth_cli/`
  (much simpler than the OAuth-flavored versions:
  just `auth status` + `auth check` — no init/revoke
  since there's no token exchange).
- `notion_client.rs` — `Authorization: Bearer <token>`
  + `Notion-Version: 2022-06-28` headers. HTTP helpers
  (get_json, post_json, patch_json).
- Two new capability bases: `notion.read` (gates
  search + get_page + list_database) and `notion.write`
  (gates the four write tools; CEILING_TRUSTED).
- A3 amendment file updated (Phase 130 addendum;
  KNOWN_BASES_COUNT 61 → 65 — four new bases total
  across Notion + Obsidian, but Obsidian's bases land
  at Task 10).

### Tasks 3-9 — Notion tools

Each task implements one tool + comprehensive tests.
Wire-shape translation between Notion's JSON shapes
(camelCase + nested rich-text arrays) and the
LLM-friendly snake_case output happens at the per-tool
layer; Notion's "page" payload is sufficiently complex
that the get_page tool's transform is the
test-heaviest piece in the Notion side of the phase.

- **Task 3 — `notion.search`** — global search across
  shared pages/databases. Input: `q`, `max_results`,
  `filter` (page-or-database). Cursor-based pagination
  with `next_cursor` in output.
- **Task 4 — `notion.get_page`** — fetch page
  properties + block tree. Input: `page_id`. Output:
  flattened block array with type-discriminated
  shapes; rich-text inline objects collapsed to a
  `plain_text` field with a `rich` field carrying the
  original JSON for operators wanting full fidelity.
- **Task 5 — `notion.list_database`** — query a
  database. Input: `database_id`, optional
  `filter`/`sorts`/`max_results`. Output: pages with
  flattened property values.
- **Task 6 — `notion.create_page`** — create a new
  page in a parent (page or database). Input:
  `parent` (object with `page_id` or `database_id`),
  `properties` (per-database schema), optional
  `children` (initial block array). Trusted-gated.
- **Task 7 — `notion.append_blocks`** — append blocks
  to an existing page. Input: `page_id`, `blocks`
  array. Trusted-gated.
- **Task 8 — `notion.update_page_properties`** —
  patch property values on a page. Input: `page_id`,
  `properties` (patch object). Trusted-gated.
- **Task 9 — `notion.archive_page`** — archive
  (Notion's "delete") a page. Idempotent on
  already-archived. Trusted-gated.

### Task 10 — `aivyx-obsidian` crate skeleton + capability bases

- New `crates/aivyx-obsidian/` with Cargo.toml + lib.rs
  + main.rs + `vault_client.rs` + minimal auth_cli
  (`auth check` to verify vault path is readable).
- `vault_client.rs` — vault root path resolution.
  **Load-bearing path-traversal guard:** all tool
  operations construct paths via a
  `resolve_under_vault(relative_path) -> Option<PathBuf>`
  helper that returns `None` if the canonicalized path
  isn't under the vault root. Symlink-aware (uses
  `tokio::fs::canonicalize` before the prefix check).
- Markdown frontmatter parser (hand-written; supports
  the common Obsidian shape: `---\nkey: value\n[...]\n---`
  at top of file, with string/number/list values).
- Wikilink extractor (`[[Page Name]]` and
  `[[Page Name|display text]]` variants) — operates on
  the body text, returns the raw link strings (no
  fuzzy path resolution in Phase 130; that's a Phase
  131+ candidate if operator pressure surfaces).
- Tag extractor (`#tag` token-bound).
- Two new capability bases: `obsidian.read` (gates
  search + get_note + list_folder) and `obsidian.write`
  (gates create_note + update_note + delete_note;
  CEILING_TRUSTED).
- A3 amendment file's Phase 130 addendum extended with
  the Obsidian bases (KNOWN_BASES_COUNT 63 → 65).

### Tasks 11-16 — Obsidian tools

- **Task 11 — `obsidian.search`** — recursively walks
  vault directory, grep-style line-by-line full-text
  search. Optional `tag` filter (matches `#tag` in
  body), optional `frontmatter_key`+`value` filter.
  Input: `q`, `max_results`, `tag`, `frontmatter_key`,
  `frontmatter_value`. Output: array of matches with
  `{path, snippet, line_number}`.
- **Task 12 — `obsidian.get_note`** — fetch one note.
  Input: `path` (relative to vault). Output:
  `{path, content, frontmatter_raw, body, wikilinks,
  tags}`. `frontmatter_raw` is the literal YAML text
  between `---` markers (LLM-side YAML parsing if
  operator cares about specific fields); `body` is
  content after frontmatter; `wikilinks` is array of
  raw link strings; `tags` is array of unique tags.
- **Task 13 — `obsidian.list_folder`** — list notes
  in a vault subdirectory. Input: `folder` (default
  vault root), `max_results`, `recursive` (default
  false). Output: array of `{path, name, size,
  modified_at}` entries.
- **Task 14 — `obsidian.create_note`** — create a new
  markdown file. Input: `path`, `content` (or
  `frontmatter`+`body` separately). Trusted-gated.
  Path-traversal guard fires before any I/O.
- **Task 15 — `obsidian.update_note`** — replace or
  append to an existing note. Input: `path`, `mode`
  (`"replace"` | `"append"`), `content`. Trusted-
  gated.
- **Task 16 — `obsidian.delete_note`** — delete a
  note. Input: `path`. Trusted-gated.

### Task 17 — INSTALL.md walkthrough + Phase 130 exit

Two new INSTALL.md sub-sections under "External
productivity integrations (Chapter F)":

**Notion (Phase 130):**
- One-time operator setup: create an integration in
  Notion's Integrations dashboard, copy the
  Integration Token, write `notion_token = "..."`
  into config.toml.
- **Critical UX note:** operators must explicitly
  share each page/database with the integration via
  Notion's UI ("Share" → "Invite" → select the
  integration). Without this, all API calls return
  empty results or 404.
- `[[tool_process]]` registration; per-role
  capability grants.
- Operator troubleshooting: "search returns nothing"
  → check sharing; "archive_page returns 404" → page
  was unshared; etc.

**Obsidian (Phase 130):**
- One-time operator setup: write `vault_path =
  "/path/to/MyVault"` (absolute path) into
  config.toml. No external auth.
- **Critical safety note:** path-traversal guard
  protects against `..` escape, but operators should
  STILL register the binary in `[[tool_process]]`
  with an `allowed_paths` constraint scoped to the
  vault directory as a belt-and-suspenders posture.
- Frontmatter/wikilink/tag semantics documented per
  the tool output shape.
- Operator troubleshooting: "vault not found", "path
  must be within vault" errors, etc.

Phase 130 exit doc: prediction-vs-reality, streak
summary, bundled-scope honesty report, Q-block
go-through.

## Exit criteria

- [ ] `docs/PHASE_130.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `aivyx-notion` crate skeleton + bases — Task 2.
- [ ] Seven Notion tools + tests — Tasks 3-9.
- [ ] `aivyx-obsidian` crate skeleton + bases +
  path-traversal substrate — Task 10.
- [ ] Six Obsidian tools + tests — Tasks 11-16.
- [ ] INSTALL.md walkthrough + Phase 130 exit doc —
  Task 17.
- [ ] Q1 / Q2 / Q3 / Q4 resolved pre-Task 2.
- [ ] DESIGN.md streak — predicted HOLD (streak → 21).
- [ ] PRODUCT.md streak — predicted HOLD (streak → 21).
- [ ] `aivyx-core/src/lib.rs` streak — predicted HOLD
  (3 → 4).
- [ ] Zero new workspace dependencies.
- [ ] Test count delta within `+200` to `+280`.
- [ ] Zero clippy warnings.
- [ ] **No live verification as exit criterion** —
  operator-discretionary per Q4a.

## Honest scope risks at sign-off

- **Bundled scope is the largest Chapter F phase yet
  by tool count (13) and task count (17).** PR-merge-
  time scope reduction (defer one of the two
  integrations to Phase 131) is the escape hatch if
  the phase becomes unwieldy mid-implementation.
  Phase 6 Q5 honest framing applies.

- **Notion's "share each page" UX requires very clear
  INSTALL.md documentation.** Operators who skip the
  sharing step will see empty results and wonder why
  the integration "doesn't work." The error messages
  in `notion.search` and `notion.get_page` should
  explicitly mention this when API calls return 404 /
  empty.

- **Obsidian's path-traversal guard is load-bearing
  for security.** Every tool MUST go through
  `resolve_under_vault` before touching the filesystem.
  Test coverage on the guard is the load-bearing exit
  criterion for Task 10 — symlink escape, `..` escape,
  absolute-path-supplied-by-operator escape all need
  explicit tests.

- **Obsidian wikilink resolution is deferred.** The
  output's `wikilinks` field contains raw link strings
  (no fuzzy path resolution to actual files). Operators
  wanting resolved wikilinks need to call
  `obsidian.search` on the linked text or
  `obsidian.list_folder` to disambiguate. Documented
  honestly; the resolution semantics are non-trivial
  (Obsidian uses case-insensitive fuzzy matching with
  vault-wide search).

- **Notion's API has cursor-based pagination** (not
  page-token like Drive/Calendar). Each tool that
  returns lists carries `next_cursor` in its output.
  Same operator UX as the `next_page_token` pattern
  in the Google integrations but with different
  naming.

- **Notion-Version pinning.** The `Notion-Version`
  header is date-pinned (`2022-06-28`). If Notion
  rev's the API breaking, the pin shields us; if
  Notion deprecates the pinned version, a future
  substrate phase bumps it. Honest scope flag.

- **Auth CLI lift posture — test signal.** Notion's
  auth_cli is much simpler than gmail/calendar/drive's
  OAuth-flavored versions. If wrapping Notion's "just
  check the token works" flow in the OAuth-shaped
  auth_cli structure proves awkward, that's a signal
  for an auth_cli lift in a follow-on substrate
  phase. Phase 130 reports this empirically at exit.

- **YAML frontmatter parsing is hand-written.** The
  common Obsidian frontmatter shape (flat key/value
  with optional list values) is what the parser
  handles. Exotic frontmatter (nested objects, multi-
  line strings, anchors/aliases) drops to raw-string
  passthrough in the `frontmatter_raw` field. Same
  posture as Phase 127's Python-call parser.

- **Eighteenth consecutive deferral of the Channel
  Activation Milestone.** Honest tracking continues.
  Audit's #1. The deferral count's signal-strength
  is now load-bearing — Phase 131+ should weigh it
  explicitly.

## Direction after Phase 130

After Phase 130, Phase 131 candidates:

1. **Channel Activation Milestone** — eighteenth
   consecutive deferral if skipped. The substrate
   completeness signal (Aivyx can DO substantial work
   across ~45 tools post-Phase-130) makes channels
   the increasingly-load-bearing question. Phase 131
   should weigh this explicitly.
2. **Chapter F #7 — GitHub** — non-Google + non-token
   auth (PAT). Completes the substrate-generalization
   coverage.
3. **Auth CLI substrate lift** — if Phase 130
   surfaces awkwardness wrapping Notion's slim auth
   in the OAuth-shaped helpers.
4. **Wikilink resolution substrate for Obsidian** —
   Phase 130 deferral.
5. **Release prep (v0.1.0 + installer)** — five
   Chapter F integrations + 130 phases is a strong
   release milestone.

## Prediction vs reality

_Populated at Phase 130 exit. Predictions captured at
sign-off: DESIGN.md HOLD → 21; PRODUCT.md HOLD → 21;
lib.rs HOLD → 4; test count `+200` to `+280`; zero
new deps; zero clippy warnings._

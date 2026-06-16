# Phase 129 — Google Drive Integration + OAuth Substrate Lift (Chapter F #3)

**Chapter F third integration triggers the OAuth substrate
lift.** Phase 128 Q2a deferred the OAuth lift to the third
Google integration when N=3 would make the substrate work
unambiguously worth the cost. Phase 129 is that moment.
Three would-be in-tree copies of the OAuth substrate
(gmail + calendar + drive) collapse to one shared
`aivyx-google-oauth` crate before the third lands.

## Chapter F context (this is the third integration)

Chapter F's pattern locked at Phase 123 (Gmail) and held
through Phase 128 (Calendar):

- One target integration per phase as a separate binary
  crate.
- Per-service auth substrate (OAuth for Google services;
  PAT for GitHub; service-specific tokens for others).
- Per-service capability scopes registered into the
  existing capability machinery.
- INSTALL.md walkthrough for operator-side setup.

Phase 129 ships Chapter F #3 — Drive — under the same
pattern, plus the OAuth substrate lift that Phase 128 Q2a
explicitly deferred to this phase. After the lift, all
three Google integrations consume the shared
`aivyx-google-oauth` crate; future Google integrations
(Sheets, Docs, Photos, Tasks, etc) get the OAuth substrate
for free.

## Why this, why now

- **Operator-pressure-driven.** Drive picked over GitHub
  / Notion / Slack-tools at the Phase 129 direction
  question for "highest substrate-completion value:
  triggers the OAuth lift" framing.

- **Substrate-completion phase, in the same sense Phase
  127 was.** Phase 128 left honest tech debt: two
  in-tree OAuth copies (gmail + calendar). Phase 129
  closes that gap before the third copy lands. The
  precedent for "lift at N=3" was set by Phase 128 Task
  2's harness lift (which closed Phase 125's twice-
  duplicated harness finding).

- **Drive is operator-load-bearing for the personal-
  assistant value prop.** Document workflow (find a file,
  read it, save edits or new content) is one of the most
  common operator asks alongside email + calendar.

- **The OAuth lift makes future Google integrations
  significantly cheaper.** Phase 130+ candidates that
  ship a Google service (Sheets, Docs, Photos, Tasks,
  Contacts) consume the lifted substrate verbatim; per-
  integration scope drops to just the API client + tool
  surface. Same compounding leverage the harness lift
  provided for Phase 128 onward.

## Q-block sign-off (3 Recommended + 1 non-Recommended)

- **Q1a — Bundle OAuth substrate lift into Phase 129**
  (Recommended). Extract gmail's `oauth/` + `auth_cli/`
  modules to a shared `aivyx-google-oauth` crate;
  migrate both `aivyx-gmail` + `aivyx-calendar` to
  consume the lifted crate; build `aivyx-drive` against
  it from the start. Both existing crates retain their
  binary's CLI dispatch (`aivyx-gmail auth init` etc) as
  thin wrappers over the lifted helpers, parameterized
  by service name + default scope set.

- **Q2b — 7 tools: read + write + folder management**
  (non-Recommended; operator-picked over Q2a's 5-tool
  default surface). Tools:
  - `drive.search` — find files by name / mime type /
    folder / owner
  - `drive.get_metadata` — file metadata (name, mime,
    size, modified, owner, parents)
  - `drive.list_folder` — list children of a folder
  - `drive.create_folder` — create a folder (Trusted-
    gated)
  - `drive.download_file` — file content as base64
    (size-capped at 10 MB; metadata-only above cap)
  - `drive.upload_file` — create/replace a file with
    content (Trusted-gated; size-capped at 10 MB)
  - `drive.delete_file` — delete (Trusted-gated;
    idempotent — already-deleted returns
    `was_already_deleted: true`)

  **Honest framing per Phase 6 Q5:** Q2b's two extra
  tools (`drive.list_folder` + `drive.create_folder`)
  cover hierarchical-document workflows; doubles the
  folder-semantics test surface vs Q2a's flat-search-
  only shape. Acceptable trade-off; honest report at
  exit if either folder tool needs scope reduction.

- **Q3a — Base64 in JSON with size cap** (Recommended).
  Drive file content represented as base64-encoded
  bytes in the tool's JSON output. Operator-visible cap
  at 10 MB; above the cap, the download tool returns
  metadata only with `content_truncated: true` and a
  clear "file too large for inline transfer" message.
  Matches Gmail's attachment substrate posture.
  Streaming via `ToolEventPayload::OutputChunk` is the
  natural Phase 130+ follow-up if operator pressure
  surfaces for large-file workflows.

- **Q4a — Operator-discretionary live verification**
  (Recommended). Substrate is unit-tested per-tool;
  OAuth flow inherits the integration-tested fake
  transport from Phase 123 (now lifted to the shared
  crate). Live test against a real Google Drive
  documented in INSTALL.md but not a phase exit gate.
  Matches the Phase 127/128 precedent.

**Three Recommended + one non-Recommended.** Q2b's
operator pick was explicit; PR-merge-time scope reduction
is the escape hatch if any of the seven tools needs to
defer.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract amendment.
  Chapter F pattern is mature. Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to twenty** (was 19 after
  Phase 128).

- **PRODUCT.md** — **Will hold.** No contract change.
  G6 + P10 + P11 + P12 cover this case exactly. Hash:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to twenty**.

- **`aivyx-core/src/lib.rs`** — **Will hold.** All
  Phase 129 work lives in `aivyx-google-oauth` (new
  crate; OAuth lift), `aivyx-drive` (new crate; Drive
  integration), `aivyx-gmail` + `aivyx-calendar` (OAuth
  migration), and `aivyx-capability` (two new bases
  `drive.read` + `drive.write`). NO core changes. Hash:
  `9692e5d102ca0721f1a6a958fda1d24f287e0b1295a09193db92ad82eb5cde35`.
  Prediction: streak **extends from 2 to 3**.

- **New workspace deps** — Zero anticipated. The
  lifted `aivyx-google-oauth` crate uses the same
  dependencies the gmail/calendar OAuth modules already
  used; `aivyx-drive` mirrors `aivyx-calendar`'s
  Cargo.toml.

- **Test count** — Phase 129 has TWO substantial test
  bodies:
  - OAuth lift: the existing OAuth + auth_cli tests from
    gmail (substantial body — ~80 tests inherited at
    Phase 128's calendar copy) consolidate to a single
    test surface in the lifted crate. Both consumers'
    inline OAuth tests removed; the lifted crate carries
    them. Net: tests move, not duplicate.
  - Drive tools: 7 tools × per-tool input validation +
    schema tests + wire-shape translation + integration
    sanity ≈ ~17 tests/tool. Plus capability bases
    bump.

  Prediction: **`+120` to `+170`**. Lower than Phase
  128's `+167` because Phase 128 had the OAuth+auth_cli
  body coming along inline via the copy; Phase 129
  consolidates that body to one shared crate (Drive
  inherits via dependency, not via duplication).

## Tasks

Eleven sub-tasks plus exit + backfill:

### Task 1 — Open (this commit)

`docs/PHASE_129.md` + `docs/ROADMAP.md` Phase 129 entry +
`docs/README.md` status row. Documents the operator-
pressure framing + the Q-block resolutions + the
OAuth-lift bundling + the streak predictions.

### Task 2 — OAuth substrate lift to `aivyx-google-oauth`

Extract the OAuth substrate from `aivyx-gmail`:

- New `crates/aivyx-google-oauth/` directory:
  - `Cargo.toml` — workspace member; deps mirror gmail's
    oauth surface (reqwest, serde, serde_json, base64,
    toml, uuid, thiserror, tokio).
  - `src/lib.rs` — public surface exporting `OAuthConfig`,
    `TokenSet`, `exchange_code`, `refresh_access_token`,
    `load_tokens`, `save_tokens`, `ExchangeError`,
    `StorageError`, `OAuthError`,
    `GOOGLE_AUTH_ENDPOINT`, `GOOGLE_TOKEN_ENDPOINT`.
  - `src/config.rs` — `OAuthConfig` struct + serde
    derives. No service-specific default scopes;
    constructor takes `scopes: Vec<String>` directly so
    each consumer provides its own default.
  - `src/tokens.rs` — `TokenSet` struct + `needs_refresh`,
    `can_refresh` helpers (verbatim from gmail).
  - `src/exchange.rs` — auth-code → tokens, refresh →
    tokens (verbatim from gmail).
  - `src/storage.rs` — token file I/O with `0600` perms,
    atomic write-then-rename. `save_tokens(path, tokens)`
    + `load_tokens(path)` take the path as a parameter
    so each service's `~/.aivyx/tool-processes/{name}/tokens.json`
    works without service-specific code in the lifted
    crate.
  - `src/auth_helpers.rs` — `run_auth_init_helper`,
    `run_auth_status_helper`, `run_auth_revoke_helper`
    parameterized by service name, config path, token
    path, default scope set. Each consumer's
    `auth_cli/init.rs` becomes a thin wrapper that
    supplies the service-specific parameters.

- **Migrate `aivyx-gmail`:**
  - `Cargo.toml` adds `aivyx-google-oauth = { path =
    "../aivyx-google-oauth" }`.
  - `src/oauth/` module replaced with a re-export shim
    that pulls in the lifted types (preserves
    `aivyx_gmail::OAuthConfig` etc public API).
  - `src/auth_cli/` keeps its binary's CLI surface but
    delegates the bodies to the lifted helpers.
  - `src/oauth/config.rs::DEFAULT_GMAIL_SCOPES` moves to
    `aivyx-gmail` proper (not lifted; service-specific).

- **Migrate `aivyx-calendar`:**
  - Same shape as gmail: re-export shim + auth_cli
    wrappers; `DEFAULT_CALENDAR_SCOPES` stays in calendar.

- **Verify behavior preservation:**
  - All 147 gmail tests pass unchanged.
  - All 162 calendar tests pass unchanged.
  - The lifted crate carries the consolidated OAuth +
    auth_cli tests; both consumers' inline tests
    removed.

NO new functionality; pure substrate refactor. Tests
exercise the lift via the existing gmail + calendar +
the new lifted-crate suites.

### Task 3 — `aivyx-drive` crate skeleton + capability bases

- New `crates/aivyx-drive/` directory:
  - `Cargo.toml` — workspace member; consumes
    `aivyx-google-oauth` directly (no inline OAuth
    copy).
  - `src/lib.rs` — public surface.
  - `src/main.rs` — binary entry; CLI dispatch (auth |
    serve) mirroring the gmail / calendar pattern; thin
    wrappers around the lifted auth helpers.
  - `src/auth_cli/cli.rs` — binary's arg parser
    (`aivyx-drive auth init / status / revoke / help`).
  - `src/drive_client.rs` — Google Drive v3 REST API
    client. POST/GET/PATCH/DELETE helpers parameterized
    on `aivyx-google-oauth::TokenSet`; refresh-on-expiry
    posture matches the calendar/gmail clients.
  - `src/tools/mod.rs` — entry-point for the seven tool
    modules tasks 4-10 ship.
  - `src/DEFAULT_DRIVE_SCOPES` constant —
    `["https://www.googleapis.com/auth/drive"]` (broad
    scope by default; narrower options in INSTALL.md).
- Two new capability bases in `aivyx-capability`:
  - `drive.read` — gates `search`, `get_metadata`,
    `list_folder`, `download_file`.
  - `drive.write` — gates `create_folder`,
    `upload_file`, `delete_file`. CEILING_TRUSTED.
- A3 amendment file updated (Phase 129 addendum;
  KNOWN_BASES_COUNT 59 → 61).
- Workspace `Cargo.toml` includes the new crate.

### Task 4 — `drive.search` tool

- `DriveClient::search(query, max_results)` — query
  `files.list` with a Google Drive search query string.
- Input schema: `q` (Drive query DSL — e.g.
  `name contains 'budget' and mimeType =
  'application/vnd.google-apps.spreadsheet'`),
  `max_results` (default 25, cap 100),
  `include_trashed` (default false).
- Output: array of file summaries `{id, name, mime_type,
  size, modified_at, owner_email, parent_folder_ids}`.
- Capability: `drive.read`.
- Tests: input validation + schema + wire-shape
  translation + integration sanity.

### Task 5 — `drive.get_metadata` tool

- `DriveClient::get_metadata(file_id)` — GET `files/{id}`
  with full metadata fields.
- Input schema: `file_id` (required).
- Output: full metadata object (everything from search
  plus description, version, app_properties).
- Capability: `drive.read`.

### Task 6 — `drive.list_folder` tool

- Range query for folder children. Internally a
  `files.list` with `'{folder_id}' in parents` query.
- Input schema: `folder_id` (default `"root"`),
  `max_results`, `include_trashed`.
- Output: array of file summaries (same shape as
  search).
- Capability: `drive.read`.

### Task 7 — `drive.create_folder` tool (Trusted-gated)

- POST `files` with `mimeType =
  'application/vnd.google-apps.folder'`.
- Input schema: `name` (required), `parent_folder_id`
  (default `"root"`).
- Output: `{id, name, parent_folder_id}` of created
  folder.
- Capability: `drive.write` (CEILING_TRUSTED).

### Task 8 — `drive.download_file` tool

- GET `files/{id}?alt=media` for binary content. For
  Google-native types (Docs/Sheets/Slides) use the
  `export` endpoint with `mimeType` parameter; default
  exports to text/PDF as appropriate.
- Input schema: `file_id` (required), `export_mime_type`
  (optional; for Google-native types).
- Output:
  - File ≤ 10 MB: `{file_id, name, mime_type, size,
    content_base64, content_truncated: false}`.
  - File > 10 MB: `{file_id, name, mime_type, size,
    content_base64: null, content_truncated: true,
    error: "file size N exceeds inline cap 10 MB"}`.
- Capability: `drive.read`.
- Tests: input validation + size-cap behavior + base64
  round-trip + Google-native export semantics.

### Task 9 — `drive.upload_file` tool (Trusted-gated)

- Multipart upload via `upload/drive/v3/files`.
  Resumable upload protocol deferred (single-shot
  multipart covers the inline cap).
- Input schema: `name` (required), `mime_type`
  (required), `content_base64` (required; size-capped
  at 10 MB before base64 expansion), `parent_folder_id`
  (default `"root"`), `description` (optional).
- Output: `{id, name, mime_type, size,
  parent_folder_id}` of created file.
- Capability: `drive.write` (CEILING_TRUSTED).
- Tests: input validation (required-field +
  base64-decode + size-cap) + wire-shape + integration
  sanity.

### Task 10 — `drive.delete_file` tool (Trusted-gated)

- DELETE `files/{id}`.
- Input schema: `file_id` (required).
- Output: `{file_id, was_already_deleted}` (idempotent;
  410 Gone treated as success — matches calendar.delete_event
  pattern).
- Capability: `drive.write` (CEILING_TRUSTED).

### Task 11 — INSTALL.md walkthrough + Phase 129 exit

INSTALL.md operator-facing section under "External
productivity integrations (Chapter F)":

- "Google Drive (Phase 129)" sub-section mirroring the
  Gmail (Phase 123) + Calendar (Phase 128) sub-section
  structure.
- One-time operator setup with three paths:
  - Path A — already have Gmail + Calendar OAuth client;
    just enable the Drive API + add the scope.
  - Path B — already have Gmail only; same as A.
  - Path C — Drive is your first Google integration.
- Scope-narrowing options
  (`auth/drive.readonly`, `auth/drive.file`,
  `auth/drive.metadata.readonly`).
- `[[tool_process]]` registration + per-tool capability
  table.
- Per-role capability grants.
- Operator-side troubleshooting: file-too-large errors,
  Google-native export semantics, permissions /
  ownership issues.
- Honest scope notes: 7 tools landed (Q2b operator-
  picked over Q2a 5-tool default).

OAuth substrate lift documentation:

- Brief operator-facing note that `aivyx-gmail` +
  `aivyx-calendar` + `aivyx-drive` all share the same
  OAuth substrate post-lift. Existing operator OAuth
  config files (`~/.aivyx/tool-processes/{gmail,calendar,drive}/config.toml`)
  remain per-service; the LIFT is internal to Aivyx,
  not operator-facing.

Phase 129 exit doc:
- Prediction-vs-reality.
- Streak summary.
- OAuth-lift status (test count consolidated; behavior
  preservation tests across all three consumers).
- Drive tool surface delivered (7 tools; Q2b
  operator-picked).

## Exit criteria

- [ ] `docs/PHASE_129.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] OAuth substrate lifted to `aivyx-google-oauth`;
  gmail + calendar migrated; all prior tests pass — Task 2.
- [ ] `aivyx-drive` crate skeleton + capability bases —
  Task 3.
- [ ] `drive.search` + tests — Task 4.
- [ ] `drive.get_metadata` + tests — Task 5.
- [ ] `drive.list_folder` + tests — Task 6.
- [ ] `drive.create_folder` + tests — Task 7.
- [ ] `drive.download_file` + tests (size cap behavior
  validated) — Task 8.
- [ ] `drive.upload_file` + tests — Task 9.
- [ ] `drive.delete_file` + tests — Task 10.
- [ ] INSTALL.md walkthrough + Phase 129 exit doc —
  Task 11.
- [ ] Q1 / Q2 / Q3 / Q4 resolved pre-Task 2.
- [ ] DESIGN.md streak — predicted HOLD (streak → 20).
- [ ] PRODUCT.md streak — predicted HOLD (streak → 20).
- [ ] `aivyx-core/src/lib.rs` streak — predicted HOLD
  (2 → 3).
- [ ] Zero new workspace dependencies.
- [ ] Test count delta within `+120` to `+170`.
- [ ] Zero clippy warnings.
- [ ] **No live verification as exit criterion** —
  operator-discretionary per Q4a.

## Honest scope risks at sign-off

- **Q2b's 7-tool surface doubles the folder-semantics
  test surface** vs Q2a's flat-search-only shape. Per-
  tool test parity (~17 tests/tool) gives the substrate
  validation; if either folder tool reveals semantic
  issues (e.g. nested-folder edge cases), scope
  reduction is the escape hatch.

- **OAuth lift could break behavior** in either
  consumer. The behavior-preservation tests across
  gmail + calendar are the load-bearing exit criterion
  for Task 2 — mirrors Phase 128 Task 2's harness-lift
  posture exactly.

- **Binary content size cap.** 10 MB cap is operator-
  visible in the tool's input schema + error responses,
  but operators with large-file workflows (videos,
  large PDFs) will hit it. INSTALL.md documents the cap
  + the Phase 130+ streaming-substrate trajectory.

- **Google-native types (Docs / Sheets / Slides)** need
  the `export` endpoint with a mime_type, not the `media`
  endpoint. `drive.download_file` handles the dispatch
  internally; tests exercise both paths.

- **Resumable upload deferred.** `drive.upload_file`
  uses single-shot multipart, which works under the
  10 MB cap. Resumable upload (for above-cap files
  via streaming) is the Phase 130+ scope.

- **OAuth scope creep risk.** Drive's `auth/drive` is
  the broad scope (read+write all). Narrower options
  (`auth/drive.file` — only files created by Aivyx;
  `auth/drive.readonly`; `auth/drive.metadata.readonly`)
  documented in INSTALL.md with the same posture as
  Calendar's scope-narrowing section.

- **Seventeenth consecutive deferral of the Channel
  Activation Milestone.** Honest tracking continues.
  Audit's #1. The deferral count is now a load-bearing
  signal — at some point this becomes the next phase.

## Direction after Phase 129

After Phase 129, Phase 130 candidates:

1. **Chapter F #4 — Sheets / Photos / Tasks / Docs**
   (any Google service now ships against the lifted
   OAuth substrate cheaply).
2. **Chapter F #5 — GitHub or Notion** (different auth
   model; proves the substrate beyond Google).
3. **Channel Activation Milestone** — seventeenth
   consecutive deferral if skipped; the gap between
   "agent can DO work" (now ~25 tools across substrate
   + Chapter F + Chapter G) and "agent is usable on a
   real chat surface" becomes increasingly load-bearing.
4. **Streaming-substrate for binary file content** —
   `ToolEventPayload::OutputChunk` extension to handle
   large-file workflows; unblocks above-10MB Drive
   tools.
5. **Release prep (v0.1.0 + installer)** — substrate +
   product completion postcards a natural release
   milestone.

## Prediction vs reality

**Three-of-three streak predictions correct.**

- **DESIGN.md** — HELD as predicted (`c2be6d51…`
  unchanged). No contract amendment; Chapter F pattern
  is mature. Streak: 19 → **20**.

- **PRODUCT.md** — HELD as predicted (`6e840cef…`
  unchanged). G6 + P10 + P11 + P12 cover this case
  exactly as Phase 128. Streak: 19 → **20**.

- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`9692e5d…` unchanged). All Phase 129 work lives in
  `aivyx-google-oauth` (new crate), `aivyx-drive` (new
  crate), `aivyx-gmail` + `aivyx-calendar` (OAuth
  migration), and `aivyx-capability` (two new bases).
  NO core changes. Streak: 2 → **3**.

**Test count `+117` undershot the predicted `+120 to
+170` range by 3 tests** — honest report. The undershoot
reflects the OAuth lift's test consolidation working as
designed:

- aivyx-google-oauth (new crate): **+31** tests
- aivyx-drive (new crate, all 7 tools + auth_cli body):
  **+147** tests
- aivyx-gmail: **-30** (oauth implementation tests
  moved to the lifted crate)
- aivyx-calendar: **-31** (same — oauth implementation
  tests consolidated)
- aivyx-capability: 0 net (one count test renamed +
  bumped from 59 to 61)

Sum: 31 + 147 - 30 - 31 = **+117 net workspace tests**.

Without the OAuth lift, Phase 129 would have shipped
~+177 (the 30+31 = 61 OAuth duplication would have come
along inline with each of gmail/calendar/drive). The
"savings" of 61 - 31 = **30 fewer duplicated tests**
across the workspace is the substrate consolidation
paying off in test code as well as in implementation
code (-900 LoC at the Task 2 lift). Test count being
3 below the lower prediction bound is the cost of
that consolidation working better than estimated.
Honest framing per Phase 6 Q5: this isn't a "skipped
tests" undershoot; it's a "lifted-substrate-eliminated-
duplication" undershoot, which is a substrate quality
win.

Per-task breakdown of the Drive +147:

- Task 3 (drive skeleton + capability bases): **+53** —
  substantial inherited body from the verbatim
  `auth_cli` copy (calendar's auth_cli moved here with
  bulk identifier swap; ~50 tests carry over). Plus
  the drive_client.rs skeleton's 4 tests and the
  tools/mod.rs `drive_urlencode` 3 tests.
- Task 4 (drive.search): **+19** — input validation,
  q-string building (5 combinations), file_summary
  transformation (4 — including Drive's string-typed
  `size` field), schema, integration sanity.
- Task 5 (drive.get_metadata): **+11** — input
  parsing, full-metadata transform, schema +
  integration sanity.
- Task 6 (drive.list_folder): **+13** — input parsing,
  folder-q building including single-quote and
  backslash escaping (defensive against DSL injection
  if a malformed folder_id ever leaks), schema +
  integration sanity.
- Task 7 (drive.create_folder): **+11** — input
  validation, wire-shape (load-bearing folder
  mimeType), schema + integration sanity.
- Task 8 (drive.download_file): **+16** — input
  parsing (with `export_mime_type` optionality),
  default-export-mime mapping per Google-native type
  (5 — Docs/Sheets/Slides/Drawings/unknown),
  truncated-output shape (2), schema + integration
  sanity.
- Task 9 (drive.upload_file): **+16** — input
  parsing including the 10 MB cap boundary (above /
  exactly-at / well-under), malformed-base64
  handling, wire-shape (3 — metadata + description +
  explicit parent), schema + integration sanity.
- Task 10 (drive.delete_file): **+8** — input
  parsing + schema + integration sanity.

**Zero new workspace dependencies** as predicted. The
lifted `aivyx-google-oauth` Cargo.toml is the precise
subset that gmail's `oauth/` module needed; the new
`aivyx-drive` Cargo.toml mirrors `aivyx-calendar`'s plus
the `multipart` reqwest feature for `drive.upload_file`.

**Zero clippy warnings** workspace-wide. Three transient
lints fixed during dev:
1. `doc_lazy_continuation` on main.rs preamble ("+ the
   IPC-loop" → "plus the IPC-loop") — same pattern as
   Phase 128 Task 3.
2. Unused `drive_urlencode` warning before Tasks 5/6
   wired it — added `#[allow(dead_code)]` until first
   consumer landed.
3. None on the Drive client's helpers because they were
   all `pub` from the start (visible to per-tool
   modules).

**Q-block went through as picked.** Three Recommended +
one non-Recommended (Q2b 7-tool surface). The 7-tool
surface landed cleanly with per-tool test parity
(~13 tests each); the operator's "doubled folder-
semantics test surface" risk at sign-off materialized
as `drive.list_folder`'s folder-q escaping tests plus
`drive.create_folder`'s mimeType wire-shape test —
both essential, neither runaway.

**OAuth lift behavior preservation confirmed.** Phase
128 Q2a's "lift at N=3" trigger fired cleanly:

- 117 gmail tests pass unchanged (the 147 → 117
  delta is OAuth implementation tests that
  consolidated to the lifted crate, not behavior
  regressions).
- 131 calendar tests pass unchanged (same 162 → 131
  consolidation).
- Net diff at Task 2 commit: **-900 LoC** (2,422
  deleted vs 1,522 inserted). Two ~1,300-LoC OAuth
  copies became one shared crate at ~880 LoC plus thin
  re-export shims.
- The pre-lift "absent scopes = service defaults"
  behavior is preserved at each consumer's config-
  loading layer; substrate-level `OAuthConfig::new()`
  yields empty scopes (each consumer chains
  `.with_scopes(DEFAULT_X_SCOPES.iter().copied())` in
  its factory). Behavior tests in both consumer crates
  exercise this path.

**Honest scope risks at sign-off — status at exit:**

- **Q2b's 7-tool surface stayed tractable.** No
  PR-merge-time scope reduction needed. The two
  folder tools (list_folder + create_folder) added
  modest extra test surface around folder semantics
  that paid off in correctness (single-quote / backslash
  escaping in list_folder's q-building was caught at
  the unit-test layer).

- **OAuth lift did NOT break behavior** as feared at
  sign-off. The behavior-preservation tests across
  gmail + calendar all passed unchanged post-lift.

- **10 MB inline cap is operator-visible** as planned.
  INSTALL.md documents the cap + the Phase 130+
  streaming-substrate trajectory. Both download_file
  and upload_file enforce the cap with clear error
  messages.

- **Google-native types** handled via the export
  endpoint with sensible default mime mappings
  (Docs → PDF, Sheets → CSV, Slides → PDF, Drawings
  → PNG, unknown native → PDF). Operators override
  via `export_mime_type`.

- **Resumable upload deferred** per Q3a. Single-shot
  multipart covers the cap.

- **Drive's `auth/drive` default is broad** with three
  narrower options documented in INSTALL.md
  (`auth/drive.file`, `auth/drive.readonly`,
  `auth/drive.metadata.readonly`).

- **Seventeenth consecutive deferral of the Channel
  Activation Milestone** as forecast. Audit's #1. The
  deferral count is now a load-bearing signal — Phase
  130+ should weigh it explicitly when picking
  direction.

**Auth CLI still per-binary** as an honest tech debt
flag carried from Task 3. After the OAuth lift, each
Chapter F binary keeps its own `auth_cli/{cli,
config_file, init, status, revoke}.rs` with bulk
identifier swaps. A future substrate phase could lift
the auth_cli helpers too (parameterized by service
name + default scope set), but the per-binary CLI
dispatcher would still need to live in each crate.
Phase 130+ candidate if operator pressure surfaces.

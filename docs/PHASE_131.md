# Phase 131 — n8n Workflow Automation Integration (Chapter F #7)

**Chapter F seventh integration.** First Chapter F phase
targeting a **workflow automation service** —
operationally and conceptually different from the prior
six (email / calendar / drive / notion / obsidian). The
agent's high-leverage move with n8n isn't authoring or
reading content; it's **inspecting what automations
exist, triggering them, and observing their executions**.

## Chapter F context

Phase 130 (Notion + Obsidian) closed by validating the
Chapter F pattern across:
- OAuth-based services (gmail / calendar / drive).
- Bearer-token services (notion).
- No-API filesystem services (obsidian).

Phase 131 extends the pattern to a **fourth substrate
shape** — REST API + bearer token, but with an **operator-
configurable base URL** because n8n is self-hosted. Every
prior Chapter F crate has hard-coded a service base URL
(`api.notion.com`, `www.googleapis.com`, etc); n8n's
substrate must accept an operator-provided
`n8n_base_url` for each tool-process install.

## Why this, why now

- **Operator-pressure-driven.** Phase 131 direction
  picked after Phase 130 (Notion + Obsidian) shipped;
  the user explicitly framed n8n as "same as we just did
  for Notion + Obsidian." Workflow automation is a
  natural extension of the knowledge-management bundle:
  Notion/Obsidian let the agent inspect and modify
  knowledge bases; n8n lets the agent inspect and
  trigger automations that act on the world.

- **Substrate generalization completes the picture.**
  After Phase 131, Chapter F covers four substrate
  shapes (OAuth, bearer-token-fixed-base,
  bearer-token-operator-base, filesystem). That's a
  meaningful breadth signal for the "Aivyx integrates
  with anything operators run" framing.

- **Auth_cli lift posture, second data point.** Phase
  130's exit empirically validated that the OAuth-shaped
  `auth_cli` is awkward to wrap around non-OAuth flows.
  Phase 131's `aivyx-n8n` auth_cli will be slim
  (basically identical to `aivyx-notion`'s) — a second
  reinforcing signal for the Phase 132+ `auth_cli`
  substrate lift candidate.

## Q-block sign-off (2 Recommended + 1 non-Recommended)

- **Q1c — 10-tool surface (read + trigger + lifecycle
  + full CRUD)** (non-Recommended; operator-picked over
  Q1a's 7-tool default).

  Tools:
  - **Read (4, capability `n8n.read`):**
    `n8n.list_workflows`, `n8n.get_workflow`,
    `n8n.list_executions`, `n8n.get_execution`.
  - **Trigger + lifecycle (3, capability `n8n.write`,
    Trusted-gated):** `n8n.execute_workflow`,
    `n8n.activate_workflow`, `n8n.deactivate_workflow`.
  - **CRUD (3, capability `n8n.write`,
    Trusted-gated):** `n8n.create_workflow`,
    `n8n.update_workflow`, `n8n.delete_workflow`.

  **Honest framing per Phase 6 Q5:** workflow CRUD is
  risky because n8n's workflow JSON is complex
  (nodes + connections + credentials references + node-
  type-specific parameters). A malformed update can
  silently break automations — n8n accepts the PATCH
  but the workflow no longer runs correctly. The Q1a
  default deliberately excluded CRUD; the operator
  picked Q1c for the "agent can be a full participant
  in n8n authoring" framing. PR-merge-time scope
  reduction (defer the 3 CRUD tools to Phase 132+) is
  the escape hatch if the per-tool implementation
  reveals issues.

- **Q2a — `n8n_base_url` + `n8n_api_key` in
  config.toml** (Recommended). Two fields in
  `~/.aivyx/tool-processes/n8n/config.toml`. The crate
  constructs requests as `{n8n_base_url}/api/v1/{path}`
  with the `X-N8N-API-KEY` header. No auto-discovery
  magic; operators know their instance URL.

- **Q4a — Operator-discretionary live verification**
  (Recommended). Matches the Phase 127/128/129/130
  precedent.

**Two Recommended + one non-Recommended (Q1c).** Same
pattern as Phase 128 / 129 which had one non-Recommended
each.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract amendment.
  Chapter F pattern accommodates new substrate shapes
  via P10 ("anything domain-specific is third-party
  territory"). Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to twenty-two** (was 21
  after Phase 130).

- **PRODUCT.md** — **Will hold.** G6 + P10 + P11 + P12
  cover this case. Hash:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to twenty-two**.

- **`aivyx-core/src/lib.rs`** — **Will hold.** All
  Phase 131 work lives in `aivyx-n8n` (new) and
  `aivyx-capability` (two new bases). NO core changes.
  Hash:
  `9692e5d102ca0721f1a6a958fda1d24f287e0b1295a09193db92ad82eb5cde35`.
  Prediction: streak **extends from 4 to 5**.

- **New workspace deps** — Zero anticipated.
  `reqwest` + `serde` + `serde_json` + `toml` + `tokio`
  + `thiserror` all already in use by other Chapter F
  crates.

- **Test count** — 10 tools × ~13 tests per tool +
  skeleton + capability count test bump. Prediction:
  **`+140` to `+200`**. About half the size of Phase
  130's bundled Notion+Obsidian (which shipped 13
  tools across two crates and landed at +220).

## Tasks

Thirteen sub-tasks plus exit + backfill:

### Task 1 — Open (this commit)

Entry doc + ROADMAP + README rotation. Phase 130 README
hash backfilled here per the established pattern.

### Task 2 — `aivyx-n8n` skeleton + capability bases

- New `crates/aivyx-n8n/` crate skeleton:
  - `Cargo.toml` — workspace member; deps mirror
    aivyx-notion's (no aivyx-google-oauth dep).
  - `src/lib.rs` — public surface + Phase 131 chapter
    framing. `default_config_path()` returning
    `~/.aivyx/tool-processes/n8n/config.toml`.
  - `src/n8n_client.rs` — token-authenticated reqwest
    client. `X-N8N-API-KEY` header (not
    `Authorization: Bearer`; n8n's specific convention).
    Helpers: `get_json`, `post_json`, `patch_json`,
    `delete`. Base URL comes from `N8nConfig`.
  - `src/auth_cli/{cli,config_file,status,mod}.rs` —
    slim CLI surface: `aivyx-n8n auth status` (offline
    config check) + `aivyx-n8n auth check` (online
    `/api/v1/workflows?limit=1` ping). Same shape as
    Phase 130's aivyx-notion auth_cli; second data
    point validating the auth_cli lift candidate.
  - `src/main.rs` — binary entry; CLI dispatch + IPC
    loop scaffolding.
  - `src/tools/mod.rs` — placeholder for Tasks 3-12.
- Two new capability bases in `aivyx-capability`:
  - `n8n.read` — gates list/get workflows + executions
    (4 tools).
  - `n8n.write` — gates execute + lifecycle + CRUD
    (6 tools). CEILING_TRUSTED.
- A3 amendment file updated (KNOWN_BASES_COUNT 65 → 67).

### Tasks 3-6 — Read tools

- **Task 3 — `n8n.list_workflows`** — GET `/api/v1/workflows`
  with optional `active` filter, `tags` filter,
  `limit`/`cursor` pagination. Output: array of
  `{id, name, active, tags, created_at, updated_at}`.
- **Task 4 — `n8n.get_workflow`** — GET `/api/v1/workflows/{id}`.
  Output: full workflow with `nodes` + `connections` +
  metadata. Flatten lightly so the LLM sees a
  predictable shape (raw JSON is too heterogeneous).
- **Task 5 — `n8n.list_executions`** — GET
  `/api/v1/executions` with optional `workflowId`,
  `status` (`success`/`error`/`waiting`),
  `limit`/`cursor`. Output: array of
  `{id, workflow_id, status, started_at, stopped_at,
  mode}`.
- **Task 6 — `n8n.get_execution`** — GET
  `/api/v1/executions/{id}?includeData=true`. Output:
  execution metadata + per-node input/output + error
  detail if present.

### Tasks 7-9 — Trigger + lifecycle tools (Trusted-gated)

- **Task 7 — `n8n.execute_workflow`** — POST
  `/api/v1/workflows/{id}/execute` with optional
  `runData` body. Output: execution id + initial
  status. Operators wanting result must call
  `n8n.get_execution` afterward (n8n's API returns
  immediately; runs are async).
- **Task 8 — `n8n.activate_workflow`** — PATCH
  `/api/v1/workflows/{id}` with `{active: true}` (or
  POST to `/api/v1/workflows/{id}/activate` depending
  on n8n version — Task 8 picks the form that works
  against n8n 1.x). Output: `{id, active,
  was_already_active}`.
- **Task 9 — `n8n.deactivate_workflow`** — Mirror of
  Task 8 with `{active: false}`. Output:
  `{id, active, was_already_inactive}`.

### Tasks 10-12 — Workflow CRUD (Trusted-gated; Q1c)

These three are the load-bearing-risky tools per Q1c
honest framing. n8n's workflow JSON is complex; tests
must exercise enough of the shape to catch
malformed-input bugs at the input-validation layer.

- **Task 10 — `n8n.create_workflow`** — POST
  `/api/v1/workflows` with full workflow JSON
  (`{name, nodes, connections, settings, ...}`).
  Input validation: required `name`, required `nodes`
  array (each node must be a JSON object with `name`
  + `type`), required `connections` object. Rest
  passed verbatim.
- **Task 11 — `n8n.update_workflow`** — PATCH
  `/api/v1/workflows/{id}` with partial workflow JSON.
  Only present keys are updated; absent keys preserved.
  Same input-validation posture as create_workflow.
- **Task 12 — `n8n.delete_workflow`** — DELETE
  `/api/v1/workflows/{id}`. Idempotent on 404 → returns
  `was_already_missing: true` (mirrors the
  `calendar.delete_event` / `drive.delete_file` /
  `notion.archive_page` pattern).

### Task 13 — INSTALL.md walkthrough + Phase 131 exit

New INSTALL.md sub-section under "External productivity
integrations (Chapter F)":

**n8n (Phase 131):**
- One-time operator setup: install n8n locally OR point
  at an existing self-hosted instance; create an API
  key in n8n's UI (Settings → API → Create API key);
  write `n8n_base_url` + `n8n_api_key` into
  `~/.aivyx/tool-processes/n8n/config.toml`; verify
  with `aivyx-n8n auth check`.
- Critical operator notes: workflow CRUD is risky
  (Q1c honest framing); n8n version compatibility (the
  crate targets n8n 1.x REST API; bumps in a future
  substrate phase if n8n revs breaking changes); the
  agent's `n8n.execute_workflow` returns immediately
  — operators must call `n8n.get_execution` afterward
  to see results.
- Per-tool capability table + per-role grants.
- Operator-side troubleshooting: 401 (token bad / not
  enabled), 404 on workflows (workflow deleted or
  wrong instance), the `executionMode` quirk
  (`active` workflows triggered via webhook vs `manual`
  triggered via execute_workflow have different result
  shapes).

Phase 131 exit doc: prediction-vs-reality, streak
summary, Q1c risk status at exit, auth_cli lift signal
reinforcement.

## Exit criteria

- [ ] `docs/PHASE_131.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `aivyx-n8n` skeleton + bases — Task 2.
- [ ] Ten n8n tools + tests — Tasks 3-12.
- [ ] INSTALL.md walkthrough + Phase 131 exit doc —
  Task 13.
- [ ] Q1 / Q2 / Q4 resolved pre-Task 2.
- [ ] DESIGN.md streak — predicted HOLD (streak → 22).
- [ ] PRODUCT.md streak — predicted HOLD (streak → 22).
- [ ] `aivyx-core/src/lib.rs` streak — predicted HOLD
  (4 → 5).
- [ ] Zero new workspace dependencies.
- [ ] Test count delta within `+140` to `+200`.
- [ ] Zero clippy warnings.
- [ ] **No live verification as exit criterion** —
  operator-discretionary per Q4a.

## Honest scope risks at sign-off

- **Q1c's workflow CRUD is the load-bearing risk.** A
  malformed `update_workflow` PATCH can silently break
  automations — n8n accepts the partial update and the
  workflow no longer runs correctly because (say) a
  `connections` key references a node id that no
  longer exists. Tests at Tasks 10/11 exercise
  required-field validation; integration testing
  against a real n8n instance is the operator's
  responsibility (Q4a).

- **n8n version compatibility.** The crate targets
  n8n's REST API at v1 (the path is literally
  `/api/v1/...`). n8n's docs are stable across n8n
  1.x. A future major n8n release could ship a v2
  path; that's a Phase 132+ substrate phase if it
  happens.

- **Self-hosted base URL hardening.** Operators may
  point `n8n_base_url` at an HTTP (non-HTTPS) URL on
  localhost or a private network. We don't enforce
  HTTPS; that's an operator decision (loopback HTTP
  is fine; public-internet HTTP would leak the API
  key). INSTALL.md flags this.

- **Execute_workflow returns immediately.** n8n's
  execution model is async — `execute_workflow`
  returns an execution id but the workflow may still
  be running. The tool description tells operators to
  poll `n8n.get_execution` for the result. Honest
  flagging because the LLM might assume synchronous
  semantics.

- **Auth_cli lift posture, second reinforcing data
  point.** Phase 130 validated the case; Phase 131's
  `aivyx-n8n` is nearly identical to `aivyx-notion`'s
  auth_cli surface. Phase 132+ candidate strengthens
  to "load-bearing" if this third token-style auth
  surface lands without revealing additional reasons
  to NOT lift.

- **Nineteenth consecutive deferral of the Channel
  Activation Milestone** if Phase 131 ships without
  taking it. Honest tracking continues.

## Direction after Phase 131

After Phase 131, Phase 132 candidates (sharpened by
the empirical signal from this phase):

1. **Auth CLI substrate lift** — now THREE data points
   (notion + obsidian + n8n) all using slim non-OAuth
   auth_cli shapes. The OAuth-shaped helpers in
   gmail/calendar/drive are awkward to wrap for these;
   the lift becomes a high-leverage substrate phase.
2. **Channel Activation Milestone** — 19th deferral
   if skipped. The deferral count's signal-strength
   keeps growing.
3. **Chapter F #8 — GitHub** (PAT auth; reinforces
   the bearer-token-with-config-URL pattern Phase 131
   establishes).
4. **Release prep (v0.1.0 + installer)** — 6 Chapter F
   integrations + ~75 capability bases is a strong
   release milestone.

## Prediction vs reality

_Populated at Phase 131 exit. Predictions captured at
sign-off: DESIGN.md HOLD → 22; PRODUCT.md HOLD → 22;
lib.rs HOLD → 5; test count `+140` to `+200`; zero
new deps; zero clippy warnings._

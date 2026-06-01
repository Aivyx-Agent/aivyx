# Amendment A3 — Capability Taxonomy Growth

**Date:** 2026-04-17
**Phase:** 22
**Supersedes:** Extends D4 (Capability Taxonomy). No text
removed — additive only.
**Implementing phases:** 11, 14, 21

---

## What changed

D4's v1 active namespace listed 21 scopes across 7 families
(fs, net, shell, llm, memory, channel, audit, config). Phases
11, 14, and 21 added two new families and grew the total to
**23 known bases**. This amendment documents the full current
inventory and the infrastructure-vs-substrate distinction that
governs which bases count against P10's seven-tool cap.

---

## Full capability base inventory (23 bases)

### Substrate bases (D4 original, unchanged)

These are the bases that gate the seven substrate tools from
P10 and the existing platform primitives.

| Family | Base | Qualifier kind | Added |
|---|---|---|---|
| fs | `fs.read` | path glob | Phase 0 |
| fs | `fs.write` | path glob | Phase 0 |
| fs | `fs.delete` | path glob | Phase 0 |
| fs | `fs.metadata` | path glob | Phase 0 |
| net | `net.fetch` | URL prefix | Phase 0 |
| net | `net.post` | URL prefix | Phase 0 |
| net | `net.dns` | domain glob | Phase 0 |
| shell | `shell.exec` | command allowlist | Phase 0 |
| shell | `shell.spawn` | command allowlist | Phase 0 |
| llm | `llm.call` | model name glob | Phase 0 |
| llm | `llm.embed` | model name glob | Phase 0 |
| memory | `memory.read` | session/topic glob | Phase 0 |
| memory | `memory.write` | session/topic glob | Phase 0 |
| memory | `memory.forget` | session/topic glob | Phase 0 |
| channel | `channel.send` | channel name glob | Phase 0 |
| channel | `channel.receive` | channel name glob | Phase 0 |
| audit | `audit.read` | event type glob | Phase 0 |
| config | `config.read` | key glob | Phase 0 |
| config | `config.write` | key glob | Phase 0 |

### Infrastructure bases (post-Phase-0 additions)

Infrastructure bases gate tools the agent uses to manage
itself per P10's infrastructure tool taxonomy. They are not
counted against the seven-tool cap.

| Family | Base | Qualifier kind | Added | Purpose |
|---|---|---|---|---|
| tool | `tool.allowlist` | *(synthetic)* | Phase 11 | Role-level tool allowlist enforcement |
| role | `role.switch` | role name (SimpleGlob) | Phase 14 | Sub-agent role switching per P1 |
| mission | `mission.create` | *(none yet)* | Phase 21 | Mission lifecycle initiation per P2 |
| mission | `mission.gate` | *(none yet)* | Phase 21 | Approval gate resolution per P2 |

### Qualifier dispatch — `QualifierKind`

D4 described four qualifier semantics (path glob, URL prefix,
allowlist, simple glob). The `QualifierKind` enum implements
these as dispatch arms:

```
QualifierKind::of(base, qualifier) -> {
    PathGlob    — base starts with "fs." or qualifier contains "/"
    UrlPrefix   — qualifier contains "://"
    Allowlist   — qualifier contains ","
    SimpleGlob  — fallback for all other qualifiers
}
```

Phase 14's `role.switch` scope uses `SimpleGlob` dispatch for
role-name matching (e.g., `role.switch:researcher`). No new
`QualifierKind` variant was needed — bare identifiers fall
through to exact-string equality as a degenerate glob case.

### Tier ceiling placement for new bases

| Base | Kernel | Trusted | SemiTrusted | Untrusted |
|---|---|---|---|---|
| `tool.allowlist` | granted | granted | granted | denied |
| `role.switch` | granted | granted | denied | denied |
| `mission.create` | granted | granted | conditional (triangle) | denied |
| `mission.gate` | granted | granted | denied | denied |

The tier ceilings are defined in `CEILING_TRUSTED`,
`CEILING_SEMITRUSTED`, and `CEILING_UNTRUSTED` in
`crates/aivyx-capability/src/lib.rs`. `CEILING_KERNEL` is
derived from `KNOWN_BASES` iteration (grants everything).

---

## The three-tier tool taxonomy

PRODUCT.md P10 establishes a three-tier taxonomy that this
amendment makes concrete:

1. **Substrate tools** (7, capped by P10): `fs.read`,
   `fs.write`, `memory.read`, `memory.write`, `memory.forget`,
   `shell.exec`, `web.fetch`. Operator-facing primitives.

2. **Infrastructure tools** (4 and growing): `tool.allowlist`,
   `role.switch`, `mission.create`, `mission.gate`. Machinery
   the agent uses to manage itself. May grow without P10
   amendment as G3 (reflection), G4 (sub-agents), and G5
   (scheduled execution) require.

3. **Third-party tools** (0 today): implemented against the
   SDK contract from P11, distributed per P12. Not yet shipped.

The `KNOWN_BASES` array in `aivyx-capability/src/lib.rs` is
the single registry for all three tiers. `Scope::parse`
rejects any base not in the array at parse time.

---

## Traceability

| Phase | What was added | Commit |
|---|---|---|
| Phase 11 | `tool.allowlist` (role primitive) | `16422e2` |
| Phase 14 | `role.switch` (sub-agent role-switching) | `0d94d32` |
| Phase 21 | `mission.create`, `mission.gate` (mission primitive) | `05cc349` |

---

## Phase 54 addendum — current scope-base count (2026-05-12)

> *Added at Phase 54 exit during the Chapter A docs sweep. The
> traceability table above stopped at Phase 21 and the
> "12 → 23" headline figure has been wrong for ~30 phases.
> This addendum brings the count current.*

`aivyx-capability::KNOWN_BASES.len() = 43` as of Phase 54 exit.

### What changed since the table above

| Phase | Bases added | Provenance |
|---|---|---|
| Phase 26 | `schedule.create`, `schedule.list`, `schedule.delete`, `schedule.update` | Scheduled execution G5 |
| Phase 27 | `webhook.create`, `webhook.list`, `webhook.delete`, `file_watch.create`, `file_watch.list`, `file_watch.delete` | Webhook + file-watch triggers G5 |
| Phase 23 | `mcp.call` | MCP client adapter |
| Phase 28 | `mission.list`, `mission.status` | Mission read-only inspection (P2 deferral closure) |
| Phase 29 | `reflection.propose`, `reflection.apply` | Reflection loop G3/P8 |
| Phase 30 | `role.update` | Runtime role mutation P8 completion |
| Phase 36 | `ollama.list`, `ollama.show`, `ollama.pull` | Ollama model management |
| Phase 37 | `net.post` | Web post primitive (substrate cap raised from 7→8, A5) |
| Phase 42 | `memory.gc` | Memory garbage collection |

### Current full enumeration

Substrate (8, capped by P10 + A5):
- `fs.read`, `fs.write`, `memory.read`, `memory.write`,
  `memory.forget`, `shell.exec`, `web.fetch`, `web.post`

Other operator-facing scopes (8):
- `fs.delete`, `fs.metadata`, `net.fetch`, `net.dns`,
  `shell.spawn`, `llm.call`, `llm.embed`, `memory.gc`

Channel / audit / config (5):
- `channel.send`, `channel.receive`, `audit.read`,
  `config.read`, `config.write`

Infrastructure tools (22, allowed to grow per P10's three-tier
taxonomy):
- Role primitive: `tool.allowlist`, `role.switch`, `role.update`
- Mission: `mission.create`, `mission.gate`, `mission.list`,
  `mission.status`
- Scheduling: `schedule.create`, `schedule.list`,
  `schedule.delete`, `schedule.update`
- Triggers: `webhook.create`, `webhook.list`, `webhook.delete`,
  `file_watch.create`, `file_watch.list`, `file_watch.delete`
- MCP: `mcp.call`
- Reflection: `reflection.propose`, `reflection.apply`
- Ollama management: `ollama.list`, `ollama.show`, `ollama.pull`

(`turn.history` from Phase 28 reuses `audit.read` rather than
declaring its own base — recorded here so a future reader
doesn't go hunting for it in `KNOWN_BASES`.)

Total: 8 + 8 + 5 + 22 = 43.

### Drift posture

The traceability table above is no longer maintained
per-base — at ~3 bases/phase it became more noise than signal.
The single source of truth is `KNOWN_BASES` in
`aivyx-capability/src/lib.rs`. This addendum is the
backwards-looking reconciliation; future amendments need not
duplicate per-base provenance.

---

## Phase 113 addendum — current scope-base count (2026-05-28)

> *Added at Phase 113 exit during the operator-surface-polish
> deferral-cleanup phase. The Phase 54 addendum (above)
> caught up to 43 bases; six more landed across the Persona,
> Reach, and Chapter D work between Phase 56 and Phase 110.
> This addendum brings the count current and pins the
> auditor-readable provenance.*

`aivyx-capability::KNOWN_BASES.len() = 49` as of Phase 113 exit.

### What changed since the Phase 54 addendum

| Phase | Bases added | Provenance |
|---|---|---|
| Phase 59 | `persona.propose` | Persona Foundation P14 — operator-side proposal-write gate distinct from `reflection.propose` |
| Phase 62 | `notify.send` | Reach: Agent-Initiated Outbound Notifications |
| Phase 109 | `git.read` | A12 — `git.status` + `git.diff` substrate tools (one base, two tools) |
| Phase 110 | `skills.propose` | Skills Auto-Creation — distinct from `persona.propose` so a role granting Persona-edit rights doesn't implicitly grant skill-draft rights |
| Phase 110 | `skills.list` | Read-only enumeration of approved skill set |
| Phase 110 | `skills.invoke` | On-demand procedure rendering of one named skill |

(Phase 113 itself ships no new bases — the deferral-cleanup
phase touches only operator-surface flags + TOML loading + this
addendum. The `[skills.auto_propose]` TOML config introduced in
Phase 113 reuses the existing `persona.propose` and
`skills.propose` bases for the auto-proposer's chain-write
path; no new capability gate.)

## Phase 123 addendum — Chapter F #1 Gmail (2026-05-30)

> *Added at Phase 123 exit. Chapter F (External Productivity
> Integrations) opens with Gmail as the first integration;
> per P10/P11/P12 it ships as a third-party tool process,
> NOT as substrate. The three new bases below gate the four
> Gmail tools (`gmail.search`, `gmail.read`, `gmail.draft`,
> `gmail.send`) that the `aivyx-gmail` tool process registers.
> All three Trusted-tier-only by default (mirrors
> `shell.exec` / `notify.send` gating per Phase 62 Q2(a)).*

| Phase | Bases added | Provenance |
|---|---|---|
| Phase 123 | `email.read` | Chapter F #1 — `gmail.search` + `gmail.read` (read-only inbox + message access via the Gmail third-party tool process) |
| Phase 123 | `email.write` | Chapter F #1 — `gmail.draft` (creates a Gmail draft; safe write — requires explicit Gmail-UI send) |
| Phase 123 | `email.send` | Chapter F #1 — `gmail.send` (direct send; Trusted-tier only at the ceiling level, matching `shell.exec` / `notify.send`) |

## Phase 125 addendum — Chapter G #1 personal assistant tool bundle (2026-05-31)

> *Added at Phase 125 exit. Chapter G (Operator-Facing Personal
> Assistant Capabilities) opens with the aivyx-toolkit bundle
> as the first integration. Per P10/P11/P12 the entire bundle
> ships as a single third-party tool process registering eight
> tools across three categories. The five new bases below gate
> those tools. All five Trusted-tier-only by default — same
> gating pattern as `email.*` and `notify.send` per Phase 62
> Q2(a): personal-assistant tools shouldn't be reachable from
> remote channels without explicit role grant.*

| Phase | Bases added | Provenance |
|---|---|---|
| Phase 125 | `web.search` | Chapter G #1 — `web.search` tool (Brave Search API; operator-provided key; distinct from `net.fetch` because the search API is a credentialed external service, not generic HTTP fetch) |
| Phase 125 | `task.read` | Chapter G #1 — `task.list` (lightweight TODO listing) |
| Phase 125 | `task.write` | Chapter G #1 — `task.create`, `task.complete`, `task.delete` (TODO CRUD; separate from `task.read` so a read-only role can browse tasks without write authority) |
| Phase 125 | `health.read` | Chapter G #1 — `health.check.list`, `health.check.recent_changes` (URL monitor state inspection) |
| Phase 125 | `health.write` | Chapter G #1 — `health.check.add` (register new URL watcher for the polling loop) |

## Phase 130 addendum (Task 2) — Chapter F #5 Notion (2026-06-01)

> *Added at Phase 130 Task 2 (Notion crate skeleton).
> Chapter F's fifth integration — Notion via the
> `aivyx-notion` third-party tool process. First non-Google
> + first non-OAuth Chapter F integration; uses Notion's
> Integration token (bearer-token) auth. Per Phase 130 Q1a,
> seven tools ship: `notion.search`, `notion.get_page`,
> `notion.list_database`, `notion.create_page`,
> `notion.append_blocks`, `notion.update_page_properties`,
> `notion.archive_page`. Two new bases gate them.
>
> The Obsidian bases (`obsidian.read`, `obsidian.write`)
> land at Phase 130 Task 10 and will extend this addendum
> with an additional row.*

| Phase | Bases added | Provenance |
|---|---|---|
| Phase 130 | `notion.read` | Chapter F #5 — `notion.search`, `notion.get_page`, `notion.list_database` against the operator's shared Notion content |
| Phase 130 | `notion.write` | Chapter F #5 — `notion.create_page`, `notion.append_blocks`, `notion.update_page_properties`, `notion.archive_page` (full page-lifecycle mutation; Trusted-tier-only at the ceiling level, matching `email.send` / `calendar.write` / `drive.write`) |

### Current enumeration after Task 2 (63 bases — superseded by Task 10 enumeration below)

### Phase 130 Task 10 addendum — Chapter F #6 Obsidian (2026-06-01)

> *Added at Phase 130 Task 10 (Obsidian skeleton).
> Chapter F's sixth integration — Obsidian vault via the
> `aivyx-obsidian` third-party tool process. First Chapter F
> integration with no external API; operates on filesystem
> reads/writes under a configured vault directory with
> load-bearing path-traversal protection. Per Phase 130 Q2a,
> six tools ship: `obsidian.search`, `obsidian.get_note`,
> `obsidian.list_folder`, `obsidian.create_note`,
> `obsidian.update_note`, `obsidian.delete_note`. Two new
> bases gate them.*

| Phase | Bases added | Provenance |
|---|---|---|
| Phase 130 | `obsidian.read` | Chapter F #6 — `obsidian.search`, `obsidian.get_note`, `obsidian.list_folder` against the operator's configured vault |
| Phase 130 | `obsidian.write` | Chapter F #6 — `obsidian.create_note`, `obsidian.update_note`, `obsidian.delete_note` (Trusted-tier-only at the ceiling level, matching the Chapter F write-tool gating pattern) |

### Current enumeration after Phase 130 (65 bases — superseded by Phase 131 enumeration below)

### Phase 131 addendum — Chapter F #7 n8n (2026-06-01)

> *Added at Phase 131 Task 2 (n8n skeleton). Chapter F's
> seventh integration — n8n workflow automation via the
> `aivyx-n8n` third-party tool process. First Chapter F
> integration with an **operator-supplied base URL**
> (self-hosted n8n instances are the norm); the crate
> constructs every request as
> `{n8n_base_url}/api/v1/<resource>` and authenticates with
> the n8n-specific `X-N8N-API-KEY` header (not Bearer). Per
> Phase 131 Q1c (operator-picked over the Recommended
> Q1b 7-tool default), ten tools ship: read surface
> `n8n.list_workflows`, `n8n.get_workflow`,
> `n8n.list_executions`, `n8n.get_execution`; lifecycle
> writes `n8n.execute_workflow`, `n8n.activate_workflow`,
> `n8n.deactivate_workflow`; and CRUD writes
> `n8n.create_workflow`, `n8n.update_workflow`,
> `n8n.delete_workflow`. The CRUD writes carry the
> highest blast radius of any Chapter F surface to date —
> a workflow definition can call arbitrary HTTP, mutate
> the operator's other services, or schedule recurring
> side effects — and ride the same Trusted-tier-only
> ceiling that every other Chapter F write-base uses.
> Two new bases gate the ten tools.*

| Phase | Bases added | Provenance |
|---|---|---|
| Phase 131 | `n8n.read` | Chapter F #7 — `n8n.list_workflows`, `n8n.get_workflow`, `n8n.list_executions`, `n8n.get_execution` against the operator's self-hosted n8n instance |
| Phase 131 | `n8n.write` | Chapter F #7 — `n8n.execute_workflow`, `n8n.activate_workflow`, `n8n.deactivate_workflow`, `n8n.create_workflow`, `n8n.update_workflow`, `n8n.delete_workflow` (Trusted-tier-only at the ceiling level, matching the Chapter F write-tool gating pattern; the CRUD trio carries definition-write blast radius that operators may want to attenuate further with role-level `capability_scopes`) |

### Current full enumeration after Phase 131 (67 bases)

Substrate-facing operator scopes (16):
- `fs.read`, `fs.write`, `fs.delete`, `fs.metadata`
- `net.fetch`, `net.post`, `net.dns`
- `shell.exec`, `shell.spawn`
- `llm.call`, `llm.embed`
- `memory.read`, `memory.write`, `memory.forget`, `memory.gc`
- `git.read`

Channel / audit / config (5)

Infrastructure (28)

Third-party tool process scopes (18):
- Email (Chapter F #1, Phase 123): `email.read`, `email.write`,
  `email.send`
- Personal assistant tool bundle (Chapter G #1, Phase 125):
  `web.search`, `task.read`, `task.write`, `health.read`,
  `health.write`
- Calendar (Chapter F #2, Phase 128): `calendar.read`,
  `calendar.write`
- Drive (Chapter F #3, Phase 129): `drive.read`, `drive.write`
- Notion (Chapter F #5, Phase 130 Task 2): `notion.read`,
  `notion.write`
- Obsidian (Chapter F #6, Phase 130 Task 10): `obsidian.read`,
  `obsidian.write`
- n8n (Chapter F #7, Phase 131 Task 2): `n8n.read`,
  `n8n.write`

Total: 16 + 5 + 28 + 18 = 67.

## Phase 129 addendum — Chapter F #3 Google Drive (2026-06-01)

> *Added at Phase 129 exit. Chapter F's third integration —
> Google Drive via the `aivyx-drive` third-party tool
> process. Per Phase 129 Q2b (operator-picked over Q2a's
> 5-tool default), seven tools ship: `drive.search`,
> `drive.get_metadata`, `drive.list_folder`,
> `drive.create_folder`, `drive.download_file`,
> `drive.upload_file`, `drive.delete_file`. Two new bases
> gate them: `drive.read` for the four read tools and
> `drive.write` for the three write tools. Both Trusted-tier-
> only by default — same gating pattern as `email.*` /
> `calendar.*` per the Phase 62 Q2(a) precedent.*

| Phase | Bases added | Provenance |
|---|---|---|
| Phase 129 | `drive.read` | Chapter F #3 — `drive.search`, `drive.get_metadata`, `drive.list_folder`, `drive.download_file` against the operator's authorized Google Drive |
| Phase 129 | `drive.write` | Chapter F #3 — `drive.create_folder`, `drive.upload_file`, `drive.delete_file` (full file-lifecycle mutation; Trusted-tier-only at the ceiling level, matching `email.send` / `calendar.write` / `shell.exec`) |

### Current full enumeration (61 bases)

Substrate-facing operator scopes (16):
- `fs.read`, `fs.write`, `fs.delete`, `fs.metadata`
- `net.fetch`, `net.post`, `net.dns`
- `shell.exec`, `shell.spawn`
- `llm.call`, `llm.embed`
- `memory.read`, `memory.write`, `memory.forget`, `memory.gc`
- `git.read`

Channel / audit / config (5):
- `channel.send`, `channel.receive`, `audit.read`,
  `config.read`, `config.write`

Infrastructure (28):
- Role primitive: `tool.allowlist`, `role.switch`, `role.update`
- Mission: `mission.create`, `mission.gate`, `mission.list`,
  `mission.status`
- Scheduling: `schedule.create`, `schedule.list`,
  `schedule.delete`, `schedule.update`
- Triggers: `webhook.create`, `webhook.list`, `webhook.delete`,
  `file_watch.create`, `file_watch.list`, `file_watch.delete`
- MCP: `mcp.call`
- Reflection: `reflection.propose`, `reflection.apply`
- Persona / Skills: `persona.propose`, `skills.propose`,
  `skills.list`, `skills.invoke`
- Notify: `notify.send`
- Ollama management: `ollama.list`, `ollama.show`, `ollama.pull`

Third-party tool process scopes (12):
- Email (Chapter F #1, Phase 123): `email.read`, `email.write`,
  `email.send`
- Personal assistant tool bundle (Chapter G #1, Phase 125):
  `web.search`, `task.read`, `task.write`, `health.read`,
  `health.write`
- Calendar (Chapter F #2, Phase 128): `calendar.read`,
  `calendar.write`
- Drive (Chapter F #3, Phase 129): `drive.read`, `drive.write`

Total: 16 + 5 + 28 + 12 = 61.

### Verification

A unit test in `aivyx-capability/src/lib.rs` pins the
count so this addendum and the runtime stay in sync; any
future base added without an accompanying addendum bump
surfaces as a test failure rather than silent drift.

## Phase 128 addendum — Chapter F #2 Google Calendar (2026-06-01)

> *Added at Phase 128 exit. Chapter F's second integration —
> Google Calendar via the `aivyx-calendar` third-party tool
> process. Per Phase 128 Q3b (operator-picked over Q3a's
> 4-tool default), five tools ship: `calendar.list_events`,
> `calendar.get_event`, `calendar.create_event`,
> `calendar.update_event`, `calendar.delete_event`. Two new
> bases gate them: `calendar.read` for the two read tools
> and `calendar.write` for the three write tools. Both
> Trusted-tier-only by default — same gating pattern as
> `email.*` per the Phase 62 Q2(a) precedent.*

| Phase | Bases added | Provenance |
|---|---|---|
| Phase 128 | `calendar.read` | Chapter F #2 — `calendar.list_events` (range query) + `calendar.get_event` (single fetch by ID) against the operator's authorized Google Calendar(s) |
| Phase 128 | `calendar.write` | Chapter F #2 — `calendar.create_event`, `calendar.update_event`, `calendar.delete_event` (full event-lifecycle mutation; Trusted-tier-only at the ceiling level, matching `email.send` / `shell.exec` / `notify.send`) |

### Current full enumeration (59 bases)

Substrate-facing operator scopes (16):
- `fs.read`, `fs.write`, `fs.delete`, `fs.metadata`
- `net.fetch`, `net.post`, `net.dns`
- `shell.exec`, `shell.spawn`
- `llm.call`, `llm.embed`
- `memory.read`, `memory.write`, `memory.forget`, `memory.gc`
- `git.read`

Channel / audit / config (5):
- `channel.send`, `channel.receive`, `audit.read`,
  `config.read`, `config.write`

Infrastructure (28):
- Role primitive: `tool.allowlist`, `role.switch`, `role.update`
- Mission: `mission.create`, `mission.gate`, `mission.list`,
  `mission.status`
- Scheduling: `schedule.create`, `schedule.list`,
  `schedule.delete`, `schedule.update`
- Triggers: `webhook.create`, `webhook.list`, `webhook.delete`,
  `file_watch.create`, `file_watch.list`, `file_watch.delete`
- MCP: `mcp.call`
- Reflection: `reflection.propose`, `reflection.apply`
- Persona / Skills: `persona.propose`, `skills.propose`,
  `skills.list`, `skills.invoke`
- Notify: `notify.send`
- Ollama management: `ollama.list`, `ollama.show`, `ollama.pull`

Third-party tool process scopes (10):
- Email (Chapter F #1, Phase 123): `email.read`, `email.write`,
  `email.send`
- Personal assistant tool bundle (Chapter G #1, Phase 125):
  `web.search`, `task.read`, `task.write`, `health.read`,
  `health.write`
- Calendar (Chapter F #2, Phase 128): `calendar.read`,
  `calendar.write`

Total: 16 + 5 + 28 + 10 = 59.

### Verification

A unit test in `aivyx-capability/src/lib.rs` pins the
count so this addendum and the runtime stay in sync; any
future base added without an accompanying addendum bump
surfaces as a test failure rather than silent drift.

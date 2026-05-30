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

### Current full enumeration (52 bases)

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

Third-party tool process scopes (3):
- Email (Chapter F #1, Phase 123): `email.read`, `email.write`,
  `email.send`

Total: 16 + 5 + 28 + 3 = 52.

### Verification

A unit test in `aivyx-capability/src/lib.rs` pins the
count so this addendum and the runtime stay in sync; any
future base added without an accompanying addendum bump
surfaces as a test failure rather than silent drift.

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

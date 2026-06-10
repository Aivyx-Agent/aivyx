# Nonagon — Multi-Agent Teams (Chapter J)

> **Status:** design contract. Chapter J is not yet opened; this is the
> spec the phases scaffold from.
>
> The **Nonagon** is Aivyx's multi-agent capability: a *lead* agent that
> convenes up to **9 attenuated specialists**, decomposes a mission into
> a DAG, delegates, verifies, and synthesizes — all inside the single
> daemon, on the single HMAC audit chain. The engine is **free core**;
> **verticals customise the team** (the kitchen pack ships a Back-of-House
> Nonagon, the factory pack a different one).
>
> Lineage: ported from the pre-rebuild archive's `aivyx-team` crate
> (~10.7k LOC), re-grounded on the new core's primitives. See
> [`VERTICAL_PACKS.md`](VERTICAL_PACKS.md) for the pack model a team config
> plugs into.

---

## 1. Why this preserves the single-agent ethos

Aivyx is deliberately one daemon, one operator-facing agent. The Nonagon
does **not** break that: there is still **one lead** the operator talks
to. Specialists are **ephemeral, subordinate, attenuated** instances the
lead convenes *within the same process*, sharing the lead's audit chain
and bounded by the lead's authority. It's "one agent that can convene a
team," never "a swarm of independent peers."

## 2. The key architectural insight

In the archive, the Nonagon is **not an agent-loop rewrite** — it is a
`SpecialistPool` plus a set of **delegation tools** layered on the
ordinary single-agent turn loop. The lead is a normal `ConcreteAgent`
whose *tools* are `delegate_task` / `spawn_specialist` / `query_agent` /
`synthesize` / `verify`. Calling such a tool spins up an attenuated
specialist, runs *its* turn loop, and returns the result.

**Consequence:** the new core's turn loop, planner, `ToolRegistry`,
capability attenuation, trust tiers, confirm-first, and HMAC audit are all
**reused unchanged**. Nonagon = (a) a pool that builds attenuated
specialists, (b) ~8 tools, (c) a message bus.

```
        operator / autonomous loop
                 │ mission
        ┌────────▼─────────┐
        │   Lead agent     │  ConcreteAgent + orchestration tools.
        │ decompose →      │  Never executes domain work directly:
        │ delegate →       │  it plans, delegates, verifies, synthesizes.
        │ verify →         │
        │ synthesize       │
        └───┬───┬───┬──────┘
            │   │   │   delegate_task / spawn_specialist
       ┌────▼┐ ┌▼──┐ ┌▼────┐    each = ephemeral ConcreteAgent whose
       │Spec1│ │S2 │ │ …≤9 │    CapabilitySet = (declared ∩ lead's caps)
       └─────┘ └───┘ └─────┘    + shared MessageBus (tokio broadcast)
                 │ results
            HMAC audit chain  (lead + every specialist tool call)
```

## 3. The team config — what verticals customise

A team is declarative TOML. Each **member maps onto a new-core Role**
(`system_prompt` + `tool_allowlist` + `capability_scopes` +
`trust_ceiling`) — so a member is a persona + a scoped role, reusing
machinery we already ship.

```toml
[team]
name = "kitchen-boh"
lead = "aria"

[[team.member]]
name = "aria"
role = "BOH Manager"
soul = "You coordinate back-of-house. You never touch stock directly —
        you decompose the shift's goals, delegate, verify, and report."
capability_scopes = ["kitchen.read", "kitchen.write", "kitchen.order.send", "kitchen.haccp.log"]
trust_ceiling = "Trusted"

[[team.member]]
name = "haccp"
role = "Food-Safety / Compliance"
soul = "You log temperature checks and flag out-of-limit readings with corrective actions."
capability_scopes = ["kitchen.haccp.log"]   # attenuated: HACCP can ONLY log
trust_ceiling = "Trusted"

[team.dialogue]
enable_peer_dialogue   = true
max_messages_per_turn  = 10
max_spawned_specialists = 5
delegation_timeout_secs = 600
message_bus_capacity    = 64
```

- **Free core** ships a **default general-purpose Nonagon** (the 9 roles
  in §5). `aivyx team run "<mission>"` works out of the box.
- **A vertical pack overrides it** with a customised roster — its
  `TeamConfig` ships inside the pack, exactly as its tools/template/skills
  do. This is the commercial surface: the *engine* is free; the
  *domain-expert team* is the product.

## 4. The safety model — NT-02

**Invariant NT-02: a specialist can never exceed its lead.** The archive's
`attenuate_for_member` collapses to a few lines on the new core because
`Scope` / `CapabilitySet` already do the work:

```rust
/// Specialist caps = declared scopes that the lead actually grants.
fn attenuate_for_member(lead: &CapabilitySet, declared: &[Scope]) -> CapabilitySet {
    declared.iter().filter(|s| lead.grants(s)).cloned().collect()   // specialist ⊆ lead
}
// trust: specialist.tier = min(member.trust_ceiling, lead.trust_tier)
```

So the HACCP specialist *physically cannot* send a PO or read inventory —
its set is `{kitchen.haccp.log}` and nothing the lead grants widens it.
Confirm-first gates (`po.send`) still fire at whichever specialist holds
`kitchen.order.send`. **Least privilege per specialist, enforced by the
same primitive the daemon already uses.**

## 5. The default 9-role roster (free core)

Ported from the archive's `NONAGON_ROLES` as new-core Roles — each a
`{ name, role, soul, tool_allowlist, capability_scopes, trust_ceiling }`:

`coordinator` (Lead) · `researcher` · `analyst` · `coder` · `writer` ·
`reviewer` · `planner` · `ops` · `archivist`.

The coordinator's soul forbids direct execution: *"You delegate, verify,
and synthesize; you never execute tasks directly."*

## 6. The tool set (all free core)

| Tool | Purpose |
|---|---|
| `decompose_task` | lead → a `MissionPlan` (a **DAG** of `Execute`/`Delegate`/`Reflect`/`Gate` steps) |
| `delegate_task` / `pipeline_delegate` | run a specialist (or a sequential chain) on a step |
| `spawn_specialist` | ephemeral specialist mid-mission (capped, attenuated) for unforeseen needs |
| `query_agent` | quick follow-up question to a specialist |
| `synthesize_results` | weave specialist outputs into one deliverable |
| `verify_output` | the Reflect/Gate quality check before a step proceeds |
| `send_message` / `read_messages` | the `MessageBus` (capped peer dialogue) |

## 7. Execution flow (DAG, future-proofed)

The `MissionPlan` is a **DAG from day one** (steps + dependencies), and
the `TeamRuntime` executes **independent branches concurrently** (tokio).
Early versions may resolve simple/linear plans, but the structure is
parallel-ready — adding wide parallelism is a *flip*, not a redesign.

1. Operator (or the autonomous loop) hands the **lead** a mission.
2. Lead `decompose`s → a `MissionPlan` DAG.
3. Lead `delegate`s ready steps; the `SpecialistPool` constructs attenuated
   specialists which run their own turn loops and return via the bus.
4. `Reflect`/`Gate` steps `verify` quality before dependents run.
5. Lead `synthesize`s the branch outputs into the final deliverable.
6. Every delegation + specialist tool call lands on the **HMAC chain**.

## 8. Free vs. commercial line

- **Free (open core):** the whole engine + the default general team —
  single-daemon, in-process, ≤9 specialists, DAG missions, message bus.
- **Commercial (vertical packs):** the **customised team configs** (kitchen
  BOH, factory, …) ship inside the private verticals. Leaves room for a
  later paid **Fleet Manager** (orchestrating many teams across nodes) as a
  Tier-2 surface — out of Chapter J scope.

## 9. Worked example — the kitchen BOH Nonagon

> **Mission (overnight loop):** *"Run end-of-day BOH close."*
>
> **Aria (lead)** decomposes → delegates: **Stocktake** counts →
> **Inventory** computes low-stock → **Purchasing** drafts per-supplier POs
> (holds `kitchen.order.send`, so `po.send` stays confirm-first) →
> **HACCP** logs the final fridge round (can *only* `haccp.log`) → Aria
> **verifies** + **synthesizes** the close-down report.
>
> One engine, a catering-shaped team; every specialist least-privileged;
> the whole run on the audit chain.

## 10. What's reused vs. new

**Reused unchanged:** `ConcreteAgent` + the turn loop, `ToolRegistry`,
`CapabilitySet`/`Scope` (attenuation), Roles/Personas (team members), trust
tiers + confirm-first (specialist gates), the HMAC audit chain, the
autonomous loop (a team mission can be a loop story), the TUI Missions
panel (mockup → live).

**New (the `aivyx-team` crate):** `TeamConfig`, the 9-role roster,
`SpecialistPool`, the delegation/message/orchestration tools, `MessageBus`,
`MissionPlan` (DAG) + `TeamRuntime`, and the `aivyx team run` command.

---

## 11. Phase plan

*Test bands priced by dense components, not by family label.*

| Phase | Goal | Deps | Tests |
|---|---|---|---|
| **J.1 Foundation** ✅ | `aivyx-team` crate; `TeamConfig` schema + validation; the 9 default roles; the `attenuate_for_member` port + **NT-02** invariant tests (incl. qualified-path attenuation) | — | **23 shipped** |
| **J.2 Pool + delegation** ✅ | `SpecialistFactory` builds attenuated specialists; `SpecialistChannel` + `SpecialistPool::run` execute a sub-turn (trust-floored); `delegate_task`/`query_agent` tools over the new `team.delegate` scope. (`collect_results` deferred to J.4 where parallel delegation makes it meaningful.) | J.1 | **15 shipped** (J.2.1–3) |
| **J.3 Message bus** ✅ | `MessageBus` (bounded broadcast, fan-out, lag/backpressure) + `send_message`/`read_message` tools over the new `team.message` scope + dialogue caps (peer-dialogue toggle, per-turn budget). Roster wiring deferred to J.5. | J.2 | **12 shipped** (J.3.1–2) |
| **J.4 Mission DAG** ⭐ ✅ | `MissionPlan` DAG (cycle detection, ready-set); `TeamRuntime` runs independent branches concurrently (`join_all`); `decompose_task`/`synthesize_results`/`verify_output` over the existing `team.delegate` scope (no new base). `collect_results` lands as the `MissionReport`. | J.2, J.3 | **31 shipped** (J.4.1–3) |
| **J.5 CLI + audit + loop** ✅ | `aivyx team run "<mission>"` (in-process) + `aivyx team roster`; the roster wiring (`TeamAssembly`, per-member dialogue tools via `SpecialistFactory::with_dialogue`, `team.message` on every default role); specialist sub-turns land on the same persistent HMAC `AuditHook`. (Loop integration deferred — optional.) | J.4 | **17 shipped** (J.5.1–2) |
| **J.6 Kitchen Nonagon** 💰 | "pack supplies a `TeamConfig`" wiring + the kitchen BOH roster + the overnight-close mission | J.5 | ~20–30 |
| **J.7 TUI Missions/Fleet** | wire live team/mission state into the TUI panel (the mockup → real) | J.5 | ~15–25 |

```
J.1 ─▶ J.2 ─▶ J.3 ─▶ J.4 ─▶ J.5 ─┬─▶ J.6  (kitchen team)
                                 └─▶ J.7  (TUI)
```

**Chapter total: ~180–260 tests, 7 phases.** J.6 and J.7 parallelise once
J.5 lands.

## 12. Definition of done

- Free core ships a working general Nonagon (`aivyx team run`, 9 roles,
  attenuated delegation, DAG missions, message bus) — single-daemon, on
  the HMAC chain.
- A vertical pack can ship a customised team; **kitchen's BOH Nonagon is
  the proof**.
- Every specialist is least-privileged (`⊆ lead`); confirm-first gates fire
  at the holder.
- The single-agent ethos is intact: one lead, ephemeral attenuated
  specialists, one audit chain.

## 13. Locked decisions

1. **Crate** — a new free-core `aivyx-team` crate (not a `aivyx-channel` module).
2. **Specialist = Role** — members reuse the existing Role machinery.
3. **All 9 default roles** ship in the free core.
4. **Future-proofed** — `MissionPlan` is a DAG and the runtime is
   parallel-capable from the start.

# Aivyx Vertical Packs

How to specialize the *one* Aivyx agent to a domain **without forking
the substrate**. A vertical pack is configuration + a tool bundle, not a
codebase branch — so every pack inherits all future foundation work for
free. This document defines the pack format and works through the first
example: a **Kitchen / Back-of-House (BOH)** pack over the existing
KitchenDB.

> Status: in progress. The `aivyx-kitchen` pack crate is **live** and ships
> the **BOH Nonagon team** — the customised `TeamConfig` (Aria + four
> least-privileged specialists over the `kitchen.*` scopes) + the
> overnight-close `MissionPlan`, as a Rust constructor and a committed TOML
> asset (Chapter **J.6**; loaded by `aivyx team run --config <path>`). The
> `kitchen.*` scope bases are registered. **Next** for the same crate: the
> KitchenDB RPC client + the read/compute/write `aivyx_core::Tool` impls
> (the domain tool surface §3.2), wired into the team's `base_tools` so the
> specialists *act* on the DB, not just plan.

---

## 1. Why packs, not forks

Aivyx is **one** agent, shaped by Profile (P13) + Persona (P14) + Roles
+ Skills + Tools + MCP + capability scopes + trust tiers. A "Chef agent"
is that same agent, configured — never a second codebase. Forking would
mean porting every hardened foundation fix forever; a pack **inherits**
them. For an *ecosystem* ("Aivyx core + a marketplace of vertical
packs"), this is the load-bearing decision.

`aivyx-core` / `aivyx-capability` / the daemon / the audit chain stay
**domain-neutral**. The domain lives only in the pack.

## 2. The pack format

A vertical pack is up to six things, each riding an existing primitive:

| Component | Primitive | New code? |
|---|---|---|
| **Template** | `aivyx init --template <name>` (Phase 66) → seeds Profile + default Role | config only |
| **Toolkit crate** | a bundled multi-tool process, same shape as `aivyx-toolkit`/`aivyx-gmail` (Chapter F/G), reusing `aivyx_tool::multi_harness` | new sibling crate |
| **Scopes + gate policy** | capability scope bases (additive to `aivyx-capability` `KNOWN_BASES`) + trust ceiling + gates | additive bases |
| **Team (Nonagon)** | a customised `aivyx_team::TeamConfig` — a lead + ≤9 least-privileged specialists over the pack's scopes — loaded by `aivyx team run --config <pack.toml>` (Chapter J) | config (TOML) |
| **Skills bundle** | starter conversationally-taught `LearnedSkill`s | config only |
| **Integrations** | `aivyx connect` tool-processes / MCP servers (Chapter F) | config only |

The **Team** component is what makes a pack a *force multiplier*: the same
free engine, shaped into a domain expert crew. The kitchen pack's BOH Nonagon
(Aria + stocktake / inventory / purchasing / HACCP) is the worked example —
see [`NONAGON.md`](NONAGON.md) §9 and `crates/aivyx-kitchen`.

The only Rust that changes outside the new crate is **additive scope
bases** in `aivyx-capability` (exactly how `web.search`, `gmail.*`,
`task.*` were added) — never a substrate fork.

### Where a pack lives
For the spike, the toolkit crate is a workspace member (like
`aivyx-toolkit`). The ecosystem question — packs as separate
repos/registry artifacts vs. in-tree — is deferred; the crate boundary
keeps either option open.

---

## 3. Worked example: the Kitchen / BOH pack

### 3.1 The integration model — KitchenDB is the system of record

The existing **KitchenDB** (Supabase/Postgres, RPC-first) is the source
of truth. The agent does **not** reimplement the domain — it calls the
existing `public.*` RPCs. Two front-ends coexist on one DB:

```
        KitchenDB  (Supabase/Postgres — system of record, RPC API)
          ▲                                   ▲
          │ RPCs                              │ RPCs (read/write, gated)
   Kitchen OS (Flutter)              aivyx-kitchen toolkit
   rich GUI for managers      ←→     Aivyx agent: chat / voice / loop / TUI
```

The Flutter app is the *look-at* surface (dashboards, bulk entry); the
agent is the *talk-to* surface (voice on the line, overnight reorder
loop, HACCP logging, compliance export).

> **Dedicated toolkit, not raw `postgres` MCP.** A raw SQL `execute`
> tool would bypass the DB's RLS, the `_v2` API contract, and domain
> safety — the wrong amount of power for a kitchen. The toolkit wraps
> RPCs as *typed, individually-scopable, individually-gateable* tools.
> Raw postgres MCP stays available for operator debugging behind a
> high-trust tier.

### 3.2 RPC → tool surface (from the real KitchenDB)

The stable contract is the `public.get_*` / `public.*_v2` functions.
Mapping (✓ = Phase-1 read-only spike; ⚑ = gated; ⚑⚑ = double-gated):

| Agent tool | KitchenDB RPC | Gate |
|---|---|---|
| `inventory.list` ✓ | `get_inventory_items_with_details_v2(p_organization_id)` | — |
| `inventory.low_stock` ✓ | `get_low_stock_item_count_v2` | — |
| `inventory.value` ✓ | `get_total_inventory_value_v2` | — |
| `inventory.movements` | `get_inventory_item_movement_history_v2` | — |
| `inventory.waste` | `get_recent_waste_value_v2` / `get_top_wasted_items_v2` | — |
| `inventory.cogs` | `get_cogs_for_period_v2` | — |
| `inventory.count.open` | `create_inventory_count_with_items_v2` | ⚑ |
| `inventory.count.submit` | `process_inventory_count_v2` | ⚑ |
| `inventory.item.upsert` | `create/update_inventory_item_v2` | ⚑ |
| `recipe.search` ✓ | `search_recipes_v2` / `search_recipes_by_ingredients_v2` | — |
| `recipe.scale` ✓ | *(none — pure compute the DB doesn't do)* | — |
| `recipe.cost.refresh` | `refresh_recipe_ingredient_costs_v2` | ⚑ |
| `recipe.dashboard` | `get_recipe_dashboard_data_v2` | — |
| `production.batches` | `get_production_batches` | — |
| `production.batch.*` | `kitchen_production.batches_public_insert/update` | ⚑ |
| `po.list` | `get_purchase_orders` | — |
| `po.receive_item` | `receive_po_item_v2` | ⚑ |
| `po.send` | `purchase_orders_public_insert` + supplier dispatch | ⚑⚑ |
| `receiving.list` | `get_receiving_events` | — |
| `receiving.process` | `process_and_mark_receiving_event_v2` | ⚑ |
| `supplier.list` ✓ | `get_suppliers` / `get_suppliers_with_categories_v2` | — |
| `alert.list` | `get_alerts` | — |
| `alert.resolve` | `simple_mark_alert_resolved` | ⚑ |
| `task.list` | `get_tasks` | — |
| `haccp.log` ✓ | *(none — validated record anchored on the HMAC audit chain; no new DB table)* | append-only |
| `prep.list` | derived from `get_production_batches` + menu | — |

All RPCs take `p_organization_id` (multi-tenant) and run under
`tenancy.require_org_context()`; the toolkit config carries the org id +
the PostgREST credentials (operator-provided, per-tool-process token at
`0600` — the Chapter F pattern).

### 3.3 Scopes + gate policy (the trust model, applied)

- `kitchen.read.*` — open (list, value, search, alerts).
- `kitchen.write.*` — **gated** (counts, adjustments, batch lifecycle).
- `kitchen.order.send` — **double-gated** (a PO spends money; a human
  approves every time, even inside the autonomous loop).
- `kitchen.haccp.log` — **append-only**, every entry on the **HMAC
  audit chain**.

These become additive bases in `aivyx-capability::KNOWN_BASES`.

### 3.4 The compliance wedge

`haccp.log → audit chain` is the differentiator: fridge-temp checks,
corrective actions, use-by overrides become HMAC-chained,
offline-verifiable, exportable records (the TUI `audit` screen,
repopulated with `haccp.*` events). "Every food-safety action is
cryptographically recorded; one command exports the EHO pack" is a claim
most kitchen software can't make — **to be marketed only after the chain
verification + HACCP semantics are independently checked.**

### 3.5 Autonomous reorder — wiring the loop

The headline autonomy story, and the reason the gate model matters. The
loop itself is **existing daemon machinery** (`aivyx loop`, the
HMAC-chained backlog, the `max_iterations` / wall-clock / token caps,
driver-side gate verification) + the **`[[schedule]]`** cron triggers —
the kitchen pack doesn't reimplement any of it. It just provides the
tools and a starter routine; the loop points at them.

**The nightly flow:**

```
[[schedule]] cron 02:00  →  aivyx loop add "nightly par reorder"
        aivyx loop start --max-iterations 3
                │
   ┌────────────┴─────────────────────────────────────────────┐
   │  agent works the story with the kitchen tools:            │
   │   1. kitchen.inventory.low_stock        (kitchen.read)    │
   │   2. kitchen.par.reorder  → draft POs   (kitchen.read)    │  ← runs unattended
   │   3. kitchen.po.send {confirmed:false}  (kitchen.order.send)
   │        → confirm-first GATE: no human at 02:00 →          │
   │          dispatches NOTHING; surfaces the draft POs        │  ← stops at the money step
   └───────────────────────────┬──────────────────────────────┘
                               │  morning
   operator reviews the draft POs  →  approves  →  kitchen.po.send {confirmed:true}
                                                    (one PO per supplier dispatched)
```

The loop does the tedious analysis **unattended overnight** and produces
ready-to-approve, per-supplier draft POs (`kitchen.par.reorder` now
emits `purchase_orders` grouped by supplier — one per `po.send`). It
**cannot** spend money autonomously: `kitchen.order.send` is
Trusted-tier *and* confirm-first, so the loop halts at the gate and
waits for a human. That is the whole safety argument — **autonomy up to
the consequential step, a human at it** — and it's enforced by two
independent mechanisms (the capability scope + the confirm-first
protocol), both audited.

**Starter skill (ships with the pack's skills bundle):**

> *"Nightly par reorder: read low stock, run `kitchen.par.reorder` to
> draft per-supplier POs, and present the drafts for approval. Never
> call `kitchen.po.send` with `confirmed: true` — leave that for the
> operator."*

---

## 4. Phasing

1. **Read + compute spike** — `aivyx-kitchen` crate: the KitchenDB RPC
   client + the `tools::catalog()` tool set (8 tools:
   `inventory.list/low_stock/value`, `recipe.search`, `recipe.scale`
   *(custom pure compute)*, `supplier.list`, `po.list`, `alert.list`) +
   a `dispatch` executor. All `kitchen.read`, ungated; zero write risk.
   Each `KitchenTool` descriptor is the spec for its `aivyx_core::Tool`
   impl. *(this PR)*
2. **Tool-process wiring** ✅ — `kitchen.read` registered in
   `aivyx-capability::KNOWN_BASES` (Trusted ceiling) + a generic
   `KitchenToolBinding` adapting every catalog entry to
   `aivyx_core::Tool` + the `aivyx-kitchen` binary serving them via
   `run_multi_tool_subprocess`. Register in `aivyx.toml`:
   `[[tool_process]]` `name = "kitchen"`, `command =
   "…/aivyx-kitchen"`. *(done — needs a live KitchenDB + config to run
   end-to-end)*
3. **Gated writes** ✅ — `kitchen.par.reorder` (pure compute — what to
   order back to par; `kitchen.read`, ungated) + `kitchen.po.send` (the
   money action — `kitchen.order.send` scope **and** confirm-first) +
   the inventory writes (count-based, the KitchenDB model):
   `kitchen.inventory.count.open` (scope-gated draft),
   `kitchen.inventory.count.submit` (confirm-first — commits counted
   quantities to on-hand), `kitchen.inventory.item.upsert` (scope-gated
   create/update). The confirm-first gate is generalized
   (`require_confirmed`); `kitchen.write` / `kitchen.order.send`
   registered.
4. **Loop + PO** ✅ — `kitchen.par.reorder` now emits per-supplier draft
   POs (`group_into_pos`); the nightly autonomy flow is wired via the
   existing `aivyx loop` + `[[schedule]]` machinery (§3.5): unattended
   draft, halt at the confirm-first `po.send` gate, human approves in
   the morning. Live supplier dispatch (the real `po.send` payload) is
   the remaining integration detail.
5. **HACCP + audit** 🚧 — `kitchen.haccp.log` built: a validated,
   canonicalized food-safety record (enforces *out-of-limit → corrective
   action*), ungated + append-only, whose call lands on the
   tamper-evident HMAC chain (tool id, scope, input hash, time,
   outcome). No new DB table — the chain *is* the anchor. **Next:** a
   durable record store + the EHO export (the chain filtered to
   `kitchen.haccp.log`, paired with the records its input hashes
   anchor).
6. **`kitchen` template + skills bundle** ✅ — `aivyx init --template
   kitchen` (examples/templates/aivyx-kitchen.toml, wired into the
   bundled-template registry): a BOH role with the `kitchen.*` scopes,
   the `aivyx-kitchen` `[[tool_process]]`, and the opt-in nightly
   reorder `[[schedule]]`. The starter skills bundle (reorder /
   fridge-temp / cook-hold / stocktake / recipe-scale) ships as
   `aivyx_kitchen::pack::skills_json()`, installed via `skills.teach`.
   The pack is now installable end to end.

## 5. Open questions

- **Distribution shape** — managed/hosted product vs. self-installed
  local pack (changes whether a fleet/hosting story is needed on top of
  the local-first base).
- **First integration targets** — which POS / supplier / inventory
  systems beyond KitchenDB (the moat is integrations, not the agent).
- **Pack format as a first-class artifact** — once there are two packs,
  formalize template + toolkit + skills + scopes as a bundle.
- **Brand** — the Kitchen OS Flutter UI uses an Aivyx-Studio-inspired
  coral/purple palette; the Aivyx TUI uses amber-on-near-black. Reconcile
  if the agent and the app are to feel like one product.

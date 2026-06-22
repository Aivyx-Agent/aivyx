# Kitchen Toolkit — give the BOH brigade real tools (Chapter Brigade)

> **Status:** 🟡 **PLANNED (BG.0–BG.5).** The first vertical pack's specialists
> can lead, delegate, and talk — but they can't *act*: the BOH Nonagon
> ([Chapter J](NONAGON.md) + the editable roster of [Chapter Roster](ROSTER.md))
> declares `kitchen.*` scopes, yet **no `kitchen.*` tools exist**. Chapter Brigade
> builds them: a **tool-process binary** (`aivyx-kitchen-toolkit`, the
> [Chapter F/G](VERTICAL_PACKS.md) substrate pattern) that talks to the operator's
> **KitchenDB** (Postgres + PostgREST RPCs) and registers the kitchen tool surface
> — inventory/recipe/supplier reads, stock/batch writes, purchase-order dispatch
> (confirm-first), and append-only HACCP logging. The `kitchen.*` capability bases
> are **already in `KNOWN_BASES`**, so there is **no new base, no P10 amendment, no
> change to `aivyx-core`/capability/daemon**. Declared as a `[[tool_process]]`, the
> toolkit's tools join the daemon `tool_list` that already feeds
> `TeamAssembly::base_tools` — so the BOH brigade gets them, capability-attenuated
> (NT-02), with no team-engine change.

## 1. Why this chapter

The team arc is mature — Nonagon (J), daemon-side missions (L), editable rosters
(Roster). The kitchen vertical pack ships a 9-role BOH brigade (`kitchen-boh.toml`)
whose members declare least-privilege `kitchen.*` scopes. But those scopes gate a
tool surface that **was never built**: `aivyx-capability` has the `kitchen.read /
write / order.send / haccp.log` bases, the pack names them in tool allowlists, and
the daemon attenuates them at spawn — yet `grep` finds no `kitchen.*` `Tool` impl.
So the brigade can decompose a goal and dialogue, then stall: there is nothing to
count stock with, no PO to dispatch, no HACCP row to write. Chapter Brigade closes
that gap — the first proof that a **vertical pack ships real domain tools** — and
turns the whole J/L/Roster team machinery into something that does commercial work.

## 2. Architecture & governance decisions (locked)

### A tool-process binary — third-party tier, the F/G substrate pattern
A new binary crate **`aivyx-kitchen-toolkit`** (mirroring `aivyx-gmail` /
`aivyx-toolkit`): `main.rs` runs the **multi-tool harness**
(`aivyx_tool::run_multi_tool_subprocess`), registering the kitchen tools. It is a
**third-party tool process** per P10/P11/P12 — **not** in the thirteen-tool core
cap, **not** an `aivyx-core` change. The operator declares it as a
`[[tool_process]]` in `aivyx.toml`; the daemon spawns it and proxies its tools
(`ToolProxy` over the bridge) into the live `tool_list`. The `kitchen.*` bases
**already exist** in `KNOWN_BASES` (registered when the pack landed), so Chapter
Brigade adds **no capability base and is not a P10 amendment** — it is the
tool-process tier, exactly like [Chapter Abacus](ABACUS.md)'s toolkit surface.

### KitchenDB is the system of record — the agent calls it, never reimplements
The toolkit is a thin **PostgREST RPC client**: `POST <base_url>/rpc/<fn>` with
`p_organization_id` + params, auth from config. The operator's **KitchenDB** (the
mature `kitchen_os_db` Postgres domain model — inventory, recipes, production,
purchase_orders, receiving, suppliers, HACCP) owns all domain logic via its stable
`get_*` / `*_v2` RPCs. The agent **calls** those RPCs; it never re-encodes inventory
math or PO rules. Operator config lives at
`~/.aivyx/tool-processes/kitchen/config.toml`: `base_url`, `api_key` (the PostgREST
`apikey` / bearer), `organization_id` (the multi-tenant key every RPC takes). No
secret is ever logged; the config file is `0600` (the substrate norm).

### Reaches the brigade with no team-engine change
A tool process's tools land in the daemon's `tool_list`, which Chapter J/Roster
already hand to `TeamAssembly::base_tools`. A BOH specialist whose `tool_allowlist`
names `kitchen.read` (etc.) receives exactly that tool, **capability-attenuated to
`declared ∩ lead`** at spawn (NT-02, unchanged). So the payoff — the brigade can
act — is delivered by the **existing** wiring; Chapter Brigade builds tools, not
plumbing.

### Confirm-first on what's irreversible or outbound
`kitchen.order.send` **dispatches a purchase order — money leaves the building** —
so it is **confirm-first**: it returns `RequiresEscalation` unless invoked with
`confirmed: true` (the established `git.commit` / `skills.teach` pattern). [Chapter
H](HEADLESS_MODE.md) already treats it as structurally blocked under any
non-interactive policy — that invariant holds here for free. Reads and the
append-only `kitchen.haccp.log` are not confirm-first. Each `kitchen.haccp.log`
call is **one row on the HMAC audit chain** — the tamper-evident HACCP record the
vertical's pitch rests on (EHO export), achieved without a new `AuditEvent` variant
(so no [[e2e-audit-chain-count-assertions]] breakage).

## 3. Scope

**In:** the `aivyx-kitchen-toolkit` binary crate + PostgREST RPC client + config
(BG.1); the read tools `kitchen.read` — inventory list / low-stock / value, recipe
search, supplier list (BG.1); the gated write tools `kitchen.write` — stock
adjustment / production-batch lifecycle (BG.2); `kitchen.order.send` — confirm-first
PO dispatch (BG.3); `kitchen.haccp.log` — append-only HACCP write, audit-chained
(BG.4); registration as a `[[tool_process]]` + a BOH-brigade live-tools check
(BG.4); tests (RPC request/parse + harness gating) + an end-to-end `multi_harness`
IPC drive against a mock PostgREST + an operator live-DB runbook (BG.5). **Out:**
re-implementing any KitchenDB domain logic (the DB owns it); a new capability base
or P10 amendment (the `kitchen.*` bases exist); changes to `aivyx-core` / capability
/ the team engine / the daemon tool-list path; the Kitchen OS Flutter GUI (stays the
operator's rich front-end); a second vertical; bundling Aivyx's own OAuth or a
hosted KitchenDB (the operator runs their own).

## 4. Phase plan (docs-first, small phases per convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **BG.0** 🟡 | **This design contract** | Locked reference; banner flips per phase. |
| **BG.1** | **Crate + RPC client + read tools** | `aivyx-kitchen-toolkit` binary over `run_multi_tool_subprocess`; PostgREST client (`/rpc/<fn>`, `p_organization_id`, bearer from `config.toml`); `kitchen.read` tools (inventory list/low-stock/value, recipe search, supplier list). Request-build + response-parse unit-tested against canned JSON. |
| **BG.2** | **Gated write tools** | `kitchen.write` — stock adjustment + production-batch lifecycle RPCs. Trusted-tier; reuses the BG.1 client. |
| **BG.3** | **PO dispatch (confirm-first)** | `kitchen.order.send` — dispatch a purchase order; **`RequiresEscalation` unless `confirmed: true`**; headless auto-blocks (Chapter H). |
| **BG.4** | **HACCP + registration** | `kitchen.haccp.log` (append-only; one audit-chain row per call). Register the toolkit as a `[[tool_process]]` (config + scope narrowing); confirm a BOH specialist receives its `kitchen.*` tools attenuated (NT-02). `docs/VERTICAL_PACKS.md` + `MCP_RECIPES`-style recipe. |
| **BG.5** | **Finalize** | RPC + harness tests green; an end-to-end `multi_harness` IPC drive of the **real release binary** against a **mock PostgREST** (per [[chapter-abacus]] AB.5); an operator live-KitchenDB runbook; full workspace suite + clippy `-D warnings` + `cargo deny`; chapter memory; status → COMPLETE. |

**Discipline:** the RPC client is structured so the request-build + JSON-parse halves
are testable without a live server (a transport seam + canned fixtures); the live
KitchenDB is a **runbook, not a CI dependency**. The one new dependency is the HTTP
client (`reqwest`, already in-tree for `web.search` / Google integrations) — confirm
`cargo deny` stays green. Test band: **moderate–high** — per-tool request/parse +
harness gating + the e2e drive; price **~30–45 new tests**.

## 5. Open questions (resolve in-phase)

- **OQ-1 — crate home (BG.1).** A dedicated `aivyx-kitchen-toolkit` binary crate
  (locked lean — the substrate convention is one binary per integration, keeps the
  `aivyx-kitchen` pack lib separate from the tools) vs. a `[[bin]]` inside
  `aivyx-kitchen`. Revisit only if the split causes friction.
- **OQ-2 — test transport (BG.1/BG.5).** A transport seam (a trait the real reqwest
  client implements; tests inject canned JSON) **plus** one end-to-end drive against
  a hand-rolled in-process mock — vs. a `wiremock`/`httpmock` dev-dep. Lean the seam
  + minimal in-process mock (no new dev-dep; `cargo deny`-friendly).
- **OQ-3 — RPC surface fidelity (BG.1+).** The exact `get_*` / `*_v2` function names
  + params are confirmed against the operator's live KitchenDB schema in-phase
  (the operator has the DB); the contract fixes the *shape* (org-scoped RPC calls),
  not the catalog.
- **OQ-4 — HACCP depth (BG.4).** Rely on the per-call HMAC audit row for HACCP
  tamper-evidence (locked — no new `AuditEvent` variant, no count-assertion churn)
  vs. a richer HACCP-specific audit event (deferred; additive if EHO export later
  wants structured fields).

---

*Chapter Brigade gives the kitchen brigade its knives. The team that Chapters J, L,
and Roster taught to organize — a lead and its least-privileged specialists — can
finally do the work: read the walk-in, adjust a count, draft and (with a human nod)
send the order, and write the HACCP log onto a tamper-evident chain. The system of
record stays KitchenDB; Aivyx is the conversational, auditable, gate-safe hands on
it. The first vertical pack stops describing a kitchen and starts running one.*

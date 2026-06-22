# Package the kitchen vertical — one-command setup + doctor checks (Chapter Mise)

> **Status:** 🟡 **PLANNED (MI.0–MI.2).** [Brigade](BRIGADE.md) + [Lockup](LOCKUP.md)
> made the kitchen vertical *work*; Mise makes it *installable*. Today an operator
> assembles it by hand — write `~/.aivyx/tool-processes/kitchen/config.toml`,
> append a `[[tool_process]]`, point `[team] config_path` at the BOH pack, hope
> KitchenDB is reachable. Mise turns that into **`aivyx connect kitchen`**: a
> guided, non-OAuth onboarding that writes the config (0600), auto-wires the
> `[[tool_process]]`, plants the bundled `kitchen-boh.toml` + sets
> `[team] config_path`, and **probes KitchenDB reachability** before declaring
> success — plus **`aivyx doctor`** checks so the operator can see the vertical is
> healthy. *Mise en place*: everything in its place before service. **No new
> capability base, no P10 amendment, no new tool** — this is install/onboarding
> glue over Brigade/Lockup/Roster primitives.

## 1. Why this chapter

The kitchen vertical is the first commercial unit, and right now it ships as
*code an operator could assemble*, not *a product they install*. The
[Chapter F/G](VERTICAL_PACKS.md) substrate already proved the onboarding pattern
for Google services — `aivyx connect gmail` collapses "make an OAuth app →
hand-write config → run auth → wire `[[tool_process]]`" into one command. The
kitchen toolkit needs the same on-ramp, minus OAuth (it authenticates to KitchenDB
with a PostgREST `base_url` + `api_key` + `organization_id`, not a consent dance).
Mise gives it that on-ramp and the `doctor` visibility, so "install the kitchen
vertical" is one command an operator runs, not a runbook they follow.

## 2. Architecture & governance decisions (locked)

### `aivyx connect kitchen` — a non-OAuth branch of the existing `connect` command
`connect` today drives **Google OAuth** services (a `ConnectService` registry with
`client_id`/`client_secret`/redirect/`tokens.json`). Kitchen is **not OAuth**, so
it does **not** join that registry — instead `connect kitchen` routes to a
dedicated flow that reuses the *generic* helpers (`append_tool_process` /
`tool_process_present` over `toml_edit`, the `0600` config writer, the
process-dir layout) but writes the **`[kitchen_db]`** config shape (Brigade BG.1)
and runs a **reachability probe** in place of the OAuth handshake. "Connected" for
kitchen = config present **and** KitchenDB answered, not a token file.

### The flow does four things, each idempotent
1. **Write** `~/.aivyx/tool-processes/kitchen/config.toml` (`0600`) with the
   operator's `base_url` / `api_key` / `organization_id` (prompted, or from flags
   / env for non-interactive installs).
2. **Auto-wire** `[[tool_process]] name = "kitchen", command = "aivyx-kitchen-toolkit"`
   into `aivyx.toml` (skipped if already present — `tool_process_present`).
3. **Plant the pack**: write the bundled `KITCHEN_BOH_TOML` (the
   `aivyx-kitchen` asset) to a `kitchen-boh.toml` beside `aivyx.toml` and set
   `[team] config_path` to it (the [Chapter Roster](ROSTER.md) lever) — unless the
   operator already set a team config (never clobber).
4. **Probe** KitchenDB: one `KitchenClient` RPC (e.g. `get_suppliers`) to confirm
   the URL + key + tenant before declaring success; a clear, actionable error
   otherwise. Re-running `connect kitchen` re-probes (a health recheck).

### `aivyx-cli` gains an `aivyx-kitchen` dependency
To plant the bundled pack, the binary needs `KITCHEN_BOH_TOML` (and may reuse
`kitchen_boh_team()` for a roster summary). `aivyx-cli` already depends on
`aivyx-team`; adding `aivyx-kitchen` (which only deps `aivyx-team` +
`aivyx-capability`) introduces no cycle and no heavy graph. The reachability probe
reuses `aivyx-kitchen-toolkit::KitchenClient` — so `aivyx-cli` also gains that
crate as a dep (its only new outward dep; `reqwest` is already in the graph).

### `aivyx doctor` learns the kitchen vertical
`doctor` gains a kitchen section (only when the vertical is configured): config
present? `[[tool_process]]` wired? `[team] config_path` set to a valid pack?
KitchenDB reachable? Each a `pass`/`fail` line with an actionable hint, reusing
doctor's existing check shape. Absent config → the section is silently skipped
(operators without the vertical see no kitchen noise).

## 3. Scope

**In:** `aivyx connect kitchen` (non-OAuth: config write + `[[tool_process]]`
auto-wire + pack plant + `[team] config_path` set + KitchenDB reachability probe),
interactive **and** flag/env-driven for non-interactive installs (MI.1); the
`aivyx doctor` kitchen section (MI.2); the `aivyx-cli` → `aivyx-kitchen` /
`aivyx-kitchen-toolkit` deps; tests (config render, tool-process wire idempotency,
pack-plant, doctor checks against a mock) + an updated install/runbook doc; finalize
(MI.2). **Out:** a docker-compose *kitchen appliance* (a future Harbor increment —
the daemon appliance already exists; a kitchen-specific compose is deferred);
hosting KitchenDB (the operator runs their own); any change to the kitchen tools
themselves, the capability model, or the team engine; a generic non-OAuth
`ConnectService` refactor (kitchen gets a dedicated branch; generalizing `connect`
is a separate cleanup if a second non-OAuth service ever appears).

## 4. Phase plan (docs-first, small phases per convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **MI.0** 🟡 | **This design contract** | Locked reference; banner flips per phase. |
| **MI.1** ✅ | **`aivyx connect kitchen`** | DONE. New `connect_kitchen.rs` routed from `connect`'s dispatch (non-OAuth branch, before the OAuth `find_service`). Writes `[kitchen_db]` config `0600` (`render_kitchen_config_toml`), auto-wires `append_tool_process("kitchen","aivyx-kitchen-toolkit")` (idempotent via `tool_process_present`), plants `KITCHEN_BOH_TOML` beside `aivyx.toml` + `set_team_config_path_if_absent` (**no-clobber**), then **probes** KitchenDB (`get_suppliers`) as the connected signal (success → restart hint; failure → actionable error + non-zero). Fields from env (`AIVYX_KITCHEN_BASE_URL`/`_API_KEY`/`_ORGANIZATION_ID`) for headless, else TTY prompts. Reuses connect's helpers (made `pub(crate)`: `prompt_line`/`prompt_yes_no`/`set_file_0600`); `aivyx-cli` gained `aivyx-kitchen` + `aivyx-kitchen-toolkit` deps. Listed in `aivyx connect`. 4 tests (config render+escape, round-trips into the toolkit loader, team-config no-clobber ×2); clippy `-D warnings` green. |
| **MI.2** | **`aivyx doctor` kitchen + finalize** | A kitchen section in `doctor` (config / tool_process / team config_path / KitchenDB reachable — each pass/fail+hint; skipped when unconfigured). Install-doc/runbook refresh. Full workspace suite + clippy `-D warnings` + `cargo deny`; chapter memory; status → COMPLETE. |

**Discipline:** the config-write + tool-process-wire reuse the existing
`connect.rs` helpers; the probe reuses `KitchenClient` (no new client mechanic).
Every step is **idempotent** (re-running `connect kitchen` is a safe recheck) and
**no-clobber** (never overwrites an operator's existing team config / tool_process).
Test band: **moderate** — config render + wire idempotency + pack-plant + probe
(mock PostgREST) + doctor checks; price **~12–20 new tests** (the interactive prompt
+ the live KitchenDB stay operator-verified, per precedent).

## 5. Open questions (resolve in-phase)

- **OQ-1 — pack-plant location (MI.1).** `kitchen-boh.toml` beside `aivyx.toml`
  (locked lean — matches Roster's conventional discovery + `[team] config_path`)
  vs. inside `~/.aivyx/`. Revisit if the operator runs from a non-cwd config.
- **OQ-2 — secret prompting (MI.1).** Prompt for `api_key` on a TTY (hidden input)
  vs. require `--api-key`/env only. Lean **prompt on TTY, flag/env otherwise** —
  matches `connect`'s guided feel while staying scriptable; the key lands `0600`.
- **OQ-3 — probe RPC (MI.1).** Use `get_suppliers` (a cheap, side-effect-free read)
  as the reachability probe vs. a dedicated health RPC. Lean `get_suppliers` (no
  new KitchenDB surface); a 4xx/connection error → a specific "check base_url / key
  / org" hint. (Carries Brigade OQ-3 — names confirmed vs the live schema.)
- **OQ-4 — connect registry shape (MI.1).** A dedicated kitchen branch (locked) vs.
  generalizing `ConnectService` to non-OAuth. Defer the generalization until a
  second non-OAuth service exists (YAGNI).

---

*Chapter Mise is the apron and the prepped station. Brigade forged the knives and
Lockup let purchasing write the order; Mise means an operator runs one command —
`aivyx connect kitchen` — and the whole vertical is wired, planted, and proven
reachable, with `aivyx doctor` to confirm it at a glance. The first vertical stops
being something you assemble and becomes something you install.*

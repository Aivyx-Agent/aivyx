# Studio Tools Screen — a read-only, searchable tool catalog (Chapter Almanac)

> **Status:** 🟡 **IN PROGRESS (AL.1–AL.3 done; AL.4 finalize pending).** Chapter
> Atlas cataloged the tool surface in docs (`docs/TOOLS.md`) and gave the agent
> its own introspection tool (`tools.list`), but deferred the optional **Studio
> "Tools" view** (AT.4) as scope-widening. This chapter builds it: a new **read-only,
> searchable Studio screen** listing every tool the daemon has registered — name,
> description, capability base, and the minimum trust tier that base requires —
> grouped by domain. Follows the exact R–Z / Lantern recipe: a wasm-clean view type
> + a new IPC query, a daemon handler, and a Studio panel. No new capability base,
> no new tool, no write surface.

## 1. Why this chapter

An operator (or a contributor extending Aivyx) who wants to know "what can this
agent actually do?" currently has three options: read `docs/TOOLS.md` (accurate but
static — it documents the *design*, not a specific running daemon's *registered*
set), ask the agent to call `tools.list` (accurate and live, but conversational —
no browse/search UI), or run `aivyx tools` (audit-derived call stats, gated on an
audit log, not a pure catalog). None of these is "open the Studio and browse." This
chapter closes that gap the same way Repertoire did for skills and Lantern did for
MCP servers: a dedicated screen over a purpose-built read-only query.

## 2. Architecture & governance decisions (locked)

### Read-only screen over a new IPC query — the R–Z recipe, no new capability
No new tool, `KNOWN_BASES` base, scope, or dependency-graph risk. A wasm-clean
**`ToolCatalogEntry`** view type (`name`, `description`, `scope_base`, `min_tier`)
+ a **`GetToolCatalog`** read-only query in `aivyx-ipc`, a daemon **handler**, and a
Studio **panel** (`ToolsPanel`/`ToolCard`) — the same shape as `GetMcpStatus`/
`McpPanel` (Chapter Lantern) and `GetSkills`/`SkillsPanel` (Chapter Repertoire).

### Reuse `tool_descriptors`, the snapshot Phase 102 already built
`aivyx.rs` already captures `Vec<ToolDescriptor>` (`name`/`description`/`scope_base`)
once at daemon construction, for the existing `GetToolStats` observability query.
`GetToolCatalog` reuses that same snapshot — no new capture site, no second
enumeration of the tool list. The two queries stay deliberately separate, though:
`GetToolStats` requires an audit log and answers "what has been *called*, how often,
with what outcomes" (a chain read every time); `GetToolCatalog` requires nothing but
the snapshot and answers "what is *registered*" (no chain read, so it works even
before the agent's first turn, and even if `[audit]` is unconfigured).

### The minimum trust tier is derived, not stored
`ToolStat`/`ToolDescriptor` carry a `scope_base` but no tier. Rather than hand-
duplicate the D5 ceiling table into a third place, `aivyx-capability` gained one
new `pub fn`:

```rust
impl TrustTier {
    /// The least-trusted tier whose default ceiling grants `scope` — the
    /// minimum tier a channel needs before a tool requiring this scope
    /// becomes reachable. Checks tiers in ascending trust order
    /// (Untrusted → SemiTrusted → Trusted → Kernel).
    pub fn min_for_scope(scope: &Scope) -> TrustTier
}
```

It walks the four `CEILING_*` statics (already private — this is the first `pub`
surface over them) in ascending trust order and returns the first tier whose
ceiling grants the *bare* (unqualified) form of the scope. Bare, not the tool's
runtime-input-derived qualified scope: a tool's `required_scope(&json!({}))` call
already proved pure and input-independent for its **base** (`planner.rs`'s
`r1_scope_derivation_uses_the_input_path` test — a missing `path` field still
returns `fs.read`, just with a fallback qualifier), so the daemon handler discards
whatever placeholder qualifier comes back and re-parses a clean bare scope from the
base string before calling `min_for_scope`. This sidesteps the D5 "▲ triangle rule"
entirely (a qualified held scope not covering an unqualified needed one) — the
catalog is asking "what's the *floor*," which is exactly what the bare-scope check
against each ceiling answers, and it lines up with `docs/TOOLS.md`'s own prose tier
list (SemiTrusted bases hold their *bare* form in `CEILING_SEMITRUSTED`, etc.).

`aivyx-ipc` picked up `aivyx-capability` as a dependency so `ToolCatalogEntry` can
embed `TrustTier` directly on the wire (both crates are dependency-free of
storage/async/network, so this stays wasm32-clean — confirmed by a `cargo check
--target wasm32-unknown-unknown -p aivyx-web` pass with the new dependency chain).

### Client-side search/filter, grouped by domain
Unlike the memory search screen (server round-trip on Enter), the tool catalog is
small (dozens, not thousands) and static per daemon run, so the Studio does the
filtering **in-WASM**: a plain `to_lowercase().contains()` over name/description/
scope_base, live on every keystroke — the same pattern the command palette
(`CommandPalette`) already uses over `View::ALL`. Grouping is by the tool name's
leading dotted segment (`fs.read` → `fs`, `data.xlsx.write` → `data`), computed
client-side from the already-fetched snapshot.

## 3. Scope

**In:** the `ToolCatalogEntry` wasm-clean type + `GetToolCatalog` query/response in
`aivyx-ipc` (AL.1); `TrustTier::min_for_scope` in `aivyx-capability` (AL.1); the
daemon handler mapping `tool_descriptors` → `ToolCatalogEntry` rows (AL.2); the
Studio screen — nav, `ToolsPanel`/`ToolCard`, `ToolsState`, query, response handling,
search/filter, domain grouping, CSS (AL.3); tests; gates + bundle + a live dogfood
check (AL.4). **Out:** editing/disabling a tool from the screen (registration stays
config/code-driven); call-count/outcome observability (that's `GetToolStats`,
unchanged); inline schema viewing (the agent's own `tools.list detail=true` already
covers that conversationally); the `/classic` legacy UI.

## 4. Phase plan (docs-first, small phases per convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **AL.0** 🟡 | **This design contract** | Locked reference; banner flips per phase. |
| **AL.1** ✅ | **Shared types + tier helper** | DONE. `TrustTier::min_for_scope(&Scope) -> TrustTier` in `aivyx-capability` (walks the four `CEILING_*` statics ascending). `ToolCatalogEntry { name, description, scope_base, min_tier }` + `QueryPayload::GetToolCatalog` + `QueryResponsePayload::GetToolCatalog { tools }` in `aivyx-ipc`, which picked up an `aivyx-capability` dependency (wasm32-clean, verified). Wire round-trip test green. |
| **AL.2** ✅ | **Daemon handler** | DONE. `daemon_server.rs`'s `handle_query` answers `GetToolCatalog` by mapping the existing `tool_descriptors: &[ToolDescriptor]` snapshot (Phase 102, already captured at daemon construction for `GetToolStats`) into `ToolCatalogEntry` rows, deriving `min_tier` via `Scope::parse(&d.scope_base)` + `TrustTier::min_for_scope`. No audit-log dependency, no chain read — a pure registry snapshot. |
| **AL.3** ✅ | **Studio screen** | DONE (host + wasm32 typecheck clean). `View::Tools` + a "Tools" nav item (System group, next to MCP) with a new hand-authored wrench icon (`tools.svg`, matching the existing line-icon style) + `ToolsPanel`/`ToolCard`/`ToolsState`, threaded through `ws_task`/`read_task` (mirrors `McpState`). On-open load + manual **Refresh** button (no poll — the registry only changes on a daemon restart or an MCP hot-swap). Client-side search box (name/description/scope_base substring) + domain grouping (tool-name leading segment). Tier rendered as a colored chip (`sage`=Trusted, `amber`=SemiTrusted, `muted`=Untrusted, `error`=Kernel — reusing existing chip classes, no new design tokens beyond the grid/card layout mirroring `.mcp-grid`/`.mcp-card`). |
| **AL.4** 🟡 | **Finalize** | **Local half DONE:** full workspace test suite (124 binaries) + `cargo clippy --workspace --all-targets -D warnings` green; `cargo deny check` green on bans/licenses/sources (one pre-existing, unrelated `RUSTSEC-2026-0195` advisory on `quick-xml` via `aivyx-desktop`'s notification stack — not touched by this chapter, flagged separately). `dx bundle --release` + `dist/` reassembled; the release `aivyx` binary confirmed (via `strings`) to embed `GetToolCatalog`/`ToolCatalogEntry`, the new search-box copy, and the hashed `tools-8a38d7f1fddc3cf6.svg` icon. Committed + pushed (`8f414ae`). **Pending (needs the operator/rig):** deploy to the dogfood rig (an explicit warning before the daemon restart — a live remote system, not a local dev loop); `scripts/studio_sweep.py` click-through (blocked in *this* sandbox — the harness reaps any long-lived TCP server, same constraint every prior Studio chapter hit); live verification that the Tools screen renders the real registered catalog **and** that a document-writing tool (`data.xlsx.write` or `data.pdf.write`, Chapter Sheaf SH.6) round-trips end-to-end. |

**Discipline:** AL.1 lands the shared types + the tier helper so AL.2/AL.3 build on
one definition, exactly as Lantern's LN.1 did for the snapshot home. Test band:
**small** — mostly the wire round-trip + the tier-helper's correctness against the
D5 ceiling tables (already covered by existing `aivyx-capability` ceiling tests);
price **~5–10 new tests**, the screen itself verified by the served-browser sweep
per the R–Z precedent.

## 5. Open questions (resolve in-phase)

- **OQ-1 — tier storage vs. derivation (AL.1).** Store `min_tier` precomputed in
  `ToolDescriptor` at capture time vs. derive it on every `GetToolCatalog` call.
  Locked derive-on-call: the derivation is a handful of `grants()` checks over
  already-`LazyLock`-cached ceilings, cheap enough that precomputing and keeping it
  in sync would be the more fragile choice.
- **OQ-2 — grouping key (AL.3).** Group by `scope_base` vs. the tool `name`'s
  leading segment. Locked `name`: several tools share one `scope_base` (`web.fetch`
  keys on `net.fetch`), so grouping by name keeps visually-related tools (e.g. all
  `email.*`) together even when their capability base differs.
- **OQ-3 — search scope vs. server-side (AL.3).** Client-side substring filter vs.
  a server round-trip per keystroke. Locked client-side: the catalog is small and
  static per daemon run (unlike memory search, which can be arbitrarily large),
  so there is no latency or payload-size reason to hit the wire on every keystroke.

---

*Chapter Almanac is the Studio's window into what Atlas cataloged: an operator can
now open one screen and see every tool the daemon has registered, the capability
it gates on, and the tier a channel needs before it's reachable — searchable,
grouped, and always live to what's actually running, not just what's documented.*

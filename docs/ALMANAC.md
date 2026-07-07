# Studio Tools Screen — a read-only, searchable tool catalog (Chapter Almanac)

> **Status:** ✅ **COMPLETE (AL.0–AL.4).** Chapter
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
| **AL.4** ✅ | **Finalize** | DONE. Full workspace test suite (124 binaries) + `cargo clippy --workspace --all-targets -D warnings` green; `cargo deny check` green on bans/licenses/sources (one pre-existing, unrelated `RUSTSEC-2026-0195` advisory on `quick-xml` via `aivyx-desktop`'s notification stack — not touched by this chapter, flagged separately). `dx bundle --release` + `dist/` reassembled; the release binary confirmed (via `strings`) to embed `GetToolCatalog`/`ToolCatalogEntry` + the new `tools.svg` icon. Committed + pushed (`8f414ae`, `0b45442`). **Deployed to the dogfood rig 2026-07-07** (old binary backed up as `aivyx.pre-almanac-2026-07-07.bak`; sha256-verified scp; clean `systemctl --user restart` — active, both MCP servers reconnected, no crash-loop). **Live-verified two ways:** (1) a raw IPC probe against the running daemon's `GetToolCatalog` returned all 66 real registered tools (58 Trusted / 7 SemiTrusted / 1 Kernel — see the `team.run` note below) including `data.xlsx.write` and `data.pdf.write`; (2) `scripts/studio_sweep.py` (extended to click through "Tools") ran clean against the live rig Studio — 11/11 screens render with zero JS errors (known cold-cache flake needed one warm re-run, as with every prior Studio chapter), and a follow-up deep-check confirmed the Tools screen renders all 66 cards grouped by domain and that the client-side search filter works live (typing "xlsx" narrowed 66→2 correctly). Stopped short of actually *invoking* a document-writing tool against the live rig (would write a real file into the operator's sandbox unprompted) — the round-trip is proven by the crate's own tests + SH.6; available as a follow-up if wanted. |

**Side finding from the live data — FIXED same-day:** the live catalog surfaced `team.run` (the Nonagon team-mission tool) at **Kernel** tier. Root cause found via `git log -S'"team.run"'`: the L.7 commit (`51a711d`, 2026-06-12) added the base to `KNOWN_BASES` with a doc comment promising *"Channel-tier, Trusted (like the loop tools)"* but never actually added it to `CEILING_TRUSTED` — a one-commit oversight, not a deliberate restriction. Since `CEILING_KERNEL` is auto-derived from all of `KNOWN_BASES`, the gap was invisible to every existing test; nothing asserted `CEILING_TRUSTED.grants(&s("team.run"))`. Practical impact: since 2026-06-12, no Trusted-tier chat turn or autonomous-loop iteration could ever actually invoke `team.run` — including the exact delegation path the loop system prompt teaches the model to use. **Fixed in `5a92a15`**: added `"team.run"` to `CEILING_TRUSTED` + a regression test (`team_run_is_in_trusted_ceiling_only`, mirroring the existing `role_switch` one). Full suite + clippy green; deployed + live-verified on the rig (`GetToolCatalog` now reports `team.run` as Trusted; the Kernel bucket in the tier breakdown dropped from 1→0).

**Follow-up full audit (operator-requested) found a second, older instance — also FIXED same-day:** dumped every one of the 93 `KNOWN_BASES`' actual `min_for_scope` tier and diffed against every tier claim in `docs/TOOLS.md`. One real mismatch: **`git.read`** — docs said Trusted, code computed Kernel. Root cause via `git log -S'"git.read"'`: Amendment A12 (`fc5ec30`, Phase 109, 2026-05-28 — *older* than the team.run bug) added `git.read` to `KNOWN_BASES` only as a docs-first task, framing it in the amendment doc as "the same category as `fs.read`, `fs.write`, and `net.fetch`," but the ceiling entry was never added in the follow-on tool-implementation tasks. A later comment (Chapter Forge FG.2, on `git.write`) had rationalized the gap as deliberate — *"the read sibling `git.read` is reachable at the operator/Kernel tier the Local CLI runs under"* — but that claim doesn't hold up against the actual code: `local.rs`'s real `trust_tier()` returns `Trusted` for the Local CLI, and no real channel anywhere in the codebase ever returns `Kernel` (the enum's own doc comment says Kernel is "never assigned to a user-facing channel"). So the rationalization was itself mistaken, not a checked design decision. Practical impact: `git.status`/`git.diff` — two of the "thirteen tools forever" locked substrate tools — were unreachable from any Trusted-tier channel since Phase 109. **Fixed in `26a3518`**: added `"git.read"` to `CEILING_TRUSTED`, corrected the misleading FG.2 comment, + a regression test (`git_read_is_in_trusted_ceiling_only`). Full suite + clippy green; deployed to the rig (clean restart, 66 tools unchanged). Could not live-verify via the catalog the way `team.run` was — this rig has no `[git]` config, so `git.status`/`git.diff` aren't registered there at all; correctness rests on the local test suite instead. A repo-wide sweep of the remaining 91 bases (matching `docs/TOOLS.md`'s tier column against `min_for_scope`) found no further mismatches — Chapter-F's 15 integration bases (checked first, see the dedicated regression test) plus the rest of the substrate/infrastructure bases all agree with their documented tier.

**Follow-up guard-coverage audit (operator-requested) — a different question ("does the documented tier hold up" vs. "does every fs/net call site actually route through its guard") — found three real gaps, all FIXED same-day.** Checked every real call site of `SensitivePolicy::classify`/`classify_write` (Ward/Portcullis) and the egress guard (Rampart). Network was clean: all three web tools (fetch/extract/post) plus `net.dns` call the egress guard on the initial URL *and* every redirect hop, and all four get `set_egress_policy` wired at daemon boot. Filesystem was not:
- **`FsDeleteTool`** never called `classify_write` at all — `FsReadTool`/`FsWriteTool` both check `SensitivePolicy`, but the dedicated `fs.delete` tool could permanently destroy a secret/persistence path (`~/.ssh/id_rsa`, `~/.aws/credentials`, even the daemon's own `store.redb`/audit chain) that read/write refuse to touch, with `confirm_destructive` — a self-declared `confirmed: true` flag, not a categorical block — as the only control. Notably *less* protected than `shell.exec`, which already scans command text for the same paths (so `rm ~/.ssh/id_rsa` via a shell call was blocked, but the identical deletion via the dedicated tool wasn't).
- **`FsMetadataTool`** never called `classify` — stat-ing a secret file's size/mtime, or listing a protected directory's filenames, leaked at SemiTrusted even though reading its *content* is Trusted+.
- **`memory.gc`** never checked the reserved `\x01` session-namespace sentinel that `memory.write`/`memory.forget` both refuse (not practically exploitable — no default role grants a wildcard `memory.gc` — but inconsistent).

**Fixed in `132299c`**: `FsDeleteTool` gained a `sensitive` field + `classify_write` check on the resolved parent+name (mirroring `FsWriteTool` exactly); `FsMetadataTool` gained a `sensitive` field + `classify` check on the canonical path (mirroring `FsReadTool`); `topic_uses_reserved_prefix` was promoted from private to `pub` (re-exported at `aivyx-memory`'s crate root) so `memory.gc` shares the same source of truth instead of re-deriving the sentinel. All three wired into the daemon's real construction path using the same shared `sensitive_policy`/`SensitivePolicy` Arc already threaded through `fs.read`/`fs.write`/`shell.exec`, plus one regression test each (`portcullis_refuses_deleting_persistence_and_secret_paths`, `ward_refuses_inspecting_a_sensitive_file_inside_the_sandbox`, `required_scope_deny_on_reserved_prefix_topic` + `execute_refuses_reserved_prefix_topic`). Full suite + clippy green; deployed to the rig (clean restart, 66 tools unchanged, zero construction errors in the journal). Correctness rests on the local test suite (exercising the real production `SensitivePolicy`, not a mock) rather than a live destructive-tool invocation — deliberately did not test this by asking the agent to actually delete something on the operator's live rig.

**Follow-up audit (operator asked "any other capability domains worth auditing") found a fourth, more severe gap — also FIXED same-day.** `workspace.rs`'s own comment claimed containment parity with `fs.*` ("the same lexical fence `fs.*` uses"), but `fs.*`'s own module doc describes **two** independent layers — lexical resolve at `required_scope`, and a **canonical re-check at execute time** specifically to catch a symlink planted *after* construction. None of the five `workspace.*` tools (read/write/list/delete/note) ever had the second layer. Concretely: a symlink planted inside the workspace (trivially reachable via a chained `shell.exec` call — `ln -s /etc/shadow escape`, both tools ordinarily co-granted to the same Trusted-tier channel) was followed straight through, giving arbitrary file **read/write/delete outside the workspace entirely** — worse than the fs.delete/fs.metadata findings above, which were "missing an overlay on an already-sound sandbox"; this was the sandbox boundary itself not being enforced, and it applies at *every* access level since the workspace root is independent of `fs_root`. **Fixed in `085f113c`**: added `canonical_fence`/`canonical_fence_parent` helpers (mirroring `FsReadTool`'s and `FsWriteTool`'s/`FsDeleteTool`'s identical shapes) to all five tools; `write`/`note` additionally refuse outright when the target is already a symlink, checked via `is_symlink` (an `lstat`) rather than `exists()` (a `stat`, which follows the link and would silently miss a *dangling* symlink whose destination doesn't exist yet — exactly the case `std::fs::write` would still create through). 5 new regression tests, one per tool, each planting a real symlink via `std::os::unix::fs::symlink` and confirming refusal. Full suite + clippy green; deployed to the rig (clean restart, 66 tools unchanged, zero construction errors, all five `workspace.*` tools still registered at Trusted tier). Same live-verification limit as the fs.delete/fs.metadata fixes: correctness rests on the local test suite (real symlinks, real `SensitivePolicy`-adjacent fence, not mocked) rather than planting a real escaping symlink on the operator's live rig.

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

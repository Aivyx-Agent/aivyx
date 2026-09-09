# Studio MCP Screen — surface server health in the web GUI (Chapter Lantern)

> **Status:** ✅ **COMPLETE (LN.0–LN.4).** The web port of [Chapter Conduit](CONDUIT.md)'s
> `aivyx-pa mcp status`: a **read-only Studio screen** that shows each configured MCP
> server as connected (with its tool count) or failed (with the reason + captured
> stderr) — the same data the CLI prints, now in the Dioxus web GUI. It follows the
> established read-only Studio-screen recipe (Chapters R–Z): a wasm-clean view type +
> a `GetMcpStatus` IPC query in `aivyx-ipc`, a daemon handler, and a new screen in
> `aivyx-web`. The one wrinkle: Conduit's status lives in a **snapshot file** (not in
> daemon memory, OQ-4), so this chapter lifts the snapshot read/write helpers to a
> shared native home both the daemon handler and the CLI use — **one source of
> truth**. No new capability base, no P10 amendment, no new dependency.

## 1. Why this chapter

Conduit made operator-added MCP servers work and gave the CLI an `aivyx-pa mcp status`
to diagnose them. But the Studio — the local-first web GUI that already surfaces
Command / Missions / Chat / Memory / Settings / Agents / Teams / Documents / Voice —
has no window into MCP at all. An operator wiring up a GitHub or remote MCP server
from the Studio currently has to drop to a terminal to see whether it connected.
Lantern closes that: the same connected/failed + tool-count + error/stderr readout,
rendered as a Studio screen, so "did my server come up, and why not?" is answerable
without leaving the GUI.

## 2. Architecture & governance decisions (locked)

### Read-only screen over a new IPC query — the R–Z recipe, no new capability
No new tool, `KNOWN_BASES` base, scope, P10 amendment, or dependency. This is the
exact shape Chapters R–Z used for every Studio screen: a wasm-clean **view type** +
a **`GetMcpStatus`** read-only query in `aivyx-ipc`, a daemon **handler**, and a web
**panel**. Governance (adding/removing servers) stays in `aivyx-pa.toml`; the screen
*shows*, it does not edit — exactly as the Teams screen renders the roster without
editing it.

### One source of truth: lift the snapshot helpers to a shared native home
Conduit's status is an `McpStatusSnapshot` the daemon writes to
`$XDG_DATA_HOME/aivyx-pa/mcp-status.json`, with the struct + path resolver currently
living **inside the `aivyx-cli` binary**. The daemon (in `aivyx-channel`) needs to
*read* that same snapshot to answer `GetMcpStatus`. So:
- the wasm-clean **`McpServerStatusView`** (name / transport / connected / tool_count /
  error / stderr_tail) moves into **`aivyx-ipc`** (pure data, serde, no `std::fs`);
- the **path resolver + read/write helpers** move into a native module
  (`aivyx-channel`, which `aivyx-cli` already depends on), serializing
  `{ captured_unix, servers: Vec<McpServerStatusView> }`.

The daemon's startup writer (CD.3) and the CLI's `aivyx-pa mcp status` reader both call
the shared helpers; the IPC handler reads the same file. No second struct, no drift,
**CLI output byte-identical**.

### The handler reads the snapshot (not in-memory daemon state)
`GetMcpStatus` is answered by reading the on-disk snapshot — the same "as of the last
daemon start" semantics the CLI already has, and consistent with the OQ-4 decision in
Conduit. A live in-memory query (post-`list_changed` runtime state) is explicitly out
of scope; if a future need arises it is an additive handler change, not a screen
rewrite. Absent snapshot → an empty/`captured_unix: 0` response the screen renders as
"no servers configured / daemon not yet started," never an error.

### Web rendering reuses the existing screen scaffolding
A new `View::Mcp` variant + nav item + `McpPanel` component + `McpState` signal in
`aivyx-web/src/main.rs`, with one new `QueryResponsePayload::GetMcpStatus` arm in the
message handler — mirroring `SkillsPanel`/`TeamsPanel`. Server cards reuse the Stitch
component kit (status pill, count, collapsible error/stderr) so it matches the rest
of the Studio with no new design tokens.

## 3. Scope

**In:** the `McpServerStatusView` wasm-clean type + `GetMcpStatus` query/response in
`aivyx-ipc` (LN.1); the snapshot helpers lifted to a shared native module + `aivyx-cli`
refactored onto them (LN.1); the daemon `GetMcpStatus` handler (LN.2); the Studio
screen — nav, panel, state, query, response handling, rendering (LN.3); tests; a
served-in-browser live-verify (LN.4). **Out:** editing MCP config from the screen
(add/remove/enable a server — `aivyx-pa.toml` stays the source; a future write-screen
chapter if wanted); a live in-memory/runtime status query (snapshot only); invoking
an MCP tool from the screen; the `/classic` legacy UI (this lands in the modern
Studio only); any change to the Conduit transports or capability model.

## 4. Phase plan (docs-first, small phases per convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **LN.0** 🟡 | **This design contract** | Locked reference; banner flips per phase. |
| **LN.1** ✅ | **Shared types + snapshot home** | DONE. `McpServerStatusView` (wasm-clean, with `connected()`/`failed()` ctors) + `QueryPayload::GetMcpStatus` + `QueryResponsePayload::GetMcpStatus { captured_unix, servers }` in `aivyx-ipc`. New `aivyx_channel::mcp_status` module owns the native fs half — `snapshot_path` / `read_snapshot` (→`Ok(None)` when absent) / `write_snapshot` / `unix_now` over the ipc view (re-exported so `aivyx-cli` needs no direct `aivyx-ipc` dep). `aivyx-cli`'s CD.3 writer + `aivyx-pa mcp status` reader refactored onto them — **`mcp status` output byte-identical** (smoke-verified, both empty + populated). The cli snapshot round-trip test moved to the channel module. ipc + channel + cli suites green; clippy `-D warnings` clean. |
| **LN.2** ✅ | **Daemon handler** | DONE (landed with LN.1 — the exhaustive `QueryPayload` match required it). `daemon_server.rs` answers `GetMcpStatus` via `mcp_status::read_snapshot().ok().flatten()` → `{ captured_unix, servers }`; absent/unreadable → an empty board (`captured_unix: 0`), never an error. |
| **LN.3** ✅ | **Studio screen** | DONE (host build + clippy `-D warnings` clean; WASM serve-verify is LN.4). `View::Mcp` + a config-adjacent "MCP" nav item (after Documents — OQ-4) + `McpPanel` + `McpState` (`servers`/`captured_unix`/`loaded`) threaded through `ws_task` + `mcp_query()` (`GetMcpStatus`) + the response arm fanning the snapshot into state. `McpServerCard` reuses existing chips (`sage`=connected / `error`=failed), shows transport + tool count, and the failure reason + a **collapsible `<details>` stderr tail** (OQ-2). On-open load + a manual **Refresh** button (OQ-3, no poll — the snapshot only changes on a daemon restart). Loading + empty states. Minimal CSS added to `stitch.css` mirroring the skills grid (no new design tokens). |
| **LN.4** ✅ | **Finalize** | DONE. `dx bundle --release` (WASM) clean; `dist/` reassembled (`.br` twins stripped) — the new wasm/css carry the full MCP screen (`MCP Servers` nav, `McpPanel`/`McpServerCard`/`McpState`, `GetMcpStatus`, `McpServerStatusView`, the empty-state copy, `mcp-grid`/`mcp-card`/`mcp-stderr` CSS) and the **release `aivyx-pa` binary embeds the new bundle** (asset hashes + symbols present). **Data path proven live:** `aivyx-pa mcp status` renders a seeded 2-server snapshot (github connected/14 tools; weather failed + stderr tail) through the *exact* `mcp_status::read_snapshot()` the `GetMcpStatus` handler calls — CLI and IPC share one reader, so the screen renders real snapshot data. Full workspace suite + clippy `-D warnings` + `cargo deny` all green. (Live browser auto-drive remains blocked in-sandbox — the harness reaps any long-lived TCP server with signal 16, confirmed identical for a plain `daemon run`; verified via bundle/embed + the live data path per the R–Z precedent.) `dist/` committed; status → COMPLETE. |

**Discipline:** LN.1 lands the shared snapshot home + wire types so LN.2/LN.3 build on
one definition; the `aivyx-cli` refactor must keep `aivyx-pa mcp status` byte-identical
(a pure move, regression-guarded by the CD.4 tests). Test band: **moderate** — mostly
the type round-trip + handler + the web wiring; price **~15–25 new tests** (the screen
itself is verified by the served browser check, per the R–Z precedent).

## 5. Open questions (resolve in-phase)

- **OQ-1 — snapshot home crate (LN.1).** Lift to `aivyx-channel` (locked default —
  `aivyx-cli` already depends on it and it hosts the daemon server) vs. a tiny new
  crate. Lean `aivyx-channel`; revisit only if it pulls an unwanted dep into the wasm
  graph (it won't — the wasm-clean part is the `aivyx-ipc` view; the fs helpers are
  native-only).
- **OQ-2 — stderr in the UI (LN.3).** Show the captured stderr tail inline (collapsible)
  vs. behind a "details" toggle. Lean collapsible-by-default-collapsed so a healthy
  board is compact but a failure is one click from its reason.
- **OQ-3 — refresh affordance (LN.3).** A manual "refresh" button re-issuing
  `GetMcpStatus` vs. query-on-screen-open only. Since the snapshot only changes on a
  daemon restart, on-open (plus a manual refresh button) is enough — no polling.
- **OQ-4 — nav placement (LN.3).** Group the MCP screen near Settings/Agents (config-
  adjacent) vs. its own slot. Lean config-adjacent; it's operator-infrastructure, like
  Settings.

---

*Chapter Lantern lights up what Conduit wired: an operator who adds an MCP server from
the Studio can now see — in the Studio — whether it came up, how many tools it
brought, and, if it didn't, exactly why. The same snapshot the CLI reads, one source
of truth, rendered as the read-only screen the R–Z recipe was built for. No new
capability, no new reach — just the window that was missing.*

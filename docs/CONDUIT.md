# Make Operator-Added MCP Servers Work — env, headers, diagnostics (Chapter Conduit)

> **Status:** 🟡 **PLANNED (CD.0–CD.5).** A **refinement** of the existing MCP
> client (`aivyx-mcp`: stdio / SSE / Streamable-HTTP transports, tools + resources
> + prompts proxies, `*/list_changed` rediscovery). The trigger is a deliberate
> redirection: the §6 "new integrations" backlog (GitHub, weather, Google Tasks)
> is better served **not** by hand-building each one, but by letting the operator
> add them as **MCP servers** — which the implementation already *almost* supports.
> The gap is that you currently **cannot give an MCP server a secret**: stdio spawn
> passes no `env` (so no `GITHUB_PERSONAL_ACCESS_TOKEN`), and SSE/HTTP send no
> operator headers (so no `Authorization: Bearer`). And when a server misconfigures,
> its stderr is discarded, so the operator gets nothing to debug. Conduit closes
> those three gaps — **auth in, auth in, and visibility out** — so a keyed MCP
> server (GitHub being the canonical case) is a few config lines away. No new
> capability base (`mcp.call` already exists), no P10 amendment, no new dependency.

## 1. Why this chapter

The §6 backlog's last bucket is keyed integrations. But Aivyx already ships a mature
MCP client, and the wider ecosystem already ships MCP servers for GitHub, weather,
maps, Google Workspace, and far more. The leverage move is to make **adding any of
them trivial and reliable**, rather than re-implementing three of them in-tree. Three
concrete blockers stand in the way today:

1. **No secret reaches a stdio server.** `StdioTransport::start` spawns
   `Command::new(command).args(args)` with no `.envs(...)`, and `[[mcp_server]]` has
   no `env` field. The official GitHub MCP server is configured *entirely* through
   `GITHUB_PERSONAL_ACCESS_TOKEN` — so it is **impossible to configure today**.
2. **No auth reaches a remote server.** The SSE and Streamable-HTTP transports
   hardcode their headers (`Accept`, `Content-Type`, protocol/session). A remote
   authenticated MCP server needs `Authorization: Bearer …` (or an API-key header) —
   there is no way to supply one.
3. **A broken server is invisible.** Stdio stderr is `Stdio::null()`, and there is no
   `aivyx mcp status`. If a server's command is missing, its token is wrong, or it
   exits on start, the operator sees no tools appear and **no reason why**.

Conduit fixes exactly these: secrets in (stdio `env`), auth in (transport `headers`),
and a way to *see* what happened (captured stderr + a status command).

## 2. Architecture & governance decisions (locked)

### Config + transport wiring + observability — **no new capability surface**
No new tool, `KNOWN_BASES` base, scope, P10 amendment, or dependency. MCP tools
already gate on the existing `mcp.call:<server>:<tool>` qualified scope; that is
untouched. The change is three config fields (`env`, `headers`), threading them into
the three transports, capturing stderr, and one read-only `aivyx mcp status` view.

### `env` mirrors `[[tool_process]]` — including `${HOST_VAR}` interpolation
`[[mcp_server]]` gains an `env` table, exactly the shape `[[tool_process]]` already
has (`Vec<(String, String)>`). A value may be a literal **or** a `${VAR}` reference
resolved from the **daemon's own environment** at load time, so an operator keeps
the actual secret in their shell/systemd environment and out of `aivyx.toml`:

```toml
[[mcp_server]]
name = "github"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_PERSONAL_ACCESS_TOKEN = "${GITHUB_TOKEN}" }   # resolved from the daemon env
```

Wired into `StdioTransport::start` via `cmd.envs(...)`. (Interpolation that
references an unset host var is a clear config error, not a silent empty string.)

### `headers` for the HTTP/SSE transports — same interpolation
`[[mcp_server]]` gains a `headers` table for `sse`/`http` transports, threaded into
the `reqwest` request builders alongside the existing hardcoded headers (operator
headers never override the protocol-required ones). Same `${VAR}` interpolation, so a
bearer token stays out of the config file:

```toml
[[mcp_server]]
name = "remote-tools"
transport = "http"
url = "https://mcp.example.com/mcp"
headers = { Authorization = "Bearer ${EXAMPLE_API_KEY}" }
```

### Capture stderr, don't discard it
Stdio servers stop sending stderr to `/dev/null`. Conduit captures it into a bounded
**ring buffer** (last N lines) surfaced by the status view, so a failed spawn or a
runtime error is diagnosable. Bounded so a chatty server can't grow memory; stdout
(the JSON-RPC channel) is unaffected.

### A read-only `aivyx mcp status`
One operator command lists each configured server: transport, enabled, **connected /
failed**, the **counts of tools / resources / prompts discovered**, and the **last
error** (incl. captured stderr tail). Read-only, no new daemon write path; it answers
"did my server connect, and if not, why?" — the question the silent failure leaves.

### Secrets are operator-trusted, same as every tool-process key
Passing a token to a child process the operator configured is the established Chapter
F/G posture (Gmail/Drive/etc. take keys in their `config.toml`). MCP `env`/`headers`
match it. `${VAR}` interpolation is the *improvement* — it lets the secret live in the
environment rather than on disk. No change to trust tiers or the audit chain.

## 3. Scope

**In:** `env` on `[[mcp_server]]` + stdio spawn wiring (CD.1); `headers` on
`[[mcp_server]]` + SSE/HTTP wiring (CD.2); `${HOST_VAR}` interpolation for both;
captured-stderr ring buffer + `aivyx mcp status` (CD.3); config round-trip tests +
`docs/MCP_RECIPES.md` keyed example (CD.4); a live-verify against a real MCP server,
including the secret path (CD.5). **Out:** a new capability base or P10 amendment;
per-tool MCP `scope_overrides` (the `mcp.call:<server>:<tool>` qualified scope already
allows per-tool role grants — note as deferred); a Studio MCP screen (CLI status
first; web deferred); OAuth flows *for* MCP servers (a server that needs OAuth runs
its own flow — Conduit only carries a static header/token); writing secrets into
`aivyx.toml` on the operator's behalf (interpolation keeps them in the env).

## 4. Phase plan (docs-first, small phases per convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **CD.0** 🟡 | **This design contract** | Locked reference; banner flips per phase. |
| **CD.1** ✅ | **stdio `env` + interpolation** | DONE. `[[mcp_server]] env` table → `McpServerConfig.env: Vec<(String,String)>` (sorted by key), with `interpolate_host_env` resolving `${VAR}` from the daemon environment at load — **unset var = hard `ConfigError::Invalid`**, `$$`→literal `$` escape, unterminated/empty `${}` rejected. Threaded `env` through `McpServerBridge::start_with_sandbox` → `StdioTransport::start` → `cmd.envs(...)` (a sandbox wrapper inherits + passes them). Backward-compat `start()` + the two CLI-flag `McpServerConfig` literals pass empty env. Unblocks the GitHub MCP server (`GITHUB_PERSONAL_ACCESS_TOKEN = "${GITHUB_TOKEN}"`). 3 config tests (literal+interp+sort / unset-var error / `$$` escape); mcp (359) + config (44) suites green; clippy `-D warnings` (all-targets) clean. End-to-end secret delivery proven in CD.5. |
| **CD.2** ✅ | **SSE/HTTP `headers` + interpolation** | DONE. `[[mcp_server]] headers` → `McpServerConfig.headers` (sorted, same `${VAR}` interpolation via the now-field-parameterized `interpolate_host_env`); **rejected on stdio** (no HTTP request to attach them to). Stored in both `SseTransport` + `StreamableHttpTransport` and re-applied to **every** request (SSE GET + POSTs; HTTP POSTs) via a shared `apply_operator_headers` that **skips protocol-reserved names** (case-insensitive: `accept`/`content-type`/`mcp-protocol-version`/`mcp-session-id`) so an operator can add `Authorization` but never clobber the wire contract. `connect()` signatures gained a `headers` param; daemon + test call sites updated. Unblocks remote authenticated servers (`headers = { Authorization = "Bearer ${TOKEN}" }`). 2 config tests (http interpolation+sort / stdio-rejection); config (361) + mcp suites green; clippy `-D warnings` clean. |
| **CD.3** ✅ | **Diagnostics** | DONE. Stdio stderr is captured into a bounded ring buffer (`StderrLog`, last 50 lines) — caller-owned so the daemon holds a clone without touching the `McpTransport` trait; `Stdio::null()` only when no log is passed (backward-compat). The daemon threads a per-server log through `start_with_sandbox`, includes the stderr tail in its startup failure log, and writes an `McpStatusSnapshot` (per-server connected/tool-count or failed/error+stderr-tail) to `$XDG_DATA_HOME/aivyx/mcp-status.json` at the end of the MCP loop. New **`aivyx mcp status`** renders it: `✓ name (transport) — N tool(s)` / `✗ name — FAILED` + reason + captured stderr, with a friendly "no snapshot yet" message. Live-verified the renderer on a crafted snapshot (1/2 connected). mcp + cli suites green; clippy `-D warnings` clean. |
| **CD.4** ✅ | **Docs + tests** | DONE. `docs/MCP_RECIPES.md` gains a **Secrets & auth** note (native `env`/`headers` + `${VAR}` + `$$` escape + unset-var-is-error) and a **Diagnosing** note (`aivyx mcp status`); every keyed recipe (github/gitlab/postgres/brave-search/slack) switched from the sandbox `--setenv` workaround to the clean native `env = { … = "${VAR}" }` field, and a remote `headers = { Authorization = "Bearer ${…}" }` example added. Tests: `mcp status` parse + extra-arg rejection, `McpStatusSnapshot` serde round-trip, `format_stderr_tail` 5-line cap (the CD.1/CD.2 config tests already cover env/headers interpolation + precedence). clippy `-D warnings` clean. |
| **CD.5** | **Finalize + live-verify** | Drive a real MCP server end-to-end — at minimum one needing `env` (prove the secret path) and confirm `aivyx mcp status` reports it connected with its tools; full workspace suite + clippy + `cargo deny` green; chapter memory; status → COMPLETE. |

**Discipline:** CD.1 lands the `env` + interpolation spine that CD.2 reuses for
headers. CD.3 is independent (observability) and could ship first, but follows so the
status view can already show a *successfully keyed* server. Test band: **moderate** —
config-parsing + interpolation edge cases dominate; price **~25–35 new tests**.

## 5. Open questions (resolve in-phase)

- **OQ-1 — interpolation syntax (CD.1).** `${VAR}` (locked default, shell-familiar)
  vs. a typed `{ from_env = "VAR" }`. `${VAR}` is terser and matches operator
  expectation; confirm it can't collide with a literal value a server legitimately
  needs (escape with `$${...}` if so).
- **OQ-2 — unset-var behavior (CD.1).** Hard config error (locked default — a missing
  token should fail loudly at startup) vs. warn + omit. Lean error.
- **OQ-3 — stderr buffer size (CD.3).** ✅ **Resolved: last 50 lines** (a line ring
  buffer reads cleanly in `mcp status`; bounded so a chatty server can't grow memory).
- **OQ-4 — status transport (CD.3).** ✅ **Resolved: a daemon-written snapshot file**
  (not a live IPC query). The snapshot carries *real* connected/failed + tool counts +
  the failure reason/stderr from the **last daemon start** — which is exactly the
  "did my server come up, and why not?" question — while keeping the surface small
  (no `aivyx-ipc` protocol change). Live runtime state (post-`list_changed`) is out of
  scope; revisit with an IPC query only if a use case needs it.
- **OQ-5 — live-verify target (CD.5).** Which real server proves the secret path — the
  GitHub MCP server (needs a real PAT) vs. a filesystem/everything server wrapped to
  require a dummy env var. Lean the latter for a credential-free, deterministic CI-able
  proof, with the GitHub recipe documented.

---

*Chapter Conduit turns Aivyx's MCP client from "connects to servers" into "connects to
the servers operators actually want" — the keyed ones. The §6 integrations backlog
doesn't need three hand-built tool-processes; it needs a token to reach the GitHub MCP
server, a header to reach a remote one, and a way to see why a server didn't come up.
Three small wires — secret in, auth in, status out — and the whole MCP ecosystem
becomes the integration story, no new capability surface required.*

# Curated MCP Server Recipes

This is the canonical catalog of MCP servers that pair cleanly with
Aivyx today. Each recipe shows a paste-able `[[mcp_server]]` block,
an inline `[mcp_server.sandbox]` block (sandbox-by-default is the
Phase 55 substrate posture; copy-paste should produce a sandboxed
config out of the gate), the required environment variables, and
notes on what capability scopes the agent gets when the server is
enabled.

For the CLI surface, run `aivyx mcp recipes` to list every recipe
in-shell, or `aivyx mcp recipes <name>` to print one recipe's worked
snippet directly to stdout (handy for piping into `aivyx.toml`).

> **Sandboxing note.** Every recipe in this doc shows a
> `[mcp_server.sandbox]` block. The wrapper choices below (`bwrap`,
> `firejail`, `docker`) are illustrative; substitute whatever
> command-wrapper sandbox you already trust on your operator host.
> See [`docs/TOOL_SDK.md`](TOOL_SDK.md) §9 for the substrate-level
> sandbox-layer reference; the wrapper / args shape is the same one
> `[[tool_process]]` uses.

> **Capability-scope note.** Once an `[[mcp_server]]` entry is
> loaded, the daemon registers each of its tools under
> `mcp.call:<server-name>:<tool-name>`. Roles that need to invoke
> the server must declare a matching scope — either `mcp.call`
> (broad: any MCP tool), `mcp.call:<server-name>` (mid: any tool on
> one named server), or `mcp.call:<server-name>:<tool-name>`
> (narrow: one specific tool). The recipes below use the
> middle granularity as the default since "if this server is
> enabled, the agent can use any of its tools" is the most common
> operator intent.

## Catalog at a glance

| Recipe | Operator-touch frequency | Notes |
|---|---|---|
| [`filesystem`](#filesystem) | High | Read/write a single directory |
| [`github`](#github) | High | Repo / issue / PR operations |
| [`gitlab`](#gitlab) | Medium | Symmetric for GitLab users |
| [`sqlite`](#sqlite) | Medium | Local `.db` file |
| [`postgres`](#postgres) | Medium | Read-only Postgres |
| [`time`](#time) | Medium | Timezone math; no deps, no network |
| [`fetch`](#fetch) | Medium | HTTP GET into markdown |
| [`brave-search`](#brave-search) | Medium | Brave Search API |
| [`slack`](#slack) | Medium | Channel read + post |
| [`memory`](#memory) | Low | Knowledge-graph (distinct from Aivyx's own memory) |
| [`puppeteer`](#puppeteer) | Low | Headless browser; higher blast radius |
| [`everything`](#everything) | First-run | Reference / smoke-test server |

---

## filesystem

Read/write files inside a configured directory. The official
`@modelcontextprotocol/server-filesystem` is the most-asked-for
first MCP server.

**Required env:** none.
**Capability scopes the agent gets:** `mcp.call:fs-local:*`.

```toml
[[mcp_server]]
name = "fs-local"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/me/projects"]

[mcp_server.sandbox]
# bubblewrap binds the same path the server is told about so a
# typo in the args can't escape into $HOME.
wrapper = "bwrap"
args = [
    "--ro-bind", "/usr", "/usr",
    "--ro-bind", "/etc", "/etc",
    "--bind", "/home/me/projects", "/home/me/projects",
    "--dev", "/dev", "--proc", "/proc",
    "--unshare-net",   # filesystem server does not need network
    "--",
]
```

Verify it works: after `aivyx daemon stop && aivyx`, ask the
agent to "list the files in my projects directory" — it should
call `mcp.call:fs-local:list_directory` and return a structured
listing.

---

## github

Read/write GitHub repos, issues, and pull requests via the GitHub
REST API.

**Required env:** `GITHUB_PERSONAL_ACCESS_TOKEN` with the scopes
the agent needs (typically `repo` for private repos +
`read:org`).
**Capability scopes the agent gets:** `mcp.call:github:*`.

```toml
[[mcp_server]]
name = "github"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]

[mcp_server.sandbox]
# Network-only sandbox: no filesystem access, network kept so
# the server can reach api.github.com.
wrapper = "bwrap"
args = [
    "--ro-bind", "/usr", "/usr",
    "--ro-bind", "/etc", "/etc",
    "--ro-bind", "/etc/ssl", "/etc/ssl",
    "--dev", "/dev", "--proc", "/proc",
    "--setenv", "GITHUB_PERSONAL_ACCESS_TOKEN", "${GITHUB_PERSONAL_ACCESS_TOKEN}",
    "--",
]
```

---

## gitlab

GitLab projects / issues / merge requests via the GitLab REST
API. Symmetric to the GitHub recipe above.

**Required env:** `GITLAB_PERSONAL_ACCESS_TOKEN`. Optional
`GITLAB_API_URL` (defaults to `https://gitlab.com`) for
self-hosted instances.
**Capability scopes the agent gets:** `mcp.call:gitlab:*`.

```toml
[[mcp_server]]
name = "gitlab"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-gitlab"]

[mcp_server.sandbox]
wrapper = "bwrap"
args = [
    "--ro-bind", "/usr", "/usr",
    "--ro-bind", "/etc", "/etc",
    "--ro-bind", "/etc/ssl", "/etc/ssl",
    "--dev", "/dev", "--proc", "/proc",
    "--setenv", "GITLAB_PERSONAL_ACCESS_TOKEN", "${GITLAB_PERSONAL_ACCESS_TOKEN}",
    "--",
]
```

---

## sqlite

Query / mutate a local SQLite database. The server takes one path
argument naming the `.db` file.

**Required env:** none.
**Capability scopes the agent gets:** `mcp.call:sqlite:*`.

```toml
[[mcp_server]]
name = "sqlite"
command = "npx"
args = [
    "-y",
    "@modelcontextprotocol/server-sqlite",
    "/home/me/data/notes.db",
]

[mcp_server.sandbox]
# Bind only the directory holding the db file; no network.
wrapper = "bwrap"
args = [
    "--ro-bind", "/usr", "/usr",
    "--ro-bind", "/etc", "/etc",
    "--bind", "/home/me/data", "/home/me/data",
    "--dev", "/dev", "--proc", "/proc",
    "--unshare-net",
    "--",
]
```

---

## postgres

Read-only Postgres queries against a configured connection. The
official server is read-only by design — even an operator who
grants write privileges at the database level cannot expose write
tools through this server.

**Required env:** `POSTGRES_CONNECTION_STRING` (e.g.
`postgresql://user:pass@host:5432/dbname`).
**Capability scopes the agent gets:** `mcp.call:postgres:*`.

```toml
[[mcp_server]]
name = "postgres"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-postgres"]

[mcp_server.sandbox]
wrapper = "bwrap"
args = [
    "--ro-bind", "/usr", "/usr",
    "--ro-bind", "/etc", "/etc",
    "--ro-bind", "/etc/ssl", "/etc/ssl",
    "--dev", "/dev", "--proc", "/proc",
    "--setenv", "POSTGRES_CONNECTION_STRING", "${POSTGRES_CONNECTION_STRING}",
    "--",
]
```

---

## time

Timezone-aware date / time math: current time, conversion,
arithmetic. Small, no deps, no network — the canonical
"hello world" recipe.

**Required env:** none.
**Capability scopes the agent gets:** `mcp.call:time:*`.

```toml
[[mcp_server]]
name = "time"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-time"]

[mcp_server.sandbox]
# Pure-compute server: no filesystem, no network.
wrapper = "bwrap"
args = [
    "--ro-bind", "/usr", "/usr",
    "--ro-bind", "/etc", "/etc",
    "--dev", "/dev", "--proc", "/proc",
    "--unshare-net",
    "--",
]
```

---

## fetch

HTTP GET into clean markdown. Respects `robots.txt`. Mostly
overlaps with Aivyx's first-party `web.fetch` tool (Phase 12); use
this recipe when an agent wants the markdown-conversion path
rather than raw response bytes.

**Required env:** none.
**Capability scopes the agent gets:** `mcp.call:fetch:*`.

```toml
[[mcp_server]]
name = "fetch"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-fetch"]

[mcp_server.sandbox]
wrapper = "bwrap"
args = [
    "--ro-bind", "/usr", "/usr",
    "--ro-bind", "/etc", "/etc",
    "--ro-bind", "/etc/ssl", "/etc/ssl",
    "--dev", "/dev", "--proc", "/proc",
    "--",
]
```

---

## brave-search

Web + local search via the Brave Search API. Aivyx already ships
a bundled `web-search` MCP server with its own Brave fallback
(Phase 46) — use this recipe only if you want the official Brave
server's exact tool surface (`web_search` + `local_search`)
rather than the bundled Aivyx surface.

**Required env:** `BRAVE_API_KEY` (sign up at
`api.search.brave.com`).
**Capability scopes the agent gets:** `mcp.call:brave-search:*`.

```toml
[[mcp_server]]
name = "brave-search"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-brave-search"]

[mcp_server.sandbox]
wrapper = "bwrap"
args = [
    "--ro-bind", "/usr", "/usr",
    "--ro-bind", "/etc", "/etc",
    "--ro-bind", "/etc/ssl", "/etc/ssl",
    "--dev", "/dev", "--proc", "/proc",
    "--setenv", "BRAVE_API_KEY", "${BRAVE_API_KEY}",
    "--",
]
```

---

## slack

Read channel history; post messages. The `slack` MCP server is
the "agent calls into Slack as a tool" surface — distinct from the
planned Phase 108 first-party Aivyx-Slack channel adapter, which
is the "operator talks to Aivyx from Slack" surface. They compose.

**Required env:** `SLACK_BOT_TOKEN` (`xoxb-…`) and `SLACK_TEAM_ID`.
**Capability scopes the agent gets:** `mcp.call:slack:*`.

```toml
[[mcp_server]]
name = "slack"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-slack"]

[mcp_server.sandbox]
wrapper = "bwrap"
args = [
    "--ro-bind", "/usr", "/usr",
    "--ro-bind", "/etc", "/etc",
    "--ro-bind", "/etc/ssl", "/etc/ssl",
    "--dev", "/dev", "--proc", "/proc",
    "--setenv", "SLACK_BOT_TOKEN", "${SLACK_BOT_TOKEN}",
    "--setenv", "SLACK_TEAM_ID", "${SLACK_TEAM_ID}",
    "--",
]
```

---

## memory

Knowledge-graph memory the agent maintains across turns. Distinct
from Aivyx's first-party `memory.*` tools — those write to
encrypted `KeyDomain::Memory` in the redb store; this MCP server
keeps a JSON knowledge graph the agent can walk by entity /
relation. Useful when an agent wants structured relational recall
that isn't a fit for the topic-keyed Aivyx memory.

**Required env:** none. The server writes its graph to a JSON
file inside the sandbox bind path.
**Capability scopes the agent gets:** `mcp.call:kg-memory:*`.

```toml
[[mcp_server]]
name = "kg-memory"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-memory"]

[mcp_server.sandbox]
wrapper = "bwrap"
args = [
    "--ro-bind", "/usr", "/usr",
    "--ro-bind", "/etc", "/etc",
    "--bind", "/home/me/.aivyx/kg-memory", "/home/me/.aivyx/kg-memory",
    "--dev", "/dev", "--proc", "/proc",
    "--unshare-net",
    "--",
]
```

---

## puppeteer

Headless-browser automation: navigate, click, screenshot,
`page.evaluate`. Higher blast radius than read-only servers — the
sandbox must constrain what URLs the headless browser can be told
to load.

**Required env:** none.
**Capability scopes the agent gets:** `mcp.call:puppeteer:*`.

```toml
[[mcp_server]]
name = "puppeteer"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-puppeteer"]

[mcp_server.sandbox]
# Docker is a better fit than bwrap for Chromium — it needs more
# pieces than a bwrap one-liner cleanly carries.
wrapper = "docker"
args = [
    "run", "--rm", "-i",
    "--cap-drop=ALL",
    "--security-opt=no-new-privileges",
    "--network=host",   # puppeteer needs internet
    "node:20-slim",
    "--",
]
```

---

## everything

The official reference / test server. Exposes one of each tool
kind so an operator can verify Aivyx's MCP wiring without paying
for a real-API setup. Useful first thing after `aivyx init`.

**Required env:** none.
**Capability scopes the agent gets:** `mcp.call:everything:*`.

```toml
[[mcp_server]]
name = "everything"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-everything"]

[mcp_server.sandbox]
wrapper = "bwrap"
args = [
    "--ro-bind", "/usr", "/usr",
    "--ro-bind", "/etc", "/etc",
    "--dev", "/dev", "--proc", "/proc",
    "--unshare-net",
    "--",
]
```

---

## Adding a recipe to this catalog

Recipes are static data in two places — `docs/MCP_RECIPES.md`
(this file) and `crates/aivyx-channel/src/bin/aivyx_modules/mcp_recipes.rs`'s
`RECIPES` slice. The `aivyx mcp recipes` CLI surface reads from
the slice; the doc is the operator-facing reference. Both must be
kept in sync — if you add a recipe to one, add the matching half
to the other in the same commit. The module's
`every_recipe_has_*` tests enforce the registry's internal
contract; the doc-side counterpart is operator review.

## What's not in this catalog

Recipes shipped here are limited to MCP servers that:

1. Have an authoritative `@modelcontextprotocol/server-*` package
   on npm (the "official" set), or
2. Have been used in tree by the Aivyx operator long enough to
   call them stable.

Servers worth a recipe but not yet added land via a future
Chapter D follow-on (the Phase 106 recipes-only scope decision
deliberately kept new bundled-server code out of scope; new
recipes can land in a focused docs-only update).

See also:
- `examples/aivyx.toml` for the broader config-file context each
  recipe drops into.
- [`docs/TOOL_SDK.md`](TOOL_SDK.md) §9 for the substrate-level
  sandbox-layer reference.
- [`docs/AUDIT_EXPORT.md`](AUDIT_EXPORT.md) for inspecting which
  MCP tools an agent has actually called (`aivyx audit export |
  jq '.event.tool_id'`).

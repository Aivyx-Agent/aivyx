# `aivyx-coder` as a Nonagon specialist — design

_2026-08-20._ "Piece B" of the ecosystem-cohesion chapter, following
`aivyx-coder`'s own MCP-server frontend ("Piece A," shipped the same
day — see `aivyx-coder/docs/superpowers/specs/2026-08-20-mcp-server-
frontend-design.md`). Reached via a mid-brainstorm reframe: the original
ask ("a unified frontend for the ecosystem") turned out to already be
mostly delivered by `aivyx`'s existing, shipped Studio "Missions" screen
(goal in, routed through a Nonagon team, gated execution) — the real gap
was never the frontend, it was this: nothing yet lets a Nonagon team
actually delegate to `aivyx-coder`.

## Why this needs almost no new engine code

Two mechanisms already shipped on either side of the boundary make this
close to a pure wiring-and-documentation task, not a new-primitive one:

- **`aivyx-coder`'s own `max_access_level` config is the real security
  ceiling**, regardless of what any caller requests. A specialist's own
  model can ask for `"execute"` on a `code` call; if the operator
  configured `aivyx-coder --mcp-server` with a lower ceiling, the call
  is rejected before any agent is even built on that side. This
  enforcement is already shipped, tested, and independently reviewed —
  nothing on `aivyx`'s side needs to re-implement or trust it, only rely
  on it.
- **`aivyx`'s own MCP client (`crates/aivyx-mcp`) and NT-02 attenuation
  already cover an arbitrary bridged MCP server's tools generically.**
  Once `aivyx-coder --mcp-server` is configured as one more
  `[[mcp_server]]` entry, its `code`/`code_reply` tools appear in the
  daemon's tool registry under `mcp.call:<server-name>:<tool-name>`
  (`docs/MCP_RECIPES.md`'s own documented convention) — a Nonagon
  specialist that declares a matching `mcp.call:...` capability scope
  can call them exactly like any other tool, attenuated by the lead's
  own grant exactly like any other tool. `aivyx-team`'s own
  `filter_tools` already has a marker (`mcp.call`) specifically for
  admitting a dynamically-named, server-defined tool set into a
  specialist's allowlist — added for a different MCP server, reused
  here unchanged.

Given both, the "coder" specialist stays a **completely ordinary**
Nonagon roster member: its own `ConcreteAgent`, its own `LlmPlanner`,
its own soul — the only thing new about it is that its tool allowlist
happens to include two more tools whose real implementation lives in a
different process entirely. No new `Agent` implementation, no new
`SpecialistFactory` branch, no new trust-boundary primitive.

## What this actually delivers: two documentation artifacts

**1. A new `aivyx-coder` entry in `docs/MCP_RECIPES.md`**, following the
catalog's own established shape exactly (verified against real entries
— `filesystem`, `github`, etc. — each has `[[mcp_server]]`
name/command/args, an optional `[mcp_server.sandbox]` block, a
"Capability scopes the agent gets" line, and a verify-it-works step):

```toml
[[mcp_server]]
name = "aivyx-coder"
command = "aivyx-coder"
args = ["--mcp-server"]
```

One honest deviation from every other entry in the catalog: every
existing recipe recommends an additional `bwrap`/`firejail`/`docker`
sandbox wrapper as the *load-bearing* protection, because the server is
unknown third-party code. `aivyx-coder` is not that — it is itself an
already-audited Aivyx-family product with its own Landlock/seccomp
confinement and its own tiered access ceiling. The recipe should say so
explicitly: the extra wrapper layer here is optional defense-in-depth,
not the thing actually keeping the operator safe (that's
`[mcp_server].max_access_level`, configured on `aivyx-coder`'s own
side, documented as a required prerequisite step in this same recipe).

**2. A worked "coder" `TeamMember` example in `docs/NONAGON.md`**,
matching that doc's existing §9 "Worked example" convention and the
real `[[team.member]]` TOML shape already used by
`crates/verticals/aivyx-kitchen/assets/kitchen-boh.toml` (fields:
`name`/`role`/`soul`/`tool_allowlist`/`capability_scopes`/
`trust_ceiling`) — a `capability_scopes` entry granting
`mcp.call:aivyx-coder` (mid-granularity — matches `MCP_RECIPES.md`'s
own stated default rationale, "if this server is enabled, the agent can
use any of its tools is the most common operator intent," and
`aivyx-coder`'s server only ever exposes the two related `code`/
`code_reply` tools; exact wildcard/suffix syntax to be verified against
`aivyx-capability`'s real `Scope` parser when this is planned, not
asserted here), and a soul that explains the three tiers (`plan`/
`edit`/`execute`) plainly enough that the specialist's own model
reasonably picks one per task — the only place tier selection is
guided at all, since (per the section above) it is not a security
control, only a quality one.

## Explicitly not done

- **No `default_nonagon` change.** This stays pure operator/vertical
  opt-in, matching every other MCP recipe's own precedent (none are
  baked into the free-core default roster) and the fact that
  `aivyx-coder` is a separate product the operator must separately
  install and run — `default_nonagon` must keep working with zero
  external dependencies.
- **No changes to `aivyx-mcp`, `aivyx-team`, or `aivyx-capability`.**
  Every mechanism this design relies on (MCP client bridging, the
  `mcp.call` marker, NT-02 attenuation) is reused exactly as it exists
  today.
- **Approach B (a new pass-through remote `Agent` with no local LLM)**
  was considered and explicitly rejected during brainstorming — real
  new machinery (a new `Agent` impl, a new `SpecialistFactory` branch,
  a genuinely new trust-boundary mechanism) to avoid a double-LLM-loop
  cost that Approach A accepts as a known, named trade-off instead.

## Testing / verification

Live-E2E on the real rig, matching this project's own established
convention (every other Nonagon/MCP feature in `aivyx/ROADMAP.md` was
verified this way, not just unit-tested): a real `aivyx-coder
--mcp-server` process with a real `[mcp_server]` config, a real `aivyx`
daemon configured with the new recipe, a real Nonagon mission that
delegates an actual coding task through the bridged `code`/`code_reply`
tools end to end — checked against real output (a file genuinely
written, a command genuinely run), not just "the call didn't error."
No new automated test suite is anticipated beyond what already exists
for `aivyx-mcp`'s generic bridging and `aivyx-team`'s `mcp.call` marker
admission (both already covered by their own existing tests) — this
design adds no new code path for either to test.

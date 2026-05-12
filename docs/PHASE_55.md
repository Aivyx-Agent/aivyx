# Phase 55 — MCP Server Sandbox Layer

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Close the **THREAT_MODEL.md §5.2** ("malicious MCP server")
sandbox gap that the project-review audit surfaced.

Phase 52 added a generic command-wrapper sandbox to
`[[tool_process]]` (Aivyx-native third-party tools) via
`aivyx-tool::SandboxConfig`. Phase 55 ports the same shape to
`[[mcp_server]]` (third-party MCP servers) via a parallel
`aivyx-mcp::SandboxConfig`. The asymmetry between the two
adapter surfaces — sandbox-able vs not — collapses.

This is the project's **first phase opened in response to
operator-feedback-shaped pressure** rather than as part of the
Phase 0–54 forward arc. The Chapter A retrospective predicted
this: "the next numbered phase opens in response to a specific
need." The specific need: an audit finding from the post-Phase-54
project review naming the gap explicitly.

## Why now

1. **The threat is documented and the fix is small.**
   `THREAT_MODEL.md` §5.2 already names this risk. Phase 52
   proved the wrapper-layer design works. Phase 55 ports the
   pattern; no design exploration.

2. **The asymmetry was unintentional.** Phase 52 wired
   `aivyx-tool`; `aivyx-mcp` was simply not in scope. Phase 55
   removes the asymmetry.

3. **Real operator hostility cost.** Adding `[[mcp_server]]`
   entries is one of the most common things an operator does
   to extend Aivyx (MCP servers from GitHub, Slack threads,
   blog posts — the npm-style discovery surface the Hermes
   threat model called out). Sandbox-ability matters most
   here.

## Architecture

```
Today (Phase 49–54):
  StdioTransport::start(command, &args) →
    tokio::process::Command::new(command).args(args).spawn()

After Phase 55:
  StdioTransport::start(command, &args, Some(SandboxConfig{wrapper, args})) →
    tokio::process::Command::new(wrapper).args(wrapper_args)
        .arg(command).args(command_args).spawn()
```

Identical wire-level behavior to Phase 52's `[tool_process.sandbox]`
path. The wrapper is responsible for setting up isolation
(mount namespaces, network namespaces, seccomp, etc.) and then
`exec`'ing the real command. Standard sandbox tools all support
this `wrapper [wrapper-args...] command [command-args...]` shape.

**SSE transport is out of scope.** SSE talks to a *remote* MCP
server over HTTP/SSE; there's no local child process to
sandbox. SSE's threat profile is network-shaped (covered by
`THREAT_MODEL.md` §5.4 + the `net.fetch` capability scope).
Phase 55 is stdio-only.

## Entry baseline

- Rust tests: 984
- Python conformance tests: 24
- Workspace crates: 12
- Clippy warnings: 0
- Deferral backlog: 4
- DESIGN.md streak: 1 phase (last touched A3 addendum at Phase 54)
- PRODUCT.md streak: 5 phases (last touched at Phase 50)
- `aivyx-core/src/lib.rs` streak: 3 phases (last touched at Phase 51)

## Q-block — resolutions

**Q1: Reuse `aivyx-tool::SandboxConfig`, lift to `aivyx-config`,
or parallel type in `aivyx-mcp`?** → **Parallel type in
`aivyx-mcp`.** Security and future-proofing converge here:

- Security: no runtime difference between the options.
- Future-proofing: MCP and Aivyx-native tools have adjacent
  but not identical sandbox needs. MCP servers may
  legitimately need network access; first-party tools
  generally don't. Independent types let the two surfaces
  diverge without a backwards-incompatible refactor.
- Crate-graph hygiene: no new dep edges. `aivyx-mcp` stays
  independent of `aivyx-tool`. The binary translates from
  `aivyx_config::SandboxConfig` to both runtime types
  separately, matching the pattern Phase 52 established.

**Q2: Default behavior?** → **Off.** Same posture as Phase 52
— backwards compatible, explicit opt-in. Operators with
existing `[[mcp_server]]` entries see no change.

**Q3: SSE transport sandbox?** → **No.** SSE is HTTP-over-network
to a remote MCP server; no local child process exists.
Sandbox is stdio-only. SSE's threat profile is documented
under `THREAT_MODEL.md` §5.4.

**Q4: Where do the docs live — extend `TOOL_SDK.md` §9 or add a
new section?** → **Extend `TOOL_SDK.md` §9** with a short
subsection noting the same sandbox-wrapper concept applies to
`[mcp_server.sandbox]`. Co-located with the existing sandbox
worked examples; no new doc file. MCP isn't an Aivyx-defined
contract, so there's no `MCP_SDK.md` shape to maintain.

**Q5: Threat-model update?** → **Yes — rewrite
`THREAT_MODEL.md` §5.2 in the same shape Phase 52 used to
narrow §5.6.** Acknowledge the Phase 55 wrapper layer.
Document the residual risk (MCP server still runs with
operator OS authority unless wrapped) as an in-scope
limitation, since the wrapper is operator-supplied.

**Q6: Streak predictions?**

| Streak target | Predicted | Notes |
|---|---|---|
| DESIGN.md | untouched (2) | No contract change. |
| PRODUCT.md | untouched (6) | No commitment edits. |
| `aivyx-core/src/lib.rs` | untouched (4) | Work lives in `aivyx-mcp` + `aivyx-config` + binary + docs. |

## Tasks

### Task 1 — Open commit + scaffold

This file. ROADMAP active marker. README row.

### Task 2 — `aivyx-mcp::SandboxConfig` + `StdioTransport` wrapper

`aivyx-mcp/src/lib.rs` re-exports a new `SandboxConfig`:

```rust
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub wrapper: String,
    pub args: Vec<String>,
}
```

`StdioTransport::start` signature gains an `Option<SandboxConfig>`
parameter. When `Some`, the effective spawn is
`wrapper wrapper_args... command command_args...`. Spawn-failure
error message reports the wrapper name when sandboxed (same
ergonomic Phase 52 wired into `aivyx-tool`).

Unit test: spawn-failure with a bogus wrapper reports the
wrapper name, not the wrapped command.

### Task 3 — `[mcp_server.sandbox]` TOML schema + binary wiring

`aivyx-config`:
- `RawMcpServer` gains an optional nested
  `sandbox: Option<RawSandbox>` field (reusing the existing
  `RawSandbox` struct from Phase 52).
- `McpServerConfig` gains
  `sandbox: Option<aivyx_config::SandboxConfig>` (reusing the
  existing runtime type).
- Loader validates non-empty wrapper, same posture as the
  Phase 52 `[tool_process.sandbox]` loader.

Binary (`aivyx-channel/src/bin/aivyx.rs`):
- At the MCP spawn call site, translate
  `aivyx_config::SandboxConfig` → `aivyx_mcp::SandboxConfig`
  if present.
- Pass through to `StdioTransport::start`.

Config-load tests pin the round-trip.

### Task 4 — Integration test using `env` as no-op wrapper

`crates/aivyx-mcp/tests/sandbox_e2e.rs` (or extension of an
existing test file): spawn an inline echo-style MCP server
through `StdioTransport` with
`SandboxConfig { wrapper: "env", args: ["AIVYX_MCP_SANDBOX_PROBE=1"] }`.
Drive a JSON-RPC `initialize` handshake. Assert the
round-trip works as the no-sandbox path does.

POSIX `env` is universally available; this test runs anywhere
`cargo test` runs without depending on bwrap/firejail/docker.

### Task 5 — `THREAT_MODEL.md` §5.2 narrowing + `TOOL_SDK.md` §9 cross-ref

`docs/THREAT_MODEL.md` §5.2 rewritten in the same shape
Phase 52 used to narrow §5.6:
- Status update acknowledging Phase 55 added the wrapper layer.
- The residual risk (MCP server runs with operator OS authority
  even when wrapped, depending on the wrapper's policy) stays
  in scope, named explicitly.
- Pointer to `TOOL_SDK.md` §9 for the worked examples.

`docs/TOOL_SDK.md` §9 gains a short subsection
*"Sandboxing MCP servers"* noting the same wrapper concept
applies to `[mcp_server.sandbox]`, with a worked TOML example
matching the `[tool_process.sandbox]` one.

### Task 6 — Exit freeze

Backfill exit stats, ship records, deferral list. Mark ROADMAP
frozen. Update `docs/README.md`. The Phase 55 trigger
(THREAT_MODEL §5.2 audit finding) is closed.

## Ship records

| Task | Commit | Notes |
|---|---|---|
| 1 | `eeaaad3` | scaffold |
| 2 | `ef11e22` | aivyx-mcp::SandboxConfig + StdioTransport wrapper — 986 tests |
| 3 | `c7d4080` | [mcp_server.sandbox] TOML schema + binary wiring — 990 tests |
| 4 | `e460442` | sandbox e2e integration test with `env` wrapper — 992 tests |
| 5 | `4edd61d` | THREAT_MODEL §5.2 narrowing + TOOL_SDK.md §9 MCP subsection |

## Deferrals carried into the phase

1. Live audit push (P47 Q4)
2. Read-write dashboard inspection (P47 Q6)
3. Conformance harness as a Rust crate (P48 Q5)
4. IPC stability window commitment (P48 Q6)

## Net-new deferrals (predicted)

None expected. Phase 55 is a Phase 52 pattern port; no
architectural exploration that could surface new deferrals.

## Exit criteria

- [x] `aivyx-mcp::SandboxConfig` exists.
- [x] `StdioTransport::start` accepts an
  `Option<SandboxConfig>` parameter; applies the wrapper
  when present.
- [x] `[mcp_server.sandbox]` TOML schema loads and validates.
- [x] Integration test using `env` as wrapper drives a
  JSON-RPC `initialize` + `tools/list` handshake end-to-end.
- [x] `THREAT_MODEL.md` §5.2 narrowed to acknowledge Phase 55.
- [x] `TOOL_SDK.md` §9 has a subsection on MCP sandbox with
  a worked example.
- [x] DESIGN.md untouched (streak → 2).
- [x] PRODUCT.md untouched (streak → 6).
- [x] `aivyx-core/src/lib.rs` untouched (streak → 4).
- [x] Zero clippy warnings.
- [x] Rust tests 984 → 992 (+8).

## Exit stats

- Rust tests: 984 → 992 (+8: 2 stdio unit, 4 config-load,
  2 sandbox e2e)
- Python conformance tests: 24 (unchanged)
- Workspace crates: 12 (unchanged)
- Clippy warnings: 0
- Deferral backlog: 4 → 4

### Streak outcomes

| Streak target | Predicted | Actual | New streak |
|---|---|---|---|
| DESIGN.md | untouched (2) | untouched | 2 |
| PRODUCT.md | untouched (6) | untouched | 6 |
| `aivyx-core/src/lib.rs` | untouched (4) | untouched | **4** |

All three predictions correct. lib.rs streak held at 4. Work
lived entirely in `aivyx-mcp` + `aivyx-config` + binary + docs.

### Items closed

| Item | Source | What was closed |
|---|---|---|
| MCP supply-chain sandbox gap | THREAT_MODEL.md §5.2 | `[mcp_server.sandbox]` block parallel to Phase 52's `[tool_process.sandbox]`. Operators on hardened deployments can wrap MCP servers with bubblewrap / firejail / docker / sandbox-exec. |

### Posture established

Phase 55 is **the project's first post-Chapter-A phase**. It
demonstrates that the "wait for pressure" posture from the
Chapter A retrospective produces well-scoped, fast-shipping
phases: the trigger was an audit finding from the
post-Phase-54 project review; the design was already proven
(Phase 52); execution took six tasks; nothing surprising
surfaced. Future post-Chapter-A phases should aim for the
same shape — operator-feedback-shaped need + proven-pattern
port + small task list.

### Deferrals carried forward (4)

Unchanged from Phase 54 exit:

1. Live audit push (P47 Q4)
2. Read-write dashboard inspection (P47 Q6)
3. Conformance harness as a Rust crate (P48 Q5)
4. IPC stability window commitment (P48 Q6)

### Operator-side verification

A real-sandbox smoke test would spawn a real bwrap or firejail
wrapper around an installed MCP server (e.g., a community
MCP server like `mcp-server-everything`) and assert the
sandbox actually blocks access to `~/.ssh/`. Recorded as
non-urgent operator-discretion verification, same posture as
the Phase 52 sandbox layer.

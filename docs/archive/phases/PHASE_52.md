# Phase 52 — Tool Process Sandbox Layer

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../../../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Third phase of **Chapter A — Foundation Closeout**. Adds a
**generic command-wrapper** sandbox layer on top of the Phase
49 + 50 tool-process IPC. Closes the Phase 49 net-new deferral
("per-tool sandboxing on top of process isolation") and
narrows `THREAT_MODEL.md` §5.6 from "we ship process isolation
only" to "operators can layer their own sandbox without us
opining."

The design choice that keeps the phase small: Aivyx supplies
the **policy slot**, the operator supplies the **policy**. We
don't bundle bubblewrap, firejail, Docker, sandbox-exec, or
seccomp filter logic into the binary; we accept a wrapper
command + args at `[[tool_process]]` config time and prepend
it to every spawn. Operators on Linux pick bwrap/firejail;
operators on macOS pick `sandbox-exec` or Docker; operators on
a personal workstation pick none.

## Why now

1. **Forward-commitment ledger is closed; Chapter A is
   hardening.** Phase 49 admitted the Phase 49 net-new
   deferral. Phase 50 closed P12 fully. Phase 51 was mechanical
   cleanup. Phase 52 is the hardening sibling.

2. **Threat model § 5.6 explicitly flags this.** The phase
   delivers what the threat model already commits to as
   future-phase work: "a future phase may add OS-level
   sandboxing on top of the IPC isolation."

3. **Hermes Agent ships Docker sandboxing; OpenClaw ships
   OpenShell.** The comparison audit surfaced this as the
   most visible competitive-hardening gap. Phase 52 closes
   it via a different shape (wrapper-not-bundled) that fits
   Aivyx's explicit-not-implicit posture.

## Architecture

```
Today (Phase 49–51):
  ToolProcessBridge::spawn → tokio::process::Command::new(config.command)
                                         .args(config.args).spawn()

After Phase 52:
  if let Some(sandbox) = config.sandbox {
      tokio::process::Command::new(sandbox.wrapper)
          .args(sandbox.args)
          .arg(config.command)
          .args(config.args)
          .spawn()
  } else {
      // unchanged
  }
```

The wrapper's job is to set up isolation and then `exec` the
real command. Standard sandbox tools (`bwrap`, `firejail`,
`docker run`) all support this `wrapper [wrapper-args...]
command [command-args...]` shape natively.

## Entry baseline

- Rust tests: 979
- Python conformance tests: 24
- Workspace crates: 12
- Clippy warnings: 0
- Deferral backlog: 4 (live audit push, read-write dashboard,
  Rust conformance harness, IPC stability window) + 1
  scheduled for Phase 52 (per-tool sandboxing)
- DESIGN.md streak: 3 phases (untouched since Phase 49 A4 addendum)
- PRODUCT.md streak: 2 phases (untouched since Phase 50)
- `aivyx-core/src/lib.rs` streak: 0 phases (Phase 51 break)

## Q-block — resolutions

**Q1: Built-in bubblewrap integration vs generic command wrapper?**
→ **Generic command wrapper.** Aivyx supplies the slot; the
operator supplies the policy. Cross-platform without `cfg`
branches; operators bring their own tool; additive on top of
P12's "process isolation is the floor."

**Q2: Default behavior?** → **Off.** A `[[tool_process]]` entry
without `[tool_process.sandbox]` spawns exactly as Phase
49–51. No silent behavior change.

**Q3: TOML schema?** → A nested `[tool_process.sandbox]` block
with `wrapper: String` and `args: Vec<String>`. Effective spawn
becomes `wrapper wrapper_args... command command_args...`.

**Q4: Testing strategy?** → Use POSIX `env` as the wrapper in
tests. Universally available; proves wiring without depending
on bwrap/firejail/docker being installed. Real-sandbox
verification is operator-side.

**Q5: Documentation surface?** → `TOOL_SDK.md` gains §10
*"Sandboxing tool processes"* with three worked examples:
bubblewrap (Linux native), firejail (Linux user-friendly),
`docker run` (cross-platform). Each names what it isolates and
what it doesn't. `THREAT_MODEL.md` §5.6 gets a follow-up note
pointing here.

**Q6: Streak predictions?**

| Streak target | Predicted | Notes |
|---|---|---|
| DESIGN.md | untouched (4) | No contract change. |
| PRODUCT.md | untouched (3) | This is hardening on top of P12, not a P12 amendment. |
| `aivyx-core/src/lib.rs` | untouched (1) | Work lives in `aivyx-tool` + `aivyx-config` + binary. |

## Tasks

### Task 1 — Open commit + scaffold

This file. Add Phase 52 entry to `docs/ROADMAP.md`. Add Phase 52
row to `docs/README.md`.

### Task 2 — `SandboxConfig` + bridge wrapper application

`aivyx-tool/src/bridge.rs`:

```rust
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub wrapper: String,
    pub args: Vec<String>,
}

pub struct ToolProcessConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub sandbox: Option<SandboxConfig>,
}
```

`ToolProcessBridge::spawn` picks the right `Command` shape based
on `sandbox`. Unit tests cover the two construction paths.

### Task 3 — `[tool_process.sandbox]` TOML schema + binary wiring

`aivyx-config`:
- `RawToolProcess` gains an optional `sandbox: RawSandbox` field.
- New `RawSandbox { wrapper: String, args: Option<Vec<String>> }`.
- Loader translates to runtime `Option<SandboxConfig>`.
- Validation: empty `wrapper` rejected the same way empty
  `command` is rejected.

Binary (`aivyx-channel/src/bin/aivyx.rs`):
- The tool-process spawn loop threads `tp_cfg.sandbox` into
  the `aivyx_tool::ToolProcessConfig` it constructs.

Config-load tests pin the round-trip.

### Task 4 — Integration test using `env` as no-op wrapper

Add to `examples/python-tool/tests/` *or* `crates/aivyx-tool/tests/`:

- Spawn the Python wordcount tool via `ToolProcessBridge` with
  `SandboxConfig { wrapper: "env", args: ["PYTHONUNBUFFERED=1"] }`.
- Drive a handshake + one invocation.
- Assert the round-trip works exactly as the no-sandbox path.

POSIX `env` is universally available; this test runs anywhere
`cargo test` runs.

### Task 5 — `TOOL_SDK.md` §10 + `THREAT_MODEL.md` §5.6 note

`docs/TOOL_SDK.md` gains a new §10 "Sandboxing tool processes":
- What the wrapper layer does (and explicitly does not)
- Bubblewrap example (Linux native, no installer)
- Firejail example (Linux user-friendly, profiles available)
- `docker run` example (cross-platform, heavyweight)
- Notes on stdio passthrough, signal forwarding, kill-on-drop
  interaction
- Notes on which `ToolEventPayload` variants might be affected
  (none — stdio remains the protocol)

`docs/THREAT_MODEL.md` §5.6 gets a follow-up note: "Phase 52
adds an operator-supplied sandbox layer; see TOOL_SDK.md §10."

### Task 6 — Exit freeze

Backfill exit stats, ship records, deferral list. Mark ROADMAP
frozen. Update README. The Phase 49 sandboxing deferral is
closed.

## Ship records

| Task | Commit | Notes |
|---|---|---|
| 1 | `05c1961` | scaffold |
| 2 | `f5e4fda` | SandboxConfig + bridge wrapper application — 980 tests |
| 3 | `1dce6cc` | [tool_process.sandbox] TOML schema + binary wiring — 983 tests |
| 4 | `1c74a80` | Integration test with `env` no-op wrapper — 984 tests |
| 5 | `5a9a72a` | TOOL_SDK.md §9 + THREAT_MODEL.md §5.6 update |

## Deferrals carried into the phase

- Live audit push (P47 Q4)
- Read-write dashboard inspection (P47 Q6)
- Conformance harness as a Rust crate (P48 Q5)
- IPC stability window commitment (P48 Q6)
- Per-tool sandboxing (P49) ← **closing this**

## Net-new deferrals (predicted)

- None expected. If the integration test reveals signal-
  forwarding subtleties (e.g., a wrapper that breaks the
  `kill_on_drop` chain), they will be recorded at exit.

## Exit criteria

- [x] `SandboxConfig { wrapper, args }` exists in `aivyx-tool`.
- [x] `ToolProcessConfig.sandbox: Option<SandboxConfig>` plumbs
  through `ToolProcessBridge::spawn`.
- [x] `[tool_process.sandbox]` TOML schema loads and validates.
- [x] Integration test using `env` as wrapper drives the inline
  Python echo tool end-to-end (closer to the wordcount tool in
  spirit; the inline harness keeps the test self-contained).
- [x] `docs/TOOL_SDK.md` has a §9 (renumbered from §10 to keep
  file order monotonic) with three worked examples.
- [x] `docs/THREAT_MODEL.md` §5.6 rewritten to acknowledge
  Phase 49 + 50 + 52 narrowing.
- [x] DESIGN.md untouched (streak → 4).
- [x] PRODUCT.md untouched (streak → 3).
- [x] `aivyx-core/src/lib.rs` untouched (streak → 1).
- [x] Zero clippy warnings.
- [x] Rust tests 979 → 984 (+5).

## Exit stats

- Rust tests: 979 → 984 (+5: 1 bridge unit, 3 config-load,
  1 e2e sandbox-wrapper)
- Python conformance tests: 24 (unchanged)
- Workspace crates: 12 (unchanged)
- Clippy warnings: 0
- Deferral backlog: 4 → 4 (P49 sandboxing closed; no net change
  because it had not made it into the rolling backlog as a
  numbered item — only into the PHASE_49.md exit list)

### Streak outcomes

| Streak target | Predicted | Actual | New streak |
|---|---|---|---|
| DESIGN.md | untouched (4) | untouched | 4 |
| PRODUCT.md | untouched (3) | untouched | 3 |
| `aivyx-core/src/lib.rs` | untouched (1) | untouched | **1** |

All three predictions correct. lib.rs was at risk (any change
to error types or substrate trait shape touches it) but the
sandbox layer lives entirely in `aivyx-tool` + `aivyx-config` +
binary — no core trait churn.

### Items closed

| Item | Source | What was closed |
|---|---|---|
| Per-tool sandboxing | Phase 49 net-new deferral | Generic command-wrapper layer in `aivyx-tool`; `[tool_process.sandbox]` TOML; `TOOL_SDK.md` §9 |
| THREAT_MODEL §5.6 narrowing | Threat-model gap | Rewritten to acknowledge Phases 49/50/52 |

### Deferrals carried forward (4)

1. Live audit push (P47 Q4)
2. Read-write dashboard inspection (P47 Q6)
3. Conformance harness as a Rust crate (P48 Q5)
4. IPC stability window commitment (P48 Q6)

### Operator-side verification

The Phase 52 sandbox wiring works end-to-end against a real
sandbox tool — needs a manual smoke test on an actual Linux box
with `bwrap` installed. Recommended verification at Chapter A
exit (Phase 54).

### Chapter A position

Three of five Chapter A phases shipped:

- ~~Phase 50~~ ✓ — P12 closeout (first-party in-process protocol unification)
- ~~Phase 51~~ ✓ — Cleanup (error typing + ConnectionContext + passphrase path)
- ~~Phase 52~~ ✓ — Tool process sandbox layer
- **Phase 53** — Audit log rotation/compaction *(optional)*
- **Phase 54** — Final documentation sweep

If Phase 53 is skipped, Phase 54 follows directly.

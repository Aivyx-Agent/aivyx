# Phase 180 — Security-by-Default: Bundled Sandbox Preset

**Chapter H, phase 1.** The Phase 179 backend review
([BACKEND_REVIEW_2026-06-06.md](BACKEND_REVIEW_2026-06-06.md))
named this the #1 Tier-1 gap: the tool-process sandbox
(`aivyx-tool`, Phases 52/55) is a *wrapper the operator must
configure*. Out of the box, a `[[tool_process]]` with no
`sandbox` block spawns with the operator's full UID — so a
*security-focused* product is not secure by default. Phase 180
ships a bundled default preset, detected and applied
automatically, so new launches get OS-level tool isolation
without operator setup.

## The crux — functional isolation, not just isolation

A naive "block everything" default would break the very tools
the operator opted into: the Chapter F/G productivity tools read
their OAuth token from `$HOME/.aivyx/tool-processes/<tool>/` and
make network calls. So the default preset must be **isolating
but functional**: read-only system dirs, a private `/tmp`, **no
access to `$HOME`** *except* a writable bind of the per-tool data
dir, and network left on (it is already capability-gated at the
IPC boundary, and the productivity tools require it). A tool that
needs more declares an explicit `sandbox` block — the existing
escape hatch.

## The discipline knot — secure-by-default vs. no-surprise

Turning the sandbox on by default is a behaviour change, which
the project's "behaviour-change-is-opt-in" rule would normally
forbid. The resolution threads the needle:

- **In-code default (no `[sandbox]` section) = `none`** — an
  existing operator's config behaves byte-identically to Phase
  179.
- **The `aivyx init` wizard writes `[sandbox] default_backend =
  "auto"`** — so every *new* launch is secure-by-default (the
  goal), while no existing config is silently changed.

## Design

- **Backend detection** — `detect_sandbox_backend()` checks
  `PATH` for `bwrap` (bubblewrap), then `firejail`. Returns the
  first found, or `None`. A path-lookup seam keeps it testable.
- **Preset argv builders** (pure, unit-tested) —
  `bubblewrap_preset(writable: &[PathBuf]) -> SandboxConfig` and
  `firejail_preset(...)`: a conservative argv (read-only `/usr`
  `/bin` `/lib*` `/etc`, `--proc`/`--dev`, `tmpfs /tmp`,
  `--die-with-parent`, network on, writable bind of the per-tool
  data dir). The argv is the testable core; runtime isolation is
  operator-verified (the threat-model pattern).
- **`[sandbox]` config** — `default_backend: "auto" |
  "bubblewrap" | "firejail" | "none"` (absent section → `none`).
  `auto` resolves to a detected backend, warning + falling back
  to `none` if neither is present.
- **Per-tool precedence** — explicit `sandbox` block (Some) wins
  → else per-tool `disable_sandbox = true` opt-out → `none` →
  else the global default-resolved preset → else `none`.
- **Legibility** — record the applied posture per tool spawn (a
  stderr breadcrumb + the `aivyx tools` view: `gmail —
  sandboxed (bubblewrap)` vs `unsandboxed`), so the operator can
  *see* the posture.
- **Wizard** — generated configs gain `[sandbox] default_backend
  = "auto"`.

## Tasks

1. **Open doc + README.** This doc, README Active row, backfill
   Phase 179's frozen hash (`0d369d1`).
2. **Detection + preset argv builders.** `detect_sandbox_backend`
   + `bubblewrap_preset` / `firejail_preset` in `aivyx-tool`.
   Tests assert the exact argv (both backends, with the per-tool
   writable bind) + detection over an injected PATH.
3. **`[sandbox]` config + spawn resolution.** The config section
   + the precedence resolver threaded into `ToolProcessBridge::
   spawn` (and the MCP stdio spawn). Per-tool `disable_sandbox`.
   Tests for every precedence branch.
4. **Wizard default + legibility.** `aivyx init` writes
   `[sandbox] default_backend = "auto"`; the applied posture is
   surfaced in `aivyx tools` + a spawn breadcrumb. Tests.
5. **THREAT_MODEL + INSTALL + exit + Frozen.** Update
   THREAT_MODEL §4.10 / §5.6 to reflect the default preset;
   INSTALL section; exit doc; README Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak: 16 → **17**. A default
  preset + a config section over the *existing* Phase 52/55
  sandbox concept — no new capability scope, no new tool, no new
  `KeyDomain`, no change to the capability/trust contract. The
  posture improvement is documented in THREAT_MODEL (a living
  doc, not a contract). *Watch item: if review judges
  secure-by-default a contract-level posture change, an A14
  amendment — but the in-code default stays `none`, so the
  contract behaviour is unchanged.*
- **PRODUCT.md** — **Will hold.** Streak: 70 → **71**.
- **`aivyx-core/src/lib.rs`** — **Will hold.** Streak: 16 →
  **17**. Work lands in `aivyx-tool` + `aivyx-config` +
  `aivyx-channel`; `aivyx-core` untouched.

## Exit criteria

- [ ] `docs/PHASE_180.md` + README row + Phase 179 backfill — Task 1.
- [ ] `detect_sandbox_backend` + preset argv builders — Task 2.
- [ ] `[sandbox]` config + spawn precedence resolver — Task 3.
- [ ] Wizard writes `auto`; posture surfaced — Task 4.
- [ ] THREAT_MODEL updated — Task 5.
- [ ] DESIGN.md / PRODUCT.md / `aivyx-core/src/lib.rs` HOLD.
- [ ] Zero new workspace dependencies (PATH lookup + argv only).
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+12` to `+20`. *(Dense-test component:
  the preset argv builders — backends × bind cases × precedence.
  No parser / IPC surface, so below the Phase 178 band.)*

## Honest scope risks at sign-off

- **The preset handles the common case; runtime isolation is
  operator-verified.** Unit tests assert the argv; whether bwrap
  actually contains a given tool is verified on the operator's
  machine (no bwrap in CI). A tool needing extra paths uses the
  explicit `sandbox` escape hatch.
- **Network is left ON by the preset.** Filesystem + process
  isolation is the win; network egress stays capability-gated at
  the IPC layer, not blocked at the OS layer (blocking it breaks
  productivity tools).
- **`auto` silently falls back to `none` when no backend is
  installed** — with a startup warning. A operator on a host
  without bwrap/firejail is no worse off than Phase 179, but
  isn't protected either; documented.
- **Existing configs are unchanged** — only new `aivyx init`
  output is secure-by-default. Existing operators opt in by
  adding `[sandbox] default_backend = "auto"`.

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 17 | Untouched (a default preset + config section over the existing Phase 52/55 sandbox; no scope/tool/`KeyDomain`/contract change) | ✅ |
| PRODUCT.md HOLD → 71 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 17 | Untouched | ✅ |
| Zero new workspace deps | PATH lookup + argv strings only | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean | ✅ |
| Test count delta `+12` to `+20` | **`+14`** (sandbox 12: 6 presets/detection + 6 resolver; config 1; wizard 2; ~4,139 → ~4,153) | ✅ **in band** |

**The watch item resolved in favour of HOLD.** The open doc
flagged that secure-by-default *might* read as a contract-level
posture change needing an A14 amendment. It didn't: the in-code
default with no `[sandbox]` section stays `none`, so the
*contract behaviour* is byte-identical to Phase 179 — only the
wizard's generated output changed, and the posture is documented
in THREAT_MODEL (a living doc). No amendment.

**Band note — priced from components, landed mid-band.** I used
`+12..+20` (a substrate phase whose dense component is the preset
argv + resolver-precedence builders, with no parser / IPC
surface), and landed `+14`. The Phase 179 lesson — price the
band from the dense-test *components* a phase contains — held a
second time.

What shipped, end-to-end:

1. **Detection + preset builders** (Task 2). `detect_sandbox_backend`
   + `bubblewrap_preset` / `firejail_preset` — argv is the
   unit-testable core; runtime isolation is operator-verified.
2. **Config + resolution** (Task 3). `[sandbox].default_backend`
   + per-tool `disable_sandbox` + the `resolve_sandbox`
   precedence resolver, wired into the tool-process spawn loop
   (ro-bind the command dir, writable-bind the token dir).
3. **Secure-by-default + legibility** (Task 4). The wizard writes
   `auto` in both render paths; the spawn breadcrumb names the
   posture per tool — including the security-relevant
   **UNSANDBOXED** case.
4. **Docs** (Task 5). THREAT_MODEL §6 / §4.10, the INSTALL
   `[sandbox]` section.

### Honest scope risks at sign-off

- **Runtime isolation is operator-verified.** The argv is
  asserted in unit tests; whether bwrap actually contains a
  process is verified on the operator's host (no backend in CI).
- **Network stays ON** in the preset — filesystem + process
  isolation is the win; egress is capability-gated at the IPC
  layer, not blocked at the OS layer.
- **MCP servers are out of the bundled preset** — they keep the
  Phase 55 explicit model (documented scope boundary).
- **The per-tool `aivyx tools` posture column was deferred** —
  the startup breadcrumb is the Phase 180 legibility; the IPC
  plumbing for a per-tool column is a follow-on. Honest cut to
  keep the phase focused.
- **Existing configs are unchanged** — only new launches are
  secure-by-default; existing operators opt in with one line.

### The result

Chapter H's first phase closes the backend review's #1 Tier-1
gap: a *security-focused* product is now secure by default for
every new launch — bundled OS-level tool isolation, no operator
setup — without changing a single existing config or breaking the
zero-new-dependency / contract-streak discipline.

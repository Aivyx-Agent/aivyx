# CLI `team run --config` Capability Floor Design

## Motivation

The `bind_lead_scopes` Floor-Clamp piece (shipped 2026-08-24,
`docs/superpowers/specs/2026-08-24-bind-lead-scopes-floor-clamp-design.md`)
closed the deferred root-cause item from Piece A of Team-Mission Triggers
for the daemon/scheduler/Studio mission path. Its own final review found a
related, worse, still-open gap in a completely different code path: the
CLI's own `aivyx team run --config <pack.toml>` command
(`crates/aivyx-cli/src/bin/aivyx_modules/team.rs`, function `run_mission`,
lines 183–238) never reaches `bind_lead_scopes` at all. It reads a pack's
`lead.declared_capabilities()` directly and passes it straight into
`TeamAssembly::build` — a pack's lead role is granted *exactly and
unconditionally* whatever `capability_scopes` it declares in its own file,
with zero floor concept applied. `docs/VERTICAL_PACKS.md:58` documents this
flag as the standard, expected way to run a vertical pack.

This design closes it. This is the same threat model the floor-clamp piece
targeted — an operator running a third-party-authored vertical pack without
having fully audited its own declared `capability_scopes` — but the CLI
path's total absence of any floor concept made it worse than the bug just
fixed, which at least applied a union against a real floor.

## Threat model and chosen approach

Unlike the daemon/scheduler/Studio paths, `aivyx team run --config` is
invoked interactively: the operator types the exact command and names the
exact pack file themselves, synchronously, with no other actor (model,
scheduler, remote sender) in the loop. This distinction was put to the user
directly rather than assuming the daemon-shaped fix (compute the operator's
real authority, clamp the pack's declared scopes to it) was automatically
right here too — the alternatives considered were a confirm-first prompt
naming what the pack's lead role would gain, a narrower/cheaper proxy floor
computed early, and doing nothing (documentation-only).

**Chosen: compute-and-clamp, the same shape as the daemon fix.** The
operator typing the command establishes *intent* to run the pack, not
*informed consent* to whatever scopes the pack's own file happens to
declare — `TeamConfig::load` only validates that scopes parse, never that
they're authorized against anything, so an operator has no legible way to
notice an overreaching declaration without reading the pack file's
`capability_scopes` lines by hand. Clamping to the operator's own real,
already-configured authority (the same `backcompat_floor` concept the
interactive agent itself runs under) makes this consistent with every other
mission path in the codebase, with no new UI surface or flag to design,
test, or document.

## What already exists (verified against real current code)

- `run_mission` (`team.rs:183–238`): loads the pack's `TeamConfig`, clones
  its lead member, calls `lead.declared_capabilities()` directly (~line
  206), and passes the result straight into `TeamAssembly::build` as the
  lead's capability floor for delegating to specialists. No clamping.
- `run_mission` is invoked from an early return in `run_async`
  (`aivyx.rs:7867`, inside
  `if let CliMode::Team(TeamSubcommand::Run { mission, config }) = &mode`)
  that fires **before** `backcompat_floor` — the codebase's own real
  "operator's authorized capability floor" concept, used by every other
  mission path via `bind_lead_scopes` — is ever computed.
  `backcompat_floor`'s construction (`aivyx.rs:8009–8186`, a ~180-line
  block) happens later in the same function, for the normal
  interactive-agent path only.
- Every input `backcompat_floor`'s construction depends on
  (`fs_read_scope`, `fs_write_scope`, `fs_metadata_scope`, `canonical_root`,
  `shell_exec_scope`, `fs_delete_scope`, `workspace_scopes`,
  `ollama_base_url_for_tools`, `mcp_bridges`, `config_tool_processes`,
  `loop_state`) is confirmed already computed **before** line 7867 — so
  closing this gap is a real move, not a redesign: nothing needs to be
  computed earlier than it already is, only the *assembly* of
  `backcompat_floor` from those pieces needs to happen earlier.
- `bind_lead_scopes` (`aivyx-channel/src/team_mission_driver.rs:1402`) is
  currently a private `fn` in an already-`pub mod`
  (`team_mission_driver`). It already clamps both a pack's lead-role and
  specialist-role declared `capability_scopes` to a passed-in floor,
  logging a warning (`eprintln!`) only when a real clamp fires.
  `aivyx-cli` already depends on `aivyx-channel` and already imports the
  same `TeamConfig` type through it (`use aivyx_team::{..., TeamConfig}`,
  the same type `team_mission_driver.rs` operates on) — no new crate
  dependency edge is needed to call it.
- `backcompat_floor`'s construction currently has **no direct unit-test
  coverage** — `assemble_role_envelope`'s own tests
  (`empty_role_inherits_backcompat_floor_verbatim` and neighbors,
  `aivyx.rs:~11060`) exercise a small local `floor()` test stub, not the
  real ~180-line block. It's exercised only indirectly today, through
  e2e/manual use of the interactive CLI.

## Architecture

Four changes, no new files:

1. **Extract `backcompat_floor`'s construction** (`aivyx.rs:8009–8186`)
   into a standalone function, `fn compute_backcompat_floor(...)`, taking
   the inputs listed above as explicit parameters. Defined in `aivyx.rs`
   itself (not exported — nothing outside this file needs it). The
   existing call site becomes
   `let backcompat_floor = compute_backcompat_floor(...);` — a single
   source of truth, so the interactive path and the CLI team-run path can
   never drift apart on what "the operator's real authority" means.
2. **Call it early**: add a call to `compute_backcompat_floor(...)` before
   the `CliMode::Team(TeamSubcommand::Run { .. })` branch at line 7867 (all
   its inputs are already in scope there, per the verification above).
3. **Make `bind_lead_scopes` `pub`** in `team_mission_driver.rs` — no
   signature or logic change.
4. **Thread the floor into `run_mission`**: add a `lead_scopes: &[String]`
   parameter, mirroring the daemon call site's own
   `lead_scopes: backcompat_floor.iter().map(|s| s.as_str().to_string()).collect()`
   (`aivyx.rs:8909`). Inside `run_mission`, right after `load_team` and
   before extracting `lead`, call
   `aivyx_channel::team_mission_driver::bind_lead_scopes(&mut config, lead_scopes)`.
   This clamps both the lead's and every specialist's pack-declared scopes
   to the real interactive-session floor before `TeamAssembly::build` ever
   sees them — specialists get the same protection as the daemon path for
   free, since `bind_lead_scopes` already handles both branches.

**Observability note, not a code change:** `bind_lead_scopes`'s existing
clamp warning becomes *more* useful reused here than in the daemon case —
it prints straight to the operator's own terminal in the same process,
live, rather than into a log file they may not be tailing.

## Testing

The refactor's real risk is regressing the *interactive* path's own
capability floor, which currently has no direct unit coverage. This design
closes that gap rather than merely hoping the extraction preserves
behavior:

- **New test for `compute_backcompat_floor` itself**: call it with a fixed,
  representative set of inputs (matching a real default-role interactive
  config — Ollama configured, one MCP bridge, applications enabled, loop
  armed) and assert the exact resulting `Vec<Scope>` (as sorted scope
  strings), covering every conditional branch (ollama grants, mcp grants,
  applications grants, loop-armed grants). This is new, durable coverage
  for logic previously exercised only end-to-end, and it is the
  regression-proof that extraction didn't silently change interactive-path
  behavior.
- **Existing tests** `empty_role_inherits_backcompat_floor_verbatim` and its
  neighbors (`aivyx.rs:~11060`) must keep passing **unchanged** — they
  exercise `assemble_role_envelope` against the local `floor()` stub, so
  they're unaffected by the extraction itself, but re-running them confirms
  nothing downstream broke.
- **New test for the CLI-clamp wiring**: `bind_lead_scopes` itself is
  already exhaustively tested in `team_mission_driver.rs` (2 original +
  3 added by the floor-clamp piece) — no new coverage needed there. What's
  new is the *wiring* in `run_mission`: a test (unit-level if `run_mission`
  can be exercised without a live provider by constructing a `TeamConfig`
  directly and calling the same `bind_lead_scopes` call inline as
  `run_mission` does; otherwise a documented manual/e2e verification step)
  confirming that a pack's lead role declaring a domain scope the passed
  floor doesn't grant ends up clamped before assembly. This is the
  mutation-proof for this piece: if the new `bind_lead_scopes` call in
  `run_mission` were removed, this test must fail.
- **Compile-level proof for the `pub` change**: `aivyx-cli` successfully
  calling `aivyx_channel::team_mission_driver::bind_lead_scopes` across the
  crate boundary is verified by the build succeeding — no separate test
  needed for visibility.

## Out of scope

- No change to `TeamConfig::load`'s own parsing/validation (same reasoning
  as the daemon fix — parsing stays parsing-only).
- No new CLI flags (`--force`, confirm-first prompts) — the chosen approach
  makes those unnecessary.
- No change to the Studio's or scheduler's own already-fixed call sites.
- No broader refactor of `run_async` beyond lifting
  `compute_backcompat_floor`'s call earlier — the function stays a large
  integration-style function; this doesn't attempt to restructure it
  further.

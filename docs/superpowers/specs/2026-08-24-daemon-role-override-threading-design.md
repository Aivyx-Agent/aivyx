# Daemon `role_override` Threading Design

## Motivation

Piece C of Team-Mission Triggers (shipped 2026-08-23) built the daemon's
own per-channel `/team run` authorization (`channel_trigger_authz`,
`crates/aivyx-channel/src/daemon_server.rs`) by re-reading the same
`aivyx.toml` the process already loaded, deliberately not trusting
anything the connecting channel-adapter process claims about its own
authorization. That re-read goes through a shared helper,
`load_settings_config`, which hardcodes `role_override: None` in the
`LoadOptions` it builds — because `DaemonConfig` (the struct `run_daemon`
takes) had no field carrying the real `--role` the process was actually
started with.

If the daemon's *primary* config load was started with a non-default
`--role` (against a config with no role literally named `"default"`), this
re-read's own role resolution can diverge from the primary load's and fail
with `ConfigError::UnknownRole`. That failure was already fixed to fail
closed loudly (a warning log naming the likely cause) rather than silently
— but `team_run_channel` still goes inert for the whole deployment. Not a
security hole (it fails closed, never grants), but a real availability/
correctness bug for any operator running a non-default `--role`, deferred
at the time as needing "a broader `DaemonConfig` refactor... out of scope
for this fix."

## Re-verified framing (2026-08-24)

The real scope turned out much smaller than the original deferral implied.
`DaemonConfig` has exactly 9 construction sites, re-counted fresh:

- **One real production site** (`crates/aivyx-cli/src/bin/aivyx.rs`, the
  actual CLI binary's daemon startup) — the only one whose behavior needs
  to change.
- **One compat wrapper** (`run_daemon_compat`, `daemon_server.rs`) — a
  minimal test-only path that never sets `config_toml_path` either, so the
  re-read it might trigger already fails closed regardless.
- **Seven test fixtures** (`crates/aivyx-channel/tests/daemon_roundtrip_e2e.rs`)
  — none exercise the real-role-divergence path.

The real active role is *already* available at the one production site: a
plain `Option<String>` local variable named `role_override` in `aivyx.rs`,
already used to build the daemon's own primary `LoadOptions`
(`role_override: print_role.clone().or(role_override)`). It simply never
gets threaded into `DaemonConfig` at all — no new resolution logic is
needed, only carrying an existing value one hop further.

A second, previously untracked instance of the identical root cause: Chapter
U's `settings_applied` (the `GetSettings`/`SetAccessLevel`/`SetBudget`
IPC-query re-read handler, 4 call sites inside `run_daemon`) calls the exact
same `load_settings_config` helper with the same hardcoded `None`. Since
`run_daemon` already destructures the full `DaemonConfig` into scope for its
whole body, adding one field fixes both call sites for the same change —
no reason to fix only the one that happened to get logged.

## Architecture

1. Add `pub role_override: Option<String>` to `DaemonConfig`
   (`daemon_server.rs`), documented as carrying the same value the
   daemon's own primary config load resolved against — not a new
   resolution mechanism, a value already computed once, threaded one hop
   further.
2. `load_settings_config` gains a `role_override: Option<String>`
   parameter, passed straight into the `LoadOptions` it builds, replacing
   the hardcoded `None`.
3. Both real call sites inside `run_daemon` — `channel_trigger_authz`'s
   build and all 4 of `settings_applied`'s call sites — pass
   `config.role_override.clone()` (in scope via the existing destructure
   at the top of `run_daemon`).
4. The one real production construction site (`aivyx.rs`) passes the
   existing `role_override` local variable — the same one already feeding
   the primary config load, zero new logic.
5. The other 8 construction sites (the compat wrapper + 7 test fixtures)
   get `role_override: None` — mechanical, matches their existing
   behavior (none of them exercise a non-default role today).

The existing warning-log path (already loud, not silent) stays as a
defense-in-depth backstop for any future genuine divergence between the
primary and re-read config loads — this design closes the one scenario
that currently, predictably triggers it, not the log path itself.

## Testing

- New test proving `channel_trigger_authz`'s build resolves correctly
  against a non-default role when `DaemonConfig.role_override` carries one
  — constructed with a real config file declaring a non-`"default"`-named
  role and a matching `--role`-equivalent override, confirming
  `team_run_channel` resolves instead of hitting `ConfigError::UnknownRole`.
  Mutation-proof: must fail if the new parameter is reverted to a
  hardcoded `None` inside `load_settings_config`.
- New test (or an assertion added to an existing `settings_applied`-facing
  test) confirming the same threading applies to that path — a
  `GetSettings`/`SetAccessLevel` round-trip against a non-default-role
  config succeeds rather than hitting the same `UnknownRole` failure.
- All 7 existing `daemon_roundtrip_e2e.rs` tests and the compat-wrapper
  path must keep passing unchanged — they don't exercise a non-default
  role, so `role_override: None` there is behavior-preserving.

## Out of scope

- No change to the warning-log path's own text or the fail-closed
  semantics — both are already correct.
- No broader `DaemonConfig` refactor — the struct doesn't need
  restructuring, only one new field.
- No change to how the *primary* config load resolves its own role — this
  design only threads the value it already produces one hop further.

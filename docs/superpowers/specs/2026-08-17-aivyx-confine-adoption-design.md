# Adopting `aivyx-confine` in `shell.exec` and `git.rs` — design

_2026-08-17._ Design for wiring the shared `aivyx-confine` crate (Landlock
+ seccomp-bpf process confinement, extracted from `aivyx-coder` on
2026-08-16 — see `aivyx-ecosystem/docs/superpowers/specs/
2026-08-16-aivyx-confine-design.md`) into `aivyx`'s own process-spawning
tools. This is the explicit follow-on that design doc named and
`aivyx-ecosystem/ROADMAP.md` logged as `aivyx-confine`'s next, not-yet-started
item. Not yet started — this document is the spec; implementation
planning is the next step.

## Problem

`aivyx`'s own `docs/THREAT_MODEL.md` states outright: *"Container-level
sandboxing of tools: No... forward work."* Confirmed by reading the actual
code, not inferred: `crates/aivyx-core/src/tools/shell.rs`'s
`ShellExecTool` spawns `sh -c <command>` via plain `tokio::process::Command`
with only process-group isolation, environment-variable stripping, and
audit logging — no kernel-level filesystem or syscall restriction.
`crates/aivyx-core/src/tools/git.rs` spawns `git` the same unconfined way,
across five call sites in three tools (`GitStatusTool`, `GitDiffTool`,
`GitCommitTool`). `aivyx-tool/src/sandbox.rs`/`bridge.rs` already wrap a
*different* code path (operator-configured `bwrap`/`firejail` for
third-party `[[tool_process]]`/MCP tools) — a separate mechanism for a
separate class of tool, not applied to either of these core tools.

`aivyx-confine` (the crate) already exists, is already consumed by
`aivyx-coder`, and is deliberately config-agnostic — built for exactly
this kind of second adoption.

## Two integration shapes, not one — a real architectural difference found before designing

`shell.exec` and `git.rs` need materially different integration patterns,
discovered by reading the actual current code rather than assuming
`aivyx-coder`'s own shape (one confiner, built once, reused for every
call) applies uniformly:

- **`shell.exec`** has a single `cwd_root` per tool instance
  (`ShellExecToolConfig::new(cwd_root)`), so one `ExecutionConfiner`
  built once, at tool-construction time, and reused for every call is the
  right shape — the same shape `aivyx-coder` already uses.
- **`git.rs`** takes an *allow-set* of potentially many repo paths
  (`GitReadToolConfig::new(repos: impl IntoIterator<Item = PathBuf>)`),
  and each call resolves one specific target repo from that set at
  request time (`resolve_repo(&input, &self.repos)`). `LandlockConfiner::
  new(cwd, extra_read_paths, deny_paths, require_enforcement)` only grants
  write access to a single root — it cannot express "write-confine to
  whichever of these N repos this particular call targets" at
  construction time. The correct shape here is a fresh, narrowly-scoped
  `LandlockConfiner` constructed *per call*, right after `resolve_repo`
  returns the validated target, not one confiner built once for the
  tool's lifetime.

## Why confining `git.rs` is worth the different shape, not just consistency

`-C <repo>` (already used at every git spawn site) scopes *git's own*
argument parsing to that directory, but does not stop the `git` process
itself — or anything it executes — from reading or writing elsewhere on
disk. Concretely: `git commit` executes `.git/hooks/post-commit` (and
`pre-commit`, etc.) as a full subprocess with the same filesystem access
as the parent process, by design. A malicious or compromised repo's hook
script is not scoped by `-C` at all today. Landlock confinement closes
this specific, concrete gap — the hook is confined to the same root as
`git` itself, unable to read or exfiltrate files elsewhere on disk even
though it executes with the operator's technical permissions otherwise.
This is the sharpest concrete justification for doing this work, not an
abstract "more sandboxing is better" argument.

## Config: one new field

A new `[confine]` section in `aivyx-config`, one field for this pass:

```toml
[confine]
require_enforcement = true   # default; fail-closed, matches aivyx-coder's own default
```

`require_enforcement = true` (the default): if Landlock fails to
establish a ruleset at runtime, refuse to run the command rather than
running unconfined. `false`: log a warning and run unconfined instead —
for operators on kernels/platforms where Landlock genuinely isn't
available and who've decided that's an acceptable tradeoff, mirroring
`aivyx-coder`'s own `sandbox.require_enforcement` semantics exactly.

**Explicitly deferred, not built in this pass:** `deny_paths`/
`extra_read_paths` configurability (e.g. reusing `SensitivePolicy`'s own
`extra_deny: Vec<PathBuf>` list — already operator-configured secret path
prefixes — as additional Landlock carve-outs, unifying the string-level
guard and the kernel-level guard's data source). Real, worthwhile
enhancement, but `fs_root` is already a narrow, purpose-built sandbox
directory (unlike `aivyx-coder`'s typical large project-repo `cwd`), so
the marginal safety value here is smaller than the config-surface cost of
building it now. Logged as backlog in `aivyx-ecosystem/ROADMAP.md`.

## `shell.exec` integration

`ShellExecTool` (`crates/aivyx-core/src/tools/shell.rs`) gains a
`confiner: Arc<dyn ExecutionConfiner>` field, following the exact shape
its existing `sensitive: Arc<SensitivePolicy>` field already uses:

- `ShellExecToolConfig::build()` sets a safe internal default —
  `default_confiner(canonical_cwd_root, &[], &[], true)` — so every
  existing caller that never touches confinement explicitly (e.g. the
  `substrate_tools_meet_quality_floor` test in
  `crates/aivyx-core/src/tools/mod.rs`, which never calls
  `.with_sensitive_policy` either) still gets real, on-by-default
  confinement, not silently none.
- A new `.with_confiner(Arc<dyn ExecutionConfiner>)` builder method lets
  the real binary call site override that default — used the same way
  `.with_sensitive_policy(sensitive)` already is.
- `crates/aivyx-cli/src/bin/aivyx.rs`'s `build_shell_exec_for_channel`
  (the one real production construction site) adds
  `.with_confiner(default_confiner(fs_root, &[], &[],
  config.confine.require_enforcement))` alongside its existing
  `.with_sensitive_policy(sensitive)` call.
- Inside `ShellExecTool::execute`, right before the existing
  `command.spawn()` call: `let command = self.confiner.confine(command);`.

## `git.rs` integration

`GitStatusTool`, `GitDiffTool`, and `GitCommitTool` each gain a
`require_enforcement: bool` field, set via new
`.with_require_enforcement(bool)` builder methods on `GitReadToolConfig`
and `GitWriteToolConfig` — mirroring `GitWriteToolConfig`'s existing
`.with_confirm_destructive(bool)` shape exactly. Default `true` if the
builder method is never called (same fail-closed default as `shell.exec`).

At each of the five `Command::new("git")` spawn sites, immediately after
`resolve_repo` returns the validated `repo: PathBuf` and before the
command is executed:

```rust
let confiner = aivyx_confine::LandlockConfiner::new(&repo, &[], &[], self.require_enforcement);
let cmd = confiner.confine(cmd);
```

(`extra_read_paths`/`deny_paths` empty here too, for the same v1-scope
reason as `shell.exec`.) The real binary call site threads
`config.confine.require_enforcement` into
`.with_require_enforcement(...)` on both `GitReadToolConfig` and
`GitWriteToolConfig` at construction time.

## Testing

New tests, in the same style `aivyx-confine`'s own suite already
uses — real filesystem assertions against a real spawned process, not
mocks:

- `shell.exec`: a command attempting to write outside `fs_root` fails
  under the default (on-by-default) confiner.
- `git.rs`: a `git.commit` invocation whose target repo is confined via
  the per-call `LandlockConfiner` cannot read or write a file outside
  that repo's root — the concrete hook-escape scenario this design's
  rationale names directly, not just a generic "confinement works" check.
- Confirm `require_enforcement = false` (once threaded through config)
  degrades to a working, unconfined call rather than refusing to run, on
  a build without `sandbox-backend` (or with Landlock unavailable).

## Explicitly out of scope

- `deny_paths`/`extra_read_paths` configurability (see "Config" above).
- Any change to `aivyx-tool/src/sandbox.rs`/`bridge.rs`'s separate
  `[[tool_process]]`/MCP sandboxing mechanism — unrelated code path, not
  touched here.
- Any policy change to `aivyx-confine` itself (syscall denylist, default
  grants) — this project only adds new call sites, no changes to the
  shared crate's own behavior.
- Non-Linux/no-Landlock-kernel behavior beyond what `aivyx-confine`
  already provides via its own `sandbox-backend` feature flag and
  `NoopConfiner` fallback — no new build-time toggle introduced here.

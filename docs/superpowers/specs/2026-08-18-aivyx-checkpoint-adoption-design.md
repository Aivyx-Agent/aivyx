# `aivyx-checkpoint` adoption design

_2026-08-18._ Part 2 of the `aivyx-checkpoint` project: wiring the
now-shipped [`aivyx-checkpoint`](https://github.com/Aivyx-Agent/aivyx-checkpoint)
crate (git-ref checkpoint/rollback — snapshot `fs_root`'s worktree to a
shadow `refs/aivyx/checkpoints/*` ref before a mutating tool call, restore
on demand, never touching the user's real HEAD/index) into `aivyx`, the
flagship agent. Follows the same two-project split as `aivyx-confine` /
`aivyx-confine-adoption`: the shared crate and `aivyx-coder`'s migration
onto it shipped as one project
(`aivyx-ecosystem/docs/superpowers/specs/2026-08-18-aivyx-checkpoint-design.md`);
this is the separate, later project that adopts it into `aivyx`, now that
a real commit SHA exists to pin against.

## Why this needed its own investigation, not just reuse of the original design

The original combined design's "`aivyx` adoption" section was written
during `aivyx-checkpoint`'s own brainstorming, before this project's
dedicated read of `aivyx`'s actual current code. Two of its assumptions
turned out to be wrong once checked against the real, much larger
codebase — both found by direct investigation (grepping every `impl Tool
for`, reading the real `SensitivePolicy` construction site), not
inferred from the prior design's summary. Both are corrected here.

### Finding 1 — the `Tool` trait is not aivyx-coder-shaped

The original design's new trait method (mirroring `aivyx-coder`'s own
`mutates_outside_session`) defaulted to `true`, with read-only tools
opting out. That polarity assumes what's true in `aivyx-coder`: nearly
every tool operates on the one repo being edited, so "mutates something
outside session state" and "mutates the thing checkpointing protects"
are nearly the same question.

`aivyx` is not shaped like that. Grepping `impl Tool for` across the
workspace finds **~90+ implementations** across a dozen+ largely
unrelated integration crates — `aivyx-gmail`, `aivyx-notion`,
`aivyx-drive`, `aivyx-calendar`, `aivyx-contacts`, `aivyx-n8n`,
`aivyx-obsidian`, `aivyx-team`, `aivyx-channel`, `aivyx-toolkit`, and
more — nearly all of which mutate *something* (a Gmail draft, a Notion
page, a calendar event) but have nothing to do with `fs_root`, the one
root `GitCheckpointer` protects in this pass. A default-`true` trait
method would checkpoint `fs_root`'s git worktree — a real `git add -A` +
`write-tree` — before every one of those calls, for zero protective
benefit.

The fix is a polarity flip, not a bigger override list: default `false`
(opt-in), and name the method for what it actually gates rather than the
misleading generic phrase inherited from `aivyx-coder`:

```rust
fn mutates_fs_root(&self) -> bool {
    false
}
```

Only `FsWriteTool` (`fs.write`), `FsDeleteTool` (`fs.delete`), and
`ShellExecTool` (`shell.exec`) override it to `true`. No other existing
`Tool` implementation anywhere in the workspace needs to change — a
smaller, safer diff than the originally-assumed opt-out polarity would
have required (which would have meant touching every read-only tool,
roughly 15-20 of them, to explicitly opt out).

### Finding 2 — `SensitivePolicy`'s `extra_deny` is always empty in practice

The original design said checkpoint's `deny_paths` would "reuse
`SensitivePolicy`'s existing `extra_deny: Vec<PathBuf>`." Reading the
real construction site in `crates/aivyx-cli/src/bin/aivyx.rs` shows this
field is never actually populated from operator config — the real call
is `SensitivePolicy::new(allow_sensitive_paths.clone(), Vec::new())`,
hardcoding `extra_deny` empty. There is no `[access]`-style config
surface wired to it at all.

The real protection `aivyx-core/src/sensitive_paths.rs` provides comes
from a curated **pattern-based classifier** (`SENSITIVE_DIR_SEGMENTS`,
`SENSITIVE_BASENAMES`, `SENSITIVE_EXTENSIONS`, exposed via
`SensitivePolicy::classify(&self, canonical: &Path) -> Option<String>`)
— not a plain path list. `GitCheckpointer::detect` needs a concrete
`Vec<PathBuf>`, so "reuse `extra_deny`" as originally planned would have
silently meant **no exclusion at all**: every `.env`, `id_rsa`, `.pem`,
etc. physically present under `fs_root` would get embedded in checkpoint
git objects. That's a real regression versus today's already-imperfect
guard — `sensitive_paths.rs`'s own doc comment already accepts that
`shell.exec` can `cat` a secret (transient exposure), but checkpointing
without exclusion would make that exposure *permanent*, readable from
`.git` history even after the live file is deleted.

**Resolution:** derive `deny_paths` by walking `fs_root` once at
`GitCheckpointer` construction time and collecting every path
`sensitive_policy.classify(...)` flags, converting the pattern-based
classification into the concrete path list the crate's existing API
requires. This is the same "fixed once at construction, not re-derived
per checkpoint" shape `deny_paths` already has in both `aivyx-coder` and
the shared `aivyx-checkpoint` crate itself — a secret file created mid-session
after the walk wouldn't be covered until the process restarts, but that's
an existing, accepted limitation of the shared crate's API, not a new one
introduced here.

## Design

### 1. New trait method

`aivyx-core`'s `Tool` trait (`crates/aivyx-core/src/lib.rs:907`) gains:

```rust
fn mutates_fs_root(&self) -> bool {
    false
}
```

Placed alongside the existing `output_is_untrusted` defaulted method,
which follows the identical shape (a `bool`-returning, defaulted trait
method gating a cross-cutting turn-loop behavior). `FsWriteTool`
(`crates/aivyx-core/src/tools/fs.rs`), `FsDeleteTool` (same file), and
`ShellExecTool` (`crates/aivyx-core/src/tools/shell.rs`) each add an
override returning `true`.

### 2. Agent-level wiring

`ConcreteAgent` (`crates/aivyx-core/src/agent.rs:141`) gains a field
mirroring the existing `budget_gate`/`rate_gate` pattern exactly —
`Option<Arc<dyn T>>`, `None` preserves current behavior byte-for-byte:

```rust
checkpointer: Option<Arc<aivyx_checkpoint::GitCheckpointer>>,
```

with a builder method, `with_checkpointer(mut self, checkpointer:
Arc<aivyx_checkpoint::GitCheckpointer>) -> Self`, matching the shape of
the existing `with_cycle_detection`-style builders on the same struct.
`ConcreteAgent::new` initializes it to `None`.

### 3. The hook

In `run_tool_call` (`crates/aivyx-core/src/agent.rs:1002`), immediately
before the existing dispatch line:

```rust
let step_start = Instant::now();
let mut outcome = tool.execute(input, &ctx).await;
```

insert:

```rust
if tool.mutates_fs_root()
    && let Some(checkpointer) = &self.checkpointer
{
    checkpointer.checkpoint(tool.name(), cancellation).await;
}
```

`cancellation: &CancellationToken` is already in scope — destructured
from `TurnCallEnv` at the top of `run_tool_call` (`crates/aivyx-core/src/agent.rs:1007-1012`)
and already used to build `ToolContext` a few lines above this insertion
point. `checkpoint`'s own contract (never fails the caller — internal
errors are logged and swallowed) means this can never turn a successful
tool call into a failed one.

### 4. Construction

In `crates/aivyx-cli/src/bin/aivyx.rs`, near where `canonical_root` is
resolved (~line 5951, immediately after `fs_read.sandbox_root()` is
pulled back out — the same point `ShellExecTool`'s confiner is built
relative to `fs_root`):

```rust
// aivyx-checkpoint — git-ref checkpoint/rollback for fs_root's mutating
// tools (fs.write, fs.delete, shell.exec). Deny paths are derived from
// the same sensitive-path classifier that guards fs.read/fs.write,
// walked once here rather than reusing SensitivePolicy's extra_deny
// directly — extra_deny is never populated from real config (always
// Vec::new() at this call site), so the real protection has to come
// from the pattern-based classifier instead.
let checkpoint_deny_paths = collect_sensitive_paths_under(&canonical_root, &sensitive_policy);
let checkpointer = aivyx_checkpoint::GitCheckpointer::detect(&canonical_root, checkpoint_deny_paths)
    .await
    .map(std::sync::Arc::new);
```

`collect_sensitive_paths_under` is a new, small private helper function
defined directly in `crates/aivyx-cli/src/bin/aivyx.rs` (its only call
site — the file already holds many similar one-off startup helpers, not
a reason to add a new module for a single caller) that walks `fs_root`
recursively and returns every canonical path for which
`sensitive_policy.classify(canonical)` returns `Some(_)`. `detect`
already tolerates `fs_root` not being a git repository (`None`, one log
line) — `checkpointer` then stays `None` for that session, matching the
crate's existing graceful-degradation contract; no operator error, no
new failure mode.

The `Agent` (or `ConcreteAgent`) builder call site gains
`.with_checkpointer(checkpointer)` when `checkpointer` is `Some`
(conditionally, since the builder method takes `Arc<GitCheckpointer>`
directly, not an `Option`).

### 5. Dependency

`crates/aivyx-core/Cargo.toml` gains, matching the exact pinned-`git`-dependency
pattern already used for `aivyx-confine` in the same file:

```toml
aivyx-checkpoint = { git = "https://github.com/Aivyx-Agent/aivyx-checkpoint", rev = "1292d6cbda34aa514856b81e11635f7385a4d168" }
```

Unlike `aivyx-confine`, `aivyx-checkpoint` has no platform-specific
backend (no Landlock/seccomp, pure git plumbing) — no
`[target.'cfg(...)'.dependencies]` split is needed; it's a plain
workspace dependency, buildable on every platform `aivyx` ships for.

## Explicitly out of scope

- **`git.rs`'s three tools** (`GitStatusTool`/`GitDiffTool`/`GitCommitTool`)
  — they operate against a separate, multi-repo allow-set (`[git]
  repos`, a `Vec<PathBuf>`), not `fs_root`. A single `GitCheckpointer`
  can only track one worktree's dedup cache and retention at a time —
  the same constraint that made `aivyx-confine`'s own adoption use a
  fresh per-call confiner for `git.rs` rather than one shared instance.
  Deferred, matches the original combined design's own scope boundary
  and the still-open backlog item in `aivyx-ecosystem/ROADMAP.md`.
- **`workspace.*` tools** (`WorkspaceWriteTool`/`WorkspaceDeleteTool`/`WorkspaceNoteTool`)
  — operate on `workspace_root` (`~/.aivyx/workspace`), explicitly a
  *different* root than `fs_root` (confirmed in `aivyx.rs`'s own comment:
  "Independent of `fs_root` / the access level"). Same reasoning as the
  `git.rs` exclusion: out of this pass's scope, not part of the root this
  `GitCheckpointer` instance protects.
- **Any new operator-facing config** — no `[checkpoint]` TOML section.
  `collect_sensitive_paths_under`'s walk reuses the existing, already-configured
  `sensitive_policy`; no new knobs.
- **A UI/CLI surface for browsing or restoring checkpoints** — this pass
  wires the mechanism in; exposing restore as an operator- or agent-facing
  action (a new tool, a CLI flag, a Studio screen) is separate, unscoped
  future work — matches the original combined design's own exclusion.

## Testing

1. **Checkpoint-on-mutation, not on unrelated tools**: a real git-backed
   `fs_root`, dispatch `fs.write` (or `shell.exec`) through
   `ConcreteAgent::run_tool_call` with a configured checkpointer —
   assert a new `refs/aivyx/checkpoints/*` ref exists. Dispatch a
   read-only tool (`fs.read`) and a fake unrelated mutating tool (a test
   double whose `mutates_fs_root()` is the default `false`, modeling
   `aivyx-gmail`'s `SendTool` or similar) — assert no new ref in either
   case.
2. **Full round-trip**: write a file, dispatch a mutating call
   (checkpoint fires), corrupt the file directly, call
   `checkpointer.restore_to(...)` with the checkpoint ref, verify the
   original content is back.
3. **Sensitive-path exclusion**: create a file under `fs_root` matching
   the classifier (e.g. `.env`), verify `collect_sensitive_paths_under`
   includes its canonical path, then verify a checkpoint's tree (via
   `git ls-tree`) does not contain it — mirrors `aivyx-checkpoint`'s own
   `denied_subpaths_are_excluded_from_the_snapshot` test, but exercising
   the real classifier-to-deny_paths derivation this project adds rather
   than a hand-supplied list.

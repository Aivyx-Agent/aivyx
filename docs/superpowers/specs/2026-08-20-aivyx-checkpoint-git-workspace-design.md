# `aivyx-checkpoint` for `git.rs` and `workspace.*` — design

_2026-08-20._ Closes the "New backlog" item logged at the end of the
`aivyx-checkpoint` remaining-sites project (see
`aivyx-ecosystem/ROADMAP.md`'s `aivyx-checkpoint` section): `git.rs`'s
three tools and `workspace.*` operate on roots other than `fs_root` and
remain entirely unprotected by checkpointing, unlike `fs.write`/
`fs.delete`/`shell.exec`/`data.xlsx.write`/`data.pdf.write`, which already
got `GitCheckpointer` wiring in the 2026-08-18/19 adoption.

## Scope

Of the eight tools touched by this backlog note, only two actually mutate
anything:

- `git.rs`: `GitCommitTool` (`git.status`/`git.diff` are read-only —
  inspection tools that never call `git add`/`git commit`).
- `workspace.rs`: `WorkspaceWriteTool`, `WorkspaceDeleteTool`,
  `WorkspaceNoteTool` (`WorkspaceReadTool`/`WorkspaceListTool` are
  read-only).

Those four tools are this project's entire surface. No change to
`GitStatusTool`, `GitDiffTool`, `WorkspaceReadTool`, or `WorkspaceListTool`.

## Why not the existing `ConcreteAgent` hook

`ConcreteAgent` already has a `checkpointer: Option<Arc<GitCheckpointer>>`
field, fired generically in the dispatch loop for any tool where
`Tool::mutates_fs_root() == true`. That checkpointer is built once, bound
to **`fs_root`**. Neither `[git] repos` (a separate, operator-configured,
potentially-multi-repo allow-set) nor `workspace_root` (a distinct fixed
path, default `~/.aivyx/workspace`) *is* `fs_root` — routing either
through the existing hook would checkpoint the wrong directory tree, or
silently no-op depending on whether `fs_root` itself happens to be a git
repo. `git.rs` already established the right precedent for this exact
situation with Landlock confinement: `confiner_for` handles confinement
entirely locally, per tool call, never touching `agent.rs`. This project
follows the same shape for checkpointing — both tool families get their
own, self-contained checkpointing. `Tool::mutates_fs_root()` stays `false`
(the default) on all four tools; setting it `true` would incorrectly wire
them into the `fs_root`-scoped hook.

## `git.rs`: one `GitCheckpointer` per configured repo, precomputed at startup

`[git] repos` is small, operator-curated, and fixed at startup (unlike
`fs_root`, which can be `/` under `[access] level = "full"`). Unlike
`confiner_for` — cheap to rebuild per call, since Landlock ruleset
construction is in-process — `GitCheckpointer::detect()` spawns a `git`
subprocess (`git rev-parse --absolute-git-dir`). Re-running that on every
single `git.commit` call would add avoidable subprocess overhead for no
benefit, since the repo set never changes at runtime. So: build one
`GitCheckpointer` **per repo, once**, mirroring exactly how `fs_root`'s
own checkpointer is built once today.

This happens in `aivyx.rs` (the binary), not `aivyx-core`, because
`deny_paths` derivation depends on `collect_sensitive_paths_under` and
`sensitive_policy`, both of which live in the binary — `aivyx-core` has no
visibility into either and must not gain a dependency in that direction.
Sequence, added near the existing git-tool construction site (currently
`aivyx.rs:6931-6974`), after `canonical_repos` (the already-canonicalized
allow-set, currently obtained via `git_status.repos()`) is available:

```rust
let mut git_checkpointers: HashMap<PathBuf, Arc<aivyx_core::GitCheckpointer>> = HashMap::new();
for repo in &canonical_repos {
    if is_inside_git_work_tree(repo).await {
        let deny_paths = collect_sensitive_paths_under(repo, &sensitive_policy);
        if let Some(cp) = aivyx_core::GitCheckpointer::detect(repo, deny_paths).await {
            git_checkpointers.insert(repo.clone(), Arc::new(cp));
        }
    }
}
```

`GitWriteToolConfig` gains a new builder method:

```rust
pub fn with_checkpointers(
    mut self,
    checkpointers: HashMap<PathBuf, Arc<GitCheckpointer>>,
) -> Self {
    self.checkpointers = checkpointers;
    self
}
```

defaulting to an empty map (matching `with_require_enforcement`'s
default-then-override shape) — `GitWriteToolConfig::build()` stays fully
synchronous; no async ripple into `aivyx-core`'s public surface. The map
is stored on `GitCommitTool` and canonicalized identically to `repos`
already is (both come from the same source list, so key lookups line up).

In `GitCommitTool::execute()`, immediately after resolving `repo` (and
before the `git add` step), look up and checkpoint if present:

```rust
if let Some(checkpointer) = self.checkpointers.get(&repo) {
    checkpointer.checkpoint("git.commit", ctx.cancellation).await;
}
```

This requires renaming the currently-unused `_ctx` parameter to `ctx` in
`GitCommitTool::execute`'s signature (the other two git tools' `_ctx`
stays unused — they never checkpoint).

## `workspace.rs`: one `GitCheckpointer`, same shape as `fs_root`'s

`workspace_root` is a single fixed path, so this mirrors `fs_root`'s own
checkpointer construction directly — no map needed. Built once in
`aivyx.rs`, near the existing `build_workspace_tools` call site (currently
`aivyx.rs:6784`):

```rust
let workspace_checkpointer: Option<Arc<aivyx_core::GitCheckpointer>> =
    if is_inside_git_work_tree(&workspace_root_path).await {
        let deny_paths = collect_sensitive_paths_under(&workspace_root_path, &sensitive_policy);
        aivyx_core::GitCheckpointer::detect(&workspace_root_path, deny_paths)
            .await
            .map(Arc::new)
    } else {
        None
    };
```

Per the approved answer to the brainstorming question, this is **opt-in,
not auto-initialized**: `provision_workspace` is not changed to `git
init` the directory. If `~/.aivyx/workspace` isn't already a git repo,
`workspace_checkpointer` is `None` and the three mutating tools get zero
checkpoint protection — the same graceful-degradation contract `fs_root`
already has when it isn't a git repo either. This is a deliberate,
documented trade-off, not an oversight: forcing a `.git` directory into
existence inside the agent's own workspace is a bigger behavior change
than this backlog item calls for.

`build_workspace_tools` gains the new parameter:

```rust
pub fn build_workspace_tools(
    root: &Path,
    checkpointer: Option<Arc<GitCheckpointer>>,
) -> Result<(Vec<Arc<dyn Tool>>, PathBuf), AivyxError>
```

threaded into `WorkspaceWriteTool::new`, `WorkspaceDeleteTool::new`, and
`WorkspaceNoteTool::new`. `WorkspaceReadTool` and `WorkspaceListTool` are
**not** touched: rather than adding an always-`None`-in-practice,
never-read checkpointer field to two read-only structs just to keep the
existing `ws_tool!` macro uniform, those two mutating-only structs are
pulled out of the macro and hand-written with the extra field. The macro
continues to generate `WorkspaceReadTool`/`WorkspaceListTool` unchanged.

Each of the three mutating tools' `execute()` gains, immediately before
its filesystem mutation:

```rust
if let Some(checkpointer) = &self.checkpointer {
    checkpointer.checkpoint(self.name(), ctx.cancellation).await;
}
```

(`self.name()` already returns `"workspace.write"` / `"workspace.delete"`
/ `"workspace.note"` — matches the string `agent.rs`'s existing generic
hook passes for `fs.write` et al.) This also requires renaming each
tool's `_ctx` parameter to `ctx`.

## Shared cleanup: `is_inside_git_work_tree` rename

`fs_root_is_inside_git_work_tree` (currently `aivyx.rs`, used once for
`fs_root`'s own `checkpoint_deny_paths` gate) is reused here for a git
repo and for `workspace_root` — its current name would be actively
misleading at two of its three call sites after this change. Renamed to
the generic `is_inside_git_work_tree`; behavior and signature (`&Path ->
bool`, async) unchanged, only the name and its doc comment's references
to "`fs_root`" generalize to "the given root".

## Non-goals

- No restore/rollback CLI surface. Checkpoints remain a write-only safety
  net today for every root (`fs_root` included) — `restore_to` is a
  library API exercised only by tests. Out of scope for this project,
  same as it was for the original `fs.write`/`shell.exec` adoption.
- No auto `git init` of `~/.aivyx/workspace` (see above — an explicit,
  approved scope decision, not an oversight).
- `GitStatusTool`/`GitDiffTool`/`WorkspaceReadTool`/`WorkspaceListTool`
  are unchanged — they never mutate, so they need no checkpointer.
- No change to `aivyx-checkpoint` itself (the crate). `GitCheckpointer`'s
  existing `detect`/`checkpoint`/`restore_to` API already supports
  exactly this per-root construction pattern with zero modification.

## Testing

- `git.rs`: a test with **two** configured repos, each with its own
  checkpointer, proving a `git.commit` against repo A produces a
  checkpoint ref only under repo A's checkpoint namespace, not repo B's —
  the one genuinely new behavior this project adds beyond the existing
  single-root precedent (repo-keyed lookup, not just "checkpointer
  present or absent"). A second test with an *unconfigured* repo (empty
  `checkpointers` map) proving no checkpoint ref is created and the
  commit still succeeds — the default-off case must not regress.
- `workspace.rs`: one test per mutating tool (write/delete/note) with a
  git-initialized workspace root, proving a checkpoint ref appears before
  the mutation; one test with a non-git workspace root (today's default)
  proving no checkpoint attempt and the mutation still succeeds
  unaffected — mirrors `agent.rs`'s existing
  `checkpoint_fires_only_for_mutates_fs_root_tools` test shape, using
  `aivyx_checkpoint::test_support` (`init_repo`, `git(...)`) the same way
  that test does.
- Existing `git.rs`/`workspace.rs` test suites must continue passing
  unmodified except for the mechanical `_ctx` → `ctx` rename (no
  behavior change to any existing test's assertions).

# `aivyx-checkpoint` adoption Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the shared `aivyx-checkpoint` crate into `aivyx`'s `fs.write`/`fs.delete`/`shell.exec` tools so a git-ref snapshot is taken before each, giving real undo (`git log`/`git show`/`GitCheckpointer::restore_to`) without ever touching the user's real git state.

**Architecture:** A new defaulted `Tool` trait method (`mutates_fs_root`, default `false`) marks exactly the three `fs_root`-scoped mutating tools; `ConcreteAgent` gains an `Option<Arc<GitCheckpointer>>` field (mirroring the existing `budget_gate`/`rate_gate` pattern exactly) checked once, centrally, in `run_tool_call` right before the existing `tool.execute(...)` call. `aivyx.rs` builds one `GitCheckpointer` for `fs_root` and threads it into both real same-process agent-construction sites (`daemon_agent`, `child_agent`).

**Tech Stack:** Rust, edition 2024. `aivyx-checkpoint` (new pinned `git` dependency), re-exported from `aivyx-core` mirroring the existing `aivyx-confine` re-export.

## Global Constraints

- Design doc: `docs/superpowers/specs/2026-08-18-aivyx-checkpoint-adoption-design.md` — read its three "Finding" sections before touching any file; they explain *why* this plan's shape differs from the original combined design.
- `aivyx-checkpoint` pinned at `rev = "1292d6cbda34aa514856b81e11635f7385a4d168"` — exact, everywhere it's referenced.
- The new trait method is `fn mutates_fs_root(&self) -> bool { false }` — **not** `mutates_outside_session` (that name/polarity was the original design's mistake, corrected by Finding 1). Only `FsWriteTool`, `FsDeleteTool`, `ShellExecTool` override it to `true`. No other `Tool` implementation anywhere in the workspace is touched by this plan.
- `ConcreteAgent`'s new builder method is `with_checkpointer(mut self, checkpointer: Option<Arc<aivyx_checkpoint::GitCheckpointer>>) -> Self` — takes `Option`, matching `with_budget_gate`/`with_rate_gate`'s exact existing shape, not a bare `Arc`.
- This pass wires the checkpointer into exactly two `ConcreteAgent` construction sites in `crates/aivyx-cli/src/bin/aivyx.rs`: `daemon_agent` and `child_agent`. No other file constructs a `ConcreteAgent` in this plan — the other 7 real sites (`aivyx-channel`, `aivyx-discord`, `aivyx-slack`, `aivyx-telegram`, `aivyx-team`) are explicitly out of scope (Finding 3); do not touch them.
- `deny_paths` is derived by walking `fs_root` and classifying every entry via the existing `SensitivePolicy::classify` — never by reading `SensitivePolicy`'s `extra_deny` field directly (Finding 2 — it's always empty in the real binary).
- `aivyx-core` re-exports `pub use aivyx_checkpoint::GitCheckpointer;` from `lib.rs`, mirroring the existing `pub use aivyx_confine::{ExecutionConfiner, NoopConfiner, default_confiner};` at the same file. `aivyx-cli` gains **no** direct `aivyx-checkpoint` dependency of its own — it calls `aivyx_core::GitCheckpointer::detect(...)`.
- No new `[checkpoint]` TOML config section, no CLI flag, no operator-facing config surface anywhere in this plan.
- `git.rs`'s three tools and `workspace.*` tools are untouched — they protect/operate on different roots than `fs_root` (see design doc's "Explicitly out of scope").

---

## Task 1: `mutates_fs_root` trait method + the three real overrides

**Files:**
- Modify: `crates/aivyx-core/src/lib.rs:907-934` (the `Tool` trait)
- Modify: `crates/aivyx-core/src/tools/fs.rs` (`FsWriteTool`, `FsDeleteTool` impls, plus their test module)
- Modify: `crates/aivyx-core/src/tools/shell.rs` (`ShellExecTool` impl, plus its test module)

**Interfaces:**
- Produces: `Tool::mutates_fs_root(&self) -> bool` (defaulted `false`), overridden `true` by `FsWriteTool`, `FsDeleteTool`, `ShellExecTool`. Every other `Tool` implementation in the workspace is unaffected (default applies, zero code change).

This task is self-contained: it touches only the trait definition and the three tools, is independently buildable, and needs no `ConcreteAgent`/`aivyx.rs` changes yet.

- [ ] **Step 1: Write the failing tests**

In `crates/aivyx-core/src/tools/fs.rs`'s `#[cfg(test)] mod tests` block (starts at line 1624), add near the existing `fs_read_output_is_untrusted_for_bulwark` test (it already uses the `build_tool`/`SandboxDir` pattern this mirrors):

```rust
    #[test]
    fn fs_write_mutates_fs_root() {
        let sandbox = SandboxDir::new();
        assert!(build_write_tool(&sandbox).mutates_fs_root());
    }

    #[test]
    fn fs_delete_mutates_fs_root() {
        let sandbox = SandboxDir::new();
        assert!(build_delete_tool(&sandbox).mutates_fs_root());
    }

    #[test]
    fn fs_read_does_not_mutate_fs_root() {
        // The default (false) — fs.read is unmodified by this plan, proving
        // the trait's default polarity without touching FsReadTool's impl.
        let sandbox = SandboxDir::new();
        assert!(!build_tool(&sandbox).mutates_fs_root());
    }
```

In `crates/aivyx-core/src/tools/shell.rs`'s `#[cfg(test)] mod tests` block (starts at line 694), add near the top of the test functions (after `build_tool` is defined):

```rust
    #[test]
    fn shell_exec_mutates_fs_root() {
        let scratch = Scratch::new();
        assert!(build_tool(&scratch.dir).mutates_fs_root());
    }
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cd /home/julian/Projects/Rust/aivyx && cargo test -p aivyx-core fs_write_mutates_fs_root fs_delete_mutates_fs_root fs_read_does_not_mutate_fs_root shell_exec_mutates_fs_root`
Expected: FAIL — `no method named mutates_fs_root found`.

- [ ] **Step 3: Add the trait method**

In `crates/aivyx-core/src/lib.rs`, inside the `Tool` trait (currently lines 907-934), add after `output_is_untrusted`'s closing brace and before the trait's own closing `}` (line 934):

```rust

    /// Whether this tool can mutate the `fs_root` sandbox directly on
    /// disk. Gates `aivyx-checkpoint`'s git-ref snapshot: the turn loop
    /// checkpoints `fs_root`'s worktree immediately before executing any
    /// tool for which this returns `true`. Default `false` — the vast
    /// majority of tools (Gmail, Notion, Drive, Calendar, …) mutate
    /// something, but nothing under `fs_root`, so checkpointing them
    /// would be pure overhead for zero protective benefit. Only
    /// `fs.write`, `fs.delete`, and `shell.exec` override this to
    /// `true` — see `aivyx-checkpoint` adoption design's Finding 1 for
    /// why this is opt-in (not opt-out, unlike `aivyx-coder`'s own
    /// analogous trait method) in this codebase specifically.
    fn mutates_fs_root(&self) -> bool {
        false
    }
```

- [ ] **Step 4: Override it on the three real tools**

In `crates/aivyx-core/src/tools/fs.rs`, find `impl Tool for FsWriteTool {` (currently line 636). Immediately after its `fn name(&self) -> &str { "fs.write" }` block, insert:

```rust

    /// Writes land on disk under fs_root — checkpoint before every call.
    fn mutates_fs_root(&self) -> bool {
        true
    }
```

Find `impl Tool for FsDeleteTool {` (currently line 1096). Immediately after its `fn name(&self) -> &str { "fs.delete" }` block, insert:

```rust

    /// Deletes remove content from fs_root — checkpoint before every call.
    fn mutates_fs_root(&self) -> bool {
        true
    }
```

In `crates/aivyx-core/src/tools/shell.rs`, find `impl Tool for ShellExecTool {` (currently line 420). Immediately after its `fn name(&self) -> &str { "shell.exec" }` block, insert:

```rust

    /// An arbitrary shell command can mutate anything under fs_root —
    /// checkpoint before every call, same as fs.write/fs.delete.
    fn mutates_fs_root(&self) -> bool {
        true
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p aivyx-core fs_write_mutates_fs_root fs_delete_mutates_fs_root fs_read_does_not_mutate_fs_root shell_exec_mutates_fs_root`
Expected: PASS — 4 passed.

- [ ] **Step 6: Run the full aivyx-core suite and clippy**

Run: `cargo test -p aivyx-core && cargo clippy -p aivyx-core --all-targets`
Expected: PASS, clean — no other test broken by the new trait method (it's defaulted, so every other `Tool` impl in the crate compiles unchanged).

- [ ] **Step 7: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx
git add crates/aivyx-core/src/lib.rs crates/aivyx-core/src/tools/fs.rs crates/aivyx-core/src/tools/shell.rs
git commit -m "Add Tool::mutates_fs_root, overridden by fs.write/fs.delete/shell.exec

Defaulted false: aivyx's Tool trait has ~90+ implementations across
unrelated integration crates (Gmail, Notion, Drive, ...), none of which
touch fs_root, so an opt-out default (mirroring aivyx-coder's own
analogous trait method) would checkpoint the fs_root git worktree before
every one of them for no protective benefit. See the aivyx-checkpoint
adoption design's Finding 1.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 2: `ConcreteAgent` checkpointer wiring

**Files:**
- Modify: `Cargo.toml` (workspace root — new `[workspace.dependencies]` entry)
- Modify: `crates/aivyx-core/Cargo.toml`
- Modify: `crates/aivyx-core/src/lib.rs` (re-export)
- Modify: `crates/aivyx-core/src/agent.rs` (`ConcreteAgent` field, builder, dispatch hook, tests)

**Interfaces:**
- Consumes: `Tool::mutates_fs_root(&self) -> bool` (Task 1).
- Produces: `ConcreteAgent::with_checkpointer(self, checkpointer: Option<Arc<aivyx_checkpoint::GitCheckpointer>>) -> Self`; `aivyx_core::GitCheckpointer` (re-exported), with its existing public API unchanged (`detect`, `checkpoint`, `latest_ref`, `restore_to`) — Task 3 depends on both.

- [ ] **Step 1: Add the pinned dependency to the workspace root**

Read `/home/julian/Projects/Rust/aivyx/Cargo.toml` first (the `[workspace.dependencies]` section, around line 118-244). Immediately after the existing `aivyx-confine = { git = ..., rev = ..., default-features = false }` line (line 244), add:

```toml

# Shared git-ref checkpoint/rollback for fs_root's mutating tools
# (fs.write, fs.delete, shell.exec), adopted from the sibling aivyx-coder
# repo (via aivyx-checkpoint) rather than reimplemented here. Not on
# crates.io — pinned by commit SHA. No platform-specific backend (pure
# git plumbing, unlike aivyx-confine) so no default-features/target-gating
# split is needed.
aivyx-checkpoint = { git = "https://github.com/Aivyx-Agent/aivyx-checkpoint", rev = "1292d6cbda34aa514856b81e11635f7385a4d168" }
```

- [ ] **Step 2: Add it to `aivyx-core`'s own `Cargo.toml`**

Read `crates/aivyx-core/Cargo.toml` first. Immediately after the existing `[target.'cfg(not(target_os = "linux"))'.dependencies]` block (the `aivyx-confine` platform split, near the end of `[dependencies]`), add a plain entry to the main `[dependencies]` table (not target-gated — `aivyx-checkpoint` has no platform split):

```toml

# Shared git-ref checkpoint/rollback for fs_root's mutating tools, gating
# ConcreteAgent's dispatch hook on Tool::mutates_fs_root(). Re-exported
# below so aivyx-cli never needs its own direct dependency on it — same
# pattern as the aivyx-confine re-export just above.
aivyx-checkpoint = { workspace = true }
```

- [ ] **Step 3: Re-export `GitCheckpointer` from `aivyx-core`**

In `crates/aivyx-core/src/lib.rs`, find the existing line `pub use aivyx_confine::{ExecutionConfiner, NoopConfiner, default_confiner};` (line 87). Immediately after it, add:

```rust
pub use aivyx_checkpoint::GitCheckpointer;
```

- [ ] **Step 4: Verify the dependency wiring builds**

Run: `cd /home/julian/Projects/Rust/aivyx && cargo build -p aivyx-core`
Expected: clean success — pulls `aivyx-checkpoint` for the first time; no other code references it yet, so nothing else should change.

- [ ] **Step 5: Write the failing tests**

In `crates/aivyx-core/src/agent.rs`'s `#[cfg(test)] mod tests` block, add (near the existing `rate_gate_throttles_after_cap_and_audits` test, which this mirrors structurally — real tool + `ToolRegistry` + `VecPlanner`-driven `agent.turn(...)` dispatch):

```rust
    #[tokio::test]
    async fn checkpoint_fires_only_for_mutates_fs_root_tools() {
        // A real git-backed fs_root, a real FsWriteTool (mutates_fs_root
        // == true) and a FakeTool standing in for an unrelated mutating
        // tool (mutates_fs_root == false, the default — e.g. what
        // aivyx-gmail's SendTool would inherit). Dispatch both through a
        // real turn; assert the checkpoint ref count only grows for the
        // fs.write call.
        let dir = tempfile::tempdir().unwrap();
        aivyx_checkpoint::test_support::init_repo(dir.path()).await;
        let fs_root = dir.path().to_path_buf();

        let write_tool: Arc<dyn Tool> = Arc::new(
            crate::tools::fs::FsWriteToolConfig::new(fs_root.clone())
                .build()
                .expect("fs_root must be canonicalizable"),
        );
        let unrelated_tool: Arc<dyn Tool> =
            Arc::new(FakeTool::new_bare("gmail.send", "gmail.send"));
        let write_id = write_tool.id();
        let unrelated_id = unrelated_tool.id();

        let checkpointer = Arc::new(
            aivyx_checkpoint::GitCheckpointer::detect(&fs_root, vec![])
                .await
                .expect("fs_root is a real git repo"),
        );

        let caps = CapabilitySet::from_scopes([
            Scope::parse(&format!("fs.write:{}/**", fs_root.display())).unwrap(),
            Scope::parse("gmail.send").unwrap(),
        ]);
        let registry = Arc::new(ToolRegistry::new(vec![write_tool, unrelated_tool]));
        let audit = RecordingAudit::new();

        let plan = vec![
            NextStep::ToolCall {
                tool_id: write_id,
                input: json!({ "path": "new.txt", "content": "hello" }),
                auto_corrected_from: None,
                extracted_from_text: None,
            },
            NextStep::ToolCall {
                tool_id: unrelated_id,
                input: json!({}),
                auto_corrected_from: None,
                extracted_from_text: None,
            },
            NextStep::FinalMessage("done".to_string()),
        ];
        let plan_arc = Arc::new(plan);

        let agent = ConcreteAgent::new(
            AgentId::new(),
            caps,
            registry,
            audit.clone(),
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        )
        .with_checkpointer(Some(checkpointer))
        .with_repeat_call_limit(0);

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "write then send");
        let _ = agent.turn(message, &channel).await;

        let refs = aivyx_checkpoint::test_support::git(
            dir.path(),
            &["for-each-ref", "refs/aivyx/checkpoints/"],
        )
        .await;
        let ref_count = refs.lines().filter(|l| !l.is_empty()).count();
        assert_eq!(
            ref_count, 1,
            "exactly one checkpoint (before fs.write), none for the unrelated tool: {refs}"
        );
    }

    #[tokio::test]
    async fn restore_to_reverts_a_checkpoint_taken_by_the_dispatch_hook() {
        // Full round-trip: the checkpoint the hook takes before a real
        // fs.write is a real, restorable snapshot via the same
        // GitCheckpointer instance the agent used.
        let dir = tempfile::tempdir().unwrap();
        aivyx_checkpoint::test_support::init_repo(dir.path()).await;
        let fs_root = dir.path().to_path_buf();
        std::fs::write(fs_root.join("tracked.txt"), "v1\n").unwrap();
        aivyx_checkpoint::test_support::git(dir.path(), &["add", "-A"]).await;
        aivyx_checkpoint::test_support::git(dir.path(), &["commit", "-q", "-m", "v1"]).await;

        let write_tool: Arc<dyn Tool> = Arc::new(
            crate::tools::fs::FsWriteToolConfig::new(fs_root.clone())
                .build()
                .expect("fs_root must be canonicalizable"),
        );
        let write_id = write_tool.id();

        let checkpointer = Arc::new(
            aivyx_checkpoint::GitCheckpointer::detect(&fs_root, vec![])
                .await
                .expect("fs_root is a real git repo"),
        );
        let checkpointer_for_restore = Arc::clone(&checkpointer);

        let caps = CapabilitySet::from_scopes([
            Scope::parse(&format!("fs.write:{}/**", fs_root.display())).unwrap(),
        ]);
        let registry = Arc::new(ToolRegistry::new(vec![write_tool]));
        let audit = RecordingAudit::new();
        let plan = vec![
            NextStep::ToolCall {
                tool_id: write_id,
                input: json!({ "path": "tracked.txt", "content": "v2 (bad edit)" }),
                auto_corrected_from: None,
                extracted_from_text: None,
            },
            NextStep::FinalMessage("done".to_string()),
        ];
        let plan_arc = Arc::new(plan);

        let agent = ConcreteAgent::new(
            AgentId::new(),
            caps,
            registry,
            audit.clone(),
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        )
        .with_checkpointer(Some(checkpointer));

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "overwrite tracked.txt");
        let _ = agent.turn(message, &channel).await;

        assert_eq!(
            std::fs::read_to_string(fs_root.join("tracked.txt")).unwrap(),
            "v2 (bad edit)"
        );

        let checkpoint_ref = checkpointer_for_restore
            .latest_ref(&CancellationToken::new())
            .await
            .expect("the dispatch hook must have taken a checkpoint");
        checkpointer_for_restore
            .restore_to(&checkpoint_ref, &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(fs_root.join("tracked.txt")).unwrap(),
            "v1\n",
            "restore_to must revert to the pre-write checkpoint"
        );
    }
```

- [ ] **Step 6: Run the tests to verify they fail to compile**

Run: `cargo test -p aivyx-core checkpoint_fires_only_for_mutates_fs_root_tools restore_to_reverts_a_checkpoint_taken_by_the_dispatch_hook`
Expected: FAIL — `no method named with_checkpointer found for struct ConcreteAgent`, and `no field checkpointer on type ConcreteAgent`.

- [ ] **Step 7: Add the field, constructor default, and builder**

In `crates/aivyx-core/src/agent.rs`, find `pub struct ConcreteAgent {` (line 141). Immediately after the `cycle_config: Option<CycleConfig>,` field (the struct's last field, line 213, right before its closing `}`), add:

```rust
    /// `aivyx-checkpoint` — snapshots `fs_root`'s worktree to a shadow
    /// git ref before any tool call for which `Tool::mutates_fs_root()`
    /// is `true`. `None` (the default) preserves pre-checkpoint behavior
    /// byte-for-byte — the same shape as `budget_gate`/`rate_gate`.
    checkpointer: Option<Arc<aivyx_checkpoint::GitCheckpointer>>,
```

In `ConcreteAgent::new` (starts line 217), find the `cycle_config: None,` line inside the returned struct literal (line 235, the constructor's last field). Immediately after it, add:

```rust
            checkpointer: None,
```

Immediately after the existing `with_rate_gate` method (ends at line 309, right before the `impl` block's closing `}` at line 310), add:

```rust

    /// Attach an `aivyx-checkpoint` `GitCheckpointer` for `fs_root`. See
    /// the [`Self::checkpointer`] field doc for semantics. `None` means
    /// "no checkpointer" (either checkpointing is disabled, or `fs_root`
    /// isn't a git repository), preserving pre-checkpoint behavior.
    pub fn with_checkpointer(
        mut self,
        checkpointer: Option<Arc<aivyx_checkpoint::GitCheckpointer>>,
    ) -> Self {
        self.checkpointer = checkpointer;
        self
    }
```

- [ ] **Step 8: Add the dispatch hook**

In `run_tool_call` (starts line 1002), find the existing dispatch lines (currently at 1263-1264):

```rust
        let step_start = Instant::now();
        let mut outcome = tool.execute(input, &ctx).await;
```

Replace with:

```rust
        // aivyx-checkpoint — snapshot fs_root before anything that can
        // mutate it, so a bad fs.write/fs.delete/shell.exec is always
        // recoverable via GitCheckpointer::restore_to. Best-effort: a
        // failed checkpoint logs and the call proceeds (see
        // GitCheckpointer::checkpoint's own contract).
        if tool.mutates_fs_root()
            && let Some(checkpointer) = &self.checkpointer
        {
            checkpointer.checkpoint(tool.name(), cancellation).await;
        }

        let step_start = Instant::now();
        let mut outcome = tool.execute(input, &ctx).await;
```

- [ ] **Step 9: Run the tests to verify they pass**

Run: `cargo test -p aivyx-core checkpoint_fires_only_for_mutates_fs_root_tools restore_to_reverts_a_checkpoint_taken_by_the_dispatch_hook`
Expected: PASS — 2 passed.

- [ ] **Step 10: Run the full aivyx-core suite and clippy**

Run: `cargo test -p aivyx-core && cargo clippy -p aivyx-core --all-targets`
Expected: PASS, clean.

- [ ] **Step 11: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx
git add Cargo.toml crates/aivyx-core/Cargo.toml crates/aivyx-core/src/lib.rs crates/aivyx-core/src/agent.rs Cargo.lock
git commit -m "Wire GitCheckpointer into ConcreteAgent's dispatch loop

New checkpointer: Option<Arc<GitCheckpointer>> field + with_checkpointer
builder, mirroring the existing budget_gate/rate_gate pattern exactly.
The dispatch hook in run_tool_call checkpoints fs_root immediately
before any tool call where Tool::mutates_fs_root() is true, right
before the existing tool.execute(...) call. aivyx-checkpoint is
re-exported from aivyx-core (as GitCheckpointer), mirroring the
existing aivyx-confine re-export, so aivyx-cli needs no direct
dependency of its own.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 3: Construction — `collect_sensitive_paths_under` + wiring `daemon_agent`/`child_agent`

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs`

**Interfaces:**
- Consumes: `aivyx_core::GitCheckpointer::detect` (Task 2), `ConcreteAgent::with_checkpointer` (Task 2), `aivyx_core::sensitive_paths::SensitivePolicy::classify` (already exists).
- Produces: a private `collect_sensitive_paths_under(root: &Path, policy: &SensitivePolicy) -> Vec<PathBuf>` helper, and a `checkpointer: Option<Arc<GitCheckpointer>>` local variable threaded into both `daemon_agent` and `child_agent`'s builder chains.

This task must run on an isolated branch/worktree — set that up before starting (the controller dispatching this task handles worktree/branch setup via `superpowers:using-git-worktrees`, not a step written here). `aivyx-cli`'s crate name in `cargo test`/`cargo build` invocations below is `aivyx-cli`, and the binary target is named `aivyx`.

- [ ] **Step 1: Write the failing test for `collect_sensitive_paths_under`**

In `crates/aivyx-cli/src/bin/aivyx.rs`'s `#[cfg(test)] mod tests` block (starts at line 9878, already has a `Scratch` helper at line 9940), add:

```rust
    #[test]
    fn collect_sensitive_paths_under_finds_curated_matches_only() {
        let scratch = Scratch::new();
        std::fs::write(scratch.dir.join(".env"), b"API_KEY=secret\n").unwrap();
        std::fs::write(scratch.dir.join("notes.md"), b"hello\n").unwrap();
        let ssh_dir = scratch.dir.join(".ssh");
        std::fs::create_dir(&ssh_dir).unwrap();
        std::fs::write(ssh_dir.join("id_rsa"), b"not a real key\n").unwrap();

        let policy = aivyx_core::sensitive_paths::SensitivePolicy::new(vec![], vec![]);
        let hits = collect_sensitive_paths_under(&scratch.dir, &policy);

        let env_canonical = std::fs::canonicalize(scratch.dir.join(".env")).unwrap();
        let ssh_canonical = std::fs::canonicalize(&ssh_dir).unwrap();
        let notes_canonical = std::fs::canonicalize(scratch.dir.join("notes.md")).unwrap();

        assert!(hits.contains(&env_canonical), "must flag .env: {hits:?}");
        assert!(
            hits.contains(&ssh_canonical),
            "must flag the .ssh directory itself: {hits:?}"
        );
        assert!(
            !hits.contains(&notes_canonical),
            "must not flag an ordinary file: {hits:?}"
        );
        // id_rsa under .ssh is not separately enumerated — the .ssh
        // directory's own entry is enough for GitCheckpointer's
        // exclude_pathspecs (a directory pathspec excludes its whole
        // subtree), and re-descending would be wasted work.
        let id_rsa_canonical = std::fs::canonicalize(ssh_dir.join("id_rsa")).unwrap();
        assert!(
            !hits.contains(&id_rsa_canonical),
            "children of an already-matched directory should not be separately listed: {hits:?}"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cd /home/julian/Projects/Rust/aivyx && cargo test -p aivyx-cli --bin aivyx collect_sensitive_paths_under_finds_curated_matches_only`
Expected: FAIL — `cannot find function collect_sensitive_paths_under in this scope`.

- [ ] **Step 3: Write `collect_sensitive_paths_under`**

In `crates/aivyx-cli/src/bin/aivyx.rs`, find where `canonical_root` is resolved (currently line 5951: `let canonical_root = fs_read.sandbox_root().to_path_buf();`). Above that function's enclosing scope — i.e., as a new top-level `fn` elsewhere in the file, near other small top-level helper functions (not nested inside the giant `run()`/`main()` function) — add:

```rust
/// Walks `root` recursively and returns the canonical path of every entry
/// `policy.classify(...)` flags as sensitive. Closes the gap
/// `SensitivePolicy`'s `extra_deny` leaves open for `aivyx-checkpoint`'s
/// `deny_paths` — `extra_deny` is never populated from real operator
/// config at this binary's own construction site (see the aivyx-checkpoint
/// adoption design's Finding 2), so the real protection has to come from
/// walking the same pattern-based classifier that already guards
/// fs.read/fs.write. Called once at startup: a secret file created under
/// `fs_root` after this walk isn't covered until the process restarts —
/// the same "fixed at construction, not re-derived per checkpoint"
/// limitation `GitCheckpointer`'s own `deny_paths` API already has.
/// Once a directory itself matches, its children are not separately
/// descended into: `exclude_pathspecs`' git pathspec semantics exclude a
/// matched directory's whole subtree from one entry.
fn collect_sensitive_paths_under(
    root: &std::path::Path,
    policy: &aivyx_core::sensitive_paths::SensitivePolicy,
) -> Vec<std::path::PathBuf> {
    let mut hits = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(canonical) = path.canonicalize() else {
                continue;
            };
            if policy.classify(&canonical).is_some() {
                hits.push(canonical);
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    hits
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p aivyx-cli --bin aivyx collect_sensitive_paths_under_finds_curated_matches_only`
Expected: PASS — 1 passed.

- [ ] **Step 5: Build the checkpointer once, near `canonical_root`**

Read the surrounding code first (around line 5951-5970). Immediately after the `let canonical_root = fs_read.sandbox_root().to_path_buf();` line, add:

```rust

    // aivyx-checkpoint — git-ref checkpoint/rollback for fs_root's
    // mutating tools (fs.write, fs.delete, shell.exec). Built once, shared
    // by daemon_agent and child_agent below (both run in this process
    // against this same fs_root). detect() already tolerates fs_root not
    // being a git repository (None, one log line) — checkpointer then
    // stays None for this session, same graceful-degradation contract as
    // every other optional agent knob (budget_gate, rate_gate, ...).
    let checkpoint_deny_paths = collect_sensitive_paths_under(&canonical_root, &sensitive_policy);
    let checkpointer: Option<std::sync::Arc<aivyx_core::GitCheckpointer>> =
        aivyx_core::GitCheckpointer::detect(&canonical_root, checkpoint_deny_paths)
            .await
            .map(std::sync::Arc::new);
```

`sensitive_policy` is already in scope at this point (constructed a few lines earlier, around line 5901, before `fs_read`).

- [ ] **Step 6: Wire it into `daemon_agent`**

Read the surrounding code first (around line 8624-8634). Find:

```rust
        let daemon_agent = ConcreteAgent::new(
            AgentId::new(),
            capabilities,
            tools,
            audit,
            planner_factory,
        )
        .with_tool_allowlist(daemon_tool_allowlist)
        .with_memory_topic_prefix(memory_topic_prefix)
        .with_budget_gate(daemon_budget_gate)
        .with_rate_gate(daemon_rate_gate);
```

Replace with:

```rust
        let daemon_agent = ConcreteAgent::new(
            AgentId::new(),
            capabilities,
            tools,
            audit,
            planner_factory,
        )
        .with_tool_allowlist(daemon_tool_allowlist)
        .with_memory_topic_prefix(memory_topic_prefix)
        .with_budget_gate(daemon_budget_gate)
        .with_rate_gate(daemon_rate_gate)
        .with_checkpointer(checkpointer.clone());
```

- [ ] **Step 7: Wire it into `child_agent`**

Read the surrounding code first (around line 8163-8171). Find:

```rust
        let child_agent = ConcreteAgent::new(
            AgentId::new(),
            child_capabilities,
            Arc::clone(&tools_for_factory),
            Arc::clone(&audit_for_factory),
            child_planner_factory,
        )
        .with_tool_allowlist(child_tool_allowlist)
        .with_memory_topic_prefix(child_memory_topic_prefix);
```

Replace with:

```rust
        let child_agent = ConcreteAgent::new(
            AgentId::new(),
            child_capabilities,
            Arc::clone(&tools_for_factory),
            Arc::clone(&audit_for_factory),
            child_planner_factory,
        )
        .with_tool_allowlist(child_tool_allowlist)
        .with_memory_topic_prefix(child_memory_topic_prefix)
        .with_checkpointer(checkpointer.clone());
```

`child_agent`'s construction is inside a closure (the `child_factory` built earlier in the file) — `checkpointer` must be captured by that closure. If `cargo build` reports a capture/lifetime error here, the fix is to clone `checkpointer` into a `_for_factory`-suffixed binding before the closure (mirroring the existing `tools_for_factory`/`audit_for_factory` pattern already used for this exact closure) and reference that clone inside — do not restructure the closure itself.

- [ ] **Step 8: Verify it builds**

Run: `cargo build -p aivyx-cli`
Expected: clean success. If Step 7's capture issue arises, apply the `checkpointer_for_factory` fix described there and rebuild.

- [ ] **Step 9: Run the full workspace test suite**

Run: `cargo test --workspace 2>&1 | grep "test result"`
Expected: every crate's count matches its pre-Task-1 baseline, plus the new tests added across Tasks 1-3 (4 in `aivyx-core`'s fs/shell tests, 2 in `aivyx-core::agent`, 1 in `aivyx-cli`'s bin tests) — 7 more passing overall, zero failures.

- [ ] **Step 10: Run clippy on the full workspace**

Run: `cargo clippy --workspace --all-targets`
Expected: clean.

- [ ] **Step 11: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx
git add crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "Wire GitCheckpointer into daemon_agent and child_agent

Builds one checkpointer for fs_root (deny_paths derived by walking
fs_root through the existing sensitive-path classifier, since
SensitivePolicy's extra_deny is never populated from real config — see
the aivyx-checkpoint adoption design's Finding 2) and threads it into
both real same-process ConcreteAgent construction sites. The other 7
real construction sites (standalone Discord/Slack/Telegram bot mode,
aivyx-team) are an explicit, documented gap for follow-on work (Finding
3), not touched here.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Self-review notes

**Spec coverage:** the design doc's five numbered sections map onto
tasks here — "1. New trait method" → Task 1; "2. Agent-level wiring" +
"3. The hook" → Task 2; "4. Construction" → Task 3; "5. Dependency" →
Task 2 Steps 1-4. Findings 1-3 are each reflected in a Global Constraint
and in the task that resolves them (Finding 1 → Task 1's default
polarity; Finding 2 → Task 3's `collect_sensitive_paths_under`; Finding
3 → Task 3's Global-Constraints-pinned scope to exactly two construction
sites). "Explicitly out of scope" in the design (the other 7 construction
sites, `git.rs`, `workspace.*`, no new config) → correctly absent from
every task's file list. "Testing" section's three items → Task 1's
override tests, Task 2's dispatch + round-trip tests, Task 3's
sensitive-path-collection test respectively.

**Placeholder scan:** no TBD/TODO; every step shows complete, real code
transcribed from files read during planning (not paraphrased) — the
`ConcreteAgent` struct/constructor/builder snippets, the `run_tool_call`
replacement, the `daemon_agent`/`child_agent` before/after blocks, and
the `SandboxDir`/`Scratch`/`build_write_tool`/`build_delete_tool`/`build_tool`
test-helper names are all copied verbatim from the actual current source,
not invented.

**Type consistency:** `mutates_fs_root(&self) -> bool` (Task 1) is called
identically in Task 2's dispatch hook (`tool.mutates_fs_root()`).
`with_checkpointer(self, Option<Arc<GitCheckpointer>>) -> Self` (Task 2)
is called identically in Task 3 (`.with_checkpointer(checkpointer.clone())`,
`checkpointer: Option<Arc<GitCheckpointer>>`). `collect_sensitive_paths_under`'s
signature (Task 3 Step 3) matches its own test's call (Step 1) and its
real call site (Step 5) exactly. `aivyx_core::GitCheckpointer` (the
re-export added in Task 2 Step 3) is the exact path Task 3 uses at its
real construction site — never `aivyx_checkpoint::GitCheckpointer`
directly in `aivyx-cli`, matching the Global Constraint.

**One residual risk flagged, not silently resolved:** Task 3 Step 7 notes
a possible closure-capture wrinkle for `child_agent` (built inside a
pre-existing closure) that this plan can't fully verify without running
the actual build — the step gives the implementer a concrete, scoped fix
(mirror the existing `_for_factory` clone pattern) rather than leaving it
as an unexplained "add appropriate handling."

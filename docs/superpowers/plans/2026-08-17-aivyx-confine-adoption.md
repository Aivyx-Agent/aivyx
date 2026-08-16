# aivyx-confine Adoption Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the shared `aivyx-confine` crate (Landlock + seccomp-bpf process confinement) into `aivyx`'s `ShellExecTool` and `git.rs`'s three tools, closing the gap `aivyx`'s own `docs/THREAT_MODEL.md` admits ("Container-level sandboxing of tools: No... forward work").

**Architecture:** `aivyx-core` depends on `aivyx-confine` directly and re-exports its public types, so `aivyx-cli` (the binary) never needs its own separate dependency on it. `ShellExecTool` gets one `ExecutionConfiner` built once at tool-construction time (mirrors its existing `SensitivePolicy` field exactly). `git.rs`'s three tools each get a fresh, narrowly-scoped `LandlockConfiner` built once per `execute()` call, right after the target repo is resolved from the operator's allow-set — a different shape than `shell.exec`'s, because the allow-set can name many repos and `LandlockConfiner::new` only grants write access to one root at a time. A new `[confine]` config section (one field, `require_enforcement`) threads through both, following the exact `Sourced<T>`/`RawToml` pattern `[access] confirm_destructive` already uses.

**Tech Stack:** Rust. `aivyx-confine` (pinned `git` dependency, `rev = "03de88616789f9a52adf41afe71fd14d538f3144"` — the current `master` HEAD of `https://github.com/Aivyx-Agent/aivyx-confine` as of this plan; Task 1 verifies this is still current before pinning).

## Global Constraints

- `aivyx-confine`'s public API (verified during the aivyx-confine project's own final review, still current): `ExecutionConfiner` trait (`fn confine(&self, command: tokio::process::Command) -> tokio::process::Command`), `NoopConfiner`, `LandlockConfiner::new(cwd: &Path, extra_read_paths: &[PathBuf], deny_paths: &[PathBuf], require_enforcement: bool) -> Self`, `default_confiner(cwd: &Path, extra_read_paths: &[PathBuf], deny_paths: &[PathBuf], require_enforcement: bool) -> Arc<dyn ExecutionConfiner>`.
- `deny_paths`/`extra_read_paths` are always `&[]` (empty) everywhere in this plan — deferred to backlog per the design doc, not built here. Every `LandlockConfiner::new`/`default_confiner` call in this plan passes `&[]` for both.
- `require_enforcement` defaults to `true` (fail-closed) everywhere — matches `aivyx-coder`'s own `aivyx-confine` usage and this design's own stated default.
- No change to `aivyx-confine` itself, no change to `aivyx-tool/src/sandbox.rs`/`bridge.rs`'s separate `[[tool_process]]` sandboxing mechanism — out of scope per the design doc.
- All work in this plan happens on an isolated branch/worktree of `aivyx` — never directly on its default branch — set up via `superpowers:using-git-worktrees` before Task 1 is dispatched, and confirmed with the user first (an actively-developed product repo, same caution `aivyx-coder`'s own migration used).
- Pushing the finished branch requires separate user confirmation — do not push without asking, regardless of how routine the change looks.

---

## Task 1: Add the `aivyx-confine` dependency and re-export it from `aivyx-core`

**Files:**
- Modify: `/home/julian/Projects/Rust/aivyx/Cargo.toml`
- Modify: `/home/julian/Projects/Rust/aivyx/crates/aivyx-core/Cargo.toml`
- Modify: `/home/julian/Projects/Rust/aivyx/crates/aivyx-core/src/lib.rs`

**Interfaces:**
- Produces: `aivyx_core::{ExecutionConfiner, NoopConfiner, LandlockConfiner, default_confiner}`, re-exported so `aivyx-cli` (Task 5) never needs its own direct dependency on `aivyx-confine`.

- [ ] **Step 1: Confirm the current `aivyx-confine` `master` commit**

```bash
git ls-remote https://github.com/Aivyx-Agent/aivyx-confine master
```

Expected: a 40-character SHA. If it differs from `03de88616789f9a52adf41afe71fd14d538f3144` (the SHA this plan was written against), use the SHA `git ls-remote` actually returns for every `rev = "..."` value in this plan instead — the crate's public API (see Global Constraints) has been stable since that commit, so a newer `rev` is safe to use, but always pin the real current value, not this plan's possibly-stale one.

- [ ] **Step 2: Add the dependency to the workspace**

Read `/home/julian/Projects/Rust/aivyx/Cargo.toml`'s `[workspace.dependencies]` section first (starts around line 118) to see the existing style, then add, alongside the other entries:

```toml
aivyx-confine = { git = "https://github.com/Aivyx-Agent/aivyx-confine", rev = "03de88616789f9a52adf41afe71fd14d538f3144" }
```

(Using the real SHA from Step 1, not necessarily this literal string.)

- [ ] **Step 3: Add it to `aivyx-core`'s own `Cargo.toml`**

In `/home/julian/Projects/Rust/aivyx/crates/aivyx-core/Cargo.toml`'s `[dependencies]` section, add:

```toml
aivyx-confine = { workspace = true }
```

- [ ] **Step 4: Re-export the public types from `aivyx-core`'s `lib.rs`**

Read the top of `/home/julian/Projects/Rust/aivyx/crates/aivyx-core/src/lib.rs` to find where existing `pub use` statements live (near the crate's other module declarations), and add, alongside them:

```rust
pub use aivyx_confine::{ExecutionConfiner, LandlockConfiner, NoopConfiner, default_confiner};
```

- [ ] **Step 5: Verify it builds**

```bash
cd /home/julian/Projects/Rust/aivyx
cargo build -p aivyx-core
```

Expected: clean success. This confirms the dependency resolves and the re-export compiles — nothing else in the workspace references these symbols yet, so this is the full verification for this task.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/aivyx-core/Cargo.toml crates/aivyx-core/src/lib.rs
git commit -m "Add aivyx-confine dependency, re-export its public types from aivyx-core"
```

---

## Task 2: `[confine]` config section

**Files:**
- Modify: `/home/julian/Projects/Rust/aivyx/crates/aivyx-config/src/lib.rs`

**Interfaces:**
- Produces: `AivyxConfig.require_enforcement: Sourced<bool>` (flat field, defaults to `true` when `[confine] require_enforcement` is absent from the TOML file).

This mirrors `[access] confirm_destructive`'s existing resolution pattern exactly — same `Sourced<T>`/`RawToml` machinery, same shape, just a new top-level `[confine]` section instead of a field nested under `[access]`. Read each of the four real sites below in full before editing; each shows a working example of the exact same kind of edit this task makes, just for a different section.

- [ ] **Step 1: Write the failing test**

Find `aivyx-config/src/lib.rs`'s existing test module (search for `#[cfg(test)]`) and locate a test that loads a TOML string and checks a resolved field's value and `FieldSource` — `confirm_destructive`'s own test is the closest model; read it in full to match its exact structure (how it constructs a config file, calls the loader, and asserts on the result) before writing this one. Add, in the same test module:

```rust
#[test]
fn confine_require_enforcement_defaults_to_true_when_absent() {
    let toml_str = "";
    let config = load_config_from_str(toml_str).expect("empty config should load");
    assert!(config.require_enforcement.value);
    assert_eq!(config.require_enforcement.source, FieldSource::Default);
}

#[test]
fn confine_require_enforcement_reads_an_explicit_false() {
    let toml_str = "[confine]\nrequire_enforcement = false\n";
    let config = load_config_from_str(toml_str).expect("config should load");
    assert!(!config.require_enforcement.value);
    assert_eq!(config.require_enforcement.source, FieldSource::Toml);
}
```

**Before running this step, find the real name of the config-loading test helper** (`load_config_from_str` above is almost certainly not its real name) — search the test module for how `confirm_destructive`'s own test constructs an `AivyxConfig` from a TOML string, and use that exact function/method call instead. Same for `FieldSource::Toml`/`FieldSource::Default` — confirm these are the real variant names by checking `confirm_destructive`'s resolution code (search for `FieldSource::` nearby it), not just this plan's guess.

- [ ] **Step 2: Run the tests to verify they fail to compile**

```bash
cd /home/julian/Projects/Rust/aivyx
cargo test -p aivyx-config confine_require_enforcement
```

Expected: FAIL — `no field require_enforcement on type AivyxConfig` (or similar — the field doesn't exist yet).

- [ ] **Step 3: Add the raw TOML section struct**

Find `RawGit` (search for `struct RawGit`) — a minimal single-field raw section struct, the closest model for what this step adds. Right after it (or anywhere among the other `RawXxx` struct definitions), add:

```rust
/// `[confine]` section deserialize target — whether OS-level process
/// confinement (Landlock + seccomp-bpf, via the `aivyx-confine` crate)
/// must succeed for `shell.exec`/`git.rs` to run a command at all.
#[derive(Debug, Default, Deserialize)]
struct RawConfine {
    #[serde(default)]
    require_enforcement: Option<bool>,
}
```

- [ ] **Step 4: Add the field to `RawToml`**

Find the `RawToml` struct (search for `struct RawToml`, around line 3660-3700). Add, alongside its other `#[serde(default)]`-annotated fields (e.g. right after the `git: RawGit,` field):

```rust
    #[serde(default)]
    confine: RawConfine,
```

- [ ] **Step 5: Add the resolved field to `AivyxConfig`**

Find the `AivyxConfig` struct (search for `pub struct AivyxConfig`, around line 771) and its `confirm_destructive: Sourced<bool>` field. Add, right after it:

```rust
    /// `[confine] require_enforcement` — whether OS-level process
    /// confinement (Landlock + seccomp-bpf) must succeed for
    /// `shell.exec`/`git.rs` to run a command at all. `true` (fail-closed)
    /// by default, matching `aivyx-coder`'s own `aivyx-confine` usage.
    pub require_enforcement: Sourced<bool>,
```

- [ ] **Step 6: Add the resolution logic**

Find the `confirm_destructive` resolution site (search for `// --- confirm_destructive`, around line 5596-5602). Add, right after that block:

```rust
        // --- confine.require_enforcement -----------------------------
        // Fail-closed by default: if Landlock can't be established at
        // runtime, refuse to run the command rather than running
        // unconfined. An explicit `[confine] require_enforcement = false`
        // opts into the opposite (log + run unconfined) for operators on
        // kernels/platforms where Landlock genuinely isn't available.
        let require_enforcement = match toml.confine.require_enforcement {
            Some(b) => Sourced::new(b, FieldSource::Toml),
            None => Sourced::new(true, FieldSource::Default),
        };
```

- [ ] **Step 7: Add it to the final `AivyxConfig` struct literal**

Find where `confirm_destructive,` appears in the final `Ok(Self { ... })` struct literal (search for `confirm_destructive,` near the end of the parsing function, around line 7290). Add `require_enforcement,` right after it.

- [ ] **Step 8: Run the tests to verify they pass**

```bash
cargo test -p aivyx-config confine_require_enforcement
```

Expected: PASS — 2 passed.

- [ ] **Step 9: Run the full `aivyx-config` test suite**

```bash
cargo test -p aivyx-config
```

Expected: every pre-existing test still passes — this change only adds a field with a default, nothing existing should be affected.

- [ ] **Step 10: Commit**

```bash
git add crates/aivyx-config/src/lib.rs
git commit -m "Add [confine] require_enforcement config field"
```

---

## Task 3: `ShellExecTool` confinement

**Files:**
- Modify: `/home/julian/Projects/Rust/aivyx/crates/aivyx-core/src/tools/shell.rs`

**Interfaces:**
- Consumes: `aivyx_core::{ExecutionConfiner, default_confiner}` (Task 1).
- Produces: `ShellExecTool::confiner: Arc<dyn ExecutionConfiner>` (private field), `ShellExecTool::with_confiner(self, confiner: Arc<dyn ExecutionConfiner>) -> Self` (public builder method).

This mirrors the existing `sensitive: Arc<SensitivePolicy>` field and `.with_sensitive_policy(...)` builder method in this exact file — read both in full before editing (the field is on the `ShellExecTool` struct definition; the builder method is defined in `impl ShellExecTool`, alongside `cwd_root()`).

- [ ] **Step 1: Write the failing test**

Find this file's existing `#[cfg(test)] mod tests` block (near the bottom — the file's own tests include `execute_captures_stdout_and_exit_code`, `execute_uses_provided_cwd`, etc.; read one in full to match its exact setup style — how it builds a `ToolContext`, a temp `cwd_root`, and calls `.execute(...)`). Add:

```rust
#[tokio::test]
async fn execute_denies_a_write_outside_cwd_root_under_the_default_confiner() {
    let dir = tempfile::tempdir().unwrap();
    let tool = ShellExecToolConfig::new(dir.path().to_path_buf())
        .build()
        .expect("shell tool should build");

    // Outside the sandbox root entirely — /var/tmp, not another
    // tempfile::tempdir() (which would also resolve under /tmp,
    // itself write-granted by aivyx-confine's default write scope).
    let outside = tempfile::Builder::new().tempdir_in("/var/tmp").unwrap();
    let target = outside.path().join("should-not-exist.txt");

    let input = serde_json::json!({
        "cmd": format!("echo hi > {}", target.display()),
    });

    let outcome = tool.execute(input, &test_tool_context()).await;

    assert!(!target.exists(), "write outside cwd_root must be denied by Landlock");
    // The shell command itself still "completes" (sh runs, the redirect
    // just fails inside it) -- assert on the filesystem effect, not the
    // ToolOutcome variant, since `sh -c` swallows the redirect failure
    // into its own non-zero exit rather than a spawn-level Failed.
    let _ = outcome;
}
```

**Before running this step**, find the real helper this file's other tests use to construct a `&ToolContext<'_>` (search for how `execute_captures_stdout_and_exit_code` or a neighboring test builds one — `test_tool_context()` above is almost certainly not the real name) and use that instead. Also confirm `tempfile` is already a dev-dependency of this crate (check `aivyx-core/Cargo.toml`'s `[dev-dependencies]` — if it's missing, add `tempfile = "3"` there first, matching whatever version the workspace's other crates already pin).

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd /home/julian/Projects/Rust/aivyx
cargo test -p aivyx-core execute_denies_a_write_outside_cwd_root
```

Expected: FAIL — the write succeeds (`target.exists()` is `true`), since there's no confinement yet.

- [ ] **Step 3: Add the `confiner` field**

In `/home/julian/Projects/Rust/aivyx/crates/aivyx-core/src/tools/shell.rs`, find the `ShellExecTool` struct definition (has `id`, `cwd_root`, `schema`, `sensitive` fields) and add, right after `sensitive`:

```rust
    /// OS-level process confinement (Landlock + seccomp-bpf, via
    /// `aivyx-confine`) — the kernel-level counterpart to `sensitive`'s
    /// string-level guard above. Built as a real, on-by-default confiner
    /// at `build()` time so every caller gets it even if they never call
    /// `with_confiner` explicitly (mirrors `sensitive`'s own "default
    /// disabled ⇒ safe" shape, but inverted: this one defaults ON).
    confiner: Arc<dyn ExecutionConfiner>,
```

- [ ] **Step 4: Set the default in `build()`**

Find `ShellExecToolConfig::build()` (constructs and returns `Ok(ShellExecTool { id: ToolId::new(), cwd_root: Arc::from(canonical), schema: shell_exec_input_schema_value(), sensitive: Arc::new(crate::sensitive_paths::SensitivePolicy::disabled()) })`). Add a `confiner` field to that struct literal:

```rust
            confiner: default_confiner(&canonical, &[], &[], true),
```

- [ ] **Step 5: Add the `with_confiner` builder method**

Find the `.with_sensitive_policy(...)` builder method in `impl ShellExecTool` and add, right after it:

```rust
    /// Override the confiner `build()` set by default. The real binary
    /// call site uses this to pass the operator's configured
    /// `require_enforcement` value instead of the hardcoded `true`
    /// `build()` itself uses.
    pub fn with_confiner(mut self, confiner: Arc<dyn ExecutionConfiner>) -> Self {
        self.confiner = confiner;
        self
    }
```

- [ ] **Step 6: Apply confinement before spawning**

In `ShellExecTool::execute`, find `let mut command = Command::new("sh"); command.arg("-c").arg(&cmd).current_dir(&canonical_cwd);` through the `command.kill_on_drop(true);` line, right before `let child = match command.spawn() {`. Add, immediately before that line:

```rust
        let command = self.confiner.confine(command);
```

- [ ] **Step 7: Run the test to verify it passes**

```bash
cargo test -p aivyx-core execute_denies_a_write_outside_cwd_root
```

Expected: PASS.

- [ ] **Step 8: Run this file's full test suite**

```bash
cargo test -p aivyx-core --lib tools::shell
```

Expected: every pre-existing test in this file still passes — confinement scoped to `cwd_root` (and, in these tests, `/tmp` — Landlock's default write grant) should not break any test that writes inside the sandbox root, only the new test that writes outside it.

- [ ] **Step 9: Commit**

```bash
git add crates/aivyx-core/src/tools/shell.rs crates/aivyx-core/Cargo.toml
git commit -m "Confine shell.exec with aivyx-confine (on by default)"
```

---

## Task 4: `git.rs` confinement

**Files:**
- Modify: `/home/julian/Projects/Rust/aivyx/crates/aivyx-core/src/tools/git.rs`

**Interfaces:**
- Consumes: `aivyx_core::{ExecutionConfiner, LandlockConfiner}` (Task 1). Note: unlike `shell.exec`, this task constructs `LandlockConfiner` directly (not via `default_confiner`), since each call needs a *specific* per-call `cwd` (the resolved target repo), not the tool's own construction-time default.
- Produces: `GitReadToolConfig::with_require_enforcement(self, require_enforcement: bool) -> Self`, `GitWriteToolConfig::with_require_enforcement(self, require_enforcement: bool) -> Self`, and a `require_enforcement: bool` field on `GitStatusTool`, `GitDiffTool`, and `GitCommitTool`.

This mirrors `GitWriteToolConfig::with_confirm_destructive`'s existing shape for the builder methods. The confiner itself is constructed fresh inside each tool's `execute()`, right after `repo` is resolved, and reused for every `git` spawn within that same call (`GitCommitTool` spawns `git` three times per call — `add`, `commit`, `rev-parse HEAD` — all against the same resolved `repo`, so one confiner per `execute()` call is correct, not one per spawn).

- [ ] **Step 1: Write the failing tests**

Find this file's existing test module and a test that exercises `GitCommitTool::execute` against a real temp repo (search for a test that calls `.execute(...)` on a `GitCommitTool` — read it in full to match its exact fixture-setup style: how it creates a temp `.git` repo, builds the tool via `GitWriteToolConfig`, and constructs input JSON). Add:

```rust
#[tokio::test]
async fn git_commit_denies_a_read_outside_the_repo_via_a_malicious_hook() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    // (Use this file's existing repo-init helper here instead of
    // reimplementing `git init` + identity config -- find it by
    // searching for how the file's other GitCommitTool tests set up
    // their fixture repo.)

    let outside = tempfile::Builder::new().tempdir_in("/var/tmp").unwrap();
    let secret = outside.path().join("secret.txt");
    std::fs::write(&secret, "top secret").unwrap();

    let hook_path = repo.join(".git/hooks/post-commit");
    std::fs::write(
        &hook_path,
        format!("#!/bin/sh\ncat {} > {}/leaked.txt\n", secret.display(), repo.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    std::fs::write(repo.join("a.txt"), "hello").unwrap();

    let tool = GitWriteToolConfig::new(vec![repo.clone()])
        .build()
        .expect("git.commit should build");

    let input = serde_json::json!({
        "repo": repo.display().to_string(),
        "message": "test commit",
        "paths": ["a.txt"],
    });

    let _ = tool.execute(input, &test_tool_context()).await;

    assert!(
        !repo.join("leaked.txt").exists(),
        "the post-commit hook must not be able to read a file outside the repo root"
    );
}
```

**Before running this step**, replace `test_tool_context()` with this file's real `ToolContext` construction helper, and replace the repo-init comment with this file's real fixture-setup code (search for how the file's existing `GitCommitTool` tests create their temp repo and git identity — `git init` alone isn't enough, a commit needs `user.name`/`user.email` configured, and the existing tests already solve this).

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd /home/julian/Projects/Rust/aivyx
cargo test -p aivyx-core git_commit_denies_a_read_outside_the_repo
```

Expected: FAIL — `leaked.txt` exists (the hook successfully read the outside file and wrote it into the repo), since there's no confinement yet.

- [ ] **Step 3: Add `require_enforcement` fields and builder methods**

In `/home/julian/Projects/Rust/aivyx/crates/aivyx-core/src/tools/git.rs`:

Add a `require_enforcement: bool` field to `GitStatusTool`, `GitDiffTool`, and `GitCommitTool`'s struct definitions (each currently has `id`, `repos`, and either `schema` alone or `schema` + `confirm_destructive`).

In `GitReadToolConfig` (used to build the `GitStatusTool`/`GitDiffTool` pair), add a field and builder method mirroring `GitWriteToolConfig::with_confirm_destructive`:

```rust
pub struct GitReadToolConfig {
    repos: Vec<PathBuf>,
    require_enforcement: bool,
}

impl GitReadToolConfig {
    pub fn new(repos: impl IntoIterator<Item = PathBuf>) -> Self {
        GitReadToolConfig {
            repos: repos.into_iter().collect(),
            require_enforcement: true,
        }
    }

    /// See `GitWriteToolConfig::with_require_enforcement` — same flag,
    /// same default, same reasoning.
    pub fn with_require_enforcement(mut self, require_enforcement: bool) -> Self {
        self.require_enforcement = require_enforcement;
        self
    }

    pub fn build(self) -> Result<(GitStatusTool, GitDiffTool), AivyxError> {
        let allow_set: Arc<[PathBuf]> = canonicalize_repo_allow_set(self.repos, "git.read")?.into();
        Ok((
            GitStatusTool {
                id: ToolId::new(),
                repos: Arc::clone(&allow_set),
                schema: status_input_schema(),
                require_enforcement: self.require_enforcement,
            },
            GitDiffTool {
                id: ToolId::new(),
                repos: allow_set,
                schema: diff_input_schema(),
                require_enforcement: self.require_enforcement,
            },
        ))
    }
}
```

(This replaces the existing `GitReadToolConfig` struct definition, `new`, and `build` — the `repos` field/constructor logic is unchanged, only `require_enforcement` and the two new struct-literal fields are added.)

In `GitWriteToolConfig`, add the same field/builder method (mirroring `with_confirm_destructive` exactly) and add `require_enforcement: self.require_enforcement,` to `GitCommitTool`'s struct literal in `build()`:

```rust
pub struct GitWriteToolConfig {
    repos: Vec<PathBuf>,
    confirm_destructive: bool,
    require_enforcement: bool,
}

impl GitWriteToolConfig {
    pub fn new(repos: impl IntoIterator<Item = PathBuf>) -> Self {
        GitWriteToolConfig {
            repos: repos.into_iter().collect(),
            confirm_destructive: false,
            require_enforcement: true,
        }
    }

    pub fn with_confirm_destructive(mut self, confirm: bool) -> Self {
        self.confirm_destructive = confirm;
        self
    }

    /// Whether Landlock confinement (via `aivyx-confine`) must succeed
    /// for `git.commit` to run at all. `true` (fail-closed) by default.
    pub fn with_require_enforcement(mut self, require_enforcement: bool) -> Self {
        self.require_enforcement = require_enforcement;
        self
    }

    pub fn build(self) -> Result<GitCommitTool, AivyxError> {
        let allow_set: Arc<[PathBuf]> =
            canonicalize_repo_allow_set(self.repos, "git.write")?.into();
        Ok(GitCommitTool {
            id: ToolId::new(),
            repos: allow_set,
            confirm_destructive: self.confirm_destructive,
            schema: commit_input_schema(),
            require_enforcement: self.require_enforcement,
        })
    }
}
```

- [ ] **Step 4: Confine `GitStatusTool::execute`**

Find `GitStatusTool::execute` — currently builds and spawns inline:

```rust
        let output = match tokio::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .arg("status")
            .arg("--porcelain")
            .arg("--untracked-files=all")
            .output()
            .await
        {
```

Replace with (breaking the chain so the confiner can wrap the command before it's run):

```rust
        let mut command = tokio::process::Command::new("git");
        command
            .arg("-C")
            .arg(&repo)
            .arg("status")
            .arg("--porcelain")
            .arg("--untracked-files=all");
        let confiner = LandlockConfiner::new(&repo, &[], &[], self.require_enforcement);
        let command = confiner.confine(command);
        let output = match command.output().await {
```

- [ ] **Step 5: Confine `GitDiffTool::execute`**

Find `GitDiffTool::execute`'s `let mut cmd = tokio::process::Command::new("git"); cmd.arg("-C").arg(&repo).arg("diff"); ... let output = match cmd.output().await {`. Insert, right before `let output = match cmd.output().await {`:

```rust
        let confiner = LandlockConfiner::new(&repo, &[], &[], self.require_enforcement);
        let cmd = confiner.confine(cmd);
        let output = match cmd.output().await {
```

- [ ] **Step 6: Confine `GitCommitTool::execute`**

Find `GitCommitTool::execute`. Right after `let repo = match resolve_repo(&input, &self.repos) { ... };` resolves successfully (before `message` is parsed), add:

```rust
        let confiner = LandlockConfiner::new(&repo, &[], &[], self.require_enforcement);
```

Then apply it at each of the three spawn sites, in order:

1. `let mut add_cmd = tokio::process::Command::new("git"); add_cmd.arg("-C").arg(&repo).arg("add").arg("--"); for p in &paths { add_cmd.arg(p); } match add_cmd.output().await {` — insert right before `match add_cmd.output().await {`:

```rust
        let add_cmd = confiner.confine(add_cmd);
        match add_cmd.output().await {
```

2. The chained `commit_out` build:

```rust
        let commit_out = match tokio::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .arg("commit")
            .arg("-m")
            .arg(message)
            .output()
            .await
        {
```

Replace with:

```rust
        let mut commit_cmd = tokio::process::Command::new("git");
        commit_cmd.arg("-C").arg(&repo).arg("commit").arg("-m").arg(message);
        let commit_cmd = confiner.confine(commit_cmd);
        let commit_out = match commit_cmd.output().await {
```

3. The chained `commit_hash` build:

```rust
        let commit_hash = match tokio::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .arg("rev-parse")
            .arg("HEAD")
            .output()
            .await
        {
```

Replace with:

```rust
        let mut rev_parse_cmd = tokio::process::Command::new("git");
        rev_parse_cmd.arg("-C").arg(&repo).arg("rev-parse").arg("HEAD");
        let rev_parse_cmd = confiner.confine(rev_parse_cmd);
        let commit_hash = match rev_parse_cmd.output().await {
```

- [ ] **Step 7: Run the test to verify it passes**

```bash
cargo test -p aivyx-core git_commit_denies_a_read_outside_the_repo
```

Expected: PASS.

- [ ] **Step 8: Run this file's full test suite**

```bash
cargo test -p aivyx-core --lib tools::git
```

Expected: every pre-existing test still passes.

- [ ] **Step 9: Run clippy**

```bash
cargo clippy -p aivyx-core --all-targets
```

Expected: clean. Watch specifically for an unused-`mut` warning on any of the `let mut X = tokio::process::Command::new("git");` bindings this step introduced — `confiner.confine(command)` takes the command by value and returns a new binding, so the *original* `mut` binding is only mutated by the `.arg(...)` calls before that point, which is correct; if clippy flags something here, re-check the exact variable being reassigned.

- [ ] **Step 10: Commit**

```bash
git add crates/aivyx-core/src/tools/git.rs
git commit -m "Confine git.rs's three tools with aivyx-confine (per-call, on by default)"
```

---

## Task 5: Wire the binary's construction sites

**Files:**
- Modify: `/home/julian/Projects/Rust/aivyx/crates/aivyx-cli/src/bin/aivyx.rs`

**Interfaces:**
- Consumes: `aivyx_core::default_confiner` (Task 1), `ShellExecTool::with_confiner` (Task 3), `GitReadToolConfig::with_require_enforcement`/`GitWriteToolConfig::with_require_enforcement` (Task 4), `AivyxConfig.require_enforcement: Sourced<bool>` (Task 2).

- [ ] **Step 1: Wire `shell.exec`**

Find `build_shell_exec_for_channel` (search for `fn build_shell_exec_for_channel`). It currently takes `sensitive: std::sync::Arc<aivyx_core::sensitive_paths::SensitivePolicy>` as a parameter and calls `.with_sensitive_policy(sensitive)` on the built tool. Add a new parameter, `require_enforcement: bool`, and chain a `.with_confiner(...)` call right after `.with_sensitive_policy(sensitive)`:

```rust
fn build_shell_exec_for_channel(
    channel_kind: ChannelKind,
    fs_root: &std::path::Path,
    sensitive: std::sync::Arc<aivyx_core::sensitive_paths::SensitivePolicy>,
    require_enforcement: bool,
) -> Result<GatedToolRegistration, String> {
    match channel_kind {
        ChannelKind::Local | ChannelKind::Voice => {
            let shell = ShellExecToolConfig::new(fs_root.to_path_buf())
                .build()
                .map_err(|e| format!("failed to build shell.exec tool: {e}"))?
                .with_sensitive_policy(sensitive)
                .with_confiner(aivyx_core::default_confiner(
                    fs_root,
                    &[],
                    &[],
                    require_enforcement,
                ));
```

(Everything after this in the function — the `canonical_cwd_root`/`scope` construction and the rest of the `match` arms — is unchanged; only the function signature and the builder chain shown above change.)

Find every call site of `build_shell_exec_for_channel(...)` (there should be exactly one production call plus the `channel_kind_telegram_has_no_shell_exec` test mentioned in this function's own doc comment) and add the new `require_enforcement` argument. At the production call site, read the surrounding function to find the already-in-scope config variable (it's whatever local binding holds the loaded `AivyxConfig` — look for how `confirm_destructive` or another `Sourced<bool>` field is already being read nearby, e.g. `config.confirm_destructive.value`) and pass `config.require_enforcement.value`. At the test call site, pass a literal `true` or `false` depending on what that specific test needs to prove (read the test first — if it's not about confinement at all, `true` is the safe default matching production).

- [ ] **Step 2: Wire `git.rs`**

Find the git tool construction site (search for `aivyx_core::GitReadToolConfig::new(repos.clone())` and `aivyx_core::GitWriteToolConfig::new(repos)`, around line 6798-6820). Add `.with_require_enforcement(...)` to both builder chains, using the same config value found in Step 1:

```rust
        let (git_status, git_diff) =
            aivyx_core::GitReadToolConfig::new(repos.clone())
                .with_require_enforcement(config.require_enforcement.value)
                .build()
                .map_err(|e| {
                    format!("failed to build git.read tool pair: {e}")
                })?;
```

```rust
        let git_commit = aivyx_core::GitWriteToolConfig::new(repos)
            .with_confirm_destructive(confirm_destructive)
            .with_require_enforcement(config.require_enforcement.value)
            .build()
            .map_err(|e| format!("failed to build git.commit tool: {e}"))?;
```

(Use whatever the real in-scope config variable is named — matching `confirm_destructive`'s own already-working reference right next to this exact code, not necessarily literally `config.require_enforcement.value` if the surrounding function destructures fields differently. The point: pass the same config value Step 1 used, following the same lookup pattern `confirm_destructive` already demonstrates one line above.)

- [ ] **Step 3: Verify the whole workspace builds**

```bash
cd /home/julian/Projects/Rust/aivyx
cargo build --workspace
```

Expected: clean success.

- [ ] **Step 4: Run the full test suite**

```bash
cargo test --workspace
```

Expected: every test passes, including Tasks 2-4's new tests and every pre-existing test in the workspace.

- [ ] **Step 5: Run clippy**

```bash
cargo clippy --workspace --all-targets
```

Expected: clean.

- [ ] **Step 6: Update `docs/THREAT_MODEL.md`**

Find the line stating *"Container-level sandboxing of tools: No... forward work."* (search for `Container-level sandboxing`). Replace it with an accurate statement that `shell.exec` and `git.rs` are now confined via Landlock + seccomp-bpf (`aivyx-confine`, on by default, `[confine] require_enforcement` configurable), while `[[tool_process]]`/MCP external tools remain on the separate, pre-existing `bwrap`/`firejail`/`docker` mechanism (`aivyx-tool/src/sandbox.rs`).

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-cli/src/bin/aivyx.rs docs/THREAT_MODEL.md
git commit -m "Wire aivyx-confine into shell.exec and git.rs's binary construction sites"
```

---

## Self-review notes

**Spec coverage:** the design doc's five sections map onto tasks directly — "Config" → Task 2; "shell.exec integration" → Task 3; "git.rs integration" → Task 4; "Testing" → each task's own test (the hook-escape scenario specifically named in the design's rationale is Task 4's test, not a generic "confinement works" check); "Explicitly out of scope" (`deny_paths`/`extra_read_paths`, `aivyx-tool`'s separate mechanism, no policy change to `aivyx-confine` itself) → correctly absent from every task; Task 5's `docs/THREAT_MODEL.md` update closes the loop the design doc's own "Problem" section opened by quoting that file.

**Placeholder scan:** Tasks 2, 3, and 4's tests each contain one explicit "find the real name" instruction (the config-loading test helper, the `ToolContext` constructor, the repo-init fixture helper) rather than a fabricated one — this is a deliberate, bounded exception to "no placeholders," the same pattern the `aivyx-confine` project's own Task 3 used ("copy verbatim from the known-good source") for code this plan's author could not fully verify byte-for-byte in a 7000+-line file without reading all of it. Every other step has complete, real code.

**Type consistency:** `ExecutionConfiner`/`LandlockConfiner`/`default_confiner` (Task 1's re-export) are used with identical signatures in Tasks 3-5 — checked against `aivyx-confine`'s own verified public API (Global Constraints). `with_confiner`/`with_require_enforcement` builder method names and the `require_enforcement: bool` field are used identically across Tasks 3, 4, and 5. `AivyxConfig.require_enforcement: Sourced<bool>` (Task 2) is the exact type Task 5 reads (`.value`) at both call sites.

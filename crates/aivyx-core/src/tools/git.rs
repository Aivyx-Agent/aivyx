//! Phase 109 — `git.status` and `git.diff` substrate tools.
//!
//! Both tools share a single `git.read` scope base qualified
//! by repo path. The shared-scope design is documented in
//! Amendment A12 (`docs/amendments/2026-05-28-substrate-tool-count-thirteen.md`):
//! `git.status` and `git.diff` are both read-only inspection
//! of the same repo, so the natural capability grant is "this
//! role can read this repo," not "this role can run `git
//! status` but not `git diff`."
//!
//! ## Repo-path validation
//!
//! Both tools take a `repo` field on their input that names
//! one of the operator's configured allowed repos. The
//! validator canonicalizes the input against the allow-set;
//! any repo path not on the list refuses to dispatch.
//!
//! Why an explicit allow-list rather than free-path
//! traversal: `git` happily walks parent `.git/` directories
//! (`-C /tmp` resolves to whatever git repo `/tmp/.git` points
//! to, or panics if there isn't one). Without the allow-list,
//! an agent with `git.read:**` could inspect any git repo on
//! the operator's filesystem. The allow-list pattern matches
//! how `fs.read` requires a sandbox root: the operator gates
//! which paths are inspectable, the tool gates which
//! operations.
//!
//! ## Shell-out, not git2
//!
//! Phase 109 ships these by shelling out to the system `git`
//! binary rather than depending on the `git2` Rust crate.
//! The trade-off: `git2` is faster, type-safer, and version-
//! independent — but `git` is universally available on any
//! developer's machine, has zero binary-size cost, and means
//! Phase 109 ships with zero new workspace deps. A future
//! phase that wants per-call performance or richer parsing
//! can swap in `git2` behind the same tool surface without
//! changing the `Tool` impl.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{
    AivyxError, CapabilitySet, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};
use aivyx_capability::Scope;

// ---------------------------------------------------------------------------
// Config + shared builder
// ---------------------------------------------------------------------------

/// Construction inputs for [`GitStatusTool`] / [`GitDiffTool`].
/// The two tools share configuration because they share the
/// `git.read` scope base — same allow-set of repo paths gates
/// both.
pub struct GitReadToolConfig {
    repos: Vec<PathBuf>,
}

impl GitReadToolConfig {
    /// Construct from an operator-supplied list of repo paths.
    /// Each path will be canonicalized at `build` time and
    /// any path that does not resolve to a directory containing
    /// a `.git` entry will fail startup with `AivyxError::Config`.
    pub fn new(repos: impl IntoIterator<Item = PathBuf>) -> Self {
        GitReadToolConfig {
            repos: repos.into_iter().collect(),
        }
    }

    /// Canonicalize the allow-set and return a ready-to-register
    /// `(GitStatusTool, GitDiffTool)` pair. The pair shares the
    /// canonicalized allow-set behind an `Arc<[PathBuf]>` so the
    /// two tools register independently in the dispatch registry
    /// while still gating against the same allow-set.
    pub fn build(self) -> Result<(GitStatusTool, GitDiffTool), AivyxError> {
        let mut canonical = Vec::with_capacity(self.repos.len());
        for repo in self.repos {
            let abs = std::fs::canonicalize(&repo).map_err(|e| {
                AivyxError::Config(format!(
                    "git.read allow-set entry {repo:?} cannot be canonicalized: {e}"
                ))
            })?;
            if !abs.is_dir() {
                return Err(AivyxError::Config(format!(
                    "git.read allow-set entry {abs:?} is not a directory"
                )));
            }
            let dot_git = abs.join(".git");
            if !dot_git.exists() {
                return Err(AivyxError::Config(format!(
                    "git.read allow-set entry {abs:?} is not a git repo \
                     (no .git/ entry)"
                )));
            }
            canonical.push(abs);
        }
        let allow_set: Arc<[PathBuf]> = canonical.into();
        Ok((
            GitStatusTool {
                id: ToolId::new(),
                repos: Arc::clone(&allow_set),
                schema: status_input_schema(),
            },
            GitDiffTool {
                id: ToolId::new(),
                repos: allow_set,
                schema: diff_input_schema(),
            },
        ))
    }
}

// ---------------------------------------------------------------------------
// Tool: git.status
// ---------------------------------------------------------------------------

/// `git.status` — runs `git status --porcelain --untracked-files=all`
/// against an operator-allowed repo path and returns parsed
/// entries as JSON.
#[derive(Debug)]
pub struct GitStatusTool {
    id: ToolId,
    repos: Arc<[PathBuf]>,
    schema: Value,
}

impl GitStatusTool {
    /// The canonical allow-set this tool gates against.
    pub fn repos(&self) -> &[PathBuf] {
        &self.repos
    }
}

#[async_trait]
impl Tool for GitStatusTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "git.status"
    }

    fn description(&self) -> &str {
        "Run `git status --porcelain` against a configured repo \
         path. Input is a JSON object with a `repo` field naming \
         one of the operator's allowed git repos (canonical path \
         match). Returns a JSON object with an `entries` array — \
         each entry has `status_code` (the two-char porcelain \
         code) and `path`."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, input: &Value) -> Scope {
        match resolve_repo(input, &self.repos) {
            Some(abs) => Scope::parse(&format!("git.read:{}", abs.display()))
                .unwrap_or_else(deny_scope),
            None => deny_scope(),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let repo = match resolve_repo(&input, &self.repos) {
            Some(p) => p,
            None => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: "git.status: `repo` field missing or not in allow-set"
                        .to_string(),
                });
            }
        };

        let output = match tokio::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .arg("status")
            .arg("--porcelain")
            .arg("--untracked-files=all")
            .output()
            .await
        {
            Ok(o) => o,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("git.status: spawn failed: {e}"),
                });
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!(
                    "git.status: exit code {:?}: {}",
                    output.status.code(),
                    stderr.trim()
                ),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let entries = parse_porcelain(&stdout);

        ToolOutcome::Completed {
            output: json!({ "repo": repo.display().to_string(), "entries": entries }),
            verified: Verification::Verified,
        }
    }
}

// ---------------------------------------------------------------------------
// Tool: git.diff
// ---------------------------------------------------------------------------

/// `git.diff` — runs `git diff [--cached] [<path>]` against
/// an operator-allowed repo path and returns the unified-diff
/// output as a string.
#[derive(Debug)]
pub struct GitDiffTool {
    id: ToolId,
    repos: Arc<[PathBuf]>,
    schema: Value,
}

impl GitDiffTool {
    /// The canonical allow-set this tool gates against.
    pub fn repos(&self) -> &[PathBuf] {
        &self.repos
    }
}

#[async_trait]
impl Tool for GitDiffTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "git.diff"
    }

    fn description(&self) -> &str {
        "Run `git diff` against a configured repo path. Input \
         is a JSON object with a `repo` field naming one of the \
         operator's allowed git repos (canonical path match), \
         an optional `cached` boolean (`true` for the staged \
         diff, default false for the working-tree diff), and an \
         optional `path` string scoping the diff to a single \
         file or directory within the repo. Returns a JSON \
         object with a `diff` field carrying the unified-diff \
         output as a string."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, input: &Value) -> Scope {
        match resolve_repo(input, &self.repos) {
            Some(abs) => Scope::parse(&format!("git.read:{}", abs.display()))
                .unwrap_or_else(deny_scope),
            None => deny_scope(),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let repo = match resolve_repo(&input, &self.repos) {
            Some(p) => p,
            None => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: "git.diff: `repo` field missing or not in allow-set"
                        .to_string(),
                });
            }
        };

        let cached = input.get("cached").and_then(|v| v.as_bool()).unwrap_or(false);
        let path = input.get("path").and_then(|v| v.as_str());

        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("-C").arg(&repo).arg("diff");
        if cached {
            cmd.arg("--cached");
        }
        if let Some(p) = path {
            // Refuse path traversal — `git diff` would happily
            // accept `../sibling/file.txt` and walk outside
            // the configured repo. We require the path to be
            // a plain relative path.
            if p.starts_with('/') || p.contains("..") {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!(
                        "git.diff: `path` must be repo-relative and \
                         must not contain `..`; got {p:?}"
                    ),
                });
            }
            cmd.arg("--").arg(p);
        }

        let output = match cmd.output().await {
            Ok(o) => o,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("git.diff: spawn failed: {e}"),
                });
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!(
                    "git.diff: exit code {:?}: {}",
                    output.status.code(),
                    stderr.trim()
                ),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        ToolOutcome::Completed {
            output: json!({
                "repo": repo.display().to_string(),
                "cached": cached,
                "path": path,
                "diff": stdout,
            }),
            verified: Verification::Verified,
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Resolve the `repo` input field against the allow-set. Returns
/// the canonical path if the input names an allowed repo;
/// returns `None` otherwise. Canonicalization happens at build
/// time (in `GitReadToolConfig::build`), so this is a pure
/// string-compare against pre-canonicalized paths plus a
/// per-call `canonicalize` on the input to handle equivalent
/// path forms (`./foo` vs. `foo`, symlinks, etc.).
fn resolve_repo(input: &Value, repos: &[PathBuf]) -> Option<PathBuf> {
    let path_str = input.get("repo").and_then(|v| v.as_str())?;
    let input_path = Path::new(path_str);
    let canonical = std::fs::canonicalize(input_path).ok()?;
    if repos.iter().any(|allowed| allowed.as_path() == canonical.as_path()) {
        Some(canonical)
    } else {
        None
    }
}

/// Parse `git status --porcelain` output into a `Vec` of JSON
/// objects with `status_code` and `path` fields. The porcelain
/// format is two characters of status code, a space, then the
/// path. We tolerate the rename arrow notation (`R  old -> new`)
/// by carrying both halves in `path` as-is.
fn parse_porcelain(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let status_code = &line[..2];
            let path = line[3..].to_string();
            Some(json!({ "status_code": status_code, "path": path }))
        })
        .collect()
}

/// Unsatisfiable scope used when the input is malformed.
/// Mirrors `fs::delete_deny_scope` shape — pin one scope the
/// agent definitely does not hold so the deny path is
/// deterministic.
fn deny_scope() -> Scope {
    Scope::parse("git.read:/__aivyx_unresolvable__").expect(
        "git.read:/__aivyx_unresolvable__ must parse — `git.read` is in KNOWN_BASES",
    )
}

fn status_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "repo": {
                "type": "string",
                "description": "Path to a configured allowed git repo."
            }
        },
        "required": ["repo"]
    })
}

fn diff_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "repo": {
                "type": "string",
                "description": "Path to a configured allowed git repo."
            },
            "cached": {
                "type": "boolean",
                "description": "When true, show the staged diff (`git diff --cached`). \
                                Default false."
            },
            "path": {
                "type": "string",
                "description": "Optional repo-relative path to scope the diff to one \
                                file or directory. Must not start with `/` and must \
                                not contain `..`."
            }
        },
        "required": ["repo"]
    })
}

// Suppress unused warning on CapabilitySet — it's pulled in by
// the `Tool` trait bound chain but not referenced directly here
// since `required_scope` returns a single `Scope` rather than a
// `CapabilitySet`. The use lives at the top of the file to match
// the convention sibling tool modules follow.
#[allow(dead_code)]
fn _unused_capability_set_ref(_cs: &CapabilitySet) {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod git_tests {
    use super::*;

    #[test]
    fn parse_porcelain_handles_typical_status_lines() {
        let input = " M src/foo.rs\n?? new.txt\nA  staged.rs\n";
        let entries = parse_porcelain(input);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0]["status_code"], " M");
        assert_eq!(entries[0]["path"], "src/foo.rs");
        assert_eq!(entries[1]["status_code"], "??");
        assert_eq!(entries[1]["path"], "new.txt");
        assert_eq!(entries[2]["status_code"], "A ");
        assert_eq!(entries[2]["path"], "staged.rs");
    }

    #[test]
    fn parse_porcelain_empty_stdout_returns_empty_vec() {
        assert!(parse_porcelain("").is_empty());
    }

    #[test]
    fn parse_porcelain_handles_rename_arrow_form() {
        let input = "R  old.rs -> new.rs\n";
        let entries = parse_porcelain(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["status_code"], "R ");
        assert_eq!(entries[0]["path"], "old.rs -> new.rs");
    }

    #[test]
    fn parse_porcelain_skips_too_short_lines() {
        // Defense: a line that is only 1-3 chars can't carry a
        // status code + space + path. Should be skipped, not
        // panic.
        let input = "X\n  \nA  ok.rs\n";
        let entries = parse_porcelain(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["path"], "ok.rs");
    }

    #[test]
    fn deny_scope_parses_and_is_unsatisfiable() {
        let scope = deny_scope();
        assert_eq!(scope.base(), "git.read");
        // The qualifier is the unresolvable sentinel; a role with
        // any normal `git.read:<path>` does not grant this scope.
        let real = Scope::parse("git.read:/home/me/projects/aivyx").unwrap();
        assert!(!scope.is_granted_by(&real));
    }

    #[test]
    fn resolve_repo_returns_none_when_input_missing() {
        let allow: Vec<PathBuf> = vec![PathBuf::from("/tmp")];
        let input = json!({});
        assert!(resolve_repo(&input, &allow).is_none());
    }

    #[test]
    fn resolve_repo_returns_none_when_input_not_in_allow_set() {
        // Use a real tmp path so canonicalize succeeds but the
        // path isn't in the allow-set.
        let tmp = std::env::temp_dir();
        let allow: Vec<PathBuf> = vec![PathBuf::from("/definitely-not-a-real-path-aivyx")];
        let input = json!({ "repo": tmp.display().to_string() });
        assert!(resolve_repo(&input, &allow).is_none());
    }

    #[test]
    fn status_input_schema_requires_repo() {
        let s = status_input_schema();
        assert_eq!(s["required"][0], "repo");
    }

    #[test]
    fn diff_input_schema_requires_repo_and_allows_optional_cached_and_path() {
        let s = diff_input_schema();
        assert_eq!(s["required"][0], "repo");
        assert!(s["properties"].as_object().unwrap().contains_key("cached"));
        assert!(s["properties"].as_object().unwrap().contains_key("path"));
    }

    #[test]
    fn config_build_rejects_nonexistent_path() {
        let cfg = GitReadToolConfig::new(vec![PathBuf::from(
            "/definitely-not-a-real-path-aivyx-git",
        )]);
        let err = cfg.build().expect_err("nonexistent path must error");
        assert!(matches!(err, AivyxError::Config(_)));
    }

    #[test]
    fn config_build_rejects_path_that_is_not_a_git_repo() {
        // tmpdir is a directory but isn't a git repo.
        let tmp = std::env::temp_dir();
        let cfg = GitReadToolConfig::new(vec![tmp]);
        let err = cfg.build().expect_err("non-repo dir must error");
        match err {
            AivyxError::Config(msg) => {
                assert!(msg.contains("not a git repo"), "got: {msg}");
            }
            other => panic!("expected Config, got {other:?}"),
        }
    }

    // Note: a build()-success test requires a real git repo on
    // disk. The workspace's own root (`/home/julian/Projects/Rust/aivyx`)
    // is a git repo, but hard-coding that path would make the
    // test fragile across operator environments. The integration
    // test in Task 5 covers the success path via a real tmpdir
    // `git init` setup.
}

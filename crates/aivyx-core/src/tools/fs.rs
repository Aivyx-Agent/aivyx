//! `FsReadTool` — the first concrete filesystem tool.
//!
//! ## What it does
//!
//! Reads a UTF-8 (or lossily-decoded binary) file from under an
//! agent-configured sandbox root and returns its contents as a
//! `ToolOutcome::Completed` JSON payload. The agent must hold a scope
//! like `fs.read:/sandbox/**` in its effective capability set; the loop's
//! scope gate at `agent.rs` ensures only in-scope reads reach
//! [`FsReadTool::execute`].
//!
//! ## Defense in depth
//!
//! Path traversal is the attack this tool defends against, and it
//! defends in **two independent layers**:
//!
//! 1. **Lexical layer (`required_scope`).** Purely string-based
//!    normalization of the input path against the pre-canonicalized
//!    sandbox root. Handles `.` and `..` without touching the
//!    filesystem. Produces the `Scope` the loop's scope gate checks.
//!    An attacker input like `"../../etc/passwd"` lexically resolves
//!    to a scope *outside* the sandbox prefix and the scope gate
//!    denies the call before [`execute`] runs.
//!
//! 2. **Canonical layer (`execute`).** Right before opening the file,
//!    `execute` calls `std::fs::canonicalize`, which resolves every
//!    symlink in the path. If the canonicalized path no longer starts
//!    with the canonicalized sandbox root, the tool returns
//!    `ToolOutcome::Failed(AivyxError::Tool { detail: "…escapes
//!    sandbox…" })`. This catches symlink-based traversal that the
//!    lexical layer cannot see: an attacker who creates a symlink
//!    `/sandbox/escape → /etc/shadow` and then asks the agent to read
//!    `"escape"` passes the lexical check (the input path is inside
//!    the sandbox) and is stopped by the canonical check.
//!
//! The two layers catch two independent attack classes. Removing
//! either one leaves a hole. Documented here so a future contributor
//! who thinks "why are we canonicalizing twice?" gets the answer
//! in-place.
//!
//! ## Purity of `required_scope`
//!
//! `Tool::required_scope` is documented as pure ("must not perform
//! side effects"). The Phase 4 task 2 decision is to interpret that
//! as **logically pure**: deterministic, referentially transparent,
//! no observable side effects. Reading the filesystem to canonicalize
//! *could* be argued as a side effect (it loads pages into the kernel
//! cache), but `required_scope` is called at most once per tool call
//! and is idempotent, so the pragmatic cost is zero.
//!
//! The cleaner answer is that this tool **does not call the filesystem
//! from `required_scope` at all**. The sandbox root is canonicalized
//! **once at construction time** by [`FsReadToolConfig::build`], and
//! `required_scope` performs only lexical path work against the
//! pre-canonicalized root. Tests can construct an `FsReadTool` against
//! any existing directory and then call `required_scope` with any
//! input, no further I/O required.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;

use crate::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

/// Default cap on the number of bytes read from a single file. Files
/// larger than this return their first `MAX_READ_BYTES` bytes along
/// with `"truncated": true`. 256 KiB is big enough for most config
/// files, source files, and notes; small enough that an agent cannot
/// drain a multi-gigabyte log into a single LLM turn by accident.
/// Const, not config knob — same philosophy as `TURN_TIMEOUT`.
pub const MAX_READ_BYTES: usize = 256 * 1024;

/// Construction inputs for [`FsReadTool`]. Split from the tool itself
/// so that the fallible `canonicalize()` call on the sandbox root
/// happens at `build()` time and the resulting `FsReadTool` is
/// infallible to construct (holding a pre-canonicalized absolute path).
pub struct FsReadToolConfig {
    sandbox_root: PathBuf,
}

impl FsReadToolConfig {
    pub fn new(sandbox_root: impl Into<PathBuf>) -> Self {
        FsReadToolConfig {
            sandbox_root: sandbox_root.into(),
        }
    }

    /// Canonicalize the sandbox root and return a ready-to-register
    /// [`FsReadTool`]. Fails if the root doesn't exist or isn't a
    /// directory — those are configuration errors the caller needs to
    /// see at startup, not at tool-call time.
    pub fn build(self) -> Result<FsReadTool, AivyxError> {
        let canonical = std::fs::canonicalize(&self.sandbox_root).map_err(|e| {
            AivyxError::Config(format!(
                "fs.read sandbox root {:?} cannot be canonicalized: {e}",
                self.sandbox_root
            ))
        })?;
        if !canonical.is_dir() {
            return Err(AivyxError::Config(format!(
                "fs.read sandbox root {canonical:?} is not a directory"
            )));
        }
        Ok(FsReadTool {
            id: ToolId::new(),
            sandbox_root: Arc::from(canonical),
            schema: input_schema_value(),
        })
    }
}

/// Reference filesystem read tool. Agents holding
/// `fs.read:<sandbox_root>/**` can read any file under the sandbox,
/// regardless of symlink topology, up to [`MAX_READ_BYTES`].
#[derive(Debug)]
pub struct FsReadTool {
    id: ToolId,
    /// Pre-canonicalized absolute path. `Arc<Path>` because the tool
    /// is registered in an `Arc<ToolRegistry>` and cloned across
    /// concurrent turns; shared ownership of an immutable path is
    /// cheaper than cloning a `PathBuf` per call.
    sandbox_root: Arc<Path>,
    schema: Value,
}

impl FsReadTool {
    /// Expose the canonicalized sandbox root (primarily for tests and
    /// for the binary's startup banner — the registered agent needs
    /// to know which directory it was actually granted access to).
    pub fn sandbox_root(&self) -> &Path {
        &self.sandbox_root
    }

    /// Lexically resolve `input_path` (which may be relative) against
    /// the sandbox root, returning an absolute path with all `.` and
    /// `..` segments collapsed. **Does not touch the filesystem** —
    /// symlinks are not resolved here.
    ///
    /// Returns `None` if the lexical resolution escapes the sandbox
    /// root (e.g., more `..` segments than there are components below
    /// the root). The caller uses this signal to produce a deny-by-
    /// construction scope.
    fn lexical_resolve(&self, input_path: &Path) -> Option<PathBuf> {
        // Join semantics: if `input_path` is absolute, `PathBuf::push`
        // *replaces* the current path. That's the right thing for an
        // agent that tries to pass an absolute path: it lands
        // wherever the absolute path points, and the post-collapse
        // prefix check will reject it if it's outside the sandbox.
        let mut joined = PathBuf::from(&*self.sandbox_root);
        joined.push(input_path);

        // Collapse `.` and `..`. We iterate components and maintain a
        // stack: `CurDir` is skipped, `ParentDir` pops the stack but
        // only if the stack has more components than the sandbox
        // root's component count (so `../` out of the sandbox root
        // itself returns None).
        let root_components: Vec<Component<'_>> =
            self.sandbox_root.components().collect();
        let mut stack: Vec<Component<'_>> = Vec::with_capacity(16);
        for comp in joined.components() {
            match comp {
                Component::CurDir => {}
                Component::ParentDir => {
                    // Pop, unless popping would take us out of the
                    // sandbox root.
                    if stack.len() <= root_components.len() {
                        return None;
                    }
                    stack.pop();
                }
                other => stack.push(other),
            }
        }

        // Verify the resolved stack still starts with the sandbox
        // root. A case this catches: a Unix absolute input like
        // `/etc/passwd` replaces the prefix entirely via `push()`,
        // so `stack` ends up as `[/, etc, passwd]` with no sandbox
        // prefix, and the check below fires.
        for (i, root_c) in root_components.iter().enumerate() {
            if stack.get(i) != Some(root_c) {
                return None;
            }
        }

        Some(stack.into_iter().collect())
    }
}

#[async_trait]
impl Tool for FsReadTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "fs.read"
    }

    fn description(&self) -> &str {
        "Read a UTF-8 (or binary) file from under the agent's sandbox root. \
         Input is a JSON object with a `path` field (relative paths are \
         resolved against the sandbox root; absolute paths must already be \
         under the sandbox root or the call is denied). Files larger than \
         256 KiB are truncated."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, input: &Value) -> Scope {
        let Some(path_str) = input.get("path").and_then(|v| v.as_str()) else {
            // Missing or non-string `path`. Produce a scope no agent
            // holds so the loop's scope gate denies the call and the
            // planner sees `Denied`. We deliberately do NOT return a
            // `Failed` here because `required_scope` returns `Scope`,
            // not `Result`, and a Denied outcome is the closest
            // equivalent to "input is malformed, don't run."
            return deny_scope();
        };

        match self.lexical_resolve(Path::new(path_str)) {
            Some(abs) => Scope::parse(&format!("fs.read:{}", abs.display()))
                .unwrap_or_else(deny_scope),
            None => deny_scope(),
        }
    }

    async fn execute(
        &self,
        input: Value,
        _ctx: &ToolContext<'_>,
    ) -> ToolOutcome {
        // ---- Re-parse and re-resolve the input --------------------
        //
        // The loop has already verified the *derived* scope is in the
        // agent's set, but that verification used the lexical resolve.
        // `execute` must do the canonical resolve as the second fence
        // (see the module-level "Defense in depth" note).
        let path_str = match input.get("path").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: "input must have a string `path` field".to_string(),
                })
            }
        };

        let lexical_abs = match self.lexical_resolve(Path::new(path_str)) {
            Some(p) => p,
            None => {
                // Should never reach here — the scope gate would have
                // denied a lexical-escape input before execute was
                // called. Treat as an internal invariant violation so
                // a bug in the gate shows up loudly in audit.
                return ToolOutcome::Failed(AivyxError::Internal(format!(
                    "fs.read: lexical resolve escaped sandbox after scope gate \
                     admitted the call (path={path_str:?})"
                )));
            }
        };

        // ---- Canonical fence --------------------------------------
        //
        // Resolve every symlink. If the canonicalized path no longer
        // lives under the canonicalized sandbox root, the call is
        // refused. This is the TOCTOU-resistant check the lexical
        // resolve cannot perform.
        let canonical = match std::fs::canonicalize(&lexical_abs) {
            Ok(p) => p,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("cannot canonicalize {lexical_abs:?}: {e}"),
                });
            }
        };
        if !canonical.starts_with(&*self.sandbox_root) {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!(
                    "path {canonical:?} escapes sandbox root {:?} after \
                     symlink resolution",
                    self.sandbox_root
                ),
            });
        }

        // ---- Read --------------------------------------------------
        //
        // Read up to MAX_READ_BYTES + 1 so we can tell "exactly at cap"
        // apart from "over cap" in one syscall.
        use std::io::Read;
        let mut file = match std::fs::File::open(&canonical) {
            Ok(f) => f,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("cannot open {canonical:?}: {e}"),
                });
            }
        };

        let mut buf = Vec::with_capacity(8 * 1024);
        let mut probe = [0u8; 8 * 1024];
        let mut total_read = 0usize;
        loop {
            match file.read(&mut probe) {
                Ok(0) => break,
                Ok(n) => {
                    total_read += n;
                    if buf.len() + n > MAX_READ_BYTES {
                        let room = MAX_READ_BYTES.saturating_sub(buf.len());
                        buf.extend_from_slice(&probe[..room]);
                        break;
                    }
                    buf.extend_from_slice(&probe[..n]);
                    if total_read >= MAX_READ_BYTES {
                        break;
                    }
                }
                Err(e) => {
                    return ToolOutcome::Failed(AivyxError::Tool {
                        tool: self.id,
                        detail: format!("read error on {canonical:?}: {e}"),
                    });
                }
            }
        }

        // Drain any trailing bytes past the cap so we can set the
        // `truncated` flag honestly even when the file is bigger
        // than what we read.
        let mut truncated = buf.len() >= MAX_READ_BYTES;
        if !truncated {
            // Try one more read — if there's more data, it's truncated.
            let mut extra = [0u8; 1];
            if let Ok(n) = file.read(&mut extra) {
                if n > 0 {
                    truncated = true;
                }
            }
        }

        // ---- Decode ------------------------------------------------
        //
        // Use `from_utf8_lossy` so binary files produce *something*
        // rather than a hard error — the agent can then decide what
        // to do with the result. Flag non-UTF8 data explicitly so the
        // LLM knows its input was lossy.
        let (text, is_binary) = match std::str::from_utf8(&buf) {
            Ok(s) => (s.to_string(), false),
            Err(_) => (String::from_utf8_lossy(&buf).into_owned(), true),
        };

        ToolOutcome::Completed {
            output: json!({
                "path": canonical.display().to_string(),
                "bytes": buf.len(),
                "truncated": truncated,
                "binary": is_binary,
                "text": text,
            }),
            // A file read has no effect to verify — it's a pure query.
            verified: Verification::NotApplicable,
        }
    }
}

/// A scope no real agent should ever hold. Returned by `required_scope`
/// when the input is malformed or escapes the sandbox lexically. The
/// loop's scope gate will deny the call and the planner sees
/// `ToolOutcome::Denied`. The specific string is meaningless — any
/// legal scope with an impossible qualifier works; the value here is
/// chosen for grep-ability in audit trails.
fn deny_scope() -> Scope {
    Scope::parse("fs.read:/aivyx/__deny__/invalid-input")
        .expect("deny scope must parse")
}

fn input_schema_value() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Path to read. Relative paths resolve against \
                               the agent's sandbox root. Absolute paths must \
                               already be under the sandbox root."
            }
        },
        "required": ["path"]
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::io::Write;

    use aivyx_capability::{CapabilitySet, TrustTier};

    /// RAII temp directory — creates `$TMPDIR/aivyx-fs-test-<uuid>/root`
    /// on construction, removes the whole tree on drop. Rolled here to
    /// avoid adding `tempfile` as a dep for ~50 lines of test hygiene.
    struct SandboxDir {
        root: PathBuf,
        _parent: PathBuf,
    }

    impl SandboxDir {
        fn new() -> Self {
            let tmp = std::env::var("TMPDIR")
                .or_else(|_| std::env::var("TEMP"))
                .unwrap_or_else(|_| "/tmp".to_string());
            let parent = PathBuf::from(tmp)
                .join(format!("aivyx-fs-test-{}", uuid::Uuid::new_v4()));
            let root = parent.join("root");
            fs::create_dir_all(&root)
                .expect("test sandbox root must be creatable");
            SandboxDir {
                root,
                _parent: parent,
            }
        }

        fn write_file(&self, rel: &str, contents: &[u8]) -> PathBuf {
            let p = self.root.join(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).expect("mkdir -p parent");
            }
            let mut f = fs::File::create(&p).expect("create test file");
            f.write_all(contents).expect("write test file");
            p
        }
    }

    impl Drop for SandboxDir {
        fn drop(&mut self) {
            // Best-effort cleanup. A failed cleanup in a test harness
            // is a warning, not a fatal error.
            let _ = fs::remove_dir_all(&self._parent);
        }
    }

    fn build_tool(sandbox: &SandboxDir) -> FsReadTool {
        FsReadToolConfig::new(sandbox.root.clone())
            .build()
            .expect("sandbox root must be canonicalizable for tests")
    }

    /// The integration test in task 5 will drive `execute` through the
    /// full turn loop; unit tests need a minimal `ToolContext`. We
    /// don't run the execute path against `ctx` fields other than the
    /// cancellation token, so the cheapest fake is a channel and audit
    /// hook that do nothing. Import them from the core test helpers.
    fn run_execute(tool: &FsReadTool, input: Value) -> ToolOutcome {
        use crate::{
            AgentId, CancellationToken, NullAuditHook, SessionId, TurnId,
        };

        // A minimal `ChannelContext` that ignores every call. This is
        // fine for a unit test that only exercises `execute`'s
        // filesystem and scope behavior — the channel is never touched.
        struct NoopChannel {
            session: SessionId,
            token: CancellationToken,
        }

        #[async_trait]
        impl crate::ChannelContext for NoopChannel {
            fn channel_name(&self) -> &str {
                "test"
            }
            fn platform(&self) -> crate::ChannelPlatform {
                crate::ChannelPlatform::Local
            }
            fn trust_tier(&self) -> aivyx_capability::TrustTier {
                aivyx_capability::TrustTier::Trusted
            }
            fn session_id(&self) -> SessionId {
                self.session
            }
            async fn stream_event(
                &self,
                _event: crate::StreamEvent<'_>,
            ) -> Result<(), crate::ChannelError> {
                Ok(())
            }
            async fn finalize(
                &self,
                _outcome: &crate::TurnOutcome,
            ) -> Result<(), crate::ChannelError> {
                Ok(())
            }
            fn cancellation_token(&self) -> CancellationToken {
                self.token.clone()
            }
        }

        let channel = NoopChannel {
            session: SessionId::new(),
            token: CancellationToken::new(),
        };
        let audit = NullAuditHook;
        let ctx = ToolContext {
            agent_id: AgentId::new(),
            session_id: channel.session,
            turn_id: TurnId::new(),
            channel: &channel,
            audit: &audit,
            cancellation: &channel.token,
        };

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(tool.execute(input, &ctx))
    }

    // ---- Construction / config --------------------------------------

    #[test]
    fn build_fails_if_root_does_not_exist() {
        let err = FsReadToolConfig::new("/definitely/not/a/real/path/aivyx-test")
            .build()
            .expect_err("nonexistent root must fail to build");
        assert!(matches!(err, AivyxError::Config(_)));
    }

    #[test]
    fn build_fails_if_root_is_a_file_not_directory() {
        let sandbox = SandboxDir::new();
        let file = sandbox.write_file("not-a-dir", b"x");
        let err = FsReadToolConfig::new(file)
            .build()
            .expect_err("file-as-root must fail to build");
        assert!(matches!(err, AivyxError::Config(_)));
    }

    #[test]
    fn sandbox_root_is_pre_canonicalized() {
        let sandbox = SandboxDir::new();
        let tool = build_tool(&sandbox);
        // The canonicalized root must be absolute and must exist.
        assert!(tool.sandbox_root().is_absolute());
        assert!(tool.sandbox_root().exists());
    }

    // ---- required_scope (lexical layer) -----------------------------

    #[test]
    fn scope_for_relative_path_inside_sandbox() {
        let sandbox = SandboxDir::new();
        sandbox.write_file("notes/today.md", b"hi");
        let tool = build_tool(&sandbox);

        let scope = tool.required_scope(&json!({"path": "notes/today.md"}));
        assert_eq!(scope.base(), "fs.read");
        let q = scope.qualifier().expect("must have a qualifier");
        assert!(q.ends_with("/notes/today.md"));
        assert!(q.starts_with(&tool.sandbox_root().display().to_string()));
    }

    #[test]
    fn scope_for_dot_slash_path_is_accepted() {
        // `./notes/today.md` is equivalent to `notes/today.md`. The
        // `CurDir` component handler skips the `.`.
        let sandbox = SandboxDir::new();
        let tool = build_tool(&sandbox);
        let scope = tool.required_scope(&json!({"path": "./notes/today.md"}));
        let q = scope.qualifier().expect("qualifier");
        assert!(q.ends_with("/notes/today.md"));
    }

    #[test]
    fn scope_for_traversal_input_is_deny_scope() {
        // The attack: `../../etc/passwd` lexically escapes the sandbox
        // root. Must produce the deny scope so the loop's scope gate
        // denies the call.
        let sandbox = SandboxDir::new();
        let tool = build_tool(&sandbox);
        let scope = tool.required_scope(&json!({"path": "../../etc/passwd"}));
        let q = scope.qualifier().expect("deny scope has a qualifier");
        assert!(
            q.contains("__deny__"),
            "traversal input must yield deny scope, got {q:?}"
        );
    }

    #[test]
    fn scope_for_absolute_outside_sandbox_is_deny_scope() {
        // Absolute `/etc/passwd` replaces the sandbox prefix via
        // `PathBuf::push` semantics. The post-collapse prefix check
        // catches it.
        let sandbox = SandboxDir::new();
        let tool = build_tool(&sandbox);
        let scope = tool.required_scope(&json!({"path": "/etc/passwd"}));
        let q = scope.qualifier().expect("deny scope has a qualifier");
        assert!(q.contains("__deny__"));
    }

    #[test]
    fn scope_for_absolute_inside_sandbox_is_accepted() {
        // An absolute path that's already under the sandbox root
        // should be accepted — the agent might have learned the full
        // path from a prior tool call.
        let sandbox = SandboxDir::new();
        sandbox.write_file("notes/today.md", b"hi");
        let tool = build_tool(&sandbox);
        let abs = tool.sandbox_root().join("notes/today.md");
        let scope = tool.required_scope(&json!({"path": abs.display().to_string()}));
        let q = scope.qualifier().expect("qualifier");
        assert!(q.ends_with("/notes/today.md"));
        assert!(!q.contains("__deny__"));
    }

    #[test]
    fn scope_for_missing_path_field_is_deny_scope() {
        let sandbox = SandboxDir::new();
        let tool = build_tool(&sandbox);
        let scope = tool.required_scope(&json!({"not_path": "hi"}));
        let q = scope.qualifier().expect("deny scope has a qualifier");
        assert!(q.contains("__deny__"));
    }

    #[test]
    fn scope_is_pure_no_io() {
        // `required_scope` must not hit the filesystem. Prove it by
        // calling it for a path that *doesn't exist* — if the tool
        // were calling `canonicalize` under the hood, this would fail.
        // Since we use lexical resolution only, it must produce a
        // valid in-sandbox scope.
        let sandbox = SandboxDir::new();
        let tool = build_tool(&sandbox);
        let scope = tool.required_scope(&json!({"path": "does/not/exist.txt"}));
        let q = scope.qualifier().expect("qualifier");
        assert!(q.ends_with("/does/not/exist.txt"));
        assert!(!q.contains("__deny__"));
    }

    // ---- required_scope intersects with a sandbox capability -------

    #[test]
    fn sandbox_capability_grants_derived_in_sandbox_scope() {
        // End-to-end scope-layer check: an agent holding
        // `fs.read:<canonical_sandbox>/**` must cover a derived scope
        // for any path under the sandbox.
        let sandbox = SandboxDir::new();
        sandbox.write_file("notes/today.md", b"hi");
        let tool = build_tool(&sandbox);

        let held_pattern = format!("fs.read:{}/**", tool.sandbox_root().display());
        let held = CapabilitySet::from_scopes([Scope::parse(&held_pattern).unwrap()]);
        let effective = held.intersect(TrustTier::Trusted.default_ceiling());

        let needed = tool.required_scope(&json!({"path": "notes/today.md"}));
        assert!(
            effective.grants(&needed),
            "sandbox capability must grant in-sandbox read, got needed={needed:?}"
        );
    }

    #[test]
    fn sandbox_capability_denies_traversal_scope() {
        let sandbox = SandboxDir::new();
        let tool = build_tool(&sandbox);

        let held_pattern = format!("fs.read:{}/**", tool.sandbox_root().display());
        let held = CapabilitySet::from_scopes([Scope::parse(&held_pattern).unwrap()]);
        let effective = held.intersect(TrustTier::Trusted.default_ceiling());

        let attack_needed = tool.required_scope(&json!({"path": "../../etc/passwd"}));
        assert!(
            !effective.grants(&attack_needed),
            "traversal scope must not be granted; needed was {attack_needed:?}"
        );
    }

    // ---- execute (canonical layer + read) ---------------------------

    #[test]
    fn execute_happy_path_reads_file() {
        let sandbox = SandboxDir::new();
        sandbox.write_file("notes/today.md", b"hello world");
        let tool = build_tool(&sandbox);

        let outcome = run_execute(&tool, json!({"path": "notes/today.md"}));
        match outcome {
            ToolOutcome::Completed { output, verified } => {
                assert!(matches!(verified, Verification::NotApplicable));
                assert_eq!(output["text"], json!("hello world"));
                assert_eq!(output["bytes"], json!(11));
                assert_eq!(output["truncated"], json!(false));
                assert_eq!(output["binary"], json!(false));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[test]
    fn execute_on_nonexistent_file_fails() {
        let sandbox = SandboxDir::new();
        let tool = build_tool(&sandbox);

        let outcome = run_execute(&tool, json!({"path": "missing.txt"}));
        match outcome {
            ToolOutcome::Failed(AivyxError::Tool { detail, .. }) => {
                assert!(
                    detail.contains("canonicalize"),
                    "expected canonicalize failure, got {detail}"
                );
            }
            other => panic!("expected Failed(Tool), got {other:?}"),
        }
    }

    #[test]
    fn execute_truncates_large_file() {
        let sandbox = SandboxDir::new();
        // A file bigger than the cap.
        let big = vec![b'A'; MAX_READ_BYTES + 1024];
        sandbox.write_file("big.bin", &big);
        let tool = build_tool(&sandbox);

        let outcome = run_execute(&tool, json!({"path": "big.bin"}));
        match outcome {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["bytes"], json!(MAX_READ_BYTES));
                assert_eq!(output["truncated"], json!(true));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[test]
    fn execute_binary_file_flags_and_lossy_decodes() {
        let sandbox = SandboxDir::new();
        // Invalid UTF-8 sequence.
        sandbox.write_file("blob.bin", &[0xFF, 0xFE, 0xFD, 0xFC]);
        let tool = build_tool(&sandbox);

        let outcome = run_execute(&tool, json!({"path": "blob.bin"}));
        match outcome {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["binary"], json!(true));
                assert!(output["text"].is_string());
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn execute_denies_symlink_that_escapes_sandbox() {
        // The symlink attack: create `<sandbox>/escape → /etc/passwd`.
        // `required_scope` produces the in-sandbox scope
        // `fs.read:<sandbox>/escape`, which the scope gate admits.
        // `execute` must then canonicalize and refuse because the
        // resolved path is outside the sandbox root.
        use std::os::unix::fs::symlink;

        let sandbox = SandboxDir::new();
        let target = "/etc/passwd";
        if !Path::new(target).exists() {
            // Skip on hosts without /etc/passwd (e.g., some minimal
            // containers). The symlink still demonstrates the escape
            // using whatever absolute file we can find, but if
            // /etc/passwd is missing the test is uninteresting.
            return;
        }
        let link = sandbox.root.join("escape");
        symlink(target, &link).expect("can create test symlink");
        let tool = build_tool(&sandbox);

        let outcome = run_execute(&tool, json!({"path": "escape"}));
        match outcome {
            ToolOutcome::Failed(AivyxError::Tool { detail, .. }) => {
                assert!(
                    detail.contains("escapes sandbox"),
                    "expected sandbox-escape failure, got {detail}"
                );
            }
            other => panic!(
                "symlink escape must be refused by the canonical fence, got {other:?}"
            ),
        }
    }

    #[test]
    #[cfg(unix)]
    fn execute_allows_symlink_that_stays_in_sandbox() {
        // The legitimate case: a symlink inside the sandbox pointing
        // to another file inside the sandbox. Canonicalization
        // resolves it, the resolved path is still in-sandbox, and the
        // read succeeds.
        use std::os::unix::fs::symlink;

        let sandbox = SandboxDir::new();
        let real = sandbox.write_file("real.md", b"inside content");
        let link = sandbox.root.join("alias");
        symlink(&real, &link).expect("can create intra-sandbox symlink");
        let tool = build_tool(&sandbox);

        let outcome = run_execute(&tool, json!({"path": "alias"}));
        match outcome {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["text"], json!("inside content"));
            }
            other => panic!("intra-sandbox symlink must succeed, got {other:?}"),
        }
    }
}

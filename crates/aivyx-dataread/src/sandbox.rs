//! The filesystem-sandbox spine the structured-data readers share.
//!
//! Chapter Sheaf (SH.1). Every reader (`data.csv`, and the SH.2/SH.3
//! `data.xlsx` / `data.pdf`) reads a file via **the exact same
//! `fs.read` capability and sandbox fence** as `aivyx_core`'s
//! [`FsReadTool`]: lexically resolve the `path` against the agent's
//! `fs_root`, derive an `fs.read:<resolved>` scope, then at execute
//! time re-resolve + canonicalize + verify the result still lives
//! under the root (the TOCTOU-resistant second fence). A reader can
//! therefore only ever read a file the agent could already `fs.read`
//! — it adds **no new capability and no new I/O reach**.
//!
//! [`FsReadTool`]: aivyx_core::tools::FsReadTool

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aivyx_capability::Scope;
use aivyx_core::tools::fs::lexical_resolve;
use aivyx_core::{AivyxError, ToolId, ToolOutcome};
use serde_json::Value;

/// Hard cap on the raw bytes a reader will pull off disk before
/// parsing, to bound memory on a hostile/huge file. Row/cell/text
/// caps apply *on top* of this per format.
pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;

/// Per-cell character cap shared by the tabular readers — a single
/// pathological cell can't blow the context. Mirrors `fs.read`'s
/// truncation discipline.
pub const MAX_CELL_CHARS: usize = 4_096;

/// Truncate an oversize cell at a char boundary, marking the cut with
/// an ellipsis.
pub fn cap_cell(s: &str) -> String {
    if s.chars().count() <= MAX_CELL_CHARS {
        return s.to_string();
    }
    let mut out: String = s.chars().take(MAX_CELL_CHARS).collect();
    out.push('…');
    out
}

/// A canonicalized read sandbox shared by all readers. Construct once
/// at agent assembly (like `FsReadToolConfig::build`); cheap to clone
/// (an `Arc<Path>`) across the readers and concurrent turns.
#[derive(Debug, Clone)]
pub struct ReaderSandbox {
    root: Arc<Path>,
    /// Chapter Ward — the sensitive-path read guard, shared with `fs.read`.
    /// Disabled by default (byte-identical) until the binary wires the
    /// operator's policy in.
    sensitive: Arc<aivyx_core::sensitive_paths::SensitivePolicy>,
}

/// A file that passed both fences, with its bytes read (capped).
pub struct GuardedFile {
    /// The canonicalized absolute path actually read.
    pub path: PathBuf,
    /// File bytes, truncated to [`MAX_FILE_BYTES`].
    pub bytes: Vec<u8>,
    /// `true` if the file was larger than the byte cap.
    pub truncated: bool,
}

impl ReaderSandbox {
    /// Canonicalize `root` and return a ready sandbox. Fails if the
    /// root doesn't exist or isn't a directory — a configuration error
    /// the caller must see at startup, not at tool-call time.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, AivyxError> {
        let root = root.into();
        let canonical = std::fs::canonicalize(&root).map_err(|e| {
            AivyxError::Config(format!(
                "dataread sandbox root {root:?} cannot be canonicalized: {e}"
            ))
        })?;
        if !canonical.is_dir() {
            return Err(AivyxError::Config(format!(
                "dataread sandbox root {canonical:?} is not a directory"
            )));
        }
        Ok(Self {
            root: Arc::from(canonical),
            sensitive: Arc::new(
                aivyx_core::sensitive_paths::SensitivePolicy::disabled(),
            ),
        })
    }

    /// Chapter Ward — install the sensitive-path read guard so the data
    /// readers refuse the same secret set `fs.read` does.
    pub fn with_sensitive_policy(
        mut self,
        policy: Arc<aivyx_core::sensitive_paths::SensitivePolicy>,
    ) -> Self {
        self.sensitive = policy;
        self
    }

    /// The canonicalized sandbox root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Derive the `fs.read:<resolved>` scope for a reader call, or a
    /// deny-by-construction scope when `path` is missing or escapes
    /// the sandbox lexically. Identical shape to `FsReadTool`'s, so a
    /// reader is gated by the *same* capability the agent already
    /// holds for plain reads.
    pub fn scope_for(&self, input: &Value) -> Scope {
        let Some(path_str) = input.get("path").and_then(|v| v.as_str()) else {
            return deny_read_scope();
        };
        match lexical_resolve(&self.root, Path::new(path_str)) {
            Some(abs) => Scope::parse(&format!("fs.read:{}", abs.display()))
                .unwrap_or_else(deny_read_scope),
            None => deny_read_scope(),
        }
    }

    /// Run both fences and read the file (capped). On any failure
    /// returns a `Failed` [`ToolOutcome`] ready to hand back from
    /// `execute`.
    pub fn read_guarded(&self, input: &Value, tool: ToolId) -> Result<GuardedFile, ToolOutcome> {
        let path_str = match input.get("path").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return Err(fail(tool, "input must have a string `path` field".to_string())),
        };

        // Lexical fence (matches required_scope's resolve).
        let lexical = match lexical_resolve(&self.root, Path::new(path_str)) {
            Some(p) => p,
            None => {
                return Err(fail(
                    tool,
                    format!("path {path_str:?} escapes the sandbox root"),
                ))
            }
        };

        // Canonical fence — resolve symlinks, re-check containment.
        let canonical = match std::fs::canonicalize(&lexical) {
            Ok(p) => p,
            Err(e) => return Err(fail(tool, format!("cannot canonicalize {lexical:?}: {e}"))),
        };
        if !canonical.starts_with(&*self.root) {
            return Err(fail(
                tool,
                format!(
                    "path {canonical:?} escapes sandbox root {:?} after symlink resolution",
                    self.root
                ),
            ));
        }

        // Chapter Ward — refuse a secret location even inside the sandbox,
        // matching the `fs.read` guard so readers aren't an exfil bypass.
        if let Some(reason) = self.sensitive.classify(&canonical) {
            return Err(fail(
                tool,
                format!(
                    "refusing to read {} — {reason}. Add it to `[access] \
                     allow_sensitive_paths` to permit it.",
                    canonical.display()
                ),
            ));
        }

        match read_capped(&canonical) {
            Ok((bytes, truncated)) => Ok(GuardedFile { path: canonical, bytes, truncated }),
            Err(e) => Err(fail(tool, format!("read error on {canonical:?}: {e}"))),
        }
    }
}

/// Read a file up to [`MAX_FILE_BYTES`], reporting whether more
/// remained (so callers can flag truncation honestly).
fn read_capped(path: &Path) -> std::io::Result<(Vec<u8>, bool)> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buf = Vec::with_capacity(64 * 1024);
    let mut probe = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut probe)?;
        if n == 0 {
            return Ok((buf, false));
        }
        if buf.len() + n > MAX_FILE_BYTES {
            let room = MAX_FILE_BYTES.saturating_sub(buf.len());
            buf.extend_from_slice(&probe[..room]);
            return Ok((buf, true));
        }
        buf.extend_from_slice(&probe[..n]);
    }
}

/// A scope no real agent holds — denies a malformed / escaping call by
/// construction (the loop's scope gate refuses it before `execute`).
fn deny_read_scope() -> Scope {
    Scope::parse("fs.read:/aivyx/__deny__/invalid-input").expect("deny scope must parse")
}

fn fail(tool: ToolId, detail: String) -> ToolOutcome {
    ToolOutcome::Failed(AivyxError::Tool { tool, detail })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch_root() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "aivyx-dataread-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn new_rejects_missing_root() {
        assert!(ReaderSandbox::new("/no/such/dir/aivyx-xyz").is_err());
    }

    #[test]
    fn scope_for_derives_fs_read() {
        let root = scratch_root();
        std::fs::write(root.join("a.csv"), b"x").unwrap();
        let sb = ReaderSandbox::new(&root).unwrap();
        let scope = sb.scope_for(&json!({"path": "a.csv"}));
        assert_eq!(scope.base(), "fs.read");
    }

    #[test]
    fn escape_is_denied_lexically() {
        let root = scratch_root();
        let sb = ReaderSandbox::new(&root).unwrap();
        // `../` past the root resolves to a deny scope.
        let scope = sb.scope_for(&json!({"path": "../../../etc/passwd"}));
        assert!(scope.qualifier().unwrap_or_default().contains("__deny__"));
        // And read_guarded refuses it too.
        let err = sb.read_guarded(&json!({"path": "../../../etc/passwd"}), ToolId::new());
        assert!(err.is_err());
    }

    #[test]
    fn ward_guard_refuses_a_sensitive_file() {
        use aivyx_core::sensitive_paths::SensitivePolicy;
        let root = scratch_root();
        std::fs::write(root.join(".env"), b"API_KEY=secret").unwrap();
        std::fs::write(root.join("data.csv"), b"a,b\n1,2\n").unwrap();
        let sb = ReaderSandbox::new(&root)
            .unwrap()
            .with_sensitive_policy(Arc::new(SensitivePolicy::new(vec![], vec![])));
        // A reader cannot slurp a secret even inside the sandbox…
        assert!(sb
            .read_guarded(&json!({"path": ".env"}), ToolId::new())
            .is_err());
        // …ordinary data still reads.
        assert!(sb
            .read_guarded(&json!({"path": "data.csv"}), ToolId::new())
            .is_ok());
    }

    #[test]
    fn read_guarded_reads_a_file() {
        let root = scratch_root();
        std::fs::write(root.join("hello.txt"), b"hello world").unwrap();
        let sb = ReaderSandbox::new(&root).unwrap();
        let gf = sb.read_guarded(&json!({"path": "hello.txt"}), ToolId::new()).unwrap();
        assert_eq!(gf.bytes, b"hello world");
        assert!(!gf.truncated);
    }
}

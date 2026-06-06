//! `obsidian.create_note` — create a new note. Trusted-
//! gated. Refuses to overwrite existing notes (use
//! `obsidian.update_note` for that).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::VaultClient;

pub struct ObsidianCreateNote {
    id: ToolId,
    schema: Value,
    client: Arc<VaultClient>,
}

impl ObsidianCreateNote {
    pub fn new(client: Arc<VaultClient>) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for ObsidianCreateNote {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "obsidian.create_note"
    }

    fn description(&self) -> &str {
        "Create a new Obsidian note. Input is a JSON object \
         with required `path` (vault-relative, must end in \
         `.md`), required `content` (full file content; \
         operators wanting separate frontmatter + body \
         combine into one string with `---\\nkey: value\\n---\\n\\n` \
         + body). Optional `create_parents` (default false; \
         when true, missing parent directories are created \
         under the vault). Refuses to overwrite existing \
         files — use `obsidian.update_note` to modify an \
         existing note. Returns `{path, bytes_written}`. \
         Requires Trusted-tier capability grant for \
         `obsidian.write`."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("obsidian.write").expect("obsidian.write in KNOWN_BASES")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("obsidian.create_note: {reason}"),
                });
            }
        };
        // Write-mode resolution: leaf may not exist; parent
        // (if exists) must be under vault root.
        let resolved = match self.client.resolve_under_vault(&parsed.path, false) {
            Ok(p) => p,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("obsidian.create_note: {e}"),
                });
            }
        };
        if resolved.exists() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!(
                    "obsidian.create_note: file already exists ({:?}). Use obsidian.update_note to modify.",
                    parsed.path
                ),
            });
        }
        if parsed.create_parents {
            if let Some(parent) = resolved.parent() {
                if !parent.exists() {
                    if let Err(e) = tokio::fs::create_dir_all(parent).await {
                        return ToolOutcome::Failed(AivyxError::Tool {
                            tool: self.id,
                            detail: format!(
                                "obsidian.create_note: failed to create parent dirs: {e}"
                            ),
                        });
                    }
                }
            }
        }
        let bytes = parsed.content.as_bytes();
        if let Err(e) = tokio::fs::write(&resolved, bytes).await {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("obsidian.create_note: write failed: {e}"),
            });
        }
        let output = json!({
            "path": parsed.path,
            "bytes_written": bytes.len(),
        });
        ToolOutcome::Completed {
            output,
            verified: Verification::Verified,
        }
    }
}

#[derive(Debug)]
struct ParsedInput {
    path: String,
    content: String,
    create_parents: bool,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let path = obj
        .get("path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| "input must include a `path` string".to_string())?;
    if path.is_empty() {
        return Err("`path` must not be empty".to_string());
    }
    if !path.to_lowercase().ends_with(".md") {
        return Err("`path` must end in `.md`".to_string());
    }
    let content = obj
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "input must include a `content` string".to_string())?
        .to_string();
    let create_parents = match obj.get("create_parents") {
        None => false,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| "`create_parents` must be a boolean".to_string())?,
    };
    Ok(ParsedInput {
        path: path.to_string(),
        content,
        create_parents,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {"type": "string"},
            "content": {"type": "string"},
            "create_parents": {"type": "boolean", "default": false}
        },
        "required": ["path", "content"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_vault() -> PathBuf {
        // Per-process monotonic counter: pid + nanos alone can collide
        // when two tests build a path in the same clock tick, and one
        // test's `remove_dir_all` cleanup would then delete another's
        // vault (Phase 185 flaky-test isolation fix).
        static SEQ: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "aivyx-obs-create-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        fs::create_dir_all(&path).unwrap();
        std::fs::canonicalize(&path).unwrap()
    }

    fn stub_ctx<'a>() -> ToolContext<'a> {
        use aivyx_core::{AgentId, CancellationToken, NullAuditHook, SessionId, TurnId};
        struct N;
        #[async_trait]
        impl aivyx_core::ChannelContext for N {
            fn channel_name(&self) -> &str { "t" }
            fn platform(&self) -> aivyx_core::ChannelPlatform { aivyx_core::ChannelPlatform::Local }
            fn trust_tier(&self) -> aivyx_capability::TrustTier { aivyx_capability::TrustTier::Trusted }
            fn session_id(&self) -> SessionId { SessionId::new() }
            async fn stream_event(&self, _e: aivyx_core::StreamEvent<'_>) -> Result<(), aivyx_core::ChannelError> { Ok(()) }
            async fn finalize(&self, _o: &aivyx_core::TurnOutcome) -> Result<(), aivyx_core::ChannelError> { Ok(()) }
            fn cancellation_token(&self) -> CancellationToken { CancellationToken::new() }
        }
        ToolContext {
            agent_id: AgentId::new(),
            session_id: SessionId::new(),
            turn_id: TurnId::new(),
            channel: Box::leak(Box::new(N)),
            audit: Box::leak(Box::new(NullAuditHook)),
            cancellation: Box::leak(Box::new(CancellationToken::new())),
        }
    }

    #[test]
    fn parse_input_requires_md_extension() {
        let e = parse_input(&json!({"path": "note.txt", "content": "x"}))
            .expect_err("must error");
        assert!(e.contains(".md"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_path() {
        let e = parse_input(&json!({"path": "   ", "content": "x"}))
            .expect_err("must error");
        assert!(e.contains("path"), "{e}");
    }

    #[test]
    fn parse_input_rejects_missing_content() {
        let e =
            parse_input(&json!({"path": "note.md"})).expect_err("must error");
        assert!(e.contains("content"), "{e}");
    }

    #[test]
    fn parse_input_accepts_create_parents_true() {
        let p = parse_input(&json!({
            "path": "subdir/note.md",
            "content": "x",
            "create_parents": true
        }))
        .expect("parse");
        assert!(p.create_parents);
    }

    #[tokio::test]
    async fn create_note_writes_new_file() {
        let path = tmp_vault();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianCreateNote::new(client);
        let outcome = tool
            .execute(
                json!({"path": "new.md", "content": "hello world\n"}),
                &stub_ctx(),
            )
            .await;
        let ToolOutcome::Completed { output, .. } = outcome else {
            panic!("not completed");
        };
        assert_eq!(output["path"], "new.md");
        assert_eq!(output["bytes_written"], "hello world\n".len());
        let body = fs::read_to_string(path.join("new.md")).unwrap();
        assert_eq!(body, "hello world\n");
        let _ = fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn create_note_refuses_to_overwrite() {
        let path = tmp_vault();
        fs::write(path.join("existing.md"), "old").unwrap();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianCreateNote::new(client);
        let outcome = tool
            .execute(json!({"path": "existing.md", "content": "new"}), &stub_ctx())
            .await;
        let ToolOutcome::Failed(e) = outcome else {
            panic!("expected failure");
        };
        let msg = format!("{e}");
        assert!(msg.contains("already exists"), "{msg}");
        // Original content untouched.
        assert_eq!(fs::read_to_string(path.join("existing.md")).unwrap(), "old");
        let _ = fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn create_note_create_parents_makes_subdir() {
        let path = tmp_vault();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianCreateNote::new(client);
        let outcome = tool
            .execute(
                json!({
                    "path": "newsub/deep.md",
                    "content": "x",
                    "create_parents": true
                }),
                &stub_ctx(),
            )
            .await;
        assert!(matches!(outcome, ToolOutcome::Completed { .. }));
        assert!(path.join("newsub/deep.md").exists());
        let _ = fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn create_note_rejects_path_traversal() {
        let path = tmp_vault();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianCreateNote::new(client);
        let outcome = tool
            .execute(json!({"path": "../evil.md", "content": "x"}), &stub_ctx())
            .await;
        assert!(matches!(outcome, ToolOutcome::Failed(_)));
        let _ = fs::remove_dir_all(&path);
    }

    fn make_tool() -> ObsidianCreateNote {
        let client = Arc::new(VaultClient::from_canonical_root(std::env::temp_dir()));
        ObsidianCreateNote::new(client)
    }

    #[test]
    fn required_scope_is_obsidian_write() {
        assert_eq!(
            make_tool().required_scope(&json!({})).to_string(),
            "obsidian.write"
        );
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "obsidian.create_note");
    }
}

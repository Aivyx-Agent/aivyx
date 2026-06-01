//! `obsidian.update_note` — replace or append content of an
//! existing note. Trusted-gated.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::VaultClient;

pub struct ObsidianUpdateNote {
    id: ToolId,
    schema: Value,
    client: Arc<VaultClient>,
}

impl ObsidianUpdateNote {
    pub fn new(client: Arc<VaultClient>) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for ObsidianUpdateNote {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "obsidian.update_note"
    }

    fn description(&self) -> &str {
        "Modify an existing Obsidian note. Input is a JSON \
         object with required `path` (vault-relative), \
         required `mode` (`\"replace\"` to overwrite the \
         full file, or `\"append\"` to add content to the \
         end), and required `content`. Returns `{path, \
         mode, bytes_written, total_size}`. Refuses to \
         operate on non-existent notes — use \
         `obsidian.create_note` to create. Requires \
         Trusted-tier capability grant for `obsidian.write`."
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
                    detail: format!("obsidian.update_note: {reason}"),
                });
            }
        };
        // Read-mode resolution — file must exist.
        let resolved = match self.client.resolve_under_vault(&parsed.path, true) {
            Ok(p) => p,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("obsidian.update_note: {e}"),
                });
            }
        };
        if !resolved.is_file() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("obsidian.update_note: path must be a file: {resolved:?}"),
            });
        }
        let bytes = parsed.content.as_bytes();
        let total_size: u64;
        match parsed.mode {
            UpdateMode::Replace => {
                if let Err(e) = tokio::fs::write(&resolved, bytes).await {
                    return ToolOutcome::Failed(AivyxError::Tool {
                        tool: self.id,
                        detail: format!("obsidian.update_note: write failed: {e}"),
                    });
                }
                total_size = bytes.len() as u64;
            }
            UpdateMode::Append => {
                use tokio::io::AsyncWriteExt;
                let mut f = match tokio::fs::OpenOptions::new()
                    .append(true)
                    .open(&resolved)
                    .await
                {
                    Ok(f) => f,
                    Err(e) => {
                        return ToolOutcome::Failed(AivyxError::Tool {
                            tool: self.id,
                            detail: format!("obsidian.update_note: open-append failed: {e}"),
                        });
                    }
                };
                if let Err(e) = f.write_all(bytes).await {
                    return ToolOutcome::Failed(AivyxError::Tool {
                        tool: self.id,
                        detail: format!("obsidian.update_note: append write failed: {e}"),
                    });
                }
                total_size = match tokio::fs::metadata(&resolved).await {
                    Ok(m) => m.len(),
                    Err(_) => bytes.len() as u64,
                };
            }
        }
        let output = json!({
            "path": parsed.path,
            "mode": parsed.mode.as_str(),
            "bytes_written": bytes.len(),
            "total_size": total_size,
        });
        ToolOutcome::Completed {
            output,
            verified: Verification::Verified,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum UpdateMode {
    Replace,
    Append,
}

impl UpdateMode {
    fn as_str(self) -> &'static str {
        match self {
            UpdateMode::Replace => "replace",
            UpdateMode::Append => "append",
        }
    }
}

#[derive(Debug)]
struct ParsedInput {
    path: String,
    mode: UpdateMode,
    content: String,
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
    let mode = obj
        .get("mode")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| "input must include a `mode` string".to_string())?;
    let mode = match mode {
        "replace" => UpdateMode::Replace,
        "append" => UpdateMode::Append,
        other => {
            return Err(format!(
                "`mode` must be `\"replace\"` or `\"append\"`; got `{other}`"
            ));
        }
    };
    let content = obj
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "input must include a `content` string".to_string())?
        .to_string();
    Ok(ParsedInput {
        path: path.to_string(),
        mode,
        content,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {"type": "string"},
            "mode": {"type": "string", "enum": ["replace", "append"]},
            "content": {"type": "string"}
        },
        "required": ["path", "mode", "content"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_vault() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "aivyx-obs-update-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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
    fn parse_input_rejects_unknown_mode() {
        let e = parse_input(&json!({
            "path": "x.md",
            "mode": "delete",
            "content": "x"
        }))
        .expect_err("must error");
        assert!(e.contains("mode"), "{e}");
    }

    #[test]
    fn parse_input_accepts_replace_and_append() {
        for mode in ["replace", "append"] {
            let p = parse_input(&json!({
                "path": "x.md",
                "mode": mode,
                "content": "x"
            }))
            .expect("parse");
            assert_eq!(p.mode.as_str(), mode);
        }
    }

    #[test]
    fn parse_input_rejects_missing_fields() {
        for missing in [
            json!({"mode": "replace", "content": "x"}),
            json!({"path": "x.md", "content": "x"}),
            json!({"path": "x.md", "mode": "replace"}),
        ] {
            assert!(parse_input(&missing).is_err());
        }
    }

    #[tokio::test]
    async fn update_replace_overwrites() {
        let path = tmp_vault();
        fs::write(path.join("n.md"), "old").unwrap();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianUpdateNote::new(client);
        let outcome = tool
            .execute(
                json!({"path": "n.md", "mode": "replace", "content": "new"}),
                &stub_ctx(),
            )
            .await;
        assert!(matches!(outcome, ToolOutcome::Completed { .. }));
        assert_eq!(fs::read_to_string(path.join("n.md")).unwrap(), "new");
        let _ = fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn update_append_grows_file() {
        let path = tmp_vault();
        fs::write(path.join("n.md"), "line1\n").unwrap();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianUpdateNote::new(client);
        let outcome = tool
            .execute(
                json!({"path": "n.md", "mode": "append", "content": "line2\n"}),
                &stub_ctx(),
            )
            .await;
        assert!(matches!(outcome, ToolOutcome::Completed { .. }));
        assert_eq!(
            fs::read_to_string(path.join("n.md")).unwrap(),
            "line1\nline2\n"
        );
        let _ = fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn update_refuses_nonexistent_note() {
        let path = tmp_vault();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianUpdateNote::new(client);
        let outcome = tool
            .execute(
                json!({"path": "nope.md", "mode": "replace", "content": "x"}),
                &stub_ctx(),
            )
            .await;
        assert!(matches!(outcome, ToolOutcome::Failed(_)));
        let _ = fs::remove_dir_all(&path);
    }

    fn make_tool() -> ObsidianUpdateNote {
        let client = Arc::new(VaultClient::from_canonical_root(std::env::temp_dir()));
        ObsidianUpdateNote::new(client)
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
        assert_eq!(make_tool().name(), "obsidian.update_note");
    }
}

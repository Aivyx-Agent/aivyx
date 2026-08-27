//! `obsidian.delete_note` — delete a note. Trusted-gated.
//!
//! Idempotent on already-missing (returns
//! `was_already_missing: true` rather than failing) so
//! repeat invocations don't generate spurious errors. NOTE:
//! this is a permanent delete (no trash). Operators who
//! want move-to-trash semantics should move the file via
//! their OS file manager.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::{VaultClient, VaultError};

pub struct ObsidianDeleteNote {
    id: ToolId,
    schema: Value,
    client: Arc<VaultClient>,
}

impl ObsidianDeleteNote {
    pub fn new(client: Arc<VaultClient>) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for ObsidianDeleteNote {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "obsidian.delete_note"
    }

    fn description(&self) -> &str {
        "Permanently delete an Obsidian note. Input is a JSON \
         object with a required `path` (vault-relative). \
         Idempotent: deleting an already-missing note succeeds \
         with `was_already_missing: true`. NOTE: this is a \
         permanent delete — no trash. Requires Trusted-tier \
         capability grant for `obsidian.write`."
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
                    detail: format!("obsidian.delete_note: {reason}"),
                });
            }
        };
        // Try read-mode resolution first. If the file is
        // missing the guard returns NoteNotFound which we
        // map to was_already_missing: true.
        let resolution = self.client.resolve_under_vault(&parsed.path, true);
        let resolved = match resolution {
            Ok(p) => p,
            Err(VaultError::NoteNotFound(_)) => {
                let output = json!({
                    "path": parsed.path,
                    "was_already_missing": true,
                });
                return ToolOutcome::Completed {
                    output,
                    verified: Verification::Verified,
                };
            }
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("obsidian.delete_note: {e}"),
                });
            }
        };
        if !resolved.is_file() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("obsidian.delete_note: path must be a file: {resolved:?}"),
            });
        }
        if let Err(e) = tokio::fs::remove_file(&resolved).await {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("obsidian.delete_note: remove failed: {e}"),
            });
        }
        let output = json!({
            "path": parsed.path,
            "was_already_missing": false,
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
    Ok(ParsedInput {
        path: path.to_string(),
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {"type": "string"}
        },
        "required": ["path"],
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
            "aivyx-obs-del-{}-{}-{}",
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
            message_origin: aivyx_core::MessageOrigin::Operator,
        }
    }

    #[test]
    fn parse_input_rejects_missing_path() {
        let e = parse_input(&json!({})).expect_err("must error");
        assert!(e.contains("path"), "{e}");
    }

    #[tokio::test]
    async fn delete_removes_existing_file() {
        let path = tmp_vault();
        fs::write(path.join("doomed.md"), "x").unwrap();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianDeleteNote::new(client);
        let outcome = tool.execute(json!({"path": "doomed.md"}), &stub_ctx()).await;
        let ToolOutcome::Completed { output, .. } = outcome else {
            panic!("not completed");
        };
        assert_eq!(output["was_already_missing"], false);
        assert!(!path.join("doomed.md").exists());
        let _ = fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn delete_idempotent_on_missing() {
        let path = tmp_vault();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianDeleteNote::new(client);
        let outcome = tool
            .execute(json!({"path": "never-existed.md"}), &stub_ctx())
            .await;
        let ToolOutcome::Completed { output, .. } = outcome else {
            panic!("not completed");
        };
        assert_eq!(output["was_already_missing"], true);
        let _ = fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn delete_rejects_path_traversal() {
        let path = tmp_vault();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianDeleteNote::new(client);
        let outcome = tool
            .execute(json!({"path": "../escape.md"}), &stub_ctx())
            .await;
        assert!(matches!(outcome, ToolOutcome::Failed(_)));
        let _ = fs::remove_dir_all(&path);
    }

    fn make_tool() -> ObsidianDeleteNote {
        let client = Arc::new(VaultClient::from_canonical_root(std::env::temp_dir()));
        ObsidianDeleteNote::new(client)
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
        assert_eq!(make_tool().name(), "obsidian.delete_note");
    }
}

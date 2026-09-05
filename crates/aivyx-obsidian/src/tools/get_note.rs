//! `obsidian.get_note` — fetch one note's content + parsed
//! pieces.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::markdown::{extract_tags, extract_wikilinks, split_frontmatter};
use crate::VaultClient;

pub struct ObsidianGetNote {
    id: ToolId,
    schema: Value,
    client: Arc<VaultClient>,
}

impl ObsidianGetNote {
    pub fn new(client: Arc<VaultClient>) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for ObsidianGetNote {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "obsidian.get_note"
    }

    // Chapter Picket follow-up (Finding 3) — note content could have
    // been written by anyone with vault access or synced from
    // elsewhere; externally authored, the same rationale as fs.read.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Fetch one Obsidian note. Input is a JSON object with \
         a required `path` (vault-relative). Returns \
         `{path, content, frontmatter_raw, body, wikilinks, \
         tags}`. `content` is the raw file text; \
         `frontmatter_raw` is the YAML between the top \
         `---` markers (null when absent); `body` is the \
         text after frontmatter (or full content when \
         absent); `wikilinks` is an array of \
         `[[link target]]` strings (deduped, display text \
         dropped); `tags` is an array of `#tag` tokens \
         (deduped, hash stripped). YAML inside \
         `frontmatter_raw` is returned as a string — \
         consumers parse it LLM-side if they care."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("obsidian.read").expect("obsidian.read in KNOWN_BASES")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("obsidian.get_note: {reason}"),
                });
            }
        };
        let resolved = match self.client.resolve_under_vault(&parsed.path, true) {
            Ok(p) => p,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("obsidian.get_note: {e}"),
                });
            }
        };
        if !resolved.is_file() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("obsidian.get_note: path must be a file: {resolved:?}"),
            });
        }
        let content = match tokio::fs::read_to_string(&resolved).await {
            Ok(s) => s,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("obsidian.get_note: read failed: {e}"),
                });
            }
        };
        let (fm, body) = split_frontmatter(&content);
        let wikilinks = extract_wikilinks(&body);
        let tags = extract_tags(&body);

        let output = json!({
            "path": parsed.path,
            "content": content,
            "frontmatter_raw": fm.map(Value::String).unwrap_or(Value::Null),
            "body": body,
            "wikilinks": wikilinks,
            "tags": tags,
        });

        ToolOutcome::Completed {
            output,
            verified: Verification::NotApplicable,
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
            "path": {"type": "string", "description": "Vault-relative path. Required."}
        },
        "required": ["path"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsidian_get_note_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }
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
            "aivyx-obs-get-{}-{}-{}",
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
        struct NoopChannel;
        #[async_trait]
        impl aivyx_core::ChannelContext for NoopChannel {
            fn channel_name(&self) -> &str { "test" }
            fn platform(&self) -> aivyx_core::ChannelPlatform { aivyx_core::ChannelPlatform::Local }
            fn trust_tier(&self) -> aivyx_capability::TrustTier { aivyx_capability::TrustTier::Trusted }
            fn session_id(&self) -> SessionId { SessionId::new() }
            async fn stream_event(&self, _e: aivyx_core::StreamEvent<'_>) -> Result<(), aivyx_core::ChannelError> { Ok(()) }
            async fn finalize(&self, _o: &aivyx_core::TurnOutcome) -> Result<(), aivyx_core::ChannelError> { Ok(()) }
            fn cancellation_token(&self) -> CancellationToken { CancellationToken::new() }
        }
        let channel: &'static dyn aivyx_core::ChannelContext = Box::leak(Box::new(NoopChannel));
        let audit: &'static NullAuditHook = Box::leak(Box::new(NullAuditHook));
        let cancel: &'static CancellationToken = Box::leak(Box::new(CancellationToken::new()));
        ToolContext {
            agent_id: AgentId::new(),
            session_id: SessionId::new(),
            turn_id: TurnId::new(),
            channel,
            audit,
            cancellation: cancel,
            message_origin: aivyx_core::MessageOrigin::Operator,
        }
    }

    #[test]
    fn parse_input_rejects_missing_path() {
        let e = parse_input(&json!({})).expect_err("must error");
        assert!(e.contains("path"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_path() {
        let e = parse_input(&json!({"path": "   "})).expect_err("must error");
        assert!(e.contains("path"), "{e}");
    }

    #[tokio::test]
    async fn get_note_returns_full_shape_for_frontmattered_note() {
        let path = tmp_vault();
        let content = "---\ntags: [project]\nstatus: active\n---\n\nHello [[Other]] and #work\n";
        fs::write(path.join("note.md"), content).unwrap();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianGetNote::new(client);
        let outcome = tool.execute(json!({"path": "note.md"}), &stub_ctx()).await;
        let ToolOutcome::Completed { output, .. } = outcome else {
            panic!("not completed");
        };
        assert_eq!(output["path"], "note.md");
        assert_eq!(output["content"], content);
        assert!(output["frontmatter_raw"].as_str().unwrap().contains("status:"));
        assert!(output["body"].as_str().unwrap().contains("Hello"));
        assert_eq!(output["wikilinks"][0], "Other");
        assert!(output["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "work"));
        let _ = fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn get_note_returns_null_frontmatter_when_absent() {
        let path = tmp_vault();
        fs::write(path.join("plain.md"), "just body\n").unwrap();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianGetNote::new(client);
        let outcome = tool.execute(json!({"path": "plain.md"}), &stub_ctx()).await;
        let ToolOutcome::Completed { output, .. } = outcome else {
            panic!("not completed");
        };
        assert!(output["frontmatter_raw"].is_null());
        assert_eq!(output["body"], "just body\n");
        let _ = fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn get_note_rejects_path_traversal() {
        let path = tmp_vault();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianGetNote::new(client);
        let outcome = tool.execute(json!({"path": "../escape.md"}), &stub_ctx()).await;
        let ToolOutcome::Failed(_) = outcome else {
            panic!("expected failure");
        };
        let _ = fs::remove_dir_all(&path);
    }

    fn make_tool() -> ObsidianGetNote {
        let client = Arc::new(VaultClient::from_canonical_root(std::env::temp_dir()));
        ObsidianGetNote::new(client)
    }

    #[test]
    fn required_scope_is_obsidian_read() {
        assert_eq!(
            make_tool().required_scope(&json!({})).to_string(),
            "obsidian.read"
        );
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "obsidian.get_note");
    }
}

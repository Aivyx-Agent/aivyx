//! `obsidian.search` — recursive vault search.
//!
//! Phase 130 Task 11. First Obsidian tool.
//!
//! ## What we search
//!
//! Walks every `.md` file under the configured vault
//! (optionally rooted at a sub-folder), running a
//! grep-style line-by-line match for `q` plus optional
//! tag and frontmatter filters. Wikilinks aren't fuzzy-
//! resolved; operators wanting wikilink-aware search
//! include `[[Target]]` in their `q`.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::markdown::{extract_tags, split_frontmatter};
use crate::{VaultClient, MARKDOWN_EXT};

const MAX_RESULTS_CAP: u64 = 200;
const DEFAULT_MAX_RESULTS: u64 = 50;
/// Per-snippet cap on extracted match context. Keeps the
/// LLM-side payload predictable.
const SNIPPET_MAX_CHARS: usize = 240;

pub struct ObsidianSearch {
    id: ToolId,
    schema: Value,
    client: Arc<VaultClient>,
}

impl ObsidianSearch {
    pub fn new(client: Arc<VaultClient>) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for ObsidianSearch {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "obsidian.search"
    }

    // Chapter Picket follow-up (Finding 3) — same rationale as
    // obsidian.get_note: externally authored vault content.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Search the Obsidian vault. Input is a JSON object \
         with optional `q` (substring matched line-by-line \
         in note bodies; case-insensitive), optional `tag` \
         (matches notes containing this `#tag` token in \
         their body), optional `frontmatter_key` + \
         `frontmatter_value` pair (substring match in the \
         raw frontmatter text), optional `folder` \
         (vault-relative path; default is the vault root), \
         and `max_results` (default 50, capped at 200). \
         Returns `matches` array of `{path, snippet, \
         line_number}` entries. Only `.md` files are \
         scanned; subfolders are walked recursively. \
         Wikilinks are not fuzzy-resolved — include \
         `[[Target]]` in `q` for wikilink-shaped search."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("obsidian.read").expect(
            "obsidian.read must parse — it is in KNOWN_BASES from Phase 130",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("obsidian.search: {reason}"),
                });
            }
        };

        // Resolve the search root via the path-traversal
        // guard. `folder` is read-mode (must exist).
        let root = match self.client.resolve_under_vault(&parsed.folder, true) {
            Ok(p) => p,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("obsidian.search: folder resolution failed: {e}"),
                });
            }
        };
        if !root.is_dir() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("obsidian.search: `folder` must be a directory, got: {root:?}"),
            });
        }

        let vault_root = self.client.vault_root().to_path_buf();
        let mut matches: Vec<Value> = Vec::new();
        let mut stack: Vec<std::path::PathBuf> = vec![root];
        let max_results = parsed.max_results as usize;

        'walk: while let Some(dir) = stack.pop() {
            let entries = match tokio::fs::read_dir(&dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };
            let mut entries = entries;
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let Ok(meta) = entry.metadata().await else {
                    continue;
                };
                if meta.is_dir() {
                    // Skip Obsidian's `.obsidian` config
                    // dir + any dotfile dir.
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with('.') {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if !is_markdown_file(&path) {
                    continue;
                }
                let body = match tokio::fs::read_to_string(&path).await {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                // Build a vault-relative path for the output.
                let rel = path
                    .strip_prefix(&vault_root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();

                // Filter pass: tag + frontmatter match if
                // the operator supplied either.
                let (fm, body_text) = split_frontmatter(&body);
                if let Some(ref needed_tag) = parsed.tag {
                    let tags = extract_tags(&body_text);
                    if !tags.iter().any(|t| t == needed_tag) {
                        continue;
                    }
                }
                if let Some((ref key, ref value)) = parsed.frontmatter_match {
                    let fm_raw = fm.as_deref().unwrap_or("");
                    if !frontmatter_matches(fm_raw, key, value) {
                        continue;
                    }
                }

                // Body match: if q is supplied, scan
                // line-by-line. If q is absent, the file
                // matches on tag/frontmatter alone (no
                // snippet — line_number 0).
                if let Some(ref q) = parsed.q {
                    let q_lower = q.to_lowercase();
                    for (idx, line) in body_text.lines().enumerate() {
                        if line.to_lowercase().contains(&q_lower) {
                            matches.push(json!({
                                "path": rel,
                                "snippet": clip_snippet(line),
                                "line_number": idx + 1,
                            }));
                            if matches.len() >= max_results {
                                break 'walk;
                            }
                        }
                    }
                } else {
                    matches.push(json!({
                        "path": rel,
                        "snippet": Value::Null,
                        "line_number": 0,
                    }));
                    if matches.len() >= max_results {
                        break 'walk;
                    }
                }
            }
        }

        let output = json!({"matches": matches});
        ToolOutcome::Completed {
            output,
            verified: Verification::NotApplicable,
        }
    }
}

fn is_markdown_file(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case(MARKDOWN_EXT))
        .unwrap_or(false)
}

fn clip_snippet(line: &str) -> Value {
    let trimmed = line.trim();
    let len = trimmed.chars().count();
    if len <= SNIPPET_MAX_CHARS {
        Value::String(trimmed.to_string())
    } else {
        let cut: String = trimmed.chars().take(SNIPPET_MAX_CHARS).collect();
        Value::String(format!("{cut}..."))
    }
}

/// Substring match against frontmatter for
/// `frontmatter_key: <something matching value>`. We scan
/// the raw frontmatter for a line whose trimmed start is
/// `key:` and whose tail contains the value substring.
/// Lightweight + tolerates the operator's preferred YAML
/// shape without a full YAML parser.
pub(crate) fn frontmatter_matches(fm_raw: &str, key: &str, value: &str) -> bool {
    for line in fm_raw.lines() {
        let trimmed = line.trim_start();
        let prefix = format!("{key}:");
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            if rest.contains(value) {
                return true;
            }
        }
    }
    false
}

#[derive(Debug)]
pub(crate) struct ParsedInput {
    pub(crate) q: Option<String>,
    pub(crate) tag: Option<String>,
    pub(crate) frontmatter_match: Option<(String, String)>,
    pub(crate) folder: String,
    pub(crate) max_results: u64,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let q = match obj.get("q") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Some(_) => return Err("`q` must be a string".to_string()),
    };
    let tag = match obj.get("tag") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let t = s.trim().trim_start_matches('#');
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Some(_) => return Err("`tag` must be a string".to_string()),
    };
    let frontmatter_match = match (
        obj.get("frontmatter_key"),
        obj.get("frontmatter_value"),
    ) {
        (None, None) | (Some(Value::Null), Some(Value::Null)) => None,
        (Some(Value::String(k)), Some(Value::String(v))) => {
            let k = k.trim();
            let v = v.trim();
            if k.is_empty() {
                return Err("`frontmatter_key` must not be empty".to_string());
            }
            Some((k.to_string(), v.to_string()))
        }
        _ => return Err(
            "`frontmatter_key` and `frontmatter_value` must be supplied together as strings"
                .to_string(),
        ),
    };
    let folder = match obj.get("folder") {
        None => ".".to_string(),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                ".".to_string()
            } else {
                t.to_string()
            }
        }
        Some(_) => return Err("`folder` must be a string".to_string()),
    };
    let max_results = match obj.get("max_results") {
        None => DEFAULT_MAX_RESULTS,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| "`max_results` must be a non-negative integer".to_string())?,
    };
    if max_results == 0 {
        return Err("`max_results` must be >= 1".to_string());
    }
    let max_results = max_results.min(MAX_RESULTS_CAP);
    if q.is_none() && tag.is_none() && frontmatter_match.is_none() {
        return Err(
            "must supply at least one of `q`, `tag`, or (frontmatter_key + frontmatter_value)"
                .to_string(),
        );
    }
    Ok(ParsedInput {
        q,
        tag,
        frontmatter_match,
        folder,
        max_results,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "q": {"type": ["string", "null"]},
            "tag": {"type": ["string", "null"]},
            "frontmatter_key": {"type": ["string", "null"]},
            "frontmatter_value": {"type": ["string", "null"]},
            "folder": {"type": ["string", "null"], "default": "."},
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_RESULTS_CAP,
                "default": DEFAULT_MAX_RESULTS
            }
        },
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsidian_search_output_is_untrusted_for_bulwark() {
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
            "aivyx-obs-search-{}-{}-{}",
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

    #[test]
    fn parse_input_requires_at_least_one_filter() {
        let e = parse_input(&json!({})).expect_err("must error");
        assert!(e.contains("must supply"), "{e}");
    }

    #[test]
    fn parse_input_accepts_q_only() {
        let p = parse_input(&json!({"q": "term"})).expect("parse");
        assert_eq!(p.q.as_deref(), Some("term"));
    }

    #[test]
    fn parse_input_accepts_tag_only() {
        let p = parse_input(&json!({"tag": "work"})).expect("parse");
        assert_eq!(p.tag.as_deref(), Some("work"));
    }

    #[test]
    fn parse_input_strips_hash_from_tag_input() {
        let p = parse_input(&json!({"tag": "#work"})).expect("parse");
        assert_eq!(p.tag.as_deref(), Some("work"));
    }

    #[test]
    fn parse_input_rejects_frontmatter_pair_without_value() {
        let e = parse_input(&json!({"frontmatter_key": "k"})).expect_err("must error");
        assert!(e.contains("together"), "{e}");
    }

    #[test]
    fn parse_input_accepts_frontmatter_pair() {
        let p = parse_input(&json!({
            "frontmatter_key": "status",
            "frontmatter_value": "active"
        }))
        .expect("parse");
        assert!(p.frontmatter_match.is_some());
    }

    #[test]
    fn parse_input_caps_max_results() {
        let p = parse_input(&json!({"q": "x", "max_results": 9999})).expect("parse");
        assert_eq!(p.max_results, MAX_RESULTS_CAP);
    }

    #[test]
    fn frontmatter_matches_finds_simple_key_value() {
        let fm = "status: active\ntags: [a, b]\n";
        assert!(frontmatter_matches(fm, "status", "active"));
        assert!(!frontmatter_matches(fm, "status", "done"));
        assert!(!frontmatter_matches(fm, "missing", "x"));
    }

    #[test]
    fn frontmatter_matches_substring_in_value() {
        let fm = "tags: [project-aivyx, work]\n";
        assert!(frontmatter_matches(fm, "tags", "aivyx"));
    }

    #[test]
    fn clip_snippet_passes_short_lines_through() {
        let v = clip_snippet("short line");
        assert_eq!(v, "short line");
    }

    #[test]
    fn clip_snippet_truncates_long_lines_with_ellipsis() {
        let long: String = "x".repeat(300);
        let Value::String(s) = clip_snippet(&long) else {
            panic!("expected string");
        };
        assert!(s.ends_with("..."));
        assert!(s.chars().count() == SNIPPET_MAX_CHARS + 3);
    }

    #[test]
    fn is_markdown_file_accepts_md_extension() {
        assert!(is_markdown_file(Path::new("note.md")));
        assert!(is_markdown_file(Path::new("Note.MD")));
        assert!(!is_markdown_file(Path::new("note.txt")));
        assert!(!is_markdown_file(Path::new("noteno-ext")));
    }

    #[tokio::test]
    async fn search_finds_q_match_in_note_body() {
        let path = tmp_vault();
        fs::write(path.join("a.md"), "hello\nworld\n").unwrap();
        fs::write(path.join("b.md"), "nothing here\n").unwrap();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianSearch::new(Arc::clone(&client));
        let ctx = stub_ctx();
        let outcome = tool.execute(json!({"q": "world"}), &ctx).await;
        let ToolOutcome::Completed { output, .. } = outcome else {
            panic!("not completed");
        };
        let matches = output["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["path"], "a.md");
        assert_eq!(matches[0]["line_number"], 2);
        let _ = fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn search_recurses_into_subfolders() {
        let path = tmp_vault();
        fs::create_dir(path.join("subfolder")).unwrap();
        fs::write(path.join("subfolder/nested.md"), "deep find me\n").unwrap();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianSearch::new(Arc::clone(&client));
        let outcome = tool.execute(json!({"q": "deep"}), &stub_ctx()).await;
        let ToolOutcome::Completed { output, .. } = outcome else {
            panic!("not completed");
        };
        let matches = output["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert!(matches[0]["path"]
            .as_str()
            .unwrap()
            .contains("nested.md"));
        let _ = fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn search_filters_by_tag() {
        let path = tmp_vault();
        fs::write(path.join("a.md"), "#work this is tagged\n").unwrap();
        fs::write(path.join("b.md"), "no tag here\n").unwrap();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianSearch::new(Arc::clone(&client));
        let outcome = tool.execute(json!({"tag": "work"}), &stub_ctx()).await;
        let ToolOutcome::Completed { output, .. } = outcome else {
            panic!("not completed");
        };
        let matches = output["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["path"], "a.md");
        let _ = fs::remove_dir_all(&path);
    }

    #[tokio::test]
    async fn search_skips_dotfile_dirs() {
        let path = tmp_vault();
        fs::create_dir(path.join(".obsidian")).unwrap();
        fs::write(path.join(".obsidian/cfg.md"), "find me\n").unwrap();
        fs::write(path.join("real.md"), "find me too\n").unwrap();
        let client = Arc::new(VaultClient::from_canonical_root(path.clone()));
        let tool = ObsidianSearch::new(Arc::clone(&client));
        let outcome = tool.execute(json!({"q": "find me"}), &stub_ctx()).await;
        let ToolOutcome::Completed { output, .. } = outcome else {
            panic!("not completed");
        };
        let matches = output["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["path"], "real.md");
        let _ = fs::remove_dir_all(&path);
    }

    fn stub_ctx<'a>() -> ToolContext<'a> {
        use aivyx_core::{
            AgentId, CancellationToken, NullAuditHook, SessionId, TurnId,
        };
        struct NoopChannel;
        #[async_trait]
        impl aivyx_core::ChannelContext for NoopChannel {
            fn channel_name(&self) -> &str {
                "test"
            }
            fn platform(&self) -> aivyx_core::ChannelPlatform {
                aivyx_core::ChannelPlatform::Local
            }
            fn trust_tier(&self) -> aivyx_capability::TrustTier {
                aivyx_capability::TrustTier::Trusted
            }
            fn session_id(&self) -> SessionId {
                SessionId::new()
            }
            async fn stream_event(
                &self,
                _event: aivyx_core::StreamEvent<'_>,
            ) -> Result<(), aivyx_core::ChannelError> {
                Ok(())
            }
            async fn finalize(
                &self,
                _outcome: &aivyx_core::TurnOutcome,
            ) -> Result<(), aivyx_core::ChannelError> {
                Ok(())
            }
            fn cancellation_token(&self) -> CancellationToken {
                CancellationToken::new()
            }
        }
        let leaked_channel: &'static dyn aivyx_core::ChannelContext =
            Box::leak(Box::new(NoopChannel));
        let leaked_audit: &'static aivyx_core::NullAuditHook =
            Box::leak(Box::new(NullAuditHook));
        let leaked_cancel: &'static CancellationToken =
            Box::leak(Box::new(CancellationToken::new()));
        ToolContext {
            agent_id: AgentId::new(),
            session_id: SessionId::new(),
            turn_id: TurnId::new(),
            channel: leaked_channel,
            audit: leaked_audit,
            cancellation: leaked_cancel,
            message_origin: aivyx_core::MessageOrigin::Operator,
        }
    }

    fn make_tool() -> ObsidianSearch {
        let path = std::env::temp_dir();
        let client = Arc::new(VaultClient::from_canonical_root(path));
        ObsidianSearch::new(client)
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
        assert_eq!(make_tool().name(), "obsidian.search");
    }
}

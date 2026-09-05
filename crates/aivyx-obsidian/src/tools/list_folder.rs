//! `obsidian.list_folder` — list markdown notes in a vault
//! subdirectory.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::{VaultClient, MARKDOWN_EXT};

const MAX_RESULTS_CAP: u64 = 500;
const DEFAULT_MAX_RESULTS: u64 = 100;

pub struct ObsidianListFolder {
    id: ToolId,
    schema: Value,
    client: Arc<VaultClient>,
}

impl ObsidianListFolder {
    pub fn new(client: Arc<VaultClient>) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for ObsidianListFolder {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "obsidian.list_folder"
    }

    // Chapter Picket follow-up (Finding 3) — filenames in the vault
    // are externally authored, the same rationale as fs.read.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "List markdown notes in a vault folder. Input is a \
         JSON object with optional `folder` (vault-relative; \
         default is the vault root), optional `recursive` \
         (default false), and `max_results` (default 100, \
         capped at 500). Returns `notes` array of \
         `{path, name, size, modified_at}` entries; only \
         `.md` files are included. Dotfile dirs (e.g., \
         `.obsidian`) are skipped during walk."
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
                    detail: format!("obsidian.list_folder: {reason}"),
                });
            }
        };
        let root = match self.client.resolve_under_vault(&parsed.folder, true) {
            Ok(p) => p,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("obsidian.list_folder: {e}"),
                });
            }
        };
        if !root.is_dir() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("obsidian.list_folder: folder must be a directory: {root:?}"),
            });
        }
        let vault_root = self.client.vault_root().to_path_buf();
        let mut notes: Vec<Value> = Vec::new();
        let mut stack: Vec<std::path::PathBuf> = vec![root];
        let max_results = parsed.max_results as usize;

        'walk: while let Some(dir) = stack.pop() {
            let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
                continue;
            };
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let Ok(meta) = entry.metadata().await else {
                    continue;
                };
                if meta.is_dir() {
                    let name = entry.file_name();
                    if name.to_string_lossy().starts_with('.') {
                        continue;
                    }
                    if parsed.recursive {
                        stack.push(path);
                    }
                    continue;
                }
                if path
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| !s.eq_ignore_ascii_case(MARKDOWN_EXT))
                    .unwrap_or(true)
                {
                    continue;
                }
                let rel = path
                    .strip_prefix(&vault_root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                let name = entry.file_name().to_string_lossy().to_string();
                let size = meta.len();
                let modified_at = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| Value::Number(d.as_secs().into()))
                    .unwrap_or(Value::Null);
                notes.push(json!({
                    "path": rel,
                    "name": name,
                    "size": size,
                    "modified_at": modified_at,
                }));
                if notes.len() >= max_results {
                    break 'walk;
                }
            }
        }

        let output = json!({"notes": notes});
        ToolOutcome::Completed {
            output,
            verified: Verification::NotApplicable,
        }
    }
}

#[derive(Debug)]
struct ParsedInput {
    folder: String,
    recursive: bool,
    max_results: u64,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let folder = match obj.get("folder") {
        None | Some(Value::Null) => ".".to_string(),
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
    let recursive = match obj.get("recursive") {
        None => false,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| "`recursive` must be a boolean".to_string())?,
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
    Ok(ParsedInput {
        folder,
        recursive,
        max_results,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "folder": {"type": ["string", "null"], "default": "."},
            "recursive": {"type": "boolean", "default": false},
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
    fn obsidian_list_folder_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }

    #[test]
    fn parse_input_defaults_when_empty() {
        let p = parse_input(&json!({})).expect("parse");
        assert_eq!(p.folder, ".");
        assert!(!p.recursive);
        assert_eq!(p.max_results, DEFAULT_MAX_RESULTS);
    }

    #[test]
    fn parse_input_accepts_recursive_true() {
        let p = parse_input(&json!({"recursive": true})).expect("parse");
        assert!(p.recursive);
    }

    #[test]
    fn parse_input_rejects_non_boolean_recursive() {
        let e = parse_input(&json!({"recursive": "yes"})).expect_err("must error");
        assert!(e.contains("recursive"), "{e}");
    }

    #[test]
    fn parse_input_caps_max_results() {
        let p = parse_input(&json!({"max_results": 99999})).expect("parse");
        assert_eq!(p.max_results, MAX_RESULTS_CAP);
    }

    #[test]
    fn parse_input_rejects_zero_max_results() {
        let e = parse_input(&json!({"max_results": 0})).expect_err("must error");
        assert!(e.contains(">= 1"), "{e}");
    }

    fn make_tool() -> ObsidianListFolder {
        let client = Arc::new(VaultClient::from_canonical_root(std::env::temp_dir()));
        ObsidianListFolder::new(client)
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
        assert_eq!(make_tool().name(), "obsidian.list_folder");
    }

    #[test]
    fn input_schema_no_required_fields() {
        let schema = input_schema();
        let req = schema.get("required");
        assert!(req.is_none() || req.unwrap().as_array().unwrap().is_empty());
    }
}

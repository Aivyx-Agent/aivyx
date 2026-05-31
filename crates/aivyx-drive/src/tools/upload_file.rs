//! `drive.upload_file` — create a new Drive file with
//! content.
//!
//! Phase 129 Task 9. Sixth Drive tool; second write tool
//! (`drive.write` capability, Trusted-tier-only at the
//! ceiling level).
//!
//! ## API call
//!
//! Multipart upload via `POST /upload/drive/v3/files` (with
//! `uploadType=multipart` query param) carrying two parts:
//! a JSON metadata part (name, mimeType, parents,
//! description) and a media part with the raw bytes.
//! Single-shot multipart covers the 10 MB Phase 129 Q3a
//! inline cap; resumable upload is deferred.
//!
//! ## Size enforcement
//!
//! Operator-supplied `content_base64` is decoded, then the
//! decoded length is checked against the 10 MB cap BEFORE
//! the HTTP request. Above-cap content is rejected at
//! input validation with a clear error rather than failing
//! the upload mid-flight.

use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use super::CONTENT_INLINE_CAP_BYTES;
use crate::drive_client::SharedDriveClient;

const DEFAULT_PARENT: &str = "root";

pub struct DriveUploadFile {
    id: ToolId,
    schema: Value,
    client: SharedDriveClient,
}

impl DriveUploadFile {
    pub fn new(client: SharedDriveClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for DriveUploadFile {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "drive.upload_file"
    }

    fn description(&self) -> &str {
        "Create a new Google Drive file with content. Input \
         is a JSON object with required `name`, `mime_type` \
         (the file's content type — e.g. `application/pdf`, \
         `text/plain`, `image/png`), `content_base64` \
         (base64-encoded file bytes; raw content capped at \
         10 MB), and optional `parent_folder_id` (default \
         `\"root\"`) and `description`. Returns `{id, name, \
         mime_type, size, parent_folder_id}` of the created \
         file. Requires Trusted-tier capability grant for \
         `drive.write`. For files above 10 MB, wait for the \
         Phase 130+ streaming substrate."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("drive.write").expect(
            "drive.write must parse — it is in KNOWN_BASES from Phase 129",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("drive.upload_file: {reason}"),
                });
            }
        };

        let metadata = build_metadata(&parsed);
        let path = "/files?uploadType=multipart&fields=id,name,mimeType,size,parents";

        let resp: Value = match self
            .client
            .post_multipart(path, metadata, parsed.content_bytes.clone(), &parsed.mime_type)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("drive.upload_file: API call failed: {e}"),
                });
            }
        };

        // Drive returns `size` as a string; parse to u64.
        let size = resp
            .get("size")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .map(|n| Value::Number(n.into()))
            .unwrap_or_else(|| {
                // Fall back to the uploaded byte count if
                // Drive didn't echo size in the response.
                Value::Number((parsed.content_bytes.len() as u64).into())
            });
        let parent_folder_id = resp
            .get("parents")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or(Value::Null);

        let output = json!({
            "id": resp.get("id").cloned().unwrap_or(Value::Null),
            "name": resp.get("name").cloned().unwrap_or(Value::Null),
            "mime_type": resp.get("mimeType").cloned().unwrap_or(Value::Null),
            "size": size,
            "parent_folder_id": parent_folder_id,
        });

        ToolOutcome::Completed {
            output,
            verified: Verification::Verified,
        }
    }
}

pub(crate) fn build_metadata(parsed: &ParsedInput) -> Value {
    let mut meta = serde_json::Map::new();
    meta.insert("name".to_string(), Value::String(parsed.name.clone()));
    meta.insert("mimeType".to_string(), Value::String(parsed.mime_type.clone()));
    meta.insert(
        "parents".to_string(),
        Value::Array(vec![Value::String(parsed.parent_folder_id.clone())]),
    );
    if let Some(ref d) = parsed.description {
        meta.insert("description".to_string(), Value::String(d.clone()));
    }
    Value::Object(meta)
}

#[derive(Debug)]
pub(crate) struct ParsedInput {
    pub(crate) name: String,
    pub(crate) mime_type: String,
    pub(crate) parent_folder_id: String,
    pub(crate) description: Option<String>,
    pub(crate) content_bytes: Vec<u8>,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| "input must include a `name` string field".to_string())?;
    if name.is_empty() {
        return Err("`name` must not be empty".to_string());
    }
    let mime_type = obj
        .get("mime_type")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| "input must include a `mime_type` string field".to_string())?;
    if mime_type.is_empty() {
        return Err("`mime_type` must not be empty".to_string());
    }
    let content_base64 = obj
        .get("content_base64")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "input must include a `content_base64` string field".to_string())?;
    let content_bytes = base64::engine::general_purpose::STANDARD
        .decode(content_base64.trim())
        .map_err(|e| format!("`content_base64` failed to decode: {e}"))?;
    if content_bytes.len() > CONTENT_INLINE_CAP_BYTES {
        return Err(format!(
            "decoded content size {} bytes exceeds inline cap {} bytes",
            content_bytes.len(),
            CONTENT_INLINE_CAP_BYTES
        ));
    }
    let parent_folder_id = match obj.get("parent_folder_id") {
        None => DEFAULT_PARENT.to_string(),
        Some(v) => v
            .as_str()
            .ok_or_else(|| "`parent_folder_id` must be a string".to_string())?
            .trim()
            .to_string(),
    };
    if parent_folder_id.is_empty() {
        return Err("`parent_folder_id` must not be empty".to_string());
    }
    let description = match obj.get("description") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(_) => return Err("`description` must be a string".to_string()),
    };
    Ok(ParsedInput {
        name: name.to_string(),
        mime_type: mime_type.to_string(),
        parent_folder_id,
        description,
        content_bytes,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "File display name. Required."
            },
            "mime_type": {
                "type": "string",
                "description": "Content type — e.g. `application/pdf`, `text/plain`, `image/png`. Required."
            },
            "content_base64": {
                "type": "string",
                "description": "Base64-encoded file content. Decoded size capped at 10 MB."
            },
            "parent_folder_id": {
                "type": "string",
                "default": DEFAULT_PARENT,
                "description": "Parent folder ID. Default `\"root\"`."
            },
            "description": {
                "type": ["string", "null"],
                "description": "Long-form description shown in Drive UI. Optional."
            }
        },
        "required": ["name", "mime_type", "content_base64"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn parse_input_accepts_minimal_required() {
        let p = parse_input(&json!({
            "name": "hello.txt",
            "mime_type": "text/plain",
            "content_base64": b64(b"hello"),
        }))
        .expect("parse");
        assert_eq!(p.name, "hello.txt");
        assert_eq!(p.mime_type, "text/plain");
        assert_eq!(p.content_bytes, b"hello");
        assert_eq!(p.parent_folder_id, DEFAULT_PARENT);
        assert!(p.description.is_none());
    }

    #[test]
    fn parse_input_rejects_missing_name() {
        let e = parse_input(&json!({
            "mime_type": "text/plain",
            "content_base64": b64(b"x"),
        }))
        .expect_err("must error");
        assert!(e.contains("name"), "{e}");
    }

    #[test]
    fn parse_input_rejects_missing_mime_type() {
        let e = parse_input(&json!({
            "name": "x",
            "content_base64": b64(b"x"),
        }))
        .expect_err("must error");
        assert!(e.contains("mime_type"), "{e}");
    }

    #[test]
    fn parse_input_rejects_missing_content_base64() {
        let e = parse_input(&json!({
            "name": "x",
            "mime_type": "text/plain",
        }))
        .expect_err("must error");
        assert!(e.contains("content_base64"), "{e}");
    }

    #[test]
    fn parse_input_rejects_malformed_base64() {
        let e = parse_input(&json!({
            "name": "x",
            "mime_type": "text/plain",
            "content_base64": "not-valid-base64!!!@@@",
        }))
        .expect_err("must error");
        assert!(e.contains("content_base64"), "{e}");
    }

    #[test]
    fn parse_input_rejects_content_above_cap() {
        // Build a 11 MB byte array; encode; expect rejection.
        let oversized = vec![0u8; CONTENT_INLINE_CAP_BYTES + 1];
        let e = parse_input(&json!({
            "name": "big.bin",
            "mime_type": "application/octet-stream",
            "content_base64": b64(&oversized),
        }))
        .expect_err("must error");
        assert!(e.contains("exceeds inline cap"), "{e}");
    }

    #[test]
    fn parse_input_accepts_content_exactly_at_cap() {
        let exactly = vec![0u8; CONTENT_INLINE_CAP_BYTES];
        let p = parse_input(&json!({
            "name": "max.bin",
            "mime_type": "application/octet-stream",
            "content_base64": b64(&exactly),
        }))
        .expect("parse");
        assert_eq!(p.content_bytes.len(), CONTENT_INLINE_CAP_BYTES);
    }

    #[test]
    fn parse_input_rejects_empty_parent_folder_id() {
        let e = parse_input(&json!({
            "name": "x",
            "mime_type": "text/plain",
            "content_base64": b64(b"x"),
            "parent_folder_id": "",
        }))
        .expect_err("must error");
        assert!(e.contains("parent_folder_id"), "{e}");
    }

    #[test]
    fn parse_input_accepts_optional_description() {
        let p = parse_input(&json!({
            "name": "x",
            "mime_type": "text/plain",
            "content_base64": b64(b"x"),
            "description": "Operator's notes about this file",
        }))
        .expect("parse");
        assert_eq!(p.description.as_deref(), Some("Operator's notes about this file"));
    }

    #[test]
    fn parse_input_empty_description_treated_as_none() {
        let p = parse_input(&json!({
            "name": "x",
            "mime_type": "text/plain",
            "content_base64": b64(b"x"),
            "description": "   ",
        }))
        .expect("parse");
        assert!(p.description.is_none());
    }

    #[test]
    fn build_metadata_emits_required_fields() {
        let p = parse_input(&json!({
            "name": "Q3.pdf",
            "mime_type": "application/pdf",
            "content_base64": b64(b"PDF bytes"),
        }))
        .unwrap();
        let meta = build_metadata(&p);
        assert_eq!(meta["name"], "Q3.pdf");
        assert_eq!(meta["mimeType"], "application/pdf");
        assert_eq!(meta["parents"][0], "root");
        // No description supplied → field omitted.
        assert!(meta.get("description").is_none());
    }

    #[test]
    fn build_metadata_includes_description_when_supplied() {
        let p = parse_input(&json!({
            "name": "x",
            "mime_type": "text/plain",
            "content_base64": b64(b"x"),
            "description": "Notes",
        }))
        .unwrap();
        let meta = build_metadata(&p);
        assert_eq!(meta["description"], "Notes");
    }

    #[test]
    fn build_metadata_uses_explicit_parent() {
        let p = parse_input(&json!({
            "name": "x",
            "mime_type": "text/plain",
            "content_base64": b64(b"x"),
            "parent_folder_id": "1XyZ",
        }))
        .unwrap();
        let meta = build_metadata(&p);
        assert_eq!(meta["parents"][0], "1XyZ");
    }

    #[test]
    fn input_schema_declares_three_required_fields() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        let req_strs: Vec<&str> = req.iter().filter_map(|v| v.as_str()).collect();
        assert!(req_strs.contains(&"name"));
        assert!(req_strs.contains(&"mime_type"));
        assert!(req_strs.contains(&"content_base64"));
        assert_eq!(schema["additionalProperties"], false);
    }

    fn make_tool() -> DriveUploadFile {
        use crate::{DriveClient, OAuthConfig, TokenSet};
        use std::sync::Arc;
        let client = Arc::new(DriveClient::new(
            reqwest::Client::new(),
            OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            std::path::PathBuf::from("/tmp/unused"),
        ));
        DriveUploadFile::new(client)
    }

    #[test]
    fn required_scope_is_drive_write() {
        assert_eq!(
            make_tool().required_scope(&json!({})).to_string(),
            "drive.write"
        );
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "drive.upload_file");
    }
}

//! `drive.download_file` — fetch file content.
//!
//! Phase 129 Task 8. Fifth Drive tool. Read content as
//! base64-encoded bytes with a 10 MB inline cap (per
//! Phase 129 Q3a).
//!
//! ## Two code paths
//!
//! Drive's wire protocol splits content download by file
//! type:
//!
//! - **Regular files** (PDFs, images, binaries, etc.) use
//!   `GET /files/{id}?alt=media` and return the file bytes
//!   directly.
//! - **Google-native types** (Docs / Sheets / Slides /
//!   etc.) MUST be exported via
//!   `GET /files/{id}/export?mimeType=<export-mime>`
//!   because they have no canonical binary representation.
//!
//! This tool detects the file's mime type via a pre-flight
//! metadata fetch, then dispatches to the right endpoint.
//! Operators can override the export target via the
//! optional `export_mime_type` input field.
//!
//! ## Size cap
//!
//! Per Phase 129 Q3a Recommended, content above 10 MB is
//! NOT transferred inline — the tool returns
//! `content_base64: null` and `content_truncated: true`
//! with the metadata so the LLM can decide how to proceed
//! (e.g., narrow the operator's query to specific
//! fragments via `drive.get_metadata` first, or skip).
//! Streaming via `ToolEventPayload::OutputChunk` is the
//! natural Phase 130+ follow-up; the cap is operator-
//! visible in the tool's description.

use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use super::CONTENT_INLINE_CAP_BYTES;
use crate::drive_client::SharedDriveClient;

const GOOGLE_NATIVE_PREFIX: &str = "application/vnd.google-apps.";

pub struct DriveDownloadFile {
    id: ToolId,
    schema: Value,
    client: SharedDriveClient,
}

impl DriveDownloadFile {
    pub fn new(client: SharedDriveClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for DriveDownloadFile {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "drive.download_file"
    }

    fn description(&self) -> &str {
        "Download a Google Drive file's content as \
         base64-encoded bytes. Input is a JSON object with \
         a required `file_id` and an optional \
         `export_mime_type` (for Google-native types like \
         Docs / Sheets / Slides — defaults to \
         `application/pdf` for Docs/Slides and `text/csv` \
         for Sheets when omitted). Content is inline up to \
         10 MB; above the cap the response returns \
         metadata-only with `content_truncated: true` and a \
         null `content_base64`. Returns `{file_id, name, \
         mime_type, size, content_base64, \
         content_truncated, exported_as}` — `exported_as` \
         is the actual export mime for Google-native files, \
         null for regular files."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("drive.read").expect(
            "drive.read must parse — it is in KNOWN_BASES from Phase 129",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("drive.download_file: {reason}"),
                });
            }
        };

        // Pre-flight metadata fetch — we need size + mime
        // before deciding which content endpoint to hit.
        let meta_path = format!(
            "/files/{}",
            super::drive_urlencode(&parsed.file_id)
        );
        let meta_query: Vec<(&str, String)> = vec![(
            "fields",
            "id,name,mimeType,size".to_string(),
        )];
        let metadata: Value = match self.client.get_json(&meta_path, &meta_query).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("drive.download_file: metadata fetch failed: {e}"),
                });
            }
        };

        let file_id = metadata
            .get("id")
            .cloned()
            .unwrap_or_else(|| Value::String(parsed.file_id.clone()));
        let name = metadata.get("name").cloned().unwrap_or(Value::Null);
        let mime_type_str = metadata
            .get("mimeType")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mime_type = if mime_type_str.is_empty() {
            Value::Null
        } else {
            Value::String(mime_type_str.clone())
        };
        let declared_size = metadata
            .get("size")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok());

        // Pre-flight size cap: regular files report `size`
        // up front; reject the download early without
        // hitting the media endpoint.
        if let Some(size) = declared_size {
            if (size as usize) > CONTENT_INLINE_CAP_BYTES {
                return ToolOutcome::Completed {
                    output: build_truncated_output(
                        &file_id,
                        &name,
                        &mime_type,
                        Some(size),
                        None,
                        Some(size as usize),
                    ),
                    verified: Verification::NotApplicable,
                };
            }
        }

        let is_google_native = mime_type_str.starts_with(GOOGLE_NATIVE_PREFIX);
        let (content_path, content_query, exported_as) = if is_google_native {
            let target_mime = parsed
                .export_mime_type
                .clone()
                .unwrap_or_else(|| default_export_mime_for(&mime_type_str).to_string());
            let path = format!(
                "/files/{}/export",
                super::drive_urlencode(&parsed.file_id)
            );
            let query = vec![("mimeType", target_mime.clone())];
            (path, query, Some(target_mime))
        } else {
            let path = format!(
                "/files/{}",
                super::drive_urlencode(&parsed.file_id)
            );
            (path, Vec::new(), None)
        };

        // Fetch the bytes. Regular files use `get_media`
        // (auto-adds `alt=media`). Google-native types use
        // `get_bytes` against the `/export` endpoint
        // because `alt=media` would conflict with the
        // `mimeType` export param.
        let fetch_result = if is_google_native {
            self.client.get_bytes(&content_path, &content_query).await
        } else {
            self.client.get_media(&content_path, &content_query).await
        };
        let bytes: Vec<u8> = match fetch_result {
            Ok(b) => b,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("drive.download_file: content fetch failed: {e}"),
                });
            }
        };

        // Post-flight cap check (for Google-native types
        // where declared_size was None).
        if bytes.len() > CONTENT_INLINE_CAP_BYTES {
            return ToolOutcome::Completed {
                output: build_truncated_output(
                    &file_id,
                    &name,
                    &mime_type,
                    declared_size,
                    exported_as.as_deref(),
                    Some(bytes.len()),
                ),
                verified: Verification::NotApplicable,
            };
        }

        let content_base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let size_for_output: u64 = bytes.len() as u64;

        let output = json!({
            "file_id": file_id,
            "name": name,
            "mime_type": mime_type,
            "size": size_for_output,
            "content_base64": content_base64,
            "content_truncated": false,
            "exported_as": exported_as.map(Value::String).unwrap_or(Value::Null),
        });

        ToolOutcome::Completed {
            output,
            verified: Verification::NotApplicable,
        }
    }
}

pub(crate) fn build_truncated_output(
    file_id: &Value,
    name: &Value,
    mime_type: &Value,
    declared_size: Option<u64>,
    exported_as: Option<&str>,
    actual_size: Option<usize>,
) -> Value {
    let size = declared_size
        .map(|n| Value::Number(n.into()))
        .or_else(|| actual_size.map(|n| Value::Number((n as u64).into())))
        .unwrap_or(Value::Null);
    json!({
        "file_id": file_id.clone(),
        "name": name.clone(),
        "mime_type": mime_type.clone(),
        "size": size,
        "content_base64": Value::Null,
        "content_truncated": true,
        "exported_as": exported_as.map(|s| Value::String(s.to_string())).unwrap_or(Value::Null),
        "error": format!(
            "file size {} exceeds inline cap {} bytes; use a narrower query or wait for Phase 130+ streaming substrate",
            actual_size.or(declared_size.map(|n| n as usize)).unwrap_or(0),
            CONTENT_INLINE_CAP_BYTES,
        ),
    })
}

/// Sensible export-mime defaults for the common Google-
/// native types. Operators can override via the
/// `export_mime_type` input.
pub(crate) fn default_export_mime_for(google_mime: &str) -> &'static str {
    match google_mime {
        "application/vnd.google-apps.document" => "application/pdf",
        "application/vnd.google-apps.spreadsheet" => "text/csv",
        "application/vnd.google-apps.presentation" => "application/pdf",
        "application/vnd.google-apps.drawing" => "image/png",
        _ => "application/pdf",
    }
}

#[derive(Debug)]
struct ParsedInput {
    file_id: String,
    export_mime_type: Option<String>,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let file_id = obj
        .get("file_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| "input must include a `file_id` string field".to_string())?;
    if file_id.is_empty() {
        return Err("`file_id` must not be empty".to_string());
    }
    let export_mime_type = match obj.get("export_mime_type") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(_) => return Err("`export_mime_type` must be a string".to_string()),
    };
    Ok(ParsedInput {
        file_id: file_id.to_string(),
        export_mime_type,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "file_id": {
                "type": "string",
                "description": "Drive file ID. Required."
            },
            "export_mime_type": {
                "type": ["string", "null"],
                "description": "Export target mime type for Google-native files (Docs / Sheets / Slides). Ignored for regular files. Defaults: Docs+Slides → application/pdf, Sheets → text/csv, Drawings → image/png."
            }
        },
        "required": ["file_id"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_input_accepts_file_id_only() {
        let p = parse_input(&json!({"file_id": "abc"})).expect("parse");
        assert_eq!(p.file_id, "abc");
        assert!(p.export_mime_type.is_none());
    }

    #[test]
    fn parse_input_accepts_export_mime_type() {
        let p = parse_input(&json!({
            "file_id": "abc",
            "export_mime_type": "application/pdf"
        }))
        .expect("parse");
        assert_eq!(p.export_mime_type.as_deref(), Some("application/pdf"));
    }

    #[test]
    fn parse_input_empty_export_mime_treated_as_none() {
        let p = parse_input(&json!({
            "file_id": "abc",
            "export_mime_type": "   "
        }))
        .expect("parse");
        assert!(p.export_mime_type.is_none());
    }

    #[test]
    fn parse_input_rejects_missing_file_id() {
        let e = parse_input(&json!({})).expect_err("must error");
        assert!(e.contains("file_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_file_id() {
        let e = parse_input(&json!({"file_id": "   "})).expect_err("must error");
        assert!(e.contains("file_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_string_export_mime() {
        let e = parse_input(&json!({"file_id": "x", "export_mime_type": 42}))
            .expect_err("must error");
        assert!(e.contains("export_mime_type"), "{e}");
    }

    #[test]
    fn default_export_mime_for_docs_is_pdf() {
        assert_eq!(
            default_export_mime_for("application/vnd.google-apps.document"),
            "application/pdf"
        );
    }

    #[test]
    fn default_export_mime_for_sheets_is_csv() {
        assert_eq!(
            default_export_mime_for("application/vnd.google-apps.spreadsheet"),
            "text/csv"
        );
    }

    #[test]
    fn default_export_mime_for_slides_is_pdf() {
        assert_eq!(
            default_export_mime_for("application/vnd.google-apps.presentation"),
            "application/pdf"
        );
    }

    #[test]
    fn default_export_mime_for_drawing_is_png() {
        assert_eq!(
            default_export_mime_for("application/vnd.google-apps.drawing"),
            "image/png"
        );
    }

    #[test]
    fn default_export_mime_for_unknown_google_native_is_pdf() {
        assert_eq!(
            default_export_mime_for("application/vnd.google-apps.script"),
            "application/pdf"
        );
    }

    #[test]
    fn build_truncated_output_carries_size_and_error() {
        let out = build_truncated_output(
            &json!("file-abc"),
            &json!("Big.pdf"),
            &json!("application/pdf"),
            Some(20_000_000),
            None,
            Some(20_000_000),
        );
        assert_eq!(out["file_id"], "file-abc");
        assert_eq!(out["size"], 20_000_000);
        assert!(out["content_base64"].is_null());
        assert_eq!(out["content_truncated"], true);
        assert!(out["error"].as_str().unwrap().contains("exceeds"));
    }

    #[test]
    fn build_truncated_output_handles_no_size() {
        let out = build_truncated_output(
            &json!("x"),
            &json!("y"),
            &json!("z"),
            None,
            None,
            None,
        );
        assert!(out["size"].is_null());
        assert_eq!(out["content_truncated"], true);
    }

    #[test]
    fn input_schema_declares_file_id_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 1);
        assert_eq!(req[0], "file_id");
        assert_eq!(schema["additionalProperties"], false);
    }

    fn make_tool() -> DriveDownloadFile {
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
        DriveDownloadFile::new(client)
    }

    #[test]
    fn required_scope_is_drive_read() {
        assert_eq!(
            make_tool().required_scope(&json!({})).to_string(),
            "drive.read",
            "download_file is a READ operation"
        );
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "drive.download_file");
    }
}

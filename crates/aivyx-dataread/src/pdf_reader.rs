//! `data.pdf` — extract the text layer from a PDF.
//!
//! Chapter Sheaf (SH.3). The third structured-data reader, reusing the
//! [`crate::sandbox::ReaderSandbox`] spine (gated on the existing
//! `fs.read` capability, no new base). It is the readability pass for
//! documents — the file-content analogue of `web.extract` for PDFs:
//! `fs.read` hands the LLM a wall of binary, this hands it the text.
//!
//! **Text layer only — no OCR.** A scanned/image-only PDF has no text
//! layer and yields empty (or near-empty) output; that is reported,
//! not faked.
//!
//! ## Tool surface
//!
//! - `data.pdf` — `{path: string (required), max_chars?: number}` →
//!   `{path, text, pages, char_count, truncated}`. Text is extracted
//!   in-memory from the sandbox-read bytes (no temp files) and capped.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::sandbox::ReaderSandbox;

/// Default + hard ceiling on extracted characters (a PDF can hold a
/// lot of text; bound the context). `max_chars` may lower the default
/// but never raise it past the ceiling.
const DEFAULT_MAX_CHARS: usize = 100_000;
const HARD_MAX_CHARS: usize = 1_000_000;

pub struct DataPdfTool {
    id: ToolId,
    schema: Value,
    sandbox: ReaderSandbox,
}

impl DataPdfTool {
    pub fn new(sandbox: ReaderSandbox) -> Self {
        Self {
            id: ToolId::new(),
            schema: pdf_schema(),
            sandbox,
        }
    }
}

#[async_trait]
impl Tool for DataPdfTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "data.pdf"
    }
    fn description(&self) -> &str {
        "Extract the text layer from a PDF under the agent's sandbox \
         root. Input: `{path: string (required), max_chars: number \
         (optional)}`. Returns `{path, text, pages, char_count, \
         truncated}`. Text layer only — a scanned/image-only PDF has \
         no text and yields empty output (no OCR). fs.read only gives \
         raw binary; use this for readable PDF text. Scope: `fs.read` \
         (same sandbox as fs.read)."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, input: &Value) -> Scope {
        self.sandbox.scope_for(input)
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let max_chars = match parse_max_chars(&input) {
            Ok(n) => n,
            Err(e) => return fail(self.id, format!("data.pdf: {e}")),
        };

        let file = match self.sandbox.read_guarded(&input, self.id) {
            Ok(f) => f,
            Err(outcome) => return outcome,
        };

        match parse_pdf(&file.bytes, max_chars) {
            Ok(parsed) => ToolOutcome::Completed {
                output: json!({
                    "path": file.path.display().to_string(),
                    "text": parsed.text,
                    "pages": parsed.pages,
                    "char_count": parsed.char_count,
                    "truncated": file.truncated || parsed.text_truncated,
                }),
                verified: Verification::NotApplicable,
            },
            Err(e) => {
                let hint = if file.truncated {
                    " (the file exceeded the read cap and was truncated)"
                } else {
                    ""
                };
                fail(self.id, format!("data.pdf: {e}{hint}"))
            }
        }
    }
}

fn pdf_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "minLength": 1, "description": "PDF path under the sandbox root." },
            "max_chars": { "type": "number", "description": "Cap on extracted characters." }
        },
        "required": ["path"],
        "additionalProperties": false
    })
}

// =====================================================================
// Pure parsing
// =====================================================================

pub struct PdfData {
    pub text: String,
    /// Page count (best-effort via the PDF structure; 0 if unreadable).
    pub pages: usize,
    pub char_count: usize,
    pub text_truncated: bool,
}

/// Extract text + page count from PDF bytes, capping the text.
pub fn parse_pdf(bytes: &[u8], max_chars: usize) -> Result<PdfData, String> {
    let raw = pdf_extract::extract_text_from_mem(bytes)
        .map_err(|e| format!("could not extract text (not a readable PDF?): {e}"))?;

    // Page count is best-effort: a successful text extract with an
    // unreadable structure still returns text, just `pages: 0`.
    let pages = lopdf::Document::load_mem(bytes)
        .map(|doc| doc.get_pages().len())
        .unwrap_or(0);

    let (text, text_truncated) = cap_text(raw, max_chars);
    let char_count = text.chars().count();
    Ok(PdfData { text, pages, char_count, text_truncated })
}

/// Truncate extracted text to `max` characters at a char boundary.
fn cap_text(s: String, max: usize) -> (String, bool) {
    if s.chars().count() <= max {
        return (s, false);
    }
    (s.chars().take(max).collect(), true)
}

fn parse_max_chars(input: &Value) -> Result<usize, String> {
    match input.get("max_chars") {
        None | Some(Value::Null) => Ok(DEFAULT_MAX_CHARS),
        Some(v) => {
            let n = v
                .as_u64()
                .ok_or_else(|| "`max_chars` must be a non-negative integer".to_string())?;
            Ok((n as usize).min(HARD_MAX_CHARS))
        }
    }
}

fn fail(tool: ToolId, detail: String) -> ToolOutcome {
    ToolOutcome::Failed(aivyx_core::AivyxError::Tool { tool, detail })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cap_text_truncates() {
        let (t, trunc) = cap_text("héllo wörld".to_string(), 5);
        assert_eq!(t.chars().count(), 5);
        assert!(trunc);
        let (t2, trunc2) = cap_text("short".to_string(), 100);
        assert_eq!(t2, "short");
        assert!(!trunc2);
    }

    #[test]
    fn non_pdf_bytes_are_rejected() {
        // pdf-extract returns an error (not a panic) on garbage input.
        assert!(parse_pdf(b"definitely not a pdf", 1000).is_err());
    }

    #[test]
    fn max_chars_parsing_and_ceiling() {
        assert_eq!(parse_max_chars(&json!({})).unwrap(), DEFAULT_MAX_CHARS);
        assert_eq!(parse_max_chars(&json!({"max_chars": 10})).unwrap(), 10);
        assert_eq!(
            parse_max_chars(&json!({"max_chars": 99_999_999})).unwrap(),
            HARD_MAX_CHARS
        );
        assert!(parse_max_chars(&json!({"max_chars": "lots"})).is_err());
    }

    #[test]
    fn tool_metadata_is_sound() {
        let dir = std::env::temp_dir();
        let sb = ReaderSandbox::new(&dir).unwrap();
        let tool = DataPdfTool::new(sb);
        assert_eq!(tool.name(), "data.pdf");
        assert!(!tool.description().is_empty());
        assert_eq!(tool.input_schema()["type"], "object");
        assert_eq!(tool.required_scope(&json!({"path": "x.pdf"})).base(), "fs.read");
    }
}

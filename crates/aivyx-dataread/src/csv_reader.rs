//! `data.csv` — read a delimited-text file into structured rows.
//!
//! Chapter Sheaf (SH.1). The first structured-data reader, and the
//! one that carries the chapter's sandbox-reuse spine
//! ([`crate::sandbox::ReaderSandbox`]). It gates on the existing
//! `fs.read` capability (no new base) and returns the column structure
//! the LLM would otherwise re-derive token-by-token from raw bytes.
//!
//! ## Tool surface
//!
//! - `data.csv` — `{path: string (required), delimiter?: string
//!   (single char, default ","), has_headers?: bool (default true),
//!   max_rows?: number}` → `{path, headers, rows, row_count,
//!   truncated}`. `headers` is `null` when `has_headers` is false.
//!   Rows past `max_rows` (or the hard cap) and oversize cells are
//!   truncated, with `truncated: true` flagged.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::sandbox::{cap_cell, ReaderSandbox};

/// Default and hard-ceiling row caps. `max_rows` may lower the default
/// but never raise it past the ceiling (bounding context + memory).
const DEFAULT_MAX_ROWS: usize = 1_000;
const HARD_MAX_ROWS: usize = 100_000;

pub struct DataCsvTool {
    id: ToolId,
    schema: Value,
    sandbox: ReaderSandbox,
}

impl DataCsvTool {
    pub fn new(sandbox: ReaderSandbox) -> Self {
        Self {
            id: ToolId::new(),
            schema: csv_schema(),
            sandbox,
        }
    }
}

#[async_trait]
impl Tool for DataCsvTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "data.csv"
    }
    // Chapter Bulwark — parsed file content is untrusted external content.
    fn output_is_untrusted(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "Read a CSV / delimited-text file from under the agent's \
         sandbox root into structured rows. Input: `{path: string \
         (required), delimiter: string (optional single char, default \
         \",\"; use \"\\t\" for TSV), has_headers: bool (optional, \
         default true), max_rows: number (optional)}`. Returns \
         `{path, headers, rows, row_count, truncated}` (`headers` is \
         null when has_headers is false). Prefer this over fs.read for \
         CSV data. Scope: `fs.read` (same sandbox as fs.read)."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, input: &Value) -> Scope {
        self.sandbox.scope_for(input)
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let delimiter = match parse_delimiter(&input) {
            Ok(d) => d,
            Err(e) => return fail(self.id, e),
        };
        let has_headers = input
            .get("has_headers")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let max_rows = match parse_max_rows(&input) {
            Ok(n) => n,
            Err(e) => return fail(self.id, e),
        };

        let file = match self.sandbox.read_guarded(&input, self.id) {
            Ok(f) => f,
            Err(outcome) => return outcome,
        };

        match parse_csv(&file.bytes, delimiter, has_headers, max_rows) {
            Ok(parsed) => ToolOutcome::Completed {
                output: json!({
                    "path": file.path.display().to_string(),
                    "headers": parsed.headers,
                    "rows": parsed.rows,
                    "row_count": parsed.row_count,
                    // Truncated if the bytes were capped OR rows were capped.
                    "truncated": file.truncated || parsed.rows_truncated,
                }),
                verified: Verification::NotApplicable,
            },
            Err(e) => fail(self.id, format!("data.csv: {e}")),
        }
    }
}

fn csv_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "minLength": 1, "description": "File path under the sandbox root." },
            "delimiter": { "type": "string", "description": "Single-character field delimiter (default \",\"; \"\\t\" for TSV)." },
            "has_headers": { "type": "boolean", "description": "Treat the first row as headers (default true)." },
            "max_rows": { "type": "number", "description": "Cap on data rows returned." }
        },
        "required": ["path"],
        "additionalProperties": false
    })
}

// =====================================================================
// Pure parsing
// =====================================================================

/// Structured result of a CSV parse.
pub struct CsvData {
    /// Header row when `has_headers`, else `None`.
    pub headers: Option<Vec<String>>,
    /// Data rows (each a vector of cell strings).
    pub rows: Vec<Vec<String>>,
    /// Number of data rows returned (== `rows.len()`).
    pub row_count: usize,
    /// `true` if rows beyond `max_rows` were dropped.
    pub rows_truncated: bool,
}

/// Parse CSV bytes into rows. `delimiter` is a single byte; `max_rows`
/// caps the data rows (cells are capped to [`MAX_CELL_CHARS`]).
pub fn parse_csv(
    bytes: &[u8],
    delimiter: u8,
    has_headers: bool,
    max_rows: usize,
) -> Result<CsvData, String> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(has_headers)
        // Flexible: don't error on ragged rows — the LLM should see the
        // data as-is rather than have a read fail on one bad line.
        .flexible(true)
        .from_reader(bytes);

    let headers = if has_headers {
        let hdr = rdr
            .headers()
            .map_err(|e| format!("header parse failed: {e}"))?;
        Some(hdr.iter().map(cap_cell).collect())
    } else {
        None
    };

    let mut rows = Vec::new();
    let mut rows_truncated = false;
    for result in rdr.records() {
        if rows.len() >= max_rows {
            rows_truncated = true;
            break;
        }
        let record = result.map_err(|e| format!("row parse failed: {e}"))?;
        rows.push(record.iter().map(cap_cell).collect());
    }

    let row_count = rows.len();
    Ok(CsvData { headers, rows, row_count, rows_truncated })
}

fn parse_delimiter(input: &Value) -> Result<u8, String> {
    match input.get("delimiter") {
        None | Some(Value::Null) => Ok(b','),
        Some(Value::String(s)) => {
            // Accept a literal tab, the escape "\t", or a single byte.
            let resolved = match s.as_str() {
                "\\t" | "\t" => "\t",
                other => other,
            };
            let b = resolved.as_bytes();
            if b.len() == 1 {
                Ok(b[0])
            } else {
                Err(format!("`delimiter` must be a single character, got {s:?}"))
            }
        }
        Some(_) => Err("`delimiter` must be a string".to_string()),
    }
}

fn parse_max_rows(input: &Value) -> Result<usize, String> {
    match input.get("max_rows") {
        None | Some(Value::Null) => Ok(DEFAULT_MAX_ROWS),
        Some(v) => {
            let n = v
                .as_u64()
                .ok_or_else(|| "`max_rows` must be a non-negative integer".to_string())?;
            Ok((n as usize).min(HARD_MAX_ROWS))
        }
    }
}

fn fail(tool: ToolId, detail: String) -> ToolOutcome {
    ToolOutcome::Failed(aivyx_core::AivyxError::Tool { tool, detail })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::MAX_CELL_CHARS;

    fn rows_of(data: &CsvData) -> Vec<Vec<&str>> {
        data.rows
            .iter()
            .map(|r| r.iter().map(String::as_str).collect())
            .collect()
    }

    #[test]
    fn parses_headers_and_rows() {
        let csv = b"name,age\nAda,36\nGrace,45\n";
        let d = parse_csv(csv, b',', true, 1000).unwrap();
        assert_eq!(d.headers.as_ref().unwrap(), &["name", "age"]);
        assert_eq!(rows_of(&d), vec![vec!["Ada", "36"], vec!["Grace", "45"]]);
        assert_eq!(d.row_count, 2);
        assert!(!d.rows_truncated);
    }

    #[test]
    fn no_headers_returns_null_headers() {
        let csv = b"1,2,3\n4,5,6\n";
        let d = parse_csv(csv, b',', false, 1000).unwrap();
        assert!(d.headers.is_none());
        assert_eq!(d.row_count, 2);
    }

    #[test]
    fn respects_quoting_and_embedded_commas() {
        let csv = b"a,b\n\"x,y\",\"line\nbreak\"\n";
        let d = parse_csv(csv, b',', true, 1000).unwrap();
        assert_eq!(rows_of(&d), vec![vec!["x,y", "line\nbreak"]]);
    }

    #[test]
    fn tsv_via_tab_delimiter() {
        let csv = b"a\tb\n1\t2\n";
        let d = parse_csv(csv, b'\t', true, 1000).unwrap();
        assert_eq!(d.headers.as_ref().unwrap(), &["a", "b"]);
        assert_eq!(rows_of(&d), vec![vec!["1", "2"]]);
    }

    #[test]
    fn ragged_rows_are_tolerated() {
        let csv = b"a,b,c\n1,2\n3,4,5,6\n";
        let d = parse_csv(csv, b',', true, 1000).unwrap();
        assert_eq!(d.row_count, 2);
        assert_eq!(rows_of(&d)[0], vec!["1", "2"]);
        assert_eq!(rows_of(&d)[1], vec!["3", "4", "5", "6"]);
    }

    #[test]
    fn max_rows_truncates_and_flags() {
        let csv = b"h\n1\n2\n3\n4\n";
        let d = parse_csv(csv, b',', true, 2).unwrap();
        assert_eq!(d.row_count, 2);
        assert!(d.rows_truncated);
    }

    #[test]
    fn oversize_cell_is_capped() {
        let big = "x".repeat(MAX_CELL_CHARS + 50);
        let csv = format!("h\n{big}\n");
        let d = parse_csv(csv.as_bytes(), b',', true, 1000).unwrap();
        let cell = &d.rows[0][0];
        assert!(cell.chars().count() <= MAX_CELL_CHARS + 1); // +1 for the ellipsis
        assert!(cell.ends_with('…'));
    }

    // ---- input helpers ------------------------------------------

    #[test]
    fn delimiter_parsing() {
        assert_eq!(parse_delimiter(&json!({})).unwrap(), b',');
        assert_eq!(parse_delimiter(&json!({"delimiter": ";"})).unwrap(), b';');
        assert_eq!(parse_delimiter(&json!({"delimiter": "\\t"})).unwrap(), b'\t');
        assert!(parse_delimiter(&json!({"delimiter": "ab"})).is_err());
    }

    #[test]
    fn max_rows_parsing_and_ceiling() {
        assert_eq!(parse_max_rows(&json!({})).unwrap(), DEFAULT_MAX_ROWS);
        assert_eq!(parse_max_rows(&json!({"max_rows": 5})).unwrap(), 5);
        assert_eq!(parse_max_rows(&json!({"max_rows": 10_000_000})).unwrap(), HARD_MAX_ROWS);
        assert!(parse_max_rows(&json!({"max_rows": "lots"})).is_err());
    }

    // ---- tool wiring --------------------------------------------

    #[test]
    fn tool_metadata_is_sound() {
        let dir = std::env::temp_dir();
        let sb = ReaderSandbox::new(&dir).unwrap();
        let tool = DataCsvTool::new(sb);
        assert_eq!(tool.name(), "data.csv");
        assert!(!tool.description().is_empty());
        assert_eq!(tool.input_schema()["type"], "object");
        // Gated by fs.read, not a new base.
        assert_eq!(
            tool.required_scope(&json!({"path": "x.csv"})).base(),
            "fs.read"
        );
    }
}

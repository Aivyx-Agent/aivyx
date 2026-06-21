//! `data.xlsx` — read a spreadsheet into structured rows.
//!
//! Chapter Sheaf (SH.2). The second structured-data reader, reusing
//! the [`crate::sandbox::ReaderSandbox`] spine (so it gates on the
//! existing `fs.read` capability, no new base). Unlike `data.csv`,
//! the `.xlsx` format is a binary zip+XML container `fs.read` cannot
//! expose at all — `calamine` parses it in-memory from the
//! sandbox-read bytes (no temp files).
//!
//! ## Tool surface
//!
//! - `data.xlsx` — `{path: string (required), sheet?: string (default
//!   first sheet), has_headers?: bool (default true), max_rows?:
//!   number}` → `{path, sheet, sheet_names, headers, rows, row_count,
//!   truncated}`. Cells are stringified (numbers, dates, booleans,
//!   text); `headers` is `null` when `has_headers` is false.

use std::io::Cursor;

use async_trait::async_trait;
use calamine::{open_workbook_from_rs, Data, Range, Reader, Xlsx};
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::sandbox::{cap_cell, ReaderSandbox};

const DEFAULT_MAX_ROWS: usize = 1_000;
const HARD_MAX_ROWS: usize = 100_000;

pub struct DataXlsxTool {
    id: ToolId,
    schema: Value,
    sandbox: ReaderSandbox,
}

impl DataXlsxTool {
    pub fn new(sandbox: ReaderSandbox) -> Self {
        Self {
            id: ToolId::new(),
            schema: xlsx_schema(),
            sandbox,
        }
    }
}

#[async_trait]
impl Tool for DataXlsxTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "data.xlsx"
    }
    fn description(&self) -> &str {
        "Read an .xlsx spreadsheet from under the agent's sandbox root \
         into structured rows. Input: `{path: string (required), \
         sheet: string (optional, default first sheet), has_headers: \
         bool (optional, default true), max_rows: number (optional)}`. \
         Returns `{path, sheet, sheet_names, headers, rows, row_count, \
         truncated}` with cells stringified. fs.read cannot expose the \
         binary .xlsx format — use this instead. Scope: `fs.read` \
         (same sandbox as fs.read)."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, input: &Value) -> Scope {
        self.sandbox.scope_for(input)
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let sheet = match input.get("sheet") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
            Some(Value::String(_)) => None,
            Some(_) => return fail(self.id, "data.xlsx: `sheet` must be a string".to_string()),
        };
        let has_headers = input
            .get("has_headers")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let max_rows = match parse_max_rows(&input) {
            Ok(n) => n,
            Err(e) => return fail(self.id, format!("data.xlsx: {e}")),
        };

        let file = match self.sandbox.read_guarded(&input, self.id) {
            Ok(f) => f,
            Err(outcome) => return outcome,
        };

        match parse_xlsx(&file.bytes, sheet.as_deref(), has_headers, max_rows) {
            Ok(parsed) => ToolOutcome::Completed {
                output: json!({
                    "path": file.path.display().to_string(),
                    "sheet": parsed.sheet,
                    "sheet_names": parsed.sheet_names,
                    "headers": parsed.headers,
                    "rows": parsed.rows,
                    "row_count": parsed.row_count,
                    "truncated": file.truncated || parsed.rows_truncated,
                }),
                verified: Verification::NotApplicable,
            },
            Err(e) => {
                // A truncated read corrupts the zip container; say so.
                let hint = if file.truncated {
                    " (the file exceeded the read cap and was truncated)"
                } else {
                    ""
                };
                fail(self.id, format!("data.xlsx: {e}{hint}"))
            }
        }
    }
}

fn xlsx_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "minLength": 1, "description": "Spreadsheet path under the sandbox root." },
            "sheet": { "type": "string", "description": "Sheet name (default: the first sheet)." },
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

pub struct XlsxData {
    /// The sheet actually read.
    pub sheet: String,
    /// All sheet names in the workbook (so the LLM can re-query).
    pub sheet_names: Vec<String>,
    pub headers: Option<Vec<String>>,
    pub rows: Vec<Vec<String>>,
    pub row_count: usize,
    pub rows_truncated: bool,
}

/// Parse xlsx bytes, selecting `sheet` (or the first sheet).
pub fn parse_xlsx(
    bytes: &[u8],
    sheet: Option<&str>,
    has_headers: bool,
    max_rows: usize,
) -> Result<XlsxData, String> {
    let mut workbook: Xlsx<_> = open_workbook_from_rs(Cursor::new(bytes))
        .map_err(|e| format!("not a readable .xlsx workbook: {e}"))?;
    let sheet_names = workbook.sheet_names().to_vec();
    if sheet_names.is_empty() {
        return Err("workbook has no sheets".to_string());
    }
    let target = match sheet {
        Some(name) => {
            if !sheet_names.iter().any(|s| s == name) {
                return Err(format!(
                    "sheet {name:?} not found; available: {sheet_names:?}"
                ));
            }
            name.to_string()
        }
        None => sheet_names[0].clone(),
    };
    let range = workbook
        .worksheet_range(&target)
        .map_err(|e| format!("cannot read sheet {target:?}: {e}"))?;

    let (headers, rows, rows_truncated) = range_to_table(&range, has_headers, max_rows);
    let row_count = rows.len();
    Ok(XlsxData {
        sheet: target,
        sheet_names,
        headers,
        rows,
        row_count,
        rows_truncated,
    })
}

/// Transform a calamine [`Range`] into header + capped string rows.
/// Factored out so it can be unit-tested without a binary fixture.
fn range_to_table(
    range: &Range<Data>,
    has_headers: bool,
    max_rows: usize,
) -> (Option<Vec<String>>, Vec<Vec<String>>, bool) {
    let mut iter = range.rows();
    let headers = if has_headers {
        iter.next()
            .map(|r| r.iter().map(stringify_cell).collect::<Vec<_>>())
    } else {
        None
    };

    let mut rows = Vec::new();
    let mut rows_truncated = false;
    for row in iter {
        if rows.len() >= max_rows {
            rows_truncated = true;
            break;
        }
        rows.push(row.iter().map(stringify_cell).collect());
    }
    (headers, rows, rows_truncated)
}

/// Stringify a single spreadsheet cell. Empty cells become `""`; every
/// other type goes through `Display` and is length-capped.
fn stringify_cell(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        other => cap_cell(&other.to_string()),
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
    use serde_json::json;

    /// Build a 3-row × 2-col range: header + two data rows of mixed types.
    fn sample_range() -> Range<Data> {
        let mut r: Range<Data> = Range::new((0, 0), (2, 1));
        r.set_value((0, 0), Data::String("name".into()));
        r.set_value((0, 1), Data::String("age".into()));
        r.set_value((1, 0), Data::String("Ada".into()));
        r.set_value((1, 1), Data::Int(36));
        r.set_value((2, 0), Data::String("Grace".into()));
        r.set_value((2, 1), Data::Float(45.5));
        r
    }

    #[test]
    fn range_with_headers() {
        let (headers, rows, trunc) = range_to_table(&sample_range(), true, 1000);
        assert_eq!(headers.unwrap(), vec!["name", "age"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["Ada", "36"]);
        assert_eq!(rows[1][0], "Grace");
        assert!(rows[1][1].starts_with("45.5"));
        assert!(!trunc);
    }

    #[test]
    fn range_without_headers() {
        let (headers, rows, _) = range_to_table(&sample_range(), false, 1000);
        assert!(headers.is_none());
        assert_eq!(rows.len(), 3); // header row is now data
    }

    #[test]
    fn range_max_rows_truncates() {
        let (_h, rows, trunc) = range_to_table(&sample_range(), true, 1);
        assert_eq!(rows.len(), 1);
        assert!(trunc);
    }

    #[test]
    fn empty_cells_become_blank() {
        let mut r: Range<Data> = Range::new((0, 0), (0, 1));
        r.set_value((0, 0), Data::String("x".into()));
        // (0,1) left default → Data::Empty
        let (_h, rows, _) = range_to_table(&r, false, 10);
        assert_eq!(rows[0], vec!["x", ""]);
    }

    #[test]
    fn bad_bytes_are_rejected() {
        assert!(parse_xlsx(b"not a spreadsheet", None, true, 100).is_err());
    }

    #[test]
    fn tool_metadata_is_sound() {
        let dir = std::env::temp_dir();
        let sb = ReaderSandbox::new(&dir).unwrap();
        let tool = DataXlsxTool::new(sb);
        assert_eq!(tool.name(), "data.xlsx");
        assert!(!tool.description().is_empty());
        assert_eq!(tool.input_schema()["type"], "object");
        assert_eq!(tool.required_scope(&json!({"path": "x.xlsx"})).base(), "fs.read");
    }
}

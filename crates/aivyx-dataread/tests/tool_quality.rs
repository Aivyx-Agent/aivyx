//! Chapter Sheaf (SH.4) — metadata quality sweep over the
//! structured-data readers.
//!
//! Atlas (AT.3) shipped `check_tool_quality` and recommended every
//! crate run it over the tools it owns. This is `aivyx-dataread`'s
//! sweep: each reader's name / description / schema — the metadata the
//! LLM relies on to call it — must meet the correctness floor, so a
//! thin description or malformed schema fails CI rather than shipping.
//! The readers are cheap to construct (a `ReaderSandbox` over a temp
//! dir); the metadata methods never touch the filesystem.

use std::sync::Arc;

use aivyx_core::tools::check_tool_quality;
use aivyx_core::Tool;
use aivyx_dataread::{DataCsvTool, DataPdfTool, DataXlsxTool, ReaderSandbox};

#[test]
fn dataread_tools_meet_quality_floor() {
    let sandbox = ReaderSandbox::new(std::env::temp_dir()).expect("temp dir sandbox");

    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(DataCsvTool::new(sandbox.clone())),
        Arc::new(DataXlsxTool::new(sandbox.clone())),
        Arc::new(DataPdfTool::new(sandbox)),
    ];

    let issues: Vec<String> = tools
        .iter()
        .flat_map(|t| check_tool_quality(t.as_ref()))
        .collect();

    assert!(
        issues.is_empty(),
        "structured-data reader quality issues:\n  {}",
        issues.join("\n  "),
    );
}

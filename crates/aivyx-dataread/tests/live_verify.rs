//! Chapter Sheaf (SH.5) — live verification of the readers end-to-end.
//!
//! Unlike Abacus (a toolkit tool-process driven over IPC), the
//! structured-data readers are **in-process** infrastructure tools, so
//! the live-verify drives each one's `execute()` through a real
//! `ToolContext` against a **real sample file** written to a temp
//! sandbox: a CSV, an `.xlsx` (generated with `rust_xlsxwriter`), and a
//! PDF (generated with the already-present `lopdf`). It also confirms
//! the shared sandbox refuses a path that escapes the root — the
//! `fs.read` fence the readers inherit. This exercises the full path
//! the daemon uses: sandbox resolve + read + parse + `ToolOutcome`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_core::{
    AgentId, CancellationToken, ChannelError, ChannelPlatform, NullAuditHook, SessionId,
    StreamEvent, Tool, ToolContext, ToolOutcome, TurnId, TurnOutcome,
};
use aivyx_dataread::{DataCsvTool, DataPdfTool, DataXlsxTool, ReaderSandbox};

// ---- minimal ToolContext (copied from the fs.rs test pattern) --------

struct NoopChannel {
    session: SessionId,
    token: CancellationToken,
}

#[async_trait]
impl aivyx_core::ChannelContext for NoopChannel {
    fn channel_name(&self) -> &str {
        "test"
    }
    fn platform(&self) -> ChannelPlatform {
        ChannelPlatform::Local
    }
    fn trust_tier(&self) -> aivyx_capability::TrustTier {
        aivyx_capability::TrustTier::Trusted
    }
    fn session_id(&self) -> SessionId {
        self.session
    }
    async fn stream_event(&self, _event: StreamEvent<'_>) -> Result<(), ChannelError> {
        Ok(())
    }
    async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), ChannelError> {
        Ok(())
    }
    fn cancellation_token(&self) -> CancellationToken {
        self.token.clone()
    }
}

fn run_execute(tool: &dyn Tool, input: Value) -> ToolOutcome {
    let channel = NoopChannel {
        session: SessionId::new(),
        token: CancellationToken::new(),
    };
    let audit = NullAuditHook;
    let ctx = ToolContext {
        agent_id: AgentId::new(),
        session_id: channel.session,
        turn_id: TurnId::new(),
        channel: &channel,
        audit: &audit,
        cancellation: &channel.token,
        message_origin: aivyx_core::MessageOrigin::Operator,
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(tool.execute(input, &ctx))
}

fn scratch_dir() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "aivyx-sheaf-live-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst),
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn output(outcome: ToolOutcome) -> Value {
    match outcome {
        ToolOutcome::Completed { output, .. } => output,
        other => panic!("expected Completed, got {other:?}"),
    }
}

// ---- fixture writers -------------------------------------------------

fn write_xlsx(path: &std::path::Path) {
    use rust_xlsxwriter::Workbook;
    let mut wb = Workbook::new();
    let s = wb.add_worksheet();
    s.write_string(0, 0, "name").unwrap();
    s.write_string(0, 1, "age").unwrap();
    s.write_string(1, 0, "Ada").unwrap();
    s.write_number(1, 1, 36.0).unwrap();
    s.write_string(2, 0, "Grace").unwrap();
    s.write_number(2, 1, 45.0).unwrap();
    wb.save(path).unwrap();
}

fn write_pdf(path: &std::path::Path) {
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Document, Object, Stream};
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 24.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal("Hello Sheaf PDF")]),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut buf = Vec::new();
    doc.save_to(&mut buf).unwrap();
    std::fs::write(path, buf).unwrap();
}

// ---- the live verification ------------------------------------------

#[test]
fn data_csv_reads_a_real_file() {
    let dir = scratch_dir();
    std::fs::write(dir.join("people.csv"), b"name,age\nAda,36\nGrace,45\n").unwrap();
    let tool = DataCsvTool::new(ReaderSandbox::new(&dir).unwrap());

    let out = output(run_execute(&tool, json!({ "path": "people.csv" })));
    assert_eq!(out["headers"], json!(["name", "age"]));
    assert_eq!(out["rows"], json!([["Ada", "36"], ["Grace", "45"]]));
    assert_eq!(out["row_count"], 2);
    assert_eq!(out["truncated"], false);
}

#[test]
fn data_xlsx_reads_a_real_file() {
    let dir = scratch_dir();
    write_xlsx(&dir.join("people.xlsx"));
    let tool = DataXlsxTool::new(ReaderSandbox::new(&dir).unwrap());

    let out = output(run_execute(&tool, json!({ "path": "people.xlsx" })));
    assert_eq!(out["headers"], json!(["name", "age"]));
    assert_eq!(out["rows"][0], json!(["Ada", "36"]));
    assert_eq!(out["row_count"], 2);
    assert_eq!(out["sheet_names"], json!(["Sheet1"]));
}

#[test]
fn data_pdf_extracts_real_text() {
    let dir = scratch_dir();
    write_pdf(&dir.join("doc.pdf"));
    let tool = DataPdfTool::new(ReaderSandbox::new(&dir).unwrap());

    let out = output(run_execute(&tool, json!({ "path": "doc.pdf" })));
    assert!(
        out["text"].as_str().unwrap().contains("Hello Sheaf PDF"),
        "extracted text: {:?}",
        out["text"],
    );
    assert_eq!(out["pages"], 1);
}

#[test]
fn sandbox_escape_is_refused_through_execute() {
    let dir = scratch_dir();
    std::fs::write(dir.join("ok.csv"), b"a\n1\n").unwrap();
    let tool = DataCsvTool::new(ReaderSandbox::new(&dir).unwrap());

    // A path climbing out of the sandbox must not read /etc/passwd.
    let outcome = run_execute(&tool, json!({ "path": "../../../../etc/passwd" }));
    assert!(
        matches!(outcome, ToolOutcome::Failed(_)),
        "escape should Fail, got {outcome:?}",
    );
}

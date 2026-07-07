//! Structured-data reader tools for Aivyx — **Chapter Sheaf**.
//!
//! Three readers turn an operator file the agent can already reach via
//! `fs.read` into legible structured content (the file-content
//! analogue of `web.extract`):
//!
//! - [`DataCsvTool`] (`data.csv`, SH.1) — delimited text → rows.
//! - [`DataXlsxTool`] (`data.xlsx`, SH.2) — spreadsheet → a sheet's rows.
//! - [`DataPdfTool`] (`data.pdf`, SH.3) — PDF → extracted text.
//!
//! Plus two SH.6 writers, symmetric with the readers above — structured
//! content the agent already has, turned into a real file:
//!
//! - [`DataXlsxWriteTool`] (`data.xlsx.write`) — rows → a new spreadsheet.
//! - [`DataPdfWriteTool`] (`data.pdf.write`) — text → a new PDF (plain
//!   single-font text flow with pagination; no rich formatting).
//!
//! ## Governance (see `docs/SHEAF.md`)
//!
//! Each reader **reuses the existing `fs.read` capability and the
//! `aivyx_core` filesystem sandbox** ([`crate::sandbox::ReaderSandbox`],
//! built on `aivyx_core::tools::fs::lexical_resolve`): it can only read
//! a file the agent could already `fs.read`, adding **no new capability
//! base and no new I/O reach**. That makes the readers an
//! **infrastructure-tier** addition — a transform over already-readable
//! bytes, not a new irreducible capability — so they grow no P10
//! substrate count. Heavy format parsers (xlsx/pdf) are isolated in
//! this crate rather than bloating `aivyx-core`.

pub mod csv_reader;
pub mod pdf_reader;
pub mod pdf_writer;
pub mod sandbox;
pub mod xlsx_reader;
pub mod xlsx_writer;

pub use csv_reader::DataCsvTool;
pub use pdf_reader::DataPdfTool;
pub use pdf_writer::DataPdfWriteTool;
pub use sandbox::ReaderSandbox;
pub use xlsx_reader::DataXlsxTool;
pub use xlsx_writer::DataXlsxWriteTool;

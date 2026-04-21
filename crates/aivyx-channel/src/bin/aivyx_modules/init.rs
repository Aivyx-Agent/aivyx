//! `aivyx init` — interactive first-run setup wizard (Phase 44).
//!
//! Detects whether Ollama is running locally and defaults to it,
//! walks the user through provider and model selection, and writes
//! a ready-to-use `aivyx.toml` config file.

/// Entry point for the init wizard. Called from `run()` in the
/// binary when `CliMode::Init` is dispatched.
pub async fn run_init_wizard() -> Result<(), String> {
    eprintln!("aivyx init: not yet implemented");
    Ok(())
}

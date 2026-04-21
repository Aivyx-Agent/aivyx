//! `aivyx init` — interactive first-run setup wizard (Phase 44).
//!
//! Detects whether Ollama is running locally and defaults to it,
//! walks the user through provider and model selection, and writes
//! a ready-to-use `aivyx.toml` config file.

use std::io::{BufRead, Write as IoWrite};
use std::time::Duration;

use aivyx_llm::openai::DEFAULT_OLLAMA_BASE_URL;

/// Connect timeout for Ollama detection — short so the wizard
/// doesn't hang when Ollama isn't running.
const DETECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Read timeout for the `/api/tags` model listing call.
const LIST_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Ollama detection + model listing
// ---------------------------------------------------------------------------

/// Check whether Ollama is reachable at the given base URL.
/// Returns `true` on any 2xx response, `false` on error.
async fn detect_ollama(base_url: &str) -> bool {
    let client = match reqwest::Client::builder()
        .connect_timeout(DETECT_TIMEOUT)
        .timeout(DETECT_TIMEOUT)
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    client.get(base_url).send().await.is_ok_and(|r| r.status().is_success())
}

/// Fetch the list of locally available model names from Ollama's
/// `GET /api/tags` endpoint. Returns model names sorted alphabetically.
async fn list_ollama_models(base_url: &str) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(DETECT_TIMEOUT)
        .timeout(LIST_TIMEOUT)
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("failed to reach {url}: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("failed to read response from {url}: {e}"))?;

    if !status.is_success() {
        return Err(format!("{url} returned HTTP {status}: {body}"));
    }

    parse_model_names(&body)
}

/// Extract model names from the Ollama `/api/tags` JSON response.
/// The expected shape is `{ "models": [{ "name": "...", ... }, ...] }`.
fn parse_model_names(json_body: &str) -> Result<Vec<String>, String> {
    let value: serde_json::Value = serde_json::from_str(json_body)
        .map_err(|e| format!("invalid JSON from /api/tags: {e}"))?;

    let models = value
        .get("models")
        .and_then(|v| v.as_array())
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(String::from))
        .collect::<Vec<_>>();

    let mut sorted = models;
    sorted.sort();
    Ok(sorted)
}

// ---------------------------------------------------------------------------
// Interactive prompt helpers
// ---------------------------------------------------------------------------

/// Read a single line from `reader`, print `prompt` to `writer` first.
/// Returns the trimmed input. Returns an error if reading fails.
#[allow(dead_code)] // wired in Task 5
fn prompt_line(
    prompt: &str,
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
) -> Result<String, String> {
    write!(writer, "{prompt}").map_err(|e| format!("write error: {e}"))?;
    writer.flush().map_err(|e| format!("flush error: {e}"))?;
    let mut buf = String::new();
    reader
        .read_line(&mut buf)
        .map_err(|e| format!("read error: {e}"))?;
    Ok(buf.trim().to_string())
}

/// Present a numbered menu and return the chosen option (0-indexed).
/// `default` is returned when the user presses Enter without typing.
///
/// Example output:
/// ```text
///   1) Ollama (local)
///   2) Anthropic
///   3) OpenAI
/// Choose [1]:
/// ```
#[allow(dead_code)] // wired in Task 5
fn prompt_choice(
    prompt: &str,
    options: &[&str],
    default: usize,
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
) -> Result<usize, String> {
    for (i, opt) in options.iter().enumerate() {
        writeln!(writer, "  {}) {opt}", i + 1)
            .map_err(|e| format!("write error: {e}"))?;
    }
    loop {
        let input = prompt_line(
            &format!("{prompt} [{}]: ", default + 1),
            reader,
            writer,
        )?;
        if input.is_empty() {
            return Ok(default);
        }
        match input.parse::<usize>() {
            Ok(n) if n >= 1 && n <= options.len() => return Ok(n - 1),
            _ => {
                writeln!(writer, "Please enter a number between 1 and {}.", options.len())
                    .map_err(|e| format!("write error: {e}"))?;
            }
        }
    }
}

/// Prompt for a secret (e.g. an API key) without echoing input.
/// Uses `rpassword` which requires a real TTY — not unit-testable
/// with injected readers.
#[allow(dead_code)] // wired in Task 5
fn prompt_secret(prompt: &str) -> Result<String, String> {
    rpassword::prompt_password(prompt).map_err(|e| format!("failed to read secret: {e}"))
}

/// Ask a yes/no question. `default` is the answer when the user
/// presses Enter. Returns `true` for yes, `false` for no.
#[allow(dead_code)] // wired in Task 5
fn prompt_yes_no(
    prompt: &str,
    default: bool,
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
) -> Result<bool, String> {
    let hint = if default { "Y/n" } else { "y/N" };
    let input = prompt_line(&format!("{prompt} [{hint}]: "), reader, writer)?;
    if input.is_empty() {
        return Ok(default);
    }
    match input.to_ascii_lowercase().as_str() {
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        _ => {
            writeln!(writer, "Please answer y or n.")
                .map_err(|e| format!("write error: {e}"))?;
            prompt_yes_no(prompt, default, reader, writer)
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point (stub — wired end-to-end in Task 5)
// ---------------------------------------------------------------------------

/// Entry point for the init wizard. Called from `run()` in the
/// binary when `CliMode::Init` is dispatched.
pub async fn run_init_wizard() -> Result<(), String> {
    let base_url = DEFAULT_OLLAMA_BASE_URL;
    let has_ollama = detect_ollama(base_url).await;

    if has_ollama {
        eprintln!("Ollama detected at {base_url}");
        match list_ollama_models(base_url).await {
            Ok(models) if !models.is_empty() => {
                eprintln!("Available models: {}", models.join(", "));
            }
            Ok(_) => {
                eprintln!("No models found. Run `ollama pull llama3.2` first.");
            }
            Err(e) => {
                eprintln!("Could not list models: {e}");
            }
        }
    } else {
        eprintln!("Ollama not detected at {base_url}");
    }

    eprintln!("aivyx init: interactive wizard not yet complete");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tags_response_extracts_model_names() {
        let json = r#"{
            "models": [
                { "name": "llama3.2:latest", "size": 2000000000 },
                { "name": "codellama:7b", "size": 3000000000 },
                { "name": "mistral:latest", "size": 4000000000 }
            ]
        }"#;
        let names = parse_model_names(json).unwrap();
        assert_eq!(names, vec![
            "codellama:7b",
            "llama3.2:latest",
            "mistral:latest",
        ]);
    }

    #[test]
    fn parse_tags_empty_models_returns_empty_vec() {
        let json = r#"{ "models": [] }"#;
        let names = parse_model_names(json).unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn parse_tags_malformed_json_returns_error() {
        let result = parse_model_names("not json");
        assert!(result.is_err());
    }

    #[test]
    fn parse_tags_missing_models_key_returns_empty() {
        let json = r#"{ "other": "data" }"#;
        let names = parse_model_names(json).unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn parse_tags_skips_entries_without_name() {
        let json = r#"{
            "models": [
                { "name": "llama3.2:latest" },
                { "size": 1000 },
                { "name": "mistral:latest" }
            ]
        }"#;
        let names = parse_model_names(json).unwrap();
        assert_eq!(names, vec!["llama3.2:latest", "mistral:latest"]);
    }

    // -- prompt helpers --------------------------------------------------

    use std::io::Cursor;

    #[test]
    fn prompt_line_trims_whitespace() {
        let mut input = Cursor::new(b"  hello world  \n" as &[u8]);
        let mut output = Vec::new();
        let result = prompt_line("Name: ", &mut input, &mut output).unwrap();
        assert_eq!(result, "hello world");
        assert_eq!(String::from_utf8(output).unwrap(), "Name: ");
    }

    #[test]
    fn prompt_choice_returns_default_on_empty_input() {
        let mut input = Cursor::new(b"\n" as &[u8]);
        let mut output = Vec::new();
        let opts = &["Ollama", "Anthropic", "OpenAI"];
        let result = prompt_choice("Choose", opts, 0, &mut input, &mut output).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn prompt_choice_returns_selected_option() {
        let mut input = Cursor::new(b"2\n" as &[u8]);
        let mut output = Vec::new();
        let opts = &["Ollama", "Anthropic", "OpenAI"];
        let result = prompt_choice("Choose", opts, 0, &mut input, &mut output).unwrap();
        assert_eq!(result, 1); // 0-indexed
    }

    #[test]
    fn prompt_choice_rejects_out_of_range_then_accepts() {
        // First input "5" is out of range, second "3" is valid.
        let mut input = Cursor::new(b"5\n3\n" as &[u8]);
        let mut output = Vec::new();
        let opts = &["Ollama", "Anthropic", "OpenAI"];
        let result = prompt_choice("Choose", opts, 0, &mut input, &mut output).unwrap();
        assert_eq!(result, 2);
        let out = String::from_utf8(output).unwrap();
        assert!(out.contains("Please enter a number between 1 and 3"));
    }

    #[test]
    fn prompt_yes_no_defaults_true() {
        let mut input = Cursor::new(b"\n" as &[u8]);
        let mut output = Vec::new();
        let result = prompt_yes_no("Overwrite?", true, &mut input, &mut output).unwrap();
        assert!(result);
        let out = String::from_utf8(output).unwrap();
        assert!(out.contains("[Y/n]"));
    }

    #[test]
    fn prompt_yes_no_defaults_false() {
        let mut input = Cursor::new(b"\n" as &[u8]);
        let mut output = Vec::new();
        let result = prompt_yes_no("Overwrite?", false, &mut input, &mut output).unwrap();
        assert!(!result);
        let out = String::from_utf8(output).unwrap();
        assert!(out.contains("[y/N]"));
    }

    #[test]
    fn prompt_yes_no_accepts_yes() {
        let mut input = Cursor::new(b"yes\n" as &[u8]);
        let mut output = Vec::new();
        let result = prompt_yes_no("Continue?", false, &mut input, &mut output).unwrap();
        assert!(result);
    }
}

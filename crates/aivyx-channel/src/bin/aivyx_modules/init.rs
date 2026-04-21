//! `aivyx init` — interactive first-run setup wizard (Phase 44).
//!
//! Detects whether Ollama is running locally and defaults to it,
//! walks the user through provider and model selection, and writes
//! a ready-to-use `aivyx.toml` config file.

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
}

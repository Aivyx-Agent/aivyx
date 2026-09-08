//! Phase 104 — provider credential verification.
//!
//! Used by `aivyx-pa init` to confirm the operator-supplied
//! `(provider, api_key, model)` triple is valid before writing
//! `aivyx-pa.toml` to disk. Catches typo'd keys and non-existent
//! model names at config-write time rather than first-turn time
//! (after the operator has already committed to a passphrase).
//!
//! Both Anthropic and OpenAI expose a `GET /v1/models` endpoint
//! that takes an API key in a provider-specific header and
//! returns a JSON document with shape `{ data: [ { id, … }, … ] }`.
//! The helper issues that GET, parses the returned list, and
//! confirms the chosen model is present. Errors carry enough
//! detail for the wizard to re-prompt the right field on retry
//! (auth → re-prompt key; model-not-found → re-prompt model).

use std::time::Duration;

use thiserror::Error;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Provider kinds verified by [`verify_provider_credentials`].
///
/// Ollama is excluded on purpose — the wizard verifies Ollama
/// via the existing `/api/tags` listing path; an empty list is
/// caught before this helper is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyProvider {
    Anthropic,
    OpenAi,
}

/// Why [`verify_provider_credentials`] failed. The variants are
/// discriminated by failure class so the wizard knows which
/// prompt to re-show on retry — auth errors → re-prompt key,
/// model-not-found → re-prompt model, network/other → both.
#[derive(Debug, Error)]
pub enum VerifyError {
    /// 401 / 403 from the provider. The supplied API key is
    /// missing, malformed, or revoked.
    #[error("authentication failed: {0}")]
    Auth(String),

    /// The provider accepted the key but the requested model id
    /// was not present in the returned list. `available` is a
    /// human-readable summary of what *is* available, so the
    /// wizard can show it on retry.
    #[error(
        "model `{model}` is not available for this account; \
         available models: {available}"
    )]
    ModelNotFound { model: String, available: String },

    /// Network-layer failure — DNS, connect timeout, TLS, etc.
    /// The wizard treats this as ambiguous (could be either key
    /// or model wrong) and re-prompts both.
    #[error("network error: {0}")]
    Network(String),

    /// The provider returned a non-auth non-success status, or
    /// the response body did not match the expected shape. The
    /// wizard treats this as ambiguous like [`VerifyError::Network`].
    #[error("provider returned unexpected response: {0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const ANTHROPIC_MODELS_URL: &str = "https://api.anthropic.com/v1/models";
const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";

/// Anthropic requires an `anthropic-version` header on every
/// request. The value here matches the version
/// `aivyx-llm::anthropic::provider` already uses.
const ANTHROPIC_API_VERSION: &str = "2023-06-01";

/// Connect + total timeout for the verify GET. Short — the
/// wizard is interactive and a stalled verify is worse than a
/// crisp `Network` error the operator can retry from.
const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Verify the `(provider, api_key, model)` triple by issuing
/// `GET /v1/models` with the supplied key and confirming the
/// chosen model is in the returned list.
///
/// Returns `Ok(())` on success; on failure the wizard branches
/// on the [`VerifyError`] variant to re-prompt the right field.
pub async fn verify_provider_credentials(
    provider: VerifyProvider,
    api_key: &str,
    model: &str,
) -> Result<(), VerifyError> {
    let client = reqwest::Client::builder()
        .timeout(VERIFY_TIMEOUT)
        .connect_timeout(VERIFY_TIMEOUT)
        .build()
        .map_err(|e| {
            VerifyError::Network(format!("failed to build HTTP client: {e}"))
        })?;

    let (url, headers) = match provider {
        VerifyProvider::Anthropic => (
            ANTHROPIC_MODELS_URL,
            vec![
                ("x-api-key", api_key.to_string()),
                ("anthropic-version", ANTHROPIC_API_VERSION.to_string()),
            ],
        ),
        VerifyProvider::OpenAi => (
            OPENAI_MODELS_URL,
            vec![("Authorization", format!("Bearer {api_key}"))],
        ),
    };

    let mut req = client.get(url);
    for (k, v) in &headers {
        req = req.header(*k, v.as_str());
    }

    let resp = req
        .send()
        .await
        .map_err(|e| VerifyError::Network(format!("failed to reach {url}: {e}")))?;

    let status = resp.status().as_u16();
    let body = resp
        .text()
        .await
        .map_err(|e| VerifyError::Network(format!("failed to read response: {e}")))?;

    classify_response(status, &body, model)
}

// ---------------------------------------------------------------------------
// Pure response classification (extracted for testability)
// ---------------------------------------------------------------------------

/// Map an HTTP status + response body + chosen model to a
/// [`Result<(), VerifyError>`]. Extracted from
/// [`verify_provider_credentials`] so unit tests can drive every
/// branch without a real HTTP server.
pub(crate) fn classify_response(
    status: u16,
    body: &str,
    model: &str,
) -> Result<(), VerifyError> {
    if status == 401 || status == 403 {
        return Err(VerifyError::Auth(format!(
            "HTTP {status}: {}",
            truncate(body, 200)
        )));
    }

    if !(200..300).contains(&status) {
        return Err(VerifyError::Other(format!(
            "HTTP {status}: {}",
            truncate(body, 200)
        )));
    }

    let models = parse_models_response(body)
        .map_err(|e| VerifyError::Other(format!("unparseable models list: {e}")))?;

    if models.iter().any(|id| id == model) {
        Ok(())
    } else {
        Err(VerifyError::ModelNotFound {
            model: model.to_string(),
            available: summarize_models(&models, 8),
        })
    }
}

/// Parse the `{ data: [ { id, … }, … ] }` shape shared by
/// Anthropic's and OpenAI's `/v1/models` responses.
pub(crate) fn parse_models_response(body: &str) -> Result<Vec<String>, String> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("invalid JSON: {e}"))?;

    let arr = v
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "missing or non-array `data` field".to_string())?;

    let ids = arr
        .iter()
        .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from))
        .collect::<Vec<_>>();

    Ok(ids)
}

/// Truncate a string to `max` chars, appending `…` if cut. Used
/// when echoing provider error bodies into [`VerifyError`] messages
/// so a multi-KB HTML 5xx page doesn't blow up the wizard's
/// re-prompt output.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

/// Render up to `cap` model ids comma-separated, with a tail
/// `(+N more)` when truncated. Used in [`VerifyError::ModelNotFound`]
/// so the wizard's retry prompt can show the operator what is
/// available.
pub(crate) fn summarize_models(models: &[String], cap: usize) -> String {
    if models.is_empty() {
        return "(none returned)".to_string();
    }
    let shown: Vec<&str> = models.iter().take(cap).map(|s| s.as_str()).collect();
    let suffix = if models.len() > cap {
        format!(", … (+{} more)", models.len() - cap)
    } else {
        String::new()
    };
    format!("{}{}", shown.join(", "), suffix)
}

// ---------------------------------------------------------------------------
// Tests — pure-function coverage only. The live HTTP path in
// `verify_provider_credentials` is integration-tested in the
// wizard's `cargo test` suite via the same model the rest of
// `aivyx-llm` uses: the parse + classify helpers are exhaustively
// unit-tested, and the HTTP layer is treated as
// integration-tested-only (matching the wizard's existing
// `parse_model_names` / `detect_ollama` split in
// `aivyx_modules/init.rs`).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_models_response -------------------------------------------

    #[test]
    fn parse_extracts_ids_from_data_array() {
        let body = r#"{
            "data": [
                { "id": "claude-sonnet-4-6", "type": "model" },
                { "id": "claude-opus-4-7", "type": "model" }
            ]
        }"#;
        let ids = parse_models_response(body).unwrap();
        assert_eq!(ids, vec!["claude-sonnet-4-6", "claude-opus-4-7"]);
    }

    #[test]
    fn parse_skips_entries_without_id() {
        let body = r#"{ "data": [
            { "id": "gpt-4.1" },
            { "type": "model" },
            { "id": "gpt-4.1-mini" }
        ] }"#;
        let ids = parse_models_response(body).unwrap();
        assert_eq!(ids, vec!["gpt-4.1", "gpt-4.1-mini"]);
    }

    #[test]
    fn parse_empty_data_returns_empty_vec() {
        let body = r#"{ "data": [] }"#;
        let ids = parse_models_response(body).unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn parse_missing_data_errors() {
        let body = r#"{ "models": [] }"#;
        assert!(parse_models_response(body).is_err());
    }

    #[test]
    fn parse_invalid_json_errors() {
        assert!(parse_models_response("not json").is_err());
    }

    // -- classify_response -----------------------------------------------

    fn body_with_models(ids: &[&str]) -> String {
        let entries: Vec<String> = ids
            .iter()
            .map(|id| format!(r#"{{"id":"{id}"}}"#))
            .collect();
        format!(r#"{{"data":[{}]}}"#, entries.join(","))
    }

    #[test]
    fn classify_ok_when_model_present() {
        let body = body_with_models(&["claude-sonnet-4-6", "claude-opus-4-7"]);
        let result = classify_response(200, &body, "claude-sonnet-4-6");
        assert!(matches!(result, Ok(())));
    }

    #[test]
    fn classify_401_is_auth() {
        let result = classify_response(401, "{\"error\":\"bad key\"}", "any");
        assert!(matches!(result, Err(VerifyError::Auth(_))));
    }

    #[test]
    fn classify_403_is_auth() {
        let result = classify_response(403, "forbidden", "any");
        assert!(matches!(result, Err(VerifyError::Auth(_))));
    }

    #[test]
    fn classify_500_is_other() {
        let result = classify_response(500, "<html>oops</html>", "any");
        assert!(matches!(result, Err(VerifyError::Other(_))));
    }

    #[test]
    fn classify_model_not_in_returned_list() {
        let body = body_with_models(&["claude-sonnet-4-6"]);
        let result = classify_response(200, &body, "claude-typo-4-6");
        match result {
            Err(VerifyError::ModelNotFound { model, available }) => {
                assert_eq!(model, "claude-typo-4-6");
                assert!(available.contains("claude-sonnet-4-6"));
            }
            other => panic!("expected ModelNotFound, got {other:?}"),
        }
    }

    #[test]
    fn classify_200_with_unparseable_body_is_other() {
        let result = classify_response(200, "not json at all", "any");
        assert!(matches!(result, Err(VerifyError::Other(_))));
    }

    #[test]
    fn classify_truncates_long_error_bodies() {
        let huge = "x".repeat(5000);
        let err = classify_response(500, &huge, "any").unwrap_err();
        let msg = err.to_string();
        // Body is truncated to 200 chars + "…" in the error message.
        assert!(msg.len() < 500, "error message too long: {} chars", msg.len());
        assert!(msg.contains('…'));
    }

    // -- summarize_models ------------------------------------------------

    #[test]
    fn summarize_empty_list() {
        assert_eq!(summarize_models(&[], 8), "(none returned)");
    }

    #[test]
    fn summarize_within_cap() {
        let models = vec!["a".to_string(), "b".to_string()];
        assert_eq!(summarize_models(&models, 8), "a, b");
    }

    #[test]
    fn summarize_truncates_with_count() {
        let models: Vec<String> = (0..12).map(|i| format!("m{i}")).collect();
        let s = summarize_models(&models, 3);
        assert!(s.starts_with("m0, m1, m2"));
        assert!(s.contains("+9 more"));
    }
}

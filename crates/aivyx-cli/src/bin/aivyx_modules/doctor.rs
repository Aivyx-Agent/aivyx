//! `aivyx doctor` — first-run health check for the local on-ramp (Chapter P).
//!
//! Confirms the path actually works end to end and, when it doesn't, says
//! *why* and *what to do* — instead of leaving a new user with a blank first
//! turn. For the local (Ollama) provider it checks: Ollama reachable → the
//! configured model present → a real test generation that returns **non-empty**
//! text (the exact failure modes — empty thinking content, dropped tool calls,
//! starved `num_ctx` — that ruined first impressions). Cloud providers get a
//! lighter config-presence check. Read-only; no daemon, no passphrase.

use std::path::Path;

use aivyx_config::{AivyxConfig, LoadOptions, ProviderKind};
use aivyx_llm::ollama::{
    OllamaConfig, OllamaOptions, OllamaProvider, DEFAULT_OLLAMA_BASE_URL, RECOMMENDED_LOCAL_MODEL,
};
use aivyx_llm::{LlmMessage, LlmProvider, LlmRequest, LlmStreamEvent, LlmToolDescriptor};

const DOCTOR_TOML_PATH: &str = "aivyx.toml";

/// `aivyx doctor` — run the health checks and report.
pub async fn run_doctor() -> Result<(), String> {
    let cfg = load_config_for_inspection()?;
    println!("aivyx doctor — checking your setup\n");

    let all_ok = match cfg.provider.value {
        ProviderKind::Ollama => check_ollama(&cfg).await,
        other => check_cloud(other, &cfg),
    };

    println!();
    if all_ok {
        println!("✓ Looks good — your agent is ready. Run `aivyx` to start.");
        Ok(())
    } else {
        Err("one or more checks failed — see the notes above.".into())
    }
}

fn pass(label: &str) {
    println!("  ✓ {label}");
}
fn fail(label: &str, hint: &str) {
    println!("  ✗ {label}\n     → {hint}");
}

/// Run the local-Ollama checks. Returns `true` iff every check passed.
async fn check_ollama(cfg: &AivyxConfig) -> bool {
    let base_url = cfg
        .openai_base_url
        .as_ref()
        .map(|s| s.value.clone())
        .unwrap_or_else(|| DEFAULT_OLLAMA_BASE_URL.to_string());
    let model = cfg.model.value.clone();
    println!("Provider: ollama (model `{model}`, {base_url})\n");

    // 1. Ollama reachable.
    if !crate::init::detect_ollama(&base_url).await {
        fail(
            &format!("Ollama is not reachable at {base_url}"),
            "Start it with `ollama serve` (and install Ollama if you haven't: https://ollama.com).",
        );
        return false;
    }
    pass("Ollama is running");

    // 2. The configured model is pulled.
    let models = crate::init::list_ollama_models(&base_url).await.unwrap_or_default();
    if !models.iter().any(|m| m == &model) {
        fail(
            &format!("model `{model}` is not downloaded"),
            &format!("Pull it with `ollama pull {model}` (or `aivyx init` to pick/pull a model)."),
        );
        return false;
    }
    pass(&format!("model `{model}` is available"));

    // 3. A real test generation returns non-empty text — the check that
    //    catches empty-content / dropped-tool / starved-context regressions.
    match test_generation(&base_url, &model).await {
        Ok(text) if !text.trim().is_empty() => {
            pass(&format!("test reply OK: \"{}\"", truncate(&text, 60)));
            true
        }
        Ok(_) => {
            fail(
                "the model returned an EMPTY reply",
                &format!(
                    "The classic local-model failure. Try the recommended model: \
                     `ollama pull {RECOMMENDED_LOCAL_MODEL}` — it's verified to work \
                     with the agent's tool-calling."
                ),
            );
            false
        }
        Err(e) => {
            fail(&format!("test generation failed: {e}"), "Check that Ollama is healthy and the model loads.");
            false
        }
    }
}

/// One real generation through the native provider — same path a turn uses
/// (auto-`num_ctx` + thinking handling apply), with a tool present so the
/// thinking-empty mode is exercised. Returns the collected text.
async fn test_generation(base_url: &str, model: &str) -> Result<String, String> {
    let provider = OllamaProvider::new(
        OllamaConfig::default_local()
            .with_base_url(base_url.to_string())
            .with_options(OllamaOptions::default()),
    )
    .map_err(|e| format!("build provider: {e}"))?;

    // A dummy tool makes the request shape match a real agent turn (the
    // empty-content bug only shows when `tools` is present).
    let tools = vec![LlmToolDescriptor {
        name: "noop".to_string(),
        description: "A tool you do not need to call for this.".to_string(),
        input_schema: serde_json::json!({ "type": "object", "properties": {} }),
    }];
    let messages = vec![LlmMessage::user_text(
        "Reply with exactly the word: OK",
    )];
    let request = LlmRequest {
        model,
        system: Some("You are a helpful assistant."),
        messages: &messages,
        tools: &tools,
        max_tokens: 64,
        temperature: None,
    };

    let token = aivyx_core::CancellationToken::new();
    let mut stream = provider
        .chat_stream(request, &token)
        .await
        .map_err(|e| format!("{e}"))?;
    let mut text = String::new();
    while let Some(event) = stream.next_event().await.map_err(|e| format!("{e}"))? {
        if let LlmStreamEvent::TextChunk(chunk) = event {
            text.push_str(&chunk);
        }
    }
    // Drain the terminal; the text-so-far is what we assert on.
    let _ = Box::new(stream).finish().await;
    Ok(text)
}

/// Lighter check for cloud providers — confirm the config is wired (a key is
/// present). The full credential round-trip lives in the `init` wizard.
fn check_cloud(provider: ProviderKind, cfg: &AivyxConfig) -> bool {
    println!("Provider: {provider} (model `{}`)\n", cfg.model.value);
    let has_key = match provider {
        ProviderKind::Anthropic => cfg.anthropic_api_key.is_some(),
        ProviderKind::OpenAi | ProviderKind::LlamaCpp | ProviderKind::Jan => {
            cfg.openai_api_key.is_some()
        }
        _ => true,
    };
    if has_key {
        pass("an API key is configured");
        println!("  (run `aivyx init` to re-verify the key against the provider)");
        true
    } else {
        fail(
            "no API key configured for this provider",
            "Run `aivyx init` to set one, or switch to a local model with `[agent] provider = \"ollama\"`.",
        );
        false
    }
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

fn load_config_for_inspection() -> Result<AivyxConfig, String> {
    let opts = LoadOptions {
        toml_path: Some(Path::new(DOCTOR_TOML_PATH).to_path_buf()),
        require_api_key: false,
        require_telegram_token: false,
        require_discord_token: false,
        require_slack_tokens: false,
        role_override: None,
    };
    AivyxConfig::load_from_env_and_toml(&opts)
        .map_err(|e| format!("failed to load {DOCTOR_TOML_PATH}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_shortens_long_strings() {
        assert_eq!(truncate("hello", 60), "hello");
        assert_eq!(truncate(&"x".repeat(100), 10), format!("{}…", "x".repeat(10)));
    }

    #[tokio::test]
    async fn test_generation_unreachable_is_error() {
        // No Ollama → a clear Err the check turns into actionable output.
        let res = test_generation("http://127.0.0.1:1", RECOMMENDED_LOCAL_MODEL).await;
        assert!(res.is_err(), "unreachable Ollama must error");
    }
}

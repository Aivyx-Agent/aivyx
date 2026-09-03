//! `aivyx init` — interactive first-run setup wizard (Phase 44).
//!
//! Detects whether Ollama is running locally and defaults to it,
//! walks the user through provider and model selection, and writes
//! a ready-to-use `aivyx.toml` config file.

use std::io::{self, BufRead, IsTerminal, Write as IoWrite};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use aivyx_llm::openai::DEFAULT_OLLAMA_BASE_URL;
use aivyx_config::AccessLevel;
use aivyx_llm::ollama::RECOMMENDED_LOCAL_MODEL;
use aivyx_llm::verify::{verify_provider_credentials, VerifyError, VerifyProvider};
use aivyx_llm::LlmProvider;

use aivyx_channel::profile_draft::{
    draft_identity, DraftedProfile, IdentityAnswers,
};

/// Default config file name (matches `aivyx-config` convention).
const CONFIG_FILE: &str = "aivyx.toml";

/// Default Anthropic model id presented to the operator on the
/// model-name prompt. Phase 104 refresh: was
/// `"claude-sonnet-4-20250514"` (Phase 44, ~1 year stale at
/// Phase 104 entry); current Sonnet generation is
/// `claude-sonnet-4-6`.
const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-6";

/// Default OpenAI model id presented to the operator on the
/// model-name prompt. Phase 104 refresh: was `"gpt-4o"` (Phase
/// 25); current flagship is `gpt-4.1`.
const DEFAULT_OPENAI_MODEL: &str = "gpt-4.1";

/// Chapter Engram — the recommended local embedding model and its vector
/// width. Ollama serves it over the OpenAI-compatible `/v1/embeddings`
/// endpoint the embedding provider already speaks, so semantic memory works
/// against a local Ollama with no key. `nomic-embed-text` emits 768-dim
/// vectors.
pub(crate) const RECOMMENDED_EMBED_MODEL: &str = "nomic-embed-text";
const RECOMMENDED_EMBED_DIMENSIONS: usize = 768;

/// Printed when `list_ollama_models` returns an empty list. Chapter P:
/// names the **tool-capable** recommended model (`RECOMMENDED_LOCAL_MODEL`)
/// rather than a small non-tool-caller — the agent needs tool-calling to be
/// useful, so the first-run model must support it.
fn ollama_empty_hint() -> String {
    format!(
        "No local models found.\n\
         The agent needs a tool-capable model. Recommended: \
         `ollama pull {RECOMMENDED_LOCAL_MODEL}`",
    )
}

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
pub(crate) async fn detect_ollama(base_url: &str) -> bool {
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
pub(crate) async fn list_ollama_models(base_url: &str) -> Result<Vec<String>, String> {
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

/// Chapter P — pull a model via Ollama's `POST /api/pull` (streaming), printing
/// coarse download progress so a multi-GB pull doesn't look hung. Best-effort
/// progress; the return value is what matters (`Ok` = the model is now local).
async fn pull_ollama_model(
    base_url: &str,
    model: &str,
    writer: &mut dyn IoWrite,
) -> Result<(), String> {
    use futures_util::StreamExt;

    let client = reqwest::Client::builder()
        .connect_timeout(DETECT_TIMEOUT)
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;
    let url = format!("{}/api/pull", base_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .json(&serde_json::json!({ "model": model, "stream": true }))
        .send()
        .await
        .map_err(|e| format!("failed to reach {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "{url} returned HTTP {} pulling {model}",
            resp.status()
        ));
    }

    writeln!(writer, "Downloading {model} …").map_err(|e| format!("write error: {e}"))?;
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut last_pct: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("pull stream error: {e}"))?;
        buf.extend_from_slice(&chunk);
        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(&line) else {
                continue;
            };
            if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                return Err(format!("pull failed: {err}"));
            }
            let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
            match (
                v.get("completed").and_then(|x| x.as_u64()),
                v.get("total").and_then(|x| x.as_u64()),
            ) {
                (Some(c), Some(t)) if t > 0 => {
                    let pct = c * 100 / t;
                    if pct >= last_pct + 5 || pct == 100 {
                        let _ = write!(writer, "\r  {status}: {pct}%        ");
                        let _ = writer.flush();
                        last_pct = pct;
                    }
                }
                _ if !status.is_empty() => {
                    let _ = write!(writer, "\r  {status}                    ");
                    let _ = writer.flush();
                }
                _ => {}
            }
        }
    }
    writeln!(writer, "\r  {model} ready.                    ")
        .map_err(|e| format!("write error: {e}"))?;
    Ok(())
}

/// Chapter Engram — resolve the `[embedding]` provider for the chosen chat
/// provider so semantic memory works from first boot.
///
/// - **Ollama** → offer to pull [`RECOMMENDED_EMBED_MODEL`] (skipped if already
///   present); point `[embedding]` at the same local server, no key. A declined
///   or failed pull degrades to `None` (semantic memory off) rather than a hard
///   error — the operator can add it later.
/// - **OpenAI** → reuse the operator's key against OpenAI's embeddings endpoint.
/// - **Anthropic** → `None`: Anthropic has no embeddings API. We note it and
///   leave semantic memory off rather than render a broken provider.
async fn decide_embedding(
    provider: Provider,
    api_key: Option<&str>,
    base_url: &str,
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
) -> Result<Option<EmbeddingFields>, String> {
    match provider {
        Provider::Ollama => {
            let present = list_ollama_models(base_url)
                .await
                .unwrap_or_default()
                .iter()
                .any(|m| {
                    m == RECOMMENDED_EMBED_MODEL
                        || m.starts_with(&format!("{RECOMMENDED_EMBED_MODEL}:"))
                });
            if !present {
                writeln!(
                    writer,
                    "\nSemantic memory needs a local embedding model so your agent \
                     can recall what it has learned."
                )
                .map_err(|e| format!("write error: {e}"))?;
                if prompt_yes_no(
                    &format!("Download {RECOMMENDED_EMBED_MODEL} now (recommended)?"),
                    true,
                    reader,
                    writer,
                )? {
                    if let Err(e) =
                        pull_ollama_model(base_url, RECOMMENDED_EMBED_MODEL, writer).await
                    {
                        writeln!(
                            writer,
                            "  (Couldn't pull {RECOMMENDED_EMBED_MODEL}: {e} — leaving \
                             semantic memory off; add an [embedding] section later.)"
                        )
                        .map_err(|e| format!("write error: {e}"))?;
                        return Ok(None);
                    }
                } else {
                    return Ok(None);
                }
            }
            Ok(Some(EmbeddingFields {
                base_url: base_url.to_string(),
                model: RECOMMENDED_EMBED_MODEL.to_string(),
                dimensions: RECOMMENDED_EMBED_DIMENSIONS,
                api_key: None,
            }))
        }
        Provider::OpenAi => Ok(api_key.map(|k| EmbeddingFields {
            base_url: aivyx_config::DEFAULT_EMBEDDING_BASE_URL.to_string(),
            model: aivyx_config::DEFAULT_EMBEDDING_MODEL.to_string(),
            dimensions: aivyx_config::DEFAULT_EMBEDDING_DIMENSIONS,
            api_key: Some(k.to_string()),
        })),
        Provider::Anthropic => {
            writeln!(
                writer,
                "\nNote: Anthropic has no embeddings API, so semantic memory stays \
                 off. Add an [embedding] provider (OpenAI, or a local Ollama running \
                 {RECOMMENDED_EMBED_MODEL}) to enable it."
            )
            .map_err(|e| format!("write error: {e}"))?;
            Ok(None)
        }
    }
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
fn prompt_secret(prompt: &str) -> Result<String, String> {
    rpassword::prompt_password(prompt).map_err(|e| format!("failed to read secret: {e}"))
}

// ---------------------------------------------------------------------------
// Phase 104 — verify-before-write
// ---------------------------------------------------------------------------

/// Hard cap on verify retries before the wizard falls through to
/// a final `Write anyway?` gate. Per Q2(a): verify is a
/// guardrail, not a lock.
const VERIFY_MAX_ATTEMPTS: u32 = 3;

/// Collect a `(model, api_key)` pair for a cloud provider with
/// `GET /v1/models` verify-before-write per Q1(a). On verify
/// failure, re-prompts the implicated field — auth → key, model-
/// not-found → model, network/other → both with a "Write anyway?"
/// escape hatch (Q2(a)).
///
/// Returns `Ok((model, Some(key)))` on success, including the
/// last-resort write-anyway acceptance path. Returns `Err` only
/// when the operator declines write-anyway after exhausting
/// retries — in that case the wizard exits without writing.
///
/// `key_prompt_label` is the secret-prompt label (e.g. `"Anthropic
/// API key: "`); `default_model` is the fallback when neither the
/// operator nor the template supplies one; `template_model` lets
/// a Phase 66 template override the default.
async fn collect_and_verify_cloud(
    provider: VerifyProvider,
    key_prompt_label: &str,
    default_model: &str,
    template_model: Option<&str>,
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
) -> Result<(String, Option<String>), String> {
    let resolved_default = template_model.unwrap_or(default_model);

    // Initial collection — same prompt order as pre-Phase-104.
    let mut model = prompt_line(
        &format!("Model [{resolved_default}]: "),
        reader,
        writer,
    )?;
    if model.is_empty() {
        model = resolved_default.into();
    }
    let mut key = prompt_secret(key_prompt_label)?;
    if key.is_empty() {
        return Err("API key cannot be empty".into());
    }

    for attempt in 1..=VERIFY_MAX_ATTEMPTS {
        writeln!(writer, "Verifying provider…")
            .map_err(|e| format!("write error: {e}"))?;
        writer.flush().map_err(|e| format!("flush error: {e}"))?;

        match verify_provider_credentials(provider, &key, &model).await {
            Ok(()) => {
                writeln!(writer, "Verified ok.")
                    .map_err(|e| format!("write error: {e}"))?;
                return Ok((model, Some(key)));
            }
            Err(VerifyError::Auth(msg)) => {
                writeln!(writer, "Authentication failed: {msg}")
                    .map_err(|e| format!("write error: {e}"))?;
                if attempt == VERIFY_MAX_ATTEMPTS {
                    break;
                }
                writeln!(
                    writer,
                    "Retry {attempt}/{VERIFY_MAX_ATTEMPTS}: re-enter the API key.",
                )
                .map_err(|e| format!("write error: {e}"))?;
                key = prompt_secret(key_prompt_label)?;
                if key.is_empty() {
                    return Err("API key cannot be empty".into());
                }
            }
            Err(VerifyError::ModelNotFound { available, .. }) => {
                writeln!(
                    writer,
                    "Model `{model}` is not available for this account.\n\
                     Available: {available}",
                )
                .map_err(|e| format!("write error: {e}"))?;
                if attempt == VERIFY_MAX_ATTEMPTS {
                    break;
                }
                writeln!(
                    writer,
                    "Retry {attempt}/{VERIFY_MAX_ATTEMPTS}: enter a model name.",
                )
                .map_err(|e| format!("write error: {e}"))?;
                let m = prompt_line("Model: ", reader, writer)?;
                if !m.is_empty() {
                    model = m;
                }
            }
            Err(VerifyError::Network(msg)) | Err(VerifyError::Other(msg)) => {
                writeln!(writer, "Verify failed: {msg}")
                    .map_err(|e| format!("write error: {e}"))?;
                // Ambiguous failure — offer immediate write-anyway
                // before consuming another retry. Operator typing
                // `y` short-circuits the loop.
                if prompt_yes_no(
                    "Write the config without verification?",
                    false,
                    reader,
                    writer,
                )? {
                    return Ok((model, Some(key)));
                }
                if attempt == VERIFY_MAX_ATTEMPTS {
                    break;
                }
                writeln!(
                    writer,
                    "Retry {attempt}/{VERIFY_MAX_ATTEMPTS}: re-enter key and model.",
                )
                .map_err(|e| format!("write error: {e}"))?;
                key = prompt_secret(key_prompt_label)?;
                if key.is_empty() {
                    return Err("API key cannot be empty".into());
                }
                let m = prompt_line(
                    &format!("Model [{model}]: "),
                    reader,
                    writer,
                )?;
                if !m.is_empty() {
                    model = m;
                }
            }
        }
    }

    // Exhausted retries — last-resort write-anyway gate.
    writeln!(
        writer,
        "Verification failed after {VERIFY_MAX_ATTEMPTS} attempts.",
    )
    .map_err(|e| format!("write error: {e}"))?;
    if prompt_yes_no("Write the config anyway?", false, reader, writer)? {
        Ok((model, Some(key)))
    } else {
        Err("provider verification failed; aivyx.toml not written".into())
    }
}

/// Ask a yes/no question. `default` is the answer when the user
/// presses Enter. Returns `true` for yes, `false` for no.
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

/// What the wizard should do about installing a background service,
/// decided from whether one's already installed and (if not) the
/// operator's answers. Pure with respect to OS calls -- doesn't itself
/// call `daemon_service::run_install`/`installed_unit_path`/`is_active`,
/// so it's testable with a scripted reader/writer like every other
/// `prompt_yes_no`-driven decision in this file.
enum ServiceInstallDecision {
    /// Already installed -- nothing to ask. `active` mirrors
    /// `daemon_service::is_active()`'s own `None` = "couldn't
    /// determine" convention.
    AlreadyInstalled { active: Option<bool> },
    /// Operator declined -- fall back to the existing passive hint.
    Declined,
    /// Operator wants it installed, with or without the Studio.
    Install { web_ui: bool },
}

fn decide_service_install(
    already_installed: bool,
    active: Option<bool>,
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
) -> Result<ServiceInstallDecision, String> {
    if already_installed {
        return Ok(ServiceInstallDecision::AlreadyInstalled { active });
    }
    let install_now = prompt_yes_no(
        "Install as a background service now? (survives logout/reboot)",
        true,
        reader,
        writer,
    )?;
    if !install_now {
        return Ok(ServiceInstallDecision::Declined);
    }
    let web_ui = prompt_yes_no("Also serve the Studio web UI?", false, reader, writer)?;
    Ok(ServiceInstallDecision::Install { web_ui })
}

/// Runs the full service-install offer: decide, then (if the operator
/// said yes) actually install via `run_install`. `run_install` is
/// injected so tests can exercise the success/failure print paths
/// without a real systemd/launchd call -- production passes
/// `crate::daemon_service::run_install` itself, whose `fn(bool, bool)
/// -> Result<(), String>` signature matches this parameter directly.
///
/// Never returns `Err` for an install failure -- by the time this runs,
/// `aivyx.toml` is already written; a failed service install is a
/// separate, later concern, not a wizard failure. Only genuinely
/// propagates `Err` if writing the prompt/output itself fails (matches
/// `prompt_yes_no`'s own convention elsewhere in this file).
fn offer_service_install(
    already_installed: bool,
    active: Option<bool>,
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
    run_install: impl FnOnce(bool, bool) -> Result<(), String>,
) -> Result<(), String> {
    match decide_service_install(already_installed, active, reader, writer)? {
        ServiceInstallDecision::AlreadyInstalled { active } => {
            let status = match active {
                Some(true) => "running",
                Some(false) => "not currently running",
                None => "status unknown",
            };
            writeln!(
                writer,
                "  (background service already installed — {status}; \
                 `aivyx doctor` has details)"
            )
            .map_err(|write_err| format!("write error: {write_err}"))?;
        }
        ServiceInstallDecision::Declined => {
            writeln!(
                writer,
                "  aivyx daemon install       — run it as a background service (survives logout/reboot)"
            )
            .map_err(|write_err| format!("write error: {write_err}"))?;
        }
        ServiceInstallDecision::Install { web_ui } => {
            writeln!(
                writer,
                "  (this sets a new passphrase for the unattended service — set \
                 AIVYX_PASSPHRASE beforehand to skip the prompt)"
            )
            .map_err(|write_err| format!("write error: {write_err}"))?;
            match run_install(web_ui, true) {
                Ok(()) => {
                    writeln!(
                        writer,
                        "  Installed as a background service — `aivyx doctor` has details"
                    )
                    .map_err(|write_err| format!("write error: {write_err}"))?;
                }
                Err(e) => {
                    writeln!(
                        writer,
                        "  Couldn't install as a background service: {e}\n  \
                         (if this left a partial install behind, `aivyx daemon uninstall` \
                         cleans it up)"
                    )
                    .map_err(|write_err| format!("write error: {write_err}"))?;
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// TOML generation
// ---------------------------------------------------------------------------

/// The provider the user selected in the wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Ollama,
    Anthropic,
    OpenAi,
}

/// Chapter W — the optional onboarding Persona/Skills seed collected by the
/// wizard. Renders a `[persona_seed]` section the daemon plants on the persona
/// chain at first boot (iff empty). All-empty ⇒ no section emitted, and the
/// agent's Persona simply starts empty and grows from use as before.
#[derive(Default)]
struct PersonaSeedFields {
    learned_context: Vec<String>,
    communication_adaptations: Vec<String>,
    character_traits: Vec<String>,
    relationship_milestones: Vec<String>,
    skills: Vec<SeedSkillFields>,
}

/// One starter skill captured by the wizard (`[[persona_seed.skill]]`).
struct SeedSkillFields {
    name: String,
    trigger: String,
    procedure: String,
}

impl PersonaSeedFields {
    /// `true` when the operator declared any seed content.
    fn has_content(&self) -> bool {
        !self.learned_context.is_empty()
            || !self.communication_adaptations.is_empty()
            || !self.character_traits.is_empty()
            || !self.relationship_milestones.is_empty()
            || !self.skills.is_empty()
    }
}

/// Captures all wizard answers needed to render `aivyx.toml`.
struct InitConfig {
    provider: Provider,
    model: String,
    api_key: Option<String>,
    storage_path: String,
    fs_root: String,
    /// Chapter N — the operator-chosen access level. `Sandbox` (default)
    /// renders exactly as before (`[fs] root`); expanded levels render an
    /// `[access]` section instead.
    access_level: AccessLevel,
    /// Chapter N — confirm-first posture for expanded levels.
    confirm_destructive: bool,
    /// Phase 46: enable bundled web search MCP server.
    enable_web_search: bool,
    /// Phase 57 / Phase 181 — the full P13 Profile, collected by
    /// the guided identity builder. A field is `None` / empty
    /// when undeclared; the renderer only emits a `[profile]`
    /// section when at least one field is set. Phase 181 expanded
    /// this from three fields to all six the substrate supports.
    profile_assistant_name: Option<String>,
    /// Who the operator is — the other half of the relationship.
    profile_operator_profile: Option<String>,
    profile_communication_style: Option<String>,
    /// 1–3 use-case archetypes (was a single `Option<String>`
    /// pre-Phase-181).
    profile_primary_use_cases: Vec<String>,
    /// Voice-layer judgment defaults.
    profile_behavioral_preferences: Vec<String>,
    /// The lines it must never cross — the trust boundaries.
    profile_behavioral_constraints: Vec<String>,
    /// Chapter W — the optional onboarding Persona/Skills seed.
    persona_seed: PersonaSeedFields,
    /// Chapter Engram — the embedding provider that makes semantic memory
    /// work out of the box. `Some` renders an `[embedding]` section (local
    /// Ollama on the local path, OpenAI on the cloud path); `None` leaves
    /// semantic memory off (e.g. Anthropic-only, which has no embeddings API,
    /// or a declined/failed local pull).
    embedding: Option<EmbeddingFields>,
}

/// Chapter Engram — the `[embedding]` settings the wizard resolved for the
/// chosen provider.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EmbeddingFields {
    base_url: String,
    model: String,
    dimensions: usize,
    /// `Some` for the cloud (OpenAI) path; `None` for local Ollama, which
    /// needs no key.
    api_key: Option<String>,
}

/// Render a ready-to-use `aivyx.toml` from the wizard answers.
fn render_toml(cfg: &InitConfig) -> String {
    let mut out = String::from("# Generated by `aivyx init`\n\n");

    // [agent] section
    let provider_str = match cfg.provider {
        Provider::Ollama => "ollama",
        Provider::Anthropic => "anthropic",
        Provider::OpenAi => "openai",
    };
    out.push_str(&format!(
        "[agent]\nprovider = \"{provider_str}\"\nmodel = \"{}\"\n",
        cfg.model,
    ));

    // Provider-specific section (API key)
    match cfg.provider {
        Provider::Anthropic => {
            if let Some(key) = &cfg.api_key {
                out.push_str(&format!(
                    "\n[anthropic]\napi_key = \"{key}\"\n"
                ));
            }
        }
        Provider::OpenAi => {
            if let Some(key) = &cfg.api_key {
                out.push_str(&format!(
                    "\n[openai]\napi_key = \"{key}\"\n"
                ));
            }
        }
        Provider::Ollama => {
            // No API key needed for Ollama.
        }
    }

    // [fs] / [access] section (Chapter N). Sandbox renders the legacy
    // `[fs] root` unchanged; expanded levels render `[access]` instead
    // (the level derives fs_root, with an explicit `root` for workspace).
    match cfg.access_level {
        AccessLevel::Sandbox => {
            out.push_str(&format!("\n[fs]\nroot = \"{}\"\n", cfg.fs_root));
        }
        level => {
            out.push_str(&format!("\n[access]\nlevel = \"{level}\"\n"));
            if matches!(level, AccessLevel::Workspace | AccessLevel::Custom) {
                out.push_str(&format!("root = \"{}\"\n", cfg.fs_root));
            }
            out.push_str(&format!(
                "confirm_destructive = {}\n",
                cfg.confirm_destructive
            ));
        }
    }
    // [storage] section
    out.push_str(&format!(
        "\n[storage]\npath = \"{}\"\n",
        cfg.storage_path,
    ));

    // Chapter Engram — `[embedding]` + `[memory] profile`. Shared with the
    // `--template` path (backlog #3) so both render this identically.
    out.push_str(&render_embedding_section(cfg));

    // Phase 46: bundled web search MCP server. Shared with the `--template`
    // path (backlog #3).
    out.push_str(&render_web_search_section(cfg));

    // Phase 57: Profile section per Q4(c). Only emit when the
    // operator customized at least one field. A blank-everywhere
    // pass leaves the substrate at its synthesized default — same
    // behavior as pre-Phase-57 configs.
    if cfg.profile_assistant_name.is_some()
        || cfg.profile_operator_profile.is_some()
        || cfg.profile_communication_style.is_some()
        || !cfg.profile_primary_use_cases.is_empty()
        || !cfg.profile_behavioral_preferences.is_empty()
        || !cfg.profile_behavioral_constraints.is_empty()
    {
        out.push_str("\n[profile]\n");
        if let Some(name) = &cfg.profile_assistant_name {
            out.push_str(&format!("assistant_name = \"{}\"\n", escape_toml_string(name)));
        }
        if let Some(who) = &cfg.profile_operator_profile {
            out.push_str(&format!(
                "operator_profile = \"{}\"\n",
                escape_toml_string(who),
            ));
        }
        if let Some(style) = &cfg.profile_communication_style {
            out.push_str(&format!(
                "communication_style = \"{}\"\n",
                escape_toml_string(style),
            ));
        }
        if !cfg.profile_primary_use_cases.is_empty() {
            out.push_str(&format!(
                "primary_use_cases = {}\n",
                toml_string_array(&cfg.profile_primary_use_cases),
            ));
        }
        if !cfg.profile_behavioral_preferences.is_empty() {
            out.push_str(&format!(
                "behavioral_preferences = {}\n",
                toml_string_array(&cfg.profile_behavioral_preferences),
            ));
        }
        if !cfg.profile_behavioral_constraints.is_empty() {
            out.push_str(&format!(
                "behavioral_constraints = {}\n",
                toml_string_array(&cfg.profile_behavioral_constraints),
            ));
        }
    }

    // Chapter W — the onboarding Persona/Skills seed. Shared with the
    // `--template` path (backlog #3).
    out.push_str(&render_persona_seed_section(cfg));

    // Phase 180 — secure-by-default. New configs request the
    // bundled sandbox preset: tool processes are OS-isolated
    // (bubblewrap / firejail, auto-detected) without further
    // setup. Set to "none" to opt out, or add a per-tool
    // `disable_sandbox = true`.
    out.push_str("\n[sandbox]\n");
    out.push_str("default_backend = \"auto\"\n");

    out
}

/// Chapter Engram — render `[embedding]` + `[memory] profile = smart`. Empty
/// string when the wizard resolved no embedding provider (semantic memory stays
/// off — the profile would be inert anyway).
///
/// Shared by `render_toml` and the `--template` path (backlog #3): a
/// `--template` init used to skip semantic memory entirely. Without
/// `[embedding]` the daemon never builds the auto-recall pipeline, so a fresh
/// templated agent had no semantic recall at all. `smart` arms graph-augmented
/// recall + the (capped) wiki / typed-graph sweeps; written explicitly here
/// rather than as a compiled default so only new configs opt in (the cloud
/// sweeps spend tokens — flipping the default would surprise-bill upgrades).
fn render_embedding_section(cfg: &InitConfig) -> String {
    let Some(emb) = &cfg.embedding else {
        return String::new();
    };
    let mut out = format!(
        "\n[embedding]\nbase_url = \"{}\"\nmodel = \"{}\"\ndimensions = {}\n",
        emb.base_url, emb.model, emb.dimensions,
    );
    if let Some(key) = &emb.api_key {
        out.push_str(&format!("api_key = \"{key}\"\n"));
    }
    out.push_str("\n[memory]\nprofile = \"smart\"\n");
    out
}

/// Phase 46 — the bundled web-search MCP server block. Empty when the operator
/// declined web search. Shared by `render_toml` and the `--template` path.
fn render_web_search_section(cfg: &InitConfig) -> String {
    if !cfg.enable_web_search {
        return String::new();
    }
    String::from(
        "\n[[mcp_server]]\n\
         name = \"web-search\"\n\
         command = \"aivyx\"\n\
         args = [\"mcp-server\", \"web-search\"]\n\
         bundled = true\n",
    )
}

/// Chapter W — render the onboarding `[persona_seed]` + `[[persona_seed.skill]]`
/// blocks. Empty string when the operator declared nothing. The daemon plants
/// this on the persona chain at first boot (iff empty); editing it later has no
/// effect (the chain is authoritative once seeded).
///
/// Shared by `render_toml` and the `--template` path (backlog #3): a
/// `--template` init used to silently drop the operator's persona answers.
fn render_persona_seed_section(cfg: &InitConfig) -> String {
    let seed = &cfg.persona_seed;
    if !seed.has_content() {
        return String::new();
    }
    let mut out = String::from("\n[persona_seed]\n");
    if !seed.learned_context.is_empty() {
        out.push_str(&format!(
            "learned_context = {}\n",
            toml_string_array(&seed.learned_context),
        ));
    }
    if !seed.communication_adaptations.is_empty() {
        out.push_str(&format!(
            "communication_adaptations = {}\n",
            toml_string_array(&seed.communication_adaptations),
        ));
    }
    if !seed.character_traits.is_empty() {
        out.push_str(&format!(
            "character_traits = {}\n",
            toml_string_array(&seed.character_traits),
        ));
    }
    if !seed.relationship_milestones.is_empty() {
        out.push_str(&format!(
            "relationship_milestones = {}\n",
            toml_string_array(&seed.relationship_milestones),
        ));
    }
    for sk in &seed.skills {
        out.push_str(&format!(
            "\n[[persona_seed.skill]]\nname = \"{}\"\ntrigger = \"{}\"\nprocedure = \"{}\"\n",
            escape_toml_string(&sk.name),
            escape_toml_string(&sk.trigger),
            escape_toml_string(&sk.procedure),
        ));
    }
    out
}

/// Minimal TOML basic-string escape for wizard-supplied free text.
/// Operators typing a quote or backslash should not break the
/// generated TOML — the renderer escapes both, plus newlines for
/// the off-chance that a paste includes them.
fn escape_toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

/// Phase 181 — render a `Vec<String>` as a TOML array of escaped
/// basic strings: `["a", "b"]`. Used for the Profile list fields
/// (`primary_use_cases`, `behavioral_preferences`,
/// `behavioral_constraints`).
fn toml_string_array(items: &[String]) -> String {
    let inner = items
        .iter()
        .map(|s| format!("\"{}\"", escape_toml_string(s)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

// ---------------------------------------------------------------------------
// Default starter routines (cron-scheduled). Read-only / observational
// prompts that run unattended; each is self-contained and explicitly forbids
// destructive actions. Verified live on real hardware before shipping.
// ---------------------------------------------------------------------------

const ROUTINE_ENVIRONMENT_REVIEW: &str = "Perform your daily environment review, strictly read-only — never modify, delete, move, or run destructive commands, and stay within your access scope. Check memory under the topic 'environment-baseline'. If no baseline exists, survey your accessible environment concisely — your workspace and key directories, the tools available to you, and a brief system summary — and save it to memory under 'environment-baseline'. If a baseline exists, compare the current state to it, journal a short note of anything new or notable to your workspace, and update the baseline. Be concise.";
const ROUTINE_NIGHTLY_REFLECTION: &str = "Nightly reflection. FIRST read your actual record — recall your recent memories and read your workspace journal. Then, based ONLY on what you actually find there, consolidate the important facts, tidy stale or duplicate memories, and note any skills or operator preferences worth refining. Journal a brief reflection. If little happened, say so briefly — never invent activity you didn't find. Keep it short.";
const ROUTINE_HEALTH_CHECK: &str = "Run a quick self-health check: confirm your model is responding and that a trivial tool call works. If everything is healthy reply with a short OK. Only raise an alert if something is actually wrong.";
const ROUTINE_WEEKLY_DIGEST: &str = "Weekly digest. FIRST read your actual record before writing anything: recall your recent memories, read your workspace journal, and check for pending persona or skill proposals awaiting review. Then write a short, friendly briefing of ONLY what you genuinely found there — what you actually learned or worked on, and any proposals to review. If the record is empty or thin, say so plainly (e.g. \"nothing notable to report yet\"). Never invent, infer, or pad the digest with activity you did not actually find in memory or the journal.";
const ROUTINE_TREND_SCAN: &str = "Run a trend-scan across the operator's interest areas (see your profile and primary use cases). You MUST actually search the web first — call your web search tool for recent, reputable sources; do not answer from memory or prior knowledge alone. Cross-reference the key points across several results, separate solid facts from speculation, save the distilled findings to memory under a clear topic, and journal a short digest leading with whatever is genuinely new, with links. If the search returns nothing useful, say so plainly and stop — never fabricate findings, sources, or links.";

/// Render the default starter routines as `[[schedule]]` blocks.
///
/// Gating (operator decisions, 2026-06): the four core routines are ENABLED on
/// a local (Ollama) provider and written present-but-DISABLED on a cloud
/// provider — discoverable, a one-line flip to turn on, and cost-aware since
/// cloud schedules spend tokens. The opt-in `trend-scan` additionally requires
/// web search (its tool comes from the bundled web-search MCP server, which is
/// only configured when the operator enabled web search).
fn render_default_schedules(cfg: &InitConfig) -> String {
    let local = matches!(cfg.provider, Provider::Ollama);
    let core_enabled = local;
    let trend_enabled = local && cfg.enable_web_search;
    let b = |on: bool| if on { "true" } else { "false" };

    let mut out = String::new();
    out.push_str(
        "\n# --------------------------------------------------------------------------\n\
         # Default starter routines (cron = \"sec min hour dom mon dow\", local time).\n\
         # Read-only / observational, scoped to the access level. Enabled on a local\n\
         # provider; written disabled on a cloud provider (they spend tokens) — flip\n\
         # `enabled = true` to turn one on.\n\
         # --------------------------------------------------------------------------\n",
    );

    let mut emit = |name: &str, cron: &str, prompt: &str, enabled: bool, notify: &str, report_kind: Option<&str>| {
        out.push_str(&format!(
            "\n[[schedule]]\n\
             name = \"{}\"\n\
             cron = \"{}\"\n\
             role = \"default\"\n\
             prompt = \"{}\"\n\
             enabled = {}\n\
             wrap_mission = true\n\
             notify_when = \"{}\"\n",
            name,
            cron,
            escape_toml_string(prompt),
            b(enabled),
            notify,
        ));
        // Chapter Ledger — a deterministic daemon-assembled report (no LLM in
        // the content path, so it can't confabulate). The `prompt` above is kept
        // only as human documentation; the scheduler ignores it for a report.
        if let Some(kind) = report_kind {
            out.push_str(&format!("report_kind = \"{kind}\"\n"));
        }
    };

    emit("environment-review", "0 0 7 * * *", ROUTINE_ENVIRONMENT_REVIEW, core_enabled, "on_completed_non_empty", None);
    emit("nightly-reflection", "0 0 2 * * *", ROUTINE_NIGHTLY_REFLECTION, core_enabled, "on_completed_non_empty", None);
    emit("health-check", "0 0 */6 * * *", ROUTINE_HEALTH_CHECK, core_enabled, "on_failed", None);
    // Chapter Ledger (#6 fix) — the weekly digest is now a deterministic report
    // built from the memory substrate, not an LLM turn that confabulates.
    emit("weekly-digest", "0 0 8 * * 1", ROUTINE_WEEKLY_DIGEST, core_enabled, "on_completed_non_empty", Some("digest"));
    // Chapter Ledger (#6 grounding-gate) — trend-scan is generative web
    // synthesis, so it can't be made deterministic; instead gate its delivery
    // on `on_completed_grounded` — if the turn made NO tool calls (no real
    // search), its "findings" are fabricated and are never broadcast.
    emit("trend-scan", "0 30 7 * * *", ROUTINE_TREND_SCAN, trend_enabled, "on_completed_grounded", None);

    // 2026-07-04 self-learning dogfood (#2) — a fresh install never got a
    // `[[reflection_schedule]]`, so the whole Phase-70 reflection-pass
    // family (persona proposals, consolidation, Whetstone skill
    // refinement, Praxis skill authoring) never fired: the routine named
    // "nightly-reflection" above is a generic prompt TURN, not the
    // reflection scheduler. Plant a real one — daily, off-peak, and
    // skip-when-idle so a quiet day costs nothing. Same local-on /
    // cloud-off posture as the routines above.
    out.push_str(&format!(
        "\n# The reflection scheduler — drives the self-learning passes \
         (persona\n\
         # proposals, skill refinement/authoring when those are enabled). \
         Distinct\n\
         # from the \"nightly-reflection\" routine above, which is a plain \
         prompt turn.\n\
         [[reflection_schedule]]\n\
         name = \"reflection\"\n\
         cron = \"0 30 2 * * *\"\n\
         enabled = {}\n\
         skip_when_idle = true\n",
        b(core_enabled),
    ));

    out
}

// ---------------------------------------------------------------------------
// Phase 181 — the guided first-launch identity builder.
// ---------------------------------------------------------------------------

/// The six P13 Profile fields the guided builder collects. The
/// confidant dimension lives in `operator_profile` (who you are to
/// each other), `communication_style` (the voice), and
/// `behavioral_constraints` (the lines it must never cross).
#[derive(Debug, Clone, Default, PartialEq)]
struct IdentityFields {
    assistant_name: Option<String>,
    operator_profile: Option<String>,
    communication_style: Option<String>,
    primary_use_cases: Vec<String>,
    behavioral_preferences: Vec<String>,
    behavioral_constraints: Vec<String>,
}

fn opt(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Split a comma-separated wizard list into trimmed, non-empty items —
/// **but not on commas inside parentheses/brackets/braces** (backlog #7). So
/// an item like `Research my passions (flying, food, coffee)` stays a single
/// item instead of being shredded into three. Unbalanced delimiters degrade
/// gracefully (depth never goes negative; trailing text is flushed).
fn split_list(s: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut depth: i32 = 0;
    for ch in s.chars() {
        match ch {
            '(' | '[' | '{' => {
                depth += 1;
                cur.push(ch);
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                cur.push(ch);
            }
            ',' if depth == 0 => {
                let t = cur.trim();
                if !t.is_empty() {
                    items.push(t.to_string());
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    let t = cur.trim();
    if !t.is_empty() {
        items.push(t.to_string());
    }
    items
}

/// Run the guided identity builder. Offers an LLM-assisted draft
/// (only when a `provider` is available + the operator opts in);
/// otherwise — or on any LLM error — collects the six fields with
/// guided manual prompts. Always returns a complete set the
/// operator can confirm in the preview step. Local-first: never
/// requires an LLM.
async fn run_identity_builder(
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
    provider: Option<&Arc<dyn LlmProvider>>,
    model: &str,
    defaults: &TemplateDefaults,
) -> Result<IdentityFields, String> {
    let w = |writer: &mut dyn IoWrite, s: &str| -> Result<(), String> {
        writeln!(writer, "{s}").map_err(|e| format!("write error: {e}"))
    };
    w(writer, "\n— Let's shape your assistant —")?;
    w(
        writer,
        "This is who you'll be working with. You can refine it \
         any time later in aivyx.toml.",
    )?;

    let assisted = provider.is_some()
        && prompt_yes_no(
            "Would you like help? I'll ask a few questions and \
             draft an identity you can edit",
            true,
            reader,
            writer,
        )?;

    if assisted {
        let answers = IdentityAnswers {
            intent: prompt_line(
                "\nIn your own words, what do you want this \
                 assistant to be for you?\n> ",
                reader,
                writer,
            )?,
            role: prompt_line(
                "What role should it play? (collaborator / coach / \
                 confidant / assistant — or your own words)\n> ",
                reader,
                writer,
            )?,
            tone: prompt_line(
                "How should it talk to you? (tone & warmth)\n> ",
                reader,
                writer,
            )?,
            never_do: prompt_line(
                "Is there anything it should never do?\n> ",
                reader,
                writer,
            )?,
        };
        w(writer, "\nDrafting your assistant's identity…")?;
        match draft_identity(provider.unwrap(), model, &answers).await {
            Some(draft) => {
                w(
                    writer,
                    "\nHere's a draft — press Enter to keep each \
                     line, or type a replacement:",
                )?;
                return review_draft(reader, writer, draft);
            }
            None => {
                w(
                    writer,
                    "\n(I couldn't reach the model for a draft — \
                     let's do it together instead.)",
                )?;
            }
        }
    }

    manual_identity(reader, writer, defaults)
}

/// Review/edit a drafted profile field-by-field. The operator is
/// always the author of record.
fn review_draft(
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
    draft: DraftedProfile,
) -> Result<IdentityFields, String> {
    Ok(IdentityFields {
        assistant_name: review_scalar(
            reader,
            writer,
            "Name",
            draft.assistant_name,
        )?,
        operator_profile: review_scalar(
            reader,
            writer,
            "Who you are",
            draft.operator_profile,
        )?,
        communication_style: review_scalar(
            reader,
            writer,
            "How I'll talk",
            draft.communication_style,
        )?,
        primary_use_cases: review_list(
            reader,
            writer,
            "What I'm here for",
            draft.primary_use_cases,
        )?,
        behavioral_preferences: review_list(
            reader,
            writer,
            "What I'll tend to do",
            draft.behavioral_preferences,
        )?,
        behavioral_constraints: review_list(
            reader,
            writer,
            "What I'll never do",
            draft.behavioral_constraints,
        )?,
    })
}

fn review_scalar(
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
    label: &str,
    drafted: Option<String>,
) -> Result<Option<String>, String> {
    let shown = drafted.as_deref().unwrap_or("(none)");
    let input =
        prompt_line(&format!("  {label} [{shown}]: "), reader, writer)?;
    Ok(if input.is_empty() {
        drafted
    } else {
        opt(input)
    })
}

fn review_list(
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
    label: &str,
    drafted: Vec<String>,
) -> Result<Vec<String>, String> {
    let shown = if drafted.is_empty() {
        "(none)".to_string()
    } else {
        drafted.join(", ")
    };
    let input = prompt_line(
        &format!("  {label} [{shown}] (comma-separated): "),
        reader,
        writer,
    )?;
    Ok(if input.is_empty() {
        drafted
    } else {
        split_list(&input)
    })
}

/// The offline / declined / draft-failed path — guided manual
/// prompts for all six fields, pre-filled from the template when
/// one was chosen.
fn manual_identity(
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
    defaults: &TemplateDefaults,
) -> Result<IdentityFields, String> {
    writeln!(
        writer,
        "\nLet's set up the identity together (press Enter to skip \
         any line):"
    )
    .map_err(|e| format!("write error: {e}"))?;

    let name_default = defaults.assistant_name.as_deref().unwrap_or("Aivyx");
    let assistant_name = opt(prompt_line(
        &format!("  Name [{name_default}]: "),
        reader,
        writer,
    )?)
    .or_else(|| defaults.assistant_name.clone());

    let operator_profile = opt(prompt_line(
        "  Who are you? (role, expertise, what you work on)\n> ",
        reader,
        writer,
    )?);

    let style_default =
        defaults.communication_style.as_deref().unwrap_or("");
    let communication_style = opt(prompt_line(
        &format!(
            "  How should it talk to you? (e.g. 'warm but concise')\
             {}\n> ",
            if style_default.is_empty() {
                String::new()
            } else {
                format!(" [{style_default}]")
            }
        ),
        reader,
        writer,
    )?)
    .or_else(|| defaults.communication_style.clone());

    let uc_raw = prompt_line(
        "  Primary use cases (1-3, comma-separated)\n> ",
        reader,
        writer,
    )?;
    let primary_use_cases = if uc_raw.is_empty() {
        defaults.primary_use_case.clone().into_iter().collect()
    } else {
        split_list(&uc_raw)
    };

    let behavioral_preferences = split_list(&prompt_line(
        "  Things it should generally do (comma-separated, optional)\n> ",
        reader,
        writer,
    )?);

    let behavioral_constraints = split_list(&prompt_line(
        "  Things it should NEVER do (comma-separated, optional)\n> ",
        reader,
        writer,
    )?);

    Ok(IdentityFields {
        assistant_name,
        operator_profile,
        communication_style,
        primary_use_cases,
        behavioral_preferences,
        behavioral_constraints,
    })
}

impl IdentityFields {
    /// Re-view the collected fields as a draft (for the preview's
    /// "edit" path — the operator tweaks what they already have).
    fn as_draft(&self) -> DraftedProfile {
        DraftedProfile {
            assistant_name: self.assistant_name.clone(),
            operator_profile: self.operator_profile.clone(),
            communication_style: self.communication_style.clone(),
            primary_use_cases: self.primary_use_cases.clone(),
            behavioral_preferences: self.behavioral_preferences.clone(),
            behavioral_constraints: self.behavioral_constraints.clone(),
        }
    }
}

/// Phase 181 — the "meet your assistant" preview. A warm,
/// first-person summary of the identity the operator just shaped,
/// rendered before anything is written. Pure + unit-testable.
fn render_identity_summary(f: &IdentityFields) -> String {
    let name = f.assistant_name.as_deref().unwrap_or("Aivyx");
    let mut s = String::from("\n— Meet your assistant —\n\n");
    s.push_str(&format!("  I'm {name}.\n"));
    if let Some(who) = &f.operator_profile {
        s.push_str(&format!("  You're {who}.\n"));
    }
    if let Some(style) = &f.communication_style {
        s.push_str(&format!("  I'll talk {style}.\n"));
    }
    if !f.primary_use_cases.is_empty() {
        s.push_str(&format!(
            "  I'm here for {}.\n",
            f.primary_use_cases.join(", ")
        ));
    }
    if !f.behavioral_preferences.is_empty() {
        s.push_str(&format!(
            "  I'll tend to {}.\n",
            f.behavioral_preferences.join(", ")
        ));
    }
    if !f.behavioral_constraints.is_empty() {
        s.push_str(&format!(
            "  I'll never {}.\n",
            f.behavioral_constraints.join(", ")
        ));
    }
    s
}

/// The full identity step: collect → preview → confirm / edit /
/// restart. Loops until the operator confirms.
async fn confirm_identity(
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
    provider: Option<&Arc<dyn LlmProvider>>,
    model: &str,
    defaults: &TemplateDefaults,
) -> Result<IdentityFields, String> {
    loop {
        let mut fields = run_identity_builder(
            reader, writer, provider, model, defaults,
        )
        .await?;
        loop {
            write!(writer, "{}", render_identity_summary(&fields))
                .map_err(|e| format!("write error: {e}"))?;
            let choice = prompt_choice(
                "\nDoes this feel right?",
                &[
                    "Yes, this is my assistant",
                    "Edit a few details",
                    "Start over",
                ],
                0,
                reader,
                writer,
            )?;
            match choice {
                0 => return Ok(fields),
                1 => {
                    writeln!(
                        writer,
                        "\nEdit each line (press Enter to keep):"
                    )
                    .map_err(|e| format!("write error: {e}"))?;
                    fields =
                        review_draft(reader, writer, fields.as_draft())?;
                }
                // "Start over" → re-run the whole builder.
                _ => break,
            }
        }
    }
}

/// Construct the provider for the optional identity draft from the
/// just-verified wizard selection. `None` on any construction
/// failure (the builder falls back to manual prompts). The draft
/// model is passed separately to `draft_identity`, so the config
/// here only needs credentials / endpoint.
fn build_wizard_provider(
    provider: Provider,
    api_key: Option<&str>,
) -> Option<Arc<dyn LlmProvider>> {
    match provider {
        Provider::Anthropic => {
            use aivyx_llm::anthropic::{AnthropicConfig, AnthropicProvider};
            let key = api_key?;
            AnthropicProvider::new(AnthropicConfig::new(key.to_string()))
                .ok()
                .map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
        }
        Provider::OpenAi => {
            use aivyx_llm::openai::{OpenAiConfig, OpenAiProvider};
            let key = api_key?;
            OpenAiProvider::new(OpenAiConfig::new(key.to_string()))
                .ok()
                .map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
        }
        Provider::Ollama => {
            use aivyx_llm::ollama::{OllamaConfig, OllamaProvider};
            OllamaProvider::new(OllamaConfig::default_local())
                .ok()
                .map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point (stub — wired end-to-end in Task 5)
// ---------------------------------------------------------------------------

/// Compute default paths for the wizard. Uses `$HOME` to derive
/// sensible defaults matching `aivyx-config` resolution logic.
fn default_paths() -> (String, String) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let fs_root = format!("{home}/aivyx-sandbox");
    let storage_path = format!("{home}/.local/share/aivyx/store.redb");
    (fs_root, storage_path)
}

/// Write `contents` to `path` with 0600 permissions on Unix.
fn write_config(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms)
            .map_err(|e| format!("failed to set permissions on {}: {e}", path.display()))?;
    }

    Ok(())
}

/// Phase 66 — values extracted from a starter template that the
/// wizard uses as prompt defaults AND as the splice-back base for
/// the final `aivyx.toml`. Each `Option` field is `None` when
/// the corresponding TOML key wasn't present in the template —
/// the wizard falls back to its existing hardcoded default in
/// that case.
struct TemplateDefaults {
    provider: Option<String>,
    model: Option<String>,
    fs_root: Option<String>,
    storage_path: Option<String>,
    assistant_name: Option<String>,
    primary_use_case: Option<String>,
    communication_style: Option<String>,
    /// Parsed template document. `Some` when a template was
    /// supplied; the wizard splices its answers into this
    /// document and writes it back. `None` when no template was
    /// supplied; the wizard uses the existing `render_toml`
    /// synthesis path.
    template_doc: Option<toml_edit::DocumentMut>,
    /// Source-of-truth name for diagnostics + the success banner.
    template_name: Option<String>,
}

impl TemplateDefaults {
    fn empty() -> Self {
        Self {
            provider: None,
            model: None,
            fs_root: None,
            storage_path: None,
            assistant_name: None,
            primary_use_case: None,
            communication_style: None,
            template_doc: None,
            template_name: None,
        }
    }

    fn from_template(
        template: &super::init_templates::Template,
    ) -> Result<Self, String> {
        let doc: toml_edit::DocumentMut = template.toml_content.parse().map_err(
            |e: toml_edit::TomlError| {
                format!(
                    "template `{}` is not valid TOML: {e}",
                    template.name,
                )
            },
        )?;

        let provider = doc
            .get("agent")
            .and_then(|a| a.get("provider"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let model = doc
            .get("agent")
            .and_then(|a| a.get("model"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let fs_root = doc
            .get("fs")
            .and_then(|a| a.get("root"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let storage_path = doc
            .get("storage")
            .and_then(|a| a.get("path"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let assistant_name = doc
            .get("profile")
            .and_then(|a| a.get("assistant_name"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let primary_use_case = doc
            .get("profile")
            .and_then(|a| a.get("primary_use_cases"))
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.iter().next())
            .and_then(|v| v.as_str())
            .map(String::from);
        let communication_style = doc
            .get("profile")
            .and_then(|a| a.get("communication_style"))
            .and_then(|v| v.as_str())
            .map(String::from);

        Ok(Self {
            provider,
            model,
            fs_root,
            storage_path,
            assistant_name,
            primary_use_case,
            communication_style,
            template_doc: Some(doc),
            template_name: Some(template.name.clone()),
        })
    }

}

/// Phase 66 — splice the wizard's answers back into the template
/// `DocumentMut` and serialize. Preserves every comment, every
/// role declaration, every MCP server block, every commented-out
/// section in the template — only the wizard-controlled keys
/// (provider, model, paths, profile fields, api key) are
/// overwritten.
fn render_with_template(
    cfg: &InitConfig,
    template_name: &str,
    mut doc: toml_edit::DocumentMut,
) -> String {
    use toml_edit::value;

    // [agent] provider + model
    let provider_str = match cfg.provider {
        Provider::Ollama => "ollama",
        Provider::Anthropic => "anthropic",
        Provider::OpenAi => "openai",
    };
    doc["agent"]["provider"] = value(provider_str);
    doc["agent"]["model"] = value(cfg.model.as_str());

    // Provider-specific API key. Wipe the inactive provider's
    // api_key block to avoid the template's "alternative
    // provider commented out" comments leaving stale keys.
    match cfg.provider {
        Provider::Anthropic => {
            if let Some(key) = &cfg.api_key {
                doc["anthropic"]["api_key"] = value(key.as_str());
            }
        }
        Provider::OpenAi => {
            if let Some(key) = &cfg.api_key {
                doc["openai"]["api_key"] = value(key.as_str());
            }
        }
        Provider::Ollama => {} // no key needed
    }

    // [fs] + [storage]
    doc["fs"]["root"] = value(cfg.fs_root.as_str());
    doc["storage"]["path"] = value(cfg.storage_path.as_str());

    // Phase 180 — secure-by-default. Every template-generated
    // config requests the bundled sandbox preset for tool
    // processes (auto-detected bubblewrap / firejail). An explicit
    // `Item::Table` renders the readable `[sandbox]` header form
    // (a plain auto-vivified assignment renders an inline table).
    let mut sandbox_tbl = toml_edit::Table::new();
    sandbox_tbl["default_backend"] = value("auto");
    doc["sandbox"] = toml_edit::Item::Table(sandbox_tbl);

    // [profile] fields. Only update keys the operator actually
    // customized; leave the template's defaults otherwise. The
    // template's assistant_name etc. is already a string the
    // operator may have wanted to keep.
    if let Some(name) = &cfg.profile_assistant_name {
        doc["profile"]["assistant_name"] = value(name.as_str());
    }
    if let Some(who) = &cfg.profile_operator_profile {
        doc["profile"]["operator_profile"] = value(who.as_str());
    }
    if let Some(style) = &cfg.profile_communication_style {
        doc["profile"]["communication_style"] = value(style.as_str());
    }
    // Phase 181 — the three list fields. Each replaces the
    // template's array when the operator supplied values; an empty
    // builder Vec leaves the template's default untouched.
    for (key, items) in [
        ("primary_use_cases", &cfg.profile_primary_use_cases),
        (
            "behavioral_preferences",
            &cfg.profile_behavioral_preferences,
        ),
        (
            "behavioral_constraints",
            &cfg.profile_behavioral_constraints,
        ),
    ] {
        if !items.is_empty() {
            let mut arr = toml_edit::Array::new();
            for s in items {
                arr.push(s.as_str());
            }
            doc["profile"][key] = value(arr);
        }
    }

    let mut out = format!(
        "# Generated by `aivyx init --template {template_name}`\n\
         # Customize freely; the template's structure is preserved.\n\n",
    );
    out.push_str(&doc.to_string());

    // Backlog #3 — parity with the plain `render_toml` path. The template
    // path previously authored only the keys above, so a `--template` init
    // silently had NO semantic memory (Chapter Engram), dropped the operator's
    // onboarding persona answers (Chapter W), and ignored the web-search
    // choice (Phase 46). Append each shared section the template doesn't
    // already declare — respecting a template author's explicit choice and
    // never producing a duplicate (non-array) table.
    //
    // `[embedding]` + `[memory]` are guarded together: if the template manages
    // either, we leave its memory configuration alone.
    if doc.get("embedding").is_none() && doc.get("memory").is_none() {
        out.push_str(&render_embedding_section(cfg));
    }
    if doc.get("persona_seed").is_none() {
        out.push_str(&render_persona_seed_section(cfg));
    }
    if !template_declares_web_search(&doc) {
        out.push_str(&render_web_search_section(cfg));
    }
    out
}

/// True when the template already declares an MCP server named `web-search`,
/// so the `--template` path doesn't append a duplicate. `[[mcp_server]]` is an
/// array of tables, so this scans every entry.
fn template_declares_web_search(doc: &toml_edit::DocumentMut) -> bool {
    doc.get("mcp_server")
        .and_then(|item| item.as_array_of_tables())
        .map(|servers| {
            servers.iter().any(|t| {
                t.get("name").and_then(|v| v.as_str()) == Some("web-search")
            })
        })
        .unwrap_or(false)
}

/// Entry point for the init wizard. Called from `run()` in the
/// binary when `CliMode::Init` is dispatched.
///
/// Phase 66 added the optional `template` argument. When
/// `Some(template)`, the wizard pre-fills prompt defaults from
/// the template's content (assistant_name, primary_use_case,
/// communication_style, provider, model, fs root, storage path),
/// then writes the operator-modified template content as the
/// final `aivyx.toml` so the template's role declarations + MCP
/// servers + commented sections all survive. When `None`, the
/// wizard runs the existing Phase 44 path with hardcoded defaults
/// and the minimal `render_toml` output.
/// Chapter W — optionally capture an onboarding Persona/Skills seed. Local,
/// quick, and skippable: the Persona grows from use regardless. Returns an
/// empty `PersonaSeedFields` when the operator declines (no `[persona_seed]`
/// section is then emitted).
async fn collect_persona_seed(
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
    provider: Option<&Arc<dyn LlmProvider>>,
    model: &str,
) -> Result<PersonaSeedFields, String> {
    let mut seed = PersonaSeedFields::default();

    writeln!(writer, "\n— Starting personality (optional) —")
        .map_err(|e| format!("write error: {e}"))?;
    writeln!(
        writer,
        "Your assistant learns and grows from use. You can also give it a head\n\
         start: a few traits, a note about your work, even a first skill."
    )
    .map_err(|e| format!("write error: {e}"))?;

    if !prompt_yes_no("Seed a starting personality now?", false, reader, writer)? {
        return Ok(seed);
    }

    // X.4 — LLM-assisted draft (only when a model is available + opted in).
    // The operator is always the author of record: the draft pre-fills, then
    // they review and confirm (or fall back to entering it by hand).
    let mut from_draft = false;
    let assisted = provider.is_some()
        && prompt_yes_no(
            "  Want help? Describe it and I'll draft a starting set",
            true,
            reader,
            writer,
        )?;
    if assisted {
        let desc = prompt_line(
            "  In a sentence or two, what should it be like?\n  > ",
            reader,
            writer,
        )?;
        writeln!(writer, "  Drafting…").map_err(|e| format!("write error: {e}"))?;
        match aivyx_channel::persona_seed_draft::draft_persona_seed(provider.unwrap(), model, &desc)
            .await
        {
            Some(draft) => {
                seed.character_traits = draft.character_traits;
                seed.communication_adaptations = draft.communication_adaptations;
                seed.learned_context = draft.learned_context;
                seed.skills = draft
                    .skills
                    .into_iter()
                    .map(|s| SeedSkillFields {
                        name: s.name,
                        trigger: s.trigger,
                        procedure: s.procedure,
                    })
                    .collect();
                render_seed_summary(writer, &seed)?;
                if prompt_yes_no(
                    "  Use this? (you can refine it any time in aivyx.toml)",
                    true,
                    reader,
                    writer,
                )? {
                    from_draft = true;
                } else {
                    seed = PersonaSeedFields::default();
                }
            }
            None => {
                writeln!(writer, "  (Couldn't draft — let's do it together instead.)")
                    .map_err(|e| format!("write error: {e}"))?;
            }
        }
    }

    if !from_draft {
        let traits = prompt_line(
            "  Character traits (comma-separated, e.g. pragmatic, witty): ",
            reader,
            writer,
        )?;
        seed.character_traits = split_comma_list(&traits);

        let ctx = prompt_line(
            "  Anything it should know about you / your work from day one? (optional): ",
            reader,
            writer,
        )?;
        if !ctx.is_empty() {
            seed.learned_context.push(ctx);
        }

        writeln!(
            writer,
            "  Your agent already comes with starter skills (summarize a document, \
             research a topic, draft a reply, daily briefing, capture a note)."
        )
        .map_err(|e| e.to_string())?;
        if prompt_yes_no("  Add one of your own?", false, reader, writer)? {
            let name = prompt_line(
                "    Skill name (kebab-case, e.g. rust-review): ",
                reader,
                writer,
            )?;
            if !name.is_empty() {
                let trigger = prompt_line("    When does it apply? (trigger): ", reader, writer)?;
                let procedure = prompt_line("    What should it do? (procedure): ", reader, writer)?;
                seed.skills.push(SeedSkillFields {
                    name,
                    trigger,
                    procedure,
                });
            }
        }
    }

    // Mark the genesis moment when the operator seeded anything — the persona
    // chain then records day one as its first relationship milestone.
    if seed.has_content() {
        seed.relationship_milestones
            .push("genesis: first launch".to_string());
    }

    Ok(seed)
}

/// Print a compact summary of a drafted seed so the operator sees exactly what
/// they're about to plant before confirming.
fn render_seed_summary(
    writer: &mut dyn IoWrite,
    seed: &PersonaSeedFields,
) -> Result<(), String> {
    let w = |writer: &mut dyn IoWrite, s: &str| -> Result<(), String> {
        writeln!(writer, "{s}").map_err(|e| format!("write error: {e}"))
    };
    w(writer, "")?;
    if !seed.character_traits.is_empty() {
        w(writer, &format!("    traits:      {}", seed.character_traits.join(", ")))?;
    }
    if !seed.communication_adaptations.is_empty() {
        w(writer, &format!("    voice:       {}", seed.communication_adaptations.join(", ")))?;
    }
    if let Some(ctx) = seed.learned_context.first() {
        w(writer, &format!("    context:     {ctx}"))?;
    }
    if let Some(sk) = seed.skills.first() {
        w(writer, &format!("    skill:       {} — {}", sk.name, sk.trigger))?;
    }
    Ok(())
}

/// Split a comma-separated line into trimmed, non-empty entries. Delegates to
/// [`split_list`] so it shares the paren-aware behaviour (backlog #7) — commas
/// inside `(...)` don't split.
fn split_comma_list(s: &str) -> Vec<String> {
    split_list(s)
}

pub async fn run_init_wizard(
    template: Option<&super::init_templates::Template>,
) -> Result<(), String> {
    let template_defaults = match template {
        Some(t) => TemplateDefaults::from_template(t)?,
        None => TemplateDefaults::empty(),
    };
    run_init_wizard_inner(template_defaults).await
}

async fn run_init_wizard_inner(template_defaults: TemplateDefaults) -> Result<(), String> {
    // 1. TTY check — bail if not interactive.
    if !io::stdin().is_terminal() {
        return Err("`aivyx init` requires an interactive terminal".into());
    }

    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut writer = io::stderr();

    // 2. Overwrite guard.
    let config_path = Path::new(CONFIG_FILE);
    if config_path.exists() {
        let overwrite = prompt_yes_no(
            &format!("{CONFIG_FILE} already exists. Overwrite?"),
            false,
            &mut reader,
            &mut writer,
        )?;
        if !overwrite {
            eprintln!("Aborted.");
            return Ok(());
        }
    }

    // Phase 66 — announce the template up front so the operator
    // sees what's pre-filling each prompt.
    if let Some(template_name) = &template_defaults.template_name {
        eprintln!(
            "Using template `{}`. Prompt defaults are pre-filled from the template; \
             press Enter to accept or type a value to override.",
            template_name,
        );
        eprintln!();
    }

    // 3. Detect Ollama + build provider menu.
    let base_url = DEFAULT_OLLAMA_BASE_URL;
    let has_ollama = detect_ollama(base_url).await;

    // Phase 66 — when a template declares a provider, that
    // becomes the default; otherwise fall back to the Phase 44
    // ollama-or-anthropic heuristic.
    let template_provider = template_defaults
        .provider
        .as_deref()
        .map(str::to_ascii_lowercase);
    let (provider_options, default_idx) = match template_provider.as_deref() {
        Some("ollama") => (vec!["Ollama (local)", "Anthropic", "OpenAI"], 0usize),
        Some("anthropic") => (vec!["Anthropic", "OpenAI", "Ollama (local)"], 0usize),
        Some("openai") => (vec!["OpenAI", "Anthropic", "Ollama (local)"], 0usize),
        _ => {
            if has_ollama {
                eprintln!("Ollama detected at {base_url}");
                (vec!["Ollama (local)", "Anthropic", "OpenAI"], 0usize)
            } else {
                eprintln!("Ollama not detected — defaulting to Anthropic");
                (vec!["Anthropic", "OpenAI", "Ollama (local)"], 0usize)
            }
        }
    };

    writeln!(writer, "\nSelect a provider:")
        .map_err(|e| format!("write error: {e}"))?;
    let choice = prompt_choice(
        "Provider",
        &provider_options,
        default_idx,
        &mut reader,
        &mut writer,
    )?;

    let provider = match provider_options[choice] {
        "Ollama (local)" => Provider::Ollama,
        "Anthropic" => Provider::Anthropic,
        "OpenAI" => Provider::OpenAi,
        _ => unreachable!(),
    };

    // 4. Provider-specific: model or API key.
    let (model, api_key) = match provider {
        Provider::Ollama => {
            let models = list_ollama_models(base_url).await.unwrap_or_default();
            let model = if models.is_empty() {
                writeln!(writer, "{}", ollama_empty_hint())
                    .map_err(|e| format!("write error: {e}"))?;
                // Chapter P — offer to pull the recommended model right here,
                // instead of leaving the user to a copy-paste command.
                if prompt_yes_no(
                    &format!("Download {RECOMMENDED_LOCAL_MODEL} now?"),
                    true,
                    &mut reader,
                    &mut writer,
                )? {
                    pull_ollama_model(base_url, RECOMMENDED_LOCAL_MODEL, &mut writer).await?;
                    RECOMMENDED_LOCAL_MODEL.to_string()
                } else {
                    let m = prompt_line(
                        &format!("Model name [{RECOMMENDED_LOCAL_MODEL}]: "),
                        &mut reader,
                        &mut writer,
                    )?;
                    if m.is_empty() {
                        RECOMMENDED_LOCAL_MODEL.to_string()
                    } else {
                        m
                    }
                }
            } else {
                writeln!(writer, "\nAvailable models:")
                    .map_err(|e| format!("write error: {e}"))?;
                let opts: Vec<&str> = models.iter().map(|s| s.as_str()).collect();
                // Default the cursor to the recommended model if it's already
                // pulled; otherwise the first listed model.
                let default_idx = models
                    .iter()
                    .position(|m| m == RECOMMENDED_LOCAL_MODEL)
                    .unwrap_or(0);
                let idx = prompt_choice("Model", &opts, default_idx, &mut reader, &mut writer)?;
                models[idx].clone()
            };
            (model, None)
        }
        Provider::Anthropic => {
            // Phase 104 — collect model + key, then verify against
            // GET /v1/models before falling through to the rest of
            // the wizard. The template_defaults.model is forwarded
            // as a Phase 66 prompt-default override.
            collect_and_verify_cloud(
                VerifyProvider::Anthropic,
                "Anthropic API key: ",
                DEFAULT_ANTHROPIC_MODEL,
                template_defaults.model.as_deref(),
                &mut reader,
                &mut writer,
            )
            .await?
        }
        Provider::OpenAi => {
            collect_and_verify_cloud(
                VerifyProvider::OpenAi,
                "OpenAI API key: ",
                DEFAULT_OPENAI_MODEL,
                template_defaults.model.as_deref(),
                &mut reader,
                &mut writer,
            )
            .await?
        }
    };

    // 5. Paths — show defaults, allow overrides. Phase 66:
    // template-supplied paths take precedence over the
    // hardcoded HOME-derived defaults.
    let (default_fs_fallback, default_storage_fallback) = default_paths();
    let default_fs = template_defaults
        .fs_root
        .clone()
        .unwrap_or(default_fs_fallback);
    let default_storage = template_defaults
        .storage_path
        .clone()
        .unwrap_or(default_storage_fallback);

    // 5a. Access level (Chapter N) — how far the agent reaches. The
    // choice picks the fs_root boundary + the confirm-first posture; the
    // capability/audit/trust-tier machinery is unchanged, and remote
    // channels stay tier-attenuated regardless.
    let level_idx = prompt_choice(
        "Access level — how much of your machine can the agent reach?",
        &[
            "sandbox   — a dedicated sandbox directory (safest; default)",
            "workspace — a single project directory you choose",
            "home      — your entire home directory (full personal assistant)",
            "full      — the whole filesystem, incl. system files (advanced)",
        ],
        0,
        &mut reader,
        &mut writer,
    )?;
    let home_dir = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let (access_level, fs_root) = match level_idx {
        1 => {
            let r = prompt_line(
                "Workspace directory: ",
                &mut reader,
                &mut writer,
            )?;
            let r = if r.is_empty() { default_fs } else { r };
            (AccessLevel::Workspace, r)
        }
        2 => {
            writeln!(
                writer,
                "\n  \u{26a0} 'home' grants full read/write/shell over your entire \
                 home directory ({home_dir}).",
            )
            .ok();
            (AccessLevel::Home, home_dir.clone())
        }
        3 => {
            writeln!(
                writer,
                "\n  \u{26a0} 'full' grants access to the ENTIRE filesystem, including \
                 system files. A mistaken command could damage your OS.",
            )
            .ok();
            (AccessLevel::Full, "/".to_string())
        }
        // 0 (or any out-of-range) → sandbox, keeping the customizable
        // sandbox-root prompt for back-compat.
        _ => {
            let r = prompt_line(
                &format!("Sandbox root [{default_fs}]: "),
                &mut reader,
                &mut writer,
            )?;
            (AccessLevel::Sandbox, if r.is_empty() { default_fs } else { r })
        }
    };
    // Expanded levels require an explicit confirmation, then offer the
    // confirm-first safety posture (default on).
    let confirm_destructive = if access_level.is_expanded() {
        let proceed = prompt_yes_no(
            &format!("Grant '{access_level}' access to this agent?"),
            false,
            &mut reader,
            &mut writer,
        )?;
        if !proceed {
            return Err("init cancelled — expanded access not confirmed".into());
        }
        prompt_yes_no(
            "Require confirmation before destructive actions \
             (delete / overwrite / destructive shell)?",
            true,
            &mut reader,
            &mut writer,
        )?
    } else {
        false
    };

    let storage_path = prompt_line(
        &format!("Storage path [{default_storage}]: "),
        &mut reader,
        &mut writer,
    )?;
    let storage_path = if storage_path.is_empty() {
        default_storage
    } else {
        storage_path
    };

    // 5b. Web search — bundled MCP server (Phase 46).
    let enable_web_search = prompt_yes_no(
        "Enable web search?",
        true,
        &mut reader,
        &mut writer,
    )?;

    // 5c. Profile bootstrap (Phase 57, PRODUCT.md P13). Q4(c)
    // resolution at sign-off: three short prompts seeding the
    // operator's identity layer with a usable starting point.
    // Each prompt is **opt-in** — blank input means "skip, leave
    // this field unset" when no template is supplied, or "keep
    // the template's default" when one is.
    // Phase 181 — the guided identity builder. A provider is
    // constructed for the optional LLM-assisted draft; `None`
    // falls back to the guided manual prompts (local-first — the
    // builder never requires an LLM).
    let draft_provider =
        build_wizard_provider(provider, api_key.as_deref());
    let identity = confirm_identity(
        &mut reader,
        &mut writer,
        draft_provider.as_ref(),
        &model,
        &template_defaults,
    )
    .await?;

    // Chapter W — optionally seed a starting Persona/Skills set. X.4 — the same
    // wizard provider that drafts the identity can draft the seed.
    let persona_seed =
        collect_persona_seed(&mut reader, &mut writer, draft_provider.as_ref(), &model).await?;

    // Chapter Engram — set up the embedding provider so semantic memory works
    // from first boot (pulls the local embedding model on the Ollama path).
    let embedding =
        decide_embedding(provider, api_key.as_deref(), base_url, &mut reader, &mut writer).await?;

    // 6. Render + write.
    let cfg = InitConfig {
        provider,
        model,
        api_key,
        storage_path,
        fs_root,
        access_level,
        confirm_destructive,
        enable_web_search,
        profile_assistant_name: identity.assistant_name,
        profile_operator_profile: identity.operator_profile,
        profile_communication_style: identity.communication_style,
        profile_primary_use_cases: identity.primary_use_cases,
        profile_behavioral_preferences: identity.behavioral_preferences,
        profile_behavioral_constraints: identity.behavioral_constraints,
        persona_seed,
        embedding,
    };
    // Phase 66 — when a template was supplied, splice wizard
    // answers into the template document so the role declarations,
    // MCP servers, commented sections, and structure all survive.
    // Otherwise use the existing minimal `render_toml` synthesis.
    let mut toml = match (template_defaults.template_doc, &template_defaults.template_name) {
        (Some(doc), Some(name)) => render_with_template(&cfg, name, doc),
        _ => render_toml(&cfg),
    };
    // Default starter routines (cron-scheduled). Appended to both the plain and
    // template render paths so every new agent gets them.
    toml.push_str(&render_default_schedules(&cfg));
    write_config(config_path, &toml)?;

    // 6b. Chapter P — for the local path, confirm the setup actually works
    // (Ollama reachable, model present, a non-empty test reply) before the
    // user's first real turn. Best-effort: the config is already written, so a
    // failed check is informational, not fatal.
    if cfg.provider == Provider::Ollama {
        eprintln!("\nRunning a quick health check…");
        if let Err(e) = crate::doctor::run_doctor().await {
            eprintln!("{e}");
            eprintln!("(Run `aivyx doctor` again any time to re-check.)");
        }
    }

    // 7. Success message + next steps. This is the moment that shapes the
    // operator's first five minutes — so it surfaces the Studio (the web GUI a
    // new user won't otherwise discover), `doctor`, and, for the local path, the
    // capable-hardware guide.
    eprintln!("\nWrote {CONFIG_FILE}");
    if cfg.provider != Provider::Ollama {
        eprintln!(
            "Warning: {CONFIG_FILE} contains your API key. \
             Permissions set to 0600 (owner-only)."
        );
    }
    eprintln!("You'll be prompted for a passphrase on first launch (or set AIVYX_PASSPHRASE).");
    // Be transparent about the unattended routines we just wrote — surprise
    // autonomous activity erodes trust.
    if cfg.provider == Provider::Ollama {
        eprintln!(
            "\nSet up background routines (in {CONFIG_FILE} under [[schedule]]): a daily \
             environment review, nightly reflection, a health check, and a weekly digest \
             run automatically{}. Edit or disable any of them there.",
            if cfg.enable_web_search {
                ", plus a daily web trend-scan of your interests"
            } else {
                ""
            }
        );
    } else {
        eprintln!(
            "\nWrote background routines (in {CONFIG_FILE} under [[schedule]]) — disabled by \
             default for cloud providers since each run spends tokens. Flip `enabled = true` \
             on any you want (a daily environment review, nightly reflection, health check, \
             weekly digest, trend-scan)."
        );
    }
    let web_ui_port = aivyx_channel::web_ui::DEFAULT_WEB_UI_PORT;
    eprintln!("\nNext steps:");
    eprintln!("  aivyx                      — chat with your agent in the terminal");
    eprintln!(
        "  aivyx daemon run --web-ui  — run the daemon + open the Studio at \
         http://127.0.0.1:{web_ui_port}"
    );
    // Chapter Anchor — the runs-for-days path: a real service so the agent keeps
    // running (and its scheduled routines keep firing) across logout + reboot.
    // Phase 187 — offer to install it right here instead of just printing the
    // command, so first-run can genuinely end with a running service.
    if matches!(
        crate::daemon_service::Platform::detect(),
        crate::daemon_service::Platform::Linux | crate::daemon_service::Platform::MacOs
    ) {
        let already_installed = crate::daemon_service::installed_unit_path().is_some();
        let active = if already_installed {
            crate::daemon_service::is_active()
        } else {
            None
        };
        offer_service_install(
            already_installed,
            active,
            &mut reader,
            &mut writer,
            crate::daemon_service::run_install,
        )?;
    }
    eprintln!("  aivyx doctor               — re-check your setup any time");
    if cfg.provider == Provider::Ollama {
        eprintln!(
            "\nOn a capable GPU (e.g. a 24GB card) you can run a bigger model with more \
             context — see docs/LOCAL_HOSTING.md."
        );
    }
    eprintln!("\nHappy building!");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Phase 181 — identity builder flow fixtures ----------

    /// A fake provider whose draft call returns a fixed labeled
    /// response (or errors, to exercise the manual fallback).
    struct FakeDraftProvider {
        reply: Option<String>,
    }
    #[async_trait::async_trait]
    impl LlmProvider for FakeDraftProvider {
        async fn chat_stream(
            &self,
            _request: aivyx_llm::LlmRequest<'_>,
            _cancel: &aivyx_core::CancellationToken,
        ) -> Result<Box<dyn aivyx_llm::LlmStream>, aivyx_llm::LlmError>
        {
            match &self.reply {
                Some(text) => Ok(Box::new(FakeDraftStream {
                    text: Some(text.clone()),
                })),
                None => Err(aivyx_llm::LlmError::Config(
                    "offline".into(),
                )),
            }
        }
    }
    struct FakeDraftStream {
        text: Option<String>,
    }
    #[async_trait::async_trait]
    impl aivyx_llm::LlmStream for FakeDraftStream {
        async fn next_event(
            &mut self,
        ) -> Result<
            Option<aivyx_llm::LlmStreamEvent>,
            aivyx_llm::LlmError,
        > {
            Ok(None)
        }
        async fn finish(
            self: Box<Self>,
        ) -> Result<aivyx_llm::LlmStepEnd, aivyx_llm::LlmError>
        {
            Ok(aivyx_llm::LlmStepEnd::FinalMessage {
                text: self.text.unwrap_or_default(),
                usage: aivyx_llm::LlmUsage::default(),
            })
        }
    }

    fn fake_provider(reply: Option<&str>) -> Arc<dyn LlmProvider> {
        Arc::new(FakeDraftProvider {
            reply: reply.map(String::from),
        })
    }

    #[tokio::test]
    async fn identity_builder_manual_path_collects_all_six() {
        // provider = None → straight to the guided manual prompts
        // (the local-first / offline path).
        let input = b"My Helper\na Rust dev\nwarm but concise\n\
            coding, review\nwrite tests first\n\
            never force push, never commit secrets\n"
            .to_vec();
        let mut reader = std::io::Cursor::new(input);
        let mut writer: Vec<u8> = Vec::new();
        let fields = run_identity_builder(
            &mut reader,
            &mut writer,
            None,
            "m",
            &TemplateDefaults::empty(),
        )
        .await
        .unwrap();
        assert_eq!(fields.assistant_name.as_deref(), Some("My Helper"));
        assert_eq!(fields.operator_profile.as_deref(), Some("a Rust dev"));
        assert_eq!(
            fields.communication_style.as_deref(),
            Some("warm but concise")
        );
        assert_eq!(fields.primary_use_cases, vec!["coding", "review"]);
        assert_eq!(
            fields.behavioral_preferences,
            vec!["write tests first"]
        );
        assert_eq!(
            fields.behavioral_constraints,
            vec!["never force push", "never commit secrets"]
        );
    }

    #[tokio::test]
    async fn identity_builder_draft_path_drafts_then_keeps_on_review()
    {
        let draft = "ASSISTANT_NAME: Sage\n\
            OPERATOR_PROFILE: a founder\n\
            COMMUNICATION_STYLE: warm\n\
            PRIMARY_USE_CASES: email, planning\n\
            BEHAVIORAL_PREFERENCES: be proactive\n\
            BEHAVIORAL_CONSTRAINTS: never flatter";
        let provider = fake_provider(Some(draft));
        // offer=yes (Enter), 4 conversation answers, 6 review
        // lines (all Enter → keep the drafted value).
        let input =
            b"\na calm partner\nconfidant\nwarm\nnothing\n\n\n\n\n\n\n"
                .to_vec();
        let mut reader = std::io::Cursor::new(input);
        let mut writer: Vec<u8> = Vec::new();
        let fields = run_identity_builder(
            &mut reader,
            &mut writer,
            Some(&provider),
            "m",
            &TemplateDefaults::empty(),
        )
        .await
        .unwrap();
        assert_eq!(fields.assistant_name.as_deref(), Some("Sage"));
        assert_eq!(fields.operator_profile.as_deref(), Some("a founder"));
        assert_eq!(fields.primary_use_cases, vec!["email", "planning"]);
        assert_eq!(fields.behavioral_constraints, vec!["never flatter"]);
    }

    #[tokio::test]
    async fn identity_builder_llm_error_degrades_to_manual() {
        // Provider present + operator accepts, but the draft call
        // errors → graceful fallback to the manual prompts.
        let provider = fake_provider(None); // errors on chat_stream
        // offer=yes, 4 conversation answers, then 6 MANUAL prompts.
        let input = b"\nintent\nrole\ntone\nnope\n\
            Fallback\na dev\nplain\ncoding\n\n\n"
            .to_vec();
        let mut reader = std::io::Cursor::new(input);
        let mut writer: Vec<u8> = Vec::new();
        let fields = run_identity_builder(
            &mut reader,
            &mut writer,
            Some(&provider),
            "m",
            &TemplateDefaults::empty(),
        )
        .await
        .unwrap();
        // Collected via the manual path after the draft failed.
        assert_eq!(fields.assistant_name.as_deref(), Some("Fallback"));
        assert_eq!(fields.operator_profile.as_deref(), Some("a dev"));
        assert_eq!(fields.primary_use_cases, vec!["coding"]);
    }

    #[test]
    fn identity_summary_reads_warmly_and_omits_empty() {
        let f = IdentityFields {
            assistant_name: Some("Sage".into()),
            operator_profile: Some("a Rust engineer".into()),
            communication_style: Some("warm but concise".into()),
            primary_use_cases: vec!["coding".into(), "review".into()],
            behavioral_preferences: vec![],
            behavioral_constraints: vec!["never force push".into()],
        };
        let s = render_identity_summary(&f);
        assert!(s.contains("Meet your assistant"));
        assert!(s.contains("I'm Sage."));
        assert!(s.contains("You're a Rust engineer."));
        assert!(s.contains("I'll talk warm but concise."));
        assert!(s.contains("I'm here for coding, review."));
        assert!(s.contains("I'll never never force push."));
        // Empty preferences line is omitted.
        assert!(!s.contains("I'll tend to"));
    }

    #[test]
    fn identity_summary_falls_back_to_default_name() {
        let s = render_identity_summary(&IdentityFields::default());
        assert!(s.contains("I'm Aivyx."));
    }

    #[tokio::test]
    async fn confirm_identity_accepts_on_first_preview() {
        // Manual collection (provider None): 6 field lines, then
        // choose option 1 ("Yes") at the preview.
        let input = b"Sage\na dev\nwarm\ncoding\n\n\n1\n".to_vec();
        let mut reader = std::io::Cursor::new(input);
        let mut writer: Vec<u8> = Vec::new();
        let fields = confirm_identity(
            &mut reader,
            &mut writer,
            None,
            "m",
            &TemplateDefaults::empty(),
        )
        .await
        .unwrap();
        assert_eq!(fields.assistant_name.as_deref(), Some("Sage"));
        assert!(String::from_utf8_lossy(&writer)
            .contains("Meet your assistant"));
    }

    #[tokio::test]
    async fn confirm_identity_edit_then_accept() {
        // Collect (6 lines) → preview → "Edit" (2) → re-review 6
        // lines (change the name) → preview → "Yes" (1).
        let input = b"Sage\na dev\nwarm\ncoding\n\n\n\
            2\n\
            Mira\n\n\n\n\n\n\
            1\n"
            .to_vec();
        let mut reader = std::io::Cursor::new(input);
        let mut writer: Vec<u8> = Vec::new();
        let fields = confirm_identity(
            &mut reader,
            &mut writer,
            None,
            "m",
            &TemplateDefaults::empty(),
        )
        .await
        .unwrap();
        // The edit replaced the name; the rest kept (Enter).
        assert_eq!(fields.assistant_name.as_deref(), Some("Mira"));
        assert_eq!(fields.operator_profile.as_deref(), Some("a dev"));
    }

    #[test]
    fn review_draft_keeps_blank_and_replaces_typed() {
        let draft = DraftedProfile {
            assistant_name: Some("Sage".into()),
            operator_profile: Some("a dev".into()),
            communication_style: Some("warm".into()),
            primary_use_cases: vec!["coding".into()],
            behavioral_preferences: vec![],
            behavioral_constraints: vec!["never force push".into()],
        };
        // Keep name (Enter), replace operator_profile, keep style,
        // replace use-cases, keep prefs (Enter), keep constraints.
        let input =
            b"\na founder\n\nplanning, email\n\n\n".to_vec();
        let mut reader = std::io::Cursor::new(input);
        let mut writer: Vec<u8> = Vec::new();
        let fields =
            review_draft(&mut reader, &mut writer, draft).unwrap();
        assert_eq!(fields.assistant_name.as_deref(), Some("Sage"));
        assert_eq!(fields.operator_profile.as_deref(), Some("a founder"));
        assert_eq!(fields.communication_style.as_deref(), Some("warm"));
        assert_eq!(fields.primary_use_cases, vec!["planning", "email"]);
        assert_eq!(
            fields.behavioral_constraints,
            vec!["never force push"]
        );
    }

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

    #[tokio::test]
    async fn collect_persona_seed_declined_is_empty() {
        // First prompt ("Seed a starting personality now?") → default No.
        let mut input = Cursor::new(b"\n" as &[u8]);
        let mut output = Vec::new();
        let seed = collect_persona_seed(&mut input, &mut output, None, "m").await.unwrap();
        assert!(!seed.has_content());
    }

    #[tokio::test]
    async fn collect_persona_seed_full_path_captures_traits_context_skill_and_genesis() {
        // No provider → no assisted prompt; straight to the manual flow:
        // y → seed; traits; context; y → skill; name; trigger; procedure.
        let script = "y\npragmatic, precise\noperator builds Aivyx\ny\nrust-review\nwhen reviewing Rust\ncheck unwraps\n";
        let mut input = Cursor::new(script.as_bytes());
        let mut output = Vec::new();
        let seed = collect_persona_seed(&mut input, &mut output, None, "m").await.unwrap();
        assert_eq!(seed.character_traits, vec!["pragmatic", "precise"]);
        assert_eq!(seed.learned_context, vec!["operator builds Aivyx"]);
        assert_eq!(seed.skills.len(), 1);
        assert_eq!(seed.skills[0].name, "rust-review");
        assert_eq!(seed.skills[0].trigger, "when reviewing Rust");
        // Seeding anything records the genesis milestone.
        assert_eq!(seed.relationship_milestones, vec!["genesis: first launch"]);
    }

    #[tokio::test]
    async fn collect_persona_seed_yes_but_all_blank_stays_empty() {
        // y → seed, but every field left blank, and decline the skill.
        let mut input = Cursor::new(b"y\n\n\nn\n" as &[u8]);
        let mut output = Vec::new();
        let seed = collect_persona_seed(&mut input, &mut output, None, "m").await.unwrap();
        // No content → no genesis milestone → no [persona_seed] section emitted.
        assert!(!seed.has_content());
    }

    #[tokio::test]
    async fn collect_persona_seed_assisted_draft_prefills_and_confirms() {
        // A fake provider returns a labeled seed draft; the operator opts into
        // help, describes, and accepts the draft.
        let provider: Arc<dyn LlmProvider> = Arc::new(FakeDraftProvider {
            reply: Some(
                "CHARACTER_TRAITS: pragmatic, witty\n\
                 COMMUNICATION_ADAPTATIONS: leads with code\n\
                 LEARNED_CONTEXT: builds a Rust agent platform\n\
                 SKILL_NAME: rust-review\n\
                 SKILL_TRIGGER: when reviewing Rust\n\
                 SKILL_PROCEDURE: check unwraps"
                    .to_string(),
            ),
        });
        // y → seed; y → want help; description; y → use this.
        let script = "y\ny\na witty pragmatic pair-programmer\ny\n";
        let mut input = Cursor::new(script.as_bytes());
        let mut output = Vec::new();
        let seed = collect_persona_seed(&mut input, &mut output, Some(&provider), "m")
            .await
            .unwrap();
        assert_eq!(seed.character_traits, vec!["pragmatic", "witty"]);
        assert_eq!(seed.communication_adaptations, vec!["leads with code"]);
        assert_eq!(seed.learned_context, vec!["builds a Rust agent platform"]);
        assert_eq!(seed.skills.len(), 1);
        assert_eq!(seed.skills[0].name, "rust-review");
        assert_eq!(seed.relationship_milestones, vec!["genesis: first launch"]);
    }

    #[tokio::test]
    async fn collect_persona_seed_assisted_decline_falls_back_to_manual() {
        let provider: Arc<dyn LlmProvider> = Arc::new(FakeDraftProvider {
            reply: Some("CHARACTER_TRAITS: drafted-trait".to_string()),
        });
        // y → seed; y → want help; description; n → don't use draft; then
        // manual: traits; blank context; n → no skill.
        let script = "y\ny\ndescribe\nn\nmy-trait\n\nn\n";
        let mut input = Cursor::new(script.as_bytes());
        let mut output = Vec::new();
        let seed = collect_persona_seed(&mut input, &mut output, Some(&provider), "m")
            .await
            .unwrap();
        // The declined draft is discarded; the manual entry wins.
        assert_eq!(seed.character_traits, vec!["my-trait"]);
    }

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

    #[test]
    fn decide_service_install_skips_the_offer_when_already_installed() {
        let mut input = Cursor::new(b"" as &[u8]);
        let mut output = Vec::new();
        let decision =
            decide_service_install(true, Some(true), &mut input, &mut output).unwrap();
        assert!(matches!(
            decision,
            ServiceInstallDecision::AlreadyInstalled { active: Some(true) }
        ));
        // No prompt was printed -- nothing was asked.
        assert!(output.is_empty());
    }

    #[test]
    fn decide_service_install_declined_falls_back() {
        let mut input = Cursor::new(b"n\n" as &[u8]);
        let mut output = Vec::new();
        let decision =
            decide_service_install(false, None, &mut input, &mut output).unwrap();
        assert!(matches!(decision, ServiceInstallDecision::Declined));
    }

    #[test]
    fn decide_service_install_yes_then_no_web_ui() {
        let mut input = Cursor::new(b"y\nn\n" as &[u8]);
        let mut output = Vec::new();
        let decision =
            decide_service_install(false, None, &mut input, &mut output).unwrap();
        assert!(matches!(
            decision,
            ServiceInstallDecision::Install { web_ui: false }
        ));
    }

    #[test]
    fn decide_service_install_yes_then_yes_web_ui() {
        let mut input = Cursor::new(b"y\ny\n" as &[u8]);
        let mut output = Vec::new();
        let decision =
            decide_service_install(false, None, &mut input, &mut output).unwrap();
        assert!(matches!(
            decision,
            ServiceInstallDecision::Install { web_ui: true }
        ));
    }

    #[test]
    fn decide_service_install_defaults_to_yes_on_bare_enter() {
        // Confirms the "install now?" prompt defaults true (bare Enter =
        // yes) -- the second bare Enter then hits the web-ui follow-up,
        // which defaults false.
        let mut input = Cursor::new(b"\n\n" as &[u8]);
        let mut output = Vec::new();
        let decision =
            decide_service_install(false, None, &mut input, &mut output).unwrap();
        assert!(matches!(
            decision,
            ServiceInstallDecision::Install { web_ui: false }
        ));
    }

    #[test]
    fn offer_service_install_prints_status_when_already_installed() {
        let mut input = Cursor::new(b"" as &[u8]);
        let mut output = Vec::new();
        offer_service_install(true, Some(false), &mut input, &mut output, |_, _| {
            panic!("run_install must not be called when already installed")
        })
        .unwrap();
        let out = String::from_utf8(output).unwrap();
        assert!(out.contains("already installed"));
        assert!(out.contains("not currently running"));
    }

    #[test]
    fn offer_service_install_prints_status_unknown_when_active_state_undetermined() {
        let mut input = Cursor::new(b"" as &[u8]);
        let mut output = Vec::new();
        offer_service_install(true, None, &mut input, &mut output, |_, _| {
            panic!("run_install must not be called when already installed")
        })
        .unwrap();
        let out = String::from_utf8(output).unwrap();
        assert!(out.contains("already installed"));
        assert!(out.contains("status unknown"));
    }

    #[test]
    fn offer_service_install_prints_todays_hint_when_declined() {
        let mut input = Cursor::new(b"n\n" as &[u8]);
        let mut output = Vec::new();
        offer_service_install(false, None, &mut input, &mut output, |_, _| {
            panic!("run_install must not be called when declined")
        })
        .unwrap();
        let out = String::from_utf8(output).unwrap();
        assert!(out.contains("aivyx daemon install"));
        assert!(out.contains("survives logout/reboot"));
    }

    #[test]
    fn offer_service_install_calls_run_install_with_the_right_args_on_yes() {
        let mut input = Cursor::new(b"y\ny\n" as &[u8]);
        let mut output = Vec::new();
        let mut captured: Option<(bool, bool)> = None;
        offer_service_install(false, None, &mut input, &mut output, |web_ui, start| {
            captured = Some((web_ui, start));
            Ok(())
        })
        .unwrap();
        assert_eq!(captured, Some((true, true)));
        let out = String::from_utf8(output).unwrap();
        assert!(out.contains("Installed as a background service"));
    }

    #[test]
    fn offer_service_install_prints_the_error_and_does_not_fail_on_install_failure() {
        let mut input = Cursor::new(b"y\nn\n" as &[u8]);
        let mut output = Vec::new();
        let result = offer_service_install(false, None, &mut input, &mut output, |_, _| {
            Err("no supported service manager on this platform".to_string())
        });
        // Must return Ok -- a failed install must never fail the wizard.
        assert!(result.is_ok());
        let out = String::from_utf8(output).unwrap();
        assert!(out.contains("Couldn't install"));
        assert!(out.contains("no supported service manager on this platform"));
    }

    // -- TOML generation -------------------------------------------------

    /// Builder helper for the six pre-Phase-57 tests that don't
    /// care about Profile fields. Wires `None` into the three new
    /// Profile slots so the test bodies stay focused.
    fn init_config_no_profile(
        provider: Provider,
        model: &str,
        api_key: Option<&str>,
        storage_path: &str,
        fs_root: &str,
        enable_web_search: bool,
    ) -> InitConfig {
        InitConfig {
            provider,
            model: model.into(),
            api_key: api_key.map(String::from),
            storage_path: storage_path.into(),
            fs_root: fs_root.into(),
            access_level: AccessLevel::Sandbox,
            confirm_destructive: false,
            enable_web_search,
            profile_assistant_name: None,
            profile_operator_profile: None,
            profile_communication_style: None,
            profile_primary_use_cases: Vec::new(),
            profile_behavioral_preferences: Vec::new(),
            profile_behavioral_constraints: Vec::new(),
            persona_seed: PersonaSeedFields::default(),
            embedding: None,
        }
    }

    #[test]
    fn render_toml_ollama() {
        let cfg = init_config_no_profile(
            Provider::Ollama,
            "llama3.2:latest",
            None,
            "data/aivyx.redb",
            "/home/user/workspace",
            false,
        );
        let toml = render_toml(&cfg);
        assert!(toml.contains("provider = \"ollama\""));
        assert!(toml.contains("model = \"llama3.2:latest\""));
        assert!(!toml.contains("[anthropic]"));
        assert!(!toml.contains("[openai]"));
        assert!(toml.contains("[fs]"));
        assert!(toml.contains("root = \"/home/user/workspace\""));
        assert!(toml.contains("[storage]"));
        assert!(toml.contains("path = \"data/aivyx.redb\""));
        assert!(!toml.contains("[[mcp_server]]"));
        // No [profile] section unless operator customized.
        assert!(!toml.contains("[profile]"));
        // No [persona_seed] section unless the operator seeded one.
        assert!(!toml.contains("[persona_seed]"));
    }

    // --- Chapter Engram (EN.3): embedding + memory-profile rendering --------

    /// Local Ollama embedding renders a keyless `[embedding]` pointing at the
    /// local server plus `[memory] profile = "smart"`.
    #[test]
    fn render_emits_embedding_and_smart_memory_for_local() {
        let mut cfg = init_config_no_profile(
            Provider::Ollama,
            "qwen3:8b",
            None,
            "data/aivyx.redb",
            "/home/user/workspace",
            false,
        );
        cfg.embedding = Some(EmbeddingFields {
            base_url: "http://localhost:11434".into(),
            model: RECOMMENDED_EMBED_MODEL.into(),
            dimensions: RECOMMENDED_EMBED_DIMENSIONS,
            api_key: None,
        });
        let toml = render_toml(&cfg);
        assert!(toml.contains("[embedding]"), "{toml}");
        assert!(toml.contains("model = \"nomic-embed-text\""), "{toml}");
        assert!(toml.contains("dimensions = 768"), "{toml}");
        assert!(toml.contains("base_url = \"http://localhost:11434\""), "{toml}");
        // Local needs no key.
        assert!(!toml.contains("api_key"), "local embedding renders no key: {toml}");
        assert!(toml.contains("[memory]"), "{toml}");
        assert!(toml.contains("profile = \"smart\""), "{toml}");
    }

    /// OpenAI embedding renders the cloud endpoint + the key + smart memory.
    #[test]
    fn render_emits_embedding_with_key_and_smart_memory_for_openai() {
        let mut cfg = init_config_no_profile(
            Provider::OpenAi,
            "gpt-4.1",
            Some("sk-test"),
            "data/aivyx.redb",
            "/home/user/workspace",
            false,
        );
        cfg.embedding = Some(EmbeddingFields {
            base_url: aivyx_config::DEFAULT_EMBEDDING_BASE_URL.into(),
            model: aivyx_config::DEFAULT_EMBEDDING_MODEL.into(),
            dimensions: aivyx_config::DEFAULT_EMBEDDING_DIMENSIONS,
            api_key: Some("sk-embed".into()),
        });
        let toml = render_toml(&cfg);
        assert!(toml.contains("[embedding]"), "{toml}");
        assert!(toml.contains("model = \"text-embedding-3-small\""), "{toml}");
        assert!(toml.contains("dimensions = 1536"), "{toml}");
        assert!(toml.contains("api_key = \"sk-embed\""), "{toml}");
        assert!(toml.contains("profile = \"smart\""), "{toml}");
    }

    /// Regression guard: no embedding provider ⇒ neither `[embedding]` nor
    /// `[memory]` is rendered — byte-for-byte the pre-Engram behavior, so
    /// existing-style configs are untouched.
    #[test]
    fn render_without_embedding_omits_embedding_and_memory() {
        let cfg = init_config_no_profile(
            Provider::Anthropic,
            "claude-sonnet-4-6",
            Some("sk-ant"),
            "data/aivyx.redb",
            "/home/user/workspace",
            false,
        );
        let toml = render_toml(&cfg);
        assert!(!toml.contains("[embedding]"), "{toml}");
        assert!(!toml.contains("[memory]"), "{toml}");
    }

    /// The generated local config round-trips through the loader: `[embedding]`
    /// parses and `[memory] profile = "smart"` yields `MemoryProfile::Smart`.
    #[test]
    fn generated_local_embedding_config_round_trips() {
        let mut cfg = init_config_no_profile(
            Provider::Ollama,
            "qwen3:8b",
            None,
            "data/aivyx.redb",
            "/home/user/workspace",
            false,
        );
        cfg.embedding = Some(EmbeddingFields {
            base_url: "http://localhost:11434".into(),
            model: RECOMMENDED_EMBED_MODEL.into(),
            dimensions: RECOMMENDED_EMBED_DIMENSIONS,
            api_key: None,
        });
        let toml = render_toml(&cfg);
        let loaded = aivyx_config::AivyxConfig::load_from_env_and_toml(&aivyx_config::LoadOptions {
            toml_path: Some(write_temp_toml(&toml, "engram-rt")),
            require_api_key: false,
            require_telegram_token: false,
            require_discord_token: false,
            require_slack_tokens: false,
            role_override: None,
        })
        .expect("generated toml loads");
        assert_eq!(loaded.memory_profile, aivyx_config::MemoryProfile::Smart);
        let emb = loaded.embedding.expect("[embedding] parsed");
        assert_eq!(emb.model, "nomic-embed-text");
        assert_eq!(emb.dimensions, 768);
    }

    /// `decide_embedding` for Anthropic is `None` (no embeddings API).
    #[tokio::test]
    async fn decide_embedding_anthropic_is_none() {
        let mut r = std::io::Cursor::new(&b""[..]);
        let mut w = Vec::new();
        let got = decide_embedding(Provider::Anthropic, Some("sk-ant"), "", &mut r, &mut w)
            .await
            .unwrap();
        assert!(got.is_none());
    }

    /// `decide_embedding` for OpenAI uses the key; without one it is `None`.
    #[tokio::test]
    async fn decide_embedding_openai_depends_on_key() {
        let mut r = std::io::Cursor::new(&b""[..]);
        let mut w = Vec::new();
        let with_key = decide_embedding(Provider::OpenAi, Some("sk-x"), "", &mut r, &mut w)
            .await
            .unwrap()
            .expect("openai + key → embedding");
        assert_eq!(with_key.model, aivyx_config::DEFAULT_EMBEDDING_MODEL);
        assert_eq!(with_key.api_key.as_deref(), Some("sk-x"));

        let no_key = decide_embedding(Provider::OpenAi, None, "", &mut r, &mut w)
            .await
            .unwrap();
        assert!(no_key.is_none(), "no key → no embedding");
    }

    #[test]
    fn default_schedules_gating_by_provider_and_web_search() {
        use toml_edit::DocumentMut;

        let enabled_of = |toml: &str, name: &str| -> bool {
            let doc: DocumentMut = toml.parse().expect("schedules render valid TOML");
            let arr = doc["schedule"]
                .as_array_of_tables()
                .expect("[[schedule]] array");
            let t = arr
                .iter()
                .find(|t| t.get("name").and_then(|v| v.as_str()) == Some(name))
                .unwrap_or_else(|| panic!("schedule {name} present"));
            t.get("enabled").and_then(|v| v.as_bool()).expect("enabled bool")
        };
        let count = |toml: &str| -> usize {
            let doc: DocumentMut = toml.parse().unwrap();
            doc["schedule"].as_array_of_tables().unwrap().len()
        };

        // Ollama + web search: all five written; core + trend-scan enabled.
        let cfg =
            init_config_no_profile(Provider::Ollama, "qwen3:8b", None, "s", "/r", true);
        let toml = render_default_schedules(&cfg);
        assert_eq!(count(&toml), 5);
        assert!(enabled_of(&toml, "environment-review"));
        assert!(enabled_of(&toml, "health-check"));
        assert!(enabled_of(&toml, "trend-scan"));

        // Ollama, no web search: core enabled, opt-in trend-scan disabled.
        let cfg =
            init_config_no_profile(Provider::Ollama, "qwen3:8b", None, "s", "/r", false);
        let toml = render_default_schedules(&cfg);
        assert!(enabled_of(&toml, "environment-review"));
        assert!(!enabled_of(&toml, "trend-scan"), "trend-scan needs web search");

        // Cloud: all five written, every one disabled (discoverable, cost-aware).
        let cfg = init_config_no_profile(
            Provider::Anthropic,
            "claude",
            Some("k"),
            "s",
            "/r",
            true,
        );
        let toml = render_default_schedules(&cfg);
        assert_eq!(count(&toml), 5);
        for name in [
            "environment-review",
            "nightly-reflection",
            "health-check",
            "weekly-digest",
            "trend-scan",
        ] {
            assert!(!enabled_of(&toml, name), "cloud routine {name} must be disabled");
        }
    }

    /// 2026-07-04 self-learning dogfood (#2) — a fresh config must arm the
    /// REAL reflection scheduler (the self-learning passes ride it), not just
    /// the lookalike "nightly-reflection" prompt routine.
    #[test]
    fn default_reflection_schedule_is_planted() {
        use toml_edit::DocumentMut;
        let cfg =
            init_config_no_profile(Provider::Ollama, "qwen3:8b", None, "s", "/r", true);
        let toml = render_default_schedules(&cfg);
        let doc: DocumentMut = toml.parse().unwrap();
        let rs = doc["reflection_schedule"].as_array_of_tables().unwrap();
        assert_eq!(rs.len(), 1);
        let t = rs.iter().next().unwrap();
        assert_eq!(t.get("name").and_then(|v| v.as_str()), Some("reflection"));
        assert_eq!(t.get("enabled").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            t.get("skip_when_idle").and_then(|v| v.as_bool()),
            Some(true),
            "a quiet day must cost nothing"
        );

        // Cloud: written but disabled (cost-aware), same as the routines.
        let cfg = init_config_no_profile(
            Provider::Anthropic,
            "claude",
            Some("k"),
            "s",
            "/r",
            true,
        );
        let toml = render_default_schedules(&cfg);
        let doc: DocumentMut = toml.parse().unwrap();
        let rs = doc["reflection_schedule"].as_array_of_tables().unwrap();
        assert_eq!(
            rs.iter().next().unwrap().get("enabled").and_then(|v| v.as_bool()),
            Some(false),
            "cloud reflection must be written disabled"
        );
    }

    /// Chapter Plumb (PL.2) — the reporting routines must stay GROUNDED: they
    /// have to name an explicit first read and forbid invention, or the local
    /// model satisfies them with prose and confabulates (the fresh-Jarvis
    /// weekly-digest fabricated a week of work with 0 tool calls). This guard
    /// fails if a future edit quietly reverts them to passive "summarize"
    /// prompts.
    #[test]
    fn reporting_routines_are_grounded_and_anti_fabrication() {
        // weekly-digest: read-first + never-invent + say-nothing-when-empty.
        let d = ROUTINE_WEEKLY_DIGEST.to_lowercase();
        assert!(d.contains("first read"), "digest must read before writing");
        assert!(d.contains("journal") && d.contains("memor"), "digest must ground in memory + journal");
        assert!(d.contains("never invent"), "digest must forbid invention");
        assert!(d.contains("nothing notable to report yet"), "digest must have an explicit empty-case");

        // trend-scan: must actually search; never fabricate findings/sources.
        let t = ROUTINE_TREND_SCAN.to_lowercase();
        assert!(t.contains("must actually search the web"), "trend-scan must force a real search");
        assert!(t.contains("never fabricate"), "trend-scan must forbid fabricated findings");

        // nightly-reflection: grounded too (read-first + don't invent).
        let r = ROUTINE_NIGHTLY_REFLECTION.to_lowercase();
        assert!(r.contains("first read"), "reflection must read before reflecting");
        assert!(r.contains("never invent"), "reflection must forbid invention");
    }

    /// Chapter Ledger (#6 fix) — the weekly-digest routine is rendered as a
    /// DETERMINISTIC report (`report_kind = "digest"`), so the scheduler builds
    /// it from the memory substrate instead of an LLM turn that confabulates.
    /// Other routines stay LLM prompts (no report_kind).
    #[test]
    fn weekly_digest_is_a_deterministic_report() {
        use toml_edit::DocumentMut;
        let cfg =
            init_config_no_profile(Provider::Ollama, "qwen3:8b", None, "s", "/r", true);
        let toml = render_default_schedules(&cfg);
        let doc: DocumentMut = toml.parse().unwrap();
        let scheds = doc["schedule"].as_array_of_tables().unwrap();
        let kind_of = |name: &str| -> Option<String> {
            scheds
                .iter()
                .find(|t| t.get("name").and_then(|v| v.as_str()) == Some(name))
                .and_then(|t| t.get("report_kind").and_then(|v| v.as_str()))
                .map(String::from)
        };
        assert_eq!(kind_of("weekly-digest").as_deref(), Some("digest"));
        // The LLM routines carry no report_kind.
        assert_eq!(kind_of("environment-review"), None);
        assert_eq!(kind_of("trend-scan"), None);
        assert_eq!(kind_of("health-check"), None);
    }

    /// Backlog #7 — list items with internal commas (parenthetical groups) must
    /// stay whole, not shred. Plain comma-separation still works.
    #[test]
    fn split_list_is_paren_aware() {
        // plain case unchanged
        assert_eq!(split_list("a, b, c"), vec!["a", "b", "c"]);
        // commas inside parens do NOT split
        assert_eq!(
            split_list("Research my passions (flying, food, coffee)"),
            vec!["Research my passions (flying, food, coffee)"]
        );
        // top-level commas split; parenthetical commas don't
        assert_eq!(
            split_list("Build apps, Research (a, b, c), Track tasks"),
            vec!["Build apps", "Research (a, b, c)", "Track tasks"]
        );
        // brackets/braces too
        assert_eq!(split_list("x [1, 2], y"), vec!["x [1, 2]", "y"]);
        // trimming + empties dropped; trailing comma ok
        assert_eq!(split_list(" a ,  , b, "), vec!["a", "b"]);
        // unbalanced parens degrade gracefully (no panic, content preserved)
        assert_eq!(split_list("a (b, c"), vec!["a (b, c"]);
    }

    #[test]
    fn render_toml_emits_persona_seed_section() {
        let mut cfg = init_config_no_profile(
            Provider::Ollama,
            "llama3.2:latest",
            None,
            "data/aivyx.redb",
            "/home/user/workspace",
            false,
        );
        cfg.persona_seed = PersonaSeedFields {
            learned_context: vec!["operator builds Aivyx".into()],
            communication_adaptations: vec!["leads with code".into()],
            character_traits: vec!["pragmatic".into(), "precise".into()],
            relationship_milestones: vec!["genesis: first launch".into()],
            skills: vec![SeedSkillFields {
                name: "rust-review".into(),
                trigger: "when reviewing Rust".into(),
                procedure: "check unwraps; cite file:line".into(),
            }],
        };
        let toml = render_toml(&cfg);
        assert!(toml.contains("[persona_seed]"), "{toml}");
        assert!(toml.contains("communication_adaptations = [\"leads with code\"]"), "{toml}");
        assert!(toml.contains("character_traits = [\"pragmatic\", \"precise\"]"), "{toml}");
        assert!(toml.contains("learned_context = [\"operator builds Aivyx\"]"), "{toml}");
        assert!(toml.contains("[[persona_seed.skill]]"), "{toml}");
        assert!(toml.contains("name = \"rust-review\""), "{toml}");

        // The wizard's output must parse back through the W.1 config loader.
        let cfg = aivyx_config::AivyxConfig::load_from_env_and_toml(&aivyx_config::LoadOptions {
            toml_path: Some(write_temp_toml(&toml, "init-seed")),
            require_api_key: false,
            require_telegram_token: false,
            require_discord_token: false,
            require_slack_tokens: false,
            role_override: None,
        })
        .expect("generated toml loads");
        let seed = cfg.persona_seed.expect("[persona_seed] parsed");
        assert_eq!(seed.character_traits, vec!["pragmatic", "precise"]);
        // Chapter Outfit — the loader merges the default starter skills into the
        // operator's seed (starter-on by default). The operator's declared skill
        // survives; the defaults are appended.
        assert!(
            seed.skills.iter().any(|s| s.name == "rust-review"),
            "operator skill survives the starter merge"
        );
        assert_eq!(
            seed.skills.len(),
            1 + aivyx_config::default_starter_skills().len()
        );
    }

    /// Write `toml` to a unique temp file and return its path (no tempfile dep).
    fn write_temp_toml(toml: &str, tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "aivyx-init-{}-{}-{tag}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&p, toml).expect("write temp toml");
        p
    }

    #[test]
    fn render_toml_anthropic() {
        let cfg = init_config_no_profile(
            Provider::Anthropic,
            DEFAULT_ANTHROPIC_MODEL,
            Some("sk-ant-test123"),
            "store.redb",
            ".",
            false,
        );
        let toml = render_toml(&cfg);
        assert!(toml.contains("provider = \"anthropic\""));
        assert!(toml.contains("[anthropic]"));
        assert!(toml.contains("api_key = \"sk-ant-test123\""));
        assert!(!toml.contains("[openai]"));
        // Phase 104 — pin the refreshed default model so a future
        // stale-default regression fails this test loudly.
        assert!(toml.contains("model = \"claude-sonnet-4-6\""));
    }

    #[test]
    fn render_toml_openai() {
        let cfg = init_config_no_profile(
            Provider::OpenAi,
            DEFAULT_OPENAI_MODEL,
            Some("sk-openai-xyz"),
            "store.redb",
            ".",
            false,
        );
        let toml = render_toml(&cfg);
        assert!(toml.contains("provider = \"openai\""));
        assert!(toml.contains("[openai]"));
        assert!(toml.contains("api_key = \"sk-openai-xyz\""));
        assert!(!toml.contains("[anthropic]"));
        // Phase 104 — pin the refreshed default model.
        assert!(toml.contains("model = \"gpt-4.1\""));
    }

    /// Phase 104 — pin the refreshed default model constants so a
    /// future stale-default regression fails loudly at the constant
    /// rather than at one of the `render_toml_*` integration tests.
    #[test]
    fn default_models_are_current() {
        assert_eq!(DEFAULT_ANTHROPIC_MODEL, "claude-sonnet-4-6");
        assert_eq!(DEFAULT_OPENAI_MODEL, "gpt-4.1");
    }

    /// Pin the empty-Ollama-models hint so it always names a concrete,
    /// tool-capable pull command (Chapter P: the recommended model).
    #[test]
    fn ollama_empty_hint_includes_concrete_pull_command() {
        let hint = ollama_empty_hint();
        assert!(
            hint.contains(&format!("ollama pull {RECOMMENDED_LOCAL_MODEL}")),
            "Ollama empty-list hint should name the recommended model: {hint:?}",
        );
        assert!(
            hint.contains("No local models found"),
            "hint should still name the condition: {hint:?}",
        );
    }

    #[tokio::test]
    async fn pull_ollama_model_unreachable_is_error() {
        // Chapter P — a pull against an unreachable Ollama returns a clear
        // Err (surfaced by the wizard / `doctor`), not a panic or a hang.
        let mut writer: Vec<u8> = Vec::new();
        let res = pull_ollama_model("http://127.0.0.1:1", RECOMMENDED_LOCAL_MODEL, &mut writer).await;
        assert!(res.is_err(), "unreachable base_url must error");
    }

    #[test]
    fn render_toml_custom_paths() {
        let cfg = init_config_no_profile(
            Provider::Ollama,
            "mistral:latest",
            None,
            "/custom/store.redb",
            "/custom/workspace",
            false,
        );
        let toml = render_toml(&cfg);
        assert!(toml.contains("root = \"/custom/workspace\""));
        assert!(toml.contains("path = \"/custom/store.redb\""));
    }

    // -- Chapter N: access levels in init --------------------------------

    #[test]
    fn render_toml_sandbox_omits_access_section() {
        // Back-compat: the default sandbox level renders `[fs] root` and
        // NO `[access]` section, byte-identical to pre-Chapter-N output.
        let cfg = init_config_no_profile(
            Provider::Ollama,
            "llama3.2:latest",
            None,
            "data/aivyx.redb",
            "/home/user/aivyx-sandbox",
            false,
        );
        let toml = render_toml(&cfg);
        assert!(toml.contains("[fs]"));
        assert!(toml.contains("root = \"/home/user/aivyx-sandbox\""));
        assert!(!toml.contains("[access]"));
    }

    #[test]
    fn render_toml_home_level_emits_access_not_fs() {
        let mut cfg = init_config_no_profile(
            Provider::Ollama,
            "llama3.2:latest",
            None,
            "data/aivyx.redb",
            "/home/user",
            false,
        );
        cfg.access_level = AccessLevel::Home;
        cfg.confirm_destructive = true;
        let toml = render_toml(&cfg);
        assert!(toml.contains("[access]"));
        assert!(toml.contains("level = \"home\""));
        assert!(toml.contains("confirm_destructive = true"));
        // fs_root is derived from the level — no [fs] root, no explicit root.
        assert!(!toml.contains("[fs]"));
        assert!(!toml.contains("\nroot = "));
    }

    #[test]
    fn render_toml_workspace_level_emits_explicit_root() {
        let mut cfg = init_config_no_profile(
            Provider::Ollama,
            "llama3.2:latest",
            None,
            "data/aivyx.redb",
            "/home/user/project",
            false,
        );
        cfg.access_level = AccessLevel::Workspace;
        cfg.confirm_destructive = true;
        let toml = render_toml(&cfg);
        assert!(toml.contains("level = \"workspace\""));
        assert!(toml.contains("root = \"/home/user/project\""));
        assert!(toml.contains("confirm_destructive = true"));
        assert!(!toml.contains("[fs]"), "workspace carries its root via [access]");
    }

    // -- Phase 46: web search in init ------------------------------------

    #[test]
    fn init_toml_with_web_search() {
        let cfg = init_config_no_profile(
            Provider::Ollama,
            "llama3.2:latest",
            None,
            "store.redb",
            ".",
            true,
        );
        let toml = render_toml(&cfg);
        assert!(toml.contains("[[mcp_server]]"));
        assert!(toml.contains("name = \"web-search\""));
        assert!(toml.contains("command = \"aivyx\""));
        assert!(toml.contains("args = [\"mcp-server\", \"web-search\"]"));
        assert!(toml.contains("bundled = true"));
    }

    #[test]
    fn init_toml_without_web_search() {
        let cfg = init_config_no_profile(
            Provider::Ollama,
            "llama3.2:latest",
            None,
            "store.redb",
            ".",
            false,
        );
        let toml = render_toml(&cfg);
        assert!(!toml.contains("[[mcp_server]]"));
        assert!(!toml.contains("web-search"));
    }

    // -- Phase 57: Profile bootstrap in init -----------------------------

    #[test]
    fn render_toml_omits_profile_section_when_all_three_unset() {
        // Default-everything operator pass: all three Profile
        // prompts skipped → no `[profile]` section in the
        // generated TOML. Preserves the substrate's non-invasive
        // default per Q5(b).
        let cfg = init_config_no_profile(
            Provider::Ollama,
            "llama3.2:latest",
            None,
            "store.redb",
            ".",
            false,
        );
        let toml = render_toml(&cfg);
        assert!(!toml.contains("[profile]"));
        assert!(!toml.contains("assistant_name"));
    }

    #[test]
    fn render_toml_emits_all_six_profile_fields() {
        let cfg = InitConfig {
            profile_assistant_name: Some("Mira".into()),
            profile_operator_profile: Some(
                "a senior Rust engineer who values directness".into(),
            ),
            profile_communication_style: Some("warm but concise".into()),
            profile_primary_use_cases: vec![
                "systems programming".into(),
                "personal-finance analysis".into(),
            ],
            profile_behavioral_preferences: vec![
                "prefer integration tests over mocks".into(),
            ],
            profile_behavioral_constraints: vec![
                "never autonomously commit code".into(),
                "always confirm destructive shell commands".into(),
            ],
            ..init_config_no_profile(
                Provider::Ollama,
                "llama3.2:latest",
                None,
                "store.redb",
                ".",
                false,
            )
        };
        let toml = render_toml(&cfg);
        // All six fields present.
        assert!(toml.contains("assistant_name = \"Mira\""));
        assert!(toml.contains(
            "operator_profile = \"a senior Rust engineer who values directness\""
        ));
        assert!(toml.contains("communication_style = \"warm but concise\""));
        assert!(toml.contains(
            "primary_use_cases = [\"systems programming\", \
             \"personal-finance analysis\"]"
        ));
        assert!(toml.contains(
            "behavioral_preferences = [\"prefer integration tests over mocks\"]"
        ));
        assert!(toml.contains(
            "behavioral_constraints = [\"never autonomously commit code\", \
             \"always confirm destructive shell commands\"]"
        ));
        // Round-trips as valid TOML — all six survive a parse.
        let parsed: toml_edit::DocumentMut =
            toml.parse().expect("valid TOML");
        let p = &parsed["profile"];
        assert_eq!(p["assistant_name"].as_str(), Some("Mira"));
        assert_eq!(
            p["primary_use_cases"].as_array().unwrap().len(),
            2
        );
        assert_eq!(
            p["behavioral_constraints"].as_array().unwrap().len(),
            2
        );
    }

    #[test]
    fn render_toml_is_secure_by_default() {
        // Phase 180 — every wizard-generated config requests the
        // bundled sandbox preset (secure-by-default for new
        // launches; existing configs without [sandbox] are
        // unchanged).
        let cfg = init_config_no_profile(
            Provider::Ollama,
            "llama3.2:latest",
            None,
            "store.redb",
            ".",
            false,
        );
        let toml = render_toml(&cfg);
        assert!(toml.contains("[sandbox]"));
        assert!(toml.contains("default_backend = \"auto\""));
    }

    #[test]
    fn template_render_is_secure_by_default() {
        // Template mode (toml_edit path) is also secure-by-default.
        let cfg = init_config_no_profile(
            Provider::Ollama,
            "llama3.2:latest",
            None,
            "store.redb",
            ".",
            false,
        );
        let doc: toml_edit::DocumentMut =
            "[agent]\nprovider = \"ollama\"\nmodel = \"x\"\n\
             [fs]\nroot = \".\"\n[storage]\npath = \"s.redb\"\n"
                .parse()
                .unwrap();
        let out = render_with_template(&cfg, "coder", doc);
        assert!(out.contains("[sandbox]"));
        assert!(out.contains("default_backend = \"auto\""));
    }

    // -- Backlog #3: --template parity with the plain render path ----------

    /// A minimal template document (just the keys the wizard always sets).
    fn minimal_template_doc() -> toml_edit::DocumentMut {
        "[agent]\nprovider = \"ollama\"\nmodel = \"x\"\n\
         [fs]\nroot = \".\"\n[storage]\npath = \"s.redb\"\n"
            .parse()
            .unwrap()
    }

    fn cfg_with_embedding_and_seed(enable_web_search: bool) -> InitConfig {
        InitConfig {
            embedding: Some(EmbeddingFields {
                base_url: "http://localhost:11434".into(),
                model: "nomic-embed-text".into(),
                dimensions: 768,
                api_key: None,
            }),
            persona_seed: PersonaSeedFields {
                character_traits: vec!["warm".into()],
                skills: vec![SeedSkillFields {
                    name: "summarize".into(),
                    trigger: "when asked to summarize".into(),
                    procedure: "give a tight gist + bullets".into(),
                }],
                ..PersonaSeedFields::default()
            },
            ..init_config_no_profile(
                Provider::Ollama,
                "qwen3:8b",
                None,
                "s.redb",
                ".",
                enable_web_search,
            )
        }
    }

    #[test]
    fn template_path_adds_engram_persona_and_web_search() {
        // The big one: a --template init must NOT silently skip semantic
        // memory (Engram), the persona seed (Chapter W), or web search.
        let cfg = cfg_with_embedding_and_seed(true);
        let out = render_with_template(&cfg, "coder", minimal_template_doc());

        assert!(out.contains("[embedding]"), "embedding section: {out}");
        assert!(out.contains("model = \"nomic-embed-text\""));
        assert!(out.contains("[memory]") && out.contains("profile = \"smart\""));
        assert!(out.contains("[persona_seed]"), "persona seed: {out}");
        assert!(out.contains("[[persona_seed.skill]]"));
        assert!(out.contains("name = \"web-search\""), "web search: {out}");
        // The result must still be valid TOML (no duplicate tables).
        out.parse::<toml_edit::DocumentMut>()
            .expect("template output is valid TOML");
    }

    #[test]
    fn template_path_respects_a_template_that_declares_these_sections() {
        // A (user) template that manages its own embedding / memory /
        // persona seed / web search must not get duplicated sections —
        // which would also make the TOML invalid.
        let cfg = cfg_with_embedding_and_seed(true);
        let doc: toml_edit::DocumentMut = "[agent]\nprovider = \"ollama\"\nmodel = \"x\"\n\
             [fs]\nroot = \".\"\n[storage]\npath = \"s.redb\"\n\
             [embedding]\nbase_url = \"http://x\"\nmodel = \"custom-embed\"\ndimensions = 1024\n\
             [memory]\nprofile = \"lite\"\n\
             [persona_seed]\ncharacter_traits = [\"curated\"]\n\
             [[mcp_server]]\nname = \"web-search\"\ncommand = \"aivyx\"\nargs = [\"mcp-server\", \"web-search\"]\n"
            .parse()
            .unwrap();
        let out = render_with_template(&cfg, "custom", doc);

        // Template's choices win; nothing duplicated.
        assert_eq!(out.matches("[embedding]").count(), 1, "{out}");
        assert_eq!(out.matches("[memory]").count(), 1);
        assert_eq!(out.matches("[persona_seed]").count(), 1);
        assert_eq!(out.matches("name = \"web-search\"").count(), 1);
        assert!(out.contains("model = \"custom-embed\""), "template embed kept");
        assert!(out.contains("profile = \"lite\""), "template profile kept");
        out.parse::<toml_edit::DocumentMut>()
            .expect("template output is valid TOML");
    }

    #[test]
    fn template_path_skips_engram_when_no_embedding_resolved() {
        // No embedding provider resolved → no [embedding]/[memory] (matches
        // render_toml: the profile would be inert anyway).
        let cfg = init_config_no_profile(
            Provider::Ollama,
            "qwen3:8b",
            None,
            "s.redb",
            ".",
            false,
        );
        let out = render_with_template(&cfg, "coder", minimal_template_doc());
        assert!(!out.contains("[embedding]"));
        assert!(!out.contains("[persona_seed]"));
        assert!(!out.contains("name = \"web-search\""));
    }

    #[test]
    fn render_toml_emits_profile_section_when_assistant_name_set() {
        // Operator customized only the assistant name. The
        // `[profile]` section appears with just that field;
        // primary_use_cases and communication_style are absent.
        let cfg = InitConfig {
            profile_assistant_name: Some("Codex".into()),
            ..init_config_no_profile(
                Provider::Ollama,
                "llama3.2:latest",
                None,
                "store.redb",
                ".",
                false,
            )
        };
        let toml = render_toml(&cfg);
        assert!(toml.contains("[profile]"));
        assert!(toml.contains("assistant_name = \"Codex\""));
        assert!(!toml.contains("communication_style"));
        assert!(!toml.contains("primary_use_cases"));
    }

    #[test]
    fn render_toml_emits_all_three_profile_fields_when_set() {
        let cfg = InitConfig {
            profile_assistant_name: Some("Mira".into()),
            profile_communication_style: Some("terse, conclusion-first".into()),
            profile_primary_use_cases: vec!["personal-finance analysis".into()],
            ..init_config_no_profile(
                Provider::Anthropic,
                DEFAULT_ANTHROPIC_MODEL,
                Some("sk-ant-x"),
                "store.redb",
                ".",
                false,
            )
        };
        let toml = render_toml(&cfg);
        assert!(toml.contains("[profile]"));
        assert!(toml.contains("assistant_name = \"Mira\""));
        assert!(toml.contains("communication_style = \"terse, conclusion-first\""));
        assert!(toml.contains("primary_use_cases = [\"personal-finance analysis\"]"));
    }

    #[test]
    fn render_toml_escapes_special_chars_in_profile_fields() {
        // Operator might paste text containing quotes or
        // backslashes. The renderer must escape them so the
        // generated TOML still parses cleanly.
        let cfg = InitConfig {
            profile_assistant_name: Some("Quote\"y".into()),
            profile_communication_style: Some(
                "with \"emphasis\" sometimes".into(),
            ),
            profile_primary_use_cases: vec!["Path C:\\\\Users\\code".into()],
            ..init_config_no_profile(
                Provider::Ollama,
                "llama3.2:latest",
                None,
                "store.redb",
                ".",
                false,
            )
        };
        let toml = render_toml(&cfg);
        assert!(toml.contains("assistant_name = \"Quote\\\"y\""));
        // The "\\\\" in the source pastes literally as `\\` (two
        // chars) in the operator's input. After escape_toml_string
        // each `\` becomes `\\`, so the final TOML quad-backslash
        // round-trips to two backslashes when parsed.
        assert!(toml.contains("primary_use_cases = [\"Path C:\\\\\\\\Users\\\\code\"]"));
        assert!(toml.contains("communication_style = \"with \\\"emphasis\\\" sometimes\""));
    }
}

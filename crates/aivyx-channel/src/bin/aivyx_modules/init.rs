//! `aivyx init` — interactive first-run setup wizard (Phase 44).
//!
//! Detects whether Ollama is running locally and defaults to it,
//! walks the user through provider and model selection, and writes
//! a ready-to-use `aivyx.toml` config file.

use std::io::{self, BufRead, IsTerminal, Write as IoWrite};
use std::path::Path;
use std::time::Duration;

use aivyx_llm::openai::DEFAULT_OLLAMA_BASE_URL;
use aivyx_llm::verify::{verify_provider_credentials, VerifyError, VerifyProvider};

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

/// Phase 104 — printed when `list_ollama_models` returns an
/// empty list. Replaces the Phase 44 hint `"Run \`ollama pull
/// <model>\` first."` (which named no concrete model) with a
/// single copy-pasteable command per Q4(a). `llama3.2:3b` is
/// small enough to download in seconds yet capable enough to
/// drive a real conversation — the right tier for a fresh-
/// laptop first turn.
const OLLAMA_EMPTY_HINT: &str =
    "No local models found.\nTry: ollama pull llama3.2:3b";

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

/// Captures all wizard answers needed to render `aivyx.toml`.
struct InitConfig {
    provider: Provider,
    model: String,
    api_key: Option<String>,
    storage_path: String,
    fs_root: String,
    /// Phase 46: enable bundled web search MCP server.
    enable_web_search: bool,
    /// Phase 57: Profile bootstrap per Q4(c). Each field is
    /// `None` when the operator left the corresponding wizard
    /// prompt blank; the renderer only emits a `[profile]`
    /// section when at least one is `Some`.
    profile_assistant_name: Option<String>,
    profile_primary_use_case: Option<String>,
    profile_communication_style: Option<String>,
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

    // [fs] and [storage] sections
    out.push_str(&format!(
        "\n[fs]\nroot = \"{}\"\n\n[storage]\npath = \"{}\"\n",
        cfg.fs_root, cfg.storage_path,
    ));

    // Phase 46: bundled web search MCP server.
    if cfg.enable_web_search {
        out.push_str(
            "\n[[mcp_server]]\n\
             name = \"web-search\"\n\
             command = \"aivyx\"\n\
             args = [\"mcp-server\", \"web-search\"]\n\
             bundled = true\n",
        );
    }

    // Phase 57: Profile section per Q4(c). Only emit when the
    // operator customized at least one field. A blank-everywhere
    // pass leaves the substrate at its synthesized default — same
    // behavior as pre-Phase-57 configs.
    if cfg.profile_assistant_name.is_some()
        || cfg.profile_primary_use_case.is_some()
        || cfg.profile_communication_style.is_some()
    {
        out.push_str("\n[profile]\n");
        if let Some(name) = &cfg.profile_assistant_name {
            out.push_str(&format!("assistant_name = \"{}\"\n", escape_toml_string(name)));
        }
        if let Some(style) = &cfg.profile_communication_style {
            out.push_str(&format!(
                "communication_style = \"{}\"\n",
                escape_toml_string(style),
            ));
        }
        if let Some(use_case) = &cfg.profile_primary_use_case {
            out.push_str(&format!(
                "primary_use_cases = [\"{}\"]\n",
                escape_toml_string(use_case),
            ));
        }
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

    // [profile] fields. Only update keys the operator actually
    // customized; leave the template's defaults otherwise. The
    // template's assistant_name etc. is already a string the
    // operator may have wanted to keep.
    if let Some(name) = &cfg.profile_assistant_name {
        doc["profile"]["assistant_name"] = value(name.as_str());
    }
    if let Some(uc) = &cfg.profile_primary_use_case {
        // Replace primary_use_cases with a single-element array
        // matching the operator's input. Templates may have
        // multi-element arrays; the wizard's single prompt is
        // intentionally a single use case.
        let mut arr = toml_edit::Array::new();
        arr.push(uc.as_str());
        doc["profile"]["primary_use_cases"] = value(arr);
    }
    if let Some(style) = &cfg.profile_communication_style {
        doc["profile"]["communication_style"] = value(style.as_str());
    }

    let mut out = format!(
        "# Generated by `aivyx init --template {template_name}`\n\
         # Customize freely; the template's structure is preserved.\n\n",
    );
    out.push_str(&doc.to_string());
    out
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
                eprintln!("{OLLAMA_EMPTY_HINT}");
                let m = prompt_line("Model name: ", &mut reader, &mut writer)?;
                if m.is_empty() {
                    return Err("model name cannot be empty".into());
                }
                m
            } else {
                writeln!(writer, "\nAvailable models:")
                    .map_err(|e| format!("write error: {e}"))?;
                let opts: Vec<&str> = models.iter().map(|s| s.as_str()).collect();
                let idx = prompt_choice("Model", &opts, 0, &mut reader, &mut writer)?;
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

    let fs_root = prompt_line(
        &format!("Sandbox root [{default_fs}]: "),
        &mut reader,
        &mut writer,
    )?;
    let fs_root = if fs_root.is_empty() { default_fs } else { fs_root };

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
    writeln!(writer, "\nAssistant identity (optional — press Enter to skip):")
        .map_err(|e| format!("write error: {e}"))?;

    let assistant_name_prompt = match &template_defaults.assistant_name {
        Some(name) => format!("Assistant name [{name}]: "),
        None => "Assistant name (default Aivyx): ".to_string(),
    };
    let assistant_name_raw = prompt_line(
        &assistant_name_prompt,
        &mut reader,
        &mut writer,
    )?;
    let profile_assistant_name = if assistant_name_raw.is_empty() {
        template_defaults.assistant_name.clone()
    } else {
        Some(assistant_name_raw)
    };

    let primary_use_case_prompt = match &template_defaults.primary_use_case {
        Some(uc) => format!("Primary use case [{uc}]: "),
        None => "Primary use case (e.g. 'Rust systems programming'): ".to_string(),
    };
    let primary_use_case_raw = prompt_line(
        &primary_use_case_prompt,
        &mut reader,
        &mut writer,
    )?;
    let profile_primary_use_case = if primary_use_case_raw.is_empty() {
        template_defaults.primary_use_case.clone()
    } else {
        Some(primary_use_case_raw)
    };

    let style_prompt = match &template_defaults.communication_style {
        Some(s) => format!("Communication style [{s}]: "),
        None => "Communication style (e.g. 'terse, conclusion-first'): "
            .to_string(),
    };
    let style_raw = prompt_line(
        &style_prompt,
        &mut reader,
        &mut writer,
    )?;
    let profile_communication_style = if style_raw.is_empty() {
        template_defaults.communication_style.clone()
    } else {
        Some(style_raw)
    };

    // 6. Render + write.
    let cfg = InitConfig {
        provider,
        model,
        api_key,
        storage_path,
        fs_root,
        enable_web_search,
        profile_assistant_name,
        profile_primary_use_case,
        profile_communication_style,
    };
    // Phase 66 — when a template was supplied, splice wizard
    // answers into the template document so the role declarations,
    // MCP servers, commented sections, and structure all survive.
    // Otherwise use the existing minimal `render_toml` synthesis.
    let toml = match (template_defaults.template_doc, &template_defaults.template_name) {
        (Some(doc), Some(name)) => render_with_template(&cfg, name, doc),
        _ => render_toml(&cfg),
    };
    write_config(config_path, &toml)?;

    // 7. Success message.
    eprintln!("\nWrote {CONFIG_FILE}");
    if cfg.provider != Provider::Ollama {
        eprintln!(
            "Warning: {CONFIG_FILE} contains your API key. \
             Permissions set to 0600 (owner-only)."
        );
    }
    eprintln!("You will be prompted for a passphrase on first launch.");
    eprintln!("Alternatively, set the AIVYX_PASSPHRASE environment variable.");
    eprintln!("\nRun `aivyx` to start the agent. Happy building!");
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
            enable_web_search,
            profile_assistant_name: None,
            profile_primary_use_case: None,
            profile_communication_style: None,
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

    /// Phase 104 — pin the empty-Ollama-models hint string so the
    /// `ollama pull llama3.2:3b` suggestion can't silently
    /// regress to a hint-with-no-model-name future.
    #[test]
    fn ollama_empty_hint_includes_concrete_pull_command() {
        assert!(
            OLLAMA_EMPTY_HINT.contains("ollama pull llama3.2:3b"),
            "Ollama empty-list hint should name a concrete model: {OLLAMA_EMPTY_HINT:?}",
        );
        assert!(
            OLLAMA_EMPTY_HINT.contains("No local models found"),
            "hint should still name the condition: {OLLAMA_EMPTY_HINT:?}",
        );
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
    fn render_toml_emits_profile_section_when_assistant_name_set() {
        // Operator customized only the assistant name. The
        // `[profile]` section appears with just that field;
        // primary_use_cases and communication_style are absent.
        let cfg = InitConfig {
            profile_assistant_name: Some("Codex".into()),
            profile_primary_use_case: None,
            profile_communication_style: None,
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
            profile_primary_use_case: Some("personal-finance analysis".into()),
            profile_communication_style: Some("terse, conclusion-first".into()),
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
            profile_primary_use_case: Some("Path C:\\\\Users\\code".into()),
            profile_communication_style: Some(
                "with \"emphasis\" sometimes".into(),
            ),
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

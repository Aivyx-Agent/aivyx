//! `aivyx init` — interactive first-run setup wizard (Phase 44).
//!
//! Detects whether Ollama is running locally and defaults to it,
//! walks the user through provider and model selection, and writes
//! a ready-to-use `aivyx.toml` config file.

use std::io::{self, BufRead, IsTerminal, Write as IoWrite};
use std::path::Path;
use std::time::Duration;

use aivyx_llm::openai::DEFAULT_OLLAMA_BASE_URL;

/// Default config file name (matches `aivyx-config` convention).
const CONFIG_FILE: &str = "aivyx.toml";

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

/// Entry point for the init wizard. Called from `run()` in the
/// binary when `CliMode::Init` is dispatched.
pub async fn run_init_wizard() -> Result<(), String> {
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

    // 3. Detect Ollama + build provider menu.
    let base_url = DEFAULT_OLLAMA_BASE_URL;
    let has_ollama = detect_ollama(base_url).await;

    let (provider_options, default_idx) = if has_ollama {
        eprintln!("Ollama detected at {base_url}");
        (vec!["Ollama (local)", "Anthropic", "OpenAI"], 0usize)
    } else {
        eprintln!("Ollama not detected — defaulting to Anthropic");
        (vec!["Anthropic", "OpenAI", "Ollama (local)"], 0usize)
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
                eprintln!("No local models found. Run `ollama pull <model>` first.");
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
            let model = prompt_line(
                "Model [claude-sonnet-4-20250514]: ",
                &mut reader,
                &mut writer,
            )?;
            let model = if model.is_empty() {
                "claude-sonnet-4-20250514".into()
            } else {
                model
            };
            let key = prompt_secret("Anthropic API key: ")?;
            if key.is_empty() {
                return Err("API key cannot be empty".into());
            }
            (model, Some(key))
        }
        Provider::OpenAi => {
            let model = prompt_line("Model [gpt-4o]: ", &mut reader, &mut writer)?;
            let model = if model.is_empty() {
                "gpt-4o".into()
            } else {
                model
            };
            let key = prompt_secret("OpenAI API key: ")?;
            if key.is_empty() {
                return Err("API key cannot be empty".into());
            }
            (model, Some(key))
        }
    };

    // 5. Paths — show defaults, allow overrides.
    let (default_fs, default_storage) = default_paths();

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

    // 6. Render + write.
    let cfg = InitConfig {
        provider,
        model,
        api_key,
        storage_path,
        fs_root,
        enable_web_search,
    };
    let toml = render_toml(&cfg);
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

    #[test]
    fn render_toml_ollama() {
        let cfg = InitConfig {
            provider: Provider::Ollama,
            model: "llama3.2:latest".into(),
            api_key: None,
            storage_path: "data/aivyx.redb".into(),
            fs_root: "/home/user/workspace".into(),
            enable_web_search: false,
        };
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
    }

    #[test]
    fn render_toml_anthropic() {
        let cfg = InitConfig {
            provider: Provider::Anthropic,
            model: "claude-sonnet-4-20250514".into(),
            api_key: Some("sk-ant-test123".into()),
            storage_path: "store.redb".into(),
            fs_root: ".".into(),
            enable_web_search: false,
        };
        let toml = render_toml(&cfg);
        assert!(toml.contains("provider = \"anthropic\""));
        assert!(toml.contains("[anthropic]"));
        assert!(toml.contains("api_key = \"sk-ant-test123\""));
        assert!(!toml.contains("[openai]"));
    }

    #[test]
    fn render_toml_openai() {
        let cfg = InitConfig {
            provider: Provider::OpenAi,
            model: "gpt-4o".into(),
            api_key: Some("sk-openai-xyz".into()),
            storage_path: "store.redb".into(),
            fs_root: ".".into(),
            enable_web_search: false,
        };
        let toml = render_toml(&cfg);
        assert!(toml.contains("provider = \"openai\""));
        assert!(toml.contains("[openai]"));
        assert!(toml.contains("api_key = \"sk-openai-xyz\""));
        assert!(!toml.contains("[anthropic]"));
    }

    #[test]
    fn render_toml_custom_paths() {
        let cfg = InitConfig {
            provider: Provider::Ollama,
            model: "mistral:latest".into(),
            api_key: None,
            storage_path: "/custom/store.redb".into(),
            fs_root: "/custom/workspace".into(),
            enable_web_search: false,
        };
        let toml = render_toml(&cfg);
        assert!(toml.contains("root = \"/custom/workspace\""));
        assert!(toml.contains("path = \"/custom/store.redb\""));
    }

    // -- Phase 46: web search in init ------------------------------------

    #[test]
    fn init_toml_with_web_search() {
        let cfg = InitConfig {
            provider: Provider::Ollama,
            model: "llama3.2:latest".into(),
            api_key: None,
            storage_path: "store.redb".into(),
            fs_root: ".".into(),
            enable_web_search: true,
        };
        let toml = render_toml(&cfg);
        assert!(toml.contains("[[mcp_server]]"));
        assert!(toml.contains("name = \"web-search\""));
        assert!(toml.contains("command = \"aivyx\""));
        assert!(toml.contains("args = [\"mcp-server\", \"web-search\"]"));
        assert!(toml.contains("bundled = true"));
    }

    #[test]
    fn init_toml_without_web_search() {
        let cfg = InitConfig {
            provider: Provider::Ollama,
            model: "llama3.2:latest".into(),
            api_key: None,
            storage_path: "store.redb".into(),
            fs_root: ".".into(),
            enable_web_search: false,
        };
        let toml = render_toml(&cfg);
        assert!(!toml.contains("[[mcp_server]]"));
        assert!(!toml.contains("web-search"));
    }
}

//! `aivyx` — the reference CLI binary for Phase 3.
//!
//! Wires every component from Phases 0–2 into a single interactive
//! loop:
//!
//! ```text
//! stdin
//!   → readline
//!   → Message::text
//!   → ConcreteAgent::turn
//!       → LlmPlanner
//!           → AnthropicProvider (live HTTPS)
//!               → tokens streamed back through LocalChannel
//!       → PersistentAuditLog (HMAC chain persisted to KeyDomain::Audit)
//!   → TurnOutcome
//!   → [turn completed] marker printed by LocalChannel::finalize
//! stdin (next prompt)
//! ```
//!
//! ## Configuration
//!
//! The binary reads one required environment variable:
//!
//! - `ANTHROPIC_API_KEY` — the API key for the Anthropic Messages
//!   endpoint. Wrapped in `SecretString` as soon as it's read so it
//!   can't accidentally end up in a `Debug` log line. A future phase
//!   will add a config-file path; for Phase 3, env var is the one
//!   and only secret source.
//!
//! Five optional variables:
//!
//! - `AIVYX_MODEL` — override the default model id (default:
//!   `claude-haiku-4-5-20251001`). Sent verbatim to the API.
//! - `AIVYX_SYSTEM_PROMPT` — override the default system prompt.
//! - `AIVYX_FS_ROOT` — directory under which the filesystem tools
//!   (`fs.read`, `fs.write`) are allowed to operate. Defaults to
//!   `$HOME/aivyx-sandbox`. Created at startup if it does not exist.
//!   The binary's capability set grants `fs.read:<root>/**` and
//!   `fs.write:<root>/**` so the LLM can exercise both tools without
//!   further wiring.
//! - `AIVYX_STORAGE_PATH` — path to the encrypted redb store (Phase 5
//!   task 4). Defaults to `$XDG_DATA_HOME/aivyx/store.redb` or
//!   `$HOME/.local/share/aivyx/store.redb` otherwise. The sidecar
//!   salt file is the same path with a `.salt` suffix appended.
//!   Parent directories are created at startup if missing.
//! - `AIVYX_PASSPHRASE` — the passphrase the Argon2id master-key
//!   derivation feeds on. If unset or empty and stdin is a
//!   terminal, the binary falls back to an interactive
//!   `aivyx passphrase: ` prompt via `rpassword` (reads
//!   `/dev/tty` directly, echo-off). If unset **and** stdin is
//!   not a terminal (systemd/launchd/scripted runs), the binary
//!   exits with a clear error rather than hanging on a tty read
//!   that will never come.
//! - `AIVYX_MEMORY_MAX_PER_TOPIC` — override the per-topic GC
//!   tripwire for `memory.write` (Phase 7 task 5). Defaults to
//!   [`aivyx_memory::DEFAULT_MAX_PER_TOPIC`] (10_000). Parsed as
//!   `usize` once at startup; an unparseable value is a hard error
//!   rather than a silent fallback, because a mis-set cap hides
//!   runaway-write bugs.
//!
//! ## Cancellation
//!
//! Ctrl-C follows the Unix-REPL convention: **the first ctrl-C during
//! a turn cancels the in-flight completion**, the loop returns to
//! the prompt, and a **second ctrl-C outside of a turn** (or a second
//! ctrl-C during the same turn if the first didn't take effect
//! quickly) **exits the process**. A background signal task owns
//! this state machine via a shared `CancellationToken` wired into the
//! `LocalChannel`.
//!
//! ## Verify-only mode
//!
//! Passing `--verify-only` as the first argument switches the binary
//! into a forensic-verification mode: it opens the encrypted store,
//! runs `PersistentAuditLog::verify_from_disk` over the whole
//! `KeyDomain::Audit` range, prints a one-line `VerifyReport`, and
//! exits with status 0 (chain OK) or non-zero (chain broken, corrupt
//! entry, or storage error). No session is started, no provider is
//! built, and `ANTHROPIC_API_KEY` is **not** required — an operator
//! running verification on a production store should not have to hand
//! the cloud key to a read-only forensic tool.
//!
//! ## What this binary is not
//!
//! - It does not persist conversation history. The planner's
//!   `LlmHistory` lives in RAM for the lifetime of one
//!   `ConcreteAgent`. Cross-turn *recall* now works through the
//!   three `memory.*` tools — the agent chooses to call
//!   `memory.write` to remember something and `memory.read` to pull
//!   it back in a later turn. This is D1's "memory is a tool, not
//!   ambient context" contract in live code: there is no hidden
//!   injection at turn start.
//! - It does not do line editing or history. Plain `stdin().read_line`.
//!   Upgrade to `rustyline` is a local refactor the day the ergonomics
//!   gap becomes painful.

use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

// Phase 9 Task 3 — `SecretString` no longer lives on the binary's
// surface: all secrets are owned by `aivyx_config::SourcedSecret`
// now, and the one production site that needed `.expose_secret()`
// (the Telegram token handoff) still imports the trait inline at
// the call site.

use aivyx_audit::PersistentAuditLog;
use aivyx_capability::{CapabilitySet, Scope};
use aivyx_channel::passphrase::{derive_master_key, PassphraseSource, DEFAULT_ENV_VAR};
use aivyx_channel::{run_session, LocalChannel, SessionConfig};
use aivyx_config::{AivyxConfig, FieldSource, LoadOptions, ToolAllowlist};
use aivyx_core::{
    AuditHook, CancellationToken, FsReadToolConfig, FsWriteToolConfig,
    ShellExecToolConfig, Tool, ToolRegistry, WebFetchToolConfig,
};
use aivyx_crypto::Argon2Params;
use aivyx_memory::{
    Memory, MemoryForgetTool, MemoryReadTool, MemoryWriteTool, RedbMemory,
};
use aivyx_llm::anthropic::{AnthropicConfig, AnthropicProvider};
use aivyx_llm::LlmProvider;
use aivyx_storage::{RedbStorage, Storage, StorageConfig};
use aivyx_telegram::{run_telegram_multi_session, TelegramSessionConfig};

const DEFAULT_MAX_TOKENS: u32 = 1024;
const PROMPT: &str = "> ";

/// Default path the binary looks at for the TOML config file.
/// `./aivyx.toml` relative to the current working directory — present
/// if the operator has written one, silently ignored if not. Absolute
/// or elsewhere paths belong in `$AIVYX_CONFIG_PATH` (future amendment)
/// or just be driven via env vars.
const DEFAULT_TOML_PATH: &str = "aivyx.toml";

/// Optional `(shell.exec tool, required capability scope)` pair
/// returned by `build_shell_exec_for_channel`. Aliased to satisfy
/// clippy's `type_complexity` lint and because the pair has a
/// specific meaning — "the shell.exec the agent gets for this
/// channel, plus the canonical cwd-root scope that lets it run".
type ShellExecRegistration = Option<(Arc<dyn Tool>, Scope)>;

/// Phase 11 Task 3 — registration-time trust-tier gate for
/// `shell.exec`.
///
/// This is the single site where the binary decides "does this
/// channel get the shell-execution tool?". `Local` (Trusted) yes,
/// `Telegram` (SemiTrusted) no. The gate is registration-time and
/// stricter than the turn loop's ceiling intersection: a
/// SemiTrusted audit chain never sees `shell.exec` mentioned, not
/// even as a denial, because the tool is simply absent from the
/// dispatch registry.
///
/// Returns `Ok(Some((tool, scope)))` if the channel receives
/// `shell.exec`, `Ok(None)` otherwise. The scope is the
/// capability the agent must hold to call the tool — it uses the
/// Phase 11 Task 3 `shell.exec:cwd:<canonical_root>/**` shape.
/// The factoring lives in a tiny free function (not inlined) so
/// `channel_kind_telegram_has_no_shell_exec` below can pin the
/// property with a real filesystem fixture without pulling the
/// rest of `run()`'s startup machinery.
fn build_shell_exec_for_channel(
    channel_kind: ChannelKind,
    fs_root: &std::path::Path,
) -> Result<ShellExecRegistration, String> {
    match channel_kind {
        ChannelKind::Local => {
            let shell = ShellExecToolConfig::new(fs_root.to_path_buf())
                .build()
                .map_err(|e| format!("failed to build shell.exec tool: {e}"))?;
            let canonical_cwd_root = shell.cwd_root().to_path_buf();
            let scope = Scope::parse(&format!(
                "shell.exec:cwd:{}/**",
                canonical_cwd_root.display()
            ))
            .ok_or_else(|| {
                format!(
                    "canonical shell.exec cwd scope not parseable from {canonical_cwd_root:?}"
                )
            })?;
            Ok(Some((Arc::new(shell) as Arc<dyn Tool>, scope)))
        }
        ChannelKind::Telegram => Ok(None),
    }
}

/// Phase 12 Task 2 — registration-time gate for `web.fetch`.
///
/// Unlike `shell.exec`, `web.fetch` is registered for **both**
/// `Trusted` (Local) and `SemiTrusted` (Telegram) channels —
/// network reads are inside the SemiTrusted default ceiling
/// (see `CEILING_SEMITRUSTED` in `aivyx-capability`) and a
/// Telegram-attached `researcher` agent should be able to
/// fetch URLs. `Untrusted` and `Kernel` channels do not exist
/// in the binary's CLI surface today, so the function only
/// needs to discriminate the two `ChannelKind`s it knows
/// about — both get the tool.
///
/// Returns `Ok(Some(tool))`. A helper rather than inline
/// code for two reasons: (1) symmetry with
/// `build_shell_exec_for_channel` so future readers find the
/// tier-gate decisions together, and (2) so the
/// `channel_{local,telegram}_receives_web_fetch` tests below
/// can pin the property without re-constructing the whole
/// `run()` startup chain.
///
/// The function does not return a scope because Phase 12 ships
/// `web.fetch` with a **narrow** capability grant configured
/// per-role, not with a broad operator-held scope. The
/// registration helper's job is "construct the tool"; the
/// grants are decided by `aivyx-config` based on role.
fn build_web_fetch_for_channel(
    _channel_kind: ChannelKind,
) -> Result<Arc<dyn Tool>, String> {
    let tool = WebFetchToolConfig::new()
        .build()
        .map_err(|e| format!("failed to build web.fetch tool: {e}"))?;
    Ok(Arc::new(tool) as Arc<dyn Tool>)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aivyx: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    // ---- CLI args -----------------------------------------------------
    // Phase 8 Task 4 extended the arg parser to accept
    // `--channel local|telegram` in addition to `--verify-only`.
    // Unknown flags still fail fast with a clear error.
    let CliArgs {
        verify_only,
        channel: channel_kind,
        role: role_override,
    } = parse_cli_args()?;

    // ---- Config -------------------------------------------------------
    // Phase 9 Task 3 — the whole "read ten env vars by hand" block that
    // Phases 3 through 8 accreted is now a single call into
    // `aivyx_config::AivyxConfig::load_from_env_and_toml`. Env vars
    // keep their Phase 8 names (operators re-exporting them need no
    // change), and a new optional `./aivyx.toml` file lives between
    // env and the encrypted store in the fall-through chain.
    //
    // `require_*` flags are derived from the CLI-arg decisions so the
    // validator's error messages land at the right point: verify-only
    // does not need `ANTHROPIC_API_KEY`, so the loader does not demand
    // it; `--channel telegram` does need `AIVYX_TELEGRAM_TOKEN`, so a
    // missing token surfaces as a clean `ConfigError::Missing` instead
    // of a Frankenstein "invalid request" on the first HTTP call.
    let load_opts = LoadOptions {
        toml_path: Some(PathBuf::from(DEFAULT_TOML_PATH)),
        require_api_key: !verify_only,
        require_telegram_token: matches!(channel_kind, ChannelKind::Telegram),
        // Phase 11 Task 4 — `--role <name>` is now the highest-
        // priority source. `parse_cli_args` turns the flag into
        // `role_override`, which `aivyx-config`'s resolver honors
        // above `AIVYX_ROLE` / TOML / `"default"`. A `None` here
        // means "no flag was passed — fall through to env/TOML."
        role_override,
    };
    let mut config = AivyxConfig::load_from_env_and_toml(&load_opts)?;

    // Sandbox root: create the directory if it does not exist so a
    // fresh install "just works" the same way Phase 4 promised. The
    // config layer returns a `PathBuf` with source provenance; we do
    // not mkdir inside the config layer because "create side effects
    // on load" is exactly the ambient-behavior trap the
    // AGENTS.md-equivalent hygiene rules in this repo try to avoid.
    //
    // Verify-only mode skips this — no session, no tools, no sandbox.
    if !verify_only {
        let root = &config.fs_root.value;
        std::fs::create_dir_all(root)
            .map_err(|e| format!("failed to create fs sandbox root {root:?}: {e}"))?;
    }

    // Resolve the encrypted store path + its sidecar salt file. The
    // config layer handled path *resolution* (env → toml → XDG → HOME
    // default); we only need to mkdir the parent directory and build
    // the sidecar path here.
    let storage_path = config.storage_path.value.clone();
    if let Some(parent) = storage_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!("failed to create storage parent directory {parent:?}: {e}")
        })?;
    }
    let salt_path = salt_path_for(&storage_path);

    // ---- Master key ---------------------------------------------------
    // Argon2id over a passphrase, using the sidecar salt file
    // (generated on first run, persisted plaintext — salts are not
    // secret per Argon2id design). Raw passphrase bytes never leave
    // `derive_master_key`; we get back a zero-on-drop `MasterKey`.
    //
    // Source selection policy — binary decides, module just honors
    // the enum:
    //
    // 1. `aivyx-config` already supplied a passphrase (env or TOML)
    //    → `Env`. This is the systemd / launchd / scripted path and
    //    must stay the first-checked branch so deployments don't
    //    accidentally trip the interactive prompt. Phase 9 Task 3
    //    routes this through `config.passphrase` instead of re-
    //    reading `AIVYX_PASSPHRASE` — the config layer has already
    //    checked env and TOML in precedence order and the result
    //    is the single source of truth at this point in bring-up.
    // 2. Config has `None` and stdin is a terminal →
    //    `InteractivePrompt`. The user gets a one-line
    //    `aivyx passphrase: ` echo-off prompt read from `/dev/tty`.
    // 3. Otherwise → bail with a clear message. Neither config nor
    //    tty means we have no interactive user *and* no configured
    //    source — continuing would either hang on a tty read that
    //    never comes, or crash with an opaque Argon2 error.
    let passphrase_source = select_passphrase_source(config.passphrase.is_some())?;
    let master_key = derive_master_key(
        passphrase_source,
        &salt_path,
        Argon2Params::d7_default(),
    )
    .map_err(|e| format!("failed to derive master key: {e}"))?;

    // Derive the audit chain key *before* `RedbStorage::open` consumes
    // `master_key`. `PersistentAuditLog` is documented as the single
    // legitimate caller of `SubKey::as_bytes`; the raw `[u8; 32]` then
    // lives on the stack until it's handed to the persistent audit
    // log, where it's cloned into an HMAC key and zeroed on drop by
    // the log itself.
    let audit_chain_key: [u8; 32] = {
        let subkey = master_key
            .derive_subkey(b"audit")
            .map_err(|e| format!("failed to derive audit chain key: {e}"))?;
        let mut out = [0u8; 32];
        out.copy_from_slice(subkey.as_bytes());
        out
    };

    // ---- Runtime ------------------------------------------------------
    // A multi-threaded runtime is overkill for a single-user REPL, but
    // the workspace tokio feature set already enables it and the cost
    // of one extra worker thread on an interactive loop is invisible.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to build tokio runtime: {e}"))?;

    runtime.block_on(async move {
        // Open the store *inside* the runtime so `RedbStorage::open`'s
        // `spawn_blocking` call lands on a live tokio pool. Opening
        // outside the runtime would panic the moment `open` tried to
        // reach for the current handle.
        let storage = RedbStorage::open(StorageConfig::new(storage_path.clone()), master_key)
            .await
            .map_err(|e| format!("failed to open encrypted store at {storage_path:?}: {e}"))?;

        if verify_only {
            return run_verify_only(storage, audit_chain_key).await;
        }

        // Phase 9 Task 3 — Phase 2 of the two-phase config load.
        // Now that the encrypted store is open, fill any still-`None`
        // secret-bearing fields (api_key, telegram.token) from
        // `KeyDomain::Secrets`. Fields already populated by env or
        // TOML are left alone per fall-through precedence.
        //
        // `validate` runs **after** hydrate: the encrypted store is
        // the last source in the fall-through, so a secret can still
        // be populated between `load_from_env_and_toml` and
        // `validate`. Moving the validation call earlier would mean
        // a store-sourced api key never gets a chance.
        config
            .hydrate_secrets_from_store(&storage)
            .await
            .map_err(String::from)?;
        config.validate(&load_opts).map_err(String::from)?;

        // Print the config-provenance banner to stderr before any
        // session traffic lands. Operators debugging a surprising
        // value ("why is my model wrong?") can read this once and
        // see which source each field came from.
        print_config_banner(&config);

        run_async(
            config,
            storage,
            audit_chain_key,
            channel_kind,
        )
        .await
    })
}

/// Print a one-block summary of every [`AivyxConfig`] field plus its
/// [`FieldSource`] tag. Secrets are redacted; paths and scalars are
/// shown verbatim because that's the useful debugging signal.
///
/// Prints to stderr, not stdout, so the banner does not interleave
/// with the local REPL's first turn output or a Telegram channel's
/// outbound messages.
fn print_config_banner(config: &AivyxConfig) {
    eprintln!("aivyx config sources:");
    eprintln!(
        "  anthropic_api_key = {}",
        match &config.anthropic_api_key {
            Some(s) => format!("<redacted> ({})", source_label(s.source)),
            None => "<unset>".to_string(),
        }
    );
    eprintln!(
        "  model             = {:?} ({})",
        config.model.value,
        source_label(config.model.source),
    );
    eprintln!(
        "  system_prompt     = {:?} ({})",
        truncate_for_log(&config.system_prompt.value, 60),
        source_label(config.system_prompt.source),
    );
    eprintln!(
        "  fs_root           = {:?} ({})",
        config.fs_root.value,
        source_label(config.fs_root.source),
    );
    eprintln!(
        "  storage_path      = {:?} ({})",
        config.storage_path.value,
        source_label(config.storage_path.source),
    );
    eprintln!(
        "  memory_max_per_topic = {} ({})",
        config.memory_max_per_topic.value,
        source_label(config.memory_max_per_topic.source),
    );
    eprintln!(
        "  passphrase        = {}",
        match &config.passphrase {
            Some(s) => format!("<redacted> ({})", source_label(s.source)),
            None => "<interactive or absent>".to_string(),
        }
    );
    if let Some(tg) = &config.telegram {
        eprintln!(
            "  telegram.token    = {}",
            match &tg.token {
                Some(s) => format!("<redacted> ({})", source_label(s.source)),
                None => "<unset>".to_string(),
            }
        );
        eprintln!(
            "  telegram.chat_id  = {}",
            match &tg.chat_filter {
                Some(c) => format!("{} ({})", c.value, source_label(c.source)),
                None => "<any>".to_string(),
            }
        );
    }
    // Phase 11 Task 1 — render the active role and any load-time
    // warnings the config layer accumulated. Role rendering stays
    // deliberately minimal at Task 1: just the active-role name and
    // its source. Task 4 (which actually wires roles through the
    // turn loop) will decide whether the banner should also show the
    // resolved system_prompt and tool_allowlist for the active role.
    eprintln!(
        "  active_role       = {:?} ({})",
        config.active_role.value,
        source_label(config.active_role.source),
    );
    if !config.warnings.is_empty() {
        eprintln!();
        eprintln!("config warnings:");
        for warning in &config.warnings {
            eprintln!("  - {warning}");
        }
    }
}

fn source_label(src: FieldSource) -> &'static str {
    match src {
        FieldSource::Env => "env",
        FieldSource::Toml => "toml",
        FieldSource::EncryptedStore => "encrypted-store",
        FieldSource::Default => "default",
    }
}

fn truncate_for_log(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

// Phase 9 Task 3 — the three `resolve_*` helpers that Phases 4, 5,
// and 7 accreted (`resolve_fs_root`, `resolve_storage_path`,
// `resolve_memory_max_per_topic`) moved into
// `aivyx_config::AivyxConfig::load_from_env_and_toml`. Default
// resolution logic (env → toml → XDG → HOME fallback, with NoHome as
// a typed error and unparseable values as typed `Invalid`) lives in
// the config crate now, which is the one place it needs to live.

/// Build the sidecar salt file path for a given store path.
///
/// Convention: append a literal `.salt` to the store path's OS string.
/// This gives `store.redb` → `store.redb.salt`, which is what the
/// passphrase module's `load_or_create_salt` reads from and, on first
/// run, writes to. We deliberately do *not* use `Path::with_extension`
/// here: `with_extension("salt")` would turn `store.redb` into
/// `store.salt`, losing the "this belongs to the redb store" signal.
fn salt_path_for(store_path: &std::path::Path) -> PathBuf {
    let mut os = store_path.as_os_str().to_owned();
    os.push(".salt");
    PathBuf::from(os)
}

/// Which channel the session runs on.
///
/// Phase 8 Task 4 added `Telegram`; earlier phases only knew `Local`.
/// Defaulting to `Local` preserves backwards compatibility with every
/// previous invocation of the `aivyx` binary — `aivyx` with no flags
/// still opens the local CLI REPL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChannelKind {
    Local,
    Telegram,
}

/// Parsed CLI arg bundle. The shape is intentionally closed — each
/// new argument lands here, so the parser's failure mode is
/// "unrecognized argument" rather than "silently ignored flag."
#[derive(Debug)]
struct CliArgs {
    verify_only: bool,
    channel: ChannelKind,
    /// Phase 11 Task 4 — `--role <name>`. Highest-priority source for
    /// `LoadOptions::role_override`; beats `AIVYX_ROLE` env var, the
    /// `aivyx.active_role` TOML field, and the implicit `"default"`
    /// fallback. `None` means "no override — fall back to the env var
    /// and config-layer resolution chain."
    role: Option<String>,
}

/// Parse the CLI arg surface.
///
/// Recognized forms:
///
/// - `aivyx` — local REPL, fresh session (default).
/// - `aivyx --verify-only` — forensic verification path; skips
///   session bring-up and exits after replaying the audit chain.
/// - `aivyx --channel local` — explicit form of the default.
/// - `aivyx --channel telegram` — Phase 8 Task 4 Telegram bot mode.
///   Requires `AIVYX_TELEGRAM_TOKEN` and `AIVYX_TELEGRAM_CHAT_ID`
///   env vars at `run_async` time.
/// - `aivyx --role <name>` — Phase 11 Task 4. Highest-priority
///   source for the active-role selection. Overrides `AIVYX_ROLE`
///   env var, the `aivyx.active_role` TOML field, and the implicit
///   `"default"` fallback. An unknown name fails at
///   `AivyxConfig::validate` with a typed error listing the roles
///   the config knows about.
///
/// `--verify-only` and `--channel` are mutually exclusive: verify
/// mode is a read-only forensic surface and has nothing to do with
/// which channel the live session would run on. Combining them is
/// an operator error we flag explicitly rather than picking a
/// silent winner.
///
/// Rejecting unknown args early keeps typos like `--verify_only` or
/// `--chanel telegram` from silently falling through into normal
/// session bring-up.
fn parse_cli_args() -> Result<CliArgs, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    parse_cli_args_from(&args)
}

/// Testable core of [`parse_cli_args`]. Split out so tests can drive
/// it with a synthetic argv without touching the real process
/// arguments. `parse_cli_args` itself is a one-line shim that hands
/// `std::env::args().skip(1)` to this function.
fn parse_cli_args_from(args: &[String]) -> Result<CliArgs, String> {
    let mut verify_only = false;
    let mut channel = ChannelKind::Local;
    let mut role: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--verify-only" => {
                verify_only = true;
                i += 1;
            }
            "--channel" => {
                let value = args.get(i + 1).ok_or_else(|| {
                    "`--channel` requires a value: `local` or `telegram`".to_string()
                })?;
                channel = match value.as_str() {
                    "local" => ChannelKind::Local,
                    "telegram" => ChannelKind::Telegram,
                    other => {
                        return Err(format!(
                            "unrecognized channel `{other}`. Supported: local, telegram"
                        ));
                    }
                };
                i += 2;
            }
            "--role" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "`--role` requires a value".to_string())?;
                if value.is_empty() {
                    return Err("`--role` requires a non-empty name".to_string());
                }
                role = Some(value.clone());
                i += 2;
            }
            other => {
                return Err(format!(
                    "unrecognized argument: `{other}`. \
                     Supported flags: --verify-only, --channel <local|telegram>, --role <name>"
                ));
            }
        }
    }

    if verify_only && channel != ChannelKind::Local {
        return Err(
            "`--verify-only` is a forensic mode and cannot be combined with `--channel`. \
             Run verify without `--channel`, or run a live session without `--verify-only`."
                .to_string(),
        );
    }

    Ok(CliArgs {
        verify_only,
        channel,
        role,
    })
}

/// Decide which `PassphraseSource` to hand to `derive_master_key`.
///
/// Phase 9 Task 3 change: the "did a source supply a passphrase"
/// check moved out of this helper into `aivyx-config`. This helper
/// now receives a boolean `config_has_passphrase` — `true` means the
/// config layer already resolved env / TOML and found a value, so we
/// can use `PassphraseSource::Env` (the `passphrase` module still
/// re-reads `AIVYX_PASSPHRASE` when honored, which is fine: if
/// `aivyx-config` saw it, the env var is still set). `false` means
/// config found nothing, so we decide between the interactive prompt
/// (tty) and a hard error (non-tty).
///
/// Policy:
/// 1. `config_has_passphrase == true` → `Env`.
/// 2. `config_has_passphrase == false` and stdin is a tty →
///    `InteractivePrompt`.
/// 3. Otherwise → `Err` with a clear operator-facing message.
fn select_passphrase_source(
    config_has_passphrase: bool,
) -> Result<PassphraseSource, String> {
    if config_has_passphrase {
        return Ok(PassphraseSource::Env {
            var_name: DEFAULT_ENV_VAR.to_string(),
        });
    }
    if io::stdin().is_terminal() {
        Ok(PassphraseSource::InteractivePrompt)
    } else {
        Err(format!(
            "no passphrase available: `{DEFAULT_ENV_VAR}` is not set and \
             stdin is not a terminal. Export the env var or run aivyx \
             from an interactive shell."
        ))
    }
}

/// Verify-only path: open the persistent audit log cold, scan
/// `KeyDomain::Audit`, replay the HMAC chain, print a one-line
/// report, and return. No session is started.
///
/// On chain break this returns `Err(..)`, which `main` maps to
/// `ExitCode::FAILURE` via the outer `match`. Callers grep the exit
/// status, not the string — but the string is still a human-readable
/// summary so an operator running the command interactively gets a
/// useful answer.
async fn run_verify_only(
    storage: Arc<dyn Storage>,
    audit_chain_key: [u8; 32],
) -> Result<(), String> {
    let report = PersistentAuditLog::verify_from_disk(storage, audit_chain_key)
        .await
        .map_err(|e| format!("audit chain verification failed: {e}"))?;

    let head_seq_display: String = report
        .head_seq
        .map(|s| s.to_string())
        .unwrap_or_else(|| "none (empty chain)".to_string());
    println!(
        "audit: verified {} events (head_seq={})",
        report.entries_verified, head_seq_display,
    );
    Ok(())
}

// `run_async` sits right at the binary's composition root: it takes
// the validated `AivyxConfig`, the open storage handle, the audit
// chain key, and the CLI-derived `ChannelKind`, and threads
// everything into the chosen `run_*_session` function.
//
// Phase 9 Task 3 changed the signature from a flat list of ten
// fields to `config: AivyxConfig` + three siblings. The earlier
// comment that argued against struct bundling was written when no
// consolidated type existed yet — the Task 3 argument is that
// `AivyxConfig` **is** the consolidated shape, so passing it whole
// lets every downstream consumer pull its exact field without the
// binary playing field-forwarder.
async fn run_async(
    config: AivyxConfig,
    storage: Arc<dyn Storage>,
    audit_chain_key: [u8; 32],
    channel_kind: ChannelKind,
) -> Result<(), String> {
    // Destructure the config at the top so each downstream block
    // reaches for the local binding rather than the nested path
    // `config.field.value`. The `SourcedSecret` fields are already
    // validated to be `Some` by the time `run_async` is called, so
    // `.expect` here encodes the Phase-9 invariant: `run_async`
    // only runs past `validate()`, and `validate()` checks
    // `require_api_key` unconditionally when `channel_kind != Local`
    // is irrelevant — api key is required whenever this function is
    // called, because `--verify-only` takes a separate branch in
    // `run()`.
    let AivyxConfig {
        anthropic_api_key,
        model,
        system_prompt: _legacy_system_prompt,
        fs_root,
        storage_path: _,
        memory_max_per_topic,
        passphrase: _,
        telegram,
        // Phase 11 Task 4 — the binary now resolves the active role
        // here and sources its `system_prompt`, `tool_allowlist`, and
        // `memory_topic_prefix` from the entry in `roles` keyed by
        // `active_role`. The legacy top-level `system_prompt` field
        // is still loaded by `aivyx-config`, but it's bridged into
        // the synthesized `default` role at config-load time, so
        // pre-Phase-11 configs that set only the top-level field
        // continue to work: the `default` role carries the same
        // prompt and the resolution below picks it up normally.
        roles,
        active_role,
        // `warnings` is rendered by the banner in `print_startup_banner`
        // directly from `&config.warnings` before the destructure; by
        // the time we land here the banner has already printed any
        // load-time warnings, so we drop the field on the floor.
        warnings: _,
    } = config;
    let api_key = anthropic_api_key
        .expect("anthropic_api_key validated non-None before run_async")
        .value;
    let model = model.value;
    let fs_root = fs_root.value;
    let memory_cap = memory_max_per_topic.value;

    // ---- Role resolution ---------------------------------------------
    // `aivyx-config::validate` has already guaranteed that
    // `active_role` keys into `roles` — either the operator's
    // explicit choice resolves, or the implicit `"default"` role is
    // present by the Task 1 backwards-compat synthesis. `.expect`
    // encodes this invariant: if it fires, validation is buggy.
    let active_role_name = active_role.value.clone();
    let role = roles
        .get(&active_role_name)
        .expect("active_role must key into roles after validate()")
        .clone();
    let system_prompt = role.system_prompt.value;
    let tool_allowlist: Option<std::collections::BTreeSet<String>> =
        match role.tool_allowlist.value {
            ToolAllowlist::AllowAll => None,
            ToolAllowlist::Only(list) => Some(list.into_iter().collect()),
        };
    let memory_topic_prefix: Option<String> = role.memory_topic_prefix.value;

    // ---- Provider -----------------------------------------------------
    let anthropic = AnthropicProvider::new(AnthropicConfig::new(api_key))
        .map_err(|e| format!("failed to build Anthropic provider: {e}"))?;
    let provider: Arc<dyn LlmProvider> = Arc::new(anthropic);

    // ---- Audit --------------------------------------------------------
    // Persistent HMAC-chained audit log over `KeyDomain::Audit`.
    // `PersistentAuditLog::open` verifies the on-disk chain as part
    // of the open sequence: if any prior session tampered with the
    // store, this call returns `ChainBroken` and the binary exits
    // with failure before any new events land. The verified event
    // count is read back via `len()` for the startup banner — no
    // second scan is performed.
    let persistent_audit = PersistentAuditLog::open(Arc::clone(&storage), audit_chain_key)
        .await
        .map_err(|e| format!("failed to open persistent audit log: {e}"))?;
    let verified_event_count = persistent_audit.len();
    let audit: Arc<dyn AuditHook> = Arc::new(persistent_audit);

    // ---- Tools --------------------------------------------------------
    // Build the Phase 4 filesystem tools. `FsReadToolConfig::build()`
    // canonicalizes the sandbox root once, so the pre-canonicalized
    // form is what flows into the scope check later — that's the
    // anchor for the `fs.read:<canonical>/**` capability below.
    let fs_read = FsReadToolConfig::new(fs_root.clone())
        .build()
        .map_err(|e| format!("failed to build fs.read tool: {e}"))?;
    let fs_write = FsWriteToolConfig::new(fs_root.clone())
        .build()
        .map_err(|e| format!("failed to build fs.write tool: {e}"))?;

    // Pull the canonicalized sandbox root back out of `fs_read` so the
    // capability scopes reference the exact same string the tools use
    // at execute time. Using the un-canonicalized `fs_root` here would
    // let a symlink in the user's `$HOME` silently widen the scope.
    let canonical_root = fs_read.sandbox_root().to_path_buf();
    let root_display = canonical_root.display();
    let fs_read_scope = Scope::parse(&format!("fs.read:{root_display}/**")).ok_or_else(|| {
        format!("canonical fs.read sandbox scope not parseable from {canonical_root:?}")
    })?;
    let fs_write_scope = Scope::parse(&format!("fs.write:{root_display}/**")).ok_or_else(|| {
        format!("canonical fs.write sandbox scope not parseable from {canonical_root:?}")
    })?;

    // Build the Phase 6 memory tools. `RedbMemory::open` clones an
    // `Arc<dyn Storage>` handle so the binary's already-open store
    // and the memory substrate share the same redb database and the
    // same master key — no second passphrase, no second sidecar. The
    // open call seeds the monotonic sequence counter from any
    // pre-existing entries, so a restart against a populated store
    // keeps the invariant that no two entries ever share a `seq`
    // (see PHASE_6.md task 2 for the crash-recovery rationale).
    let memory: Arc<dyn Memory> = RedbMemory::open(Arc::clone(&storage))
        .await
        .map_err(|e| format!("failed to open memory substrate: {e}"))?;
    let memory_read = MemoryReadTool::new(Arc::clone(&memory));
    // Phase 7 task 5 — per-topic GC tripwire. Phase 9 Task 3 moved
    // resolution into `aivyx-config`; the cap arrives pre-parsed
    // from env / TOML / default with typed `Invalid` errors if a
    // source supplied a non-usize value. Destructured above as
    // `memory_cap` from `config.memory_max_per_topic.value`.
    let memory_write =
        MemoryWriteTool::new(Arc::clone(&memory)).set_max_per_topic(memory_cap);
    let memory_forget = MemoryForgetTool::new(Arc::clone(&memory));

    // ---- Tool list (with the Phase 11 Task 3 trust-tier gate) --------
    // This is the single registration site where the binary decides
    // "what tools do I expose for this channel?" Phase 11 Task 3
    // specifies `shell.exec` is registered **only** for `Trusted`
    // channels — `aivyx-telegram` (`SemiTrusted`) must never even
    // see the tool in its dispatch registry. The gate is a single
    // match on `channel_kind` right here; scattered per-call
    // `if tier == Trusted` checks are explicitly out of scope.
    //
    // Note: this is belt-and-suspenders with the turn loop's
    // ceiling intersection (`default_ceiling()` strips `shell.exec`
    // from any SemiTrusted capability set at dispatch time), but
    // registration-time gating is stricter: the audit chain for a
    // SemiTrusted channel never sees `shell.exec` mentioned, not
    // even as a denial. That strictness is the point.
    let mut tool_list: Vec<Arc<dyn Tool>> = vec![
        Arc::new(fs_read) as Arc<dyn Tool>,
        Arc::new(fs_write) as Arc<dyn Tool>,
        Arc::new(memory_read) as Arc<dyn Tool>,
        Arc::new(memory_write) as Arc<dyn Tool>,
        Arc::new(memory_forget) as Arc<dyn Tool>,
    ];
    let shell_exec_scope: Option<Scope> =
        match build_shell_exec_for_channel(channel_kind, &fs_root)? {
            Some((shell, scope)) => {
                tool_list.push(shell);
                Some(scope)
            }
            None => None,
        };
    // Phase 12 Task 2 — `web.fetch` is registered for both
    // channel kinds (Trusted and SemiTrusted). Unlike
    // `shell.exec`, no operator-scoped capability is appended
    // at this site: role-scoped capability grants from
    // `aivyx-config` decide which URLs a given role may fetch,
    // and the broad `net.fetch` held by the Local CLI
    // (granted below) covers the Trusted-tier catch-all.
    tool_list.push(build_web_fetch_for_channel(channel_kind)?);
    let tools: Arc<ToolRegistry> = Arc::new(ToolRegistry::new(tool_list));

    // ---- Capabilities -------------------------------------------------
    // The CLI is the most-trusted channel on the box; the agent gets
    // a broad capability set so chat-only turns don't get denied for
    // scopes they never actually request. The `fs.*` scopes are
    // rooted at the canonicalized sandbox path so the scope-derivation
    // path in `FsReadTool::required_scope` lines up exactly with a
    // held capability. The three `memory.*` scopes are granted
    // **unqualified**: by D4 Rule 2 an unqualified held scope grants
    // any qualified needed scope with the same base, so the per-topic
    // scopes each `Memory*Tool::required_scope` returns
    // (`memory.read:topic:<topic>`, etc.) are all covered. Per-topic
    // attenuation becomes interesting the moment Phase 7+ introduces
    // a non-Trusted channel for the memory tools; Trusted CLI gets
    // the full family by default per D4's tier table.
    let mut scopes: Vec<Scope> = vec![
        Scope::parse("memory.read").unwrap(),
        Scope::parse("memory.write").unwrap(),
        Scope::parse("memory.forget").unwrap(),
        fs_read_scope,
        fs_write_scope,
        // Phase 12 Task 2 — unqualified `net.fetch` for the
        // broad operator-held capability set. D4 Rule 2:
        // unqualified held grants any qualified needed with
        // the same base, so the per-URL
        // `net.fetch:<url>` that `WebFetchTool::required_scope`
        // emits is always granted for the Local CLI. The
        // SemiTrusted Telegram path gets the same unqualified
        // scope via this one insert, and the turn loop's
        // ceiling intersection narrows it on the Telegram
        // side using `CEILING_SEMITRUSTED` — which is also
        // `net.fetch` unqualified, so both tiers get the same
        // broad grant here and per-role attenuation happens
        // via `tool_allowlist` + per-role capability files.
        Scope::parse("net.fetch").unwrap(),
    ];
    // Only the Local (Trusted) branch ever holds this scope — the
    // tool itself is absent from the registry on SemiTrusted
    // channels, so the scope is useless there anyway and we elide
    // it to keep the audited capability footprint tight.
    if let Some(s) = shell_exec_scope {
        scopes.push(s);
    }
    let capabilities = CapabilitySet::from_scopes(scopes);

    // ---- Channel branch ----------------------------------------------
    // Phase 8 Task 4 — fork here on `channel_kind`. Everything upstream
    // of this point is shared: same provider, same audit, same memory
    // substrate, same fs sandbox, same capability set. The branches
    // diverge only on (a) which `ChannelContext` drives the turn loop,
    // (b) which REPL-style function runs, and (c) the shape of the
    // ctrl-C signal handler.
    //
    // Trust-tier narrowing is **not** duplicated here: the broad
    // `capabilities` set above is passed to both branches, and the
    // Phase 4 turn loop intersects it with the channel's
    // `trust_tier().default_ceiling()` on every turn. LocalChannel
    // reports `Trusted` and the intersection is a no-op; TelegramChannel
    // reports `SemiTrusted` and `shell.exec` would be stripped. The
    // Task 3 pin test at `aivyx-telegram/src/tests.rs` verifies that
    // attenuation through a real `TelegramChannel`.
    match channel_kind {
        ChannelKind::Local => {
            let channel = LocalChannel::new("aivyx-cli", io::stdout());
            let token_slot = channel.token_slot();

            // Signal task (local): first ctrl-C during a turn cancels
            // the turn; a second ctrl-C exits the process. We re-read
            // the current token from the slot on every ctrl-C so that
            // turn-N+1 sees a fresh token after turn-N's
            // reset_cancellation() call (inside `run_session`).
            tokio::spawn(async move {
                loop {
                    if tokio::signal::ctrl_c().await.is_err() {
                        std::process::exit(130);
                    }
                    let current = token_slot.lock().expect("token slot poisoned").clone();
                    if current.is_cancelled() {
                        eprintln!("\naivyx: interrupted, exiting.");
                        std::process::exit(130);
                    }
                    eprintln!("\naivyx: cancelling in-flight turn (ctrl-C again to exit).");
                    current.cancel();
                }
            });

            let session_config = SessionConfig {
                model,
                system_prompt,
                max_tokens: DEFAULT_MAX_TOKENS,
                capabilities,
                tools,
                storage,
                prompt: PROMPT.to_string(),
                banner: Some(format!(
                    "aivyx {} — type a message, ctrl-C to cancel, ctrl-D to exit.\n\
                     fs sandbox: {}\n\
                     memory: live (recall persists across restarts)\n\
                     audit: persistent ({} events verified from disk)\n\
                     active role: {}",
                    env!("CARGO_PKG_VERSION"),
                    canonical_root.display(),
                    verified_event_count,
                    active_role_name,
                )),
                tool_allowlist,
                memory_topic_prefix,
            };

            let stdin = io::stdin();
            let reader = stdin.lock();
            run_session(provider, audit, session_config, channel, reader)
                .await
                .map(|_report| ())
        }

        ChannelKind::Telegram => {
            // Unwrap chain is safe: `run()` set `require_telegram_token
            // = true` for this channel, so `validate()` already
            // rejected a `None` token, and the `telegram` field is
            // `Some` because the loader constructs it whenever any
            // telegram source fires.
            let tg = telegram
                .expect("telegram config validated for ChannelKind::Telegram");
            let token_secret = tg
                .token
                .expect("telegram.token validated non-None before run_async")
                .value;
            let chat_filter: Option<i64> = tg.chat_filter.map(|c| c.value);

            // Signal handler (telegram): a single `shutdown`
            // CancellationToken the signal task cancels on first
            // ctrl-C. The session loop checks this at the top of each
            // iteration and exits cleanly — we don't need the "cancel
            // one turn, exit on second ctrl-C" staging that the local
            // path uses, because a Telegram session is expected to be
            // long-running and the only legitimate interrupt is
            // "bring the bot down."
            let shutdown = CancellationToken::new();
            let shutdown_for_signal = shutdown.clone();
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_err() {
                    std::process::exit(130);
                }
                eprintln!("\naivyx: shutting down telegram bot after current poll completes.");
                shutdown_for_signal.cancel();
            });

            // Startup message: print to stderr (not the Telegram
            // chat) so an operator running the bot in a terminal sees
            // confirmation it's alive. The Telegram chat itself gets
            // nothing at startup — the first user message is the
            // implicit "session started" affordance.
            let chat_scope_label: String = match chat_filter {
                Some(chat_id) => format!("chat_id: {chat_id} (single-chat mode)"),
                None => "chat_id: <any> (multi-chat mode)".to_string(),
            };
            eprintln!(
                "aivyx {} — telegram bot live\n\
                 {}\n\
                 fs sandbox: {}\n\
                 memory: live (recall persists across restarts)\n\
                 audit: persistent ({} events verified from disk)",
                env!("CARGO_PKG_VERSION"),
                chat_scope_label,
                canonical_root.display(),
                verified_event_count,
            );

            // `SecretString` exposes the inner string via
            // `secrecy::ExposeSecret`. We pull it out here at the
            // last moment before handing it to `ReqwestTransport`,
            // which owns the `frankenstein::Bot` and never logs the
            // token.
            use secrecy::ExposeSecret;
            let telegram_config = TelegramSessionConfig {
                model,
                system_prompt,
                max_tokens: DEFAULT_MAX_TOKENS,
                capabilities,
                tools,
                storage,
                tool_allowlist,
                memory_topic_prefix,
            };
            run_telegram_multi_session(
                "aivyx-telegram",
                token_secret.expose_secret(),
                chat_filter,
                telegram_config,
                provider,
                audit,
                shutdown,
            )
            .await
            .map(|_report| ())
        }
    }
}

#[cfg(test)]
mod tests {
    //! Phase 11 Task 3 — registration-time gate pin tests.
    //!
    //! These tests live in the binary so they sit next to the
    //! gate they protect (`build_shell_exec_for_channel`). The
    //! point is not to re-verify the capability layer (that's
    //! aivyx-telegram's `tier_attenuation_denies_shell_exec_
    //! through_real_telegram_channel` test) but to pin the
    //! *binary's own choice* of what tools to hand each channel.
    //! If a future refactor of `run()` accidentally lifts the
    //! `shell.exec` append out of the `ChannelKind::Local` arm,
    //! this test breaks loudly.
    //!
    //! No `tempfile` crate — the zero-new-dep streak is sacred.
    //! The `Scratch` helper mirrors the one in
    //! `aivyx-core/src/tools/shell.rs`.
    use super::*;
    use std::path::PathBuf;

    struct Scratch {
        dir: PathBuf,
    }

    impl Scratch {
        fn new() -> Self {
            let tmp = std::env::var("TMPDIR")
                .or_else(|_| std::env::var("TEMP"))
                .unwrap_or_else(|_| "/tmp".to_string());
            let dir = PathBuf::from(tmp)
                .join(format!("aivyx-bin-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).expect("scratch dir must be creatable");
            let canonical = std::fs::canonicalize(&dir).expect("canonicalize scratch");
            Scratch { dir: canonical }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn channel_local_receives_shell_exec() {
        let scratch = Scratch::new();
        let result = build_shell_exec_for_channel(ChannelKind::Local, &scratch.dir)
            .expect("local branch must build shell.exec cleanly");
        let (tool, scope) = result.expect("local must receive shell.exec");
        assert_eq!(tool.name(), "shell.exec");
        assert_eq!(scope.base(), "shell.exec");
        let qualifier = scope.qualifier().expect("scope must be qualified");
        // The `cwd:` prefix is required — that's the Phase 11
        // Task 3 convention that lets one base name carry two
        // attenuation shapes (path-glob vs. program-allowlist).
        assert!(
            qualifier.starts_with("cwd:"),
            "shell.exec scope must use `cwd:` qualifier prefix, got {qualifier}"
        );
        assert!(
            qualifier.ends_with("/**"),
            "shell.exec scope must end with `/**`, got {qualifier}"
        );
    }

    // -----------------------------------------------------------------
    // Phase 11 Task 4 — `--role <name>` CLI flag parser tests.
    //
    // These pin the binary's own arg parser. The higher-level priority
    // chain (`--role` beats `AIVYX_ROLE` beats TOML beats `"default"`)
    // is covered by `aivyx-config`'s test `role_override_beats_env_var`
    // and its siblings — the binary's contribution is turning the flag
    // into `LoadOptions::role_override`, which is what these tests
    // verify.
    // -----------------------------------------------------------------

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn role_flag_parses_into_cli_args_role_field() {
        let parsed = parse_cli_args_from(&argv(&["--role", "researcher"]))
            .expect("`--role researcher` must parse");
        assert_eq!(parsed.role.as_deref(), Some("researcher"));
        assert!(!parsed.verify_only);
        assert_eq!(parsed.channel, ChannelKind::Local);
    }

    #[test]
    fn role_flag_absent_leaves_role_none() {
        // No `--role` at all — the parser must return `None` so that
        // `LoadOptions::role_override = None` and the config layer
        // falls through to `AIVYX_ROLE` / TOML / `"default"`.
        let parsed = parse_cli_args_from(&argv(&[])).expect("empty argv must parse");
        assert!(parsed.role.is_none());
    }

    #[test]
    fn role_flag_missing_value_is_an_error() {
        let err = parse_cli_args_from(&argv(&["--role"]))
            .expect_err("`--role` with no value must error");
        assert!(
            err.contains("--role"),
            "error must mention the flag: {err}"
        );
    }

    #[test]
    fn role_flag_empty_value_is_an_error() {
        // An explicit empty string as the role name is a user mistake
        // we catch at the binary rather than forwarding as a "role not
        // found" error from the config layer — closer to the operator,
        // clearer message.
        let err = parse_cli_args_from(&argv(&["--role", ""]))
            .expect_err("`--role ''` must error");
        assert!(
            err.contains("non-empty"),
            "error must call out non-empty: {err}"
        );
    }

    #[test]
    fn role_flag_combines_with_channel_flag() {
        // `--role` and `--channel` are orthogonal and must compose.
        let parsed = parse_cli_args_from(&argv(&["--channel", "telegram", "--role", "coder"]))
            .expect("orthogonal flags must compose");
        assert_eq!(parsed.channel, ChannelKind::Telegram);
        assert_eq!(parsed.role.as_deref(), Some("coder"));
    }

    #[test]
    fn channel_telegram_receives_no_shell_exec() {
        // The binary's single-match gate must return `None` for
        // the Telegram (SemiTrusted) branch. If this test ever
        // fails, it means `shell.exec` has leaked into a
        // SemiTrusted registry at registration time — which
        // would bypass the strictness Phase 11 Task 3 requires
        // (no mention in audit chains, not even as denials).
        let scratch = Scratch::new();
        let result = build_shell_exec_for_channel(ChannelKind::Telegram, &scratch.dir)
            .expect("telegram branch must not error — it's a no-op");
        assert!(
            result.is_none(),
            "Telegram channel must NOT receive shell.exec; \
             this is the registration-time gate"
        );
    }

    // -----------------------------------------------------------------
    // Phase 12 Task 2 — registration-time gate for `web.fetch`.
    //
    // These are the symmetric counterparts to the `shell.exec` gate
    // tests above. The property Phase 12 pins: `web.fetch` is
    // registered for BOTH `Local` (Trusted) and `Telegram`
    // (SemiTrusted) channels. If a future refactor accidentally
    // restricts the tool to one tier, these tests break loudly.
    // -----------------------------------------------------------------

    #[test]
    fn channel_local_receives_web_fetch() {
        let tool = build_web_fetch_for_channel(ChannelKind::Local)
            .expect("local branch must build web.fetch cleanly");
        assert_eq!(tool.name(), "web.fetch");
    }

    #[test]
    fn channel_telegram_receives_web_fetch() {
        // The opposite property from shell.exec: Telegram MUST
        // receive web.fetch. Network reads are inside the
        // SemiTrusted default ceiling and a researcher agent
        // attached to Telegram should be able to fetch URLs.
        // If this test ever fails, it means web.fetch has been
        // accidentally restricted to Trusted-only — which
        // would silently regress the Phase 12 "Telegram
        // researcher can fetch" goal.
        let tool = build_web_fetch_for_channel(ChannelKind::Telegram)
            .expect("telegram branch must build web.fetch cleanly");
        assert_eq!(tool.name(), "web.fetch");
    }
}

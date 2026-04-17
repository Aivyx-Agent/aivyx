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
use aivyx_capability::Scope;
use aivyx_channel::passphrase::{derive_master_key, PassphraseSource, DEFAULT_ENV_VAR};
use aivyx_channel::daemon_ipc::default_socket_path;
use aivyx_channel::daemon_server::{run_daemon, ChannelFactory};
use aivyx_channel::daemon_client::DaemonSession;
use aivyx_channel::{
    assemble_role_envelope, render_role_envelope, run_daemon_session_connected, run_session,
    ChannelKind, DaemonSessionConfig, LocalChannel, SessionConfig,
};
use aivyx_config::{AivyxConfig, FieldSource, LoadOptions, ToolAllowlist};
use aivyx_core::tools::role_switch::{ChildAgentFactory, RoleSwitchTool};
use aivyx_core::{
    Agent, AgentId, AuditHook, CancellationToken, ConcreteAgent, FsReadToolConfig,
    FsWriteToolConfig, LlmPlanner, LlmPlannerConfig, ShellExecToolConfig, Tool, ToolRegistry,
    WebFetchToolConfig,
};
use aivyx_crypto::Argon2Params;
use aivyx_memory::{
    Memory, MemoryForgetTool, MemoryReadTool, MemoryWriteTool, RedbMemory,
};
use aivyx_llm::anthropic::{AnthropicConfig, AnthropicProvider};
use aivyx_llm::LlmProvider;
use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
use aivyx_channel::mission_tool::MissionCreateTool;
use aivyx_channel::telegram_daemon_frontend::{
    run_telegram_daemon_multi_session, TelegramDaemonChannel,
};
use aivyx_telegram::transport::ReqwestTransport;
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

// Phase 13 Task 2's `assemble_role_envelope` walker lifted into
// `aivyx-channel/src/role_envelope.rs` in Phase 14 Task 1. The
// function is now reachable at `aivyx_channel::assemble_role_
// envelope` for every library-side caller (including the future
// `role.switch` tool from Phase 14 Tasks 2–3). The binary
// continues to call it through the re-export in the `use
// aivyx_channel::...` block above, so every production call site
// and the `tests::example_aivyx_toml_*` regression block below
// works unchanged. See `role_envelope.rs` for the doc-comment and
// `MAX_INHERITANCE_DEPTH` const that used to live here.

// Phase 13 Task 4's `--print-role` rendering helpers lived here
// as binary-private free functions until Phase 15 Task 3, which
// lifted `render_role_envelope` + the two private helpers
// `build_display_floor` and `drop_reason_for` into
// `aivyx-channel/src/role_render.rs` alongside the `ChannelKind`
// enum they take as a parameter. The renderer is now reachable as
// `aivyx_channel::render_role_envelope`, and its functional tests
// moved to `crates/aivyx-channel/tests/role_render_e2e.rs`. The
// parse-time `--print-role` CLI tests stay in `mod tests` below
// because they exercise `parse_cli_args`, which remains
// binary-private. See `role_render.rs` for the rendered-output
// structure and per-section teaching commentary.


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
    let CliArgs {
        mode,
        channel: channel_kind,
        role: role_override,
        no_daemon,
    } = parse_cli_args()?;

    // ---- Lightweight daemon management subcommands ----------------------
    // These need only the socket path — no API key, no config, no store.
    // A minimal tokio runtime is spun up just for the IPC round-trip.
    if matches!(mode, CliMode::DaemonStatus | CliMode::DaemonStop) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
        return rt.block_on(run_daemon_management(mode));
    }

    let verify_only = mode == CliMode::VerifyOnly;
    let print_role = match &mode {
        CliMode::PrintRole(name) => Some(name.clone()),
        _ => None,
    };

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
    // `--print-role` is a debug exit mode that should not require
    // any of the secret-bearing fields. `--verify-only` already
    // gets the same treatment for `require_api_key`; `--print-role`
    // additionally relaxes `require_telegram_token` because the
    // print path never opens a telegram connection regardless of
    // `--channel`.
    let print_role_mode = print_role.is_some();
    let load_opts = LoadOptions {
        toml_path: Some(PathBuf::from(DEFAULT_TOML_PATH)),
        require_api_key: !verify_only && !print_role_mode,
        require_telegram_token: matches!(channel_kind, ChannelKind::Telegram) && !print_role_mode,
        // Phase 11 Task 4 — `--role <name>` is now the highest-
        // priority source. `parse_cli_args` turns the flag into
        // `role_override`, which `aivyx-config`'s resolver honors
        // above `AIVYX_ROLE` / TOML / `"default"`. A `None` here
        // means "no flag was passed — fall through to env/TOML."
        //
        // For `--print-role`, the print-role name *is* the active
        // role for the load: this is the cleanest way to surface a
        // typo'd name as `UnknownRole` with the candidate list at
        // load time, instead of either fabricating a synthetic
        // "default" expectation that may not exist in the config or
        // letting the print branch hit a `roles.get(name)` with no
        // helpful error context. The trade-off: the print branch
        // never starts a session, so "active role" here is purely a
        // load-time validation hook, not a runtime behavior.
        role_override: print_role.clone().or(role_override),
    };
    let mut config = AivyxConfig::load_from_env_and_toml(&load_opts)?;

    // Phase 13 Task 4 — `--print-role` exit branch. Lands here,
    // *before* `mkdir fs_root`, master-key derivation, store open,
    // and runtime build. None of those are needed to render an
    // envelope; skipping them keeps `--print-role` fast, free of
    // passphrase prompts, and free of filesystem side effects (no
    // sandbox dir creation, no store file creation). The
    // `print_role` Option holds the requested role name.
    if let Some(name) = print_role {
        let rendered = render_role_envelope(&name, &config, channel_kind)?;
        print!("{rendered}");
        return Ok(());
    }

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
            mode,
            no_daemon,
        )
        .await
    })
}

async fn run_daemon_management(mode: CliMode) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    match mode {
        CliMode::DaemonStatus => {
            let info = aivyx_channel::daemon_client::daemon_status(&socket_path).await;
            if info.running {
                let version = info.version.as_deref().unwrap_or("unknown");
                let pid_str = info.pid
                    .map(|p| format!("  pid: {p}\n"))
                    .unwrap_or_default();
                eprintln!(
                    "aivyx daemon: running (protocol {version})\n  socket: {}\n{pid_str}",
                    socket_path.display(),
                );
            } else {
                eprintln!(
                    "aivyx daemon: not running\n  socket: {} (not listening)",
                    socket_path.display(),
                );
            }
        }
        CliMode::DaemonStop => {
            if !aivyx_channel::daemon_client::daemon_is_running(&socket_path).await {
                eprintln!(
                    "aivyx daemon: not running — nothing to stop.\n  socket: {}",
                    socket_path.display(),
                );
                return Ok(());
            }
            match aivyx_channel::daemon_client::daemon_stop(&socket_path).await {
                Ok(reason) => {
                    eprintln!("aivyx daemon: stopped ({reason})");
                }
                Err(e) => {
                    eprintln!("aivyx daemon: stop failed — {e}");
                    return Err(e);
                }
            }
        }
        _ => unreachable!("run_daemon_management called with non-management mode"),
    }
    Ok(())
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

// `ChannelKind` lifted into `crates/aivyx-channel/src/role_render.rs`
// in Phase 15 Task 3. The binary still constructs values of this
// type from CLI parsing (`--channel local|telegram`), but the enum
// itself is library-side so `aivyx_channel::render_role_envelope`
// and any future IPC frontends share one discriminator.

/// Phase 17 Task 3 — the binary's primary mode of operation.
#[derive(Debug, PartialEq)]
enum CliMode {
    /// Default: interactive REPL session (in-process or over daemon).
    Session,
    /// `--verify-only`: forensic audit-chain verification, no session.
    VerifyOnly,
    /// `--print-role <name>`: render a role's capability envelope.
    PrintRole(String),
    /// `aivyx daemon run`: launch the daemon in the foreground.
    DaemonRun,
    /// `aivyx daemon status`: check whether a daemon is running.
    DaemonStatus,
    /// `aivyx daemon stop`: send graceful shutdown to a running daemon.
    DaemonStop,
}

/// Parsed CLI arg bundle. The shape is intentionally closed — each
/// new argument lands here, so the parser's failure mode is
/// "unrecognized argument" rather than "silently ignored flag."
#[derive(Debug)]
struct CliArgs {
    mode: CliMode,
    channel: ChannelKind,
    role: Option<String>,
    no_daemon: bool,
}

/// Parse the CLI arg surface.
///
/// Recognized forms:
///
/// - `aivyx` — local REPL, fresh session (default).
/// - `aivyx --verify-only` — forensic verification path.
/// - `aivyx --channel local` — explicit form of the default.
/// - `aivyx --channel telegram` — Phase 8 Task 4 Telegram bot mode.
/// - `aivyx --role <name>` — Phase 11 Task 4.
/// - `aivyx --print-role <name>` — Phase 13 Task 4.
/// - `aivyx daemon run` — Phase 17 Task 3 daemon foreground mode.
///
/// Mutual exclusions: `--verify-only` vs `--channel`, `--verify-only`
/// vs `--print-role`, `daemon run` vs all other modes.
fn parse_cli_args() -> Result<CliArgs, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    parse_cli_args_from(&args)
}

/// Testable core of [`parse_cli_args`].
fn parse_cli_args_from(args: &[String]) -> Result<CliArgs, String> {
    // Check for `daemon <subcommand>` first.
    if args.len() >= 2 && args[0] == "daemon" {
        let (mode, subcmd) = match args[1].as_str() {
            "run" => (CliMode::DaemonRun, "daemon run"),
            "status" => (CliMode::DaemonStatus, "daemon status"),
            "stop" => (CliMode::DaemonStop, "daemon stop"),
            other => {
                return Err(format!(
                    "unrecognized daemon subcommand: `{other}`. \
                     Supported: daemon run, daemon status, daemon stop"
                ));
            }
        };
        if args.len() > 2 {
            return Err(format!(
                "unrecognized argument after `{subcmd}`: `{}`. \
                 `{subcmd}` takes no additional flags.",
                args[2]
            ));
        }
        return Ok(CliArgs {
            mode,
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
        });
    }

    let mut verify_only = false;
    let mut channel = ChannelKind::Local;
    let mut role: Option<String> = None;
    let mut print_role: Option<String> = None;
    let mut no_daemon = false;

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
            "--print-role" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "`--print-role` requires a value".to_string())?;
                if value.is_empty() {
                    return Err("`--print-role` requires a non-empty name".to_string());
                }
                print_role = Some(value.clone());
                i += 2;
            }
            "--no-daemon" => {
                no_daemon = true;
                i += 1;
            }
            "daemon" => {
                return Err(
                    "unrecognized subcommand. Did you mean `daemon run`, `daemon status`, or `daemon stop`?".to_string()
                );
            }
            other => {
                return Err(format!(
                    "unrecognized argument: `{other}`. \
                     Supported: --verify-only, --channel <local|telegram>, --role <name>, --print-role <name>, --no-daemon, daemon run|status|stop"
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

    if verify_only && print_role.is_some() {
        return Err(
            "`--verify-only` and `--print-role` are mutually exclusive exit modes. \
             Pick one: verify (audit-chain replay) or print-role (capability envelope render)."
                .to_string(),
        );
    }

    if no_daemon && verify_only {
        return Err(
            "`--no-daemon` and `--verify-only` cannot be combined. \
             `--verify-only` does not use the daemon."
                .to_string(),
        );
    }

    if no_daemon && print_role.is_some() {
        return Err(
            "`--no-daemon` and `--print-role` cannot be combined. \
             `--print-role` does not use the daemon."
                .to_string(),
        );
    }

    let mode = if verify_only {
        CliMode::VerifyOnly
    } else if let Some(name) = print_role {
        CliMode::PrintRole(name)
    } else {
        CliMode::Session
    };

    Ok(CliArgs {
        mode,
        channel,
        role,
        no_daemon,
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
    mode: CliMode,
    no_daemon: bool,
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
        mcp_servers,
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
    // Phase 13 Task 2 — keep a second handle on the active role
    // for `assemble_role_envelope` to borrow downstream. The
    // per-field destructure below moves `system_prompt`,
    // `tool_allowlist`, and `memory_topic_prefix` out of `role`,
    // which would leave `role` partially moved by the time the
    // envelope-assembly call runs. The clone is cheap (one
    // role, a few `Sourced<T>` fields) and confines the move
    // discipline to two adjacent lines.
    let role_for_envelope = role.clone();
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

    // Phase 14 Task 3 — `role.switch` sub-agent primitive. The
    // tool is created here with an empty `child_factory` slot
    // (an internal `OnceLock`) and pushed into the registry
    // alongside the Phase 4–12 tools. After the registry is
    // built *and* after `provider` / `audit` / `backcompat_floor`
    // are in scope below, we come back with
    // `role_switch_tool.set_child_factory(...)` and install the
    // real wiring. The outer `Arc<RoleSwitchTool>` handle kept
    // here and the registry's `Arc<dyn Tool>` clone point at the
    // same instance, so the `OnceLock` write is visible through
    // both the dispatch lookup and the outer handle. See
    // `RoleSwitchTool`'s struct doc for the rationale behind the
    // initialization dance.
    //
    // The tool is registered unconditionally (both Local and
    // Telegram channels). On a SemiTrusted channel,
    // `role.switch` is still registered but the turn loop's
    // ceiling intersection strips the scope — `CEILING_TRUSTED`
    // holds `role.switch` but `CEILING_SEMITRUSTED` does not —
    // so any call dispatched by a SemiTrusted planner hits the
    // scope gate's `Denied` path before reaching `execute`. This
    // is deliberate: role-switching is a Trusted-tier operation
    // per Phase 14 Task 2's ceiling decision.
    let role_switch_tool: Arc<RoleSwitchTool> = Arc::new(RoleSwitchTool::new());
    tool_list.push(Arc::clone(&role_switch_tool) as Arc<dyn Tool>);

    let mission_create_tool: Arc<MissionCreateTool> = Arc::new(MissionCreateTool::new());
    tool_list.push(Arc::clone(&mission_create_tool) as Arc<dyn Tool>);

    let mut mcp_bridges: Vec<aivyx_mcp::McpServerBridge> = Vec::new();
    for mcp_cfg in &mcp_servers {
        let args_ref: Vec<&str> = mcp_cfg.args.iter().map(|s| s.as_str()).collect();
        match aivyx_mcp::McpServerBridge::start(
            &mcp_cfg.command,
            &args_ref,
            &mcp_cfg.name,
        )
        .await
        {
            Ok(bridge) => {
                match bridge.discover_tools().await {
                    Ok(mcp_tools) => {
                        let count = mcp_tools.len();
                        tool_list.extend(mcp_tools);
                        eprintln!(
                            "aivyx: MCP server {:?} — {} tool(s) registered",
                            mcp_cfg.name, count,
                        );
                    }
                    Err(e) => {
                        eprintln!(
                            "aivyx: MCP server {:?} tool discovery failed: {e}",
                            mcp_cfg.name,
                        );
                    }
                }
                mcp_bridges.push(bridge);
            }
            Err(e) => {
                eprintln!(
                    "aivyx: MCP server {:?} failed to start: {e}",
                    mcp_cfg.name,
                );
            }
        }
    }

    let tools: Arc<ToolRegistry> = Arc::new(ToolRegistry::new(tool_list));

    // ---- Capabilities -------------------------------------------------
    // Phase 13 Task 2 — capability assembly is now role-driven.
    // The hard-coded vector below is the **backcompat floor**
    // (Q6): it represents the capabilities the binary used to
    // grant unconditionally before Phase 13 introduced per-role
    // envelopes, and it is now used **only** for roles whose
    // declared `capability_scopes` list is empty. A role that
    // declares any non-empty `capability_scopes` set in TOML
    // bypasses this floor entirely and runs with exactly its
    // declared envelope, walked through its inheritance chain
    // by `assemble_role_envelope` per PRODUCT.md P7's
    // attenuation rule.
    //
    // The fs.* scopes are still rooted at the canonicalized
    // sandbox path so `FsReadTool::required_scope` lines up
    // exactly with the held capability — that's a per-process
    // anchor, not a per-role decision, so it stays inside the
    // floor. The three `memory.*` scopes remain unqualified
    // (D4 Rule 2 — unqualified held grants any qualified
    // needed). `shell.exec` is appended only on the Local
    // branch because the tool itself is absent from the
    // SemiTrusted dispatch registry. `net.fetch` is granted
    // unqualified for both tiers; the turn loop's ceiling
    // intersection narrows it for SemiTrusted via
    // `CEILING_SEMITRUSTED`.
    let mut backcompat_floor: Vec<Scope> = vec![
        Scope::parse("memory.read").unwrap(),
        Scope::parse("memory.write").unwrap(),
        Scope::parse("memory.forget").unwrap(),
        fs_read_scope,
        fs_write_scope,
        Scope::parse("net.fetch").unwrap(),
    ];
    if let Some(s) = shell_exec_scope {
        backcompat_floor.push(s);
    }

    // Walk the active role's inheritance chain, intersecting
    // declared scopes leaf-to-root. Empty levels substitute the
    // floor. This is the new primary code path for any operator
    // who has written a `[[role]]` entry; the floor is consulted
    // only as a per-empty-level fallback.
    let role_envelope = assemble_role_envelope(&role_for_envelope, &roles, &backcompat_floor);

    // Q3 — apply the role's declared `trust_ceiling` as a
    // second intersection layer. This composes with the turn
    // loop's existing per-turn `channel.tier().default_ceiling()`
    // intersection (Phase 11) to give the effective ceiling
    // `min(channel_tier, role_tier)` per `default_ceiling`'s
    // tier-table entries. A role declaring `Trusted` on a
    // SemiTrusted channel still runs at SemiTrusted (the
    // channel layer wins); a role declaring `SemiTrusted` on
    // a Trusted channel runs at SemiTrusted (the role layer
    // chooses to run more restrictively). Both directions
    // resolve to the more-restrictive tier, which is the
    // structural-impossibility rule from P1+P7.
    let role_tier_ceiling = role_for_envelope.trust_ceiling.value.default_ceiling();
    let capabilities = role_envelope.intersect(role_tier_ceiling);

    // ---- Phase 14 Task 3 — wire the role.switch child factory --------
    //
    // Everything the child agent factory needs is now in scope:
    // provider, audit, tool registry, the full roles map, the
    // backcompat floor, the model id, and the token budget. We
    // build a closure that captures `Arc::clone`s of the shared
    // handles (and plain clones of the non-Arc data), then install
    // it into the `RoleSwitchTool` via `OnceLock::set`.
    //
    // The closure's signature — `Fn(&str) -> Result<Box<dyn Agent>,
    // String>` — takes a target role name and returns either a
    // fully-wired child agent or a human-readable error. Errors
    // here surface as `ToolOutcome::Failed` from `role.switch`'s
    // `execute`, which the parent's planner observes as a tool
    // failure and can react to (typically by reporting the error
    // in its next message to the user).
    //
    // ### Structural-impossibility rationale
    //
    // The closure *must* call `assemble_role_envelope` against the
    // target role — there is no other code path inside it that
    // synthesizes a `CapabilitySet`. This is the type-system
    // enforcement of PRODUCT.md P1.3: a child cannot hold any
    // scope that the parent's chain does not transitively grant,
    // because the child's envelope is always computed by walking
    // the target's chain and intersecting against the same
    // `backcompat_floor` the parent was built against. The
    // `role_tier_ceiling` intersection below adds the second gate
    // (child's declared tier, same semantics as the parent's).
    // The channel layer's per-turn ceiling then composes on top
    // when `child.turn()` runs, giving a three-way intersection
    // identical to the one the parent sees.
    //
    // ### Memory-topic-prefix and tool-allowlist inheritance
    //
    // Phase 11 Tasks 2 and 4 wire role-derived memory prefixes
    // and tool allowlists through the agent builder (`.with_*`).
    // The child factory applies the *target* role's values here,
    // not the parent's: a child `researcher` reads and writes
    // memory under `researcher/` even if the parent is `coder`
    // running with `coder/`. This is deliberate — the memory
    // namespace is one of the things the role config actually
    // configures, and inheriting it from the parent would defeat
    // the partitioning.
    let provider_for_factory = Arc::clone(&provider);
    let audit_for_factory = Arc::clone(&audit);
    let tools_for_factory = Arc::clone(&tools);
    let roles_for_factory = roles.clone();
    let backcompat_floor_for_factory = backcompat_floor.clone();
    let model_for_factory = model.clone();
    let max_tokens_for_factory: u32 = DEFAULT_MAX_TOKENS;

    let child_factory: Arc<ChildAgentFactory> = Arc::new(move |target: &str| {
        // Resolve the target role. `roles` is the same validated
        // map the parent was built against, so a missing key is a
        // planner bug (or an operator config change mid-session,
        // which the current architecture doesn't support) — the
        // error message distinguishes the two cases.
        let target_role = roles_for_factory
            .get(target)
            .ok_or_else(|| {
                format!(
                    "unknown target role {target:?}; declared roles are: {}",
                    roles_for_factory
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?
            .clone();

        // Compute the child's effective capability envelope.
        // Structural-impossibility property: the only path that
        // produces a child `CapabilitySet` goes through
        // `assemble_role_envelope`, which walks the target's
        // ancestor chain and intersects at every level.
        let child_envelope = assemble_role_envelope(
            &target_role,
            &roles_for_factory,
            &backcompat_floor_for_factory,
        );
        let child_tier_ceiling = target_role.trust_ceiling.value.default_ceiling();
        let child_capabilities = child_envelope.intersect(child_tier_ceiling);

        // Destructure the target role's per-role config for the
        // child's planner and agent builder. Mirrors the parent
        // path at lines 1193-1199.
        let child_system_prompt = target_role.system_prompt.value;
        let child_tool_allowlist: Option<std::collections::BTreeSet<String>> =
            match target_role.tool_allowlist.value {
                ToolAllowlist::AllowAll => None,
                ToolAllowlist::Only(list) => Some(list.into_iter().collect()),
            };
        let child_memory_topic_prefix: Option<String> =
            target_role.memory_topic_prefix.value;

        // Build the child's planner factory. Same shape as the
        // parent's `run_session` planner factory: captures the
        // provider and registry by Arc, the planner config by
        // value (cloned per-turn). The child's planner is
        // independent of the parent's — a fresh `LlmPlanner` per
        // sub-session turn, exactly like the parent.
        let planner_config = LlmPlannerConfig::new(model_for_factory.clone())
            .with_system_prompt(child_system_prompt)
            .with_max_tokens(max_tokens_for_factory)
            .with_tool_allowlist(child_tool_allowlist.clone());
        let planner_provider = Arc::clone(&provider_for_factory);
        let planner_tools = Arc::clone(&tools_for_factory);
        let child_planner_factory = move || {
            Box::new(LlmPlanner::new(
                Arc::clone(&planner_provider),
                Arc::clone(&planner_tools),
                planner_config.clone(),
            )) as Box<dyn aivyx_core::TurnPlanner>
        };

        let child_agent = ConcreteAgent::new(
            AgentId::new(),
            child_capabilities,
            Arc::clone(&tools_for_factory),
            Arc::clone(&audit_for_factory),
            child_planner_factory,
        )
        .with_tool_allowlist(child_tool_allowlist)
        .with_memory_topic_prefix(child_memory_topic_prefix);

        Ok(Box::new(child_agent) as Box<dyn Agent>)
    });

    // Install the factory into the tool. `set_child_factory`
    // returns `Err` if called twice — we treat that as a binary
    // startup bug (the factory is built exactly once at this
    // site) and surface it as a startup error rather than
    // silently dropping the factory.
    role_switch_tool
        .set_child_factory(child_factory)
        .map_err(|_| {
            "role.switch child factory was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;

    mission_create_tool
        .set_mission_store(storage.domain(KeyDomain::Missions))
        .map_err(|_| {
            "mission.create store was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    mission_create_tool
        .set_role_name(active_role_name.clone())
        .map_err(|_| {
            "mission.create role_name was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;

    // ---- Phase 17 Task 3: daemon-run branch ----------------------------
    // If the operator invoked `aivyx daemon run`, launch the daemon
    // server in the foreground. The daemon reuses the same agent,
    // provider, audit, and capability stack as the in-process path.
    if mode == CliMode::DaemonRun {
        let socket_path = default_socket_path()?;

        let daemon_tool_allowlist = tool_allowlist.clone();
        let planner_config = LlmPlannerConfig::new(model.clone())
            .with_system_prompt(system_prompt)
            .with_max_tokens(DEFAULT_MAX_TOKENS)
            .with_tool_allowlist(tool_allowlist);
        let planner_provider = Arc::clone(&provider);
        let planner_tools = Arc::clone(&tools);
        let planner_factory = move || {
            Box::new(LlmPlanner::new(
                Arc::clone(&planner_provider),
                Arc::clone(&planner_tools),
                planner_config.clone(),
            )) as Box<dyn aivyx_core::TurnPlanner>
        };
        let agent: Arc<dyn Agent> = Arc::new(
            ConcreteAgent::new(
                AgentId::new(),
                capabilities,
                tools,
                audit,
                planner_factory,
            )
            .with_tool_allowlist(daemon_tool_allowlist)
            .with_memory_topic_prefix(memory_topic_prefix),
        );

        let channel_factory: ChannelFactory = Arc::new(|frontend_type| {
            match frontend_type {
                aivyx_channel::daemon_ipc::FrontendType::Telegram => {
                    Arc::new(TelegramDaemonChannel::new())
                }
                aivyx_channel::daemon_ipc::FrontendType::Local => {
                    Arc::new(LocalChannel::new("aivyx-daemon", io::stdout()))
                }
            }
        });

        let shutdown = CancellationToken::new();
        let shutdown_for_signal = shutdown.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_err() {
                std::process::exit(130);
            }
            eprintln!("\naivyx daemon: shutting down.");
            shutdown_for_signal.cancel();
        });

        eprintln!(
            "aivyx daemon {} — listening on {}",
            env!("CARGO_PKG_VERSION"),
            socket_path.display(),
        );

        let result = run_daemon(
            &socket_path,
            agent,
            channel_factory,
            shutdown,
            Some(storage.domain(KeyDomain::Missions)),
        )
            .await;

        for bridge in mcp_bridges {
            let _ = bridge.shutdown().await;
        }
        return result;
    }

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
    // reports `SemiTrusted` and `shell.exec` would be stripped.
    match channel_kind {
        ChannelKind::Local => {
            // Phase 18 Task 3: try daemon-backed REPL first, fall back
            // to in-process if the daemon path fails.
            // Phase 20 Task 4: `--no-daemon` skips daemon dispatch entirely.
            if !no_daemon && let Ok(sp) = default_socket_path() {
                let session = DaemonSession::connect(
                    &sp,
                    Some(active_role_name.clone()),
                    Some(aivyx_channel::daemon_ipc::FrontendType::Local),
                ).await;

                if let Ok(session) = session {
                    let cancel_handle = session.cancel_handle();
                    let cancelled_once = std::sync::Arc::new(
                        std::sync::atomic::AtomicBool::new(false),
                    );
                    let flag_for_signal = std::sync::Arc::clone(&cancelled_once);

                    // Signal task (daemon mode): first ctrl-C sends
                    // CancelTurn; second ctrl-C exits. The REPL loop
                    // resets `cancelled_once` to false before each turn
                    // via `DaemonSessionConfig::cancel_flag`.
                    tokio::spawn(async move {
                        loop {
                            if tokio::signal::ctrl_c().await.is_err() {
                                std::process::exit(130);
                            }
                            if flag_for_signal.load(std::sync::atomic::Ordering::Relaxed) {
                                eprintln!("\naivyx: interrupted, exiting.");
                                std::process::exit(130);
                            }
                            eprintln!(
                                "\naivyx: cancelling in-flight turn (ctrl-C again to exit)."
                            );
                            cancel_handle.cancel().await;
                            flag_for_signal.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    });

                    let daemon_config = DaemonSessionConfig {
                        socket_path: sp.clone(),
                        role: Some(active_role_name.clone()),
                        prompt: PROMPT.to_string(),
                        banner: Some(format!(
                            "aivyx {} (daemon) — type a message, ctrl-C to cancel, \
                             ctrl-D to exit.\n\
                             daemon: {}\n\
                             fs sandbox: {}\n\
                             memory: live (recall persists across restarts)\n\
                             audit: persistent ({} events verified from disk)\n\
                             active role: {}",
                            env!("CARGO_PKG_VERSION"),
                            sp.display(),
                            canonical_root.display(),
                            verified_event_count,
                            active_role_name,
                        )),
                        cancel_flag: Some(cancelled_once),
                        frontend_type: Some(aivyx_channel::daemon_ipc::FrontendType::Local),
                    };

                    let stdin = io::stdin();
                    let reader = stdin.lock();
                    match run_daemon_session_connected(
                        session, daemon_config, reader, io::stdout(),
                    ).await {
                        Ok(_report) => return Ok(()),
                        Err(e) => {
                            eprintln!(
                                "aivyx: daemon session failed ({e}), \
                                 falling back to in-process."
                            );
                        }
                    }
                } else {
                    eprintln!(
                        "aivyx: no daemon at {}, using in-process mode.",
                        sp.display(),
                    );
                }
            } else {
                eprintln!("aivyx: no socket path available, using in-process mode.");
            }

            // In-process fallback (original Phase 3 path).
            let channel = LocalChannel::new("aivyx-cli", io::stdout());
            let token_slot = channel.token_slot();

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
            let tg = telegram
                .expect("telegram config validated for ChannelKind::Telegram");
            let token_secret = tg
                .token
                .expect("telegram.token validated non-None before run_async")
                .value;
            let chat_filter: Option<i64> = tg.chat_filter.map(|c| c.value);

            let shutdown = CancellationToken::new();
            let shutdown_for_signal = shutdown.clone();
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_err() {
                    std::process::exit(130);
                }
                eprintln!("\naivyx: shutting down telegram bot after current poll completes.");
                shutdown_for_signal.cancel();
            });

            let chat_scope_label: String = match chat_filter {
                Some(chat_id) => format!("chat_id: {chat_id} (single-chat mode)"),
                None => "chat_id: <any> (multi-chat mode)".to_string(),
            };

            use secrecy::ExposeSecret;
            let token_str = token_secret.expose_secret();

            // Phase 19 Task 3: daemon-first, in-process fallback —
            // same pattern as the Local branch.
            // Phase 20 Task 4: `--no-daemon` skips daemon dispatch entirely.
            if !no_daemon && let Ok(sp) = default_socket_path() {
                let transport = Arc::new(ReqwestTransport::new(token_str));

                eprintln!(
                    "aivyx {} (daemon) — telegram bot live\n\
                     {}\n\
                     daemon: {}\n\
                     fs sandbox: {}\n\
                     memory: live (recall persists across restarts)\n\
                     audit: persistent ({} events verified from disk)",
                    env!("CARGO_PKG_VERSION"),
                    chat_scope_label,
                    sp.display(),
                    canonical_root.display(),
                    verified_event_count,
                );

                match run_telegram_daemon_multi_session(
                    transport,
                    chat_filter,
                    sp.clone(),
                    Some(active_role_name.clone()),
                    shutdown.clone(),
                )
                .await
                {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        eprintln!(
                            "aivyx: daemon telegram session failed ({e}), \
                             falling back to in-process."
                        );
                    }
                }
            } else {
                eprintln!("aivyx: no socket path available, using in-process mode.");
            }

            // In-process fallback (original Phase 8 path).
            eprintln!(
                "aivyx {} — telegram bot live (in-process)\n\
                 {}\n\
                 fs sandbox: {}\n\
                 memory: live (recall persists across restarts)\n\
                 audit: persistent ({} events verified from disk)",
                env!("CARGO_PKG_VERSION"),
                chat_scope_label,
                canonical_root.display(),
                verified_event_count,
            );

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
                token_str,
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
    use aivyx_config::Role;
    use std::collections::BTreeMap;
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
        assert_eq!(parsed.mode, CliMode::Session);
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

    // ================================================================
    // Phase 13 Task 2 — `assemble_role_envelope` regression tests
    // ================================================================
    //
    // These tests pin the leaf-to-root inheritance walk + the
    // empty-level floor substitution rule. They build `Role`
    // fixtures by hand (the `Role` struct is `pub` with `pub`
    // fields per `aivyx-config`'s
    // `role_struct_is_constructible_and_matchable_from_outside`
    // test) so they exercise the assembly fn without booting any
    // of the rest of `run()`'s startup machinery.

    use aivyx_config::{FieldSource, Sourced, ToolAllowlist};

    /// Build a minimal `Role` fixture with the given name,
    /// declared scopes, and parent. Other fields land at their
    /// default-source values — they are not load-bearing for
    /// envelope assembly tests.
    fn make_role(name: &str, scopes: Vec<&str>, parent: Option<&str>) -> Role {
        Role {
            name: Sourced::new(name.to_string(), FieldSource::Default),
            system_prompt: Sourced::new(String::new(), FieldSource::Default),
            tool_allowlist: Sourced::new(ToolAllowlist::AllowAll, FieldSource::Default),
            memory_topic_prefix: Sourced::new(None, FieldSource::Default),
            capability_scopes: Sourced::new(
                scopes
                    .into_iter()
                    .map(|s| Scope::parse(s).unwrap_or_else(|| panic!("bad scope: {s}")))
                    .collect(),
                FieldSource::Default,
            ),
            trust_ceiling: Sourced::new(
                aivyx_capability::TrustTier::Trusted,
                FieldSource::Default,
            ),
            parent_role: Sourced::new(parent.map(String::from), FieldSource::Default),
        }
    }

    fn floor() -> Vec<Scope> {
        vec![
            Scope::parse("memory.read").unwrap(),
            Scope::parse("memory.write").unwrap(),
            Scope::parse("net.fetch").unwrap(),
        ]
    }

    /// A role that declares one capability scope and has no
    /// parent runs with exactly that scope and **bypasses the
    /// floor entirely**. This is the core P9 promise: declare
    /// what you want, get what you declared.
    #[test]
    fn role_with_declared_scope_runs_only_that_scope_not_floor() {
        let role = make_role("solo", vec!["fs.read:/tmp/**"], None);
        let mut roles = BTreeMap::new();
        roles.insert("solo".to_string(), role.clone());

        let envelope = assemble_role_envelope(&role, &roles, &floor());
        let scope_strings: Vec<&str> = envelope.iter().map(|s| s.as_str()).collect();
        assert_eq!(scope_strings, vec!["fs.read:/tmp/**"]);
    }

    /// A role with empty `capability_scopes` and no parent runs
    /// with the backcompat floor. This is the Phase 1–10
    /// zero-config path: a synthesized `default` (or any
    /// operator-declared role with no scopes) inherits the
    /// hard-coded vector that Phase 13 Task 2 demoted from
    /// "primary code path" to "fallback for empty roles."
    #[test]
    fn empty_role_inherits_backcompat_floor_verbatim() {
        let role = make_role("empty", vec![], None);
        let mut roles = BTreeMap::new();
        roles.insert("empty".to_string(), role.clone());

        let envelope = assemble_role_envelope(&role, &roles, &floor());
        let scope_strings: Vec<&str> = envelope.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            scope_strings,
            vec!["memory.read", "memory.write", "net.fetch"]
        );
    }

    /// A child with a narrower declared scope and an empty
    /// parent runs the **intersection** of the child's
    /// declaration with the parent's substituted floor. The
    /// floor grants `memory.read` unqualified, the child
    /// declares `memory.read:topic:secrets`, and intersection
    /// keeps the narrower child scope (D4 Rule 2 — unqualified
    /// held grants any qualified needed with the same base, so
    /// the child scope is granted by the parent's floor and
    /// survives intersection). This pins that the floor
    /// substitution composes correctly with child attenuation,
    /// not just at root level.
    #[test]
    fn child_attenuates_parents_substituted_floor_at_runtime() {
        let parent = make_role("parent", vec![], None);
        let child = make_role(
            "child",
            vec!["memory.read:topic:secrets"],
            Some("parent"),
        );
        let mut roles = BTreeMap::new();
        roles.insert("parent".to_string(), parent);
        roles.insert("child".to_string(), child.clone());

        let envelope = assemble_role_envelope(&child, &roles, &floor());
        let scope_strings: Vec<&str> = envelope.iter().map(|s| s.as_str()).collect();
        // Intersection keeps only `memory.read:topic:secrets` —
        // the child's narrow scope is granted by the parent's
        // floor `memory.read`, and the parent's other floor
        // scopes (`memory.write`, `net.fetch`) are NOT granted
        // by the child's narrow set, so they drop out.
        assert_eq!(scope_strings, vec!["memory.read:topic:secrets"]);
    }

    /// A two-level inheritance walk where both levels declare
    /// non-empty scopes and the child's declarations are a
    /// proper subset of the parent's. Pins that
    /// `assemble_role_envelope` walks the chain and that
    /// intersection at each step preserves the child's
    /// narrower scopes intact.
    #[test]
    fn multi_level_inheritance_preserves_child_attenuation() {
        let parent = make_role("parent", vec!["fs.read", "fs.write"], None);
        let child = make_role(
            "child",
            vec!["fs.read:/etc/**"],
            Some("parent"),
        );
        let mut roles = BTreeMap::new();
        roles.insert("parent".to_string(), parent);
        roles.insert("child".to_string(), child.clone());

        let envelope = assemble_role_envelope(&child, &roles, &floor());
        let scope_strings: Vec<&str> = envelope.iter().map(|s| s.as_str()).collect();
        // Child declared `fs.read:/etc/**`; parent declared
        // `fs.read` (which grants the child's qualified scope
        // by D4 Rule 2) and `fs.write` (no overlap with the
        // child's set). Intersection keeps only the child's
        // narrower scope.
        assert_eq!(scope_strings, vec!["fs.read:/etc/**"]);
    }

    /// `trust_ceiling` intersection at the call site (not in
    /// `assemble_role_envelope`, which deliberately stays
    /// scope-only). A role declaring `SemiTrusted` runs with
    /// the `CEILING_SEMITRUSTED` set even on a Trusted channel
    /// — the role chooses to run more restrictively. Verified
    /// by composing the same intersection step the binary
    /// does. `Untrusted` would be the strictest test, but its
    /// ceiling is so narrow (`memory.read:scope:public:*` +
    /// `audit.read:public`) that no realistic role envelope
    /// survives it; `SemiTrusted` is the right "more
    /// restrictive than channel but still functional" demo
    /// and matches the Telegram-channel attenuation the
    /// binary does in production.
    #[test]
    fn role_declared_trust_ceiling_attenuates_envelope_below_channel_tier() {
        // A role that declares `net.fetch` (which SemiTrusted
        // tier permits) and `shell.exec` (which SemiTrusted
        // strips per the D5 ⊘ list).
        let role = Role {
            name: Sourced::new("locked".to_string(), FieldSource::Default),
            system_prompt: Sourced::new(String::new(), FieldSource::Default),
            tool_allowlist: Sourced::new(ToolAllowlist::AllowAll, FieldSource::Default),
            memory_topic_prefix: Sourced::new(None, FieldSource::Default),
            capability_scopes: Sourced::new(
                vec![
                    Scope::parse("net.fetch").unwrap(),
                    Scope::parse("shell.exec").unwrap(),
                ],
                FieldSource::Default,
            ),
            trust_ceiling: Sourced::new(
                aivyx_capability::TrustTier::SemiTrusted,
                FieldSource::Default,
            ),
            parent_role: Sourced::new(None, FieldSource::Default),
        };
        let mut roles = BTreeMap::new();
        roles.insert("locked".to_string(), role.clone());

        // Compose the same two-step assembly the binary does.
        let envelope = assemble_role_envelope(&role, &roles, &floor());
        let role_tier_ceiling = role.trust_ceiling.value.default_ceiling();
        let capabilities = envelope.intersect(role_tier_ceiling);

        // `net.fetch` survives — `CEILING_SEMITRUSTED` includes
        // network reads. `shell.exec` is stripped — SemiTrusted
        // never gets shell execution per D5. The role declared
        // `SemiTrusted` so this attenuation happens *here*, not
        // in the per-turn channel ceiling intersection.
        let scope_strings: Vec<&str> =
            capabilities.iter().map(|s| s.as_str()).collect();
        assert!(
            scope_strings.contains(&"net.fetch"),
            "net.fetch must survive SemiTrusted ceiling: {scope_strings:?}"
        );
        assert!(
            !scope_strings.contains(&"shell.exec"),
            "shell.exec must be stripped by SemiTrusted ceiling: {scope_strings:?}"
        );
    }

    // ----------------------------------------------------------------
    // Phase 13 Task 3 — `examples/aivyx.toml` worked-case regression
    // ----------------------------------------------------------------
    //
    // These tests load the canonical worked example from
    // `examples/aivyx.toml` (resolved via `CARGO_MANIFEST_DIR`) and
    // pin the runtime envelopes for each of the four declared roles.
    // The example file is a teaching artifact; these tests are the
    // mechanical guarantee that the file's claims about each role's
    // envelope are still true. If a Phase 14 capability-layer change
    // shifts the math, the test breaks loud and the operator-facing
    // doc gets updated alongside it.
    //
    // The test deliberately lives inside `aivyx.rs`'s `mod tests`
    // (not in `crates/aivyx-channel/tests/`) because
    // `assemble_role_envelope` is a binary-private free fn and Rust
    // integration tests cannot reach binary internals. Lifting the
    // fn into `aivyx-channel/src/lib.rs` would be a structural shift
    // beyond Task 3's scope; keeping the test binary-internal is the
    // smaller move and the Task 3 plan explicitly allowed "or an
    // appropriate location."

    /// Build the runtime backcompat floor the binary uses on a
    /// `Local` channel. The path-qualified `fs.read`/`fs.write`
    /// scopes mirror the production code's startup canonicalization
    /// (using `/tmp/sandbox` as a stand-in for the real
    /// `fs_root`), so the `junior_researcher` test below sees the
    /// same floor shape that a real Local-channel session would.
    fn local_channel_floor_with_sandbox(sandbox: &str) -> Vec<Scope> {
        vec![
            Scope::parse("memory.read").unwrap(),
            Scope::parse("memory.write").unwrap(),
            Scope::parse("memory.forget").unwrap(),
            Scope::parse(&format!("fs.read:{sandbox}/**")).unwrap(),
            Scope::parse(&format!("fs.write:{sandbox}/**")).unwrap(),
            Scope::parse("net.fetch").unwrap(),
            Scope::parse("shell.exec").unwrap(),
        ]
    }

    /// Load `examples/aivyx.toml` from the repo root. Returns the
    /// loaded `AivyxConfig` with no env vars set; the example is
    /// designed to load without secrets via `require_api_key:
    /// false`.
    fn load_example_config() -> AivyxConfig {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let example_path = PathBuf::from(manifest_dir)
            .join("..")
            .join("..")
            .join("examples")
            .join("aivyx.toml");
        assert!(
            example_path.exists(),
            "examples/aivyx.toml must exist at {example_path:?}"
        );
        // We need to pick *some* role for `LoadOptions` to succeed;
        // the example file declares all four roles and the active
        // role gets picked here only to satisfy the loader. Each
        // test re-resolves the role it actually wants from
        // `cfg.roles` directly.
        let opts = LoadOptions {
            toml_path: Some(example_path),
            require_api_key: false,
            require_telegram_token: false,
            role_override: Some("default".to_string()),
        };
        AivyxConfig::load_from_env_and_toml(&opts).expect("examples/aivyx.toml must load cleanly")
    }

    /// `coder` declares its own attenuation of `default` and runs
    /// at `Trusted`. The example file's comment block claims the
    /// runtime envelope is exactly the seven scopes coder declared
    /// (since each is granted by `default`'s unqualified
    /// counterpart and `Trusted`'s ceiling keeps everything). This
    /// test pins that claim.
    ///
    /// Phase 14 Task 2 widened the documented set from six to
    /// seven: coder now declares `role.switch:researcher` as well,
    /// which survives intersection with `default`'s unqualified
    /// `role.switch` (Rule 2) and with CEILING_TRUSTED's
    /// unqualified `role.switch` (also Rule 2). No scope is
    /// dropped on this path — the widening is additive.
    #[test]
    fn example_aivyx_toml_coder_envelope_matches_documented_set() {
        let cfg = load_example_config();
        let coder = cfg.roles.get("coder").expect("coder role declared");
        let floor = local_channel_floor_with_sandbox("/tmp/sandbox");

        let envelope = assemble_role_envelope(coder, &cfg.roles, &floor);
        let role_tier_ceiling = coder.trust_ceiling.value.default_ceiling();
        let effective = envelope.intersect(role_tier_ceiling);

        let mut got: Vec<&str> = effective.iter().map(|s| s.as_str()).collect();
        got.sort();
        let mut expected = vec![
            "fs.read",
            "fs.write",
            "memory.read",
            "memory.write",
            "memory.forget",
            "shell.exec",
            "role.switch:researcher",
        ];
        expected.sort();
        assert_eq!(
            got, expected,
            "coder runtime envelope (after role-tier intersection at Trusted) \
             must be exactly the documented seven scopes, including \
             role.switch:researcher from Phase 14 Task 2"
        );
    }

    /// `researcher` attenuates `default` differently — drops
    /// `fs.write` and `shell.exec`, narrows `net.fetch` to a URL
    /// prefix — and runs at `Trusted` (deliberately, so `fs.read`
    /// and `memory.forget` survive the ceiling intersection;
    /// CEILING_SEMITRUSTED omits the unqualified `fs.read` and
    /// `memory.forget` rows by design and would strip them). The
    /// envelope is `researcher_declared ∩ default_declared ∩
    /// CEILING_TRUSTED`, which under D4 reduces to exactly what
    /// `researcher` declared.
    #[test]
    fn example_aivyx_toml_researcher_envelope_matches_documented_set() {
        let cfg = load_example_config();
        let researcher = cfg.roles.get("researcher").expect("researcher role declared");
        let floor = local_channel_floor_with_sandbox("/tmp/sandbox");

        let envelope = assemble_role_envelope(researcher, &cfg.roles, &floor);
        let role_tier_ceiling = researcher.trust_ceiling.value.default_ceiling();
        let effective = envelope.intersect(role_tier_ceiling);

        let mut got: Vec<&str> = effective.iter().map(|s| s.as_str()).collect();
        got.sort();
        let mut expected = vec![
            "fs.read",
            "memory.read",
            "memory.write",
            "memory.forget",
            "net.fetch:url-prefix:https://httpbin.org/",
        ];
        expected.sort();
        assert_eq!(
            got, expected,
            "researcher runtime envelope (after Trusted ceiling) must be \
             exactly the documented five scopes — note unqualified fs.read \
             survives because Trusted ceiling includes the fs.read base"
        );
    }

    /// `junior_researcher` is the **empty-child surprise** case.
    /// Its `capability_scopes` is `[]`, so the assembler
    /// substitutes the binary's backcompat floor for that level.
    /// The envelope is then `floor ∩ researcher_declared ∩
    /// default_declared ∩ CEILING_TRUSTED`. Crucially, the
    /// floor's path-qualified `fs.read:<sandbox>/**` is what
    /// makes it through, NOT `researcher`'s unqualified `fs.read`
    /// (because Rule 4 forbids qualified-held from granting
    /// unqualified-needed). Likewise `floor.net.fetch`
    /// (unqualified) gets dropped because `researcher`'s
    /// qualified URL-prefix scope cannot grant it back, but
    /// `researcher`'s qualified URL-prefix scope itself survives
    /// (granted by `floor.net.fetch` via Rule 2). The Trusted
    /// ceiling then keeps everything that survived intersection.
    ///
    /// This is the entire point of the example file: a config
    /// that *looks* like simple inheritance silently produces a
    /// runtime envelope shaped by the backcompat floor, not the
    /// declared parent. The test pins the behavior so future
    /// refactors of either the floor shape or the assembler walk
    /// trigger a loud failure here, with the example-file
    /// docstring as the natural place to update the operator-
    /// facing explanation.
    #[test]
    fn example_aivyx_toml_junior_researcher_envelope_demonstrates_floor_substitution() {
        let cfg = load_example_config();
        let junior = cfg.roles.get("junior_researcher").expect("junior_researcher role declared");
        let floor = local_channel_floor_with_sandbox("/tmp/sandbox");

        let envelope = assemble_role_envelope(junior, &cfg.roles, &floor);
        let role_tier_ceiling = junior.trust_ceiling.value.default_ceiling();
        let effective = envelope.intersect(role_tier_ceiling);

        let mut got: Vec<&str> = effective.iter().map(|s| s.as_str()).collect();
        got.sort();
        let mut expected = vec![
            "fs.read:/tmp/sandbox/**",
            "memory.read",
            "memory.write",
            "memory.forget",
            "net.fetch:url-prefix:https://httpbin.org/",
        ];
        expected.sort();
        assert_eq!(
            got, expected,
            "junior_researcher runtime envelope demonstrates the \
             backcompat-floor substitution: path-qualified fs.read survives, \
             researcher's unqualified fs.read does NOT, the qualified \
             net.fetch URL-prefix survives but the floor's unqualified \
             net.fetch does NOT, and shell.exec is gone (not in \
             researcher's declared set)"
        );

        // Also pin the divergence from `researcher`'s envelope to
        // make the surprise mechanically visible: the two roles
        // produce *different* runtime sets, even though
        // junior_researcher declared an empty `capability_scopes`
        // and an operator's mental model would expect them
        // identical. The specific diff: junior has
        // `fs.read:/tmp/sandbox/**` (path-qualified); researcher
        // has `fs.read` (unqualified). Same base, same tier
        // ceiling, but different envelopes — entirely because the
        // floor was substituted in for junior's empty level.
        let researcher = cfg.roles.get("researcher").expect("researcher role declared");
        let researcher_envelope = assemble_role_envelope(researcher, &cfg.roles, &floor)
            .intersect(researcher.trust_ceiling.value.default_ceiling());
        let mut researcher_strs: Vec<&str> =
            researcher_envelope.iter().map(|s| s.as_str()).collect();
        researcher_strs.sort();
        assert_ne!(
            got, researcher_strs,
            "junior_researcher and researcher must produce DIFFERENT runtime \
             envelopes: that divergence is the empty-child surprise the \
             example file documents (junior gets fs.read path-qualified by \
             the floor; researcher keeps it unqualified)"
        );
    }

    // ----------------------------------------------------------------
    // Phase 13 Task 4 — `--print-role` flag tests
    // ----------------------------------------------------------------
    //
    // Two layers: parse-time tests (mirror the existing `role_flag_*`
    // tests in shape) and functional tests that drive
    // `render_role_envelope` against `examples/aivyx.toml` and assert
    // that the rendered string contains the load-bearing teaching
    // strings the operator needs to see.
    //
    // The rendered output is asserted *by content*, not by
    // byte-for-byte match, because the floor's path-qualified scopes
    // depend on whether `/tmp/sandbox` happens to exist on the test
    // host (it usually does not, so the rendering uses the
    // as-written path with a footnote — and the footnote is itself
    // one of the things we assert is present).

    #[test]
    fn print_role_flag_parses_into_cli_args() {
        let parsed = parse_cli_args_from(&argv(&["--print-role", "junior_researcher"]))
            .expect("`--print-role junior_researcher` must parse");
        assert_eq!(parsed.mode, CliMode::PrintRole("junior_researcher".into()));
    }

    #[test]
    fn print_role_flag_missing_value_is_an_error() {
        let err = parse_cli_args_from(&argv(&["--print-role"]))
            .expect_err("`--print-role` with no value must error");
        assert!(
            err.contains("--print-role"),
            "error must mention the flag: {err}"
        );
    }

    #[test]
    fn print_role_flag_empty_value_is_an_error() {
        let err = parse_cli_args_from(&argv(&["--print-role", ""]))
            .expect_err("`--print-role ''` must error");
        assert!(
            err.contains("non-empty"),
            "error must call out non-empty: {err}"
        );
    }

    #[test]
    fn print_role_and_verify_only_are_mutually_exclusive() {
        let err = parse_cli_args_from(&argv(&["--verify-only", "--print-role", "coder"]))
            .expect_err("--verify-only + --print-role must error");
        assert!(
            err.contains("mutually exclusive"),
            "error must call out mutual exclusion: {err}"
        );
    }

    #[test]
    fn print_role_composes_with_channel_flag() {
        // `--print-role` and `--channel` are orthogonal — the
        // channel determines which floor gets rendered.
        let parsed = parse_cli_args_from(&argv(&[
            "--channel",
            "telegram",
            "--print-role",
            "researcher",
        ]))
        .expect("orthogonal flags must compose");
        assert_eq!(parsed.channel, ChannelKind::Telegram);
        assert_eq!(parsed.mode, CliMode::PrintRole("researcher".into()));
    }

    // -----------------------------------------------------------------
    // Phase 17 Task 3 — `daemon run` subcommand parser tests.
    // -----------------------------------------------------------------

    #[test]
    fn daemon_run_parses_to_daemon_mode() {
        let parsed = parse_cli_args_from(&argv(&["daemon", "run"]))
            .expect("`daemon run` must parse");
        assert_eq!(parsed.mode, CliMode::DaemonRun);
    }

    #[test]
    fn daemon_without_run_is_an_error() {
        let err = parse_cli_args_from(&argv(&["daemon"]))
            .expect_err("`daemon` alone must error");
        assert!(
            err.contains("daemon run"),
            "error must suggest `daemon run`: {err}"
        );
    }

    #[test]
    fn daemon_run_rejects_extra_args() {
        let err = parse_cli_args_from(&argv(&["daemon", "run", "--verbose"]))
            .expect_err("`daemon run --verbose` must error");
        assert!(
            err.contains("unrecognized"),
            "error must mention unrecognized: {err}"
        );
    }

    #[test]
    fn daemon_run_is_not_combinable_with_channel_flag() {
        let err = parse_cli_args_from(&argv(&["--channel", "telegram", "daemon", "run"]));
        assert!(err.is_err(), "`--channel telegram daemon run` must error");
    }

    // -----------------------------------------------------------------
    // Phase 20 Task 2 — `daemon status` and `daemon stop` parser tests.
    // -----------------------------------------------------------------

    #[test]
    fn daemon_status_parses_to_daemon_status_mode() {
        let parsed = parse_cli_args_from(&argv(&["daemon", "status"]))
            .expect("`daemon status` must parse");
        assert_eq!(parsed.mode, CliMode::DaemonStatus);
    }

    #[test]
    fn daemon_stop_parses_to_daemon_stop_mode() {
        let parsed = parse_cli_args_from(&argv(&["daemon", "stop"]))
            .expect("`daemon stop` must parse");
        assert_eq!(parsed.mode, CliMode::DaemonStop);
    }

    #[test]
    fn daemon_status_rejects_extra_args() {
        let err = parse_cli_args_from(&argv(&["daemon", "status", "--verbose"]))
            .expect_err("`daemon status --verbose` must error");
        assert!(
            err.contains("unrecognized"),
            "error must mention unrecognized: {err}"
        );
    }

    #[test]
    fn daemon_stop_rejects_extra_args() {
        let err = parse_cli_args_from(&argv(&["daemon", "stop", "--force"]))
            .expect_err("`daemon stop --force` must error");
        assert!(
            err.contains("unrecognized"),
            "error must mention unrecognized: {err}"
        );
    }

    #[test]
    fn daemon_unknown_subcommand_is_an_error() {
        let err = parse_cli_args_from(&argv(&["daemon", "restart"]))
            .expect_err("`daemon restart` must error");
        assert!(
            err.contains("daemon run") && err.contains("daemon status") && err.contains("daemon stop"),
            "error must list all subcommands: {err}"
        );
    }

    // -----------------------------------------------------------------
    // Phase 20 Task 4 — `--no-daemon` flag parser tests.
    // -----------------------------------------------------------------

    #[test]
    fn no_daemon_flag_sets_no_daemon_true() {
        let parsed = parse_cli_args_from(&argv(&["--no-daemon"]))
            .expect("`--no-daemon` must parse");
        assert!(parsed.no_daemon, "no_daemon must be true");
        assert_eq!(parsed.mode, CliMode::Session);
    }

    #[test]
    fn no_daemon_combines_with_channel_and_role() {
        let parsed = parse_cli_args_from(&argv(&[
            "--no-daemon", "--channel", "telegram", "--role", "coder",
        ]))
        .expect("orthogonal flags must compose");
        assert!(parsed.no_daemon);
        assert_eq!(parsed.channel, ChannelKind::Telegram);
        assert_eq!(parsed.role.as_deref(), Some("coder"));
    }

    #[test]
    fn no_daemon_with_verify_only_is_an_error() {
        let err = parse_cli_args_from(&argv(&["--no-daemon", "--verify-only"]))
            .expect_err("`--no-daemon --verify-only` must error");
        assert!(err.contains("--no-daemon"), "error must mention flag: {err}");
    }

    #[test]
    fn no_daemon_with_print_role_is_an_error() {
        let err = parse_cli_args_from(&argv(&["--no-daemon", "--print-role", "coder"]))
            .expect_err("`--no-daemon --print-role` must error");
        assert!(err.contains("--no-daemon"), "error must mention flag: {err}");
    }

    #[test]
    fn default_args_have_no_daemon_false() {
        let parsed = parse_cli_args_from(&argv(&[]))
            .expect("empty argv must parse");
        assert!(!parsed.no_daemon, "no_daemon must default to false");
    }

}

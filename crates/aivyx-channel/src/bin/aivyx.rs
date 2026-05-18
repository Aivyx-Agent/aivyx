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

#[path = "aivyx_modules/identity.rs"]
mod identity;
#[path = "aivyx_modules/init.rs"]
mod init;
#[path = "aivyx_modules/init_templates.rs"]
mod init_templates;
#[path = "aivyx_modules/learning.rs"]
mod learning;
#[path = "aivyx_modules/mcp_server.rs"]
mod mcp_server;
#[path = "aivyx_modules/memory.rs"]
mod memory;
#[path = "aivyx_modules/notify.rs"]
mod notify;
#[path = "aivyx_modules/persona.rs"]
mod persona;
#[path = "aivyx_modules/profile.rs"]
mod profile;

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
use aivyx_channel::daemon_server::{run_daemon, ChannelFactory, DaemonConfig};
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
    WebFetchTool, WebFetchToolConfig, WebPostTool, WebPostToolConfig,
};
use aivyx_crypto::Argon2Params;
use aivyx_memory::{
    Memory, MemoryForgetTool, MemoryReadTool, MemorySearchTool, MemoryWriteTool, RedbMemory,
};
use aivyx_config::ProviderKind;
use aivyx_llm::anthropic::{AnthropicConfig, AnthropicProvider};
use aivyx_llm::openai::{OpenAiConfig, OpenAiProvider, DEFAULT_OLLAMA_BASE_URL};
use aivyx_llm::LlmProvider;
use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
use aivyx_channel::mission_tool::{MissionCreateTool, MissionListTool, MissionStatusTool};
use aivyx_channel::schedule_tool::{
    ScheduleCreateTool, ScheduleDeleteTool, ScheduleListTool, ScheduleUpdateTool,
};
use aivyx_channel::webhook_tool::{
    WebhookCreateTool, WebhookDeleteTool, WebhookListTool,
};
use aivyx_channel::file_watch_tool::{
    FileWatchCreateTool, FileWatchDeleteTool, FileWatchListTool,
};
use aivyx_channel::reflection_tool::{ReflectionApplyTool, ReflectionProposeTool};
use aivyx_channel::role_update_tool::RoleUpdateTool;
use aivyx_channel::turn_history_tool::TurnHistoryTool;
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
) -> Result<Arc<WebFetchTool>, String> {
    let tool = WebFetchToolConfig::new()
        .build()
        .map_err(|e| format!("failed to build web.fetch tool: {e}"))?;
    Ok(Arc::new(tool))
}

/// Build `web.post` for the given channel kind.
///
/// Unlike `web.fetch`, `web.post` is **Trusted-only** — it requires
/// `net.post` which lives in `CEILING_TRUSTED` but not
/// `CEILING_SEMITRUSTED`. The helper still accepts `ChannelKind`
/// for consistency with the other `build_*_for_channel` helpers
/// and to give a future tier-gate hook point if needed.
fn build_web_post_for_channel(
    _channel_kind: ChannelKind,
) -> Result<Arc<WebPostTool>, String> {
    let tool = WebPostToolConfig::new()
        .build()
        .map_err(|e| format!("failed to build web.post tool: {e}"))?;
    Ok(Arc::new(tool))
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
        mcp_servers: cli_mcp_servers,
        mcp_sse_servers: cli_mcp_sse_servers,
        provider: cli_provider,
        web_ui_port: cli_web_ui_port,
    } = parse_cli_args()?;

    // ---- Phase 61: --version short-circuit -----------------------------
    // Prints `aivyx <CARGO_PKG_VERSION>` to stdout and exits 0. Runs
    // before every other dispatch path so the probe never touches the
    // config loader, the storage layer, or the daemon socket — the
    // installer smoke test must succeed on a host with no config and
    // no running daemon.
    if mode == CliMode::Version {
        println!("aivyx {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

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

    // ---- Phase 44: interactive init wizard ----------------------------
    // Like daemon management, init needs only a small runtime (for async
    // Ollama detection) and no config/store/API key.
    if let CliMode::Init(init_mode) = &mode {
        match init_mode {
            InitMode::ListTemplates => {
                // No tokio runtime needed — pure filesystem +
                // string operations.
                let templates = init_templates::list_templates();
                print!("{}", init_templates::render_template_list(&templates));
                return Ok(());
            }
            InitMode::Interactive => {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
                return rt.block_on(init::run_init_wizard(None));
            }
            InitMode::InteractiveFromTemplate { template_name } => {
                let template = init_templates::load_template(template_name)?;
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
                return rt.block_on(init::run_init_wizard(Some(&template)));
            }
        }
    }

    // ---- Phase 46: bundled MCP server -----------------------------------
    // Like init, the MCP server needs only a minimal runtime and no
    // config/store/API key — it reads from stdin and writes to stdout.
    if let CliMode::McpServer(ref name) = mode {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
        return rt.block_on(mcp_server::run_mcp_server(name));
    }

    // ---- Phase 58: profile inspection / edit (PRODUCT.md P13) ----------
    // The profile subcommands are synchronous file operations — no
    // tokio runtime, no daemon dispatch, no API key required. `show`
    // reads `aivyx.toml` and prints the resolved Profile to stdout
    // (Q3(a)); `edit` opens `$EDITOR` against the `[profile]` section
    // (Q2(a), wired in Task 3).
    if let CliMode::Profile(sub) = mode {
        return match sub {
            ProfileSubcommand::Show => profile::run_profile_show(),
            ProfileSubcommand::Edit => profile::run_profile_edit(),
        };
    }

    // ---- Phase 60: persona inspection / revert (PRODUCT.md P14) --------
    // All three persona subcommands talk to the running daemon over
    // IPC (the chain lives in encrypted storage; routing through the
    // daemon avoids duplicating the master-key unlock path here and
    // keeps the runtime state in sync after revert). A minimal tokio
    // runtime is spun up just for the IPC round-trip.
    if let CliMode::Persona(sub) = mode {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
        return rt.block_on(async move {
            match sub {
                PersonaSubcommand::Show => persona::run_persona_show().await,
                PersonaSubcommand::List => persona::run_persona_list().await,
                PersonaSubcommand::Revert { target_delta_id } => {
                    persona::run_persona_revert(&target_delta_id).await
                }
                PersonaSubcommand::Proposals(sub) => match sub {
                    ProposalsSubcommand::List { status } => {
                        persona::run_persona_proposals_list(&status).await
                    }
                    ProposalsSubcommand::Show { proposal_id } => {
                        persona::run_persona_proposals_show(&proposal_id).await
                    }
                    ProposalsSubcommand::Approve { proposal_id } => {
                        persona::run_persona_proposals_approve(&proposal_id).await
                    }
                    ProposalsSubcommand::Reject {
                        proposal_id,
                        reason,
                    } => {
                        persona::run_persona_proposals_reject(
                            &proposal_id,
                            reason.as_deref(),
                        )
                        .await
                    }
                },
            }
        });
    }

    // ---- Phase 73: notify history subcommand --------------------
    // IPC-backed; the audit chain lives in encrypted storage and
    // is fetched via the daemon's ListNotificationHistory query.
    if let CliMode::Notify(sub) = mode {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
        return rt.block_on(async move {
            match sub {
                NotifySubcommand::History { target, limit } => {
                    notify::run_notify_history(target.as_deref(), limit).await
                }
            }
        });
    }

    // ---- Phase 74: memory subcommand ----------------------------
    // IPC-backed; the substrate lives in encrypted storage and is
    // reached via the daemon's memory queries.
    if let CliMode::Memory(sub) = mode {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
        return rt.block_on(async move {
            match sub {
                MemorySubcommand::List => memory::run_memory_list().await,
                MemorySubcommand::Show { topic, limit } => {
                    memory::run_memory_show(&topic, limit).await
                }
                MemorySubcommand::Search {
                    query,
                    limit,
                    semantic,
                } => {
                    memory::run_memory_search(&query, limit, semantic).await
                }
                MemorySubcommand::Evict { topic, yes } => {
                    memory::run_memory_evict(&topic, yes).await
                }
            }
        });
    }

    // Phase 78 — `aivyx learning`: read-only window into the
    // self-learning loop. IPC-backed; terminal parity with the
    // Web UI Learning pane.
    if let CliMode::Learning { window_secs } = mode {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
        return rt
            .block_on(async move { learning::run_learning(window_secs).await });
    }

    // ---- Phase 64: identity export/import (Persona Phase 3) -----
    // Daemon-IPC-backed for the Persona half; reads aivyx.toml
    // directly for the Profile half. Same minimal-runtime pattern
    // as the persona subcommands above.
    if let CliMode::Identity(sub) = mode {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
        return rt.block_on(async move {
            match sub {
                IdentitySubcommand::Export { path } => {
                    identity::run_identity_export(&path).await
                }
                IdentitySubcommand::Import { path, force } => {
                    identity::run_identity_import(&path, force).await
                }
            }
        });
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

    if let Some(kind) = cli_provider {
        config.provider = aivyx_config::Sourced::new(kind, aivyx_config::FieldSource::Env);
    }

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
    let passphrase_source = select_passphrase_source(config.passphrase.as_ref())?;
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
    // Phase 59 — derive the Persona chain HMAC key from the same
    // master, against `KeyDomain::Persona::as_bytes()`. Same pattern
    // as the audit chain key above; raw bytes live on the stack
    // until handed to `PersistentPersonaLog::open`, which clones
    // them into the chain log and zeroes on drop.
    let persona_chain_key: [u8; 32] = {
        let subkey = master_key
            .derive_subkey(b"persona")
            .map_err(|e| format!("failed to derive persona chain key: {e}"))?;
        let mut out = [0u8; 32];
        out.copy_from_slice(subkey.as_bytes());
        out
    };
    // Phase 70 — distinct HMAC key for the Persona proposal chain.
    // Domain-separated from the persona chain via the HKDF info
    // bytes (`persona-proposals`) so a chain-confusion attack
    // (swapping a Pending row into the persona chain or vice
    // versa) is structurally rejected at MAC verification.
    let persona_proposal_chain_key: [u8; 32] = {
        let subkey = master_key
            .derive_subkey(b"persona-proposals")
            .map_err(|e| {
                format!("failed to derive persona proposal chain key: {e}")
            })?;
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
            persona_chain_key,
            persona_proposal_chain_key,
            channel_kind,
            mode,
            no_daemon,
            cli_mcp_servers,
            cli_mcp_sse_servers,
            cli_web_ui_port,
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
                    return Err(e.to_string());
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
        "  provider          = {} ({})",
        config.provider.value,
        source_label(config.provider.source),
    );
    eprintln!(
        "  anthropic_api_key = {}",
        match &config.anthropic_api_key {
            Some(s) => format!("<redacted> ({})", source_label(s.source)),
            None => "<unset>".to_string(),
        }
    );
    if config.provider.value.is_openai_compatible() {
        if config.provider.value == aivyx_config::ProviderKind::OpenAi {
            eprintln!(
                "  openai_api_key    = {}",
                match &config.openai_api_key {
                    Some(s) => format!("<redacted> ({})", source_label(s.source)),
                    None => "<unset>".to_string(),
                }
            );
        }
        if let Some(base_url) = &config.openai_base_url {
            eprintln!(
                "  base_url          = {:?} ({})",
                base_url.value,
                source_label(base_url.source),
            );
        } else if config.provider.value == aivyx_config::ProviderKind::Ollama {
            eprintln!(
                "  base_url          = {:?} (default)",
                DEFAULT_OLLAMA_BASE_URL,
            );
        }
    }
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
    if let Some(ref ttl) = config.memory_ttl_secs {
        eprintln!(
            "  memory_ttl_secs   = {} ({})",
            ttl.value,
            source_label(ttl.source),
        );
    }
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
    // Phase 57 Task 5 — Profile row. Shows the assistant_name with
    // provenance so the operator can tell at startup whether a
    // `[profile]` section was loaded or the synthesized default is
    // in effect. The trailing detail counts the *additional*
    // operator-declared fields (operator_profile,
    // communication_style, primary_use_cases,
    // behavioral_preferences, behavioral_constraints) so the
    // operator knows whether Profile-injection is shaping every
    // turn's prompt.
    let extra_declared = count_extra_profile_fields(&config.profile);
    eprintln!(
        "  profile           = name={:?} ({}){}",
        config.profile.assistant_name.value,
        source_label(config.profile.assistant_name.source),
        if extra_declared == 0 {
            String::new()
        } else {
            format!(", +{extra_declared} operator-declared field(s)")
        },
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

/// Count the number of operator-declared Profile fields other than
/// `assistant_name` (which already has its own banner cell). Phase
/// 57 Task 5 — feeds the trailing detail of the `profile` banner row
/// so the operator can see at startup how many categories beyond
/// the assistant name are shaping every turn's prompt.
fn count_extra_profile_fields(p: &aivyx_config::Profile) -> usize {
    let mut n = 0;
    if p.operator_profile.is_some() {
        n += 1;
    }
    if p.communication_style.is_some() {
        n += 1;
    }
    if !p.primary_use_cases.is_empty() {
        n += 1;
    }
    if !p.behavioral_preferences.is_empty() {
        n += 1;
    }
    if !p.behavioral_constraints.is_empty() {
        n += 1;
    }
    n
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
    /// `aivyx init`: interactive first-run setup wizard (Phase 44).
    /// Phase 66 added the optional template pre-fill via
    /// `aivyx init --template <name>`. The wizard still walks the
    /// operator through each prompt; the template sets the suggested
    /// defaults.
    Init(InitMode),
    /// `aivyx mcp-server <name>`: bundled MCP server (Phase 46).
    McpServer(String),
    /// `aivyx profile <subcommand>`: Profile inspection / edit
    /// (Phase 58 — PRODUCT.md P13). Q1(a) at sign-off: nested
    /// [`ProfileSubcommand`] enum so future additions (e.g. `Reset`,
    /// `Reload`) stay additive without fragmenting `CliMode`.
    Profile(ProfileSubcommand),
    /// `aivyx persona <subcommand>`: Persona inspection / revert
    /// (Phase 60 — PRODUCT.md P14 closure). Q1(c) at sign-off:
    /// nested enum with `Show`, `List`, and `Revert` variants.
    /// Revert carries its target delta id inline. All three
    /// subcommands talk to a running daemon over IPC.
    Persona(PersonaSubcommand),
    /// `aivyx --version` / `aivyx -V`: print `aivyx <version>` and
    /// exit 0 (Phase 61 Task 2). Standard hygiene for binaries
    /// shipped via package managers and required by cargo-dist's
    /// installer smoke test.
    Version,
    /// `aivyx identity <subcommand>`: Profile + Persona
    /// export/import (Phase 64). Closes the Phase 60
    /// deferral; lets operators move identity between hosts.
    /// Phase 64 ships export only; import lands in Phase 65
    /// per the implementation-time scope adjustment.
    Identity(IdentitySubcommand),
    /// `aivyx notify <subcommand>`: Reach Milestone history /
    /// inspection (Phase 73 — Tier-2 polish). Talks to the
    /// running daemon over IPC; renders the notification
    /// history audit chain as a flat-text table for terminal
    /// operators. Web UI parity in the Notifications pane.
    Notify(NotifySubcommand),
    /// `aivyx memory <subcommand>`: memory inspection /
    /// management (Phase 74 — memory polish). IPC-backed;
    /// terminal parity with the Web UI Memory pane.
    Memory(MemorySubcommand),
    /// `aivyx learning [--window <secs>]`: Phase 78 read-only
    /// view of what the self-learning loop has learned and why.
    /// IPC-backed; terminal parity with the Web UI Learning
    /// pane. `window_secs = None` → the daemon's default
    /// lookback.
    Learning { window_secs: Option<u64> },
}

/// Phase 73 — `aivyx notify` subcommand variants.
#[derive(Debug, PartialEq, Eq, Clone)]
enum NotifySubcommand {
    /// `aivyx notify history [--target NAME] [--limit N]`.
    /// Defaults: no target filter, limit 100.
    History {
        target: Option<String>,
        limit: u32,
    },
}

/// Phase 74 — `aivyx memory` subcommand variants.
#[derive(Debug, PartialEq, Eq, Clone)]
enum MemorySubcommand {
    /// `aivyx memory list` — print every topic.
    List,
    /// `aivyx memory show <topic> [--limit N]` — entries for a
    /// topic, newest first. Default limit 32.
    Show { topic: String, limit: u32 },
    /// `aivyx memory search <query> [--semantic] [--limit N]` —
    /// search across topics + bodies. Default limit 32.
    /// `--semantic` requests embedding-ranked retrieval; the
    /// daemon transparently falls back to keyword (with a
    /// stderr note) when embedding is unavailable.
    Search {
        query: String,
        limit: u32,
        semantic: bool,
    },
    /// `aivyx memory evict <topic> [--yes]` — delete every
    /// entry under a topic. `--yes` skips the confirm prompt.
    Evict { topic: String, yes: bool },
}

/// Phase 66 — `aivyx init` variant discriminator.
#[derive(Debug, PartialEq, Eq, Clone)]
enum InitMode {
    /// Plain `aivyx init` — interactive wizard, no template
    /// pre-fill. Existing Phase 44 behavior.
    Interactive,
    /// `aivyx init --template <name>` — wizard pre-filled from
    /// the named template (Q3(b) at Phase 66 sign-off).
    InteractiveFromTemplate { template_name: String },
    /// `aivyx init --list-templates` or `aivyx init --template`
    /// (no name). Prints available templates and exits per
    /// Q4(c) at sign-off.
    ListTemplates,
}

/// Subcommand discriminator under [`CliMode::Identity`]. Phase 64.
#[derive(Debug, PartialEq, Eq, Clone)]
enum IdentitySubcommand {
    /// `aivyx identity export <path>` — write the full identity
    /// bundle (Profile + Persona chain + effective snapshot) to
    /// the operator-supplied path as pretty-printed JSON with
    /// `0600` permissions. Daemon must be running (the Persona
    /// half is fetched over IPC).
    Export { path: PathBuf },
    /// `aivyx identity import <path> [--force]` — Phase 65.
    /// Replays the exported bundle onto the local chain. The
    /// daemon refuses on a non-empty existing chain unless
    /// `force` is set. Profile half remains a hand-edit per
    /// Q2(a) at Phase 65 sign-off.
    Import { path: PathBuf, force: bool },
}

/// Subcommand discriminator under [`CliMode::Persona`]. Phase 60.
#[derive(Debug, PartialEq, Eq, Clone)]
enum PersonaSubcommand {
    /// `aivyx persona show` — print the effective Persona snapshot.
    Show,
    /// `aivyx persona list` — print every approved delta in chain
    /// order with id, category, op, and approval timestamp.
    List,
    /// `aivyx persona revert <delta_id>` — operator-initiated
    /// revert. Daemon appends a `Revert` op delta and recomputes
    /// the shared runtime state so the next turn reflects the undo.
    Revert { target_delta_id: String },
    /// `aivyx persona proposals <sub>` — Phase 70 review surface
    /// for the reflection auto-loop's pending Persona proposals.
    Proposals(ProposalsSubcommand),
}

/// Phase 70 — operator-facing CLI for the proposal review flow.
#[derive(Debug, PartialEq, Eq, Clone)]
enum ProposalsSubcommand {
    /// `aivyx persona proposals list [--status pending|approved|
    /// rejected|all]`. Defaults to `pending`.
    List { status: String },
    /// `aivyx persona proposals show <id>`.
    Show { proposal_id: String },
    /// `aivyx persona proposals approve <id>`. No edit-on-approve
    /// in the CLI v1 (operator can `aivyx persona proposals show`
    /// to inspect then use the Web UI Proposals pane for editing).
    Approve { proposal_id: String },
    /// `aivyx persona proposals reject <id> [--reason TEXT]`.
    Reject {
        proposal_id: String,
        reason: Option<String>,
    },
}

/// Subcommand discriminator under [`CliMode::Profile`]. Phase 58
/// — PRODUCT.md P13.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum ProfileSubcommand {
    /// `aivyx profile show` — print the current Profile to stdout
    /// in a labeled human-readable form. Reads `aivyx.toml` from
    /// disk per Q3(a) at sign-off.
    Show,
    /// `aivyx profile edit` — surgical `[profile]` section edit in
    /// `$EDITOR` per Q2(a) at sign-off (wired in Task 3).
    Edit,
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
    mcp_servers: Vec<CliMcpServer>,
    mcp_sse_servers: Vec<CliMcpSse>,
    provider: Option<ProviderKind>,
    /// Web UI port override from `--web-ui` or `--web-ui-port <N>`.
    /// `Some(port)` enables the web UI in daemon mode. Phase 39.
    web_ui_port: Option<u16>,
}

#[derive(Debug)]
struct CliMcpServer {
    name: String,
    command: String,
    args: Vec<String>,
}

#[derive(Debug)]
struct CliMcpSse {
    name: String,
    url: String,
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
    // Phase 61 Task 2 — `--version` / `-V` short-circuit. Matches
    // before every subcommand and flag so the version probe is
    // stable regardless of future surface additions.
    if !args.is_empty() && (args[0] == "--version" || args[0] == "-V") {
        if args.len() > 1 {
            return Err(format!(
                "`{}` does not accept additional arguments. Got: `{}`",
                args[0],
                args[1..].join(" ")
            ));
        }
        return Ok(CliArgs {
            mode: CliMode::Version,
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
            mcp_servers: Vec::new(),
            mcp_sse_servers: Vec::new(),
            provider: None,
            web_ui_port: None,
        });
    }

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
        // Parse optional flags after `daemon run`.
        let mut daemon_web_ui_port: Option<u16> = None;
        let mut di = 2;
        while di < args.len() {
            match args[di].as_str() {
                "--web-ui" if mode == CliMode::DaemonRun => {
                    daemon_web_ui_port = Some(aivyx_channel::web_ui::DEFAULT_WEB_UI_PORT);
                    di += 1;
                }
                "--web-ui-port" if mode == CliMode::DaemonRun => {
                    let value = args.get(di + 1).ok_or_else(|| {
                        "`--web-ui-port` requires a port number".to_string()
                    })?;
                    let port: u16 = value.parse().map_err(|_| {
                        format!("`--web-ui-port` value `{value}` is not a valid port number")
                    })?;
                    daemon_web_ui_port = Some(port);
                    di += 2;
                }
                other => {
                    return Err(format!(
                        "unrecognized argument after `{subcmd}`: `{other}`. \
                         `{subcmd}` supports: --web-ui, --web-ui-port <N>"
                    ));
                }
            }
        }
        return Ok(CliArgs {
            mode,
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
            mcp_servers: Vec::new(),
            mcp_sse_servers: Vec::new(),
            provider: None,
            web_ui_port: daemon_web_ui_port,
        });
    }

    // Check for `identity <subcommand>` — Phase 64 (Profile +
    // Persona export). `import` lands in Phase 65 — the parser
    // here recognizes only `export` today; an unknown subcommand
    // returns a descriptive error that names what's supported.
    if !args.is_empty() && args[0] == "identity" {
        let sub = args.get(1).ok_or_else(|| {
            "`aivyx identity` requires a subcommand. Supported: export <path>"
                .to_string()
        })?;
        let subcommand = match sub.as_str() {
            "export" => {
                let path = args.get(2).ok_or_else(|| {
                    "`aivyx identity export` requires a path. Usage: \
                     `aivyx identity export <path>`"
                        .to_string()
                })?;
                if args.len() > 3 {
                    return Err(format!(
                        "`aivyx identity export` accepts exactly one path argument. \
                         Got extra args: `{}`",
                        args[3..].join(" ")
                    ));
                }
                IdentitySubcommand::Export {
                    path: PathBuf::from(path),
                }
            }
            "import" => {
                // Phase 65 — replaces the Phase 64 deferral
                // message. `aivyx identity import <path>
                // [--force]`. The --force flag may appear in
                // any trailing position; reject extras.
                let path = args.get(2).ok_or_else(|| {
                    "`aivyx identity import` requires a path. Usage: \
                     `aivyx identity import <path> [--force]`"
                        .to_string()
                })?;
                let mut force = false;
                for extra in args.iter().skip(3) {
                    if extra == "--force" {
                        if force {
                            return Err(
                                "`--force` specified more than once".into(),
                            );
                        }
                        force = true;
                    } else {
                        return Err(format!(
                            "`aivyx identity import` accepts a path and \
                             optional --force. Got unexpected arg: `{extra}`",
                        ));
                    }
                }
                IdentitySubcommand::Import {
                    path: PathBuf::from(path),
                    force,
                }
            }
            other => {
                return Err(format!(
                    "unrecognized identity subcommand: `{other}`. \
                     Supported: identity export <path>"
                ));
            }
        };
        return Ok(CliArgs {
            mode: CliMode::Identity(subcommand),
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
            mcp_servers: Vec::new(),
            mcp_sse_servers: Vec::new(),
            provider: None,
            web_ui_port: None,
        });
    }

    // Phase 73 — `aivyx notify <subcommand>` CLI surface.
    // Today only `history` is implemented; future Tier-2+
    // subcommands (e.g. `notify test <target>`) plug into the
    // same dispatcher.
    if !args.is_empty() && args[0] == "notify" {
        let sub = args.get(1).ok_or_else(|| {
            "`aivyx notify` requires a subcommand. Supported: history".to_string()
        })?;
        match sub.as_str() {
            "history" => {
                let mut target: Option<String> = None;
                let mut limit: u32 = 100;
                let mut idx = 2;
                while idx < args.len() {
                    match args[idx].as_str() {
                        "--target" => {
                            let value = args.get(idx + 1).ok_or_else(|| {
                                "`aivyx notify history --target` requires a name"
                                    .to_string()
                            })?;
                            target = Some(value.clone());
                            idx += 2;
                        }
                        "--limit" => {
                            let value = args.get(idx + 1).ok_or_else(|| {
                                "`aivyx notify history --limit` requires a value"
                                    .to_string()
                            })?;
                            let parsed: u32 = value.parse().map_err(|_| {
                                format!(
                                    "`aivyx notify history --limit` expects \
                                     a positive integer, got `{value}`"
                                )
                            })?;
                            if parsed == 0 {
                                return Err(
                                    "`aivyx notify history --limit` must be ≥ 1"
                                        .to_string(),
                                );
                            }
                            limit = parsed;
                            idx += 2;
                        }
                        other => {
                            return Err(format!(
                                "unrecognized argument to `aivyx notify \
                                 history`: `{other}`"
                            ));
                        }
                    }
                }
                return Ok(CliArgs {
                    mode: CliMode::Notify(NotifySubcommand::History {
                        target,
                        limit,
                    }),
                    channel: ChannelKind::Local,
                    role: None,
                    no_daemon: false,
                    mcp_servers: vec![],
                    mcp_sse_servers: vec![],
                    provider: None,
                    web_ui_port: None,
                });
            }
            other => {
                return Err(format!(
                    "unrecognized `aivyx notify` subcommand: `{other}`. \
                     Supported: history"
                ));
            }
        }
    }

    // Phase 74 — `aivyx memory <subcommand>` CLI surface.
    if !args.is_empty() && args[0] == "memory" {
        let sub = args.get(1).ok_or_else(|| {
            "`aivyx memory` requires a subcommand. Supported: \
             list, show, search, evict"
                .to_string()
        })?;
        // Helper: parse a trailing `--limit N` flag from the
        // arg slice starting at `start`, returning (limit,
        // positional-consumed). Default 32.
        let parse_limit_from = |args: &[String],
                                start: usize|
         -> Result<u32, String> {
            let mut idx = start;
            let mut limit = 32u32;
            while idx < args.len() {
                match args[idx].as_str() {
                    "--limit" => {
                        let v = args.get(idx + 1).ok_or_else(|| {
                            "`--limit` requires a value".to_string()
                        })?;
                        let parsed: u32 = v.parse().map_err(|_| {
                            format!(
                                "`--limit` expects a positive integer, \
                                 got `{v}`"
                            )
                        })?;
                        if parsed == 0 {
                            return Err(
                                "`--limit` must be ≥ 1".to_string()
                            );
                        }
                        limit = parsed;
                        idx += 2;
                    }
                    other => {
                        return Err(format!(
                            "unrecognized argument: `{other}`"
                        ));
                    }
                }
            }
            Ok(limit)
        };
        let mem_sub = match sub.as_str() {
            "list" => {
                if args.len() > 2 {
                    return Err(
                        "`aivyx memory list` takes no arguments".into()
                    );
                }
                MemorySubcommand::List
            }
            "show" => {
                let topic = args.get(2).ok_or_else(|| {
                    "`aivyx memory show` requires a topic".to_string()
                })?;
                let limit = parse_limit_from(args, 3)?;
                MemorySubcommand::Show {
                    topic: topic.clone(),
                    limit,
                }
            }
            "search" => {
                let query = args.get(2).ok_or_else(|| {
                    "`aivyx memory search` requires a query".to_string()
                })?;
                // Hand-parsed (not `parse_limit_from`) because
                // search additionally accepts the `--semantic`
                // flag, which the shared limit parser rejects.
                let mut limit = 32u32;
                let mut semantic = false;
                let mut idx = 3;
                while idx < args.len() {
                    match args[idx].as_str() {
                        "--semantic" => {
                            semantic = true;
                            idx += 1;
                        }
                        "--limit" => {
                            let v = args.get(idx + 1).ok_or_else(|| {
                                "`--limit` requires a value".to_string()
                            })?;
                            let parsed: u32 = v.parse().map_err(|_| {
                                format!(
                                    "`--limit` expects a positive \
                                     integer, got `{v}`"
                                )
                            })?;
                            if parsed == 0 {
                                return Err(
                                    "`--limit` must be ≥ 1".to_string()
                                );
                            }
                            limit = parsed;
                            idx += 2;
                        }
                        other => {
                            return Err(format!(
                                "unrecognized argument to `aivyx \
                                 memory search`: `{other}`"
                            ));
                        }
                    }
                }
                MemorySubcommand::Search {
                    query: query.clone(),
                    limit,
                    semantic,
                }
            }
            "evict" => {
                let topic = args.get(2).ok_or_else(|| {
                    "`aivyx memory evict` requires a topic".to_string()
                })?;
                let mut yes = false;
                for a in &args[3..] {
                    match a.as_str() {
                        "--yes" => yes = true,
                        other => {
                            return Err(format!(
                                "unrecognized argument to `aivyx memory \
                                 evict`: `{other}`"
                            ));
                        }
                    }
                }
                MemorySubcommand::Evict {
                    topic: topic.clone(),
                    yes,
                }
            }
            other => {
                return Err(format!(
                    "unrecognized `aivyx memory` subcommand: `{other}`. \
                     Supported: list, show, search, evict"
                ));
            }
        };
        return Ok(CliArgs {
            mode: CliMode::Memory(mem_sub),
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
            mcp_servers: vec![],
            mcp_sse_servers: vec![],
            provider: None,
            web_ui_port: None,
        });
    }

    // Phase 78 — `aivyx learning [--window <secs>]`.
    if !args.is_empty() && args[0] == "learning" {
        let mut window_secs: Option<u64> = None;
        let mut idx = 1;
        while idx < args.len() {
            match args[idx].as_str() {
                "--window" => {
                    let v = args.get(idx + 1).ok_or_else(|| {
                        "`--window` requires a value (seconds)"
                            .to_string()
                    })?;
                    let parsed: u64 = v.parse().map_err(|_| {
                        format!(
                            "`--window` expects a positive integer \
                             (seconds), got `{v}`"
                        )
                    })?;
                    if parsed == 0 {
                        return Err(
                            "`--window` must be >= 1".to_string()
                        );
                    }
                    window_secs = Some(parsed);
                    idx += 2;
                }
                other => {
                    return Err(format!(
                        "unrecognized argument to `aivyx learning`: \
                         `{other}`"
                    ));
                }
            }
        }
        return Ok(CliArgs {
            mode: CliMode::Learning { window_secs },
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
            mcp_servers: vec![],
            mcp_sse_servers: vec![],
            provider: None,
            web_ui_port: None,
        });
    }

    // Check for `init` subcommand — interactive first-run wizard
    // (Phase 44, extended at Phase 66 with starter templates).
    //
    // Accepted forms:
    //   `aivyx init`                          → interactive, no template
    //   `aivyx init --template <name>`        → interactive, pre-filled
    //   `aivyx init --template` (no name)     → list templates + exit
    //   `aivyx init --list-templates`         → list templates + exit
    if !args.is_empty() && args[0] == "init" {
        let mut init_mode = InitMode::Interactive;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--list-templates" => {
                    // Setting list-mode twice is idempotent;
                    // `--template <name>` followed by
                    // `--list-templates` collapses to list-mode
                    // (the operator clearly wants to discover).
                    init_mode = InitMode::ListTemplates;
                    i += 1;
                }
                "--template" => {
                    // `--template <name>` or `--template` (no name).
                    let next = args.get(i + 1);
                    match next {
                        Some(name) if !name.starts_with("--") => {
                            init_mode = InitMode::InteractiveFromTemplate {
                                template_name: name.clone(),
                            };
                            i += 2;
                        }
                        _ => {
                            // No name (or another flag follows) — list mode.
                            init_mode = InitMode::ListTemplates;
                            i += 1;
                        }
                    }
                }
                other => {
                    return Err(format!(
                        "`aivyx init`: unrecognized argument `{other}`. \
                         Supported: `--template <name>`, `--list-templates`",
                    ));
                }
            }
        }
        return Ok(CliArgs {
            mode: CliMode::Init(init_mode),
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
            mcp_servers: Vec::new(),
            mcp_sse_servers: Vec::new(),
            provider: None,
            web_ui_port: None,
        });
    }

    // Check for `persona <subcommand>` — Phase 60 (PRODUCT.md P14).
    // Q1(c) at sign-off: nested enum with show/list/revert variants.
    // Phase 70 adds `proposals` for the self-learning loop's
    // operator review surface.
    if !args.is_empty() && args[0] == "persona" {
        let sub = args.get(1).ok_or_else(|| {
            "`aivyx persona` requires a subcommand. Supported: \
             show, list, revert, proposals"
                .to_string()
        })?;
        let subcommand = match sub.as_str() {
            "show" => {
                if args.len() > 2 {
                    return Err(format!(
                        "`aivyx persona show` does not accept additional arguments. \
                         Got: `{}`",
                        args[2..].join(" ")
                    ));
                }
                PersonaSubcommand::Show
            }
            "list" => {
                if args.len() > 2 {
                    return Err(format!(
                        "`aivyx persona list` does not accept additional arguments. \
                         Got: `{}`",
                        args[2..].join(" ")
                    ));
                }
                PersonaSubcommand::List
            }
            "revert" => {
                let target = args.get(2).ok_or_else(|| {
                    "`aivyx persona revert` requires a delta id. Usage: \
                     `aivyx persona revert <delta_id>`"
                        .to_string()
                })?;
                if args.len() > 3 {
                    return Err(format!(
                        "`aivyx persona revert` accepts exactly one delta id. \
                         Got: `{}`",
                        args[3..].join(" ")
                    ));
                }
                PersonaSubcommand::Revert {
                    target_delta_id: target.clone(),
                }
            }
            "proposals" => {
                // Sub-subcommand: list / show / approve / reject.
                let sub2 = args.get(2).ok_or_else(|| {
                    "`aivyx persona proposals` requires a subcommand. \
                     Supported: list, show, approve, reject"
                        .to_string()
                })?;
                let proposals_sub = match sub2.as_str() {
                    "list" => {
                        let mut status = "pending".to_string();
                        let mut idx = 3;
                        while idx < args.len() {
                            match args[idx].as_str() {
                                "--status" => {
                                    idx += 1;
                                    let value = args.get(idx).ok_or_else(|| {
                                        "`aivyx persona proposals list \
                                         --status` requires a value"
                                            .to_string()
                                    })?;
                                    let normalized = value.to_ascii_lowercase();
                                    match normalized.as_str() {
                                        "pending" | "approved" | "rejected"
                                        | "superseded" | "all" => {
                                            status = normalized;
                                        }
                                        other => {
                                            return Err(format!(
                                                "unknown --status value `{other}`. \
                                                 Supported: pending, approved, \
                                                 rejected, superseded, all"
                                            ));
                                        }
                                    }
                                }
                                other => {
                                    return Err(format!(
                                        "unrecognized argument to \
                                         `aivyx persona proposals list`: `{other}`"
                                    ));
                                }
                            }
                            idx += 1;
                        }
                        ProposalsSubcommand::List { status }
                    }
                    "show" => {
                        let pid = args.get(3).ok_or_else(|| {
                            "`aivyx persona proposals show` requires a \
                             proposal id"
                                .to_string()
                        })?;
                        if args.len() > 4 {
                            return Err(format!(
                                "`aivyx persona proposals show` accepts exactly \
                                 one proposal id. Got: `{}`",
                                args[4..].join(" ")
                            ));
                        }
                        ProposalsSubcommand::Show {
                            proposal_id: pid.clone(),
                        }
                    }
                    "approve" => {
                        let pid = args.get(3).ok_or_else(|| {
                            "`aivyx persona proposals approve` requires a \
                             proposal id"
                                .to_string()
                        })?;
                        if args.len() > 4 {
                            return Err(format!(
                                "`aivyx persona proposals approve` accepts \
                                 exactly one proposal id. Got: `{}`",
                                args[4..].join(" ")
                            ));
                        }
                        ProposalsSubcommand::Approve {
                            proposal_id: pid.clone(),
                        }
                    }
                    "reject" => {
                        let pid = args.get(3).ok_or_else(|| {
                            "`aivyx persona proposals reject` requires a \
                             proposal id"
                                .to_string()
                        })?;
                        let mut reason: Option<String> = None;
                        let mut idx = 4;
                        while idx < args.len() {
                            match args[idx].as_str() {
                                "--reason" => {
                                    idx += 1;
                                    let value =
                                        args.get(idx).ok_or_else(|| {
                                            "`aivyx persona proposals reject \
                                             --reason` requires a value"
                                                .to_string()
                                        })?;
                                    reason = Some(value.clone());
                                }
                                other => {
                                    return Err(format!(
                                        "unrecognized argument to \
                                         `aivyx persona proposals reject`: `{other}`"
                                    ));
                                }
                            }
                            idx += 1;
                        }
                        ProposalsSubcommand::Reject {
                            proposal_id: pid.clone(),
                            reason,
                        }
                    }
                    other => {
                        return Err(format!(
                            "unrecognized `aivyx persona proposals` \
                             subcommand: `{other}`. \
                             Supported: list, show, approve, reject"
                        ));
                    }
                };
                PersonaSubcommand::Proposals(proposals_sub)
            }
            other => {
                return Err(format!(
                    "unrecognized persona subcommand: `{other}`. \
                     Supported: persona show, persona list, \
                     persona revert <id>, persona proposals <sub>"
                ));
            }
        };
        return Ok(CliArgs {
            mode: CliMode::Persona(subcommand),
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
            mcp_servers: Vec::new(),
            mcp_sse_servers: Vec::new(),
            provider: None,
            web_ui_port: None,
        });
    }

    // Check for `profile <subcommand>` — Phase 58 (PRODUCT.md P13).
    // Q1(a) at sign-off: nested `ProfileSubcommand` enum with Show
    // and Edit variants today; future variants land additively.
    if !args.is_empty() && args[0] == "profile" {
        let sub = args.get(1).ok_or_else(|| {
            "`aivyx profile` requires a subcommand. Supported: show, edit".to_string()
        })?;
        let subcommand = match sub.as_str() {
            "show" => ProfileSubcommand::Show,
            "edit" => ProfileSubcommand::Edit,
            other => {
                return Err(format!(
                    "unrecognized profile subcommand: `{other}`. \
                     Supported: profile show, profile edit"
                ));
            }
        };
        if args.len() > 2 {
            return Err(format!(
                "`aivyx profile {sub}` does not accept additional arguments. \
                 Got: `{}`",
                args[2..].join(" ")
            ));
        }
        return Ok(CliArgs {
            mode: CliMode::Profile(subcommand),
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
            mcp_servers: Vec::new(),
            mcp_sse_servers: Vec::new(),
            provider: None,
            web_ui_port: None,
        });
    }

    // Check for `mcp-server <name>` subcommand — bundled MCP server (Phase 46).
    if !args.is_empty() && args[0] == "mcp-server" {
        let name = args.get(1).ok_or_else(|| {
            "`aivyx mcp-server` requires a server name. Supported: web-search".to_string()
        })?;
        if args.len() > 2 {
            return Err(format!(
                "`aivyx mcp-server {name}` does not accept additional arguments. \
                 Got: `{}`",
                args[2..].join(" ")
            ));
        }
        // Validate server name eagerly at parse time.
        match name.as_str() {
            "web-search" => {}
            other => {
                return Err(format!(
                    "unknown MCP server name: `{other}`. Supported: web-search"
                ));
            }
        }
        return Ok(CliArgs {
            mode: CliMode::McpServer(name.clone()),
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
            mcp_servers: Vec::new(),
            mcp_sse_servers: Vec::new(),
            provider: None,
            web_ui_port: None,
        });
    }

    let mut verify_only = false;
    let mut channel = ChannelKind::Local;
    let mut role: Option<String> = None;
    let mut print_role: Option<String> = None;
    let mut no_daemon = false;
    let mut mcp_servers: Vec<CliMcpServer> = Vec::new();
    let mut mcp_sse_servers: Vec<CliMcpSse> = Vec::new();
    let mut cli_provider: Option<ProviderKind> = None;

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
            "--provider" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| {
                        "`--provider` requires a value: `anthropic`, `openai`, or `ollama`"
                            .to_string()
                    })?;
                cli_provider = Some(match value.as_str() {
                    "anthropic" => ProviderKind::Anthropic,
                    "openai" => ProviderKind::OpenAi,
                    "ollama" => ProviderKind::Ollama,
                    other => {
                        return Err(format!(
                            "unrecognized provider `{other}`. \
                             Supported: anthropic, openai, ollama"
                        ));
                    }
                });
                i += 2;
            }
            "--mcp-server" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| {
                        "`--mcp-server` requires a value in the format \
                         `name:command` or `name:command:arg1,arg2,...`"
                            .to_string()
                    })?;
                let parts: Vec<&str> = value.splitn(3, ':').collect();
                if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
                    return Err(format!(
                        "`--mcp-server` value `{value}` is malformed. \
                         Expected `name:command` or `name:command:arg1,arg2,...`"
                    ));
                }
                let cli_args = if parts.len() == 3 && !parts[2].is_empty() {
                    parts[2].split(',').map(|s| s.to_string()).collect()
                } else {
                    Vec::new()
                };
                mcp_servers.push(CliMcpServer {
                    name: parts[0].to_string(),
                    command: parts[1].to_string(),
                    args: cli_args,
                });
                i += 2;
            }
            "--mcp-sse" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| {
                        "`--mcp-sse` requires a value in the format \
                         `name:url`"
                            .to_string()
                    })?;
                let parts: Vec<&str> = value.splitn(2, ':').collect();
                if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
                    return Err(format!(
                        "`--mcp-sse` value `{value}` is malformed. \
                         Expected `name:url` (e.g. `myserver:http://host:8080/sse`)"
                    ));
                }
                mcp_sse_servers.push(CliMcpSse {
                    name: parts[0].to_string(),
                    url: parts[1].to_string(),
                });
                i += 2;
            }
            "daemon" => {
                return Err(
                    "unrecognized subcommand. Did you mean `daemon run`, `daemon status`, or `daemon stop`?".to_string()
                );
            }
            other => {
                return Err(format!(
                    "unrecognized argument: `{other}`. \
                     Supported: --verify-only, --channel <local|telegram>, --role <name>, --print-role <name>, --no-daemon, --provider <anthropic|openai|ollama>, --mcp-server <name:command[:args]>, daemon run|status|stop"
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
        mcp_servers,
        mcp_sse_servers,
        provider: cli_provider,
        web_ui_port: None,
    })
}

/// Decide which `PassphraseSource` to hand to `derive_master_key`.
///
/// Phase 9 Task 3 introduced the "config-aware" shape — the helper
/// receives a hint from `aivyx-config` about whether *some* source
/// (env or TOML) supplied a passphrase. Phase 51 Task 4 fixes a
/// quiet bug from that change: the helper always returned
/// `PassphraseSource::Env` when the config had a value, even if
/// the value came from TOML. That worked when `AIVYX_PASSPHRASE`
/// was set; it errored at startup with "env var not set" when
/// only TOML was set, contradicting the config-loader contract.
///
/// The fix: take the actual `SourcedSecret` (or `None`), and when
/// it's present pick the right source kind based on `FieldSource`.
/// Env → `PassphraseSource::Env` (re-read to honor any env-var
/// rotation between config-load and derive); TOML or
/// EncryptedStore → `PassphraseSource::FromConfig(secret)` so the
/// value is used directly.
///
/// Policy:
/// 1. `passphrase = Some(env)` → `Env` (re-read AIVYX_PASSPHRASE).
/// 2. `passphrase = Some(non-env)` → `FromConfig(secret.clone())`.
/// 3. `passphrase = None` and stdin is a tty → `InteractivePrompt`.
/// 4. Otherwise → `Err` with a clear operator-facing message.
fn select_passphrase_source(
    passphrase: Option<&aivyx_config::SourcedSecret>,
) -> Result<PassphraseSource, String> {
    if let Some(secret) = passphrase {
        return match secret.source {
            aivyx_config::FieldSource::Env => Ok(PassphraseSource::Env {
                var_name: DEFAULT_ENV_VAR.to_string(),
            }),
            _ => Ok(PassphraseSource::FromConfig(secret.value.clone())),
        };
    }
    if io::stdin().is_terminal() {
        Ok(PassphraseSource::InteractivePrompt)
    } else {
        Err(format!(
            "no passphrase available: `{DEFAULT_ENV_VAR}` is not set, \
             no `[aivyx] passphrase` in the TOML config, and stdin is \
             not a terminal. Export the env var, set the TOML field, \
             or run aivyx from an interactive shell."
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
#[allow(clippy::too_many_arguments)] // Startup wiring; bundling deferred to SDK phase
async fn run_async(
    config: AivyxConfig,
    storage: Arc<dyn Storage>,
    audit_chain_key: [u8; 32],
    persona_chain_key: [u8; 32],
    persona_proposal_chain_key: [u8; 32],
    channel_kind: ChannelKind,
    mode: CliMode,
    no_daemon: bool,
    cli_mcp_servers: Vec<CliMcpServer>,
    cli_mcp_sse_servers: Vec<CliMcpSse>,
    cli_web_ui_port: Option<u16>,
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
        openai_api_key,
        openai_base_url,
        provider: provider_kind,
        model,
        system_prompt: _legacy_system_prompt,
        fs_root,
        storage_path: _,
        memory_max_per_topic,
        passphrase: _,
        telegram,
        // Phase 68 — shared SMTP config consumed by
        // `build_notify_dispatcher` when any
        // `[[notify_target]] kind = "email"` exists.
        email,
        // Phase 75 — `[embedding]` config. `Some` iff the
        // `[embedding]` section is present; threaded into the
        // write tool's embedding hook and the daemon's
        // lazy-backfill timer below.
        embedding: config_embedding,
        // Phase 80 — `[proactive]` config. Threaded into the
        // daemon's reflection-cron proactive pass below.
        proactive: config_proactive,
        // Phase 81 — `[persona_lifecycle]` config. Threaded
        // into the daemon's reflection-cron lifecycle pass via
        // DaemonConfig below.
        persona_lifecycle: config_persona_lifecycle,
        // Phase 84 — `[recall_cluster]` config. Wired into the
        // recall provider in Task 4 (bound here so the
        // destructure stays exhaustive).
        recall_cluster: _config_recall_cluster,
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
        // Phase 57 — operator-declared identity layer per PRODUCT.md
        // P13. Task 3 wires this through `assemble_session_prompt` so
        // Profile flavors every turn's system prompt alongside (not
        // inside) the role envelope. Cloned once for the role-switch
        // factory capture (`profile_for_factory`) below.
        profile,
        // `warnings` is rendered by the banner in `print_startup_banner`
        // directly from `&config.warnings` before the destructure; by
        // the time we land here the banner has already printed any
        // load-time warnings, so we drop the field on the floor.
        warnings: _,
        mut mcp_servers,
        tool_processes: config_tool_processes,
        schedules: config_schedules,
        webhooks: config_webhooks,
        file_watches: config_file_watches,
        // Phase 62 Task 8 — consumed below at the notify
        // dispatcher / NotifySendTool wiring site.
        notify_targets: config_notify_targets,
        // Phase 70 — P14 self-learning closure. Consumed by the
        // reflection-scheduler subsystem at the daemon startup
        // path below; reflection turns fire on the configured
        // cron and write proposals into the persona_proposals
        // domain.
        reflection_schedules: config_reflection_schedules,
        webhook_port: config_webhook_port,
        web_ui_port: config_web_ui_port,
        memory_ttl_secs,
        // Phase 74 — per-topic-glob retention rules. Threaded
        // into the daemon's memory-GC timer below so the hourly
        // pass respects first-match retention before falling
        // back to the global memory_ttl_secs.
        memory_retention: config_memory_retention,
    } = config;
    for cli in cli_mcp_servers {
        mcp_servers.push(aivyx_config::McpServerConfig {
            name: cli.name,
            transport: aivyx_config::McpTransportKind::Stdio,
            command: Some(cli.command),
            args: cli.args,
            url: None,
            enabled: true,
            bundled: false,
            // CLI-flag MCP servers don't carry a sandbox config —
            // the `--mcp-server` flag is for quick experimentation,
            // not for hardened deployments. Operators who want a
            // sandbox use the TOML config path.
            sandbox: None,
        });
    }
    for cli in cli_mcp_sse_servers {
        mcp_servers.push(aivyx_config::McpServerConfig {
            name: cli.name,
            transport: aivyx_config::McpTransportKind::Sse,
            command: None,
            args: Vec::new(),
            url: Some(cli.url),
            enabled: true,
            bundled: false,
            // SSE has no local child to wrap; sandbox is always None
            // for this transport kind.
            sandbox: None,
        });
    }
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
    // Phase 59 — open the persistent Persona chain (PRODUCT.md P14)
    // and replay it into a SharedEffectivePersona before the
    // system-prompt assembly. The reflection.apply tool gets a
    // handle on both the chain and the shared state further down
    // (the setters need the constructed Arc<ReflectionApplyTool>,
    // which is built later in the startup path).
    let persona_log = match aivyx_channel::persona::PersistentPersonaLog::open(
        storage.domain(KeyDomain::Persona),
        persona_chain_key.to_vec(),
    )
    .await
    {
        Ok(log) => Arc::new(log),
        Err(e) => {
            return Err(format!(
                "failed to open persona chain (KeyDomain::Persona): {e}"
            ));
        }
    };
    // Phase 70 — open the persistent Persona proposal chain
    // (KeyDomain::PersonaProposals). Parallel to the persona log
    // above; this is the operator-pending side of P14's self-
    // learning loop. Scheduled reflection turns append Pending
    // rows here; operators resolve them via the Web UI Proposals
    // pane / `aivyx persona proposals` CLI.
    let persona_proposal_log = match aivyx_channel::persona_proposal::PersistentPersonaProposalLog::open(
        storage.domain(KeyDomain::PersonaProposals),
        persona_proposal_chain_key.to_vec(),
    )
    .await
    {
        Ok(log) => Arc::new(log),
        Err(e) => {
            return Err(format!(
                "failed to open persona proposal chain \
                 (KeyDomain::PersonaProposals): {e}"
            ));
        }
    };
    let shared_persona = aivyx_channel::persona::shared_effective_persona(
        aivyx_channel::persona::compute_effective_persona(&persona_log.entries()),
    );
    // Phase 57 Task 3 — assemble the final system prompt by layering
    // Profile (operator-declared identity per PRODUCT.md P13), Persona
    // (reflection-written identity per PRODUCT.md P14, Phase 59 Task 6),
    // and the active role's `system_prompt`. When Profile is at its
    // synthesized default AND the Persona chain is empty, the helper
    // returns the role's `system_prompt` unchanged — zero behavior
    // change for pre-Phase-57 configs that haven't started accumulating
    // Persona deltas yet.
    let system_prompt = {
        let persona_snapshot = shared_persona.read().expect("persona lock not poisoned at startup");
        aivyx_channel::assemble_session_prompt(
            &profile,
            Some(&*persona_snapshot),
            &active_role_name,
            &role.system_prompt.value,
        )
    };
    let tool_allowlist: Option<std::collections::BTreeSet<String>> =
        match role.tool_allowlist.value {
            ToolAllowlist::AllowAll => None,
            ToolAllowlist::Only(list) => Some(list.into_iter().collect()),
        };
    let memory_topic_prefix: Option<String> = role.memory_topic_prefix.value;

    // ---- Provider -----------------------------------------------------
    // Track the Ollama base URL for tool registration (Phase 36).
    let mut ollama_base_url_for_tools: Option<String> = None;
    let provider: Arc<dyn LlmProvider> = match provider_kind.value {
        ProviderKind::Anthropic => {
            let api_key = anthropic_api_key
                .expect("anthropic_api_key validated non-None before run_async")
                .value;
            let p = AnthropicProvider::new(AnthropicConfig::new(api_key))
                .map_err(|e| format!("failed to build Anthropic provider: {e}"))?;
            Arc::new(p)
        }
        ProviderKind::OpenAi => {
            let api_key = openai_api_key
                .expect("openai_api_key validated non-None before run_async")
                .value;
            let mut cfg = OpenAiConfig::new(api_key);
            if let Some(base_url) = openai_base_url {
                cfg = cfg.with_base_url(base_url.value);
            }
            let p = OpenAiProvider::new(cfg)
                .map_err(|e| format!("failed to build OpenAI provider: {e}"))?;
            Arc::new(p)
        }
        ProviderKind::Ollama => {
            let mut cfg = match openai_api_key {
                Some(key) => OpenAiConfig::new(key.value),
                None => OpenAiConfig::without_api_key(),
            };
            // Ollama default base URL; explicit config overrides.
            let base_url = openai_base_url
                .map(|s| s.value)
                .unwrap_or_else(|| DEFAULT_OLLAMA_BASE_URL.to_string());
            ollama_base_url_for_tools = Some(base_url.clone());
            cfg = cfg.with_base_url(base_url);
            let p = OpenAiProvider::new(cfg)
                .map_err(|e| format!("failed to build Ollama provider: {e}"))?;

            // Lightweight health check — warn (don't abort) if Ollama
            // is unreachable so the user gets actionable guidance.
            if let Err(msg) = p.health_check().await {
                eprintln!("\n⚠  Ollama health check failed:");
                eprintln!("   {msg}");
                eprintln!();
            }

            Arc::new(p)
        }
    };

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
    let persistent_audit = Arc::new(persistent_audit);
    let audit_log_for_tool: Arc<dyn aivyx_audit::AuditLog + Send + Sync> =
        Arc::clone(&persistent_audit) as _;
    // Phase 47 — daemon needs a concrete `Arc<PersistentAuditLog>` for the
    // `ListAuditEntries` / `VerifyAuditChain` queries (the
    // `entries_range` API lives on the concrete type, not the
    // `AuditWriter` / `AuditLog` traits).
    let persistent_audit_for_query: Arc<PersistentAuditLog> = Arc::clone(&persistent_audit);
    let audit: Arc<dyn AuditHook> = persistent_audit;

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
    // Phase 75 — construct the embedding provider once iff
    // `[embedding]` is configured. Shared two ways: the write
    // tool's synchronous write-time hook, and the daemon's
    // hourly lazy-backfill timer (passed via DaemonConfig).
    let embedding_provider: Option<
        Arc<dyn aivyx_llm::embedding::EmbeddingProvider>,
    > = match config_embedding.as_ref() {
        Some(cfg) => Some(
            aivyx_channel::memory_embedding::build_embedding_provider(cfg)?,
        ),
        None => None,
    };

    // Phase 76 — automatic semantic recall. Built once when both
    // a provider and the `[embedding]` config exist (the config
    // carries the rag_top_k / rag_min_similarity knobs). Shared
    // by-Arc into every planner-factory site below; `None` →
    // no auto-recall attached → pre-Phase-76 behavior.
    // Phase 77 — the recall-feedback log. Built once and shared:
    // the recall hook appends to it, and (Task 8) the reflection
    // loop reads it. `None` when auto-recall is off.
    let recall_log: Option<
        Arc<aivyx_channel::recall_log::PersistentRecallLog>,
    > = match (&embedding_provider, config_embedding.as_ref()) {
        (Some(_), Some(_)) => {
            Some(Arc::new(aivyx_channel::recall_log::PersistentRecallLog::new(
                storage.domain(KeyDomain::RecallEvents),
            )))
        }
        _ => None,
    };
    // Phase 82 — the durable helpfulness ledger. Zero-config:
    // built under the same condition as the recall log (the
    // signal it folds only exists when auto-recall is on), no
    // `[…]` block. The reflection recall-feedback pass folds
    // each window into it and prunes on the same cadence.
    let helpfulness_ledger: Option<
        Arc<
            aivyx_channel::helpfulness_ledger::PersistentHelpfulnessLedger,
        >,
    > = recall_log.as_ref().map(|_| {
        Arc::new(
            aivyx_channel::helpfulness_ledger::PersistentHelpfulnessLedger::new(
                storage.domain(KeyDomain::HelpfulnessLedger),
            ),
        )
    });
    // Phase 83 — the durable cross-session co-occurrence
    // ledger. Zero-config, same condition + rationale as the
    // helpfulness ledger (the pair signal only exists when
    // auto-recall is on). Folded + pruned on the same cadence.
    let cooccurrence_ledger: Option<
        Arc<
            aivyx_channel::cooccurrence_ledger::PersistentCooccurrenceLedger,
        >,
    > = recall_log.as_ref().map(|_| {
        Arc::new(
            aivyx_channel::cooccurrence_ledger::PersistentCooccurrenceLedger::new(
                storage.domain(KeyDomain::CooccurrenceLedger),
            ),
        )
    });
    // Phase 80 — proactive dedup log, created when the
    // `[proactive]` section is armed (enabled). The reflection
    // cron pass uses it for cross-cycle dedup + the cap.
    let proactive_log: Option<
        Arc<aivyx_channel::proactive_log::PersistentProactiveLog>,
    > = match &config_proactive {
        Some(p) if p.enabled => Some(Arc::new(
            aivyx_channel::proactive_log::PersistentProactiveLog::new(
                storage.domain(KeyDomain::ProactiveLog),
            ),
        )),
        _ => None,
    };
    let recall_context: Option<
        Arc<dyn aivyx_core::llm_planner::ContextProvider>,
    > = match (&embedding_provider, config_embedding.as_ref()) {
        (Some(provider), Some(cfg)) => {
            let mut sc =
                aivyx_channel::memory_recall::SemanticMemoryContext::new(
                    Arc::clone(&memory),
                    Arc::clone(provider),
                    cfg.rag_top_k,
                    cfg.rag_min_similarity,
                );
            if let Some(log) = &recall_log {
                sc = sc.with_recall_log(Arc::clone(log));
            }
            Some(Arc::new(sc))
        }
        _ => None,
    };
    // Phase 79 — adaptive Persona. Built once when an embedding
    // provider exists; attached by-Arc at every planner-factory
    // site below. `None` → no refiner → the full Persona is
    // injected unchanged (pre-Phase-79 behavior).
    // Phase 79 (Q4a) — shared last-selection stat: the refiner
    // writes it, the daemon's GetLearningInsights handler reads
    // the *same* handle. `None` when adaptive Persona is off.
    let persona_selection_stat = embedding_provider.as_ref().map(|_| {
        aivyx_channel::persona_context::shared_persona_selection_stat()
    });
    // Phase 80 (Q4a) — shared last-proactive-cycle stat: the
    // pass writes it, GetLearningInsights reads the same handle.
    // Created iff proactive is armed (enabled).
    let proactive_stat = match &config_proactive {
        Some(p) if p.enabled => Some(
            aivyx_channel::proactive_detect::shared_proactive_stat(),
        ),
        _ => None,
    };
    // Phase 81 (Q4a) — shared last-lifecycle-cycle stat: the
    // pass writes it, GetLearningInsights reads the same
    // handle. Created iff the lifecycle pass is armed.
    let persona_lifecycle_stat = match &config_persona_lifecycle {
        Some(p) if p.enabled => Some(
            aivyx_channel::persona_lifecycle::shared_persona_lifecycle_stat(),
        ),
        _ => None,
    };
    let persona_refiner: Option<
        Arc<dyn aivyx_core::llm_planner::SystemPromptRefiner>,
    > = match &embedding_provider {
        Some(provider) => {
            let mut r =
                aivyx_channel::persona_context::PersonaContextRefiner::with_defaults(
                    profile.clone(),
                    shared_persona.clone(),
                    active_role_name.clone(),
                    role_for_envelope.system_prompt.value.clone(),
                    Arc::clone(provider),
                );
            if let Some(stat) = &persona_selection_stat {
                r = r.with_stat(stat.clone());
            }
            Some(Arc::new(r))
        }
        None => None,
    };

    let memory_read = MemoryReadTool::new(Arc::clone(&memory));
    // Phase 7 task 5 — per-topic GC tripwire. Phase 9 Task 3 moved
    // resolution into `aivyx-config`; the cap arrives pre-parsed
    // from env / TOML / default with typed `Invalid` errors if a
    // source supplied a non-usize value. Destructured above as
    // `memory_cap` from `config.memory_max_per_topic.value`.
    let mut memory_write =
        MemoryWriteTool::new(Arc::clone(&memory)).set_max_per_topic(memory_cap);
    // Phase 75 — write-time embed hook (non-fatal; backfill is
    // the safety net). Only attached when a provider exists.
    if let Some(provider) = &embedding_provider {
        memory_write = memory_write.with_embedding_hook(Arc::new(
            aivyx_channel::memory_embedding::LlmEmbeddingHook::new(
                Arc::clone(provider),
            ),
        ));
    }
    let memory_forget = MemoryForgetTool::new(Arc::clone(&memory));
    let mut memory_search = MemorySearchTool::new(Arc::clone(&memory));
    // Phase 75 — semantic `mode` for the agent-facing tool.
    // Same hook the write path uses; absent → semantic requests
    // transparently fall back to keyword.
    if let Some(provider) = &embedding_provider {
        memory_search = memory_search.with_embedding_hook(Arc::new(
            aivyx_channel::memory_embedding::LlmEmbeddingHook::new(
                Arc::clone(provider),
            ),
        ));
    }
    let memory_gc = aivyx_channel::memory_gc_tool::MemoryGcTool::new(Arc::clone(&memory));

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
        Arc::new(memory_search) as Arc<dyn Tool>,
        Arc::new(memory_gc) as Arc<dyn Tool>,
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
    let web_fetch_tool: Arc<WebFetchTool> = build_web_fetch_for_channel(channel_kind)?;
    tool_list.push(Arc::clone(&web_fetch_tool) as Arc<dyn Tool>);
    let web_post_tool: Arc<WebPostTool> = build_web_post_for_channel(channel_kind)?;
    tool_list.push(Arc::clone(&web_post_tool) as Arc<dyn Tool>);

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
    let mission_list_tool: Arc<MissionListTool> = Arc::new(MissionListTool::new());
    tool_list.push(Arc::clone(&mission_list_tool) as Arc<dyn Tool>);
    let mission_status_tool: Arc<MissionStatusTool> = Arc::new(MissionStatusTool::new());
    tool_list.push(Arc::clone(&mission_status_tool) as Arc<dyn Tool>);

    let schedule_create_tool: Arc<ScheduleCreateTool> = Arc::new(ScheduleCreateTool::new());
    tool_list.push(Arc::clone(&schedule_create_tool) as Arc<dyn Tool>);
    let schedule_list_tool: Arc<ScheduleListTool> = Arc::new(ScheduleListTool::new());
    tool_list.push(Arc::clone(&schedule_list_tool) as Arc<dyn Tool>);
    let schedule_delete_tool: Arc<ScheduleDeleteTool> = Arc::new(ScheduleDeleteTool::new());
    tool_list.push(Arc::clone(&schedule_delete_tool) as Arc<dyn Tool>);
    let schedule_update_tool: Arc<ScheduleUpdateTool> = Arc::new(ScheduleUpdateTool::new());
    tool_list.push(Arc::clone(&schedule_update_tool) as Arc<dyn Tool>);

    let webhook_create_tool: Arc<WebhookCreateTool> = Arc::new(WebhookCreateTool::new());
    tool_list.push(Arc::clone(&webhook_create_tool) as Arc<dyn Tool>);
    let webhook_list_tool: Arc<WebhookListTool> = Arc::new(WebhookListTool::new());
    tool_list.push(Arc::clone(&webhook_list_tool) as Arc<dyn Tool>);
    let webhook_delete_tool: Arc<WebhookDeleteTool> = Arc::new(WebhookDeleteTool::new());
    tool_list.push(Arc::clone(&webhook_delete_tool) as Arc<dyn Tool>);

    let file_watch_create_tool: Arc<FileWatchCreateTool> = Arc::new(FileWatchCreateTool::new());
    tool_list.push(Arc::clone(&file_watch_create_tool) as Arc<dyn Tool>);
    let file_watch_list_tool: Arc<FileWatchListTool> = Arc::new(FileWatchListTool::new());
    tool_list.push(Arc::clone(&file_watch_list_tool) as Arc<dyn Tool>);
    let file_watch_delete_tool: Arc<FileWatchDeleteTool> = Arc::new(FileWatchDeleteTool::new());
    tool_list.push(Arc::clone(&file_watch_delete_tool) as Arc<dyn Tool>);

    let turn_history_tool: Arc<TurnHistoryTool> = Arc::new(TurnHistoryTool::new());
    tool_list.push(Arc::clone(&turn_history_tool) as Arc<dyn Tool>);

    let reflection_propose_tool: Arc<ReflectionProposeTool> = Arc::new(ReflectionProposeTool::new());
    tool_list.push(Arc::clone(&reflection_propose_tool) as Arc<dyn Tool>);
    let reflection_apply_tool: Arc<ReflectionApplyTool> = Arc::new(ReflectionApplyTool::new());
    tool_list.push(Arc::clone(&reflection_apply_tool) as Arc<dyn Tool>);

    let role_update_tool: Arc<RoleUpdateTool> = Arc::new(RoleUpdateTool::new());
    tool_list.push(Arc::clone(&role_update_tool) as Arc<dyn Tool>);
    let shared_role_overrides = aivyx_channel::role_overrides::shared_role_overrides();
    // `persona_log` + `shared_persona` were created earlier (right
    // after the role assemble) so the system-prompt path could read
    // the startup snapshot. The apply-tool setters land just below.

    let mut mcp_bridges: Vec<aivyx_mcp::McpServerBridge> = Vec::new();
    for mcp_cfg in &mcp_servers {
        let bridge_result = match mcp_cfg.transport {
            aivyx_config::McpTransportKind::Stdio => {
                // Phase 46: resolve `bundled = true` to current binary path.
                let resolved_cmd: String = if mcp_cfg.bundled {
                    std::env::current_exe()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| {
                            mcp_cfg.command.clone().unwrap_or_default()
                        })
                } else {
                    mcp_cfg.command.clone().unwrap_or_default()
                };
                let args_ref: Vec<&str> =
                    mcp_cfg.args.iter().map(|s| s.as_str()).collect();
                // Phase 55 — translate the operator's
                // [mcp_server.sandbox] config into the runtime
                // aivyx_mcp::SandboxConfig (parallel type per Phase
                // 55 Q1 resolution).
                let mcp_sandbox = mcp_cfg.sandbox.as_ref().map(|s| {
                    aivyx_mcp::SandboxConfig {
                        wrapper: s.wrapper.clone(),
                        args: s.args.clone(),
                    }
                });
                aivyx_mcp::McpServerBridge::start_with_sandbox(
                    &resolved_cmd,
                    &args_ref,
                    mcp_sandbox.as_ref(),
                    &mcp_cfg.name,
                )
                .await
            }
            aivyx_config::McpTransportKind::Sse => {
                let url = mcp_cfg.url.as_deref().unwrap_or("");
                match aivyx_mcp::SseTransport::connect(url).await {
                    Ok(transport) => {
                        aivyx_mcp::McpServerBridge::from_transport(
                            std::sync::Arc::new(transport),
                            &mcp_cfg.name,
                        )
                        .await
                    }
                    Err(e) => Err(e),
                }
            }
        };
        match bridge_result {
            Ok(bridge) => {
                match bridge.discover_tools().await {
                    Ok(mcp_tools) => {
                        let count = mcp_tools.len();
                        tool_list.extend(mcp_tools);
                        let transport_label = match mcp_cfg.transport {
                            aivyx_config::McpTransportKind::Stdio => "stdio",
                            aivyx_config::McpTransportKind::Sse => "sse",
                        };
                        eprintln!(
                            "aivyx: MCP server {:?} ({transport_label}) — {} tool(s) registered",
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

    // ---- Phase 49: tool processes (PRODUCT.md P12) ----------------------
    // Same shape as the MCP block above. One ToolProcessBridge per
    // `[[tool_process]]` entry. Each tool the process registers
    // becomes a ToolProxy in the registry. The bridge holds the
    // child via kill_on_drop, so SIGKILL fires automatically when
    // the bridge vec is dropped at daemon shutdown.
    //
    // Failure mode: a tool process that fails to spawn, fails the
    // handshake, or declares a scope outside the active role's
    // envelope is **logged and skipped**, not fatal. The daemon
    // continues with the tools that registered successfully.
    let mut tool_bridges: Vec<std::sync::Arc<aivyx_tool::ToolProcessBridge>> = Vec::new();
    for tp_cfg in &config_tool_processes {
        // Phase 52 — thread the operator's [tool_process.sandbox]
        // through to the aivyx-tool spawn config. None when the
        // operator omitted the nested block.
        let spawn_sandbox = tp_cfg.sandbox.as_ref().map(|s| aivyx_tool::SandboxConfig {
            wrapper: s.wrapper.clone(),
            args: s.args.clone(),
        });
        let spawn_cfg = aivyx_tool::ToolProcessConfig {
            name: tp_cfg.name.clone(),
            command: tp_cfg.command.clone(),
            args: tp_cfg.args.clone(),
            env: tp_cfg.env.clone(),
            sandbox: spawn_sandbox,
        };
        let bridge = match aivyx_tool::ToolProcessBridge::spawn(spawn_cfg).await {
            Ok(b) => std::sync::Arc::new(b),
            Err(e) => {
                eprintln!(
                    "aivyx: tool process {:?} failed to start: {e}",
                    tp_cfg.name,
                );
                continue;
            }
        };

        let mut registered = 0usize;
        for descriptor in bridge.descriptors() {
            // Resolve the effective scope: operator override (if
            // any) or the declared scope. Operator overrides must
            // be `is_granted_by(declared)` — anything wider is a
            // configuration error and the tool is skipped.
            let declared = match aivyx_capability::Scope::parse(&descriptor.required_scope) {
                Some(s) => s,
                None => {
                    eprintln!(
                        "aivyx: tool process {:?} tool {:?} declared unparseable scope {:?} — \
                         skipped",
                        tp_cfg.name, descriptor.name, descriptor.required_scope,
                    );
                    continue;
                }
            };
            let effective_scope = match tp_cfg.scope_overrides.get(&descriptor.name) {
                Some(override_str) => {
                    let parsed = match aivyx_capability::Scope::parse(override_str) {
                        Some(s) => s,
                        None => {
                            eprintln!(
                                "aivyx: tool process {:?} tool {:?} has unparseable \
                                 scope_override {:?} — skipped",
                                tp_cfg.name, descriptor.name, override_str,
                            );
                            continue;
                        }
                    };
                    if !parsed.is_granted_by(&declared) {
                        eprintln!(
                            "aivyx: tool process {:?} tool {:?} scope_override {:?} is not \
                             narrower than declared {:?} — skipped",
                            tp_cfg.name,
                            descriptor.name,
                            override_str,
                            descriptor.required_scope,
                        );
                        continue;
                    }
                    parsed
                }
                None => declared,
            };

            let proxy = aivyx_tool::ToolProxy::with_override_scope(
                std::sync::Arc::clone(&bridge),
                descriptor.name.clone(),
                descriptor.description.clone(),
                descriptor.input_schema.clone(),
                effective_scope,
            );
            tool_list.push(std::sync::Arc::new(proxy) as std::sync::Arc<dyn Tool>);
            registered += 1;
        }
        eprintln!(
            "aivyx: tool process {:?} — {} tool(s) registered",
            tp_cfg.name, registered,
        );
        tool_bridges.push(bridge);
    }

    // ---- Phase 36: Ollama model management tools ----------------------
    // Registered only when provider = "ollama". Uses the same base URL
    // resolved during provider construction.
    if let Some(ref ollama_url) = ollama_base_url_for_tools {
        use aivyx_channel::ollama_tools::{OllamaListTool, OllamaShowTool, OllamaPullTool};
        let list_tool = OllamaListTool::new(ollama_url)
            .map_err(|e| format!("failed to build ollama.list tool: {e}"))?;
        tool_list.push(Arc::new(list_tool) as Arc<dyn Tool>);
        let show_tool = OllamaShowTool::new(ollama_url)
            .map_err(|e| format!("failed to build ollama.show tool: {e}"))?;
        tool_list.push(Arc::new(show_tool) as Arc<dyn Tool>);
        let pull_tool = OllamaPullTool::new(ollama_url)
            .map_err(|e| format!("failed to build ollama.pull tool: {e}"))?;
        tool_list.push(Arc::new(pull_tool) as Arc<dyn Tool>);
    }

    // ---- Phase 62 — notify dispatcher + notify.send tool -------------
    // Build the dispatcher from the operator's `[[notify_target]]`
    // entries. For Telegram targets we share a single
    // `ReqwestTransport` constructed from `[telegram] token` (the
    // same bot client the channel-mode Telegram adapter uses).
    // Webhook targets are kind-independent of telegram config.
    //
    // If the operator has no `[[notify_target]]` entries, the
    // dispatcher is empty and every `notify.send` call surfaces
    // `UnknownTarget` — the operator-correct behavior (the agent
    // learns from the tool's `unknown_target` error_kind that no
    // targets exist).
    let notify_telegram_transport: Option<Arc<dyn aivyx_telegram::transport::TelegramTransport>> =
        if config_notify_targets.iter().any(|t| matches!(
            t.kind,
            aivyx_config::NotifyTargetKind::Telegram { .. }
        )) {
            // Build a transport iff there's at least one telegram
            // notify_target. The token is sourced from the same
            // `[telegram] token` slot the channel-mode adapter
            // uses; if it's absent here the dispatcher build below
            // returns a descriptive error.
            if let Some(token) = telegram.as_ref().and_then(|t| t.token.as_ref()) {
                use secrecy::ExposeSecret;
                Some(Arc::new(
                    aivyx_telegram::transport::ReqwestTransport::new(
                        token.value.expose_secret(),
                    ),
                ))
            } else {
                None
            }
        } else {
            None
        };
    // Phase 68 — build the shared SMTP transport once if any
    // email notify_target exists. Mirrors the Telegram pattern
    // above: shared client, Arc-cloned into each email backend.
    let email_context: Option<aivyx_channel::notify_dispatcher::EmailDispatchContext> =
        if config_notify_targets.iter().any(|t| matches!(
            t.kind,
            aivyx_config::NotifyTargetKind::Email { .. }
        )) {
            // The config loader already rejected
            // email-target-without-[email]-section, so `email`
            // is guaranteed Some here. Defense-in-depth fall-
            // through: build_notify_dispatcher returns a
            // descriptive error if email_context is None and
            // an email target is present.
            email.as_ref().and_then(|cfg| {
                match aivyx_channel::notify_email::LettreEmailSender::from_config(cfg) {
                    Ok(sender) => Some(
                        aivyx_channel::notify_dispatcher::EmailDispatchContext {
                            sender: Arc::new(sender),
                            from: cfg.from.clone(),
                        },
                    ),
                    Err(e) => {
                        eprintln!(
                            "aivyx: failed to build SMTP transport from \
                             [email] config: {e}"
                        );
                        None
                    }
                }
            })
        } else {
            None
        };
    // Phase 69 — Web UI desktop notify broadcaster. Constructed
    // here when the Web UI server is configured so the same
    // `Arc<WebUiBroadcaster>` is shared between the notify
    // dispatcher (push side, below) and the Web UI WS handler
    // (subscribe side, threaded through `DaemonConfig`).
    let web_ui_enabled = cli_web_ui_port.or(config_web_ui_port).is_some();
    let web_ui_broadcaster: Option<Arc<aivyx_channel::notify_webui::WebUiBroadcaster>> =
        if web_ui_enabled {
            Some(Arc::new(aivyx_channel::notify_webui::WebUiBroadcaster::new()))
        } else {
            None
        };
    let notify_dispatcher = aivyx_channel::notify_dispatcher::build_notify_dispatcher(
        &config_notify_targets,
        notify_telegram_transport,
        email_context,
        web_ui_broadcaster.clone(),
    )?;
    let notify_send_tool: Arc<aivyx_channel::notify_tool::NotifySendTool> =
        Arc::new(aivyx_channel::notify_tool::NotifySendTool::new());
    // Phase 63 Task 3: the same dispatcher is shared between
    // the agent-facing tool (Phase 62) and the trigger dispatch
    // path's auto-notify (Phase 63). Arc::clone for the tool;
    // a second clone goes to DaemonConfig below.
    notify_send_tool
        .set_dispatcher(Arc::clone(&notify_dispatcher))
        .map_err(|_| "notify.send dispatcher was set twice (programming error)")?;
    tool_list.push(Arc::clone(&notify_send_tool) as Arc<dyn Tool>);

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
        Scope::parse("memory.gc").unwrap(),
        fs_read_scope,
        fs_write_scope,
        Scope::parse("net.fetch").unwrap(),
        Scope::parse("net.post").unwrap(),
    ];
    if let Some(s) = shell_exec_scope {
        backcompat_floor.push(s);
    }
    // Phase 36 — grant ollama model management scopes in the
    // backcompat floor when provider is Ollama, so the default
    // role (empty capability_scopes) can use the tools.
    if ollama_base_url_for_tools.is_some() {
        backcompat_floor.push(Scope::parse("ollama.list").unwrap());
        backcompat_floor.push(Scope::parse("ollama.show").unwrap());
        backcompat_floor.push(Scope::parse("ollama.pull").unwrap());
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

    // ---- Phase 37 Task 4 — wire effective capabilities for redirect
    //      scope re-checks on web.fetch and web.post tools. The
    //      OnceLock is set once here; the tools read it on every
    //      redirect hop in their execute() loop.
    let _ = web_fetch_tool.set_effective_capabilities(capabilities.clone());
    let _ = web_post_tool.set_effective_capabilities(capabilities.clone());

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
    let memory_for_factory = Arc::clone(&memory);
    // Phase 57 Task 3 — Profile flavors every turn's system prompt,
    // including turns running inside a role-switch sub-session.
    // Clone once for the factory closure to capture (Profile holds
    // only owned data, no Arc indirection needed).
    let profile_for_factory = profile.clone();
    // Phase 59 Task 6 — same shape: the SharedEffectivePersona is
    // captured by the factory so child sessions see Persona-shaped
    // prompts identical to the parent. The clone is an `Arc<RwLock<_>>`
    // — cheap, lock-free at construction time.
    let persona_for_factory = shared_persona.clone();
    // Phase 76 — sub-agents recall too. Captured by-Option-Arc so
    // each child planner gets the same auto-recall hook the parent
    // has (or none, identically, when `[embedding]` is off).
    let recall_context_for_factory = recall_context.clone();
    // Phase 79 — sub-agents get the adaptive Soul too.
    let persona_refiner_for_factory = persona_refiner.clone();

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
        //
        // Phase 57 Task 3 + Phase 59 Task 6 — same Profile and
        // Persona injection as the parent path: layered three-section
        // composition when either Profile or Persona has content,
        // passthrough otherwise. Reading under the read lock per
        // child-session build is cheap; the lock is contended only
        // when reflection.apply lands a new delta.
        let persona_snapshot = persona_for_factory
            .read()
            .expect("persona lock not poisoned at child session build");
        let child_system_prompt = aivyx_channel::assemble_session_prompt(
            &profile_for_factory,
            Some(&*persona_snapshot),
            target,
            &target_role.system_prompt.value,
        );
        drop(persona_snapshot);
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
        let mut planner_config = LlmPlannerConfig::new(model_for_factory.clone())
            .with_system_prompt(child_system_prompt)
            .with_max_tokens(max_tokens_for_factory)
            .with_tool_allowlist(child_tool_allowlist.clone())
            .with_context_window(provider_kind.value.default_context_window())
            .with_prune_sink(Arc::new(
                aivyx_channel::prune_sink::MemoryPruneSink::new(Arc::clone(&memory_for_factory)),
            ));
        // Phase 76 — same auto-recall hook as the parent.
        if let Some(rc) = &recall_context_for_factory {
            planner_config =
                planner_config.with_context_provider(Arc::clone(rc));
        }
        // Phase 79 — same adaptive-Persona refiner as the parent.
        if let Some(pr) = &persona_refiner_for_factory {
            planner_config = planner_config
                .with_system_prompt_refiner(Arc::clone(pr));
        }
        let planner_provider = Arc::clone(&provider_for_factory);
        let planner_tools = Arc::clone(&tools_for_factory);
        // Phase 60 Task 3 — per-turn Persona refresh inside the
        // role-switch child agent. Each sub-session turn rebuilds
        // its system prompt from the current shared state, so an
        // operator-approved delta lands across the entire role
        // tree (parent + every active child).
        let child_refresher_profile = profile_for_factory.clone();
        let child_refresher_role_name = target.to_string();
        let child_refresher_role_prompt = target_role.system_prompt.value.clone();
        let child_refresher_shared = persona_for_factory.clone();
        let child_planner_factory = move || {
            let mut cfg = planner_config.clone();
            let snap = child_refresher_shared
                .read()
                .expect("persona lock not poisoned at child turn build");
            cfg.system_prompt = Some(aivyx_channel::assemble_session_prompt(
                &child_refresher_profile,
                Some(&*snap),
                &child_refresher_role_name,
                &child_refresher_role_prompt,
            ));
            drop(snap);
            Box::new(LlmPlanner::new(
                Arc::clone(&planner_provider),
                Arc::clone(&planner_tools),
                cfg,
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
    mission_list_tool
        .set_mission_store(storage.domain(KeyDomain::Missions))
        .map_err(|_| {
            "mission.list store was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    mission_status_tool
        .set_mission_store(storage.domain(KeyDomain::Missions))
        .map_err(|_| {
            "mission.status store was already set — startup path \
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

    schedule_create_tool
        .set_schedule_store(storage.domain(KeyDomain::Schedules))
        .map_err(|_| {
            "schedule.create store was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    schedule_list_tool
        .set_schedule_store(storage.domain(KeyDomain::Schedules))
        .map_err(|_| {
            "schedule.list store was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    schedule_delete_tool
        .set_schedule_store(storage.domain(KeyDomain::Schedules))
        .map_err(|_| {
            "schedule.delete store was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    schedule_update_tool
        .set_schedule_store(storage.domain(KeyDomain::Schedules))
        .map_err(|_| {
            "schedule.update store was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;

    webhook_create_tool
        .set_webhook_store(storage.domain(KeyDomain::Webhooks))
        .map_err(|_| {
            "webhook.create store was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    webhook_list_tool
        .set_webhook_store(storage.domain(KeyDomain::Webhooks))
        .map_err(|_| {
            "webhook.list store was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    webhook_delete_tool
        .set_webhook_store(storage.domain(KeyDomain::Webhooks))
        .map_err(|_| {
            "webhook.delete store was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;

    file_watch_create_tool
        .set_file_watch_store(storage.domain(KeyDomain::FileWatches))
        .map_err(|_| {
            "file_watch.create store was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    file_watch_list_tool
        .set_file_watch_store(storage.domain(KeyDomain::FileWatches))
        .map_err(|_| {
            "file_watch.list store was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    file_watch_delete_tool
        .set_file_watch_store(storage.domain(KeyDomain::FileWatches))
        .map_err(|_| {
            "file_watch.delete store was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;

    turn_history_tool
        .set_audit_log(Arc::clone(&audit_log_for_tool))
        .map_err(|_| {
            "turn.history audit log was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;

    reflection_propose_tool
        .set_audit_log(audit_log_for_tool)
        .map_err(|_| {
            "reflection.propose audit log was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    reflection_propose_tool
        .set_mission_store(storage.domain(KeyDomain::Missions))
        .map_err(|_| {
            "reflection.propose mission store was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    reflection_propose_tool
        .set_role_name(active_role_name.clone())
        .map_err(|_| {
            "reflection.propose role_name was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    // Phase 70 — register the proposal chain so agent-supplied
    // persona deltas also land in the Web UI Proposals pane /
    // `aivyx persona proposals` CLI alongside the existing
    // mission-gate flow.
    reflection_propose_tool
        .set_persona_proposal_log(Arc::clone(&persona_proposal_log))
        .map_err(|_| {
            "reflection.propose persona_proposal_log was already set — \
             startup path bug, should be called exactly once"
                .to_string()
        })?;

    reflection_apply_tool
        .set_mission_store(storage.domain(KeyDomain::Missions))
        .map_err(|_| {
            "reflection.apply mission store was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    reflection_apply_tool
        .set_memory(Arc::clone(&memory))
        .map_err(|_| {
            "reflection.apply memory was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    reflection_apply_tool
        .set_role_overrides(shared_role_overrides.clone())
        .map_err(|_| {
            "reflection.apply role_overrides was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    // Phase 59 — apply tool gets the persistent Persona chain (for
    // delta append on approval) and the shared runtime state (for
    // immediate hot-reload of the next turn's effective Persona).
    reflection_apply_tool
        .set_persona_log(Arc::clone(&persona_log))
        .map_err(|_| {
            "reflection.apply persona_log was already set — startup path \
             bug, should be called exactly once"
                .to_string()
        })?;
    reflection_apply_tool
        .set_effective_persona(shared_persona.clone())
        .map_err(|_| {
            "reflection.apply effective_persona was already set — startup \
             path bug, should be called exactly once"
                .to_string()
        })?;

    role_update_tool
        .set_overrides(shared_role_overrides.clone())
        .map_err(|_| {
            "role.update overrides was already set — startup path \
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
        let mut planner_config = LlmPlannerConfig::new(model.clone())
            .with_system_prompt(system_prompt)
            .with_max_tokens(DEFAULT_MAX_TOKENS)
            .with_tool_allowlist(tool_allowlist)
            .with_context_window(provider_kind.value.default_context_window())
            .with_prune_sink(Arc::new(
                aivyx_channel::prune_sink::MemoryPruneSink::new(Arc::clone(&memory)),
            ));
        // Phase 76 — automatic recall (Q1a). Carried by-Arc
        // through the per-turn `planner_config.clone()` in the
        // factory below, exactly like the prune sink.
        if let Some(rc) = &recall_context {
            planner_config =
                planner_config.with_context_provider(Arc::clone(rc));
        }
        // Phase 79 — adaptive Persona refiner (daemon path).
        if let Some(pr) = &persona_refiner {
            planner_config = planner_config
                .with_system_prompt_refiner(Arc::clone(pr));
        }
        let planner_provider = Arc::clone(&provider);
        let planner_tools = Arc::clone(&tools);
        let daemon_overrides = shared_role_overrides.clone();
        // Phase 60 Task 3 — per-turn Persona refresh, same shape
        // as the local-CLI session config above.
        let daemon_refresher_profile = profile.clone();
        let daemon_refresher_role_name = active_role_name.clone();
        let daemon_refresher_role_prompt = role_for_envelope.system_prompt.value.clone();
        let daemon_refresher_shared = shared_persona.clone();
        let planner_factory = move || {
            let mut cfg = planner_config.clone();
            // Per-turn rebuild from current Persona state.
            let snap = daemon_refresher_shared
                .read()
                .expect("persona lock not poisoned at turn build");
            cfg.system_prompt = Some(aivyx_channel::assemble_session_prompt(
                &daemon_refresher_profile,
                Some(&*snap),
                &daemon_refresher_role_name,
                &daemon_refresher_role_prompt,
            ));
            drop(snap);
            if let Ok(overrides) = daemon_overrides.read() {
                if !overrides.is_empty() {
                    aivyx_channel::role_overrides::apply_to_planner_config(
                        &overrides,
                        &mut cfg,
                    );
                }
            }
            Box::new(LlmPlanner::new(
                Arc::clone(&planner_provider),
                Arc::clone(&planner_tools),
                cfg,
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
                aivyx_channel::daemon_ipc::FrontendType::Web => {
                    Arc::new(aivyx_channel::web_ui::WebDaemonChannel::new())
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

        // Sync TOML [[schedule]] entries into the schedule store.
        let schedule_domain = storage.domain(KeyDomain::Schedules);
        if !config_schedules.is_empty() {
            match aivyx_channel::daemon_scheduler::config_to_records(&config_schedules) {
                Ok(records) => {
                    match aivyx_channel::daemon_scheduler::sync_config_schedules(
                        &schedule_domain,
                        &records,
                    )
                    .await
                    {
                        Ok(n) if n > 0 => {
                            eprintln!("aivyx daemon: synced {n} schedule(s) from config");
                        }
                        Err(e) => {
                            eprintln!("aivyx daemon: failed to sync config schedules: {e}");
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    eprintln!("aivyx daemon: invalid schedule config: {e}");
                }
            }
        }

        // Sync TOML [[webhook]] entries into the webhook store.
        let webhook_domain = storage.domain(KeyDomain::Webhooks);
        if !config_webhooks.is_empty() {
            let mut synced = 0usize;
            for wh_cfg in &config_webhooks {
                let wh_id = format!("cfg-{}", wh_cfg.name);
                match aivyx_channel::webhook::get_webhook(&webhook_domain, &wh_id).await {
                    Ok(None) => {
                        let mut record = aivyx_channel::webhook::WebhookRecord::new(
                            wh_id,
                            wh_cfg.role.clone(),
                            wh_cfg.prompt.clone(),
                        );
                        record.wrap_mission = wh_cfg.wrap_mission;
                        record.notify_target = wh_cfg.notify_target.clone();
                        record.notify_targets = wh_cfg.notify_targets.clone();
                        record.notify_when = wh_cfg.notify_when;
                        if let Err(e) = aivyx_channel::webhook::create_webhook(
                            &webhook_domain,
                            &record,
                        ).await {
                            eprintln!("aivyx daemon: failed to sync webhook {:?}: {e}", wh_cfg.name);
                        } else {
                            synced += 1;
                        }
                    }
                    Ok(Some(_)) => {} // already exists in storage
                    Err(e) => {
                        eprintln!("aivyx daemon: webhook sync lookup failed: {e}");
                    }
                }
            }
            if synced > 0 {
                eprintln!("aivyx daemon: synced {synced} webhook(s) from config");
            }
        }

        // Sync TOML [[file_watch]] entries into the file-watch store.
        let file_watch_domain = storage.domain(KeyDomain::FileWatches);
        if !config_file_watches.is_empty() {
            let records = aivyx_channel::file_watcher::config_to_records(&config_file_watches);
            match aivyx_channel::file_watcher::sync_config_file_watches(
                &file_watch_domain,
                &records,
            )
            .await
            {
                Ok(n) if n > 0 => {
                    eprintln!("aivyx daemon: synced {n} file watch(es) from config");
                }
                Err(e) => {
                    eprintln!("aivyx daemon: file-watch config sync failed: {e}");
                }
                _ => {}
            }
        }

        let result = run_daemon(DaemonConfig {
            socket_path,
            agent,
            channel_factory,
            shutdown,
            mission_store: Some(storage.domain(KeyDomain::Missions)),
            // Phase 63 Task 3 — pass the same NotifyDispatcher
            // the NotifySendTool got (Task 8 / Phase 62) so the
            // trigger dispatch path can auto-notify on
            // trigger-fired turns.
            notify_dispatcher: Some(Arc::clone(&notify_dispatcher)),
            schedule_store: Some(schedule_domain),
            webhook_store: Some(webhook_domain),
            file_watch_store: Some(file_watch_domain),
            webhook_port: config_webhook_port,
            web_ui_port: cli_web_ui_port.or(config_web_ui_port),
            memory: Some(Arc::clone(&memory)),
            memory_ttl_secs: memory_ttl_secs.map(|s| s.value),
            audit_log: Some(Arc::clone(&persistent_audit_for_query)),
            // Phase 60 — persona log + shared state for inspection
            // queries (Query::ListPersonaDeltas /
            // Query::GetEffectivePersona) and the revert flow
            // (FrontendMessage::RevertPersonaDelta). Both were
            // opened at the daemon startup path (Phase 59 Task 2 +
            // Task 5).
            persona_log: Some(Arc::clone(&persona_log)),
            shared_persona: shared_persona.clone(),
            // Phase 58 — operator-declared Profile snapshot for the
            // `Query::GetProfile` IPC handler. `profile` was bound
            // at the AivyxConfig destructure (Phase 57 Task 2); a
            // clone here lives alongside `profile_for_factory` the
            // role-switch path captured.
            profile: Arc::new(profile.clone()),
            // Phase 69 — same Arc<WebUiBroadcaster> the notify
            // dispatcher above received. Threading the single
            // instance into both sides is what gives Web UI
            // desktop notify its fan-out: the dispatcher pushes
            // and the WS handler subscribes one receiver per
            // browser connection.
            web_ui_broadcaster: web_ui_broadcaster.clone(),
            // Phase 70 — proposal chain opened at startup (see
            // `persona_proposal_log` binding above). Threaded
            // into `DaemonConfig` so the IPC query / resolve
            // handlers in `handle_query` and the resolve arm
            // can read the chain and append status transitions.
            persona_proposal_log: Some(Arc::clone(&persona_proposal_log)),
            // Phase 71 — validated reflection schedules from the
            // config loader. The daemon spawns
            // run_reflection_scheduler when this is non-empty AND
            // an audit log is configured.
            reflection_schedules: config_reflection_schedules.clone(),
            // Phase 73 — per-target retry + rate-limit policies
            // built from the loaded `[[notify_target]]` blocks.
            // Empty map when no targets exist; the dispatcher's
            // retry loop defaults to zero retries either way.
            target_policies: aivyx_channel::trigger::TargetPolicy::map_from_targets(
                &config_notify_targets,
            ),
            // Phase 74 — per-topic-glob retention rules. Threaded
            // into the daemon's memory-GC timer (the hourly pass
            // respects first-match retention before falling back
            // to the global memory_ttl_secs).
            memory_retention: config_memory_retention,
            // Phase 75 — shared embedding provider. `Some` iff
            // `[embedding]` is configured; drives the daemon's
            // hourly lazy-backfill pass.
            embedding_provider: embedding_provider.clone(),
            // Phase 77 — the shared recall-feedback log. `Some`
            // iff auto-recall is configured; the reflection
            // scheduler reads/clamps it on cadence.
            recall_log: recall_log.clone(),
            // Phase 82 — durable helpfulness ledger; the
            // recall-feedback pass folds each window into it.
            helpfulness_ledger: helpfulness_ledger.clone(),
            // Phase 83 — durable cross-session co-occurrence
            // ledger; folded by the same pass.
            cooccurrence_ledger: cooccurrence_ledger.clone(),
            // Phase 79 (Q4a) — same handle the adaptive refiner
            // writes; the GetLearningInsights handler reads it.
            persona_selection_stat: persona_selection_stat.clone(),
            // Phase 80 — proactive surfacing config + dedup log.
            proactive_config: config_proactive.clone(),
            proactive_log: proactive_log.clone(),
            proactive_stat: proactive_stat.clone(),
            // Phase 81 — persona-lifecycle config + last-cycle
            // stat. The persona/proposal chains + embedding
            // are already on DaemonConfig; the pass picks them
            // up there.
            persona_lifecycle_config: config_persona_lifecycle
                .clone(),
            persona_lifecycle_stat: persona_lifecycle_stat
                .clone(),
        })
            .await;

        for bridge in mcp_bridges {
            let _ = bridge.shutdown().await;
        }
        // Phase 49 — tool processes get a polite ToolShutdown; the
        // kill_on_drop safety net SIGKILLs anything that doesn't
        // exit cleanly when the Vec drops.
        for bridge in &tool_bridges {
            let _ = bridge.shutdown().await;
        }
        drop(tool_bridges);
        return result.map_err(|e| e.to_string());
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

            // Phase 60 Task 3 — assemble a per-turn refresher
            // closure that re-runs assemble_session_prompt with the
            // current shared_persona state. Approved Persona deltas
            // applied via reflection.apply take effect on the next
            // turn through this path.
            let refresher_profile = profile.clone();
            let refresher_role_name = active_role_name.clone();
            let refresher_role_prompt = role_for_envelope.system_prompt.value.clone();
            let refresher_shared = shared_persona.clone();
            let prompt_refresher: Arc<dyn Fn() -> String + Send + Sync> =
                Arc::new(move || {
                    let snap = refresher_shared
                        .read()
                        .expect("persona lock not poisoned at turn build");
                    aivyx_channel::assemble_session_prompt(
                        &refresher_profile,
                        Some(&*snap),
                        &refresher_role_name,
                        &refresher_role_prompt,
                    )
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
                role_overrides: Some(shared_role_overrides),
                prompt_refresher: Some(prompt_refresher),
                context_window_tokens: Some(provider_kind.value.default_context_window()),
                prune_sink: Some(Arc::new(
                    aivyx_channel::prune_sink::MemoryPruneSink::new(Arc::clone(&memory)),
                )),
                // Phase 76 — automatic recall (Q1a). `None` when
                // `[embedding]` is unconfigured → no auto-recall.
                context_provider: recall_context.clone(),
                // Phase 79 — adaptive Persona (local-CLI path).
                system_prompt_refiner: persona_refiner.clone(),
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

    // -----------------------------------------------------------------
    // Phase 37 Task 3 — registration-time gate for `web.post`.
    //
    // `web.post` is registered for both channel kinds (like
    // `web.fetch`), but is Trusted-only by ceiling — the tool
    // will be present in the registry for SemiTrusted channels,
    // but the capability intersection against CEILING_SEMITRUSTED
    // (which lacks `net.post`) prevents its use there.
    // -----------------------------------------------------------------

    #[test]
    fn channel_local_receives_web_post() {
        let tool = build_web_post_for_channel(ChannelKind::Local)
            .expect("local branch must build web.post cleanly");
        assert_eq!(tool.name(), "web.post");
    }

    #[test]
    fn channel_telegram_receives_web_post() {
        let tool = build_web_post_for_channel(ChannelKind::Telegram)
            .expect("telegram branch must build web.post cleanly");
        assert_eq!(tool.name(), "web.post");
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
            Scope::parse("memory.gc").unwrap(),
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
            "role.switch:junior_researcher",
        ];
        expected.sort();
        assert_eq!(
            got, expected,
            "researcher runtime envelope (after Trusted ceiling) must be \
             exactly the documented six scopes — includes \
             role.switch:junior_researcher added in Phase 33"
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

    // -----------------------------------------------------------------
    // Phase 24 Task 5 — `--mcp-server` flag parser tests.
    // -----------------------------------------------------------------

    #[test]
    fn mcp_server_flag_parses_name_and_command() {
        let parsed = parse_cli_args_from(&argv(&["--mcp-server", "github:npx"]))
            .expect("`--mcp-server github:npx` must parse");
        assert_eq!(parsed.mcp_servers.len(), 1);
        assert_eq!(parsed.mcp_servers[0].name, "github");
        assert_eq!(parsed.mcp_servers[0].command, "npx");
        assert!(parsed.mcp_servers[0].args.is_empty());
    }

    #[test]
    fn mcp_server_flag_parses_with_args() {
        let parsed = parse_cli_args_from(&argv(&[
            "--mcp-server",
            "github:npx:-y,@modelcontextprotocol/server-github",
        ]))
        .expect("--mcp-server with args must parse");
        assert_eq!(parsed.mcp_servers[0].name, "github");
        assert_eq!(parsed.mcp_servers[0].command, "npx");
        assert_eq!(
            parsed.mcp_servers[0].args,
            vec!["-y", "@modelcontextprotocol/server-github"]
        );
    }

    #[test]
    fn mcp_server_flag_repeatable() {
        let parsed = parse_cli_args_from(&argv(&[
            "--mcp-server", "a:cmd-a",
            "--mcp-server", "b:cmd-b:arg1,arg2",
        ]))
        .expect("repeated --mcp-server must parse");
        assert_eq!(parsed.mcp_servers.len(), 2);
        assert_eq!(parsed.mcp_servers[0].name, "a");
        assert_eq!(parsed.mcp_servers[1].name, "b");
        assert_eq!(parsed.mcp_servers[1].args, vec!["arg1", "arg2"]);
    }

    #[test]
    fn mcp_server_flag_missing_value_is_an_error() {
        let err = parse_cli_args_from(&argv(&["--mcp-server"]))
            .expect_err("`--mcp-server` with no value must error");
        assert!(err.contains("--mcp-server"), "error must mention flag: {err}");
    }

    #[test]
    fn mcp_server_flag_malformed_value_is_an_error() {
        let err = parse_cli_args_from(&argv(&["--mcp-server", "nocolon"]))
            .expect_err("`--mcp-server nocolon` must error");
        assert!(err.contains("malformed"), "error must say malformed: {err}");
    }

    #[test]
    fn mcp_server_flag_empty_name_is_an_error() {
        let err = parse_cli_args_from(&argv(&["--mcp-server", ":npx"]))
            .expect_err("`--mcp-server :npx` must error");
        assert!(err.contains("malformed"), "error must say malformed: {err}");
    }

    #[test]
    fn default_args_have_empty_mcp_servers() {
        let parsed = parse_cli_args_from(&argv(&[]))
            .expect("empty argv must parse");
        assert!(parsed.mcp_servers.is_empty());
        assert!(parsed.mcp_sse_servers.is_empty());
    }

    // -----------------------------------------------------------------
    // Phase 32 Task 4 — `--mcp-sse` flag parser tests.
    // -----------------------------------------------------------------

    #[test]
    fn mcp_sse_flag_parses_name_and_url() {
        let parsed = parse_cli_args_from(&argv(&[
            "--mcp-sse",
            "remote:http://host:8080/sse",
        ]))
        .expect("`--mcp-sse remote:url` must parse");
        assert_eq!(parsed.mcp_sse_servers.len(), 1);
        assert_eq!(parsed.mcp_sse_servers[0].name, "remote");
        assert_eq!(parsed.mcp_sse_servers[0].url, "http://host:8080/sse");
    }

    #[test]
    fn mcp_sse_flag_repeatable() {
        let parsed = parse_cli_args_from(&argv(&[
            "--mcp-sse", "a:http://a/sse",
            "--mcp-sse", "b:https://b/sse",
        ]))
        .expect("repeated --mcp-sse must parse");
        assert_eq!(parsed.mcp_sse_servers.len(), 2);
        assert_eq!(parsed.mcp_sse_servers[0].name, "a");
        assert_eq!(parsed.mcp_sse_servers[1].name, "b");
    }

    #[test]
    fn mcp_sse_flag_missing_value_is_error() {
        let err = parse_cli_args_from(&argv(&["--mcp-sse"]))
            .expect_err("--mcp-sse with no value must error");
        assert!(err.contains("--mcp-sse"), "error must mention flag: {err}");
    }

    #[test]
    fn mcp_sse_flag_malformed_is_error() {
        let err = parse_cli_args_from(&argv(&["--mcp-sse", "nocolon"]))
            .expect_err("--mcp-sse nocolon must error");
        assert!(err.contains("malformed"), "error must say malformed: {err}");
    }

    #[test]
    fn mcp_sse_and_stdio_flags_combine() {
        let parsed = parse_cli_args_from(&argv(&[
            "--mcp-server", "local:npx",
            "--mcp-sse", "remote:http://host/sse",
        ]))
        .expect("combining --mcp-server and --mcp-sse must parse");
        assert_eq!(parsed.mcp_servers.len(), 1);
        assert_eq!(parsed.mcp_sse_servers.len(), 1);
    }

    // ---- --provider flag tests ----------------------------------------

    #[test]
    fn provider_flag_anthropic() {
        let parsed = parse_cli_args_from(&argv(&["--provider", "anthropic"]))
            .expect("must parse");
        assert_eq!(parsed.provider, Some(ProviderKind::Anthropic));
    }

    #[test]
    fn provider_flag_openai() {
        let parsed = parse_cli_args_from(&argv(&["--provider", "openai"]))
            .expect("must parse");
        assert_eq!(parsed.provider, Some(ProviderKind::OpenAi));
    }

    #[test]
    fn provider_flag_unknown_is_error() {
        let err = parse_cli_args_from(&argv(&["--provider", "gemini"]))
            .expect_err("unknown provider must error");
        assert!(err.contains("unrecognized provider"), "error: {err}");
    }

    #[test]
    fn provider_flag_missing_value_is_error() {
        let err = parse_cli_args_from(&argv(&["--provider"]))
            .expect_err("missing value must error");
        assert!(err.contains("requires a value"), "error: {err}");
    }

    #[test]
    fn no_provider_flag_defaults_to_none() {
        let parsed = parse_cli_args_from(&argv(&[]))
            .expect("must parse");
        assert!(parsed.provider.is_none());
    }

    #[test]
    fn provider_flag_ollama() {
        let parsed = parse_cli_args_from(&argv(&["--provider", "ollama"]))
            .expect("must parse");
        assert_eq!(parsed.provider, Some(ProviderKind::Ollama));
    }

    // -----------------------------------------------------------------
    // --web-ui / --web-ui-port — Phase 39
    // -----------------------------------------------------------------

    #[test]
    fn web_ui_flag_sets_default_port() {
        let parsed = parse_cli_args_from(&argv(&["daemon", "run", "--web-ui"]))
            .expect("must parse");
        assert_eq!(
            parsed.web_ui_port,
            Some(aivyx_channel::web_ui::DEFAULT_WEB_UI_PORT)
        );
    }

    #[test]
    fn web_ui_port_flag_sets_custom_port() {
        let parsed = parse_cli_args_from(&argv(&["daemon", "run", "--web-ui-port", "8080"]))
            .expect("must parse");
        assert_eq!(parsed.web_ui_port, Some(8080));
    }

    #[test]
    fn web_ui_port_flag_invalid_value_errors() {
        let err = parse_cli_args_from(&argv(&["daemon", "run", "--web-ui-port", "notaport"]))
            .expect_err("invalid port must error");
        assert!(err.contains("not a valid port"), "error: {err}");
    }

    #[test]
    fn no_web_ui_flag_means_none() {
        let parsed = parse_cli_args_from(&argv(&["daemon", "run"]))
            .expect("must parse");
        assert_eq!(parsed.web_ui_port, None);
    }

    // -----------------------------------------------------------------------
    // Phase 61 — `aivyx --version` / `-V` flag
    // -----------------------------------------------------------------------

    #[test]
    fn parse_version_long_flag() {
        let parsed = parse_cli_args_from(&argv(&["--version"]))
            .expect("--version must parse");
        assert_eq!(parsed.mode, CliMode::Version);
    }

    #[test]
    fn parse_version_short_flag() {
        let parsed = parse_cli_args_from(&argv(&["-V"]))
            .expect("-V must parse");
        assert_eq!(parsed.mode, CliMode::Version);
    }

    #[test]
    fn parse_version_rejects_extra_args() {
        let err = parse_cli_args_from(&argv(&["--version", "--channel", "local"]))
            .expect_err("--version with flags must error");
        assert!(
            err.contains("does not accept additional arguments"),
            "error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Phase 44 — `aivyx init` subcommand
    // -----------------------------------------------------------------------

    #[test]
    fn parse_init_subcommand() {
        let parsed = parse_cli_args_from(&argv(&["init"]))
            .expect("init must parse");
        assert_eq!(parsed.mode, CliMode::Init(InitMode::Interactive));
    }

    #[test]
    fn parse_init_rejects_extra_args() {
        let err = parse_cli_args_from(&argv(&["init", "--channel", "local"]))
            .expect_err("init with flags must error");
        assert!(
            err.contains("unrecognized argument"),
            "error: {err}"
        );
    }

    #[test]
    fn parse_init_with_template_pre_fills() {
        let parsed = parse_cli_args_from(&argv(&["init", "--template", "coder"]))
            .expect("`init --template coder` must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Init(InitMode::InteractiveFromTemplate {
                template_name: "coder".into(),
            }),
        );
    }

    #[test]
    fn parse_init_template_without_name_is_list_mode() {
        let parsed = parse_cli_args_from(&argv(&["init", "--template"]))
            .expect("`init --template` (no name) must parse to list mode");
        assert_eq!(parsed.mode, CliMode::Init(InitMode::ListTemplates));
    }

    #[test]
    fn parse_init_list_templates_flag() {
        let parsed = parse_cli_args_from(&argv(&["init", "--list-templates"]))
            .expect("`init --list-templates` must parse");
        assert_eq!(parsed.mode, CliMode::Init(InitMode::ListTemplates));
    }

    #[test]
    fn parse_init_template_followed_by_flag_is_list_mode() {
        // `aivyx init --template --list-templates` — the second
        // flag follows immediately, so --template has no name and
        // routes to list mode. Cleaner than erroring; intent is
        // recoverable.
        let parsed = parse_cli_args_from(&argv(&[
            "init",
            "--template",
            "--list-templates",
        ]))
        .expect("must parse");
        assert_eq!(parsed.mode, CliMode::Init(InitMode::ListTemplates));
    }

    #[test]
    fn parse_init_is_not_daemon_subcommand() {
        let err = parse_cli_args_from(&argv(&["daemon", "init"]))
            .expect_err("daemon init is not valid");
        assert!(
            err.contains("unrecognized daemon subcommand"),
            "error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Phase 46 — `aivyx mcp-server <name>` subcommand
    // -----------------------------------------------------------------------

    #[test]
    fn parse_mcp_server_subcommand() {
        let parsed = parse_cli_args_from(&argv(&["mcp-server", "web-search"]))
            .expect("mcp-server web-search must parse");
        assert_eq!(parsed.mode, CliMode::McpServer("web-search".into()));
    }

    #[test]
    fn parse_mcp_server_missing_name() {
        let err = parse_cli_args_from(&argv(&["mcp-server"]))
            .expect_err("mcp-server without name must error");
        assert!(
            err.contains("requires a server name"),
            "error: {err}"
        );
    }

    #[test]
    fn parse_mcp_server_unknown_name() {
        let err = parse_cli_args_from(&argv(&["mcp-server", "bogus"]))
            .expect_err("unknown server name must error");
        assert!(
            err.contains("unknown MCP server name"),
            "error: {err}"
        );
    }

    #[test]
    fn parse_mcp_server_no_extra_args() {
        let err = parse_cli_args_from(&argv(&["mcp-server", "web-search", "--verbose"]))
            .expect_err("extra args must error");
        assert!(
            err.contains("does not accept additional arguments"),
            "error: {err}"
        );
    }

    // -----------------------------------------------------------------
    // Phase 58 — `aivyx profile <subcommand>` parser tests.
    // -----------------------------------------------------------------

    #[test]
    fn profile_show_parses_to_profile_show_mode() {
        let parsed = parse_cli_args_from(&argv(&["profile", "show"]))
            .expect("`profile show` must parse");
        assert_eq!(parsed.mode, CliMode::Profile(ProfileSubcommand::Show));
    }

    #[test]
    fn profile_edit_parses_to_profile_edit_mode() {
        let parsed = parse_cli_args_from(&argv(&["profile", "edit"]))
            .expect("`profile edit` must parse");
        assert_eq!(parsed.mode, CliMode::Profile(ProfileSubcommand::Edit));
    }

    #[test]
    fn profile_without_subcommand_is_an_error() {
        let err = parse_cli_args_from(&argv(&["profile"]))
            .expect_err("`profile` alone must error");
        assert!(
            err.contains("show") && err.contains("edit"),
            "error must list both subcommands: {err}"
        );
    }

    #[test]
    fn profile_unknown_subcommand_is_an_error() {
        let err = parse_cli_args_from(&argv(&["profile", "delete"]))
            .expect_err("`profile delete` must error");
        assert!(
            err.contains("unrecognized profile subcommand"),
            "error: {err}"
        );
    }

    #[test]
    fn profile_show_rejects_extra_args() {
        let err = parse_cli_args_from(&argv(&["profile", "show", "--verbose"]))
            .expect_err("extra args must error");
        assert!(
            err.contains("does not accept additional arguments"),
            "error: {err}"
        );
    }

    // -----------------------------------------------------------------
    // Phase 60 — `aivyx persona <subcommand>` parser tests.
    // -----------------------------------------------------------------

    #[test]
    fn persona_show_parses_to_persona_show_mode() {
        let parsed = parse_cli_args_from(&argv(&["persona", "show"]))
            .expect("`persona show` must parse");
        assert_eq!(parsed.mode, CliMode::Persona(PersonaSubcommand::Show));
    }

    #[test]
    fn persona_list_parses_to_persona_list_mode() {
        let parsed = parse_cli_args_from(&argv(&["persona", "list"]))
            .expect("`persona list` must parse");
        assert_eq!(parsed.mode, CliMode::Persona(PersonaSubcommand::List));
    }

    #[test]
    fn persona_revert_parses_with_target_id() {
        let parsed = parse_cli_args_from(&argv(&["persona", "revert", "pd-abc123"]))
            .expect("`persona revert pd-abc123` must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Persona(PersonaSubcommand::Revert {
                target_delta_id: "pd-abc123".into(),
            })
        );
    }

    #[test]
    fn persona_revert_without_target_is_an_error() {
        let err = parse_cli_args_from(&argv(&["persona", "revert"]))
            .expect_err("`persona revert` without id must error");
        assert!(
            err.contains("requires a delta id"),
            "error: {err}"
        );
    }

    #[test]
    fn persona_revert_with_extra_args_is_an_error() {
        let err = parse_cli_args_from(&argv(&["persona", "revert", "pd-1", "pd-2"]))
            .expect_err("extra revert args must error");
        assert!(
            err.contains("exactly one delta id"),
            "error: {err}"
        );
    }

    #[test]
    fn persona_without_subcommand_is_an_error() {
        let err = parse_cli_args_from(&argv(&["persona"]))
            .expect_err("`persona` alone must error");
        assert!(
            err.contains("show") && err.contains("list") && err.contains("revert"),
            "error must list all subcommands: {err}"
        );
    }

    // -----------------------------------------------------------------
    // Phase 70 — `aivyx persona proposals <subcommand>` parser tests.
    // -----------------------------------------------------------------

    #[test]
    fn proposals_list_default_status_is_pending() {
        let parsed = parse_cli_args_from(&argv(&["persona", "proposals", "list"]))
            .expect("`persona proposals list` must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Persona(PersonaSubcommand::Proposals(
                ProposalsSubcommand::List {
                    status: "pending".into(),
                }
            ))
        );
    }

    #[test]
    fn proposals_list_status_flag_normalizes_case() {
        let parsed = parse_cli_args_from(&argv(&[
            "persona", "proposals", "list", "--status", "APPROVED",
        ]))
        .expect("must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Persona(PersonaSubcommand::Proposals(
                ProposalsSubcommand::List {
                    status: "approved".into(),
                }
            ))
        );
    }

    #[test]
    fn proposals_list_unknown_status_errors() {
        let err = parse_cli_args_from(&argv(&[
            "persona", "proposals", "list", "--status", "bogus",
        ]))
        .expect_err("must error");
        assert!(err.contains("unknown --status"), "{err}");
        assert!(err.contains("pending"), "{err}");
    }

    #[test]
    fn proposals_show_parses_with_id() {
        let parsed = parse_cli_args_from(&argv(&[
            "persona", "proposals", "show", "pp-xyz",
        ]))
        .expect("must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Persona(PersonaSubcommand::Proposals(
                ProposalsSubcommand::Show {
                    proposal_id: "pp-xyz".into(),
                }
            ))
        );
    }

    #[test]
    fn proposals_approve_parses_with_id() {
        let parsed = parse_cli_args_from(&argv(&[
            "persona", "proposals", "approve", "pp-xyz",
        ]))
        .expect("must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Persona(PersonaSubcommand::Proposals(
                ProposalsSubcommand::Approve {
                    proposal_id: "pp-xyz".into(),
                }
            ))
        );
    }

    #[test]
    fn proposals_reject_parses_with_optional_reason() {
        let no_reason = parse_cli_args_from(&argv(&[
            "persona", "proposals", "reject", "pp-1",
        ]))
        .expect("must parse");
        assert_eq!(
            no_reason.mode,
            CliMode::Persona(PersonaSubcommand::Proposals(
                ProposalsSubcommand::Reject {
                    proposal_id: "pp-1".into(),
                    reason: None,
                }
            ))
        );

        let with_reason = parse_cli_args_from(&argv(&[
            "persona", "proposals", "reject", "pp-2", "--reason", "too aggressive",
        ]))
        .expect("must parse");
        assert_eq!(
            with_reason.mode,
            CliMode::Persona(PersonaSubcommand::Proposals(
                ProposalsSubcommand::Reject {
                    proposal_id: "pp-2".into(),
                    reason: Some("too aggressive".into()),
                }
            ))
        );
    }

    #[test]
    fn proposals_without_subcommand_errors() {
        let err = parse_cli_args_from(&argv(&["persona", "proposals"]))
            .expect_err("must error");
        assert!(err.contains("requires a subcommand"), "{err}");
    }

    #[test]
    fn proposals_unknown_subcommand_errors() {
        let err = parse_cli_args_from(&argv(&[
            "persona", "proposals", "delete",
        ]))
        .expect_err("must error");
        assert!(err.contains("unrecognized"), "{err}");
    }

    #[test]
    fn persona_unknown_subcommand_is_an_error() {
        let err = parse_cli_args_from(&argv(&["persona", "delete"]))
            .expect_err("`persona delete` must error");
        assert!(
            err.contains("unrecognized persona subcommand"),
            "error: {err}"
        );
    }

    #[test]
    fn persona_show_rejects_extra_args() {
        let err = parse_cli_args_from(&argv(&["persona", "show", "--verbose"]))
            .expect_err("extra args must error");
        assert!(
            err.contains("does not accept additional arguments"),
            "error: {err}"
        );
    }

    // -----------------------------------------------------------------
    // Phase 64 — `aivyx identity <subcommand>` parser tests.
    // -----------------------------------------------------------------

    #[test]
    fn identity_export_parses_with_path() {
        let parsed =
            parse_cli_args_from(&argv(&["identity", "export", "/tmp/snap.json"]))
                .expect("`identity export /tmp/snap.json` must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Identity(IdentitySubcommand::Export {
                path: PathBuf::from("/tmp/snap.json"),
            })
        );
    }

    #[test]
    fn identity_without_subcommand_is_an_error() {
        let err = parse_cli_args_from(&argv(&["identity"]))
            .expect_err("`identity` without subcommand must error");
        assert!(err.contains("requires a subcommand"), "error: {err}");
    }

    #[test]
    fn identity_export_without_path_is_an_error() {
        let err = parse_cli_args_from(&argv(&["identity", "export"]))
            .expect_err("`identity export` without path must error");
        assert!(err.contains("requires a path"), "error: {err}");
    }

    #[test]
    fn identity_export_rejects_extra_args() {
        let err =
            parse_cli_args_from(&argv(&["identity", "export", "/tmp/x", "--verbose"]))
                .expect_err("extra args must error");
        assert!(err.contains("extra args"), "error: {err}");
    }

    #[test]
    fn identity_import_parses_with_path_no_force() {
        // Phase 65 — import is real now (no longer a deferral).
        let parsed = parse_cli_args_from(&argv(&["identity", "import", "/tmp/x.json"]))
            .expect("`identity import` must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Identity(IdentitySubcommand::Import {
                path: PathBuf::from("/tmp/x.json"),
                force: false,
            })
        );
    }

    #[test]
    fn identity_import_parses_with_force_flag() {
        let parsed =
            parse_cli_args_from(&argv(&["identity", "import", "/tmp/x.json", "--force"]))
                .expect("`identity import --force` must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Identity(IdentitySubcommand::Import {
                path: PathBuf::from("/tmp/x.json"),
                force: true,
            })
        );
    }

    #[test]
    fn identity_import_without_path_is_an_error() {
        let err = parse_cli_args_from(&argv(&["identity", "import"]))
            .expect_err("`identity import` without path must error");
        assert!(err.contains("requires a path"), "error: {err}");
    }

    #[test]
    fn identity_import_rejects_double_force() {
        let err = parse_cli_args_from(&argv(&[
            "identity", "import", "/tmp/x", "--force", "--force",
        ]))
        .expect_err("double --force must error");
        assert!(err.contains("more than once"), "error: {err}");
    }

    #[test]
    fn identity_import_rejects_unknown_trailing_arg() {
        let err = parse_cli_args_from(&argv(&[
            "identity", "import", "/tmp/x", "--bogus",
        ]))
        .expect_err("unknown trailing arg must error");
        assert!(err.contains("unexpected arg"), "error: {err}");
    }

    #[test]
    fn identity_unrecognized_subcommand_errors() {
        let err = parse_cli_args_from(&argv(&["identity", "wat", "/tmp/x"]))
            .expect_err("`identity wat` must error");
        assert!(
            err.contains("unrecognized identity subcommand"),
            "error: {err}"
        );
    }

    // -----------------------------------------------------------------
    // Phase 73 — `aivyx notify <subcommand>` parser tests.
    // -----------------------------------------------------------------

    #[test]
    fn notify_history_default_limit_no_target() {
        let parsed =
            parse_cli_args_from(&argv(&["notify", "history"])).expect("must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Notify(NotifySubcommand::History {
                target: None,
                limit: 100,
            })
        );
    }

    #[test]
    fn notify_history_with_target_and_limit_flags() {
        let parsed = parse_cli_args_from(&argv(&[
            "notify", "history", "--target", "phone", "--limit", "50",
        ]))
        .expect("must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Notify(NotifySubcommand::History {
                target: Some("phone".into()),
                limit: 50,
            })
        );
    }

    #[test]
    fn notify_history_zero_limit_errors() {
        let err = parse_cli_args_from(&argv(&[
            "notify", "history", "--limit", "0",
        ]))
        .expect_err("must error");
        assert!(err.contains("must be ≥ 1"), "{err}");
    }

    #[test]
    fn notify_history_non_numeric_limit_errors() {
        let err = parse_cli_args_from(&argv(&[
            "notify", "history", "--limit", "many",
        ]))
        .expect_err("must error");
        assert!(err.contains("positive integer"), "{err}");
    }

    #[test]
    fn notify_without_subcommand_errors() {
        let err = parse_cli_args_from(&argv(&["notify"]))
            .expect_err("must error");
        assert!(err.contains("requires a subcommand"), "{err}");
        assert!(err.contains("history"), "{err}");
    }

    #[test]
    fn notify_unknown_subcommand_errors() {
        let err = parse_cli_args_from(&argv(&["notify", "wat"]))
            .expect_err("must error");
        assert!(err.contains("unrecognized"), "{err}");
    }

    // -----------------------------------------------------------------
    // Phase 74 — `aivyx memory <subcommand>` parser tests.
    // -----------------------------------------------------------------

    #[test]
    fn memory_list_parses() {
        let parsed = parse_cli_args_from(&argv(&["memory", "list"]))
            .expect("must parse");
        assert_eq!(parsed.mode, CliMode::Memory(MemorySubcommand::List));
    }

    #[test]
    fn memory_list_with_args_errors() {
        let err = parse_cli_args_from(&argv(&["memory", "list", "extra"]))
            .expect_err("must error");
        assert!(err.contains("takes no arguments"), "{err}");
    }

    #[test]
    fn memory_show_parses_with_default_limit() {
        let parsed = parse_cli_args_from(&argv(&["memory", "show", "notes"]))
            .expect("must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Memory(MemorySubcommand::Show {
                topic: "notes".into(),
                limit: 32,
            })
        );
    }

    #[test]
    fn memory_show_parses_with_limit_flag() {
        let parsed = parse_cli_args_from(&argv(&[
            "memory", "show", "notes", "--limit", "5",
        ]))
        .expect("must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Memory(MemorySubcommand::Show {
                topic: "notes".into(),
                limit: 5,
            })
        );
    }

    #[test]
    fn memory_show_without_topic_errors() {
        let err = parse_cli_args_from(&argv(&["memory", "show"]))
            .expect_err("must error");
        assert!(err.contains("requires a topic"), "{err}");
    }

    #[test]
    fn memory_search_parses() {
        let parsed = parse_cli_args_from(&argv(&[
            "memory", "search", "foo", "--limit", "10",
        ]))
        .expect("must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Memory(MemorySubcommand::Search {
                query: "foo".into(),
                limit: 10,
                semantic: false,
            })
        );
    }

    #[test]
    fn memory_search_semantic_flag_parses() {
        // `--semantic` in either order relative to `--limit`.
        let parsed = parse_cli_args_from(&argv(&[
            "memory", "search", "foo", "--semantic", "--limit", "7",
        ]))
        .expect("must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Memory(MemorySubcommand::Search {
                query: "foo".into(),
                limit: 7,
                semantic: true,
            })
        );
        let parsed2 = parse_cli_args_from(&argv(&[
            "memory", "search", "bar", "--semantic",
        ]))
        .expect("must parse");
        assert_eq!(
            parsed2.mode,
            CliMode::Memory(MemorySubcommand::Search {
                query: "bar".into(),
                limit: 32,
                semantic: true,
            })
        );
    }

    #[test]
    fn memory_search_zero_limit_errors() {
        let err = parse_cli_args_from(&argv(&[
            "memory", "search", "foo", "--limit", "0",
        ]))
        .expect_err("must error");
        assert!(err.contains("must be ≥ 1"), "{err}");
    }

    #[test]
    fn memory_evict_parses_without_yes() {
        let parsed = parse_cli_args_from(&argv(&[
            "memory", "evict", "stale",
        ]))
        .expect("must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Memory(MemorySubcommand::Evict {
                topic: "stale".into(),
                yes: false,
            })
        );
    }

    #[test]
    fn memory_evict_parses_with_yes() {
        let parsed = parse_cli_args_from(&argv(&[
            "memory", "evict", "stale", "--yes",
        ]))
        .expect("must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Memory(MemorySubcommand::Evict {
                topic: "stale".into(),
                yes: true,
            })
        );
    }

    #[test]
    fn memory_without_subcommand_errors() {
        let err = parse_cli_args_from(&argv(&["memory"]))
            .expect_err("must error");
        assert!(err.contains("requires a subcommand"), "{err}");
    }

    #[test]
    fn memory_unknown_subcommand_errors() {
        let err = parse_cli_args_from(&argv(&["memory", "wat"]))
            .expect_err("must error");
        assert!(err.contains("unrecognized"), "{err}");
    }

    // -----------------------------------------------------------------
    // Phase 78 — `aivyx learning [--window <secs>]` parser tests.
    // -----------------------------------------------------------------

    #[test]
    fn learning_parses_without_window() {
        let parsed = parse_cli_args_from(&argv(&["learning"]))
            .expect("must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Learning { window_secs: None }
        );
    }

    #[test]
    fn learning_parses_with_window() {
        let parsed = parse_cli_args_from(&argv(&[
            "learning", "--window", "604800",
        ]))
        .expect("must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Learning {
                window_secs: Some(604_800),
            }
        );
    }

    #[test]
    fn learning_window_zero_errors() {
        let err = parse_cli_args_from(&argv(&[
            "learning", "--window", "0",
        ]))
        .expect_err("must error");
        assert!(err.contains("must be >= 1"), "{err}");
    }

    #[test]
    fn learning_unknown_arg_errors() {
        let err =
            parse_cli_args_from(&argv(&["learning", "--bogus"]))
                .expect_err("must error");
        assert!(err.contains("unrecognized"), "{err}");
    }
}

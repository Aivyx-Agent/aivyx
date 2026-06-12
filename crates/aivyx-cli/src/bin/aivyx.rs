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
//!   (`fs.read`, `fs.write`, `fs.metadata`, and — on Local channels
//!   only — `fs.delete`) are allowed to operate. Defaults to
//!   `$HOME/aivyx-sandbox`. Created at startup if it does not exist.
//!   The binary's capability set grants `fs.read:<root>/**`,
//!   `fs.write:<root>/**`, and `fs.metadata:<root>/**` for every
//!   channel, plus `fs.delete:<root>/**` on Local channels, so the
//!   LLM can exercise the tools without further wiring.
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

#[path = "aivyx_modules/audit_export.rs"]
mod audit_export;
#[path = "aivyx_modules/identity.rs"]
mod identity;
#[path = "aivyx_modules/mcp_recipes.rs"]
mod mcp_recipes;
#[path = "aivyx_modules/init.rs"]
mod init;
#[path = "aivyx_modules/identity_draft.rs"]
mod identity_draft;
#[path = "aivyx_modules/connect.rs"]
mod connect;
#[path = "aivyx_modules/init_templates.rs"]
mod init_templates;
#[path = "aivyx_modules/learning.rs"]
mod learning;
#[path = "aivyx_modules/loop_cli.rs"]
mod loop_cli;
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
#[path = "aivyx_modules/role.rs"]
mod role;
#[path = "aivyx_modules/tool_relevance.rs"]
mod tool_relevance;
#[path = "aivyx_modules/tools.rs"]
mod tools;
#[path = "aivyx_modules/team.rs"]
mod team;
#[path = "aivyx_modules/team_cli.rs"]
mod team_cli;
#[path = "aivyx_modules/cost.rs"]
mod cost;
#[path = "aivyx_modules/tool_init.rs"]
mod tool_init;
#[path = "aivyx_modules/toml_edit_apply.rs"]
mod toml_edit_apply;

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
use aivyx_channel::daemon_server::{run_daemon, ChannelFactory, DaemonConfig, ToolDescriptor};
use aivyx_channel::daemon_client::DaemonSession;
use aivyx_channel::{
    assemble_role_envelope, render_role_envelope, run_daemon_session_connected, run_session,
    ChannelKind, DaemonSessionConfig, LocalChannel, SessionConfig,
};
use aivyx_config::{AivyxConfig, FieldSource, LoadOptions, ToolAllowlist};
use aivyx_core::tools::role_switch::{ChildAgentFactory, RoleSwitchTool};
use aivyx_core::{
    Agent, AgentId, AuditHook, CancellationToken, ConcreteAgent, FsDeleteToolConfig,
    FsMetadataToolConfig, FsReadToolConfig, FsWriteToolConfig, LlmPlanner, LlmPlannerConfig,
    ShellExecToolConfig, Tool, ToolRegistry,
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

/// Optional `(tool, required capability scope)` pair returned by a
/// registration-time trust gate — `build_shell_exec_for_channel`
/// (Phase 11) and `build_fs_delete_for_channel` (Phase 100). Aliased
/// to satisfy clippy's `type_complexity` lint and because the pair
/// has a specific meaning — "the gated tool the agent gets for this
/// channel, plus the canonical scope that lets it run; `None` if the
/// channel's trust tier does not receive the tool at all."
type GatedToolRegistration = Option<(Arc<dyn Tool>, Scope)>;

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
) -> Result<GatedToolRegistration, String> {
    match channel_kind {
        // Phase 135 — Voice runs in-process on the
        // operator's machine and shares the Local
        // (Trusted) tier posture. shell.exec is
        // registered identically.
        ChannelKind::Local | ChannelKind::Voice => {
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
        // Phase 107 + Phase 108 — Discord and Slack share the
        // SemiTrusted tier posture with Telegram. None of the
        // three remote adapters receives `shell.exec` at
        // registration time. Symmetric behavior keeps the
        // SemiTrusted audit chain free of shell.exec mentions
        // across every remote adapter.
        ChannelKind::Telegram | ChannelKind::Discord | ChannelKind::Slack => Ok(None),
    }
}

/// Phase 100 task 5 — registration-time trust-tier gate for
/// `fs.delete`.
///
/// `fs.delete` is destructive. Like `shell.exec` — and unlike the
/// read-only `fs.read` / `fs.write` / `fs.metadata`, which every
/// channel receives — it is registered for `Local` (Trusted)
/// channels only. A `Telegram` (SemiTrusted) dispatch registry
/// never contains `fs.delete`, so a SemiTrusted audit chain never
/// mentions it, not even as a denial. See PHASE_100.md Q3.
///
/// Returns `Ok(Some((tool, scope)))` for `Local`, `Ok(None)` for
/// `Telegram`. The scope is `fs.delete:<canonical_root>/**`,
/// anchored to the canonicalized sandbox root so it lines up
/// exactly with `FsDeleteTool::required_scope`.
fn build_fs_delete_for_channel(
    channel_kind: ChannelKind,
    fs_root: &std::path::Path,
) -> Result<GatedToolRegistration, String> {
    match channel_kind {
        // Phase 135 — Voice shares the Local Trusted
        // tier; fs.delete is registered identically.
        ChannelKind::Local | ChannelKind::Voice => {
            let tool = FsDeleteToolConfig::new(fs_root.to_path_buf())
                .build()
                .map_err(|e| format!("failed to build fs.delete tool: {e}"))?;
            let canonical_root = tool.sandbox_root().to_path_buf();
            let scope = Scope::parse(&format!(
                "fs.delete:{}/**",
                canonical_root.display()
            ))
            .ok_or_else(|| {
                format!(
                    "canonical fs.delete sandbox scope not parseable from {canonical_root:?}"
                )
            })?;
            Ok(Some((Arc::new(tool) as Arc<dyn Tool>, scope)))
        }
        // Phase 107 + Phase 108 — Discord and Slack share the
        // SemiTrusted tier posture with Telegram for
        // destructive tool gating. A SemiTrusted dispatch
        // registry never contains `fs.delete` regardless of
        // which remote adapter is wired.
        ChannelKind::Telegram | ChannelKind::Discord | ChannelKind::Slack => Ok(None),
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

    // ---- Phase 182: guided credential onboarding ------------------------
    // Like init, `connect` needs only a minimal runtime — it writes a
    // per-tool-process config.toml and shells out to the service's
    // `auth init`. No config/store/API key.
    if let CliMode::Connect(ref service) = mode {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
        return rt.block_on(connect::run_connect(service.as_deref()));
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

    // ---- Phase 185: terminal UI (Chapter I #1) -------------------------
    // The TUI is a frontend client over the daemon IPC, exactly like
    // the REPL — it needs only a tokio runtime + the socket path; the
    // daemon owns the agent, provider, config, and audit, and is
    // auto-spawned if not already running. A multi-thread runtime is
    // used because the event loop drives a blocking key-poll on the
    // blocking pool concurrently with the daemon turn future. Opt-in;
    // the REPL stays the default and the non-TTY / scripting path.
    if mode == CliMode::Tui {
        let socket_path = default_socket_path()?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
        return rt.block_on(aivyx_tui::run(socket_path, role_override.clone()));
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
            ProfileSubcommand::ApplyHint { proposal_id, yes } => {
                // Phase 119 Task 4 — daemon IPC, hence a minimal
                // tokio runtime (matches the persona subcommand
                // dispatch pattern in this binary).
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("failed to start runtime: {e}"))?;
                runtime.block_on(profile::run_profile_apply_hint(
                    &proposal_id,
                    yes,
                ))
            }
        };
    }

    // ---- Phase 119 Task 5: role import (PRODUCT.md P9 + P13) ---------
    if let CliMode::Role(sub) = mode {
        return match sub {
            RoleSubcommand::Import {
                proposal_id,
                yes,
                force,
            } => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("failed to start runtime: {e}"))?;
                runtime.block_on(role::run_role_import(
                    &proposal_id,
                    yes,
                    force,
                ))
            }
        };
    }

    // ---- Phase 119 Task 6: tool-relevance dump (Phase 116 deferral) --
    if let CliMode::ToolRelevance(sub) = mode {
        return match sub {
            ToolRelevanceSubcommand::Dump { keyword_key_filter } => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("failed to start runtime: {e}"))?;
                runtime.block_on(tool_relevance::run_tool_relevance_dump(
                    keyword_key_filter.as_deref(),
                ))
            }
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
                PersonaSubcommand::List { filter } => {
                    let prefix_filter = match filter {
                        PersonaListFilter::All => persona::PersonaListPrefix::All,
                        PersonaListFilter::AutoOnly => {
                            persona::PersonaListPrefix::AutoOnly
                        }
                        PersonaListFilter::ManualOnly => {
                            persona::PersonaListPrefix::ManualOnly
                        }
                    };
                    persona::run_persona_list(prefix_filter).await
                }
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

    // Phase 173 — `aivyx loop <subcommand>`: autonomous loop
    // control. IPC-backed; same minimal-runtime shape as
    // `learning`.
    if let CliMode::Loop(sub) = mode {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
        return rt.block_on(async move {
            loop_cli::run_loop(sub).await
        });
    }

    // Phase 102 — `aivyx tools`: read-only tool-observability
    // view. IPC-backed; same daemon-query shape as `learning`.
    if let CliMode::Tools { window_secs } = mode {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
        return rt
            .block_on(async move { tools::run_tools(window_secs).await });
    }

    // Phase 103 — `aivyx tool init <path>`: scaffold a starter
    // Rust tool-process project at the target path. Pure sync fs
    // work, so it skips the tokio runtime the IPC subcommands need.
    if let CliMode::Tool(ToolSubcommand::Init { path, force }) = mode {
        return tool_init::run_tool_init(&path, force);
    }

    // Phase 106 — `aivyx mcp recipes [<name>]`: print the
    // curated MCP recipes catalog (or one recipe's worked
    // snippet). Pure stdout emission — no storage, no daemon,
    // no tokio runtime.
    if let CliMode::Mcp(McpSubcommand::Recipes { name }) = mode {
        return run_mcp_recipes(name.as_deref());
    }

    // Chapter J — `aivyx team roster`: render the default Nonagon. Pure
    // stdout, no storage/provider/daemon (like `mcp recipes`). `team run`
    // takes the run_async path below — it needs the live provider + audit.
    if let CliMode::Team(TeamSubcommand::Roster { config }) = &mode {
        return team::run_roster(config.as_deref());
    }

    // Chapter L (L.5b) — `aivyx team start|list|status|approve|reject`: the
    // daemon-run mission control surface. IPC-backed, same minimal-runtime
    // shape as `loop` / `tools` — no provider, no in-process assembly.
    if let CliMode::Team(sub) = &mode {
        if sub.is_daemon_verb() {
            let sub = sub.clone();
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
            return rt.block_on(async move { team_cli::run_team_daemon(sub).await });
        }
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
    // Phase 105 — `aivyx audit export` carries the same
    // cold-start posture as `--verify-only`: no session, no
    // sandbox, no API key required. The dispatch lands after
    // storage open below; the params are extracted here so the
    // load-options + sandbox-mkdir guards downstream can branch
    // off the same flag without rebuilding the match.
    let audit_export_params: Option<(Option<u64>, Option<usize>, Option<String>)> =
        match &mode {
            CliMode::Audit(AuditSubcommand::Export {
                from,
                limit,
                event_type,
            }) => Some((*from, *limit, event_type.clone())),
            _ => None,
        };
    let audit_export_mode = audit_export_params.is_some();
    // Chapter K — `aivyx cost` shares the same offline cold-start posture
    // (no session / sandbox / API key); it dispatches after storage open.
    let cost_today: Option<bool> = match &mode {
        CliMode::Cost { today } => Some(*today),
        _ => None,
    };
    let cost_mode = cost_today.is_some();

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
        // Phase 105 — `aivyx audit export` shares `--verify-only`'s
        // posture: cold-start storage open via passphrase, no
        // session opened, no provider call made. No API key
        // required, regardless of `--channel`.
        require_api_key: !verify_only && !print_role_mode && !audit_export_mode && !cost_mode,
        require_telegram_token: matches!(channel_kind, ChannelKind::Telegram) && !print_role_mode,
        // Phase 107 — mirrors the Telegram check for the
        // Discord adapter. `--print-role` does not open a
        // Discord connection regardless of `--channel`, so
        // the print path relaxes this just like Telegram.
        require_discord_token: matches!(channel_kind, ChannelKind::Discord) && !print_role_mode,
        // Phase 108 — same shape for the Slack adapter. Socket
        // Mode needs both bot_token + app_token; validate
        // surfaces whichever is missing as a `Missing` error.
        require_slack_tokens: matches!(channel_kind, ChannelKind::Slack) && !print_role_mode,
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
    // Phase 105 — audit-export shares the same skip: no fs sandbox
    // is touched by a read-only chain dump.
    if !verify_only && !audit_export_mode && !cost_mode {
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
    // Phase 173 — distinct HMAC key for the autonomous-loop
    // backlog chain. Domain-separated via the HKDF info bytes
    // (`loop-backlog`) so a chain-confusion attack across the
    // persona / proposal / backlog chains is structurally
    // rejected at MAC verification.
    let loop_backlog_chain_key: [u8; 32] = {
        let subkey = master_key
            .derive_subkey(b"loop-backlog")
            .map_err(|e| {
                format!("failed to derive loop backlog chain key: {e}")
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

        // Phase 105 — `aivyx audit export`. Shares the cold-start
        // storage open path with `--verify-only` (passphrase
        // required, no session, no daemon needed). Emits the chain
        // as JSONL on stdout; both `--from <seq>` and `--limit <N>`
        // are forwarded straight to
        // `PersistentAuditLog::entries_range`.
        if let Some((from, limit, event_type)) = audit_export_params {
            return run_audit_export(
                storage,
                audit_chain_key,
                from,
                limit,
                event_type,
            )
            .await;
        }

        // Chapter K — `aivyx cost`. Same cold-start posture: open the chain,
        // price its `LlmCost` events (with the operator's `[pricing]`
        // overrides over the built-in defaults), print the report, exit.
        if let Some(today) = cost_today {
            let pricing =
                aivyx_cost::Pricing::with_overrides(config.pricing.clone());
            return cost::run_cost(storage, audit_chain_key, today, pricing).await;
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
            loop_backlog_chain_key,
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
        } else if config.provider.value == aivyx_config::ProviderKind::LlamaCpp {
            // Phase 133 — llama-server default.
            eprintln!(
                "  base_url          = {:?} (default)",
                "http://localhost:8080",
            );
        } else if config.provider.value == aivyx_config::ProviderKind::Jan {
            // Phase 133 — Jan default.
            eprintln!(
                "  base_url          = {:?} (default)",
                "http://localhost:1337/v1",
            );
        }
    } else if config.provider.value.is_in_process() {
        // Phase 134 — MistralRs runs in-process; no base URL.
        // Surface the model path instead so the operator can
        // confirm at-a-glance which GGUF will be loaded.
        if let Some(path) = &config.mistralrs_options.model_path {
            eprintln!("  model_path        = {path:?} ([mistralrs])");
        }
    }
    eprintln!(
        "  model             = {:?} ({})",
        config.model.value,
        source_label(config.model.source),
    );
    // Phase 122 Task 6 — surface the resolved Ollama prompt
    // strategy so the operator can confirm at a glance whether
    // `structured_injection` is active, which family was
    // detected, and whether their `[ollama.prompt_strategies]`
    // override (if any) is being honored. Only shown when the
    // provider is Ollama — cloud providers handle their own
    // tool-catalog surface. Pure-string formatting lives in
    // `format_ollama_prompt_strategy_banner_line` so it can be
    // unit-tested without capturing stderr.
    if let Some(line) = format_ollama_prompt_strategy_banner_line(config) {
        eprintln!("{line}");
    }
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

/// Phase 122 Task 6 — Format the `ollama_prompt_strategy`
/// banner line for the operator-facing config banner.
///
/// Returns `None` when the provider is not Ollama (cloud
/// providers handle their own tool-catalog surface; no
/// strategy line to show).
///
/// Returns `Some(line)` when the provider is Ollama. The line
/// reports the resolved strategy label and a provenance hint:
/// - `family: <name>, default` — model detected; per-family
///   default applies (operator has no override for this family).
/// - `family: <name>, override` — model detected; operator has
///   an explicit `[ollama.prompt_strategies] <name> = "..."`
///   entry that's overriding the per-family default.
/// - `family: undetected` — model name doesn't parse to any
///   known Ollama family. Strategy is always `"none"` in this
///   case (operator-conservative).
fn format_ollama_prompt_strategy_banner_line(
    config: &AivyxConfig,
) -> Option<String> {
    if config.provider.value != aivyx_config::ProviderKind::Ollama {
        return None;
    }
    let family = aivyx_config::detect_model_family(&config.model.value);
    let strategy = aivyx_config::resolve_ollama_prompt_strategy(
        &config.model.value,
        &config.ollama_prompt_strategies,
    );
    let provenance = match &family {
        None => "family: undetected".to_string(),
        Some(fam) => {
            if config.ollama_prompt_strategies.contains_key(fam) {
                format!("family: {fam}, override")
            } else {
                format!("family: {fam}, default")
            }
        }
    };
    Some(format!(
        "  ollama_prompt_strategy = {:?} ({provenance})",
        strategy.label(),
    ))
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
    /// `aivyx connect [service]`: guided credential onboarding for
    /// the Google productivity tools (Phase 182). `None` lists the
    /// connectable services + status; `Some(service)` runs the
    /// guided OAuth flow.
    Connect(Option<String>),
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
    /// `aivyx tools [--window <secs>]`: Phase 102 read-only
    /// tool-observability view — every registered tool
    /// annotated with audit-derived call/outcome stats.
    /// IPC-backed. `window_secs = None` → the whole audit
    /// chain.
    Tools { window_secs: Option<u64> },
    /// `aivyx tool <subcommand>`: Phase 103 third-party-tool
    /// authoring helpers. Currently only `init <path>` — a
    /// scaffolder for a runnable Rust tool-process starter.
    Tool(ToolSubcommand),
    /// `aivyx audit <subcommand>`: Phase 105 read-only audit
    /// chain access. Currently only `export` — emit the
    /// chain as JSONL on stdout. Offline-only (cold-start
    /// storage open via the operator's passphrase) per Q3a.
    Audit(AuditSubcommand),
    /// `aivyx mcp <subcommand>`: Phase 106 curated-recipes
    /// catalog. Currently only `recipes [<name>]` — list or
    /// print MCP server recipes. Distinct from the
    /// pre-existing `aivyx mcp-server <name>` (Phase 46),
    /// which *runs* a bundled MCP server; `aivyx mcp
    /// recipes` is the *catalog* of recipes for the operator
    /// to copy into `aivyx.toml`.
    Mcp(McpSubcommand),
    /// `aivyx role <subcommand>`: Phase 119 — operator-side
    /// role-config commands. Currently only `import <id>` —
    /// applies an Approved `RoleDefinitionSuggestion`
    /// proposal to `aivyx.toml`'s `[roles.<name>]` section
    /// via the Task 3 atomic primitive and records the
    /// `AuditEvent::RoleDraftImported` event via daemon IPC.
    Role(RoleSubcommand),
    /// `aivyx tool-relevance <subcommand>`: Phase 119 Task 6 —
    /// closes the Phase 116 deferred inspection surface. Currently
    /// only `dump [--keyword-key <key>]` — renders the encrypted
    /// per-keyword-key relevance ledger as a human-readable table.
    /// IPC-backed.
    ToolRelevance(ToolRelevanceSubcommand),
    /// `aivyx loop <subcommand>`: Phase 173 — the autonomous
    /// loop (the Aivyx Ralph loop). IPC-backed; stocks the
    /// backlog + drives runs.
    Loop(LoopSubcommand),
    /// `aivyx tui [--role <name>]`: Phase 185 — the ratatui terminal
    /// UI. A frontend client over the local daemon IPC (auto-spawns
    /// the daemon if needed), exactly like the default REPL — only
    /// rendered into a real terminal application. Opt-in; the REPL
    /// stays the default and the non-TTY / scripting path. The role
    /// rides on the top-level `CliArgs::role` (parsed below); this
    /// variant carries no fields.
    Tui,
    /// `aivyx team <subcommand>`: Chapter J — the Nonagon. `roster`
    /// renders the default team (offline); `run "<mission>"` assembles
    /// the team in-process and hands the mission to the lead, whose
    /// specialist sub-turns land on the same HMAC chain.
    Team(TeamSubcommand),
    /// `aivyx cost [--today]`: Chapter K — the priced spend report.
    /// Offline (cold-start storage like `audit export`): scans the chain's
    /// `LlmCost` events, prices them, and prints a per-model breakdown.
    /// `--today` scopes to the last 24h.
    Cost { today: bool },
}

/// Chapter J — `aivyx team <subcommand>` variants. The optional
/// `--config <path.toml>` loads a **vertical pack's** customised `TeamConfig`
/// (e.g. the kitchen BOH Nonagon); omitted, the default 9-role Nonagon runs.
#[derive(Debug, PartialEq, Eq, Clone)]
enum TeamSubcommand {
    /// `aivyx team roster [--config <path>]` — render a team. Offline.
    Roster { config: Option<String> },
    /// `aivyx team run "<mission>" [--config <path>]` — run the lead.
    Run {
        mission: String,
        config: Option<String>,
    },
    /// Chapter L — `aivyx team start --plan <file.json> [--config <pack.toml>]`:
    /// submit an explicit mission plan to the daemon (durable, gate-pausable),
    /// optionally on a vertical-pack team.
    Start {
        plan_path: String,
        config: Option<String>,
    },
    /// Chapter L — `aivyx team start "<goal>" [--config <pack.toml>]`: the daemon
    /// decomposes the goal into a plan (one LLM planning call) and runs it,
    /// optionally on a vertical-pack team.
    StartGoal {
        goal: String,
        config: Option<String>,
    },
    /// Chapter L — `aivyx team list`: the daemon's mission feed.
    List,
    /// Chapter L — `aivyx team status [<id>]`: one mission's detail, or the
    /// whole feed when no id is given.
    Status { mission_id: Option<String> },
    /// Chapter L — `aivyx team approve <id> <step>`: pass a human gate.
    Approve { mission_id: String, step: String },
    /// Chapter L — `aivyx team reject <id> <step>`: reject a human gate.
    Reject { mission_id: String, step: String },
}

impl TeamSubcommand {
    /// Whether this verb talks to the running daemon's `TeamMissionService`
    /// (Chapter L) — as opposed to the offline `roster` / in-process `run`.
    fn is_daemon_verb(&self) -> bool {
        matches!(
            self,
            TeamSubcommand::Start { .. }
                | TeamSubcommand::StartGoal { .. }
                | TeamSubcommand::List
                | TeamSubcommand::Status { .. }
                | TeamSubcommand::Approve { .. }
                | TeamSubcommand::Reject { .. }
        )
    }
}

/// Phase 173 — `aivyx loop <subcommand>` variants.
#[derive(Debug, PartialEq, Eq, Clone)]
enum LoopSubcommand {
    /// `aivyx loop add <title> [--body <text>] [--priority <n>]`
    Add {
        title: String,
        body: String,
        priority: Option<u32>,
    },
    /// `aivyx loop list`
    List,
    /// `aivyx loop start [--max-iterations <n>]`
    Start { max_iterations: Option<u32> },
    /// `aivyx loop stop`
    Stop,
    /// `aivyx loop status`
    Status,
    /// `aivyx loop log [--limit <n>]`
    Log { limit: Option<u32> },
    /// `aivyx loop skip <story-id>`
    Skip { story_id: String },
}

/// Phase 119 Task 6 — `aivyx tool-relevance <subcommand>` variants.
#[derive(Debug, PartialEq, Eq, Clone)]
enum ToolRelevanceSubcommand {
    /// `aivyx tool-relevance dump [--keyword-key <key>]` — render
    /// the Phase 116 relevance ledger as a flat table. With
    /// `--keyword-key`, restricts to the single key.
    Dump { keyword_key_filter: Option<String> },
}

/// Phase 119 Task 5 — `aivyx role <subcommand>` variants.
#[derive(Debug, PartialEq, Eq, Clone)]
enum RoleSubcommand {
    /// `aivyx role import <proposal-id> [--yes] [--force]` —
    /// applies a Phase 118 `RoleDefinitionSuggestion`
    /// proposal to `aivyx.toml`. Refuses to overwrite an
    /// existing role of the same name without `--force`.
    Import {
        proposal_id: String,
        yes: bool,
        force: bool,
    },
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

/// Phase 113 — `aivyx persona list` filter discriminator.
/// `All` = unfiltered (pre-Phase-113 behaviour); `AutoOnly` =
/// only deltas whose `delta_id` starts with `pd-auto-` (the
/// Phase 112 auto-accept synthesized prefix); `ManualOnly` =
/// only deltas whose `delta_id` does NOT start with that
/// prefix. Flags are mutually exclusive at the parse step.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum PersonaListFilter {
    All,
    AutoOnly,
    ManualOnly,
}

/// Subcommand discriminator under [`CliMode::Persona`]. Phase 60.
#[derive(Debug, PartialEq, Eq, Clone)]
enum PersonaSubcommand {
    /// `aivyx persona show` — print the effective Persona snapshot.
    Show,
    /// `aivyx persona list [--auto-only | --manual-only]` — print
    /// every approved delta in chain order with id, category, op,
    /// and approval timestamp. Phase 113 — the optional filter
    /// scopes to auto-accepted vs operator-approved entries by
    /// matching the `pd-auto-` `delta_id` prefix the Phase 112
    /// auto-proposer synthesizes.
    List { filter: PersonaListFilter },
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
#[derive(Debug, PartialEq, Eq, Clone)]
enum ProfileSubcommand {
    /// `aivyx profile show` — print the current Profile to stdout
    /// in a labeled human-readable form. Reads `aivyx.toml` from
    /// disk per Q3(a) at sign-off.
    Show,
    /// `aivyx profile edit` — surgical `[profile]` section edit in
    /// `$EDITOR` per Q2(a) at sign-off (wired in Task 3).
    Edit,
    /// `aivyx profile apply-hint <id> [--yes]` — Phase 119 Task 4.
    /// Applies an Approved ProfileHint to `aivyx.toml`'s [profile]
    /// section via the Task 3 atomic primitive, then records the
    /// `AuditEvent::ProfileHintApplied` event via daemon IPC.
    ApplyHint { proposal_id: String, yes: bool },
}

/// Phase 103 — `aivyx tool` subcommand variants.
#[derive(Debug, PartialEq, Eq, Clone)]
enum ToolSubcommand {
    /// `aivyx tool init <path> [--force]` — scaffold a runnable
    /// Rust tool-process starter at `path`. Refuses to write into
    /// a non-empty directory unless `--force`.
    Init { path: PathBuf, force: bool },
}

/// Phase 106 — `aivyx mcp` subcommand variants. Distinct
/// from the Phase 46 `aivyx mcp-server <name>` runner — that
/// one *starts* a bundled MCP server on stdio; this one
/// catalogs the curated recipes operators paste into
/// `aivyx.toml`.
#[derive(Debug, PartialEq, Eq, Clone)]
enum McpSubcommand {
    /// `aivyx mcp recipes [<name>]` — bare form lists every
    /// recipe with a one-line description; named form prints
    /// the worked snippet for `<name>`.
    Recipes { name: Option<String> },
}

/// Phase 105 — `aivyx audit` subcommand variants.
#[derive(Debug, PartialEq, Eq, Clone)]
enum AuditSubcommand {
    /// `aivyx audit export [--from <seq>] [--limit <N>]
    /// [--event-type <kind>]` — emit the audit chain as JSONL
    /// on stdout. Read-only, offline-only (cold-start storage
    /// open via the operator's passphrase). `--from`/`--limit`
    /// map directly onto `PersistentAuditLog::entries_range(
    /// from, limit)`; missing `--from` means seq 0, missing
    /// `--limit` means no upper bound. Phase 113 — `--event-
    /// type <kind>` filters to a single `AuditEvent` variant
    /// label (e.g. `SkillAutoProposal`, `ToolCall`,
    /// `AutoNotifyDispatched`). `None` means "all events"
    /// (pre-Phase-113 behaviour).
    Export {
        from: Option<u64>,
        limit: Option<usize>,
        event_type: Option<String>,
    },
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

    // Check for `tui` — Phase 185. The ratatui terminal UI. Accepts
    // only an optional `--role <name>`; everything else is an error so
    // typos surface instead of being silently ignored.
    if !args.is_empty() && args[0] == "tui" {
        let mut role: Option<String> = None;
        let mut ti = 1;
        while ti < args.len() {
            match args[ti].as_str() {
                "--role" => {
                    let value = args.get(ti + 1).ok_or_else(|| {
                        "`--role` requires a value".to_string()
                    })?;
                    if value.trim().is_empty() {
                        return Err("`--role` requires a non-empty name".to_string());
                    }
                    role = Some(value.clone());
                    ti += 2;
                }
                other => {
                    return Err(format!(
                        "unrecognized argument after `tui`: `{other}`. \
                         `aivyx tui` supports: --role <name>"
                    ));
                }
            }
        }
        return Ok(CliArgs {
            mode: CliMode::Tui,
            channel: ChannelKind::Local,
            role,
            no_daemon: false,
            mcp_servers: Vec::new(),
            mcp_sse_servers: Vec::new(),
            provider: None,
            web_ui_port: None,
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

    // Phase 182 — `aivyx connect [service]`: guided credential
    // onboarding. No arg lists the connectable services; one arg
    // runs the guided OAuth flow for that service.
    if !args.is_empty() && args[0] == "connect" {
        let service = args.get(1).cloned();
        if let Some(s) = &service {
            if s.starts_with("--") {
                return Err(format!(
                    "`aivyx connect` takes a service name, not a flag \
                     (`{s}`). Run `aivyx connect` to list services."
                ));
            }
        }
        if args.len() > 2 {
            return Err(
                "`aivyx connect` takes at most one service name".into(),
            );
        }
        return Ok(CliArgs {
            mode: CliMode::Connect(service),
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
            mcp_servers: vec![],
            mcp_sse_servers: vec![],
            provider: None,
            web_ui_port: None,
        });
    }

    // Phase 173 — `aivyx loop <subcommand>`: stock the backlog +
    // drive autonomous runs. IPC-backed.
    if !args.is_empty() && args[0] == "loop" {
        let sub = args.get(1).map(|s| s.as_str()).unwrap_or("");
        let loop_sub = match sub {
            "add" => {
                let title = args.get(2).ok_or_else(|| {
                    "`aivyx loop add` requires a <title>".to_string()
                })?;
                if title.starts_with('-') {
                    return Err(
                        "`aivyx loop add` requires a <title> before any \
                         flags"
                            .to_string(),
                    );
                }
                let mut body = String::new();
                let mut priority: Option<u32> = None;
                let mut idx = 3;
                while idx < args.len() {
                    match args[idx].as_str() {
                        "--body" => {
                            body = args
                                .get(idx + 1)
                                .ok_or_else(|| {
                                    "`--body` requires a value".to_string()
                                })?
                                .clone();
                            idx += 2;
                        }
                        "--priority" => {
                            let v = args.get(idx + 1).ok_or_else(|| {
                                "`--priority` requires a value".to_string()
                            })?;
                            priority = Some(v.parse().map_err(|_| {
                                format!(
                                    "`--priority` expects a non-negative \
                                     integer, got `{v}`"
                                )
                            })?);
                            idx += 2;
                        }
                        other => {
                            return Err(format!(
                                "unrecognized argument to `aivyx loop \
                                 add`: `{other}`"
                            ));
                        }
                    }
                }
                LoopSubcommand::Add {
                    title: title.clone(),
                    body,
                    priority,
                }
            }
            "list" => LoopSubcommand::List,
            "stop" => LoopSubcommand::Stop,
            "status" => LoopSubcommand::Status,
            "skip" => {
                let story_id = args.get(2).ok_or_else(|| {
                    "`aivyx loop skip` requires a <story-id>".to_string()
                })?;
                if args.len() > 3 {
                    return Err(format!(
                        "unrecognized argument to `aivyx loop skip`: \
                         `{}`",
                        args[3]
                    ));
                }
                LoopSubcommand::Skip {
                    story_id: story_id.clone(),
                }
            }
            "log" => {
                let mut limit: Option<u32> = None;
                let mut idx = 2;
                while idx < args.len() {
                    match args[idx].as_str() {
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
                                    "`--limit` must be >= 1".to_string()
                                );
                            }
                            limit = Some(parsed);
                            idx += 2;
                        }
                        other => {
                            return Err(format!(
                                "unrecognized argument to `aivyx loop \
                                 log`: `{other}`"
                            ));
                        }
                    }
                }
                LoopSubcommand::Log { limit }
            }
            "start" => {
                let mut max_iterations: Option<u32> = None;
                let mut idx = 2;
                while idx < args.len() {
                    match args[idx].as_str() {
                        "--max-iterations" => {
                            let v = args.get(idx + 1).ok_or_else(|| {
                                "`--max-iterations` requires a value"
                                    .to_string()
                            })?;
                            let parsed: u32 = v.parse().map_err(|_| {
                                format!(
                                    "`--max-iterations` expects a positive \
                                     integer, got `{v}`"
                                )
                            })?;
                            if parsed == 0 {
                                return Err(
                                    "`--max-iterations` must be >= 1"
                                        .to_string(),
                                );
                            }
                            max_iterations = Some(parsed);
                            idx += 2;
                        }
                        other => {
                            return Err(format!(
                                "unrecognized argument to `aivyx loop \
                                 start`: `{other}`"
                            ));
                        }
                    }
                }
                LoopSubcommand::Start { max_iterations }
            }
            "" => {
                return Err(
                    "`aivyx loop` requires a subcommand: add | list | \
                     skip | start | status | stop | log"
                        .to_string(),
                );
            }
            other => {
                return Err(format!(
                    "unknown `aivyx loop` subcommand `{other}` \
                     (expected: add | list | skip | start | status | \
                     stop | log)"
                ));
            }
        };
        return Ok(CliArgs {
            mode: CliMode::Loop(loop_sub),
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
            mcp_servers: vec![],
            mcp_sse_servers: vec![],
            provider: None,
            web_ui_port: None,
        });
    }

    // Chapter J — `aivyx team <subcommand>`: roster (offline) | run "<mission>".
    if !args.is_empty() && args[0] == "team" {
        let sub = args.get(1).map(|s| s.as_str()).unwrap_or("");
        // Shared `--config <path>` parser over a tail of args.
        let parse_config = |tail: &[String], cmd: &str| -> Result<Option<String>, String> {
            let mut config: Option<String> = None;
            let mut idx = 0;
            while idx < tail.len() {
                match tail[idx].as_str() {
                    "--config" => {
                        let v = tail.get(idx + 1).ok_or_else(|| {
                            "`--config` requires a path to a team TOML".to_string()
                        })?;
                        config = Some(v.clone());
                        idx += 2;
                    }
                    other => {
                        return Err(format!(
                            "unrecognized argument to `aivyx team {cmd}`: `{other}`"
                        ));
                    }
                }
            }
            Ok(config)
        };
        let team_sub = match sub {
            "roster" => TeamSubcommand::Roster {
                config: parse_config(args.get(2..).unwrap_or(&[]), "roster")?,
            },
            "run" => {
                let mission = args.get(2).ok_or_else(|| {
                    "`aivyx team run` requires a \"<mission>\" argument".to_string()
                })?;
                if mission.starts_with('-') {
                    return Err(
                        "`aivyx team run` expects the mission text before any flags".to_string(),
                    );
                }
                TeamSubcommand::Run {
                    mission: mission.clone(),
                    config: parse_config(args.get(3..).unwrap_or(&[]), "run")?,
                }
            }
            "start" => {
                // `aivyx team start "<goal>" [--config <pack.toml>]` — daemon
                // decomposes the goal — or `--plan <file.json>` — explicit plan.
                let tail = args.get(2..).unwrap_or(&[]);
                let mut plan_path: Option<String> = None;
                let mut config: Option<String> = None;
                let mut goal: Option<String> = None;
                let mut idx = 0;
                while idx < tail.len() {
                    match tail[idx].as_str() {
                        "--plan" => {
                            plan_path = Some(tail.get(idx + 1).ok_or_else(|| {
                                "`--plan` requires a path to a plan JSON file".to_string()
                            })?.clone());
                            idx += 2;
                        }
                        "--config" => {
                            config = Some(tail.get(idx + 1).ok_or_else(|| {
                                "`--config` requires a path to a team TOML".to_string()
                            })?.clone());
                            idx += 2;
                        }
                        other if other.starts_with('-') => {
                            return Err(format!(
                                "unrecognized argument to `aivyx team start`: `{other}`"
                            ));
                        }
                        other => {
                            if goal.is_some() {
                                return Err(
                                    "`aivyx team start \"<goal>\"` takes a single quoted goal"
                                        .to_string(),
                                );
                            }
                            goal = Some(other.to_string());
                            idx += 1;
                        }
                    }
                }
                match (plan_path, goal) {
                    (Some(_), Some(_)) => {
                        return Err(
                            "`aivyx team start` takes either a \"<goal>\" or `--plan`, \
                             not both"
                                .to_string(),
                        );
                    }
                    (Some(plan_path), None) => TeamSubcommand::Start { plan_path, config },
                    (None, Some(goal)) => TeamSubcommand::StartGoal { goal, config },
                    (None, None) => {
                        return Err(
                            "`aivyx team start` requires a \"<goal>\" or `--plan <file.json>`"
                                .to_string(),
                        );
                    }
                }
            }
            "list" => {
                if args.len() > 2 {
                    return Err(format!(
                        "`aivyx team list` takes no arguments (got `{}`)",
                        args[2]
                    ));
                }
                TeamSubcommand::List
            }
            "status" => TeamSubcommand::Status {
                mission_id: args.get(2).cloned(),
            },
            "approve" | "reject" => {
                let mission_id = args.get(2).cloned().ok_or_else(|| {
                    format!("`aivyx team {sub}` requires <mission-id> <step>")
                })?;
                let step = args.get(3).cloned().ok_or_else(|| {
                    format!("`aivyx team {sub}` requires a <step> argument")
                })?;
                if sub == "approve" {
                    TeamSubcommand::Approve { mission_id, step }
                } else {
                    TeamSubcommand::Reject { mission_id, step }
                }
            }
            "" => {
                return Err(
                    "`aivyx team` requires a subcommand: roster | run | start | \
                     list | status | approve | reject"
                        .to_string(),
                );
            }
            other => {
                return Err(format!(
                    "unknown `aivyx team` subcommand `{other}` (expected: roster | \
                     run | start | list | status | approve | reject)"
                ));
            }
        };
        return Ok(CliArgs {
            mode: CliMode::Team(team_sub),
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
            mcp_servers: vec![],
            mcp_sse_servers: vec![],
            provider: None,
            web_ui_port: None,
        });
    }

    // Chapter K — `aivyx cost [--today]`: the priced spend report (offline).
    if !args.is_empty() && args[0] == "cost" {
        let mut today = false;
        for arg in &args[1..] {
            match arg.as_str() {
                "--today" => today = true,
                other => {
                    return Err(format!("unrecognized argument to `aivyx cost`: `{other}`"));
                }
            }
        }
        return Ok(CliArgs {
            mode: CliMode::Cost { today },
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
            mcp_servers: vec![],
            mcp_sse_servers: vec![],
            provider: None,
            web_ui_port: None,
        });
    }

    // Phase 102 — `aivyx tools [--window <secs>]`. Same `--window`
    // grammar as `aivyx learning`.
    if !args.is_empty() && args[0] == "tools" {
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
                        "unrecognized argument to `aivyx tools`: \
                         `{other}`"
                    ));
                }
            }
        }
        return Ok(CliArgs {
            mode: CliMode::Tools { window_secs },
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
            mcp_servers: vec![],
            mcp_sse_servers: vec![],
            provider: None,
            web_ui_port: None,
        });
    }

    // Phase 103 — `aivyx tool <subcommand>`. The first sub-
    // subcommand is `init <path> [--force]`.
    if !args.is_empty() && args[0] == "tool" {
        let sub = args.get(1).ok_or_else(|| {
            "`aivyx tool` requires a subcommand. Supported: init"
                .to_string()
        })?;
        match sub.as_str() {
            "init" => {
                let path_arg = args.get(2).ok_or_else(|| {
                    "`aivyx tool init` requires a target path".to_string()
                })?;
                if path_arg.starts_with("--") {
                    return Err(format!(
                        "`aivyx tool init` requires a target path \
                         (got flag `{path_arg}` where a path was expected)"
                    ));
                }
                let path = PathBuf::from(path_arg);
                let mut force = false;
                for extra in &args[3..] {
                    match extra.as_str() {
                        "--force" => force = true,
                        other => {
                            return Err(format!(
                                "unrecognized argument to `aivyx tool init`: \
                                 `{other}`"
                            ));
                        }
                    }
                }
                return Ok(CliArgs {
                    mode: CliMode::Tool(ToolSubcommand::Init { path, force }),
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
                    "unrecognized `aivyx tool` subcommand: `{other}`. \
                     Supported: init"
                ));
            }
        }
    }

    // Phase 105 — `aivyx audit <subcommand>`. The first sub-
    // subcommand is `export [--from <seq>] [--limit <N>]`. Both
    // flags are optional and map onto the existing
    // `PersistentAuditLog::entries_range(from, limit)` reader;
    // missing `--from` means seq 0, missing `--limit` means no
    // upper bound.
    if !args.is_empty() && args[0] == "audit" {
        let sub = args.get(1).ok_or_else(|| {
            "`aivyx audit` requires a subcommand. Supported: export"
                .to_string()
        })?;
        match sub.as_str() {
            "export" => {
                let mut from: Option<u64> = None;
                let mut limit: Option<usize> = None;
                let mut event_type: Option<String> = None;
                let mut i = 2;
                while i < args.len() {
                    match args[i].as_str() {
                        "--from" => {
                            let val = args.get(i + 1).ok_or_else(|| {
                                "`--from` requires a sequence number"
                                    .to_string()
                            })?;
                            let parsed: u64 = val.parse().map_err(|e| {
                                format!(
                                    "invalid `--from` value `{val}`: \
                                     expected a non-negative integer ({e})"
                                )
                            })?;
                            from = Some(parsed);
                            i += 2;
                        }
                        "--limit" => {
                            let val = args.get(i + 1).ok_or_else(|| {
                                "`--limit` requires an integer".to_string()
                            })?;
                            let parsed: usize = val.parse().map_err(|e| {
                                format!(
                                    "invalid `--limit` value `{val}`: \
                                     expected a positive integer ({e})"
                                )
                            })?;
                            if parsed == 0 {
                                return Err(
                                    "`--limit 0` would emit nothing — \
                                     omit `--limit` for an unbounded export"
                                        .to_string(),
                                );
                            }
                            limit = Some(parsed);
                            i += 2;
                        }
                        "--event-type" => {
                            // Phase 113 — operator-side filter on the
                            // `AuditEvent` variant label. Validated
                            // against the known set so a typo errors
                            // out before opening storage.
                            let val = args.get(i + 1).ok_or_else(|| {
                                "`--event-type` requires a variant name (e.g. \
                                 SkillAutoProposal)"
                                    .to_string()
                            })?;
                            const KNOWN: &[&str] = &[
                                "ToolCall",
                                "ScopeDenied",
                                "TurnStarted",
                                "TurnEnded",
                                "MemoryAccess",
                                "AutoNotifyDispatched",
                                "SkillAutoProposal",
                                "SkillInvocation",
                            ];
                            if !KNOWN.contains(&val.as_str()) {
                                return Err(format!(
                                    "unrecognized `--event-type` value `{val}`. \
                                     Supported: {}",
                                    KNOWN.join(", ")
                                ));
                            }
                            event_type = Some(val.clone());
                            i += 2;
                        }
                        other => {
                            return Err(format!(
                                "unrecognized argument to `aivyx audit export`: \
                                 `{other}`"
                            ));
                        }
                    }
                }
                return Ok(CliArgs {
                    mode: CliMode::Audit(AuditSubcommand::Export {
                        from,
                        limit,
                        event_type,
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
                    "unrecognized `aivyx audit` subcommand: `{other}`. \
                     Supported: export"
                ));
            }
        }
    }

    // Phase 106 — `aivyx mcp <subcommand>`. Distinct from the
    // Phase 46 `aivyx mcp-server <name>` runner ("mcp-server"
    // is one token, "mcp recipes" is two); the namespacing
    // matches the project pattern of one subcommand tree per
    // operator-facing surface.
    if !args.is_empty() && args[0] == "mcp" {
        let sub = args.get(1).ok_or_else(|| {
            "`aivyx mcp` requires a subcommand. Supported: recipes"
                .to_string()
        })?;
        match sub.as_str() {
            "recipes" => {
                // Optional positional name. Anything starting
                // with `--` is rejected loudly so a future
                // flag isn't silently consumed as a recipe
                // name.
                let name = match args.get(2) {
                    Some(n) if n.starts_with("--") => {
                        return Err(format!(
                            "unrecognized argument to `aivyx mcp recipes`: \
                             `{n}` (no flags are defined yet)"
                        ));
                    }
                    Some(n) => Some(n.clone()),
                    None => None,
                };
                if let Some(extra) = args.get(3) {
                    return Err(format!(
                        "unrecognized extra argument to \
                         `aivyx mcp recipes`: `{extra}`"
                    ));
                }
                return Ok(CliArgs {
                    mode: CliMode::Mcp(McpSubcommand::Recipes { name }),
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
                    "unrecognized `aivyx mcp` subcommand: `{other}`. \
                     Supported: recipes"
                ));
            }
        }
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
                // Phase 113 — `--auto-only` / `--manual-only` flags
                // (mutually exclusive). Anything else past `list` is
                // rejected.
                let mut filter = PersonaListFilter::All;
                let mut seen_auto = false;
                let mut seen_manual = false;
                for arg in &args[2..] {
                    match arg.as_str() {
                        "--auto-only" => {
                            seen_auto = true;
                            filter = PersonaListFilter::AutoOnly;
                        }
                        "--manual-only" => {
                            seen_manual = true;
                            filter = PersonaListFilter::ManualOnly;
                        }
                        other => {
                            return Err(format!(
                                "`aivyx persona list` unrecognized argument: `{other}`. \
                                 Supported flags: --auto-only, --manual-only."
                            ));
                        }
                    }
                }
                if seen_auto && seen_manual {
                    return Err(
                        "`aivyx persona list` --auto-only and --manual-only \
                         are mutually exclusive."
                            .into(),
                    );
                }
                PersonaSubcommand::List { filter }
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
            "`aivyx profile` requires a subcommand. Supported: show, edit, apply-hint <id>"
                .to_string()
        })?;
        let subcommand = match sub.as_str() {
            "show" => {
                if args.len() > 2 {
                    return Err(format!(
                        "`aivyx profile show` does not accept additional arguments. \
                         Got: `{}`",
                        args[2..].join(" ")
                    ));
                }
                ProfileSubcommand::Show
            }
            "edit" => {
                if args.len() > 2 {
                    return Err(format!(
                        "`aivyx profile edit` does not accept additional arguments. \
                         Got: `{}`",
                        args[2..].join(" ")
                    ));
                }
                ProfileSubcommand::Edit
            }
            "apply-hint" => {
                // Phase 119 Task 4 — `apply-hint <proposal-id> [--yes]`.
                let mut proposal_id: Option<String> = None;
                let mut yes = false;
                for arg in args[2..].iter() {
                    if arg == "--yes" || arg == "-y" {
                        yes = true;
                    } else if arg.starts_with('-') {
                        return Err(format!(
                            "unrecognized flag for `aivyx profile apply-hint`: `{arg}`. \
                             Supported flag: --yes"
                        ));
                    } else if proposal_id.is_none() {
                        proposal_id = Some(arg.clone());
                    } else {
                        return Err(format!(
                            "`aivyx profile apply-hint` takes exactly one proposal id. \
                             Got extra argument: `{arg}`"
                        ));
                    }
                }
                let id = proposal_id.ok_or_else(|| {
                    "`aivyx profile apply-hint` requires a proposal id. \
                     Usage: `aivyx profile apply-hint <proposal-id> [--yes]`"
                        .to_string()
                })?;
                ProfileSubcommand::ApplyHint {
                    proposal_id: id,
                    yes,
                }
            }
            other => {
                return Err(format!(
                    "unrecognized profile subcommand: `{other}`. \
                     Supported: profile show, profile edit, \
                     profile apply-hint <id>"
                ));
            }
        };
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

    // Phase 119 Task 5 — `aivyx role <subcommand>`.
    if !args.is_empty() && args[0] == "role" {
        let sub = args.get(1).ok_or_else(|| {
            "`aivyx role` requires a subcommand. Supported: import <id>"
                .to_string()
        })?;
        let subcommand = match sub.as_str() {
            "import" => {
                let mut proposal_id: Option<String> = None;
                let mut yes = false;
                let mut force = false;
                for arg in args[2..].iter() {
                    if arg == "--yes" || arg == "-y" {
                        yes = true;
                    } else if arg == "--force" {
                        force = true;
                    } else if arg.starts_with('-') {
                        return Err(format!(
                            "unrecognized flag for `aivyx role import`: `{arg}`. \
                             Supported flags: --yes, --force"
                        ));
                    } else if proposal_id.is_none() {
                        proposal_id = Some(arg.clone());
                    } else {
                        return Err(format!(
                            "`aivyx role import` takes exactly one proposal id. \
                             Got extra argument: `{arg}`"
                        ));
                    }
                }
                let id = proposal_id.ok_or_else(|| {
                    "`aivyx role import` requires a proposal id. \
                     Usage: `aivyx role import <proposal-id> [--yes] [--force]`"
                        .to_string()
                })?;
                RoleSubcommand::Import {
                    proposal_id: id,
                    yes,
                    force,
                }
            }
            other => {
                return Err(format!(
                    "unrecognized role subcommand: `{other}`. \
                     Supported: role import <id>"
                ));
            }
        };
        return Ok(CliArgs {
            mode: CliMode::Role(subcommand),
            channel: ChannelKind::Local,
            role: None,
            no_daemon: false,
            mcp_servers: Vec::new(),
            mcp_sse_servers: Vec::new(),
            provider: None,
            web_ui_port: None,
        });
    }

    // Phase 119 Task 6 — `aivyx tool-relevance <subcommand>`.
    if !args.is_empty() && args[0] == "tool-relevance" {
        let sub = args.get(1).ok_or_else(|| {
            "`aivyx tool-relevance` requires a subcommand. \
             Supported: dump [--keyword-key <key>]"
                .to_string()
        })?;
        let subcommand = match sub.as_str() {
            "dump" => {
                let mut keyword_key_filter: Option<String> = None;
                let mut i = 2;
                while i < args.len() {
                    let arg = &args[i];
                    if arg == "--keyword-key" {
                        let value = args.get(i + 1).ok_or_else(|| {
                            "`--keyword-key` requires a value. \
                             Usage: `aivyx tool-relevance dump --keyword-key <key>`"
                                .to_string()
                        })?;
                        keyword_key_filter = Some(value.clone());
                        i += 2;
                    } else if arg.starts_with('-') {
                        return Err(format!(
                            "unrecognized flag for `aivyx tool-relevance dump`: \
                             `{arg}`. Supported flag: --keyword-key <key>"
                        ));
                    } else {
                        return Err(format!(
                            "`aivyx tool-relevance dump` does not accept \
                             positional arguments. Got: `{arg}`"
                        ));
                    }
                }
                ToolRelevanceSubcommand::Dump { keyword_key_filter }
            }
            other => {
                return Err(format!(
                    "unrecognized tool-relevance subcommand: `{other}`. \
                     Supported: tool-relevance dump"
                ));
            }
        };
        return Ok(CliArgs {
            mode: CliMode::ToolRelevance(subcommand),
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
                    "`--channel` requires a value: `local`, `telegram`, `discord`, `slack`, or `voice`"
                        .to_string()
                })?;
                channel = match value.as_str() {
                    "local" => ChannelKind::Local,
                    "telegram" => ChannelKind::Telegram,
                    // Phase 107 — Discord adapter parse arm.
                    "discord" => ChannelKind::Discord,
                    // Phase 108 — Slack adapter parse arm.
                    "slack" => ChannelKind::Slack,
                    // Phase 135 — Voice channel parse arm.
                    "voice" => ChannelKind::Voice,
                    other => {
                        return Err(format!(
                            "unrecognized channel `{other}`. \
                             Supported: local, telegram, discord, slack, voice"
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
                        "`--provider` requires a value: `anthropic`, `openai`, `ollama`, `llamacpp`, or `jan`"
                            .to_string()
                    })?;
                cli_provider = Some(match value.as_str() {
                    "anthropic" => ProviderKind::Anthropic,
                    "openai" => ProviderKind::OpenAi,
                    "ollama" => ProviderKind::Ollama,
                    // Phase 133 — accept the same aliases as the
                    // serde alias attribute on the enum.
                    "llamacpp" | "llama-cpp" | "llama_cpp" => ProviderKind::LlamaCpp,
                    "jan" => ProviderKind::Jan,
                    // Phase 134 — embedded mistralrs.
                    "mistralrs" | "mistral-rs" | "mistral_rs" => ProviderKind::MistralRs,
                    other => {
                        return Err(format!(
                            "unrecognized provider `{other}`. \
                             Supported: anthropic, openai, ollama, llamacpp, jan, mistralrs"
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
                     Supported: --verify-only, --channel <local|telegram|discord|slack>, --role <name>, --print-role <name>, --no-daemon, --provider <anthropic|openai|ollama>, --mcp-server <name:command[:args]>, daemon run|status|stop"
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

/// Phase 106 — print the curated MCP recipes catalog or a
/// single recipe's worked snippet. Pure stdout emission — no
/// storage, no daemon, no tokio runtime. Lookup failures
/// surface as `Err(String)` so `main`'s outer `match` maps
/// them to `ExitCode::FAILURE` with the candidate list on
/// stderr.
fn run_mcp_recipes(name: Option<&str>) -> Result<(), String> {
    match name {
        Some(n) => {
            let snippet = mcp_recipes::render_recipe(n).map_err(|e| e.to_string())?;
            print!("{snippet}");
            Ok(())
        }
        None => {
            print!("{}", mcp_recipes::render_listing());
            Ok(())
        }
    }
}

/// Phase 105 — drive the JSONL audit-chain emitter against
/// stdout. Mirrors [`run_verify_only`]'s no-session,
/// cold-start posture: opens the chain via the supplied
/// storage + audit key, streams entries, exits. Errors land in
/// `main`'s outer `match` and map to `ExitCode::FAILURE`.
async fn run_audit_export(
    storage: Arc<dyn Storage>,
    audit_chain_key: [u8; 32],
    from: Option<u64>,
    limit: Option<usize>,
    event_type: Option<String>,
) -> Result<(), String> {
    // `BufWriter` here keeps stdout-flushing cost out of the
    // per-line loop; the inner `export_chain` calls `flush`
    // once at the end. `stdout().lock()` is the recommended
    // pattern for high-throughput writes against the global
    // handle.
    let stdout = io::stdout();
    let mut writer = std::io::BufWriter::new(stdout.lock());
    let emitted = audit_export::export_chain(
        storage,
        audit_chain_key,
        from,
        limit,
        event_type.as_deref(),
        &mut writer,
    )
    .await?;
    // Drop the writer before printing the summary so its
    // buffered bytes hit stdout in chain-emission order. The
    // summary itself goes to stderr — the JSONL stream is what
    // a pipe consumer wants on stdout.
    drop(writer);
    eprintln!("audit: exported {emitted} entries");
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
    loop_backlog_chain_key: [u8; 32],
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
        // Chapter N — the resolved access level already shaped `fs_root`
        // (the reach lever) in `aivyx-config`. N.2 consumes these here to
        // assemble the operator grant set + the confirm-first posture.
        access_level: _access_level,
        confirm_destructive: _confirm_destructive,
        storage_path: _,
        memory_max_per_topic,
        passphrase: _,
        telegram,
        // Phase 107 — `discord` config consumed by the
        // `ChannelKind::Discord` dispatch arm below; carries
        // the bot token through to `run_discord_session`.
        discord,
        // Phase 108 — `slack` config consumed by the
        // `ChannelKind::Slack` dispatch arm below; carries
        // the bot + app tokens through to
        // `run_slack_session`.
        slack,
        // Phase 109 — `[git]` config consumed at the tool-
        // registration site below. `None` means no git tools
        // get registered (zero-config posture).
        git: config_git,
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
        // recall provider's cluster-aware expansion below.
        recall_cluster: config_recall_cluster,
        // Phase 87 — `[persona_consolidation]` config. Wired
        // into the daemon's reflection-cron consolidation
        // pass via DaemonConfig below.
        persona_consolidation: config_persona_consolidation,
        // Phase 173 — `[loop]` config (the Aivyx Ralph loop).
        // Arms the autonomous-loop driver + sets the iteration
        // cap and default story priority. Wired through
        // DaemonConfig below.
        loop_config: config_loop,
        // Phase 172 — `[correction_consolidation]` config.
        // Wired into the daemon's reflection-cron correction-
        // consolidation pass via DaemonConfig below.
        correction_consolidation: config_correction_consolidation,
        // Phase 91 — `[recall_judgment]` config. Bound here;
        // Task 4 of Phase 91 wires it through DaemonConfig
        // to the reflection-cron LLM-judged recall pass.
        recall_judgment: _config_recall_judgment,
        // Phase 178 — `[correction_judgment]` config. Wired
        // through DaemonConfig to the reflection-cron correction
        // fold (only Rework folds when armed).
        correction_judgment: config_correction_judgment,
        // Phase 179 — `[correction_signal]` config (tool
        // correction attribution toggle).
        correction_signal: config_correction_signal,
        // Phase 93 — `[recall_feedback]` config. Wired
        // through DaemonConfig to thread the per-hit
        // judgment-signal switch into `correlate_detailed`
        // (both the reflection-cron actuator + the
        // GetLearningInsights surface).
        recall_feedback: config_recall_feedback,
        // Phase 113 — `[skills.auto_propose]` loaded config.
        // Phase 114 — preserved as the alias path. The bin's
        // construction (below) prefers `config_persona_auto_propose`
        // if present, otherwise falls back to this Phase 113
        // single-config shape.
        skill_auto_propose: config_skill_auto_propose,
        // Phase 114 — `[persona.auto_propose]` loaded config.
        // When `Some`, takes precedence over the Phase 113 alias.
        // Used by the bin's `SkillAutoProposerContext`
        // construction to populate the per-category config in
        // the runtime auto-proposer.
        persona_auto_propose: config_persona_auto_propose,
        // Phase 116 — `[tool_relevance]` loaded config. When
        // `Some(_).enabled == true` the bin constructs a
        // PersistentToolRelevanceLedger handle from the
        // storage domain and threads it through DaemonConfig.
        tool_relevance: config_tool_relevance,
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
        sandbox_default_backend: config_sandbox_default_backend,
        reminders_check_interval_secs: config_reminders_check_interval_secs,
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
        // Phase 89 — opt-in topic canonicalization at the
        // `Memory` trait's topic-string boundary. When
        // `true`, the memory below is wrapped in a
        // `CanonicalizingMemory` delegate.
        memory_canonicalize_topics: config_memory_canonicalize_topics,
        // Phase 121 — `[ollama]` operator-configured generation
        // options. Converted to `aivyx_llm::ollama::OllamaOptions`
        // at provider-construction time below for
        // `ProviderKind::Ollama`.
        ollama_options: config_ollama_options,
        // Phase 122 Task 5 — operator-overridable per-family
        // prompt-strategy map. Consumed at the Ollama-strategy
        // resolution site below via
        // `resolve_ollama_prompt_strategy(&model, &..)`.
        ollama_prompt_strategies: config_ollama_prompt_strategies,
        // Chapter K — `[pricing]` overrides feed both the `aivyx cost`
        // dispatch in `run()` (before `run_async`) and, since K.4.2, the
        // loop's dollar cap via the `pricing` table on `DaemonConfig` below.
        pricing: config_pricing,
        // Chapter K (K.4.2) — `[budget]` dollar caps. Parsed + validated by
        // the loader; consumed by the turn-loop budget gate built below.
        budget: config_budget,
        // Phase 120 — operator-configurable threshold for the
        // planner's tool-name fuzzy-match recovery. Threaded
        // into `LlmPlannerConfig` below.
        tool_name_auto_correct_threshold: config_tool_name_auto_correct_threshold,
        // Phase 134 — [mistralrs] embedded-provider config. Only
        // referenced when `provider = "mistralrs"`; for any other
        // provider the field is bound and ignored.
        mistralrs_options: config_mistralrs_options,
        // Phase 135 — [voice] section. Bound here so the
        // ChannelKind::Voice dispatch arm reads the operator's
        // ASR + TTS paths.
        voice_options: config_voice_options,
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
    // Phase 173 — the autonomous-loop backlog. Always opened
    // (zero-config, like memory): the `aivyx loop add` CLI + the
    // loop tools need it even when no run is active. The driver
    // (armed only by `[loop]`) and both loop tools share this
    // one `Arc` so they see a single HMAC-chained backlog.
    let loop_backlog: Arc<
        aivyx_channel::loop_backlog::PersistentLoopBacklog,
    > = match aivyx_channel::loop_backlog::PersistentLoopBacklog::open(
        storage.domain(KeyDomain::LoopBacklog),
        loop_backlog_chain_key.to_vec(),
    )
    .await
    {
        Ok(bl) => Arc::new(bl),
        Err(e) => {
            return Err(format!(
                "failed to open loop backlog chain \
                 (KeyDomain::LoopBacklog): {e}"
            ));
        }
    };
    // Phase 183 — the reminder store, shared between the remind.*
    // tools and the reminder driver. Zero-config (like the loop
    // backlog): always opened.
    let reminder_store: Arc<aivyx_channel::reminder_store::ReminderStore> =
        Arc::new(aivyx_channel::reminder_store::ReminderStore::new(
            storage.domain(KeyDomain::Reminders),
        ));

    // Phase 173 — the shared loop run state, created iff the
    // `[loop]` section is armed. `Some` → the daemon spawns the
    // loop driver + the `loop start/stop/status` IPC handlers
    // operate on this handle; `None` → loop runs cannot start.
    let loop_state: Option<aivyx_channel::loop_driver::SharedLoopState> =
        match &config_loop {
            Some(c) if c.enabled => {
                Some(aivyx_channel::loop_driver::SharedLoopState::new())
            }
            _ => None,
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
            // Phase 121 — route through the native OllamaProvider
            // (against `/api/chat` with JSONL streaming) instead of
            // the OpenAI-compat path. Q3a at Phase 121 sign-off:
            // `provider = "ollama"` in aivyx.toml uses the new
            // adapter transparently.
            use aivyx_llm::ollama::{
                OllamaConfig, OllamaOptions as LlmOllamaOptions,
                OllamaProvider, DEFAULT_OLLAMA_BASE_URL as OLLAMA_BASE,
            };
            let base_url = openai_base_url
                .map(|s| s.value)
                .unwrap_or_else(|| OLLAMA_BASE.to_string());
            ollama_base_url_for_tools = Some(base_url.clone());
            // Mirror config-layer OllamaOptions field-for-field
            // into the LLM-crate type. Keeps aivyx-config free of
            // an aivyx-llm dep.
            let llm_options = LlmOllamaOptions {
                num_ctx: config_ollama_options.num_ctx,
                num_predict: config_ollama_options.num_predict,
                num_thread: config_ollama_options.num_thread,
                mirostat: config_ollama_options.mirostat,
                top_k: config_ollama_options.top_k,
                top_p: config_ollama_options.top_p,
                repeat_penalty: config_ollama_options.repeat_penalty,
                repeat_last_n: config_ollama_options.repeat_last_n,
                seed: config_ollama_options.seed,
            };
            let mut cfg = OllamaConfig::default_local()
                .with_base_url(base_url)
                .with_options(llm_options);
            if let Some(key) = openai_api_key {
                cfg = cfg.with_api_key(key.value);
            }
            let p = OllamaProvider::new(cfg)
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
        ProviderKind::LlamaCpp => {
            // Phase 133 — route through the OpenAI-compat provider
            // against `llama-server`'s default port. Defaults to
            // `http://localhost:8080`; operator overrides via
            // `[llm] provider_base_url` in aivyx.toml.
            //
            // No native /api/chat equivalent — llama-server speaks
            // OpenAI-compat exclusively. The API key is accepted if
            // present but never required (llama-server ignores it),
            // so we use `OpenAiConfig::without_api_key` as the
            // baseline and layer a real key on top only when the
            // operator supplied one.
            const DEFAULT_LLAMACPP_BASE_URL: &str = "http://localhost:8080";
            let base_url = openai_base_url
                .map(|s| s.value)
                .unwrap_or_else(|| DEFAULT_LLAMACPP_BASE_URL.to_string());
            let cfg = match openai_api_key {
                Some(k) => OpenAiConfig::new(k.value).with_base_url(base_url),
                None => OpenAiConfig::without_api_key().with_base_url(base_url),
            };
            let p = OpenAiProvider::new(cfg)
                .map_err(|e| format!("failed to build llama-server provider: {e}"))?;
            Arc::new(p)
        }
        ProviderKind::Jan => {
            // Phase 133 — route through the OpenAI-compat provider
            // against Jan's default port. Defaults to
            // `http://localhost:1337/v1`; operator overrides via
            // `[llm] provider_base_url` in aivyx.toml.
            //
            // Jan's API mirrors api.openai.com/v1 exactly — no
            // server-side adaptation needed. The API key is accepted
            // if present but never required (Jan ignores it).
            const DEFAULT_JAN_BASE_URL: &str = "http://localhost:1337/v1";
            let base_url = openai_base_url
                .map(|s| s.value)
                .unwrap_or_else(|| DEFAULT_JAN_BASE_URL.to_string());
            let cfg = match openai_api_key {
                Some(k) => OpenAiConfig::new(k.value).with_base_url(base_url),
                None => OpenAiConfig::without_api_key().with_base_url(base_url),
            };
            let p = OpenAiProvider::new(cfg)
                .map_err(|e| format!("failed to build Jan provider: {e}"))?;
            Arc::new(p)
        }
        ProviderKind::MistralRs => {
            // Phase 134 — Direction B: embedded Rust-native
            // inference. Only available when Aivyx was built with
            // `--features provider-mistral-rs`; without the
            // feature, this arm surfaces an actionable error
            // pointing the operator at the build flag.
            #[cfg(feature = "provider-mistral-rs")]
            {
                use aivyx_llm::mistral_rs::{MistralRsConfig, MistralRsProvider};
                let opts = &config_mistralrs_options;
                let model_path = opts.model_path.as_ref().ok_or_else(|| {
                    "MistralRs provider selected but [mistralrs] model_path is missing. \
                     Set `model_path = \"/abs/path/to/model.gguf\"` in aivyx.toml."
                        .to_string()
                })?;
                let mut mr_cfg = MistralRsConfig::new(model_path.clone());
                if let Some(f) = &opts.model_file {
                    mr_cfg = mr_cfg.with_model_file(f.clone());
                }
                if let Some(t) = &opts.chat_template_path {
                    mr_cfg = mr_cfg.with_chat_template(t.clone());
                }
                if let Some(n) = opts.max_seq_len {
                    mr_cfg = mr_cfg.with_max_seq_len(n);
                }
                let p = MistralRsProvider::new(mr_cfg).await.map_err(|e| {
                    format!("failed to build mistralrs provider: {e}")
                })?;
                Arc::new(p)
            }
            #[cfg(not(feature = "provider-mistral-rs"))]
            {
                // Suppress dead-binding warning for the field we
                // pattern-bound earlier.
                let _ = &config_mistralrs_options;
                return Err(
                    "MistralRs provider selected but this Aivyx binary was built \
                     without the `provider-mistral-rs` feature. \
                     Rebuild with `cargo install --features recommended-providers aivyx` \
                     (or `--features provider-mistral-rs` for the lean variant) \
                     to enable embedded inference."
                        .to_string(),
                );
            }
        }
    };

    // ---- Phase 122 Task 4/5 — per-family prompt strategy -------------
    // Resolve the Ollama prompt strategy from the model name, layering
    // operator overrides (Task 5) over per-family defaults (Task 4):
    //
    // - `[ollama.prompt_strategies] qwen3 = "none"` → operator opts out
    //   of the structured-injection default for qwen3.
    // - Family detected, no override → per-family default.
    // - Family not detected (cloud model, bare-family-without-digits) →
    //   `OllamaFamilyStrategy::None`. Operator-conservative.
    //
    // Non-Ollama providers always end up at `None` (no catalog
    // injection) since the strategy enum gates the
    // `append_tool_catalog` call below — cloud providers handle their
    // tool-catalog surface natively.
    let ollama_prompt_strategy: aivyx_config::OllamaFamilyStrategy =
        if matches!(provider_kind.value, ProviderKind::Ollama) {
            aivyx_config::resolve_ollama_prompt_strategy(
                &model,
                &config_ollama_prompt_strategies,
            )
        } else {
            aivyx_config::OllamaFamilyStrategy::None
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
    // Phase 100 — fs.metadata is read-only; like fs.read / fs.write
    // it is registered for every channel. Destructive fs.delete is
    // built behind a Local-only trust gate further down
    // (`build_fs_delete_for_channel`).
    let fs_metadata = FsMetadataToolConfig::new(fs_root.clone())
        .build()
        .map_err(|e| format!("failed to build fs.metadata tool: {e}"))?;

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
    let fs_metadata_scope =
        Scope::parse(&format!("fs.metadata:{root_display}/**")).ok_or_else(|| {
            format!("canonical fs.metadata sandbox scope not parseable from {canonical_root:?}")
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
    // Phase 89 — opt-in topic canonicalization. When the
    // operator sets `[memory].canonicalize_topics = true`,
    // wrap the redb-backed memory in a `CanonicalizingMemory`
    // delegate that lowercases + stems the topic argument at
    // every topic-keyed trait entry point before delegating.
    // Every downstream signal (recall log, helpfulness
    // ledger, co-occurrence ledger, Persona facet provenance)
    // inherits the canonical topic form through the existing
    // pipeline. With the flag off (default), the memory is
    // byte-identical to pre-Phase-89.
    let memory: Arc<dyn Memory> =
        if config_memory_canonicalize_topics.value {
            Arc::new(
                aivyx_memory::CanonicalizingMemory::new(memory),
            )
        } else {
            memory
        };
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
    // Phase 172 — the durable correction ledger. Zero-config,
    // same condition + rationale as the helpfulness ledger (the
    // correction signal only exists when auto-recall is on).
    // Folded + pruned on the same cadence; consumed by the Phase
    // 172 correction-consolidation pass.
    let correction_ledger: Option<
        Arc<
            aivyx_channel::correction_ledger::PersistentCorrectionLedger,
        >,
    > = recall_log.as_ref().map(|_| {
        Arc::new(
            aivyx_channel::correction_ledger::PersistentCorrectionLedger::new(
                storage.domain(KeyDomain::CorrectionLedger),
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
    // Phase 84 (Q4a) — shared last-turn cluster-recall stat:
    // the recall provider writes it, GetLearningInsights reads
    // the *same* handle. `None` when [recall_cluster] is
    // absent (cluster expansion can never run).
    let recall_cluster_stat = config_recall_cluster
        .as_ref()
        .map(|_| {
            aivyx_channel::memory_recall::shared_recall_cluster_stat()
        });
    // Phase 86 — daemon-scoped per-session conversation windows.
    // Built iff the embedding substrate is configured (without
    // embeddings there is nothing to feed the window into and no
    // recall + Persona-selection path that would read it). The
    // same Arc handle is attached to both relevance providers
    // and to `DaemonConfig` so the turn loop's write site and
    // the read sites share state.
    let conversation_windows = embedding_provider.as_ref().map(|_| {
        aivyx_channel::conversation_window::shared_conversation_windows()
    });
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
            // Phase 84 — arm cluster-aware co-recall iff the
            // [recall_cluster] section is present and the
            // co-occurrence ledger exists (built above under
            // the same recall-substrate condition). The pass
            // still no-ops unless `enabled = true`.
            if let (Some(rc_cfg), Some(cooc)) =
                (&config_recall_cluster, &cooccurrence_ledger)
            {
                sc = sc.with_cluster(
                    Arc::clone(cooc),
                    rc_cfg.clone(),
                );
            }
            if let Some(stat) = &recall_cluster_stat {
                sc = sc.with_cluster_stat(stat.clone());
            }
            // Phase 86 — opt-in conversational-window relevance:
            // when `[embedding].recall_window_turns > 1` the
            // assembled prior-turns context (read from the shared
            // handle the daemon turn loop writes) becomes the
            // embedded query; otherwise byte-identical to
            // pre-Phase-86.
            if let Some(windows) = &conversation_windows {
                sc = sc.with_conversation_windows(
                    windows.clone(),
                    cfg.recall_window_turns,
                );
            }
            // Phase 90 — heuristic recall gate. Default `0`
            // (gate disabled) is byte-identical to
            // pre-Phase-90.
            sc = sc.with_recall_gate(cfg.recall_gate_min_chars);
            // Phase 96 — ANN index opt-in. Default
            // `false` is byte-identical to pre-Phase-96
            // brute-force.
            sc = sc.with_ann_index(
                cfg.ann_index,
                cfg.ann_rebuild_threshold,
            );
            // Phase 97 — token-budget. Default `0` is
            // byte-identical to pre-Phase-97 count-based.
            sc = sc.with_recall_token_budget(
                cfg.recall_token_budget,
            );
            // Phase 98 — hybrid keyword+semantic fusion.
            // Default `false` is byte-identical to
            // pre-Phase-98 semantic-only.
            sc = sc.with_recall_hybrid(cfg.recall_hybrid);
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
    // Phase 87 (Q4a) — shared last-consolidation-cycle stat:
    // the pass writes it, GetLearningInsights reads the same
    // handle. Created iff the consolidation pass is armed.
    let persona_consolidation_stat =
        match &config_persona_consolidation {
            Some(p) if p.enabled => Some(
                aivyx_channel::persona_consolidation::shared_persona_consolidation_stat(),
            ),
            _ => None,
        };
    // Phase 87 — production `PairPhraser` adapting the same
    // `LlmProvider` the agent uses. Created iff the
    // consolidation pass is armed; the reflection-cron path
    // skips the pass when this is None (no LLM access).
    let persona_consolidation_phraser: Option<
        Arc<dyn aivyx_channel::persona_consolidation::PairPhraser>,
    > = match &config_persona_consolidation {
        Some(p) if p.enabled => Some(Arc::new(
            aivyx_channel::persona_consolidation::LlmPairPhraser::new(
                Arc::clone(&provider),
                model.clone(),
            ),
        )),
        _ => None,
    };
    // Phase 172 — shared last-correction-consolidation-cycle
    // stat + production `TopicPhraser`. Created iff the
    // correction-consolidation pass is armed; the reflection-
    // cron path skips the pass when the phraser is None.
    let correction_consolidation_stat =
        match &config_correction_consolidation {
            Some(c) if c.enabled => Some(
                aivyx_channel::correction_consolidation::shared_correction_consolidation_stat(),
            ),
            _ => None,
        };
    let correction_consolidation_phraser: Option<
        Arc<dyn aivyx_channel::correction_consolidation::TopicPhraser>,
    > = match &config_correction_consolidation {
        Some(c) if c.enabled => Some(Arc::new(
            aivyx_channel::correction_consolidation::LlmTopicPhraser::new(
                Arc::clone(&provider),
                model.clone(),
            ),
        )),
        _ => None,
    };
    // Phase 91 — `[recall_judgment]` config + stat + LLM
    // judge. Built when the section is enabled; the daemon
    // arms the pass only when every piece is present.
    let recall_judgment_stat =
        match &_config_recall_judgment {
            Some(r) if r.enabled => Some(
                aivyx_channel::recall_judgment::shared_recall_judgment_stat(),
            ),
            _ => None,
        };
    let recall_judge: Option<
        Arc<dyn aivyx_channel::recall_judgment::RecallJudge>,
    > = match &_config_recall_judgment {
        Some(r) if r.enabled => Some(Arc::new(
            aivyx_channel::recall_judgment::LlmRecallJudge::new(
                Arc::clone(&provider),
                model.clone(),
            ),
        )),
        _ => None,
    };
    // Phase 178 — `[correction_judgment]` stat + LLM judge.
    // Built when the section is enabled; the reflection-cron
    // correction fold uses them only when armed.
    let correction_judgment_stat =
        match &config_correction_judgment {
            Some(c) if c.enabled => Some(
                aivyx_channel::correction_judgment::shared_correction_judgment_stat(),
            ),
            _ => None,
        };
    let correction_judge: Option<
        Arc<dyn aivyx_channel::correction_judgment::CorrectionJudge>,
    > = match &config_correction_judgment {
        Some(c) if c.enabled => Some(Arc::new(
            aivyx_channel::correction_judgment::LlmCorrectionJudge::new(
                Arc::clone(&provider),
                model.clone(),
            ),
        )),
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
            // Phase 86 — same shared conversational-window
            // handle the recall provider above uses; the
            // adaptive Persona selection now considers the
            // recent-turns context (same opt-in floor).
            if let (Some(windows), Some(cfg)) =
                (&conversation_windows, config_embedding.as_ref())
            {
                r = r.with_conversation_windows(
                    windows.clone(),
                    cfg.recall_window_turns,
                );
            }
            // Phase 90 — heuristic recall gate. Same opt-in
            // knob as the auto-recall provider above; with
            // the default `0` the refiner is byte-identical
            // to pre-Phase-90.
            if let Some(cfg) = config_embedding.as_ref() {
                r = r.with_recall_gate(cfg.recall_gate_min_chars);
                // Phase 97 — token-budget for adaptive
                // Persona facet selection. Default `0` is
                // byte-identical to pre-Phase-97.
                r = r.with_recall_token_budget(
                    cfg.recall_token_budget,
                );
            }
            Some(Arc::new(r))
        }
        None => None,
    };

    // Phase 117 Task 5 — when the operator has
    // `[tool_relevance] enabled = true`, construct a
    // `RelevancePromptRefiner` and install it as the
    // planner's `system_prompt_refiner`. If the Phase 79
    // PersonaContextRefiner is ALSO armed, chain the
    // Persona refiner as the inner so the operator gets
    // both adaptive Persona reduction AND the relevance
    // section augmentation from a single refiner slot.
    let system_prompt_refiner: Option<
        Arc<dyn aivyx_core::llm_planner::SystemPromptRefiner>,
    > = match &config_tool_relevance {
        Some(tr) if tr.enabled => {
            // The ledger handle has already been constructed
            // for DaemonConfig.tool_relevance_ledger below;
            // re-derive it here from the storage domain to
            // hand into the refiner. (Construction is cheap;
            // sharing the Arc via `tool_relevance_ledger` is
            // an option but the bin flow constructs both at
            // the same site, so re-derivation keeps the
            // dataflow legible.)
            let ledger = Arc::new(
                aivyx_channel::tool_relevance_ledger::PersistentToolRelevanceLedger::new(
                    storage.domain(KeyDomain::ToolRelevanceLedger),
                ),
            );
            let mut r = aivyx_channel::relevance_prompt_refiner::RelevancePromptRefiner::new(
                ledger,
                tr.clone(),
            );
            if let Some(inner) = &persona_refiner {
                r = r.with_inner_refiner(Arc::clone(inner));
            }
            Some(Arc::new(r)
                as Arc<dyn aivyx_core::llm_planner::SystemPromptRefiner>)
        }
        // No [tool_relevance] (or disabled): fall back to
        // the Phase 79 persona refiner if armed; otherwise
        // None.
        _ => persona_refiner.clone(),
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
        Arc::new(fs_metadata) as Arc<dyn Tool>,
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
    // Phase 100 — destructive `fs.delete` behind the same Local-only
    // trust gate as `shell.exec` (PHASE_100.md Q3). Read-only
    // `fs.metadata` is already in `tool_list` above (every channel);
    // `fs.delete` is registered only when the gate returns it.
    let fs_delete_scope: Option<Scope> =
        match build_fs_delete_for_channel(channel_kind, &fs_root)? {
            Some((fs_delete, scope)) => {
                tool_list.push(fs_delete);
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

    // Phase 109 — `net.dns` registers unconditionally (no
    // config required; uses the existing `net.dns` scope base
    // from Phase 0). All three channel kinds get it — like
    // `web.fetch`, network reads are inside the SemiTrusted
    // ceiling so Telegram / Discord / Slack get it too.
    tool_list.push(Arc::new(aivyx_core::NetDnsTool::new()) as Arc<dyn Tool>);

    // Phase 110 — Skills Auto-Creation tools. skills.list +
    // skills.invoke register unconditionally; both read the
    // approved skill set from the SharedEffectivePersona via a
    // closure that takes the read lock per call. The closure is
    // shared between the two tools but each tool holds its own
    // clone — the closure is cheap (only the lock + clone of
    // the Vec<String>).
    //
    // The skills tools live in CEILING_TRUSTED only (Phase 110
    // Task 3 sign-off). SemiTrusted roles do not get skills.*
    // by default; operators who want SemiTrusted skill access
    // grant individual bases through role capability_scopes.
    // Registration here is unconditional; the tier-ceiling
    // intersection at agent construction time enforces the
    // SemiTrusted exclusion.
    let skills_reader: aivyx_core::SkillReader = {
        let shared = shared_persona.clone();
        Arc::new(move || {
            shared
                .read()
                .map(|p| p.learned_skills.clone())
                .unwrap_or_default()
        })
    };
    tool_list.push(
        Arc::new(aivyx_core::SkillsListTool::new(skills_reader.clone()))
            as Arc<dyn Tool>,
    );
    tool_list.push(
        Arc::new(aivyx_core::SkillsInvokeTool::new(skills_reader))
            as Arc<dyn Tool>,
    );

    // Phase 109 — `git.status` + `git.diff` register only
    // when `[git]` config supplies an allow-set. Operators
    // who don't configure git repos don't pay any cost; agents
    // see no git tools in their dispatch surface.
    //
    // Like `fs.delete`, git tools are operator-scoped (the
    // allow-set is the operator's repos), not role-scoped.
    // Both register for all channel kinds — the SemiTrusted
    // adapters get the same read-only inspection access the
    // Local CLI gets.
    // Phase 109 — `git_read_scope` is the operator-held capability
    // representative for the first configured repo. Future
    // role-allowlist plumbing can consume it; today it just
    // documents that the Local CLI's capability set transitively
    // grants access to all configured repos through the role's
    // own `git.read:**` or per-repo grant declarations in
    // `aivyx.toml`. Underscore-prefixed to acknowledge the
    // landing-site is intentional but the consumer is
    // role-config-driven.
    let _git_read_scope: Option<Scope> = if let Some(gc) = config_git {
        let repos: Vec<std::path::PathBuf> =
            gc.repos.into_iter().map(|s| s.value).collect();
        let (git_status, git_diff) =
            aivyx_core::GitReadToolConfig::new(repos).build().map_err(|e| {
                format!("failed to build git.read tool pair: {e}")
            })?;
        // The canonical allow-set is the same for both tools;
        // construct one scope per canonical path so the
        // operator-held capability set includes them all.
        let canonical_repos: Vec<std::path::PathBuf> =
            git_status.repos().to_vec();
        tool_list.push(Arc::new(git_status) as Arc<dyn Tool>);
        tool_list.push(Arc::new(git_diff) as Arc<dyn Tool>);
        // Build a representative scope for the first repo so
        // the Local CLI's operator-held grants include
        // `git.read:<first_repo>/**`. Role-scoped grants in
        // `aivyx.toml` can name per-repo scopes for finer
        // control.
        canonical_repos.first().and_then(|p| {
            Scope::parse(&format!("git.read:{}", p.display()))
        })
    } else {
        None
    };

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

    // Phase 173 — the autonomous-loop backlog tools. Built with
    // the shared backlog `Arc` so the loop agent (and any Trusted
    // turn) can read the next story + mark stories done. Always
    // registered; capability-gated under loop.next / loop.complete.
    let loop_next_tool: Arc<aivyx_channel::loop_tool::LoopNextTool> =
        Arc::new(aivyx_channel::loop_tool::LoopNextTool::new());
    let _ = loop_next_tool.set_backlog(Arc::clone(&loop_backlog));
    tool_list.push(Arc::clone(&loop_next_tool) as Arc<dyn Tool>);
    let loop_complete_tool: Arc<
        aivyx_channel::loop_tool::LoopCompleteTool,
    > = Arc::new(aivyx_channel::loop_tool::LoopCompleteTool::new());
    let _ = loop_complete_tool.set_backlog(Arc::clone(&loop_backlog));
    tool_list.push(Arc::clone(&loop_complete_tool) as Arc<dyn Tool>);
    // Phase 175 — loop.note appends a learning to the reserved
    // progress topic the driver injects into each fresh
    // iteration. Built with the shared memory handle.
    let loop_note_tool: Arc<aivyx_channel::loop_tool::LoopNoteTool> =
        Arc::new(aivyx_channel::loop_tool::LoopNoteTool::new());
    let _ = loop_note_tool.set_memory(Arc::clone(&memory));
    tool_list.push(Arc::clone(&loop_note_tool) as Arc<dyn Tool>);

    // Chapter L (L.7) — team.run lets a daemon turn (notably an autonomous-loop
    // iteration) delegate a large goal to a durable Nonagon team mission. The
    // TeamMissionService is built later (it needs the assembled provider/tools),
    // so the tool is wired now and `set_service` is called in the daemon branch.
    let team_run_tool: Arc<aivyx_channel::team_mission_driver::TeamRunTool> =
        Arc::new(aivyx_channel::team_mission_driver::TeamRunTool::new());
    tool_list.push(Arc::clone(&team_run_tool) as Arc<dyn Tool>);

    // Phase 183 — the remind.* channel-tier tools, sharing the
    // reminder store with the driver.
    let remind_set_tool =
        Arc::new(aivyx_channel::reminder_tool::RemindSetTool::new());
    let _ = remind_set_tool.set_store(Arc::clone(&reminder_store));
    tool_list.push(Arc::clone(&remind_set_tool) as Arc<dyn Tool>);
    let remind_list_tool =
        Arc::new(aivyx_channel::reminder_tool::RemindListTool::new());
    let _ = remind_list_tool.set_store(Arc::clone(&reminder_store));
    tool_list.push(Arc::clone(&remind_list_tool) as Arc<dyn Tool>);
    let remind_cancel_tool =
        Arc::new(aivyx_channel::reminder_tool::RemindCancelTool::new());
    let _ = remind_cancel_tool.set_store(Arc::clone(&reminder_store));
    tool_list.push(Arc::clone(&remind_cancel_tool) as Arc<dyn Tool>);

    // Phase 184 — conversational skill-teaching. The edit tools
    // append LearnedSkill deltas to the persona chain after the
    // agent confirms the drafted skill with the operator.
    {
        use aivyx_channel::skill_tool::{
            SkillForgetTool, SkillTeachTool, SkillUpdateTool,
        };
        let teach = Arc::new(SkillTeachTool::new());
        let _ = teach.set_persona_log(Arc::clone(&persona_log));
        let _ = teach.set_effective_persona(shared_persona.clone());
        tool_list.push(Arc::clone(&teach) as Arc<dyn Tool>);
        let update = Arc::new(SkillUpdateTool::new());
        let _ = update.set_persona_log(Arc::clone(&persona_log));
        let _ = update.set_effective_persona(shared_persona.clone());
        tool_list.push(Arc::clone(&update) as Arc<dyn Tool>);
        let forget = Arc::new(SkillForgetTool::new());
        let _ = forget.set_persona_log(Arc::clone(&persona_log));
        let _ = forget.set_effective_persona(shared_persona.clone());
        tool_list.push(Arc::clone(&forget) as Arc<dyn Tool>);
    }

    let shared_role_overrides = aivyx_channel::role_overrides::shared_role_overrides();
    // `persona_log` + `shared_persona` were created earlier (right
    // after the role assemble) so the system-prompt path could read
    // the startup snapshot. The apply-tool setters land just below.

    let mut mcp_bridges: Vec<std::sync::Arc<aivyx_mcp::McpServerBridge>> = Vec::new();
    // Phase: live `tools/list_changed` hot-swap — the tool ids each MCP
    // server currently contributes, so the refresh coordinator can swap
    // exactly that server's tools when it signals a change.
    let mut mcp_server_tool_ids: std::collections::HashMap<String, Vec<aivyx_core::ToolId>> =
        std::collections::HashMap::new();
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
            aivyx_config::McpTransportKind::Http => {
                let url = mcp_cfg.url.as_deref().unwrap_or("");
                match aivyx_mcp::StreamableHttpTransport::connect(url).await {
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
                        mcp_server_tool_ids.insert(
                            mcp_cfg.name.clone(),
                            mcp_tools.iter().map(|t| t.id()).collect(),
                        );
                        tool_list.extend(mcp_tools);
                        let transport_label = match mcp_cfg.transport {
                            aivyx_config::McpTransportKind::Stdio => "stdio",
                            aivyx_config::McpTransportKind::Sse => "sse",
                            aivyx_config::McpTransportKind::Http => "http",
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
                mcp_bridges.push(std::sync::Arc::new(bridge));
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
    // Phase 180 — resolve the bundled default sandbox once. `auto`
    // detects bwrap/firejail on PATH; an unresolved `auto` warns
    // and falls back to no sandbox (no worse than pre-Phase-180).
    let sandbox_choice = match config_sandbox_default_backend {
        aivyx_config::SandboxDefaultBackend::None => {
            aivyx_tool::SandboxChoice::None
        }
        aivyx_config::SandboxDefaultBackend::Auto => {
            aivyx_tool::SandboxChoice::Auto
        }
        aivyx_config::SandboxDefaultBackend::Bubblewrap => {
            aivyx_tool::SandboxChoice::Backend(
                aivyx_tool::SandboxBackend::Bubblewrap,
            )
        }
        aivyx_config::SandboxDefaultBackend::Firejail => {
            aivyx_tool::SandboxChoice::Backend(
                aivyx_tool::SandboxBackend::Firejail,
            )
        }
    };
    let detected_backend = aivyx_tool::detect_sandbox_backend();
    if matches!(sandbox_choice, aivyx_tool::SandboxChoice::Auto)
        && detected_backend.is_none()
        && !config_tool_processes.is_empty()
    {
        eprintln!(
            "aivyx: [sandbox] default_backend = auto but neither \
             bwrap nor firejail is on PATH — tool processes will \
             run UNSANDBOXED. Install bubblewrap or firejail, or \
             set an explicit [tool_process.sandbox] block."
        );
    }
    for tp_cfg in &config_tool_processes {
        // Phase 182 — if a Google productivity tool is configured
        // but not yet authenticated, name its own remedy.
        if let Some(home) = std::env::var_os("HOME") {
            if let Some(hint) = connect::unauthenticated_hint(
                &tp_cfg.command,
                std::path::Path::new(&home),
            ) {
                eprintln!("aivyx: {hint}");
            }
        }
        // Phase 52 — the operator's explicit [tool_process.sandbox]
        // wins outright. None when the block is omitted.
        let explicit = tp_cfg.sandbox.as_ref().map(|s| aivyx_tool::SandboxConfig {
            wrapper: s.wrapper.clone(),
            args: s.args.clone(),
        });
        // Phase 180 — read-only bind the command-binary dir (a
        // bundled tool may live outside /usr) and writable-bind the
        // per-tool data dir (where its OAuth token lives).
        let ro_extra: Vec<std::path::PathBuf> =
            std::path::Path::new(&tp_cfg.command)
                .parent()
                .map(|p| vec![p.to_path_buf()])
                .unwrap_or_default();
        let writable: Vec<std::path::PathBuf> = std::env::var_os("HOME")
            .map(|home| {
                vec![std::path::PathBuf::from(home)
                    .join(".aivyx")
                    .join("tool-processes")
                    .join(&tp_cfg.name)]
            })
            .unwrap_or_default();
        let spawn_sandbox = aivyx_tool::resolve_sandbox(
            explicit,
            tp_cfg.disable_sandbox,
            sandbox_choice,
            detected_backend,
            &ro_extra,
            &writable,
        );
        match &spawn_sandbox {
            Some(s) => eprintln!(
                "aivyx: tool process {:?} — sandboxed ({})",
                tp_cfg.name, s.wrapper
            ),
            None => eprintln!(
                "aivyx: tool process {:?} — UNSANDBOXED (runs with \
                 your full user identity)",
                tp_cfg.name
            ),
        }
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
    // Phase 183 — spawn the reminder driver. It shares the
    // notify dispatcher; a reminder that names no target goes to
    // every registered target (the operator configured them).
    {
        use aivyx_channel::reminder_driver::{
            run_reminder_driver, DispatcherNotifier, ReminderNotifier,
            DEFAULT_CHECK_INTERVAL_SECS,
        };
        let default_targets: Vec<String> = notify_dispatcher
            .list_targets()
            .into_iter()
            .map(|(name, _kind)| name.to_string())
            .collect();
        let notifier: Arc<dyn ReminderNotifier> =
            Arc::new(DispatcherNotifier::new(
                Arc::clone(&notify_dispatcher),
                default_targets,
            ));
        let interval = std::time::Duration::from_secs(
            config_reminders_check_interval_secs
                .filter(|s| *s > 0)
                .unwrap_or(DEFAULT_CHECK_INTERVAL_SECS),
        );
        let store = Arc::clone(&reminder_store);
        tokio::spawn(async move {
            run_reminder_driver(store, notifier, interval).await;
        });
    }

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

    // Chapter J — `aivyx team run "<mission>"`. We now hold the live provider,
    // the persistent HMAC audit hook, AND the daemon's full `tool_list` — so
    // assemble the team and run the lead in-process with specialists that get
    // their real (attenuated) tools. A one-shot command: it consumes
    // `tool_list` and returns here rather than falling through to the
    // session/daemon wiring.
    if let CliMode::Team(TeamSubcommand::Run { mission, config }) = &mode {
        return team::run_mission(
            Arc::clone(&provider),
            &model,
            DEFAULT_MAX_TOKENS,
            Arc::clone(&audit),
            tool_list,
            mission,
            config.as_deref(),
        )
        .await;
    }

    let tools: Arc<ToolRegistry> = Arc::new(ToolRegistry::new(tool_list));

    // Live MCP `tools/list_changed` hot-swap. A background task polls
    // each bridge's drained list-changed set (recorded by the shared
    // `McpConn` demux during normal tool calls); when a server signals a
    // change it re-discovers that server and swaps exactly its tools in
    // the shared `ToolRegistry` — no restart. Capability-safe: the
    // replacements carry the same `mcp.call:<server>` scope family the
    // role already granted. The round-trip lock in `McpConn` keeps the
    // coordinator's `rediscover()` from racing in-flight agent calls.
    if !mcp_bridges.is_empty() {
        let bridges = mcp_bridges.clone();
        let registry = Arc::clone(&tools);
        let mut server_tool_ids = mcp_server_tool_ids.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                ticker.tick().await;
                for bridge in &bridges {
                    let changed = bridge.take_pending_list_changed();
                    if changed.is_empty() {
                        continue;
                    }
                    let name = bridge.server_name().to_string();
                    match bridge.rediscover().await {
                        Ok(new_tools) => {
                            let new_ids: Vec<aivyx_core::ToolId> =
                                new_tools.iter().map(|t| t.id()).collect();
                            let old_ids =
                                server_tool_ids.get(&name).cloned().unwrap_or_default();
                            let (removed, added) =
                                registry.replace_tools(&old_ids, new_tools);
                            server_tool_ids.insert(name.clone(), new_ids);
                            eprintln!(
                                "aivyx: MCP server {name:?} signalled {changed:?} — \
                                 hot-swapped tools ({removed} removed, {added} added)"
                            );
                        }
                        Err(e) => eprintln!(
                            "aivyx: MCP server {name:?} list-changed rediscover failed: {e}"
                        ),
                    }
                }
            }
        });
    }

    // Phase 102 — snapshot the registered tool set for the daemon's
    // `GetToolStats` query, captured here before `tools` is moved
    // into the planner factory. The scope base comes from
    // `required_scope` (it differs from the tool name for the web
    // tools — `web.fetch` keys on `net.fetch`). Cheap (~20 tools);
    // only the daemon path consumes it, but capturing here keeps it
    // ahead of every move of `tools`.
    let tool_descriptors: Vec<ToolDescriptor> = tools
        .snapshot()
        .into_iter()
        .map(|t| ToolDescriptor {
            name: t.name().to_string(),
            description: t.description().to_string(),
            scope_base: t
                .required_scope(&serde_json::json!({}))
                .base()
                .to_string(),
        })
        .collect();

    // Phase 122 Task 4 / Phase 124 Task 3 — snapshot a tool
    // catalog for the prompt-augmentation strategies. Built
    // for any strategy other than `None`; an empty `Vec`
    // for `None` so the dispatcher is a guaranteed no-op.
    //
    // Filtered by the role's `tool_allowlist` so the catalog
    // mirrors what the model can actually invoke (an allowlist
    // role shouldn't see tools it'll be denied). `AllowAll`
    // roles see every registered tool.
    let prompt_tool_catalog: Vec<aivyx_llm::LlmToolDescriptor> =
        if matches!(
            ollama_prompt_strategy,
            aivyx_config::OllamaFamilyStrategy::StructuredInjection
                | aivyx_config::OllamaFamilyStrategy::FewShotExamples
        ) {
            tools
                .snapshot()
                .into_iter()
                .filter(|t| match &tool_allowlist {
                    None => true,
                    Some(set) => set.contains(t.name()),
                })
                .map(|t| aivyx_llm::LlmToolDescriptor {
                    name: t.name().to_string(),
                    description: t.description().to_string(),
                    input_schema: t.input_schema().clone(),
                })
                .collect()
        } else {
            Vec::new()
        };

    // Phase 124 Task 3 — apply the per-family strategy via
    // the dispatcher. `None` returns the base prompt
    // unchanged; `StructuredInjection` adds the catalog;
    // `FewShotExamples` adds catalog + worked examples.
    let system_prompt = aivyx_channel::profile_prompt::apply_ollama_prompt_strategy(
        &system_prompt,
        &prompt_tool_catalog,
        ollama_prompt_strategy,
    );

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
        fs_metadata_scope,
        Scope::parse("net.fetch").unwrap(),
        Scope::parse("net.post").unwrap(),
    ];
    if let Some(s) = shell_exec_scope {
        backcompat_floor.push(s);
    }
    if let Some(s) = fs_delete_scope {
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
    // Phase 117 — `system_prompt_refiner` is now the
    // combined (potentially Phase 117 + Phase 79) refiner;
    // sub-agents inherit the same composition.
    let persona_refiner_for_factory = system_prompt_refiner.clone();

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
        let child_assembled = aivyx_channel::assemble_session_prompt(
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
        // Phase 122 Task 4 / Phase 124 Task 3 — child agents
        // get their own catalog snapshot filtered by the child
        // role's allowlist. Built for any non-`None` strategy;
        // empty otherwise → dispatcher is a no-op.
        let child_prompt_tool_catalog: Vec<aivyx_llm::LlmToolDescriptor> =
            if matches!(
                ollama_prompt_strategy,
                aivyx_config::OllamaFamilyStrategy::StructuredInjection
                    | aivyx_config::OllamaFamilyStrategy::FewShotExamples
            ) {
                tools_for_factory
                    .snapshot()
                    .into_iter()
                    .filter(|t| match &child_tool_allowlist {
                        None => true,
                        Some(set) => set.contains(t.name()),
                    })
                    .map(|t| aivyx_llm::LlmToolDescriptor {
                        name: t.name().to_string(),
                        description: t.description().to_string(),
                        input_schema: t.input_schema().clone(),
                    })
                    .collect()
            } else {
                Vec::new()
            };
        let child_system_prompt =
            aivyx_channel::profile_prompt::apply_ollama_prompt_strategy(
                &child_assembled,
                &child_prompt_tool_catalog,
                ollama_prompt_strategy,
            );

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
            ))
            // Phase 120 — operator-configured fuzzy-match threshold
            // for the planner's tool-name recovery path.
            .with_tool_name_auto_correct_threshold(
                config_tool_name_auto_correct_threshold.value,
            );
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
        // Phase 122 Task 4 — same catalog as the initial
        // child_system_prompt, cloned into every per-turn
        // re-assembly.
        let child_refresher_catalog = child_prompt_tool_catalog.clone();
        let child_planner_factory = move || {
            let mut cfg = planner_config.clone();
            let snap = child_refresher_shared
                .read()
                .expect("persona lock not poisoned at child turn build");
            let assembled = aivyx_channel::assemble_session_prompt(
                &child_refresher_profile,
                Some(&*snap),
                &child_refresher_role_name,
                &child_refresher_role_prompt,
            );
            cfg.system_prompt = Some(
                aivyx_channel::profile_prompt::apply_ollama_prompt_strategy(
                    &assembled,
                    &child_refresher_catalog,
                    ollama_prompt_strategy,
                ),
            );
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
            ))
            // Phase 120 — daemon path mirror of the in-process
            // factory above.
            .with_tool_name_auto_correct_threshold(
                config_tool_name_auto_correct_threshold.value,
            );
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
        // Phase 122 Task 4 — clone the structured-injection
        // tool catalog into the refresher so every per-turn
        // re-assembly preserves the `## Tools available`
        // block. Empty when strategy is `None` → no-op append.
        let daemon_refresher_catalog = prompt_tool_catalog.clone();
        let planner_factory = move || {
            let mut cfg = planner_config.clone();
            // Per-turn rebuild from current Persona state.
            let snap = daemon_refresher_shared
                .read()
                .expect("persona lock not poisoned at turn build");
            let assembled = aivyx_channel::assemble_session_prompt(
                &daemon_refresher_profile,
                Some(&*snap),
                &daemon_refresher_role_name,
                &daemon_refresher_role_prompt,
            );
            cfg.system_prompt = Some(
                aivyx_channel::profile_prompt::apply_ollama_prompt_strategy(
                    &assembled,
                    &daemon_refresher_catalog,
                    ollama_prompt_strategy,
                ),
            );
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
        // Chapter K (K.4.2) — the shared pre-call dollar gate. Built once and
        // attached to the daemon's agent so every interactive / team turn is
        // checked against the operator's `[budget] per_day_usd` cap before any
        // model call. `None` when no day cap is set, so an ungated daemon
        // keeps today's behavior byte-for-byte.
        let daemon_budget_gate: Option<Arc<dyn aivyx_core::BudgetGate>> =
            aivyx_channel::budget_gate::ChannelBudgetGate::new_gate(
                config_budget.clone(),
                Arc::clone(&persistent_audit_for_query),
                aivyx_cost::Pricing::with_overrides(config_pricing.clone()),
                DEFAULT_MAX_TOKENS,
            );

        // Chapter L (L.5) — the daemon's team-mission service: the registry
        // over KeyDomain::TeamMissions (reloaded on startup so paused missions
        // resume across a restart) plus the run deps (provider/model/audit/the
        // full tool set) every specialist sub-turn assembles over. Built here,
        // before `tools` + `audit` are moved into the agent. The default
        // Nonagon is the team for L.5; vertical-pack configs are a later
        // increment.
        let team_missions = {
            let state = aivyx_channel::team_mission_driver::SharedMissionState::new(
                storage.domain(KeyDomain::TeamMissions),
            );
            match state.reload().await {
                Ok(n) if n > 0 => {
                    eprintln!("aivyx team: reloaded {n} persisted team mission(s)");
                }
                Ok(_) => {}
                Err(e) => eprintln!("aivyx team: mission reload failed — {e}"),
            }
            let deps = aivyx_channel::team_mission_driver::TeamRunDeps {
                provider: Arc::clone(&provider),
                model: model.clone(),
                max_tokens: DEFAULT_MAX_TOKENS,
                audit: Arc::clone(&audit),
                base_tools: tools.snapshot(),
            };
            let service = aivyx_channel::team_mission_driver::TeamMissionService::new(
                state,
                deps,
                aivyx_team::default_nonagon(),
                // Chapter H — the daemon's gate posture (Interactive for now;
                // H.4/H.5 set headless for operator-absent runs).
                aivyx_core::GatePolicy::default(),
            );
            // L.7 — give the team.run tool the live service so loop / interactive
            // turns can delegate goals to durable team missions.
            let _ = team_run_tool.set_service(service.clone());
            Some(service)
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
            .with_memory_topic_prefix(memory_topic_prefix)
            .with_budget_gate(daemon_budget_gate),
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
                // Phase 111 — Discord daemon-mode channel stub.
                // Mirrors TelegramDaemonChannel; the
                // discord_daemon_frontend.rs module wires the
                // actual two-way bridge.
                aivyx_channel::daemon_ipc::FrontendType::Discord => {
                    Arc::new(aivyx_channel::discord_daemon_frontend::DiscordDaemonChannel::new())
                }
                // Phase 111 — Slack daemon-mode channel stub.
                // Mirrors TelegramDaemonChannel and
                // DiscordDaemonChannel; the slack_daemon_frontend.rs
                // module wires the actual two-way bridge.
                aivyx_channel::daemon_ipc::FrontendType::Slack => {
                    Arc::new(aivyx_channel::slack_daemon_frontend::SlackDaemonChannel::new())
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

        // Phase 113 Task 3 — Construct the Skill Auto-Proposer
        // context from the loaded TOML config. `None` when the
        // operator hasn't configured any auto-proposer section,
        // in which case the daemon wires
        // `DaemonConfig::skill_auto_proposer = None` and the
        // Phase 112 substrate is bypassed. With a section
        // present, the auto-proposer reuses the same LLM
        // provider the planner uses for tool calls — keeping
        // the configured-provider invariant the operator
        // declared in their `[provider]` config.
        //
        // Phase 114 — prefer the new `[persona.auto_propose]`
        // section if the operator configured it; fall back to
        // the Phase 113 `[skills.auto_propose]` alias
        // otherwise. Both convert to the same runtime
        // `SkillAutoProposeConfig`; the Phase 114 path
        // populates `per_category` and the alias path leaves
        // it `None` (Phase 113 single-config posture).
        let runtime_cfg: Option<aivyx_channel::skill_auto_proposer::SkillAutoProposeConfig> =
            match (config_persona_auto_propose, config_skill_auto_propose) {
                (Some(p), _) => Some(p.into()),
                (None, Some(s)) => Some(s.into()),
                (None, None) => None,
            };
        let skill_auto_proposer_ctx = runtime_cfg.map(|cfg| {
            Arc::new(
                aivyx_channel::skill_auto_proposer::SkillAutoProposerContext {
                    config: cfg,
                    llm_provider: Arc::clone(&provider),
                },
            )
        });

        let result = run_daemon(DaemonConfig {
            socket_path,
            agent,
            channel_factory,
            shutdown,
            tool_descriptors,
            // Phase 113 Task 3 — wired from the loaded TOML
            // `[skills.auto_propose]` section. `None` when the
            // section is absent (operator hasn't opted in).
            skill_auto_proposer: skill_auto_proposer_ctx,
            // Phase 116 — tool/skill relevance ledger handle.
            // Constructed when the operator has
            // `[tool_relevance] enabled = true` in their TOML.
            // None when absent / disabled.
            tool_relevance_ledger: config_tool_relevance
                .as_ref()
                .filter(|c| c.enabled)
                .map(|_| {
                    Arc::new(
                        aivyx_channel::tool_relevance_ledger::PersistentToolRelevanceLedger::new(
                            storage.domain(KeyDomain::ToolRelevanceLedger),
                        ),
                    )
                }),
            // Phase 173 — autonomous loop: the always-built
            // backlog, the shared run state (Some iff armed), and
            // the [loop] config.
            loop_backlog: Some(Arc::clone(&loop_backlog)),
            loop_state: loop_state.clone(),
            loop_config: config_loop.clone(),
            // Chapter L (L.5) — the team-mission service built above.
            team_missions,
            // Chapter H — the daemon's default gate posture. Interactive for
            // now; the `--headless` flag + operator-absent drivers (H.4/H.5)
            // set RejectAndAbort per run.
            gate_policy: aivyx_core::GatePolicy::default(),
            // K.4.2 — the override-aware rate table the autonomous loop's
            // dollar cap prices with. Built once from the built-in defaults
            // plus any `[pricing.<model>]` overrides the operator declared.
            pricing: aivyx_cost::Pricing::with_overrides(
                config_pricing.clone(),
            ),
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
            // Phase 172 — durable correction ledger; folded by
            // the same recall-feedback pass.
            correction_ledger: correction_ledger.clone(),
            // Phase 79 (Q4a) — same handle the adaptive refiner
            // writes; the GetLearningInsights handler reads it.
            persona_selection_stat: persona_selection_stat.clone(),
            // Phase 84 — shared last-turn cluster-recall stat
            // (same handle the recall provider writes).
            recall_cluster_stat: recall_cluster_stat.clone(),
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
            // Phase 86 — shared per-session windows. The daemon
            // turn loop writes `(user, assistant)` pairs into
            // this on every `TurnOutcome::Completed`; both
            // relevance providers read via the same handle.
            // `None` → embedding off → no window path.
            conversation_windows: conversation_windows.clone(),
            // Phase 87 — pattern-driven Persona consolidation
            // config + Phase 78 surface stat + LLM phraser.
            // All three are `Some` iff the section is enabled
            // (the daemon arms the pass only when every piece
            // is present).
            persona_consolidation_config:
                config_persona_consolidation.clone(),
            persona_consolidation_stat:
                persona_consolidation_stat.clone(),
            persona_consolidation_phraser:
                persona_consolidation_phraser.clone(),
            // Phase 172 — correction-driven Persona consolidation
            // config + Phase 78 surface stat + LLM topic phraser.
            // All three are `Some` iff the section is enabled.
            correction_consolidation_config:
                config_correction_consolidation.clone(),
            correction_consolidation_stat:
                correction_consolidation_stat.clone(),
            correction_consolidation_phraser:
                correction_consolidation_phraser.clone(),
            // Phase 91 — `[recall_judgment]` config + stat +
            // LLM judge. All three are `Some` iff the
            // section is enabled.
            recall_judgment_config:
                _config_recall_judgment.clone(),
            recall_judgment_stat:
                recall_judgment_stat.clone(),
            recall_judge: recall_judge.clone(),
            // Phase 178 — correction judgment config + judge +
            // stat (Some iff `[correction_judgment].enabled`).
            correction_judgment_config:
                config_correction_judgment.clone(),
            correction_judge: correction_judge.clone(),
            correction_judgment_stat:
                correction_judgment_stat.clone(),
            // Phase 179 — `[correction_signal]` config.
            correction_signal_config:
                config_correction_signal.clone(),
            // Phase 93 — `[recall_feedback]` config threaded
            // into the daemon. Drives `correlate_detailed` in
            // both the reflection-cron recall-feedback pass
            // AND the `GetLearningInsights` IPC surface so
            // the operator's insights view reflects the same
            // signal source the actuator uses.
            recall_feedback_config:
                config_recall_feedback.clone(),
        })
            .await;

        for bridge in &mcp_bridges {
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
            // Phase 122 Task 4 — clone the catalog into the
            // refresher; preserves the `## Tools available`
            // block on every per-turn re-assembly.
            let refresher_catalog = prompt_tool_catalog.clone();
            let prompt_refresher: Arc<dyn Fn() -> String + Send + Sync> =
                Arc::new(move || {
                    let snap = refresher_shared
                        .read()
                        .expect("persona lock not poisoned at turn build");
                    let assembled = aivyx_channel::assemble_session_prompt(
                        &refresher_profile,
                        Some(&*snap),
                        &refresher_role_name,
                        &refresher_role_prompt,
                    );
                    aivyx_channel::profile_prompt::apply_ollama_prompt_strategy(
                        &assembled,
                        &refresher_catalog,
                        ollama_prompt_strategy,
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
                // Phase 79 — adaptive Persona; Phase 117 —
                // tool/skill relevance section. Both ride on
                // the same refiner slot through the combined
                // `system_prompt_refiner` constructed above.
                system_prompt_refiner: system_prompt_refiner.clone(),
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

        // Phase 107 Task 5 — Discord adapter dispatch. Mirrors
        // the Telegram arm above but skips the chat_filter +
        // long-poll-cursor machinery: Discord's Gateway is a
        // continuous event stream and the inner multiplexer
        // routes per `channel_id` straight from
        // `MessageCreate` events.
        ChannelKind::Discord => {
            let dc = discord
                .expect("discord config validated for ChannelKind::Discord");
            let token_secret = dc
                .token
                .expect("discord.token validated non-None before run_async")
                .value;

            let shutdown = CancellationToken::new();
            let shutdown_for_signal = shutdown.clone();
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_err() {
                    std::process::exit(130);
                }
                eprintln!(
                    "\naivyx: shutting down discord bot after current event drains."
                );
                shutdown_for_signal.cancel();
            });

            use secrecy::ExposeSecret;
            let token_str = token_secret.expose_secret();

            // Phase 111 — daemon-first, in-process fallback —
            // same pattern as Telegram's Phase 19 wiring. The
            // Phase 107 Task 5 carve-out is now closed; the
            // daemon-mode path goes through
            // run_discord_daemon_multi_session.
            if !no_daemon && let Ok(sp) = default_socket_path() {
                let transport = std::sync::Arc::new(
                    aivyx_discord::transport::TwilightTransport::new(token_str),
                );

                eprintln!(
                    "aivyx {} (daemon) — discord bot live\n\
                     daemon: {}\n\
                     fs sandbox: {}\n\
                     memory: live (recall persists across restarts)\n\
                     audit: persistent ({} events verified from disk)",
                    env!("CARGO_PKG_VERSION"),
                    sp.display(),
                    canonical_root.display(),
                    verified_event_count,
                );

                match aivyx_channel::discord_daemon_frontend::run_discord_daemon_multi_session(
                    transport,
                    sp.clone(),
                    Some(active_role_name.clone()),
                    shutdown.clone(),
                )
                .await
                {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        eprintln!(
                            "aivyx: daemon discord session failed ({e}), \
                             falling back to in-process."
                        );
                    }
                }
            } else {
                eprintln!("aivyx: no socket path available, using in-process mode.");
            }

            // In-process fallback (Phase 107 path).
            eprintln!(
                "aivyx {} — discord bot live (in-process)\n\
                 fs sandbox: {}\n\
                 memory: live (recall persists across restarts)\n\
                 audit: persistent ({} events verified from disk)",
                env!("CARGO_PKG_VERSION"),
                canonical_root.display(),
                verified_event_count,
            );

            let discord_config = aivyx_discord::DiscordSessionConfig {
                model,
                system_prompt,
                max_tokens: DEFAULT_MAX_TOKENS,
                capabilities,
                tools,
                storage,
                tool_allowlist,
                memory_topic_prefix,
            };
            aivyx_discord::run_discord_session(
                "aivyx-discord",
                token_str,
                discord_config,
                provider,
                audit,
                shutdown,
            )
            .await
            .map(|_report| ())
        }

        // Phase 108 Task 5 — Slack adapter dispatch. Mirrors
        // the Discord arm above. Socket Mode requires two
        // tokens (bot for REST, app for the WebSocket); both
        // validated non-None by `require_slack_tokens` in
        // load-options upstream.
        //
        // The production SlackMorphismTransport is currently
        // a compile-only stub (Phase 108 Task 3 carved out
        // the live Socket Mode wiring as an internal deferral
        // bundled with the Phase 107 daemon-frontend
        // follow-on). An operator who runs `--channel slack`
        // today reaches the session driver but
        // `transport.next_message()` returns a clean
        // "production transport not yet wired" error rather
        // than panicking. The scripted-test layer fully
        // exercises the channel + session substrate.
        ChannelKind::Slack => {
            let sc = slack.expect("slack config validated for ChannelKind::Slack");
            let bot_token_secret = sc
                .bot_token
                .expect("slack.bot_token validated non-None before run_async")
                .value;
            let app_token_secret = sc
                .app_token
                .expect("slack.app_token validated non-None before run_async")
                .value;

            let shutdown = CancellationToken::new();
            let shutdown_for_signal = shutdown.clone();
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_err() {
                    std::process::exit(130);
                }
                eprintln!(
                    "\naivyx: shutting down slack bot after current event drains."
                );
                shutdown_for_signal.cancel();
            });

            use secrecy::ExposeSecret;
            let bot_token_str = bot_token_secret.expose_secret();
            let app_token_str = app_token_secret.expose_secret();

            // Phase 111 — daemon-first, in-process fallback —
            // same pattern as Telegram + Discord. The Phase 108
            // Task 3 SlackMorphismTransport carve-out is now
            // closed too; the live Socket Mode wiring lands
            // through Task 4 of Phase 111.
            if !no_daemon && let Ok(sp) = default_socket_path() {
                let transport_result =
                    aivyx_slack::transport::SlackMorphismTransport::connect(
                        bot_token_str,
                        app_token_str,
                    )
                    .await;
                match transport_result {
                    Ok(transport) => {
                        let transport = std::sync::Arc::new(transport);
                        eprintln!(
                            "aivyx {} (daemon) — slack bot live\n\
                             daemon: {}\n\
                             fs sandbox: {}\n\
                             memory: live (recall persists across restarts)\n\
                             audit: persistent ({} events verified from disk)",
                            env!("CARGO_PKG_VERSION"),
                            sp.display(),
                            canonical_root.display(),
                            verified_event_count,
                        );
                        match aivyx_channel::slack_daemon_frontend::run_slack_daemon_multi_session(
                            transport,
                            sp.clone(),
                            Some(active_role_name.clone()),
                            shutdown.clone(),
                        )
                        .await
                        {
                            Ok(()) => return Ok(()),
                            Err(e) => {
                                eprintln!(
                                    "aivyx: daemon slack session failed ({e}), \
                                     falling back to in-process."
                                );
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "aivyx: slack socket-mode connect failed ({e}), \
                             falling back to in-process."
                        );
                    }
                }
            } else {
                eprintln!("aivyx: no socket path available, using in-process mode.");
            }

            // In-process fallback (Phase 108 path).
            eprintln!(
                "aivyx {} — slack bot live (in-process)\n\
                 fs sandbox: {}\n\
                 memory: live (recall persists across restarts)\n\
                 audit: persistent ({} events verified from disk)",
                env!("CARGO_PKG_VERSION"),
                canonical_root.display(),
                verified_event_count,
            );

            let slack_config = aivyx_slack::SlackSessionConfig {
                model,
                system_prompt,
                max_tokens: DEFAULT_MAX_TOKENS,
                capabilities,
                tools,
                storage,
                tool_allowlist,
                memory_topic_prefix,
            };
            aivyx_slack::run_slack_session(
                "aivyx-slack",
                bot_token_str,
                app_token_str,
                slack_config,
                provider,
                audit,
                shutdown,
            )
            .await
            .map(|_report| ())
        }
        // Phase 135 — Voice channel dispatch. The
        // substrate (aivyx-voice crate) is feature-
        // gated through aivyx-channel; without the
        // feature, this arm surfaces an actionable
        // error pointing the operator at the right
        // `cargo install --features` invocation.
        // With the feature, the arm constructs the
        // VoiceChannel and calls into the substrate's
        // push-to-talk loop driver, which in Phase
        // 135 returns a documented "loop not yet
        // implemented" error (real audio I/O wiring
        // is operator-validation work — see
        // PHASE_135.md). The arm is structured so
        // Phase 136+ replaces the inner call with the
        // real cpal + rodio loop without changing
        // the dispatch shape.
        ChannelKind::Voice => {
            // `channel-voice-full` (defined in this crate's
            // Cargo.toml) bundles `channel-voice` plus the
            // engines via `aivyx-voice/recommended-voice`, so
            // gating on it transitively guarantees both
            // `asr-whisper-rs` and `tts-piper` are compiled.
            // The lean `channel-voice` alone pulls in the
            // substrate without engines — useful for Phase 137+
            // alternative-engine wiring; not enough for the
            // binary's default loop here.
            #[cfg(feature = "channel-voice-full")]
            {
                use aivyx_voice::asr::whisper_rs::WhisperRsEngine;
                use aivyx_voice::tts::piper::{config_from_generic, PiperEngine};
                use aivyx_voice::{
                    run_push_to_talk_loop_streaming, VoiceChannel, VoiceChannelConfig,
                };

                eprintln!(
                    "aivyx {} — voice channel (Phase 138 streaming TTS)\n\
                     fs sandbox: {}\n\
                     audit: persistent ({} events verified from disk)",
                    env!("CARGO_PKG_VERSION"),
                    canonical_root.display(),
                    verified_event_count,
                );

                // Phase 137 — build the agent stack via the
                // shared `build_agent_stack` helper so the
                // voice arm gets feature parity with Local:
                // role overrides, recall context, memory
                // prune sinks, prompt refresher,
                // system_prompt_refiner all flow through.
                //
                // The prompt_refresher closure mirrors the
                // Local arm exactly (lines 5736-5760 in this
                // file). A Phase 138+ candidate could DRY
                // this up into a small helper.
                let refresher_profile = profile.clone();
                let refresher_role_name = active_role_name.clone();
                let refresher_role_prompt =
                    role_for_envelope.system_prompt.value.clone();
                let refresher_shared = shared_persona.clone();
                let refresher_catalog = prompt_tool_catalog.clone();
                let prompt_refresher: Arc<dyn Fn() -> String + Send + Sync> =
                    Arc::new(move || {
                        let snap = refresher_shared
                            .read()
                            .expect("persona lock not poisoned at turn build");
                        let assembled = aivyx_channel::assemble_session_prompt(
                            &refresher_profile,
                            Some(&*snap),
                            &refresher_role_name,
                            &refresher_role_prompt,
                        );
                        aivyx_channel::profile_prompt::apply_ollama_prompt_strategy(
                            &assembled,
                            &refresher_catalog,
                            ollama_prompt_strategy,
                        )
                    });

                let agent_spec = aivyx_channel::session::AgentStackSpec {
                    model: model.clone(),
                    system_prompt: system_prompt.clone(),
                    max_tokens: DEFAULT_MAX_TOKENS,
                    capabilities,
                    tools,
                    tool_allowlist,
                    memory_topic_prefix,
                    role_overrides: Some(shared_role_overrides.clone()),
                    prompt_refresher: Some(prompt_refresher),
                    context_window_tokens: Some(
                        provider_kind.value.default_context_window(),
                    ),
                    prune_sink: Some(Arc::new(
                        aivyx_channel::prune_sink::MemoryPruneSink::new(
                            Arc::clone(&memory),
                        ),
                    )),
                    context_provider: recall_context.clone(),
                    system_prompt_refiner: system_prompt_refiner.clone(),
                    // Chapter K (K.4.2) — the voice channel writes to the same
                    // persistent HMAC chain as every other turn, so it gets the
                    // same pre-call dollar gate over the operator's
                    // `[budget] per_day_usd` cap. `None` when no cap is set.
                    budget_gate: aivyx_channel::budget_gate::ChannelBudgetGate::new_gate(
                        config_budget.clone(),
                        Arc::clone(&persistent_audit_for_query),
                        aivyx_cost::Pricing::with_overrides(config_pricing.clone()),
                        DEFAULT_MAX_TOKENS,
                    ),
                };
                let agent = aivyx_channel::session::build_agent_stack(
                    Arc::clone(&provider),
                    Arc::clone(&audit),
                    agent_spec,
                );

                // Build the voice channel + engines from
                // [voice] config.
                let v = &config_voice_options;
                let asr_cfg = aivyx_voice::asr::AsrConfig {
                    model_path: v.asr_model_path.clone(),
                    language: v.asr_language.clone(),
                    beam_size: v.asr_beam_size,
                };
                let asr_engine = WhisperRsEngine::new(asr_cfg).map_err(|e| {
                    format!("voice: build WhisperRsEngine: {e}")
                })?;
                let tts_cfg = aivyx_voice::tts::TtsConfig {
                    voice_path: v.tts_voice_path.clone(),
                    speaker_id: None,
                };
                let espeak_path = v.tts_espeak_data_path.clone().ok_or_else(|| {
                    "voice: [voice] tts_espeak_data_path is required for Piper TTS. \
                     Linux: `/usr/share/espeak-ng-data` (apt install espeak-ng-data). \
                     macOS: `/opt/homebrew/share/espeak-ng-data` (brew install espeak-ng)."
                        .to_string()
                })?;
                let piper_cfg = config_from_generic(&tts_cfg, espeak_path)
                    .map_err(|e| format!("voice: build PiperEngine config: {e}"))?;
                let tts_engine = PiperEngine::new(piper_cfg).map_err(|e| {
                    format!("voice: build PiperEngine: {e}")
                })?;
                let channel_cfg = VoiceChannelConfig {
                    asr_engine: v.asr_engine.clone(),
                    tts_engine: v.tts_engine.clone(),
                    asr: aivyx_voice::asr::AsrConfig {
                        model_path: v.asr_model_path.clone(),
                        language: v.asr_language.clone(),
                        beam_size: v.asr_beam_size,
                    },
                    tts: aivyx_voice::tts::TtsConfig {
                        voice_path: v.tts_voice_path.clone(),
                        speaker_id: None,
                    },
                    input_device: v.input_device.clone(),
                    output_device: v.output_device.clone(),
                    capture_debug_path: None,
                };
                let channel = Arc::new(VoiceChannel::new(channel_cfg));

                let asr_dyn: Arc<dyn aivyx_voice::asr::AsrEngine> = Arc::new(asr_engine);
                let tts_dyn: Arc<dyn aivyx_voice::tts::TtsEngine> = Arc::new(tts_engine);

                run_push_to_talk_loop_streaming(agent, channel, asr_dyn, tts_dyn)
                    .await
                    .map_err(|e| format!("voice loop: {e}"))
            }
            #[cfg(not(feature = "channel-voice-full"))]
            {
                let _ = &config_voice_options;
                Err(
                    "aivyx voice: this binary was built without the `channel-voice-full` \
                     feature (which bundles channel-voice + whisper-rs ASR + Piper TTS). \
                     Rebuild with `cargo install --features \
                     aivyx-channel/channel-voice-full aivyx-channel`. See INSTALL.md \
                     Phase 135 voice section for prerequisites (ONNX runtime,
                     espeak-ng) and Phase 136 for the integrated loop."
                        .to_string(),
                )
            }
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
    fn tui_subcommand_parses_to_tui_mode() {
        let parsed =
            parse_cli_args_from(&argv(&["tui"])).expect("`tui` must parse");
        assert_eq!(parsed.mode, CliMode::Tui);
        assert_eq!(parsed.channel, ChannelKind::Local);
        assert!(parsed.role.is_none());
        assert!(!parsed.no_daemon);
    }

    #[test]
    fn tui_accepts_role_flag() {
        let parsed = parse_cli_args_from(&argv(&["tui", "--role", "coder"]))
            .expect("`tui --role coder` must parse");
        assert_eq!(parsed.mode, CliMode::Tui);
        assert_eq!(parsed.role.as_deref(), Some("coder"));
    }

    #[test]
    fn tui_role_missing_value_is_an_error() {
        let err = parse_cli_args_from(&argv(&["tui", "--role"]))
            .expect_err("`tui --role` with no value must error");
        assert!(err.contains("--role"), "error names the flag: {err}");
    }

    #[test]
    fn tui_role_empty_value_is_an_error() {
        let err = parse_cli_args_from(&argv(&["tui", "--role", "  "]))
            .expect_err("`tui --role '  '` must error");
        assert!(err.contains("non-empty"), "error explains why: {err}");
    }

    #[test]
    fn tui_rejects_unknown_argument() {
        let err = parse_cli_args_from(&argv(&["tui", "--bogus"]))
            .expect_err("`tui --bogus` must error");
        assert!(
            err.contains("tui") && err.contains("--bogus"),
            "error names the subcommand and the bad arg: {err}"
        );
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
    // Phase 100 task 5 — registration-time gate for `fs.delete`.
    //
    // Symmetric with the `shell.exec` gate tests above: destructive
    // `fs.delete` is registered for `Local` (Trusted) only and is
    // absent from a `Telegram` (SemiTrusted) dispatch registry.
    // -----------------------------------------------------------------

    #[test]
    fn channel_local_receives_fs_delete() {
        let scratch = Scratch::new();
        let result = build_fs_delete_for_channel(ChannelKind::Local, &scratch.dir)
            .expect("local branch must build fs.delete cleanly");
        let (tool, scope) = result.expect("local must receive fs.delete");
        assert_eq!(tool.name(), "fs.delete");
        assert_eq!(scope.base(), "fs.delete");
        let qualifier = scope.qualifier().expect("scope must be qualified");
        assert!(
            qualifier.ends_with("/**"),
            "fs.delete scope must end with `/**`, got {qualifier}"
        );
    }

    #[test]
    fn channel_telegram_receives_no_fs_delete() {
        // `fs.delete` is destructive — the SemiTrusted (Telegram)
        // branch must return `None` so the tool is absent from the
        // dispatch registry entirely, the same registration-time
        // strictness `shell.exec` gets. PHASE_100.md Q3.
        let scratch = Scratch::new();
        let result = build_fs_delete_for_channel(ChannelKind::Telegram, &scratch.dir)
            .expect("telegram branch must not error — it's a no-op");
        assert!(
            result.is_none(),
            "Telegram channel must NOT receive fs.delete"
        );
    }

    // -----------------------------------------------------------------
    // Phase 102 — `aivyx tools` subcommand parse tests.
    // -----------------------------------------------------------------

    #[test]
    fn tools_subcommand_parses_with_no_window() {
        let parsed = parse_cli_args_from(&argv(&["tools"]))
            .expect("`tools` must parse");
        assert!(matches!(parsed.mode, CliMode::Tools { window_secs: None }));
    }

    #[test]
    fn tools_window_flag_parses() {
        let parsed = parse_cli_args_from(&argv(&["tools", "--window", "3600"]))
            .expect("`tools --window 3600` must parse");
        assert!(matches!(
            parsed.mode,
            CliMode::Tools {
                window_secs: Some(3600)
            }
        ));
    }

    #[test]
    fn tools_window_zero_is_an_error() {
        let err = parse_cli_args_from(&argv(&["tools", "--window", "0"]))
            .expect_err("`tools --window 0` must error");
        assert!(
            err.contains("--window"),
            "error must mention the flag: {err}"
        );
    }

    // -----------------------------------------------------------------
    // Phase 103 — `aivyx tool init` parse tests.
    // -----------------------------------------------------------------

    #[test]
    fn tool_init_subcommand_parses() {
        let parsed = parse_cli_args_from(&argv(&["tool", "init", "/tmp/x"]))
            .expect("`tool init /tmp/x` must parse");
        match parsed.mode {
            CliMode::Tool(ToolSubcommand::Init { path, force }) => {
                assert_eq!(path, PathBuf::from("/tmp/x"));
                assert!(!force);
            }
            other => panic!("expected Tool(Init), got {other:?}"),
        }
    }

    #[test]
    fn tool_init_force_flag_parses() {
        let parsed =
            parse_cli_args_from(&argv(&["tool", "init", "/tmp/x", "--force"]))
                .expect("`tool init ... --force` must parse");
        match parsed.mode {
            CliMode::Tool(ToolSubcommand::Init { force, .. }) => {
                assert!(force);
            }
            other => panic!("expected Tool(Init), got {other:?}"),
        }
    }

    #[test]
    fn tool_init_missing_path_is_error() {
        let err = parse_cli_args_from(&argv(&["tool", "init"]))
            .expect_err("`tool init` with no path must error");
        assert!(err.contains("path"), "got: {err}");
    }

    #[test]
    fn tool_with_no_subcommand_is_error() {
        let err = parse_cli_args_from(&argv(&["tool"]))
            .expect_err("`tool` alone must error");
        assert!(err.contains("subcommand"), "got: {err}");
    }

    #[test]
    fn tool_unknown_subcommand_is_error() {
        let err = parse_cli_args_from(&argv(&["tool", "doesnotexist"]))
            .expect_err("unknown `tool` subcommand must error");
        assert!(err.contains("doesnotexist"), "got: {err}");
    }

    // -----------------------------------------------------------------
    // Phase 105 — `aivyx audit export` parse tests.
    // -----------------------------------------------------------------

    #[test]
    fn audit_export_bare_parses() {
        let parsed = parse_cli_args_from(&argv(&["audit", "export"]))
            .expect("`audit export` must parse");
        assert!(matches!(
            parsed.mode,
            CliMode::Audit(AuditSubcommand::Export {
                from: None,
                limit: None,
                event_type: None,
            })
        ));
    }

    #[test]
    fn audit_export_from_flag_parses() {
        let parsed =
            parse_cli_args_from(&argv(&["audit", "export", "--from", "42"]))
                .expect("`audit export --from 42` must parse");
        match parsed.mode {
            CliMode::Audit(AuditSubcommand::Export { from, limit, .. }) => {
                assert_eq!(from, Some(42));
                assert_eq!(limit, None);
            }
            other => panic!("expected Audit(Export), got {other:?}"),
        }
    }

    #[test]
    fn audit_export_limit_flag_parses() {
        let parsed =
            parse_cli_args_from(&argv(&["audit", "export", "--limit", "100"]))
                .expect("`audit export --limit 100` must parse");
        match parsed.mode {
            CliMode::Audit(AuditSubcommand::Export { from, limit, .. }) => {
                assert_eq!(from, None);
                assert_eq!(limit, Some(100));
            }
            other => panic!("expected Audit(Export), got {other:?}"),
        }
    }

    #[test]
    fn audit_export_both_flags_parse_in_either_order() {
        let parsed = parse_cli_args_from(&argv(&[
            "audit", "export", "--from", "7", "--limit", "13",
        ]))
        .expect("`audit export --from 7 --limit 13` must parse");
        match parsed.mode {
            CliMode::Audit(AuditSubcommand::Export { from, limit, .. }) => {
                assert_eq!(from, Some(7));
                assert_eq!(limit, Some(13));
            }
            other => panic!("expected Audit(Export), got {other:?}"),
        }

        let parsed = parse_cli_args_from(&argv(&[
            "audit", "export", "--limit", "13", "--from", "7",
        ]))
        .expect("flag order must not matter");
        assert!(matches!(
            parsed.mode,
            CliMode::Audit(AuditSubcommand::Export {
                from: Some(7),
                limit: Some(13),
                event_type: None,
            })
        ));
    }

    #[test]
    fn audit_export_invalid_from_is_error() {
        let err = parse_cli_args_from(&argv(&[
            "audit", "export", "--from", "notanumber",
        ]))
        .expect_err("non-integer `--from` must error at parse time");
        assert!(err.contains("--from"), "got: {err}");
    }

    #[test]
    fn audit_export_zero_limit_is_error() {
        // `--limit 0` would emit zero entries — almost certainly
        // operator error. Reject with a hint to omit the flag.
        let err =
            parse_cli_args_from(&argv(&["audit", "export", "--limit", "0"]))
                .expect_err("`--limit 0` must error");
        assert!(err.contains("--limit"), "got: {err}");
    }

    #[test]
    fn audit_with_no_subcommand_is_error() {
        let err = parse_cli_args_from(&argv(&["audit"]))
            .expect_err("`audit` alone must error");
        assert!(err.contains("subcommand"), "got: {err}");
    }

    #[test]
    fn audit_unknown_subcommand_is_error() {
        let err = parse_cli_args_from(&argv(&["audit", "doesnotexist"]))
            .expect_err("unknown `audit` subcommand must error");
        assert!(err.contains("doesnotexist"), "got: {err}");
    }

    // -----------------------------------------------------------------
    // Phase 106 — `aivyx mcp recipes` parse tests.
    // -----------------------------------------------------------------

    #[test]
    fn mcp_recipes_bare_parses() {
        let parsed = parse_cli_args_from(&argv(&["mcp", "recipes"]))
            .expect("`mcp recipes` must parse");
        assert!(matches!(
            parsed.mode,
            CliMode::Mcp(McpSubcommand::Recipes { name: None })
        ));
    }

    #[test]
    fn mcp_recipes_with_name_parses() {
        let parsed =
            parse_cli_args_from(&argv(&["mcp", "recipes", "filesystem"]))
                .expect("`mcp recipes filesystem` must parse");
        match parsed.mode {
            CliMode::Mcp(McpSubcommand::Recipes { name }) => {
                assert_eq!(name.as_deref(), Some("filesystem"));
            }
            other => panic!("expected Mcp(Recipes), got {other:?}"),
        }
    }

    #[test]
    fn mcp_recipes_flag_in_name_position_is_error() {
        // A `--foo`-shaped token where a recipe name belongs
        // is rejected at parse time so a future flag is not
        // silently consumed as a recipe name.
        let err = parse_cli_args_from(&argv(&["mcp", "recipes", "--force"]))
            .expect_err("flag in name position must error");
        assert!(err.contains("--force"), "got: {err}");
    }

    #[test]
    fn mcp_recipes_extra_argument_is_error() {
        let err =
            parse_cli_args_from(&argv(&["mcp", "recipes", "filesystem", "junk"]))
                .expect_err("trailing extra arg must error");
        assert!(err.contains("junk"), "got: {err}");
    }

    #[test]
    fn mcp_with_no_subcommand_is_error() {
        let err = parse_cli_args_from(&argv(&["mcp"]))
            .expect_err("`mcp` alone must error");
        assert!(err.contains("subcommand"), "got: {err}");
    }

    #[test]
    fn mcp_unknown_subcommand_is_error() {
        let err = parse_cli_args_from(&argv(&["mcp", "doesnotexist"]))
            .expect_err("unknown `mcp` subcommand must error");
        assert!(err.contains("doesnotexist"), "got: {err}");
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
            require_discord_token: false,
            require_slack_tokens: false,
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
    fn parse_connect_no_service_lists() {
        let parsed = parse_cli_args_from(&argv(&["connect"]))
            .expect("connect must parse");
        assert_eq!(parsed.mode, CliMode::Connect(None));
    }

    #[test]
    fn parse_connect_with_service() {
        let parsed = parse_cli_args_from(&argv(&["connect", "gmail"]))
            .expect("connect gmail must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Connect(Some("gmail".to_string()))
        );
    }

    #[test]
    fn parse_connect_rejects_flag_and_extra_args() {
        assert!(parse_cli_args_from(&argv(&["connect", "--foo"])).is_err());
        assert!(parse_cli_args_from(&argv(&["connect", "gmail", "x"]))
            .is_err());
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

    // ----- Phase 119 Task 4 — `profile apply-hint` parser -----

    #[test]
    fn profile_apply_hint_parses_proposal_id() {
        let parsed = parse_cli_args_from(&argv(&[
            "profile",
            "apply-hint",
            "pp-abc",
        ]))
        .expect("`profile apply-hint pp-abc` must parse");
        match parsed.mode {
            CliMode::Profile(ProfileSubcommand::ApplyHint {
                proposal_id,
                yes,
            }) => {
                assert_eq!(proposal_id, "pp-abc");
                assert!(!yes, "default yes must be false");
            }
            other => panic!("unexpected mode: {other:?}"),
        }
    }

    #[test]
    fn profile_apply_hint_parses_yes_flag() {
        let parsed = parse_cli_args_from(&argv(&[
            "profile",
            "apply-hint",
            "pp-abc",
            "--yes",
        ]))
        .expect("--yes flag must parse");
        match parsed.mode {
            CliMode::Profile(ProfileSubcommand::ApplyHint { yes, .. }) => {
                assert!(yes);
            }
            other => panic!("unexpected mode: {other:?}"),
        }
    }

    #[test]
    fn profile_apply_hint_accepts_short_y_flag() {
        let parsed = parse_cli_args_from(&argv(&[
            "profile",
            "apply-hint",
            "pp-abc",
            "-y",
        ]))
        .expect("-y flag must parse");
        match parsed.mode {
            CliMode::Profile(ProfileSubcommand::ApplyHint { yes, .. }) => {
                assert!(yes);
            }
            other => panic!("unexpected mode: {other:?}"),
        }
    }

    #[test]
    fn profile_apply_hint_without_id_is_an_error() {
        let err = parse_cli_args_from(&argv(&["profile", "apply-hint"]))
            .expect_err("missing id must error");
        assert!(
            err.contains("requires a proposal id"),
            "error: {err}"
        );
    }

    // ---- Phase 177 — `aivyx loop skip` parsing ----------------

    #[test]
    fn loop_skip_parses_story_id() {
        let parsed =
            parse_cli_args_from(&argv(&["loop", "skip", "ls-abc"]))
                .expect("loop skip must parse");
        match parsed.mode {
            CliMode::Loop(LoopSubcommand::Skip { story_id }) => {
                assert_eq!(story_id, "ls-abc");
            }
            other => panic!("unexpected mode: {other:?}"),
        }
    }

    #[test]
    fn loop_skip_without_id_is_an_error() {
        let err = parse_cli_args_from(&argv(&["loop", "skip"]))
            .expect_err("missing story id must error");
        assert!(err.contains("requires a <story-id>"), "error: {err}");
    }

    #[test]
    fn loop_unknown_subcommand_lists_skip() {
        let err = parse_cli_args_from(&argv(&["loop", "frobnicate"]))
            .expect_err("unknown subcommand must error");
        assert!(err.contains("skip"), "help should mention skip: {err}");
    }

    // ---- Chapter J — `aivyx team` parsing ---------------------

    #[test]
    fn team_roster_parses() {
        let parsed = parse_cli_args_from(&argv(&["team", "roster"]))
            .expect("team roster must parse");
        assert_eq!(parsed.mode, CliMode::Team(TeamSubcommand::Roster { config: None }));
    }

    #[test]
    fn team_roster_with_config_parses_the_pack_path() {
        let parsed = parse_cli_args_from(&argv(&["team", "roster", "--config", "kitchen.toml"]))
            .expect("team roster --config must parse");
        match parsed.mode {
            CliMode::Team(TeamSubcommand::Roster { config }) => {
                assert_eq!(config.as_deref(), Some("kitchen.toml"));
            }
            other => panic!("unexpected mode: {other:?}"),
        }
    }

    #[test]
    fn team_run_parses_the_mission_and_optional_config() {
        let parsed = parse_cli_args_from(&argv(&[
            "team", "run", "close the kitchen", "--config", "kitchen.toml",
        ]))
        .expect("team run must parse");
        match parsed.mode {
            CliMode::Team(TeamSubcommand::Run { mission, config }) => {
                assert_eq!(mission, "close the kitchen");
                assert_eq!(config.as_deref(), Some("kitchen.toml"));
            }
            other => panic!("unexpected mode: {other:?}"),
        }
    }

    #[test]
    fn team_config_flag_without_a_value_is_an_error() {
        let err = parse_cli_args_from(&argv(&["team", "roster", "--config"]))
            .expect_err("--config needs a path");
        assert!(err.contains("requires a path"), "error: {err}");
    }

    #[test]
    fn team_run_without_a_mission_is_an_error() {
        let err = parse_cli_args_from(&argv(&["team", "run"]))
            .expect_err("missing mission must error");
        assert!(err.contains("requires a"), "error: {err}");
    }

    #[test]
    fn team_unknown_subcommand_lists_roster_and_run() {
        let err = parse_cli_args_from(&argv(&["team", "frobnicate"]))
            .expect_err("unknown subcommand must error");
        assert!(err.contains("roster") && err.contains("run"), "error: {err}");
    }

    #[test]
    fn team_roster_rejects_extra_args() {
        let err = parse_cli_args_from(&argv(&["team", "roster", "extra"]))
            .expect_err("roster takes no args");
        assert!(err.contains("unrecognized"), "error: {err}");
    }

    // ---- Chapter L (L.5b) — daemon team verbs --------------------

    #[test]
    fn team_start_parses_the_plan_path() {
        let parsed = parse_cli_args_from(&argv(&["team", "start", "--plan", "p.json"]))
            .expect("team start must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Team(TeamSubcommand::Start { plan_path: "p.json".into(), config: None })
        );
    }

    #[test]
    fn team_start_parses_a_positional_goal() {
        let parsed = parse_cli_args_from(&argv(&["team", "start", "close the kitchen"]))
            .expect("team start <goal> must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Team(TeamSubcommand::StartGoal {
                goal: "close the kitchen".into(),
                config: None,
            })
        );
    }

    #[test]
    fn team_start_goal_with_config_parses_the_pack() {
        let parsed = parse_cli_args_from(&argv(&[
            "team", "start", "close the kitchen", "--config", "kitchen.toml",
        ]))
        .expect("team start <goal> --config must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Team(TeamSubcommand::StartGoal {
                goal: "close the kitchen".into(),
                config: Some("kitchen.toml".into()),
            })
        );
    }

    #[test]
    fn team_start_rejects_both_goal_and_plan() {
        let err = parse_cli_args_from(&argv(&[
            "team", "start", "a goal", "--plan", "p.json",
        ]))
        .expect_err("goal + --plan is ambiguous");
        assert!(err.contains("not both"), "error: {err}");
    }

    #[test]
    fn team_start_with_no_args_is_an_error() {
        let err = parse_cli_args_from(&argv(&["team", "start"]))
            .expect_err("start needs a goal or --plan");
        assert!(err.contains("goal") && err.contains("--plan"), "error: {err}");
    }

    #[test]
    fn team_list_and_status_parse() {
        assert_eq!(
            parse_cli_args_from(&argv(&["team", "list"])).unwrap().mode,
            CliMode::Team(TeamSubcommand::List)
        );
        assert_eq!(
            parse_cli_args_from(&argv(&["team", "status"])).unwrap().mode,
            CliMode::Team(TeamSubcommand::Status { mission_id: None })
        );
        assert_eq!(
            parse_cli_args_from(&argv(&["team", "status", "m-1"])).unwrap().mode,
            CliMode::Team(TeamSubcommand::Status { mission_id: Some("m-1".into()) })
        );
    }

    #[test]
    fn team_approve_and_reject_parse_id_and_step() {
        assert_eq!(
            parse_cli_args_from(&argv(&["team", "approve", "m-1", "gate"]))
                .unwrap()
                .mode,
            CliMode::Team(TeamSubcommand::Approve {
                mission_id: "m-1".into(),
                step: "gate".into(),
            })
        );
        assert_eq!(
            parse_cli_args_from(&argv(&["team", "reject", "m-1", "gate"]))
                .unwrap()
                .mode,
            CliMode::Team(TeamSubcommand::Reject {
                mission_id: "m-1".into(),
                step: "gate".into(),
            })
        );
    }

    #[test]
    fn team_approve_without_step_is_an_error() {
        let err = parse_cli_args_from(&argv(&["team", "approve", "m-1"]))
            .expect_err("approve needs a step");
        assert!(err.contains("<step>"), "error: {err}");
    }

    #[test]
    fn team_unknown_subcommand_lists_the_daemon_verbs() {
        let err = parse_cli_args_from(&argv(&["team", "frobnicate"]))
            .expect_err("unknown subcommand must error");
        assert!(err.contains("status") && err.contains("approve"), "error: {err}");
    }

    // ---- Chapter K — `aivyx cost` parsing ---------------------

    #[test]
    fn cost_parses_all_time_and_today() {
        assert_eq!(
            parse_cli_args_from(&argv(&["cost"])).unwrap().mode,
            CliMode::Cost { today: false }
        );
        assert_eq!(
            parse_cli_args_from(&argv(&["cost", "--today"])).unwrap().mode,
            CliMode::Cost { today: true }
        );
    }

    #[test]
    fn cost_rejects_unknown_flags() {
        let err = parse_cli_args_from(&argv(&["cost", "--yesterday"]))
            .expect_err("unknown flag must error");
        assert!(err.contains("unrecognized"), "error: {err}");
    }

    #[test]
    fn profile_apply_hint_rejects_unknown_flag() {
        let err = parse_cli_args_from(&argv(&[
            "profile",
            "apply-hint",
            "pp-abc",
            "--force",
        ]))
        .expect_err("--force is not supported on apply-hint");
        assert!(
            err.contains("unrecognized flag"),
            "error: {err}"
        );
    }

    #[test]
    fn profile_apply_hint_rejects_extra_positional_args() {
        let err = parse_cli_args_from(&argv(&[
            "profile",
            "apply-hint",
            "pp-abc",
            "pp-def",
        ]))
        .expect_err("two ids must error");
        assert!(err.contains("exactly one proposal id"), "error: {err}");
    }

    // ----- Phase 119 Task 5 — `role import` parser -----

    #[test]
    fn role_import_parses_proposal_id() {
        let parsed = parse_cli_args_from(&argv(&[
            "role",
            "import",
            "pp-xyz",
        ]))
        .expect("`role import pp-xyz` must parse");
        match parsed.mode {
            CliMode::Role(RoleSubcommand::Import {
                proposal_id,
                yes,
                force,
            }) => {
                assert_eq!(proposal_id, "pp-xyz");
                assert!(!yes, "default yes must be false");
                assert!(!force, "default force must be false");
            }
            other => panic!("unexpected mode: {other:?}"),
        }
    }

    #[test]
    fn role_import_parses_yes_and_force_flags_in_either_order() {
        let parsed_a = parse_cli_args_from(&argv(&[
            "role", "import", "pp-1", "--yes", "--force",
        ]))
        .expect("`--yes --force` must parse");
        let parsed_b = parse_cli_args_from(&argv(&[
            "role", "import", "pp-1", "--force", "--yes",
        ]))
        .expect("`--force --yes` must parse");
        for parsed in [parsed_a, parsed_b] {
            match parsed.mode {
                CliMode::Role(RoleSubcommand::Import { yes, force, .. }) => {
                    assert!(yes);
                    assert!(force);
                }
                other => panic!("unexpected mode: {other:?}"),
            }
        }
    }

    #[test]
    fn role_import_without_id_is_an_error() {
        let err = parse_cli_args_from(&argv(&["role", "import"]))
            .expect_err("missing id must error");
        assert!(
            err.contains("requires a proposal id"),
            "error: {err}"
        );
    }

    #[test]
    fn role_import_rejects_unknown_flag() {
        let err = parse_cli_args_from(&argv(&[
            "role", "import", "pp-1", "--dry-run",
        ]))
        .expect_err("--dry-run is not supported");
        assert!(err.contains("unrecognized flag"), "error: {err}");
    }

    #[test]
    fn role_without_subcommand_is_an_error() {
        let err = parse_cli_args_from(&argv(&["role"]))
            .expect_err("`role` alone must error");
        assert!(err.contains("import"), "error must list import: {err}");
    }

    #[test]
    fn role_unknown_subcommand_is_an_error() {
        let err = parse_cli_args_from(&argv(&["role", "delete"]))
            .expect_err("`role delete` must error");
        assert!(
            err.contains("unrecognized role subcommand"),
            "error: {err}"
        );
    }

    // ----- Phase 119 Task 6 — `tool-relevance dump` parser -----

    #[test]
    fn tool_relevance_dump_parses_without_filter() {
        let parsed = parse_cli_args_from(&argv(&[
            "tool-relevance",
            "dump",
        ]))
        .expect("`tool-relevance dump` must parse");
        match parsed.mode {
            CliMode::ToolRelevance(ToolRelevanceSubcommand::Dump {
                keyword_key_filter,
            }) => {
                assert!(keyword_key_filter.is_none());
            }
            other => panic!("unexpected mode: {other:?}"),
        }
    }

    #[test]
    fn tool_relevance_dump_parses_keyword_key_filter() {
        let parsed = parse_cli_args_from(&argv(&[
            "tool-relevance",
            "dump",
            "--keyword-key",
            "research+deploy",
        ]))
        .expect("`--keyword-key research+deploy` must parse");
        match parsed.mode {
            CliMode::ToolRelevance(ToolRelevanceSubcommand::Dump {
                keyword_key_filter,
            }) => {
                assert_eq!(keyword_key_filter.as_deref(), Some("research+deploy"));
            }
            other => panic!("unexpected mode: {other:?}"),
        }
    }

    #[test]
    fn tool_relevance_dump_requires_value_after_keyword_key_flag() {
        let err = parse_cli_args_from(&argv(&[
            "tool-relevance",
            "dump",
            "--keyword-key",
        ]))
        .expect_err("--keyword-key without value must error");
        assert!(err.contains("requires a value"), "error: {err}");
    }

    #[test]
    fn tool_relevance_dump_rejects_unknown_flag() {
        let err = parse_cli_args_from(&argv(&[
            "tool-relevance",
            "dump",
            "--limit",
            "10",
        ]))
        .expect_err("--limit is not supported");
        assert!(err.contains("unrecognized flag"), "error: {err}");
    }

    #[test]
    fn tool_relevance_dump_rejects_positional_args() {
        let err = parse_cli_args_from(&argv(&[
            "tool-relevance",
            "dump",
            "some-key",
        ]))
        .expect_err("positional args not supported (use --keyword-key)");
        assert!(
            err.contains("does not accept positional arguments"),
            "error: {err}"
        );
    }

    #[test]
    fn tool_relevance_without_subcommand_is_an_error() {
        let err = parse_cli_args_from(&argv(&["tool-relevance"]))
            .expect_err("`tool-relevance` alone must error");
        assert!(err.contains("dump"), "error must list dump: {err}");
    }

    #[test]
    fn tool_relevance_unknown_subcommand_is_an_error() {
        let err = parse_cli_args_from(&argv(&["tool-relevance", "clear"]))
            .expect_err("`tool-relevance clear` must error");
        assert!(
            err.contains("unrecognized tool-relevance subcommand"),
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
        assert_eq!(
            parsed.mode,
            CliMode::Persona(PersonaSubcommand::List {
                filter: PersonaListFilter::All,
            }),
        );
    }

    #[test]
    fn persona_list_auto_only_flag_parses() {
        let parsed =
            parse_cli_args_from(&argv(&["persona", "list", "--auto-only"]))
                .expect("`persona list --auto-only` must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Persona(PersonaSubcommand::List {
                filter: PersonaListFilter::AutoOnly,
            }),
        );
    }

    #[test]
    fn persona_list_manual_only_flag_parses() {
        let parsed =
            parse_cli_args_from(&argv(&["persona", "list", "--manual-only"]))
                .expect("`persona list --manual-only` must parse");
        assert_eq!(
            parsed.mode,
            CliMode::Persona(PersonaSubcommand::List {
                filter: PersonaListFilter::ManualOnly,
            }),
        );
    }

    #[test]
    fn persona_list_both_filter_flags_is_an_error() {
        let err = parse_cli_args_from(&argv(&[
            "persona",
            "list",
            "--auto-only",
            "--manual-only",
        ]))
        .expect_err("mutually-exclusive flags must error");
        assert!(err.contains("mutually exclusive"), "{err}");
    }

    #[test]
    fn persona_list_unknown_arg_is_an_error() {
        let err =
            parse_cli_args_from(&argv(&["persona", "list", "--what"]))
                .expect_err("unknown flag must error");
        assert!(err.contains("--what"), "{err}");
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

    // ------------------------------------------------------------------
    // Phase 122 Task 6 — banner-line formatter for the resolved
    // Ollama prompt strategy. The formatter is pure so we don't
    // need to capture stderr; the binary calls it from
    // `print_config_banner` and emits the returned line verbatim.
    // ------------------------------------------------------------------

    fn load_phase_122_config(extra: &str) -> AivyxConfig {
        let scratch = Scratch::new();
        let toml_path = scratch.dir.join("aivyx.toml");
        let body = format!(
            "[anthropic]\n\
             api_key = \"sk-test\"\n\
             \n\
             [aivyx]\n\
             passphrase = \"test\"\n\
             {extra}\n",
        );
        std::fs::write(&toml_path, body).unwrap();
        let opts = LoadOptions {
            toml_path: Some(toml_path),
            require_api_key: false,
            require_telegram_token: false,
            require_discord_token: false,
            require_slack_tokens: false,
            role_override: None,
        };
        AivyxConfig::load_from_env_and_toml(&opts).expect("load")
    }

    #[test]
    fn phase_122_banner_line_absent_for_anthropic_provider() {
        let cfg = load_phase_122_config(
            "[agent]\nprovider = \"anthropic\"\nmodel = \"claude-haiku-4-5\"\n",
        );
        assert!(
            format_ollama_prompt_strategy_banner_line(&cfg).is_none(),
            "non-Ollama providers must not emit the strategy line"
        );
    }

    #[test]
    fn phase_124_banner_line_shows_default_for_detected_family() {
        // qwen3.6 → family "qwen3" → default FewShotExamples
        // (Phase 124 upgrade from Phase 122's StructuredInjection).
        let cfg = load_phase_122_config(
            "[agent]\nprovider = \"ollama\"\nmodel = \"qwen3.6:27b\"\n",
        );
        let line =
            format_ollama_prompt_strategy_banner_line(&cfg).expect("Ollama line");
        assert!(line.contains("\"few_shot_examples\""), "{line}");
        assert!(line.contains("family: qwen3"), "{line}");
        assert!(line.contains("default"), "{line}");
        assert!(!line.contains("override"), "{line}");
    }

    #[test]
    fn phase_122_banner_line_shows_override_when_operator_sets_one() {
        // qwen3.6 default is StructuredInjection; operator overrides
        // to "none". Line must reflect "none" + "override" so the
        // operator sees their TOML is being honored.
        let cfg = load_phase_122_config(
            "[agent]\n\
             provider = \"ollama\"\n\
             model = \"qwen3.6:27b\"\n\
             \n\
             [ollama.prompt_strategies]\n\
             qwen3 = \"none\"\n",
        );
        let line =
            format_ollama_prompt_strategy_banner_line(&cfg).expect("Ollama line");
        assert!(line.contains("\"none\""), "{line}");
        assert!(line.contains("family: qwen3"), "{line}");
        assert!(line.contains("override"), "{line}");
    }

    #[test]
    fn phase_122_banner_line_shows_undetected_family_for_unknown_model() {
        // Made-up model name that doesn't parse to any Ollama family.
        let cfg = load_phase_122_config(
            "[agent]\nprovider = \"ollama\"\nmodel = \"madeup-99:latest\"\n",
        );
        let line =
            format_ollama_prompt_strategy_banner_line(&cfg).expect("Ollama line");
        assert!(line.contains("family: undetected"), "{line}");
        // Undetected families always get strategy "none"
        // (operator-conservative).
        assert!(line.contains("\"none\""), "{line}");
    }

    #[test]
    fn phase_124_banner_line_default_for_gemma4() {
        // Pin gemma4 → "gemma4" → FewShotExamples (Phase 124
        // default upgrade from Phase 122's StructuredInjection).
        let cfg = load_phase_122_config(
            "[agent]\nprovider = \"ollama\"\nmodel = \"gemma4:31b\"\n",
        );
        let line =
            format_ollama_prompt_strategy_banner_line(&cfg).expect("Ollama line");
        assert!(line.contains("\"few_shot_examples\""), "{line}");
        assert!(line.contains("family: gemma4"), "{line}");
        assert!(line.contains("default"), "{line}");
    }

    #[test]
    fn phase_124_banner_line_shows_few_shot_examples_label_explicitly() {
        // Pin the wire label for FewShotExamples — operators
        // see this in the startup banner; the label must match
        // the TOML wire form so an operator copy-pasting from
        // the banner into their `[ollama.prompt_strategies]`
        // section gets a valid string.
        let cfg = load_phase_122_config(
            "[agent]\nprovider = \"ollama\"\nmodel = \"qwen3.6:27b\"\n",
        );
        let line =
            format_ollama_prompt_strategy_banner_line(&cfg).expect("Ollama line");
        // Spot-pinned exact wire label.
        assert!(line.contains("\"few_shot_examples\""), "{line}");
        // Confirm wire round-trip: parse the label back.
        let parsed = aivyx_config::OllamaFamilyStrategy::parse(
            "few_shot_examples",
        )
        .expect("parse");
        assert_eq!(
            parsed,
            aivyx_config::OllamaFamilyStrategy::FewShotExamples
        );
    }

    #[test]
    fn phase_122_banner_line_default_for_llama3() {
        // Pin llama3 → "llama3" → None (default; protocol-only).
        let cfg = load_phase_122_config(
            "[agent]\nprovider = \"ollama\"\nmodel = \"llama3.1:latest\"\n",
        );
        let line =
            format_ollama_prompt_strategy_banner_line(&cfg).expect("Ollama line");
        assert!(line.contains("\"none\""), "{line}");
        assert!(line.contains("family: llama3"), "{line}");
        assert!(line.contains("default"), "{line}");
    }
}

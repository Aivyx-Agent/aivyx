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
use std::path::{Path, PathBuf};
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
use aivyx_channel::{assemble_role_envelope, run_session, LocalChannel, SessionConfig};
use aivyx_config::{AivyxConfig, FieldSource, LoadOptions, Role, ToolAllowlist};
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

/// Build the backcompat floor that the binary would use at
/// startup for the given channel kind. This is the *display*
/// version: where the production code calls
/// `std::fs::canonicalize` on the sandbox root (which requires
/// the directory to exist), this helper takes the configured
/// path as-written. It returns `(floor, canonicalized)` —
/// `canonicalized` is `true` if the directory existed and
/// canonicalization succeeded, `false` if we used the as-written
/// path. Callers (currently just `render_role_envelope`) use the
/// flag to print a footnote when the displayed envelope might
/// differ from the production envelope by a symlink resolution.
///
/// This duplicates the floor-construction logic from `run()`
/// (see the `backcompat_floor` builder around the `assemble_
/// role_envelope` call site). The duplication is acceptable
/// because the `--print-role` path is a debug surface and
/// "factor into a single helper used by both sites" would
/// require pulling channel kind, sandbox path, and shell.exec
/// gate together at a layer above either call site — more
/// structural shift than a Task 4 working-session warrants.
/// If a future task moves the production floor builder into a
/// helper, this function should call into it.
fn build_display_floor(
    fs_root: &Path,
    channel_kind: ChannelKind,
) -> Result<(Vec<Scope>, bool), String> {
    let (root_string, canonicalized) = match std::fs::canonicalize(fs_root) {
        Ok(c) => (c.display().to_string(), true),
        Err(_) => (fs_root.display().to_string(), false),
    };
    let fs_read_scope = Scope::parse(&format!("fs.read:{root_string}/**"))
        .ok_or_else(|| format!("could not parse fs.read scope for sandbox {root_string}"))?;
    let fs_write_scope = Scope::parse(&format!("fs.write:{root_string}/**"))
        .ok_or_else(|| format!("could not parse fs.write scope for sandbox {root_string}"))?;
    let mut floor: Vec<Scope> = vec![
        Scope::parse("memory.read").unwrap(),
        Scope::parse("memory.write").unwrap(),
        Scope::parse("memory.forget").unwrap(),
        fs_read_scope,
        fs_write_scope,
        Scope::parse("net.fetch").unwrap(),
    ];
    if matches!(channel_kind, ChannelKind::Local) {
        floor.push(Scope::parse("shell.exec").unwrap());
    }
    Ok((floor, canonicalized))
}

/// Render the named role's effective capability envelope into a
/// human-readable string suitable for printing. Pure function —
/// returns the string instead of writing to stdout — so tests
/// can assert on its content without redirecting global state.
///
/// Output structure:
///
/// ```text
/// role: <name>
/// channel: <local|telegram>
/// parent chain: <leaf> -> <mid> -> <root>
///
/// level 1 — <name> (declared)
///   capability_scopes: <list, or "<empty - backcompat floor will be substituted>">
///   trust_ceiling: <tier>
///
/// level 2 — <parent> (declared)
///   ...
///
/// backcompat floor (substituted for empty levels):
///   <list>
/// (optional footnote about non-canonical fs root)
///
/// effective envelope:
///   <list of surviving scopes>
///
/// dropped:
///   <scope> [from <where>]  reason: <short reason>
///   ...
/// ```
///
/// The "dropped" section is the load-bearing teaching feature.
/// For each level transition, it walks both sides of the
/// intersection and lists scopes that did not survive, with a
/// short reason: either "not granted by <other side>" (the base
/// is missing entirely) or "qualifier mismatch (D4 Rule 4 —
/// qualified-held cannot grant unqualified-needed)" (the base is
/// present but the qualifier shape blocks it). This is
/// deliberately less precise than reimplementing the full D4
/// rule dispatch; it is precise enough for an operator to
/// understand why a declared scope evaporated.
fn render_role_envelope(
    role_name: &str,
    cfg: &AivyxConfig,
    channel_kind: ChannelKind,
) -> Result<String, String> {
    use std::fmt::Write as _;

    let role = cfg.roles.get(role_name).ok_or_else(|| {
        let known: Vec<&str> = cfg.roles.keys().map(String::as_str).collect();
        format!(
            "role `{role_name}` is not declared in this config. Known roles: {known:?}"
        )
    })?;

    let (floor, canonicalized) = build_display_floor(&cfg.fs_root.value, channel_kind)?;

    let mut out = String::new();
    writeln!(out, "role: {role_name}").unwrap();
    writeln!(
        out,
        "channel: {}",
        match channel_kind {
            ChannelKind::Local => "local",
            ChannelKind::Telegram => "telegram",
        }
    )
    .unwrap();

    // Walk parent chain leaf-to-root and collect each level's
    // role + the level number. The chain is bounded by the
    // validator; an unbounded loop here would be a regression
    // hazard if the validator ever ships broken, so we bound it
    // explicitly the same way `assemble_role_envelope` does.
    let mut chain: Vec<&Role> = vec![role];
    {
        let mut cursor = role.parent_role.value.as_deref();
        let mut depth = 0;
        while let Some(parent_name) = cursor {
            depth += 1;
            if depth > 64 {
                break;
            }
            let Some(parent) = cfg.roles.get(parent_name) else {
                break;
            };
            chain.push(parent);
            cursor = parent.parent_role.value.as_deref();
        }
    }
    let chain_names: Vec<&str> = chain.iter().map(|r| r.name.value.as_str()).collect();
    writeln!(out, "parent chain: {}", chain_names.join(" -> ")).unwrap();
    writeln!(out).unwrap();

    for (i, level) in chain.iter().enumerate() {
        let level_num = i + 1;
        let level_name = level.name.value.as_str();
        writeln!(out, "level {level_num} - {level_name} (declared)").unwrap();
        if level.capability_scopes.value.is_empty() {
            writeln!(
                out,
                "  capability_scopes: <empty - backcompat floor will be substituted at runtime>"
            )
            .unwrap();
        } else {
            writeln!(out, "  capability_scopes:").unwrap();
            for scope in &level.capability_scopes.value {
                writeln!(out, "    {}", scope.as_str()).unwrap();
            }
        }
        writeln!(
            out,
            "  trust_ceiling: {:?}",
            level.trust_ceiling.value
        )
        .unwrap();
        writeln!(out).unwrap();
    }

    writeln!(out, "backcompat floor (substituted for empty levels):").unwrap();
    for scope in &floor {
        writeln!(out, "  {}", scope.as_str()).unwrap();
    }
    if !canonicalized {
        writeln!(
            out,
            "  (note: fs sandbox at {} did not exist or could not be canonicalized; the floor's fs.read/fs.write scopes use the as-written path. A live binary would canonicalize through any symlinks at startup.)",
            cfg.fs_root.value.display()
        )
        .unwrap();
    }
    writeln!(out).unwrap();

    // Compute the effective envelope by reusing the production
    // assembly fn, then folding in the role's declared trust
    // ceiling — exactly the same composition the production
    // `run()` path uses at the role-resolution site.
    let role_envelope = assemble_role_envelope(role, &cfg.roles, &floor);
    let role_tier_ceiling = role.trust_ceiling.value.default_ceiling();
    let effective = role_envelope.intersect(role_tier_ceiling);

    writeln!(
        out,
        "effective envelope (after intersection chain + role tier ceiling {:?}):",
        role.trust_ceiling.value
    )
    .unwrap();
    let effective_strs: Vec<&str> = effective.iter().map(|s| s.as_str()).collect();
    if effective_strs.is_empty() {
        writeln!(out, "  <empty - this role has no live capabilities>").unwrap();
    } else {
        for s in &effective_strs {
            writeln!(out, "  {s}").unwrap();
        }
    }
    writeln!(out).unwrap();

    // Compute dropped scopes — but only those that represent a
    // *surprise* for the operator, not those they declared away
    // on purpose by attenuating an ancestor. Two surfaces qualify
    // as "surprise":
    //
    // 1. Scopes the **active (leaf) role itself** declared but
    //    that didn't survive intersection. This is the
    //    SemiTrusted-fs.read footgun: the operator wrote
    //    `["fs.read", ...]` and the ceiling silently stripped
    //    it. They asked for it explicitly; getting nothing back
    //    deserves a loud explanation.
    //
    // 2. **Backcompat floor** scopes that didn't survive — but
    //    only when at least one level in the chain is empty,
    //    because that's the only path that pulls the floor into
    //    the assembly. This is the empty-child surprise: the
    //    operator wrote `parent_role = "researcher"` thinking
    //    they'd get researcher's view, and instead the floor
    //    leaked in and got partially stripped.
    //
    // Drops from *ancestor* levels (level >= 2) are *not*
    // reported. Those are by-design attenuations: if `coder`
    // omits `net.fetch` and `default` declares it, that's coder
    // narrowing the envelope on purpose. Listing it as "dropped"
    // would conflate intent with surprise and bury the actual
    // surprises in noise.
    writeln!(out, "dropped (surprises only — scopes the active role or backcompat floor declared that did not survive intersection; intentional ancestor-level attenuations are not listed):").unwrap();
    let mut any_dropped = false;

    // Surface (1): drops from the active/leaf role.
    let leaf = chain[0];
    let leaf_name = leaf.name.value.as_str();
    for scope in &leaf.capability_scopes.value {
        if effective.iter().any(|s| s == scope) || effective.grants(scope) {
            continue;
        }
        any_dropped = true;
        let reason = drop_reason_for(scope, &effective);
        writeln!(
            out,
            "  {} [active role {leaf_name}]  reason: {reason}",
            scope.as_str()
        )
        .unwrap();
    }

    // Surface (2): drops from the floor (only when at least one
    // chain level is empty — otherwise the floor was never
    // substituted in and listing its drops would be misleading).
    let any_empty = chain.iter().any(|r| r.capability_scopes.value.is_empty());
    if any_empty {
        for scope in &floor {
            if effective.grants(scope) || effective.iter().any(|s| s.is_granted_by(scope)) {
                continue;
            }
            any_dropped = true;
            let reason = drop_reason_for(scope, &effective);
            writeln!(
                out,
                "  {} [floor]  reason: {reason}",
                scope.as_str()
            )
            .unwrap();
        }
    }
    if !any_dropped {
        writeln!(out, "  <none - every scope the active role declared survived intersection>").unwrap();
    }

    Ok(out)
}

/// Produce a short human-readable reason for why `dropped` is
/// not in `effective`. Three cases, in priority order:
///
/// 1. The base does not appear at all in `effective`. Reason:
///    "no scope with this base in the effective envelope". This
///    is the common case (the intersection chain stripped every
///    scope sharing the base).
/// 2. The base appears, but only in a *more* qualified form than
///    `dropped`, i.e. `dropped` is unqualified and the effective
///    set has a path-qualified or url-prefix-qualified
///    counterpart. Reason: "qualified-held cannot grant
///    unqualified-needed (D4 Rule 4)".
/// 3. Fallback: "qualifier shape mismatch with surviving scopes"
///    — covers the rare cases where two qualified scopes share a
///    base but have different qualifier kinds.
///
/// This is deliberately less precise than full D4 rule dispatch.
/// The goal is to give an operator a hint, not a formal proof.
fn drop_reason_for(dropped: &Scope, effective: &CapabilitySet) -> String {
    let base = dropped.base();
    let same_base: Vec<&Scope> = effective.iter().filter(|s| s.base() == base).collect();
    if same_base.is_empty() {
        return format!("no scope with base `{base}` in the effective envelope");
    }
    if dropped.qualifier().is_none() {
        return format!(
            "qualified-held cannot grant unqualified-needed (D4 Rule 4); effective envelope has only qualified `{base}` scopes: {:?}",
            same_base.iter().map(|s| s.as_str()).collect::<Vec<_>>()
        );
    }
    format!(
        "qualifier shape mismatch with surviving `{base}` scopes: {:?}",
        same_base.iter().map(|s| s.as_str()).collect::<Vec<_>>()
    )
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
        print_role,
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
    /// Phase 13 Task 4 — `--print-role <name>`. A debug-mode flag
    /// that loads the config, walks the named role's inheritance
    /// chain, and prints a structured rendering of the effective
    /// capability envelope (declared scopes per level, the
    /// substituted backcompat floor, the final intersected set,
    /// and a "dropped" diff explaining which scopes were removed
    /// at each layer and why). Exits without starting a session,
    /// without deriving the master key, and without opening the
    /// encrypted store. `None` means "no print mode — run a normal
    /// session." `Some(name)` means "render this role and exit."
    /// Composes with `--channel` (the floor differs per channel),
    /// is mutually exclusive with `--verify-only`.
    print_role: Option<String>,
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
/// - `aivyx --print-role <name>` — Phase 13 Task 4. Debug-mode
///   flag. Loads the config, renders the named role's capability
///   envelope (declared scopes per inheritance level, the
///   substituted backcompat floor, the final intersected set, and
///   a "dropped" diff with reasons), then exits. Does not derive
///   the master key, open the encrypted store, or start a session.
///   Composes with `--channel local|telegram` to render the floor
///   the matching session would use. Mutually exclusive with
///   `--verify-only` (different exit modes).
///
/// `--verify-only` and `--channel` are mutually exclusive: verify
/// mode is a read-only forensic surface and has nothing to do with
/// which channel the live session would run on. Combining them is
/// an operator error we flag explicitly rather than picking a
/// silent winner.
///
/// `--verify-only` and `--print-role` are also mutually exclusive
/// for the same structural reason: each is its own exit mode.
/// `--print-role` and `--channel` *do* compose — channel selection
/// determines which floor is shown.
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
    let mut print_role: Option<String> = None;

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
            other => {
                return Err(format!(
                    "unrecognized argument: `{other}`. \
                     Supported flags: --verify-only, --channel <local|telegram>, --role <name>, --print-role <name>"
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

    Ok(CliArgs {
        verify_only,
        channel,
        role,
        print_role,
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
        assert_eq!(parsed.print_role.as_deref(), Some("junior_researcher"));
        assert!(!parsed.verify_only);
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
        assert_eq!(parsed.print_role.as_deref(), Some("researcher"));
    }

    /// Render `coder` against `examples/aivyx.toml`. Verifies the
    /// structural elements (header, parent chain, level breakdown,
    /// effective envelope) and pins that the documented six scopes
    /// each appear in the rendered output. Coder has no dropped
    /// scopes, so the dropped block reads "<none - every declared
    /// scope survived intersection>".
    #[test]
    fn print_role_renders_coder_envelope_against_example_config() {
        let cfg = load_example_config();
        let rendered = render_role_envelope("coder", &cfg, ChannelKind::Local)
            .expect("coder must render");

        assert!(rendered.contains("role: coder"), "header line: {rendered}");
        assert!(
            rendered.contains("parent chain: coder -> default"),
            "parent chain line: {rendered}"
        );
        assert!(
            rendered.contains("level 1 - coder (declared)"),
            "level header for coder: {rendered}"
        );
        assert!(
            rendered.contains("level 2 - default (declared)"),
            "level header for default: {rendered}"
        );
        assert!(
            rendered.contains("trust_ceiling: Trusted"),
            "trust ceiling label: {rendered}"
        );
        for scope in &[
            "fs.read", "fs.write", "memory.read", "memory.write", "memory.forget", "shell.exec",
            // Phase 14 Task 2 — `coder`'s declared envelope now
            // includes `role.switch:researcher` so the worked
            // example demonstrates the sub-agent role-switching
            // primitive from PRODUCT.md P1. The scope survives
            // intersection with `default`'s unqualified
            // `role.switch` (Rule 2) and with CEILING_TRUSTED's
            // unqualified `role.switch` (Rule 2 again), so it
            // must appear in the rendered effective envelope.
            "role.switch:researcher",
        ] {
            assert!(
                rendered.contains(scope),
                "expected scope `{scope}` somewhere in render: {rendered}"
            );
        }
        assert!(
            rendered.contains("<none - every scope the active role declared survived intersection>"),
            "coder has no dropped scopes; render must say so: {rendered}"
        );
    }

    /// Render `junior_researcher`. This is the load-bearing test:
    /// it pins that the rendered output mechanically surfaces the
    /// "empty-child surprise" by showing both
    /// (a) the empty `capability_scopes` line for the junior
    /// level, and
    /// (b) at least one *dropped* entry in the dropped section
    /// sourced from the backcompat floor, because researcher
    /// declares no `fs.write` and no `shell.exec` at all — so
    /// those floor scopes cannot be granted upward and get
    /// reported as drops. (Note: the floor's unqualified
    /// `net.fetch` does NOT drop, because researcher's
    /// `net.fetch:url-prefix:...` is granted by it via D4 Rule 2.
    /// The surprise is specifically that fs.write and shell.exec
    /// silently disappear, not net.fetch.) If a future refactor
    /// makes `--print-role` claim "no dropped scopes" for a
    /// junior_researcher run, the operator loses the only
    /// surface that exposes the surprise *before* production.
    #[test]
    fn print_role_renders_junior_researcher_with_visible_drops() {
        let cfg = load_example_config();
        let rendered = render_role_envelope("junior_researcher", &cfg, ChannelKind::Local)
            .expect("junior_researcher must render");

        assert!(
            rendered.contains("role: junior_researcher"),
            "header: {rendered}"
        );
        assert!(
            rendered.contains("parent chain: junior_researcher -> researcher -> default"),
            "three-level parent chain: {rendered}"
        );
        assert!(
            rendered.contains("<empty - backcompat floor will be substituted at runtime>"),
            "empty-capability-scopes line for junior level: {rendered}"
        );
        // The dropped block must mention at least one of the
        // surprises documented in `examples/aivyx.toml`. We assert
        // the strongest signal: shell.exec from the floor gets
        // dropped (researcher does not declare it), with a reason
        // line that names the floor.
        assert!(
            rendered.contains("shell.exec [floor]"),
            "shell.exec must be reported as dropped from the floor: {rendered}"
        );
        // fs.write from the floor should also drop: researcher
        // declares no fs.write at all (not even qualified), so
        // the floor's `fs.write:<sandbox>/**` cannot be granted
        // upward through researcher's declared set.
        assert!(
            rendered.contains("fs.write") && rendered.contains("[floor]"),
            "fs.write (floor-qualified) must be reported as dropped from the floor: {rendered}"
        );
    }

    /// Render `researcher` and verify the dropped block stays
    /// empty for it (researcher's declared scopes are all granted
    /// by default and survive CEILING_TRUSTED intact). This test
    /// exists as a *contrast* to the junior_researcher test: it
    /// pins that a "cleanly attenuated" role does not produce
    /// false-positive drop noise.
    #[test]
    fn print_role_renders_researcher_with_no_drops() {
        let cfg = load_example_config();
        let rendered = render_role_envelope("researcher", &cfg, ChannelKind::Local)
            .expect("researcher must render");

        assert!(rendered.contains("role: researcher"));
        assert!(rendered.contains("parent chain: researcher -> default"));
        assert!(
            rendered.contains("<none - every scope the active role declared survived intersection>"),
            "researcher's declared scopes all survive intersection; \
             the dropped block must say so explicitly to avoid false-positive \
             noise: {rendered}"
        );
    }

    /// Asking for an unknown role name produces a typed error
    /// listing the known roles. Mirrors the load-time `UnknownRole`
    /// behavior the `--role` flag already exposes.
    #[test]
    fn print_role_unknown_name_lists_known_roles_in_error() {
        let cfg = load_example_config();
        let err = render_role_envelope("nonsense", &cfg, ChannelKind::Local)
            .expect_err("unknown role must error");
        assert!(
            err.contains("nonsense"),
            "error must echo the requested name: {err}"
        );
        assert!(
            err.contains("coder"),
            "error must list known roles so the operator can spot a typo: {err}"
        );
    }
}

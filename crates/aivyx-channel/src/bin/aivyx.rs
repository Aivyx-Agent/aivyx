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
//!       → AuditBridge<HmacChainLog>
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
//!   derivation feeds on. Phase 5 Q2 resolved to env-var-only for
//!   this phase; an interactive prompt is a follow-up. Required
//!   whenever storage is enabled (i.e., always in the binary path).
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
//! ## What this binary is not
//!
//! - It does not persist the audit chain. The `HmacChainLog` still
//!   uses an ephemeral per-process key. Phase 5 task 4 added the
//!   encrypted store for *session metadata* (`KeyDomain::Sessions`);
//!   Phase 6 task 4 added memory (`KeyDomain::Memory`). Audit
//!   persistence (`KeyDomain::Audit`) is the remaining hardening
//!   item on the ROADMAP for Phase 7+.
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

use std::io::{self};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use secrecy::SecretString;

use aivyx_audit::{AuditBridge, HmacChainLog};
use aivyx_capability::{CapabilitySet, Scope};
use aivyx_channel::passphrase::{derive_master_key, PassphraseSource, DEFAULT_ENV_VAR};
use aivyx_channel::{run_session, LocalChannel, SessionConfig};
use aivyx_core::{
    AuditHook, FsReadToolConfig, FsWriteToolConfig, Tool, ToolRegistry,
};
use aivyx_crypto::Argon2Params;
use aivyx_memory::{Memory, MemoryForgetTool, MemoryReadTool, MemoryWriteTool, RedbMemory};
use aivyx_llm::anthropic::{AnthropicConfig, AnthropicProvider};
use aivyx_llm::LlmProvider;
use aivyx_storage::{RedbStorage, Storage, StorageConfig};

const DEFAULT_MODEL: &str = "claude-haiku-4-5-20251001";
const DEFAULT_SYSTEM_PROMPT: &str =
    "You are Aivyx, a terse and thoughtful assistant running in a local terminal.";
const DEFAULT_MAX_TOKENS: u32 = 1024;
const PROMPT: &str = "> ";

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
    // ---- Config -------------------------------------------------------
    let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
        "ANTHROPIC_API_KEY is not set. Export it and retry: \
         `export ANTHROPIC_API_KEY=sk-ant-...`"
            .to_string()
    })?;
    let api_key = SecretString::from(api_key);

    let model = std::env::var("AIVYX_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    let system_prompt = std::env::var("AIVYX_SYSTEM_PROMPT")
        .unwrap_or_else(|_| DEFAULT_SYSTEM_PROMPT.to_string());

    // Sandbox root for the filesystem tools. `AIVYX_FS_ROOT` overrides;
    // otherwise default to `$HOME/aivyx-sandbox`. Create the directory
    // at startup if missing — a fresh install should "just work" without
    // the user having to mkdir a magic path. `FsReadToolConfig::build()`
    // will canonicalize and reject non-directories, so we don't need a
    // second layer of validation here.
    let fs_root = resolve_fs_root()?;
    std::fs::create_dir_all(&fs_root)
        .map_err(|e| format!("failed to create fs sandbox root {fs_root:?}: {e}"))?;

    // Resolve the encrypted store path + its sidecar salt file. Both
    // live under `$XDG_DATA_HOME/aivyx/` by default; the binary mkdirs
    // the parent so a fresh install "just works" the same way
    // `fs_root` does above. Canonicalization happens inside
    // `RedbStorage::open` — we only need the raw path here.
    let storage_path = resolve_storage_path()?;
    if let Some(parent) = storage_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!("failed to create storage parent directory {parent:?}: {e}")
        })?;
    }
    let salt_path = salt_path_for(&storage_path);

    // ---- Master key ---------------------------------------------------
    // Argon2id over the `AIVYX_PASSPHRASE` env var, using the sidecar
    // salt file (generated on first run, persisted plaintext — salts
    // are not secret per Argon2id design). Raw passphrase bytes never
    // leave `derive_master_key`; we get back a zero-on-drop `MasterKey`.
    let master_key = derive_master_key(
        PassphraseSource::Env {
            var_name: DEFAULT_ENV_VAR.to_string(),
        },
        &salt_path,
        Argon2Params::d7_default(),
    )
    .map_err(|e| format!("failed to derive master key: {e}"))?;

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
        run_async(api_key, model, system_prompt, fs_root, storage).await
    })
}

/// Resolve the filesystem sandbox root.
///
/// Priority:
/// 1. `AIVYX_FS_ROOT` env var, if set and non-empty.
/// 2. `$HOME/aivyx-sandbox` otherwise.
///
/// Returns an error if neither is available (no `HOME` and no override).
fn resolve_fs_root() -> Result<PathBuf, String> {
    if let Ok(explicit) = std::env::var("AIVYX_FS_ROOT") {
        if !explicit.is_empty() {
            return Ok(PathBuf::from(explicit));
        }
    }
    let home = std::env::var("HOME").map_err(|_| {
        "HOME is not set and AIVYX_FS_ROOT is not set — cannot locate a sandbox root. \
         Export one of them and retry."
            .to_string()
    })?;
    Ok(PathBuf::from(home).join("aivyx-sandbox"))
}

/// Resolve the encrypted store path (Phase 5 task 4).
///
/// Priority:
/// 1. `AIVYX_STORAGE_PATH` env var, if set and non-empty.
/// 2. `$XDG_DATA_HOME/aivyx/store.redb` if `XDG_DATA_HOME` is set.
/// 3. `$HOME/.local/share/aivyx/store.redb` otherwise.
///
/// Returns an error if none of the above yield a path (i.e. no `HOME`
/// and no explicit override).
fn resolve_storage_path() -> Result<PathBuf, String> {
    if let Ok(explicit) = std::env::var("AIVYX_STORAGE_PATH") {
        if !explicit.is_empty() {
            return Ok(PathBuf::from(explicit));
        }
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("aivyx").join("store.redb"));
        }
    }
    let home = std::env::var("HOME").map_err(|_| {
        "HOME is not set and neither AIVYX_STORAGE_PATH nor XDG_DATA_HOME is set — \
         cannot locate a default storage path. Export one of them and retry."
            .to_string()
    })?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("aivyx")
        .join("store.redb"))
}

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

async fn run_async(
    api_key: SecretString,
    model: String,
    system_prompt: String,
    fs_root: PathBuf,
    storage: Arc<dyn Storage>,
) -> Result<(), String> {
    // ---- Provider -----------------------------------------------------
    let anthropic = AnthropicProvider::new(AnthropicConfig::new(api_key))
        .map_err(|e| format!("failed to build Anthropic provider: {e}"))?;
    let provider: Arc<dyn LlmProvider> = Arc::new(anthropic);

    // ---- Audit --------------------------------------------------------
    // HmacChainLog uses an ephemeral per-process key. Phase 5's
    // encrypted storage phase will wire this to a persisted key
    // derived from the user's passphrase. For now the in-memory chain
    // is enough to prove the audit seam works end-to-end.
    let audit_key: [u8; 32] = rand_bytes_from_os()?;
    let audit: Arc<dyn AuditHook> =
        Arc::new(AuditBridge::new(HmacChainLog::new(audit_key.to_vec())));

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
    let memory_write = MemoryWriteTool::new(Arc::clone(&memory));
    let memory_forget = MemoryForgetTool::new(Arc::clone(&memory));

    let tools: Arc<ToolRegistry> = Arc::new(ToolRegistry::new(vec![
        Arc::new(fs_read) as Arc<dyn Tool>,
        Arc::new(fs_write) as Arc<dyn Tool>,
        Arc::new(memory_read) as Arc<dyn Tool>,
        Arc::new(memory_write) as Arc<dyn Tool>,
        Arc::new(memory_forget) as Arc<dyn Tool>,
    ]));

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
    let capabilities = CapabilitySet::from_scopes([
        Scope::parse("memory.read").unwrap(),
        Scope::parse("memory.write").unwrap(),
        Scope::parse("memory.forget").unwrap(),
        fs_read_scope,
        fs_write_scope,
    ]);

    // ---- Channel + signal handler ------------------------------------
    // One LocalChannel per process: its SessionId is the session the
    // user is in, and re-creating it per turn would make the LLM lose
    // conversation context across turns (which the planner keys on
    // SessionId-derived history).
    let channel = LocalChannel::new("aivyx-cli", io::stdout());
    let token_slot = channel.token_slot();

    // Signal task: first ctrl-C during a turn cancels the turn; a
    // second ctrl-C exits the process. We re-read the current token
    // from the slot on every ctrl-C so that turn-N+1 sees a fresh
    // token after turn-N's reset_cancellation() call (inside
    // `run_session`).
    tokio::spawn(async move {
        loop {
            if tokio::signal::ctrl_c().await.is_err() {
                // Signal listener broke — bail rather than hanging.
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

    // ---- Session ------------------------------------------------------
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
             memory: live (recall persists across restarts)",
            env!("CARGO_PKG_VERSION"),
            canonical_root.display(),
        )),
    };

    // Lock stdin for the whole session. `io::Stdin::lock` returns a
    // `StdinLock<'static>` on stable, so this binds for the whole
    // `run_session` call.
    let stdin = io::stdin();
    let reader = stdin.lock();

    run_session(provider, audit, session_config, channel, reader)
        .await
        .map(|_report| ())
}

/// Pull 32 bytes of OS entropy without adding a new crate dep. Uses
/// `getrandom` indirectly via the standard library's thread-local RNG
/// on every supported platform.
fn rand_bytes_from_os() -> Result<[u8; 32], String> {
    // The standard library's `HashMap` seed pulls from the OS RNG
    // transitively, but there's no public API. Read from /dev/urandom
    // directly — it's guaranteed present on every Unix target the
    // workspace supports, and the single call is simple enough to
    // justify skipping a dep for a 32-byte read.
    use std::io::Read;
    let mut f = std::fs::File::open("/dev/urandom")
        .map_err(|e| format!("failed to open /dev/urandom: {e}"))?;
    let mut buf = [0u8; 32];
    f.read_exact(&mut buf)
        .map_err(|e| format!("failed to read /dev/urandom: {e}"))?;
    Ok(buf)
}

//! # aivyx-config
//!
//! Phase 9 Task 3 (Fork B) — unified configuration layer.
//!
//! Before this crate landed, `crates/aivyx-channel/src/bin/aivyx.rs`
//! read ten environment variables directly via `std::env::var`, each
//! with its own parsing helper and error message. The per-variable
//! helpers worked fine at Phase 3 (when there were two or three), but
//! by the end of Phase 8 the binary had accreted `ANTHROPIC_API_KEY`,
//! `AIVYX_MODEL`, `AIVYX_SYSTEM_PROMPT`, `AIVYX_FS_ROOT`,
//! `AIVYX_STORAGE_PATH`, `AIVYX_MEMORY_MAX_PER_TOPIC`,
//! `AIVYX_PASSPHRASE`, `AIVYX_TELEGRAM_TOKEN`, `AIVYX_TELEGRAM_CHAT_ID`,
//! and the platform-level `HOME` / `XDG_DATA_HOME`. Every new adapter
//! added at least one more. The env-var sprawl was a compounding debt:
//! each tweak touched three places (parser, docs comment, startup
//! banner), and the three were easy to let drift.
//!
//! `aivyx-config` replaces that sprawl with a single typed
//! [`AivyxConfig`] object and a typed [`ConfigError`]. The loader
//! reads from three sources, in fall-through order:
//!
//! 1. **Environment variables** — highest priority, unchanged names
//!    so existing deployments keep working.
//! 2. **TOML file** (default path: `./aivyx.toml`) — second priority,
//!    for operators who want a readable config file.
//! 3. **Encrypted secrets store** — third priority, read from
//!    [`aivyx_storage::KeyDomain::Secrets`] after the store opens.
//!    This source is only consulted for secret-bearing fields
//!    (`anthropic_api_key`, `telegram.token`, `aivyx_passphrase`) —
//!    a plaintext setting like `model` has no business living in an
//!    encrypted key-value row.
//!
//! ## Why two phases
//!
//! Secret hydration is async (the storage layer's `DomainHandle::get`
//! is `async` because redb calls land on a blocking worker). Env vars
//! and TOML parsing are sync. A single async loader would force every
//! caller — especially unit tests for env/TOML precedence — to pull
//! in `tokio` and an async runtime just to exercise sync parsing.
//!
//! The two-phase API splits the work:
//!
//! 1. [`AivyxConfig::load_from_env_and_toml`] — sync, fills every
//!    field that env or TOML can supply. Secret-bearing fields that
//!    were found (e.g. `ANTHROPIC_API_KEY` exported in the
//!    environment) come back populated; secrets that weren't found
//!    stay `None`.
//! 2. [`AivyxConfig::hydrate_secrets_from_store`] — async, called
//!    once the storage layer is open. Fills any remaining `None`
//!    secret fields from `KeyDomain::Secrets`. A secret that was
//!    already populated by env/TOML is left alone — env wins.
//! 3. [`AivyxConfig::validate`] — final gate. Checks that every
//!    field required by the caller's [`LoadOptions`] is populated.
//!    Returns [`ConfigError::Missing`] with the field name if any
//!    required slot is still empty.
//!
//! Each populated field carries its [`FieldSource`] so the binary
//! can print a "source: env / toml / encrypted-store / default"
//! line in its startup banner. An operator debugging a surprising
//! value ("why is the model wrong?") can point at the banner and
//! see which source won the precedence race.
//!
//! ## What does NOT live in this crate
//!
//! The interactive passphrase prompt. That is a binary concern: the
//! config crate should not depend on `rpassword`, `isatty`, or any
//! terminal I/O — it reports "passphrase not found in any source" and
//! the binary decides whether to prompt (tty branch) or bail
//! (non-tty branch). This keeps `aivyx-config` headless and testable
//! with zero real-terminal setup.
//!
//! The `HOME`/`XDG_DATA_HOME` default-path logic also stays partly in
//! the binary: [`AivyxConfig`] stores resolved paths (what the loader
//! decided), but the *resolution* (if env didn't set, look in XDG,
//! fall back to HOME) happens once in [`AivyxConfig::load_from_env_and_toml`]
//! so the binary can delete its hand-rolled `resolve_fs_root` /
//! `resolve_storage_path` helpers.

// `deny` rather than `forbid` so the `tests` module can carry a
// narrowly-scoped `#[allow(unsafe_code)]` on the env-var mutation
// helpers. Rust 2024 edition made `std::env::set_var` unsafe, and
// the only *safe* workaround would be to refactor every test to
// drive the loader through an injected "EnvLike" trait seam —
// which would bloat the public API for test-only reasons. The
// narrow allow is the lesser evil.
#![deny(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use secrecy::SecretString;
use serde::Deserialize;

use aivyx_capability::{Scope, TrustTier};
use aivyx_storage::{KeyDomain, Storage};

// --------------------------------------------------------------------
// FieldSource & Sourced<T>
// --------------------------------------------------------------------

/// Which source a field's value came from. Stored alongside every
/// populated field so the binary's startup banner can show provenance.
///
/// Precedence at load time is **Env → Toml → EncryptedStore**, so a
/// field tagged [`FieldSource::Env`] won over a TOML entry for the
/// same key and over a row in `KeyDomain::Secrets`. [`FieldSource::Default`]
/// means no source supplied a value and the loader fell back to a
/// hard-coded default (e.g. [`DEFAULT_MODEL`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldSource {
    /// Read from a process environment variable.
    Env,
    /// Read from a TOML file (typically `./aivyx.toml`).
    Toml,
    /// Read from the encrypted `KeyDomain::Secrets` store. Only secret-
    /// bearing fields can come from this source.
    EncryptedStore,
    /// No source supplied a value; the loader used a hard-coded default.
    Default,
}

/// A loaded config value plus the [`FieldSource`] it came from.
///
/// Used for every non-secret field on [`AivyxConfig`]. Secret-bearing
/// fields use [`SourcedSecret`] instead so the [`SecretString`] wrapper
/// keeps them out of any accidental `Debug` impl.
#[derive(Debug, Clone)]
pub struct Sourced<T> {
    pub value: T,
    pub source: FieldSource,
}

impl<T> Sourced<T> {
    pub fn new(value: T, source: FieldSource) -> Self {
        Self { value, source }
    }
}

/// A loaded secret value plus its [`FieldSource`]. Wraps
/// [`SecretString`] — the inner value never materializes in a `Debug`
/// impl. The struct's own `Debug` is hand-written to redact the value.
pub struct SourcedSecret {
    pub value: SecretString,
    pub source: FieldSource,
}

impl SourcedSecret {
    pub fn new(value: SecretString, source: FieldSource) -> Self {
        Self { value, source }
    }
}

impl std::fmt::Debug for SourcedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourcedSecret")
            .field("value", &"<redacted>")
            .field("source", &self.source)
            .finish()
    }
}

impl Clone for SourcedSecret {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            source: self.source,
        }
    }
}

// --------------------------------------------------------------------
// Hard-coded defaults
// --------------------------------------------------------------------

/// Default Anthropic model. Matches the Phase 3 `DEFAULT_MODEL` that
/// the binary previously owned. Moved here so tests can reference it
/// without reaching into the binary crate.
pub const DEFAULT_MODEL: &str = "claude-haiku-4-5-20251001";

/// Default system prompt. Matches the Phase 3 value.
pub const DEFAULT_SYSTEM_PROMPT: &str =
    "You are Aivyx, a terse and thoughtful assistant running in a local terminal.";

/// Default per-topic memory-write tripwire. Matches
/// [`aivyx_memory::DEFAULT_MAX_PER_TOPIC`] (10_000) by value. We pin
/// the constant here rather than re-exporting from `aivyx-memory` to
/// keep the dep graph one-way: `aivyx-config` does not depend on
/// `aivyx-memory`, so a future change to one does not force the other
/// to rebuild. If the two ever diverge the test in `src/tests.rs`
/// exposes the drift.
pub const DEFAULT_MEMORY_MAX_PER_TOPIC: usize = 10_000;

/// Name of the implicit role synthesized when a loaded config has no
/// explicit `[[role]]` entries. Task 1 of Phase 11 introduced the
/// [`Role`] primitive; the backwards-compatibility bridge synthesizes
/// a single role under this name from the legacy top-level
/// [`AivyxConfig::system_prompt`] field so existing config files keep
/// working with zero edits.
///
/// Also the fall-through default for [`AivyxConfig::active_role`] when
/// neither [`LoadOptions::role_override`] nor the `AIVYX_ROLE` env var
/// supplies a value.
pub const DEFAULT_ROLE_NAME: &str = "default";

/// Which LLM provider backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Anthropic,
    #[serde(alias = "openai")]
    OpenAi,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderKind::Anthropic => f.write_str("anthropic"),
            ProviderKind::OpenAi => f.write_str("openai"),
        }
    }
}

// --------------------------------------------------------------------
// ConfigError
// --------------------------------------------------------------------

/// Every failure mode of [`AivyxConfig::load_from_env_and_toml`],
/// [`AivyxConfig::hydrate_secrets_from_store`], and
/// [`AivyxConfig::validate`] funnels through this type.
///
/// Each variant carries enough context to produce an actionable error
/// message: the missing field name, the invalid value, the TOML path,
/// etc. The binary converts `ConfigError` to its own `String`-typed
/// error via the blanket `From<ConfigError> for String` impl below.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A field required by [`LoadOptions`] was not found in any source.
    /// Typically hit by [`AivyxConfig::validate`] after env+TOML+store
    /// have all been consulted.
    #[error(
        "required config field `{field}` missing from all sources \
         (env, TOML, encrypted store)"
    )]
    Missing {
        /// The canonical field name, matching the TOML key path or the
        /// env var name the user is most likely to recognize.
        field: &'static str,
    },

    /// A source supplied a value but it failed to parse (e.g. a
    /// non-integer `AIVYX_MEMORY_MAX_PER_TOPIC` or a non-`i64`
    /// `AIVYX_TELEGRAM_CHAT_ID`).
    #[error("field `{field}` had invalid value: {reason}")]
    Invalid {
        field: &'static str,
        reason: String,
    },

    /// The TOML file at `path` could not be parsed as TOML.
    #[error("TOML parse failed at {path:?}: {reason}")]
    TomlParse { path: PathBuf, reason: String },

    /// The TOML file at `path` existed but could not be read from disk.
    /// A *missing* TOML file is not an error — it is simply absent from
    /// the fall-through chain.
    #[error("TOML file I/O error at {path:?}: {reason}")]
    TomlIo { path: PathBuf, reason: String },

    /// A default-path resolver needed `$HOME` and did not find it in
    /// the environment. Covers the `fs_root` and `storage_path`
    /// defaults that the Phase 8 binary previously raised from its
    /// own `resolve_fs_root` / `resolve_storage_path` helpers.
    #[error(
        "no HOME and no explicit override for field `{field}` \
         — export HOME or set an explicit path and retry"
    )]
    NoHome { field: &'static str },

    /// A storage error while reading from `KeyDomain::Secrets` during
    /// [`AivyxConfig::hydrate_secrets_from_store`]. Wraps the storage
    /// layer's error as a string — we do not want this crate's public
    /// API to re-export `StorageError` because then every caller sees
    /// a storage type they do not need.
    #[error("encrypted-store read failed for secret `{field}`: {reason}")]
    StoreRead {
        field: &'static str,
        reason: String,
    },

    /// A secret row in `KeyDomain::Secrets` held bytes that were not
    /// valid UTF-8. Secrets are serialized as UTF-8 strings (and
    /// wrapped in `SecretString`), so a non-UTF-8 blob is a corrupted
    /// or mis-written row.
    #[error("secret `{field}` in encrypted store is not valid UTF-8")]
    NonUtf8Secret { field: &'static str },

    /// The caller selected an active role (via
    /// [`LoadOptions::role_override`] or the `AIVYX_ROLE` env var) that
    /// was not present in the loaded [`AivyxConfig::roles`] map. The
    /// error lists every known role name so the operator can see what
    /// was actually loaded alongside what was requested.
    #[error(
        "active role `{name}` is not defined in config \
         (known roles: {known:?})"
    )]
    UnknownRole {
        name: String,
        known: Vec<String>,
    },

    /// The `parent_role` graph declared by one or more `[[role]]`
    /// entries does not form a valid single-inheritance tree. Phase
    /// 13 Task 1 added this variant to enforce **PRODUCT.md P7**'s
    /// structural guarantee at config-load time.
    ///
    /// Reasons this variant fires:
    /// - A role's `parent_role` names a role that does not exist in
    ///   the loaded config (typo, renamed role, etc.).
    /// - A role names itself as its own `parent_role` (self-cycle).
    /// - A chain of `parent_role` references forms a cycle (A → B →
    ///   A, or longer).
    /// - Every role has a `parent_role`, leaving the tree without a
    ///   terminating root. Multi-root configs (a forest of disjoint
    ///   trees) are *legal* — PRODUCT.md P7's single-inheritance
    ///   rule is "no role has more than one parent," which a forest
    ///   satisfies — so this variant only fires when *zero* roots
    ///   exist, not when there are two or more.
    /// - A role declares a `capability_scopes` entry that is not
    ///   granted by its nearest non-empty ancestor's declared
    ///   scopes. PRODUCT.md P7's "child can attenuate, never widen"
    ///   rule, enforced at config-load time per Q5. The error
    ///   message names the offending child role, the offending
    ///   scope string, and the constraining ancestor whose
    ///   declared scopes failed to grant it.
    ///
    /// The `reason` field carries a human-readable explanation that
    /// includes the offending role name(s) and, where applicable,
    /// the cycle path. Sufficient to write a precise error message
    /// without needing to re-walk the graph from the caller side.
    #[error("role inheritance invalid: {reason}")]
    RoleInheritance { reason: String },
}

impl From<ConfigError> for String {
    fn from(e: ConfigError) -> Self {
        e.to_string()
    }
}

// --------------------------------------------------------------------
// LoadOptions
// --------------------------------------------------------------------

/// Caller-supplied constraints on the load.
///
/// The loader itself never decides what's "required" — the binary does,
/// based on its CLI args. A `--verify-only` run does not need
/// `anthropic_api_key`; a `--channel telegram` run does need
/// `telegram.token`. Encoding those decisions here keeps the loader
/// pure: given the same `LoadOptions` and the same environment, it
/// produces the same result and the same errors.
#[derive(Debug, Clone)]
pub struct LoadOptions {
    /// Optional TOML file path. `None` means "no TOML source" (tests
    /// use this to isolate env-only behavior). `Some(path)` means
    /// "read this file if it exists; its absence is not an error,
    /// but its existence-plus-parse-failure is." The binary defaults
    /// this to `Some(PathBuf::from("./aivyx.toml"))`.
    pub toml_path: Option<PathBuf>,
    /// If `true`, [`AivyxConfig::validate`] errors out when
    /// `anthropic_api_key` is still `None`. Set to `false` by
    /// `--verify-only`.
    pub require_api_key: bool,
    /// If `true`, [`AivyxConfig::validate`] errors out when
    /// `telegram.token` is still `None`. Set to `true` by
    /// `--channel telegram`, `false` otherwise.
    pub require_telegram_token: bool,
    /// Caller-supplied override for which role should be activated at
    /// load time. Highest priority in the active-role resolution
    /// chain:
    ///
    /// 1. `LoadOptions::role_override` (this field) — typically populated
    ///    from a future `--role <name>` CLI flag.
    /// 2. `AIVYX_ROLE` environment variable.
    /// 3. [`DEFAULT_ROLE_NAME`] (`"default"`).
    ///
    /// An override that does not match any role loaded from config
    /// surfaces as [`ConfigError::UnknownRole`] at
    /// [`AivyxConfig::load_from_env_and_toml`] time — the error
    /// includes the list of known role names so the operator can
    /// see what was actually loaded.
    ///
    /// Task 1 of Phase 11 added the field; the `--role` CLI flag that
    /// populates it lands in Task 4. Until then the binary always
    /// leaves this as `None` and the env-var path is the only user-
    /// facing surface.
    pub role_override: Option<String>,
}

impl LoadOptions {
    /// Minimal options — no TOML, no required fields. Used by tests
    /// that want to exercise env-only precedence without touching disk.
    pub fn test_env_only() -> Self {
        Self {
            toml_path: None,
            require_api_key: false,
            require_telegram_token: false,
            role_override: None,
        }
    }
}

// --------------------------------------------------------------------
// AivyxConfig
// --------------------------------------------------------------------

/// Top-level typed configuration. Built by
/// [`AivyxConfig::load_from_env_and_toml`] and optionally mutated by
/// [`AivyxConfig::hydrate_secrets_from_store`] before being handed to
/// the binary's session-wiring code.
///
/// Every field that was populated carries its [`FieldSource`]. Every
/// optional field that was *not* populated is `None`; the binary
/// decides whether `None` is fatal via [`AivyxConfig::validate`].
///
/// The `Debug` derive is safe because every secret field is a
/// [`SourcedSecret`], whose hand-written `Debug` redacts the inner
/// value.
#[derive(Debug, Clone)]
pub struct AivyxConfig {
    /// Anthropic API key. `Option` because `--verify-only` runs do not
    /// need it. `SourcedSecret` so a stray `{:?}` never leaks the key.
    pub anthropic_api_key: Option<SourcedSecret>,
    /// OpenAI API key. `Option` because only needed when
    /// `provider == ProviderKind::OpenAi`.
    pub openai_api_key: Option<SourcedSecret>,
    /// OpenAI-compatible base URL override. `None` means use the
    /// provider's default (`https://api.openai.com`). Set for
    /// Ollama / local endpoints.
    pub openai_base_url: Option<Sourced<String>>,
    /// Which LLM provider backend to use. Default: `Anthropic`.
    pub provider: Sourced<ProviderKind>,
    /// Model id. Always populated — falls through to [`DEFAULT_MODEL`]
    /// if no source supplied one (tagged [`FieldSource::Default`]).
    pub model: Sourced<String>,
    /// Legacy top-level system prompt. Always populated with the same
    /// default semantics as [`Self::model`].
    ///
    /// As of Phase 11 Task 1 this field is **no longer** the canonical
    /// source of the agent's system prompt at run time — that role
    /// belongs to `roles[active_role.value()].system_prompt`. The
    /// field stays here for three reasons:
    ///
    /// 1. Backwards compatibility — configs that predate Phase 11 and
    ///    set `[agent] system_prompt = "..."` (or `AIVYX_SYSTEM_PROMPT`)
    ///    continue to work because the loader synthesizes an implicit
    ///    `"default"` role whose `system_prompt` is sourced from this
    ///    field.
    /// 2. The startup banner in `aivyx-channel/src/bin/aivyx.rs` still
    ///    renders this field directly; demoting it to a role-only
    ///    field would be a Task 4 concern. Task 1 leaves the banner
    ///    untouched.
    /// 3. It's the fixture the `FieldSource::Default` fall-through
    ///    path uses so a brand-new config with no explicit roles and
    ///    no legacy `system_prompt` still produces a functional agent
    ///    with the Phase 3 default prompt wrapped inside the
    ///    synthesized `default` role.
    pub system_prompt: Sourced<String>,
    /// Filesystem sandbox root for `fs.read` / `fs.write` tools.
    /// Resolution order: `AIVYX_FS_ROOT` → TOML `fs.root` →
    /// `$HOME/aivyx-sandbox`. Missing HOME with no override is a
    /// [`ConfigError::NoHome`].
    pub fs_root: Sourced<PathBuf>,
    /// Encrypted-store path (redb file). Resolution order:
    /// `AIVYX_STORAGE_PATH` → TOML `storage.path` →
    /// `$XDG_DATA_HOME/aivyx/store.redb` → `$HOME/.local/share/aivyx/store.redb`.
    pub storage_path: Sourced<PathBuf>,
    /// Per-topic memory-write tripwire. Always populated — default is
    /// [`DEFAULT_MEMORY_MAX_PER_TOPIC`].
    pub memory_max_per_topic: Sourced<usize>,
    /// Aivyx store passphrase. `None` means "no source supplied one"
    /// and the binary should either prompt the user (tty branch) or
    /// error out (non-tty branch). Config layer does not do terminal
    /// I/O.
    pub passphrase: Option<SourcedSecret>,
    /// Telegram channel config. `None` when the caller did not enable
    /// Telegram loading (i.e. `--channel local` or
    /// `--channel` was not passed). The loader still fills this in if
    /// any Telegram fields are set, so the startup banner can warn
    /// about orphan config.
    pub telegram: Option<TelegramConfig>,
    /// All roles defined in this config, keyed by role name.
    ///
    /// Phase 11 Task 1 introduced the [`Role`] primitive. The loader
    /// populates this map from either (a) the `[[role]]` table-array
    /// in the loaded TOML file, or (b) a synthesized implicit
    /// `"default"` role built from the legacy top-level fields when
    /// no explicit roles are configured.
    ///
    /// Invariant: always non-empty, and always contains at least one
    /// key (`active_role.value()`). `BTreeMap` (not `HashMap`) so
    /// iteration order is stable — matters for the startup banner's
    /// role-summary line and for any future `--list-roles` surface.
    pub roles: BTreeMap<String, Role>,
    /// Name of the currently active role.
    ///
    /// Resolution priority at load time (highest first):
    /// 1. [`LoadOptions::role_override`] (populated by the future
    ///    `--role <name>` CLI flag landing in Phase 11 Task 4).
    /// 2. `AIVYX_ROLE` environment variable.
    /// 3. [`DEFAULT_ROLE_NAME`] (`"default"`).
    ///
    /// The loader validates at load time that `self.roles` contains
    /// a matching entry; if not, it returns
    /// [`ConfigError::UnknownRole`]. This means every downstream
    /// consumer can safely `self.roles.get(self.active_role.value())
    /// .expect("validated at load")` without re-checking.
    pub active_role: Sourced<String>,
    /// Non-fatal warnings accumulated by the loader.
    ///
    /// Phase 11 Task 1 introduced this field so the loader can
    /// surface "your config is probably a typo but it still loaded"
    /// conditions without writing to stderr from inside a library
    /// crate (the crate's module docstring explicitly forbids
    /// terminal I/O). The binary's startup banner prints each entry
    /// after the config table.
    ///
    /// Current emitters:
    /// - Both a legacy top-level `[agent] system_prompt` and one or
    ///   more explicit `[[role]]` entries are present in the same
    ///   config. The explicit roles win at run time and the legacy
    ///   field is ignored; the warning tells the operator to move
    ///   the prompt into the role they want to use. Only fires when
    ///   the legacy `system_prompt` actually came from Env or TOML,
    ///   not from the hard-coded default, so a brand-new config
    ///   that defines one role but inherits `DEFAULT_SYSTEM_PROMPT`
    ///   doesn't get a spurious warning.
    pub warnings: Vec<String>,
    /// MCP server configurations from `[[mcp_server]]` entries.
    /// Empty when no entries are configured.
    pub mcp_servers: Vec<McpServerConfig>,
    /// Scheduled execution entries from `[[schedule]]` entries.
    /// Empty when no entries are configured.
    pub schedules: Vec<ScheduleConfig>,
    /// Webhook trigger entries from `[[webhook]]` entries.
    /// Empty when no entries are configured.
    pub webhooks: Vec<WebhookConfig>,
    /// File-watch trigger entries from `[[file_watch]]` entries.
    /// Empty when no entries are configured.
    pub file_watches: Vec<FileWatchConfig>,
    /// Webhook listener port override. `None` means use the default
    /// (7842). Loaded from `[daemon] webhook_port` in the TOML file.
    pub webhook_port: Option<u16>,
}

/// A named bundle of role-scoped configuration loaded from a single
/// `[[role]]` entry in the config file, or synthesized from legacy
/// top-level fields for backwards compatibility.
///
/// Roles are **user-defined** — there is no fixed enum of role names
/// in the codebase. The set of valid roles is whatever the operator
/// wrote into their config. Every field carries its [`FieldSource`]
/// via [`Sourced`] so the startup banner can render provenance for
/// individual role properties independently of the role as a whole.
///
/// Phase 11 Task 1 added the type with three fields: `system_prompt`,
/// `tool_allowlist`, `memory_topic_prefix`. Phase 13 Task 1 adds
/// three more — `capability_scopes`, `trust_ceiling`, `parent_role` —
/// so each role can declare its **complete capability envelope**
/// directly in config per **PRODUCT.md P9**, and inherit that envelope
/// from a parent role per the single-inheritance rule in **PRODUCT.md
/// P7**. Phase 13 Task 2 consumes those three fields in
/// `aivyx-channel/src/bin/aivyx.rs` to construct the binary's
/// `CapabilitySet` from the active role's declared envelope instead
/// of from a hard-coded `Vec<Scope>`.
///
/// Phase 11 wiring: Task 2 wires [`Self::memory_topic_prefix`] into
/// every `memory.*` tool dispatch; Task 4 wires
/// [`Self::system_prompt`] into the LLM planner, wires
/// [`Self::tool_allowlist`] into the tool-advertisement filter, and
/// wires the `--role` CLI flag into [`LoadOptions::role_override`].
#[derive(Debug, Clone)]
pub struct Role {
    /// The role's unique name as written in the TOML `name = "..."`
    /// field. Also the key under which the role lives in
    /// [`AivyxConfig::roles`]. `Sourced<String>` so the banner can
    /// show whether the name came from TOML or from the implicit
    /// `default` synthesis.
    pub name: Sourced<String>,
    /// System prompt used when this role is active. For the
    /// synthesized `default` role this is sourced from the legacy
    /// top-level [`AivyxConfig::system_prompt`] (preserving its
    /// original `FieldSource`, so a banner-reader can still tell
    /// whether the default role's prompt came from env, TOML, or the
    /// hard-coded [`DEFAULT_SYSTEM_PROMPT`]).
    pub system_prompt: Sourced<String>,
    /// Which tools this role is allowed to call. See [`ToolAllowlist`]
    /// for the absent-vs-empty distinction — omitting the
    /// `tool_allowlist` key entirely means "no filter, allow every
    /// registered tool," while setting it to an empty list means
    /// "deny every tool." Tool-catalog filtering is Phase 11 Task 4.
    pub tool_allowlist: Sourced<ToolAllowlist>,
    /// Optional prefix prepended to every `memory.*` topic when this
    /// role is active. `None` means "no prefix — topics used bare,
    /// identical to Phase 8–10 behavior." A value like
    /// `Some("coder/")` means that when the active role is this role,
    /// a `memory.write` to topic `"notes"` is stored under
    /// `"coder/notes"` from the substrate's perspective. The prefix
    /// is invisible to the model — it still writes to `"notes"` in
    /// the tool call. Dispatch-layer injection is Phase 11 Task 2.
    pub memory_topic_prefix: Sourced<Option<String>>,
    /// Capability scopes declared by this role **in addition to**
    /// whatever it inherits from its parent. Phase 13 Task 1.
    ///
    /// An absent `capability_scopes` key in TOML (or the synthesized
    /// `default` role's no-legacy-scope path) maps to an **empty
    /// `Vec`** with [`FieldSource::Default`] — meaning "this role
    /// adds no scopes beyond what its parent already holds." An
    /// explicit empty list (`capability_scopes = []`) maps to an
    /// empty `Vec` with [`FieldSource::Toml`] — same value, different
    /// provenance, so a startup-banner consumer can still tell the
    /// two apart. Unlike [`ToolAllowlist`], there is no behavioral
    /// difference between absent and empty for this field: "no
    /// additional scopes" is the same whether you say so explicitly
    /// or leave the key out. The provenance distinction exists for
    /// banner-reading and audit clarity, nothing more.
    ///
    /// Scope strings are parsed at config-load time via
    /// [`Scope::parse`]. A string that does not parse (unknown base,
    /// per the `KNOWN_BASES` check in `aivyx-capability`) fails the
    /// load with [`ConfigError::Invalid`] pointing at the role name
    /// and the bad string. This is Q2's resolution (config-time
    /// parsing, one-way dep on `aivyx-capability`) — the alternative
    /// of storing opaque strings and parsing lazily at role-activation
    /// time was rejected because it hides typos until the operator
    /// tries to use a role that's been broken for weeks.
    ///
    /// Phase 13 Task 2 consumes this field in
    /// `aivyx-channel/src/bin/aivyx.rs` by walking the role's
    /// inheritance chain (via [`Self::parent_role`]) and unioning
    /// each ancestor's scopes into the effective envelope. The
    /// resulting [`aivyx_capability::CapabilitySet`] is then passed
    /// through the existing registration-time per-tool gate and the
    /// turn loop's ceiling intersection from Phase 11 — Phase 13 is
    /// a config-substrate phase, not a capability-layer rewrite.
    pub capability_scopes: Sourced<Vec<Scope>>,
    /// Maximum trust tier this role may run at. Phase 13 Task 1.
    ///
    /// An absent `trust_ceiling` key maps to
    /// `Sourced::new(TrustTier::Trusted, FieldSource::Default)` —
    /// matching Phase 11's de-facto Trusted default on the Local
    /// channel. An explicit `trust_ceiling = "SemiTrusted"` maps to
    /// `Sourced::new(TrustTier::SemiTrusted, FieldSource::Toml)`.
    ///
    /// Phase 13 Task 2 folds this value into the existing channel-
    /// tier intersection in the binary: the **effective** ceiling is
    /// `min(channel_tier, role_declared_ceiling)`. A role declaring
    /// `Trusted` on a Telegram (`SemiTrusted`) channel still runs
    /// `SemiTrusted` because the channel tier dominates downward. A
    /// role declaring `SemiTrusted` on a Local (`Trusted`) channel
    /// runs `SemiTrusted` because the role is choosing to run more
    /// restrictively than the channel would allow. This matches Q3's
    /// resolution — the role-declared ceiling is an additional input
    /// to the existing intersection, not a replacement for it.
    pub trust_ceiling: Sourced<TrustTier>,
    /// The role this role inherits from. Phase 13 Task 1.
    ///
    /// `None` means "this role is the root of its inheritance tree."
    /// Multiple roles may carry `None` — a forest of disjoint trees
    /// is legal. PRODUCT.md P7's single-inheritance rule forbids
    /// *multi-parent*, not multi-root.
    ///
    /// An absent `parent_role` key in TOML maps in two ways:
    ///
    /// - If the same TOML file declares an explicit `default` role
    ///   alongside this one, this role implicitly inherits from
    ///   `default`: `Sourced::new(Some("default"), FieldSource::
    ///   Default)`. This is Q4's "implicit-from-default" ergonomic.
    /// - If no `default` role is declared in the same file, this
    ///   role is its own tree root: `Sourced::new(None, FieldSource::
    ///   Default)`. This preserves Phase 11 backcompat for fixtures
    ///   that defined a single non-`default` role and never touched
    ///   inheritance.
    ///
    /// An explicit `parent_role = "coder"` maps to `Sourced::new(
    /// Some("coder"), FieldSource::Toml)` and is honored regardless
    /// of whether `default` exists.
    ///
    /// **Tree-shape invariant.** At config-load time the loader
    /// validates that the `parent_role` graph (a) names only
    /// existing roles, (b) has no self-references, (c) has no
    /// cycles, and (d) terminates somewhere — i.e. at least one
    /// role has `parent_role = None`. Violations surface as
    /// [`ConfigError::RoleInheritance`] at `load_from_env_and_toml`
    /// return time. This is the **structural** enforcement of
    /// **PRODUCT.md P7** — multi-parent inheritance is not a
    /// forward commitment and the config layer refuses to represent
    /// it.
    pub parent_role: Sourced<Option<String>>,
}

/// Tool-allowlist policy for a [`Role`]. Distinguishes "the config
/// key was absent" from "the config key was present and empty" —
/// these two states have **opposite** meanings and collapsing them
/// would be a silent footgun.
///
/// - [`ToolAllowlist::AllowAll`] — the `tool_allowlist` key was not
///   present in the role's TOML entry at all. The role inherits
///   every registered tool with no filter. This is also the value
///   used for the synthesized `default` role in the backwards-
///   compatibility path, so pre-Phase-11 configs see zero behavior
///   change.
/// - [`ToolAllowlist::Only`] — the `tool_allowlist` key was present
///   and holds a list (possibly empty). The role can call exactly
///   the listed tools and nothing else. An explicit empty list
///   (`tool_allowlist = []`) means "this role can call no tools" —
///   probably a user error, but a legal configuration.
///
/// Phase 11 Task 4 consumes this enum to filter the tool catalog
/// before it's advertised to the LLM planner. Task 1 (this task)
/// only loads and stores the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolAllowlist {
    /// Field absent in config → no filtering, every registered tool
    /// is available to the role.
    AllowAll,
    /// Field present in config → only the named tools are available.
    /// An empty vec here means "deny all tools for this role."
    Only(Vec<String>),
}

/// Telegram-specific configuration loaded as a sub-object.
#[derive(Debug, Clone)]
pub struct TelegramConfig {
    /// Bot token. `Option` because a Telegram-enabled binary run might
    /// still fail at validate time if the token is missing everywhere.
    pub token: Option<SourcedSecret>,
    /// Optional chat_id filter. `None` = accept all chats (Phase 9
    /// multi-chat mode). `Some` = single-chat compat mode.
    pub chat_filter: Option<Sourced<i64>>,
}

/// One MCP server to connect to at daemon startup.
/// Loaded from `[[mcp_server]]` entries in `aivyx.toml`.
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub enabled: bool,
}

/// One scheduled execution entry loaded from `[[schedule]]` in the TOML file.
#[derive(Debug, Clone)]
pub struct ScheduleConfig {
    pub name: String,
    pub cron: String,
    pub role: String,
    pub prompt: String,
    pub enabled: bool,
    pub wrap_mission: bool,
}

/// One webhook trigger entry loaded from `[[webhook]]` in the TOML file.
/// Phase 27 Task 3.
#[derive(Debug, Clone)]
pub struct WebhookConfig {
    pub name: String,
    pub role: String,
    pub prompt: String,
    pub enabled: bool,
    pub wrap_mission: bool,
}

/// One file-watch trigger entry loaded from `[[file_watch]]` in the TOML file.
/// Phase 27 Task 4.
#[derive(Debug, Clone)]
pub struct FileWatchConfig {
    pub name: String,
    pub path: String,
    pub role: String,
    pub prompt: String,
    pub enabled: bool,
    pub debounce_ms: Option<u64>,
    pub wrap_mission: bool,
}

// --------------------------------------------------------------------
// TOML schema (internal deserialize target)
// --------------------------------------------------------------------

/// Private type that mirrors the TOML file layout. Deliberately
/// separate from [`AivyxConfig`] so the TOML schema is a versionable,
/// flat surface independent of the runtime config's provenance-tracked
/// shape. A future Phase 10 schema change lands here without touching
/// [`AivyxConfig`]'s public API.
#[derive(Debug, Default, Deserialize)]
struct RawToml {
    #[serde(default)]
    anthropic: RawAnthropic,
    #[serde(default)]
    openai: RawOpenAi,
    #[serde(default)]
    agent: RawAgent,
    #[serde(default)]
    fs: RawFs,
    #[serde(default)]
    storage: RawStorage,
    #[serde(default)]
    memory: RawMemory,
    #[serde(default)]
    telegram: RawTelegram,
    #[serde(default)]
    aivyx: RawAivyx,
    /// `[[role]]` table-array. One entry per role. Unset in the TOML
    /// → `None`, which triggers the implicit-`default`-role synthesis
    /// in the loader. `Some(vec)` (including `Some(vec![])` for a
    /// TOML file with `role = []`) means the operator is opting in
    /// to explicit roles; the loader will not synthesize anything
    /// and will instead require `active_role` to match one of the
    /// entries.
    #[serde(default, rename = "role")]
    roles: Option<Vec<RawRole>>,
    /// `[[mcp_server]]` table-array. Phase 24 Task 2.
    #[serde(default, rename = "mcp_server")]
    mcp_servers: Option<Vec<RawMcpServer>>,
    /// `[[schedule]]` table-array. Phase 26 Task 2.
    #[serde(default, rename = "schedule")]
    schedules: Option<Vec<RawSchedule>>,
    /// `[[webhook]]` table-array. Phase 27 Task 3.
    #[serde(default, rename = "webhook")]
    webhooks: Option<Vec<RawWebhook>>,
    /// `[[file_watch]]` table-array. Phase 27 Task 4.
    #[serde(default, rename = "file_watch")]
    file_watches: Option<Vec<RawFileWatch>>,
    /// `[daemon]` section. Phase 28 Task 3.
    #[serde(default)]
    daemon: RawDaemon,
}

/// `[daemon]` section in the TOML file. Phase 28 Task 3.
#[derive(Debug, Default, Deserialize)]
struct RawDaemon {
    webhook_port: Option<u16>,
}

/// One `[[role]]` entry in the TOML file. Mirrors the runtime
/// [`Role`] shape but uses raw types ready for deserialization —
/// the [`AivyxConfig::load_from_env_and_toml`] loader maps each
/// `RawRole` to a [`Role`] with proper [`Sourced`] wrappers.
///
/// `tool_allowlist` is `Option<Vec<String>>` on purpose: `None`
/// (key absent) maps to [`ToolAllowlist::AllowAll`], while
/// `Some(vec)` maps to [`ToolAllowlist::Only`]. This is the Q3
/// resolution from the Phase 11 plan — "absent" and "empty" have
/// opposite meanings and must not collapse.
///
/// Phase 13 Task 1 adds three mirror fields for the per-role
/// capability envelope: `capability_scopes`, `trust_ceiling`, and
/// `parent_role`. The `capability_scopes` field is a
/// `Vec<String>` at the raw layer — the loader parses each string
/// into a [`Scope`] via [`Scope::parse`] and fails with
/// [`ConfigError::Invalid`] on any unknown scope base. `trust_
/// ceiling` deserializes into the real [`TrustTier`] enum directly
/// because `aivyx-capability` derives `Deserialize` on it — a
/// typo'd tier surfaces as a TOML parse error at `load_toml` time,
/// not as a config-load error, which is fine for operator
/// ergonomics (the error message still includes the file path).
/// `parent_role` is `Option<String>` with the usual absent-vs-
/// explicit distinction.
#[derive(Debug, Default, Deserialize)]
struct RawRole {
    name: String,
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    tool_allowlist: Option<Vec<String>>,
    #[serde(default)]
    memory_topic_prefix: Option<String>,
    #[serde(default)]
    capability_scopes: Option<Vec<String>>,
    #[serde(default)]
    trust_ceiling: Option<TrustTier>,
    #[serde(default)]
    parent_role: Option<String>,
}

/// One `[[mcp_server]]` entry in the TOML file. Phase 24 Task 2.
#[derive(Debug, Default, Deserialize)]
struct RawMcpServer {
    name: String,
    command: String,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default = "default_true")]
    enabled: bool,
}

/// One `[[schedule]]` entry in the TOML file. Phase 26 Task 2.
#[derive(Debug, Default, Deserialize)]
struct RawSchedule {
    name: String,
    cron: String,
    #[serde(default = "default_role_name")]
    role: String,
    prompt: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    wrap_mission: bool,
}

/// One `[[webhook]]` entry in the TOML file. Phase 27 Task 3.
#[derive(Debug, Default, Deserialize)]
struct RawWebhook {
    name: String,
    #[serde(default = "default_role_name")]
    role: String,
    prompt: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    wrap_mission: bool,
}

/// One `[[file_watch]]` entry in the TOML file. Phase 27 Task 4.
#[derive(Debug, Default, Deserialize)]
struct RawFileWatch {
    name: String,
    path: String,
    #[serde(default = "default_role_name")]
    role: String,
    prompt: String,
    #[serde(default = "default_true")]
    enabled: bool,
    debounce_ms: Option<u64>,
    #[serde(default)]
    wrap_mission: bool,
}

fn default_role_name() -> String {
    DEFAULT_ROLE_NAME.to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Deserialize)]
struct RawAnthropic {
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawOpenAi {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawAgent {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    provider: Option<ProviderKind>,
}

#[derive(Debug, Default, Deserialize)]
struct RawFs {
    #[serde(default)]
    root: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct RawStorage {
    #[serde(default)]
    path: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct RawMemory {
    #[serde(default)]
    max_per_topic: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTelegram {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    chat_id: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawAivyx {
    #[serde(default)]
    passphrase: Option<String>,
}

// --------------------------------------------------------------------
// Encrypted-store key constants
// --------------------------------------------------------------------

/// Canonical byte keys used to look up secrets in
/// [`KeyDomain::Secrets`]. Defined as module constants so any future
/// `aivyx secrets set` CLI subcommand writes the exact same keys.
pub mod secret_keys {
    /// Storage key for the Anthropic API key. Value: UTF-8 string.
    pub const ANTHROPIC_API_KEY: &[u8] = b"anthropic_api_key";
    /// Storage key for the OpenAI API key. Value: UTF-8 string.
    pub const OPENAI_API_KEY: &[u8] = b"openai_api_key";
    /// Storage key for the Telegram bot token. Value: UTF-8 string.
    pub const TELEGRAM_TOKEN: &[u8] = b"telegram_token";
    /// Storage key for the Aivyx master-key passphrase. Value: UTF-8 string.
    ///
    /// Storing the passphrase inside a store that is itself encrypted
    /// by that passphrase is obviously useless, so in practice this
    /// key will never be populated — but we reserve it anyway for
    /// symmetry and to make the future `aivyx secrets set` surface
    /// complete.
    pub const AIVYX_PASSPHRASE: &[u8] = b"aivyx_passphrase";
}

// --------------------------------------------------------------------
// Env var name constants
// --------------------------------------------------------------------

const ENV_ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
const ENV_MODEL: &str = "AIVYX_MODEL";
const ENV_SYSTEM_PROMPT: &str = "AIVYX_SYSTEM_PROMPT";
const ENV_FS_ROOT: &str = "AIVYX_FS_ROOT";
const ENV_STORAGE_PATH: &str = "AIVYX_STORAGE_PATH";
const ENV_XDG_DATA_HOME: &str = "XDG_DATA_HOME";
const ENV_HOME: &str = "HOME";
const ENV_MEMORY_MAX_PER_TOPIC: &str = "AIVYX_MEMORY_MAX_PER_TOPIC";
const ENV_PASSPHRASE: &str = "AIVYX_PASSPHRASE";
const ENV_TELEGRAM_TOKEN: &str = "AIVYX_TELEGRAM_TOKEN";
const ENV_TELEGRAM_CHAT_ID: &str = "AIVYX_TELEGRAM_CHAT_ID";
/// Env-var override for the active role name, second-priority in the
/// active-role resolution chain (below [`LoadOptions::role_override`]
/// and above the [`DEFAULT_ROLE_NAME`] fall-through). Phase 11 Task 1.
const ENV_ROLE: &str = "AIVYX_ROLE";
const ENV_OPENAI_API_KEY: &str = "AIVYX_OPENAI_API_KEY";
const ENV_OPENAI_BASE_URL: &str = "AIVYX_OPENAI_BASE_URL";
const ENV_PROVIDER: &str = "AIVYX_PROVIDER";

// --------------------------------------------------------------------
// Loader
// --------------------------------------------------------------------

impl AivyxConfig {
    /// Phase 1 of the two-phase load: env vars + TOML file.
    ///
    /// Precedence per field: env > TOML > default (or `None` for
    /// secret-bearing fields). Missing TOML files are *not* an error;
    /// missing required fields become errors only at
    /// [`AivyxConfig::validate`] time.
    pub fn load_from_env_and_toml(opts: &LoadOptions) -> Result<Self, ConfigError> {
        let toml = load_toml(opts.toml_path.as_deref())?;

        // --- anthropic_api_key --------------------------------------
        // Secret; stays None if neither env nor TOML supplied one.
        // Storage hydration in Phase 2 of the loader can still fill it.
        let anthropic_api_key = env_secret(ENV_ANTHROPIC_API_KEY)
            .map(|s| SourcedSecret::new(s, FieldSource::Env))
            .or_else(|| {
                toml.anthropic
                    .api_key
                    .as_ref()
                    .map(|s| SourcedSecret::new(SecretString::from(s.clone()), FieldSource::Toml))
            });

        // --- openai_api_key -----------------------------------------
        let openai_api_key = env_secret(ENV_OPENAI_API_KEY)
            .map(|s| SourcedSecret::new(s, FieldSource::Env))
            .or_else(|| {
                toml.openai
                    .api_key
                    .as_ref()
                    .map(|s| SourcedSecret::new(SecretString::from(s.clone()), FieldSource::Toml))
            });

        // --- openai_base_url ----------------------------------------
        let openai_base_url = match env_string(ENV_OPENAI_BASE_URL) {
            Some(v) => Some(Sourced::new(v, FieldSource::Env)),
            None => toml
                .openai
                .base_url
                .clone()
                .map(|v| Sourced::new(v, FieldSource::Toml)),
        };

        // --- provider -----------------------------------------------
        let provider = match env_string(ENV_PROVIDER) {
            Some(v) => {
                let kind = match v.as_str() {
                    "anthropic" => ProviderKind::Anthropic,
                    "openai" => ProviderKind::OpenAi,
                    other => {
                        return Err(ConfigError::Invalid {
                            field: "provider",
                            reason: format!(
                                "{ENV_PROVIDER}={other:?} is not valid. \
                                 Supported: anthropic, openai"
                            ),
                        });
                    }
                };
                Sourced::new(kind, FieldSource::Env)
            }
            None => match toml.agent.provider {
                Some(kind) => Sourced::new(kind, FieldSource::Toml),
                None => Sourced::new(ProviderKind::Anthropic, FieldSource::Default),
            },
        };

        // --- model --------------------------------------------------
        // Always populated — falls through to DEFAULT_MODEL.
        let model = match env_string(ENV_MODEL) {
            Some(v) => Sourced::new(v, FieldSource::Env),
            None => match toml.agent.model.clone() {
                Some(v) => Sourced::new(v, FieldSource::Toml),
                None => Sourced::new(DEFAULT_MODEL.to_string(), FieldSource::Default),
            },
        };

        // --- system_prompt -----------------------------------------
        let system_prompt = match env_string(ENV_SYSTEM_PROMPT) {
            Some(v) => Sourced::new(v, FieldSource::Env),
            None => match toml.agent.system_prompt.clone() {
                Some(v) => Sourced::new(v, FieldSource::Toml),
                None => Sourced::new(DEFAULT_SYSTEM_PROMPT.to_string(), FieldSource::Default),
            },
        };

        // --- fs_root ------------------------------------------------
        // Phase 8 binary logic: env → default `$HOME/aivyx-sandbox`.
        // Phase 9 adds TOML `fs.root` between them. A missing HOME
        // with no explicit override is a typed NoHome error.
        let fs_root = match env_path(ENV_FS_ROOT) {
            Some(p) => Sourced::new(p, FieldSource::Env),
            None => match toml.fs.root.clone() {
                Some(p) => Sourced::new(p, FieldSource::Toml),
                None => {
                    let home = env_path(ENV_HOME).ok_or(ConfigError::NoHome { field: "fs_root" })?;
                    Sourced::new(home.join("aivyx-sandbox"), FieldSource::Default)
                }
            },
        };

        // --- storage_path -------------------------------------------
        // Phase 8 logic: env → $XDG_DATA_HOME/aivyx/store.redb →
        // $HOME/.local/share/aivyx/store.redb. Phase 9 adds a TOML
        // `storage.path` entry with env-beats-toml precedence.
        let storage_path = match env_path(ENV_STORAGE_PATH) {
            Some(p) => Sourced::new(p, FieldSource::Env),
            None => match toml.storage.path.clone() {
                Some(p) => Sourced::new(p, FieldSource::Toml),
                None => {
                    let default_path = if let Some(xdg) = env_path(ENV_XDG_DATA_HOME) {
                        xdg.join("aivyx").join("store.redb")
                    } else {
                        let home =
                            env_path(ENV_HOME).ok_or(ConfigError::NoHome { field: "storage_path" })?;
                        home.join(".local")
                            .join("share")
                            .join("aivyx")
                            .join("store.redb")
                    };
                    Sourced::new(default_path, FieldSource::Default)
                }
            },
        };

        // --- memory_max_per_topic ----------------------------------
        // Env value is parsed as usize; unparseable is a hard Invalid.
        let memory_max_per_topic = match env_string(ENV_MEMORY_MAX_PER_TOPIC) {
            Some(s) => {
                let parsed = s.parse::<usize>().map_err(|e| ConfigError::Invalid {
                    field: "memory_max_per_topic",
                    reason: format!(
                        "{ENV_MEMORY_MAX_PER_TOPIC}={s:?} is not a valid usize: {e}"
                    ),
                })?;
                Sourced::new(parsed, FieldSource::Env)
            }
            None => match toml.memory.max_per_topic {
                Some(n) => Sourced::new(n, FieldSource::Toml),
                None => Sourced::new(DEFAULT_MEMORY_MAX_PER_TOPIC, FieldSource::Default),
            },
        };

        // --- passphrase --------------------------------------------
        // Secret; "set but empty" is treated as unset at this layer,
        // preserving Phase 7's bailout behavior for `export
        // AIVYX_PASSPHRASE=` with no value.
        let passphrase = env_secret(ENV_PASSPHRASE)
            .map(|s| SourcedSecret::new(s, FieldSource::Env))
            .or_else(|| {
                toml.aivyx
                    .passphrase
                    .as_ref()
                    .filter(|s| !s.is_empty())
                    .map(|s| SourcedSecret::new(SecretString::from(s.clone()), FieldSource::Toml))
            });

        // --- telegram ----------------------------------------------
        // Always constructed if any telegram source fires. Token is an
        // inner Option because an operator might set chat_id but not
        // the token, and we want to surface that as a
        // ConfigError::Missing at validate time — not at load time.
        let telegram_token = env_secret(ENV_TELEGRAM_TOKEN)
            .map(|s| SourcedSecret::new(s, FieldSource::Env))
            .or_else(|| {
                toml.telegram
                    .token
                    .as_ref()
                    .map(|s| SourcedSecret::new(SecretString::from(s.clone()), FieldSource::Toml))
            });

        let telegram_chat_filter = match env_string(ENV_TELEGRAM_CHAT_ID) {
            Some(s) => {
                let parsed = s.parse::<i64>().map_err(|e| ConfigError::Invalid {
                    field: "telegram.chat_id",
                    reason: format!(
                        "{ENV_TELEGRAM_CHAT_ID}={s:?} is not a valid i64: {e}"
                    ),
                })?;
                Some(Sourced::new(parsed, FieldSource::Env))
            }
            None => toml
                .telegram
                .chat_id
                .map(|n| Sourced::new(n, FieldSource::Toml)),
        };

        let telegram = if telegram_token.is_some() || telegram_chat_filter.is_some() {
            Some(TelegramConfig {
                token: telegram_token,
                chat_filter: telegram_chat_filter,
            })
        } else {
            None
        };

        // --- roles -------------------------------------------------
        // Phase 11 Task 1. Either the TOML file defined one or more
        // `[[role]]` entries (explicit roles, each lifted into the
        // runtime `Role` type with `FieldSource::Toml` wrappers), or
        // the file defined zero roles and we synthesize an implicit
        // `default` role from the legacy top-level fields. The two
        // branches are mutually exclusive — a config with both
        // legacy `system_prompt` and explicit roles accumulates a
        // warning below and the explicit roles win.
        let mut warnings: Vec<String> = Vec::new();
        let mut roles: BTreeMap<String, Role> = BTreeMap::new();

        if let Some(raw_roles) = toml.roles.as_ref() {
            // Explicit-roles branch. Any `[[role]]` entries land here.
            // An empty `Some(vec![])` — e.g. `role = []` in TOML — is
            // structurally legal but produces no usable role; the
            // active-role resolution below will fail with
            // `UnknownRole` for any active-role selection, which is
            // the correct "your config defined zero roles" surface.
            //
            // Phase 13 Task 1: Q4 resolution — implicit `parent_role`
            // for non-`default` roles only kicks in **when an explicit
            // `default` role is present** in the same TOML file. This
            // keeps the loader transparent: nothing appears in
            // `cfg.roles` that the operator did not write themselves,
            // and a Phase 11 fixture like `[[role]] name = "coder"`
            // (no `default` declared) continues to load with that one
            // role as its own tree root. The day an operator adds a
            // `default` alongside `coder`, `coder` starts implicitly
            // inheriting from it — which is the inheritance ergonomics
            // promise from PRODUCT.md P7 without secretly fabricating
            // a phantom default that operators never see.
            let has_explicit_default = raw_roles
                .iter()
                .any(|r| r.name == DEFAULT_ROLE_NAME);

            for raw in raw_roles {
                let name = Sourced::new(raw.name.clone(), FieldSource::Toml);
                let role_system_prompt = match raw.system_prompt.clone() {
                    Some(v) => Sourced::new(v, FieldSource::Toml),
                    None => Sourced::new(
                        DEFAULT_SYSTEM_PROMPT.to_string(),
                        FieldSource::Default,
                    ),
                };
                let tool_allowlist = match raw.tool_allowlist.clone() {
                    Some(v) => Sourced::new(ToolAllowlist::Only(v), FieldSource::Toml),
                    None => Sourced::new(ToolAllowlist::AllowAll, FieldSource::Default),
                };
                let memory_topic_prefix = match raw.memory_topic_prefix.clone() {
                    Some(v) => Sourced::new(Some(v), FieldSource::Toml),
                    None => Sourced::new(None, FieldSource::Default),
                };
                // --- Phase 13 Task 1 — capability_scopes ----------
                // Parse each raw scope string via `Scope::parse`.
                // Unknown bases fail loudly here with the offending
                // role name + the bad string in the error message.
                // `FieldSource::Toml` for explicit (even empty)
                // lists; `FieldSource::Default` only when the key
                // was absent from the TOML.
                let capability_scopes = match raw.capability_scopes.clone() {
                    Some(raw_scopes) => {
                        let mut parsed: Vec<Scope> = Vec::with_capacity(raw_scopes.len());
                        for raw_scope in &raw_scopes {
                            let scope = Scope::parse(raw_scope).ok_or_else(|| {
                                ConfigError::Invalid {
                                    field: "role.capability_scopes",
                                    reason: format!(
                                        "role `{}`: scope string {:?} does not parse \
                                         (unknown base or malformed qualifier — see \
                                         aivyx-capability::KNOWN_BASES)",
                                        raw.name, raw_scope
                                    ),
                                }
                            })?;
                            parsed.push(scope);
                        }
                        Sourced::new(parsed, FieldSource::Toml)
                    }
                    None => Sourced::new(Vec::new(), FieldSource::Default),
                };
                // --- Phase 13 Task 1 — trust_ceiling --------------
                // `TrustTier` derives `Deserialize` in
                // `aivyx-capability`, so a typo'd tier is already
                // caught at TOML-parse time in `load_toml`. Here we
                // only need to apply the absent-key default.
                let trust_ceiling = match raw.trust_ceiling {
                    Some(tier) => Sourced::new(tier, FieldSource::Toml),
                    None => Sourced::new(TrustTier::Trusted, FieldSource::Default),
                };
                // --- Phase 13 Task 1 — parent_role ----------------
                // Q4 resolution: a non-`default` role with no
                // explicit `parent_role` implicitly inherits from
                // `default` *only if an explicit `default` role is
                // declared in the same file*. Without that anchor,
                // the role is its own tree root — which preserves
                // Phase 11's "single role, no default" backcompat
                // path. An explicit `parent_role = "name"` is always
                // honored regardless of whether `default` exists.
                let parent_role = match raw.parent_role.clone() {
                    Some(name) => Sourced::new(Some(name), FieldSource::Toml),
                    None if raw.name == DEFAULT_ROLE_NAME => {
                        Sourced::new(None, FieldSource::Default)
                    }
                    None if has_explicit_default => Sourced::new(
                        Some(DEFAULT_ROLE_NAME.to_string()),
                        FieldSource::Default,
                    ),
                    None => Sourced::new(None, FieldSource::Default),
                };
                roles.insert(
                    raw.name.clone(),
                    Role {
                        name,
                        system_prompt: role_system_prompt,
                        tool_allowlist,
                        memory_topic_prefix,
                        capability_scopes,
                        trust_ceiling,
                        parent_role,
                    },
                );
            }

            // Q4 resolution (Option B — non-fatal warning accumulated
            // on the config, not stderr). Only fire when the legacy
            // `system_prompt` came from a real source (Env or TOML);
            // the hard-coded `FieldSource::Default` case is silent so
            // a brand-new role-using config doesn't eat a spurious
            // warning every load.
            if matches!(
                system_prompt.source,
                FieldSource::Env | FieldSource::Toml
            ) {
                warnings.push(
                    "both a legacy `[agent] system_prompt` (or \
                     AIVYX_SYSTEM_PROMPT env var) and one or more \
                     explicit `[[role]]` entries are present in this \
                     config. The explicit roles win at run time and \
                     the legacy prompt is ignored — move the prompt \
                     into a role's `system_prompt` field to silence \
                     this warning."
                        .to_string(),
                );
            }
        } else {
            // Implicit-default-role branch. Zero explicit roles → we
            // synthesize a single `default` role whose fields come
            // from the legacy top-level values, preserving their
            // original `FieldSource` so the banner can still show
            // "env" / "toml" / "default" for the synthesized role's
            // system_prompt. Every pre-Phase-11 config file hits this
            // branch and behaves exactly as it did before.
            //
            // Phase 13 Task 1: the three new fields populate from
            // their "absent key" defaults — empty `capability_scopes`
            // (the binary-side fallback in Phase 13 Task 2 supplies
            // the actual substrate scopes when no config is present),
            // `Trusted` ceiling (matches Phase 11's Local-channel
            // behavior), and `None` parent (the synthesized `default`
            // is its own root).
            let default_role = Role {
                name: Sourced::new(DEFAULT_ROLE_NAME.to_string(), FieldSource::Default),
                system_prompt: system_prompt.clone(),
                tool_allowlist: Sourced::new(ToolAllowlist::AllowAll, FieldSource::Default),
                memory_topic_prefix: Sourced::new(None, FieldSource::Default),
                capability_scopes: Sourced::new(Vec::new(), FieldSource::Default),
                trust_ceiling: Sourced::new(TrustTier::Trusted, FieldSource::Default),
                parent_role: Sourced::new(None, FieldSource::Default),
            };
            roles.insert(DEFAULT_ROLE_NAME.to_string(), default_role);
        }

        // --- Phase 13 Task 1 — single-inheritance tree validation -
        // Validate that the `parent_role` graph forms a tree: every
        // referenced parent exists, no self-cycles, no longer
        // cycles, and exactly one root (a role with `parent_role =
        // None`). This is the structural enforcement of PRODUCT.md
        // P7 — multi-parent is not a forward commitment and the
        // config layer refuses to represent it at load time.
        //
        // Runs only when there is actually a tree to validate: a
        // zero-role config (the `role = []` edge case in the
        // explicit branch) has nothing to check and will already
        // fail with `UnknownRole` at the active-role check below.
        if !roles.is_empty() {
            validate_role_inheritance(&roles)?;
        }

        // --- active_role -------------------------------------------
        // Priority: LoadOptions::role_override > AIVYX_ROLE env var >
        // DEFAULT_ROLE_NAME. At this point `roles` is non-empty — the
        // explicit branch only lands here on behalf of the loader
        // (even an explicit `role = []` is a user error that surfaces
        // as `UnknownRole` below rather than a load-time panic).
        //
        // `role_override` tags the source as `Env` because the
        // existing `FieldSource` enum has no "cli-override" variant
        // and the binary-caller path is morally equivalent to an env
        // var in the startup-banner display. If Task 4 (the task that
        // actually adds the `--role` CLI flag) wants cli/env to
        // display differently in the banner it can either add a
        // `FieldSource::Cli` variant then, or leave this as-is. The
        // env-var branch below is tagged `Env` unambiguously.
        let active_role = if let Some(name) = opts.role_override.clone() {
            Sourced::new(name, FieldSource::Env)
        } else if let Some(name) = env_string(ENV_ROLE) {
            Sourced::new(name, FieldSource::Env)
        } else {
            Sourced::new(DEFAULT_ROLE_NAME.to_string(), FieldSource::Default)
        };

        if !roles.contains_key(active_role.value.as_str()) {
            let mut known: Vec<String> = roles.keys().cloned().collect();
            known.sort();
            return Err(ConfigError::UnknownRole {
                name: active_role.value.clone(),
                known,
            });
        }

        // --- mcp_servers ------------------------------------------
        let mcp_servers: Vec<McpServerConfig> = toml
            .mcp_servers
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.enabled)
            .map(|r| McpServerConfig {
                name: r.name,
                command: r.command,
                args: r.args.unwrap_or_default(),
                enabled: true,
            })
            .collect();

        // --- schedules ---------------------------------------------
        let schedules: Vec<ScheduleConfig> = toml
            .schedules
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.enabled)
            .map(|r| ScheduleConfig {
                name: r.name,
                cron: r.cron,
                role: r.role,
                prompt: r.prompt,
                enabled: true,
                wrap_mission: r.wrap_mission,
            })
            .collect();

        // --- webhooks ----------------------------------------------
        let webhooks: Vec<WebhookConfig> = toml
            .webhooks
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.enabled)
            .map(|r| WebhookConfig {
                name: r.name,
                role: r.role,
                prompt: r.prompt,
                enabled: true,
                wrap_mission: r.wrap_mission,
            })
            .collect();

        // --- file watches ------------------------------------------
        let file_watches: Vec<FileWatchConfig> = toml
            .file_watches
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.enabled)
            .map(|r| FileWatchConfig {
                name: r.name,
                path: r.path,
                role: r.role,
                prompt: r.prompt,
                enabled: true,
                debounce_ms: r.debounce_ms,
                wrap_mission: r.wrap_mission,
            })
            .collect();

        Ok(Self {
            anthropic_api_key,
            openai_api_key,
            openai_base_url,
            provider,
            model,
            system_prompt,
            fs_root,
            storage_path,
            memory_max_per_topic,
            passphrase,
            telegram,
            roles,
            active_role,
            warnings,
            mcp_servers,
            schedules,
            webhooks,
            file_watches,
            webhook_port: toml.daemon.webhook_port,
        })
    }

    /// Phase 2 of the two-phase load: fill any still-`None` secret
    /// fields from `KeyDomain::Secrets`.
    ///
    /// Secrets already populated by env or TOML are left alone — env
    /// and TOML beat the store per fall-through precedence. A secret
    /// that is still `None` afterwards means the store had no row
    /// either; [`AivyxConfig::validate`] decides if that's fatal.
    ///
    /// The `telegram` substructure is only touched if the caller has
    /// already constructed one by passing `require_telegram_token = true`.
    /// We do not materialize a brand-new [`TelegramConfig`] here just
    /// because the store happens to hold a token — the binary must
    /// have opted in to Telegram via CLI args first.
    pub async fn hydrate_secrets_from_store(
        &mut self,
        storage: &Arc<dyn Storage>,
    ) -> Result<(), ConfigError> {
        let secrets = storage.domain(KeyDomain::Secrets);

        if self.anthropic_api_key.is_none() {
            if let Some(bytes) = secrets
                .get(secret_keys::ANTHROPIC_API_KEY)
                .await
                .map_err(|e| ConfigError::StoreRead {
                    field: "anthropic_api_key",
                    reason: e.to_string(),
                })?
            {
                let s = String::from_utf8(bytes).map_err(|_| ConfigError::NonUtf8Secret {
                    field: "anthropic_api_key",
                })?;
                self.anthropic_api_key = Some(SourcedSecret::new(
                    SecretString::from(s),
                    FieldSource::EncryptedStore,
                ));
            }
        }

        if self.openai_api_key.is_none() {
            if let Some(bytes) = secrets
                .get(secret_keys::OPENAI_API_KEY)
                .await
                .map_err(|e| ConfigError::StoreRead {
                    field: "openai_api_key",
                    reason: e.to_string(),
                })?
            {
                let s = String::from_utf8(bytes).map_err(|_| ConfigError::NonUtf8Secret {
                    field: "openai_api_key",
                })?;
                self.openai_api_key = Some(SourcedSecret::new(
                    SecretString::from(s),
                    FieldSource::EncryptedStore,
                ));
            }
        }

        if let Some(tg) = self.telegram.as_mut() {
            if tg.token.is_none() {
                if let Some(bytes) = secrets
                    .get(secret_keys::TELEGRAM_TOKEN)
                    .await
                    .map_err(|e| ConfigError::StoreRead {
                        field: "telegram.token",
                        reason: e.to_string(),
                    })?
                {
                    let s = String::from_utf8(bytes).map_err(|_| ConfigError::NonUtf8Secret {
                        field: "telegram.token",
                    })?;
                    tg.token = Some(SourcedSecret::new(
                        SecretString::from(s),
                        FieldSource::EncryptedStore,
                    ));
                }
            }
        }

        // `passphrase` is deliberately *not* hydrated from the store:
        // the store is itself sealed by the passphrase, so reading it
        // at hydrate time is circular. The field stays `None` and the
        // binary's tty branch prompts the user.

        Ok(())
    }

    /// Final validation. Checks that every field required by `opts`
    /// is populated. Returns [`ConfigError::Missing`] for the first
    /// missing required field.
    pub fn validate(&self, opts: &LoadOptions) -> Result<(), ConfigError> {
        if opts.require_api_key {
            match self.provider.value {
                ProviderKind::Anthropic => {
                    if self.anthropic_api_key.is_none() {
                        return Err(ConfigError::Missing {
                            field: "anthropic_api_key",
                        });
                    }
                }
                ProviderKind::OpenAi => {
                    if self.openai_api_key.is_none() {
                        return Err(ConfigError::Missing {
                            field: "openai_api_key",
                        });
                    }
                }
            }
        }
        if opts.require_telegram_token {
            match self.telegram.as_ref().and_then(|t| t.token.as_ref()) {
                Some(_) => {}
                None => {
                    return Err(ConfigError::Missing {
                        field: "telegram.token",
                    });
                }
            }
        }
        Ok(())
    }
}

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

/// Read an env var, treating empty strings as unset. Matches the
/// Phase 8 binary's behavior so `export FOO=` never trips a parse
/// error at startup.
fn env_string(var: &str) -> Option<String> {
    match std::env::var(var) {
        Ok(s) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// Read an env var as a `SecretString`, same empty-is-unset rule.
fn env_secret(var: &str) -> Option<SecretString> {
    env_string(var).map(SecretString::from)
}

/// Read an env var as a `PathBuf`, same empty-is-unset rule.
fn env_path(var: &str) -> Option<PathBuf> {
    env_string(var).map(PathBuf::from)
}

/// Phase 13 Task 1 — validate the `parent_role` graph.
///
/// Enforces four invariants, all of which together mean "the
/// `parent_role` edges form a tree with exactly one root":
///
/// 1. **Every referenced parent exists.** A role with
///    `parent_role = Some("researhcer")` must have `"researhcer"`
///    actually present in the `roles` map. A typo here is usually
///    the reason this function fires, so the error message names
///    both the child and the bad parent.
/// 2. **No self-reference.** A role may not name itself as its own
///    parent. (This is technically a degenerate 1-cycle and would
///    be caught by the cycle check below, but catching it first
///    gives a clearer error message.)
/// 3. **No cycles.** For each role, walk up its `parent_role`
///    chain until either the root (`None`) is reached or a
///    previously-visited role shows up again. The latter is a
///    cycle and is rejected with the full offending path in the
///    error message.
/// 4. **At least one root.** Some role must have `parent_role =
///    None`. Zero roots means every chain cycles (already caught
///    by invariant 3, but the explicit check gives a clearer error
///    if cycle detection ever drifts). PRODUCT.md P7 commits to
///    single-inheritance — *no role has more than one parent* —
///    which is satisfied by a forest of disjoint trees as well as
///    by a single rooted tree, so we deliberately tolerate
///    multi-root configs (Phase 11 fixtures with two sibling roles
///    and no `default` are the canonical example).
/// 5. **Child-parent attenuation.** Each role's declared
///    `capability_scopes` (when non-empty) must be a subset, under
///    D4 prefix-attenuation, of its **nearest non-empty ancestor**'s
///    declared scopes. PRODUCT.md P7's "child can attenuate, never
///    widen" rule, enforced at config-load time per Q5. Empty
///    `capability_scopes` is the unconstrained sentinel: an empty
///    role declares no constraint, so the walk skips it and looks
///    at the next ancestor up. If every ancestor up to the root is
///    empty, there's no constraint to enforce and the child's
///    declared set is legal at this level (the binary's backcompat
///    floor and the channel ceiling cap it at runtime).
///
/// Does not mutate `roles`. On success returns `Ok(())`; on any
/// violation returns `Err(ConfigError::RoleInheritance { reason })`
/// with a human-readable message.
///
/// The walk is `O(N * depth)` where `N` is the number of roles and
/// `depth` is the longest inheritance chain. Realistic configs
/// have at most a handful of roles and depth 2–3, so this is
/// cheap. A `HashSet` is created per role for cycle detection;
/// could be hoisted out for a pathological config with thousands
/// of roles, but we do not design for that today.
fn validate_role_inheritance(roles: &BTreeMap<String, Role>) -> Result<(), ConfigError> {
    use std::collections::HashSet;

    // Invariant 1 + 2: every `parent_role = Some(name)` must refer
    // to an existing, non-self role.
    for (name, role) in roles {
        if let Some(parent_name) = role.parent_role.value.as_ref() {
            if parent_name == name {
                return Err(ConfigError::RoleInheritance {
                    reason: format!(
                        "role `{name}` names itself as its own `parent_role` \
                         (self-cycle) — a role cannot inherit from itself"
                    ),
                });
            }
            if !roles.contains_key(parent_name) {
                let mut known: Vec<&str> = roles.keys().map(String::as_str).collect();
                known.sort();
                return Err(ConfigError::RoleInheritance {
                    reason: format!(
                        "role `{name}` has `parent_role = {parent_name:?}` \
                         but `{parent_name}` is not a known role \
                         (known roles: {known:?})"
                    ),
                });
            }
        }
    }

    // Invariant 3: no cycles. For each role, walk its parent chain
    // and bail on a repeat visit. Uses a per-role `HashSet<&str>`
    // of names seen on the current walk.
    for start in roles.keys() {
        let mut seen: HashSet<&str> = HashSet::new();
        seen.insert(start.as_str());
        let mut current = start.as_str();
        while let Some(parent) = roles
            .get(current)
            .and_then(|r| r.parent_role.value.as_deref())
        {
            if !seen.insert(parent) {
                // Cycle detected. Render the path as a chain from
                // `start` through the repeat.
                let mut path: Vec<&str> = Vec::new();
                path.push(start.as_str());
                let mut cursor = start.as_str();
                while let Some(p) = roles
                    .get(cursor)
                    .and_then(|r| r.parent_role.value.as_deref())
                {
                    path.push(p);
                    if p == parent && path.len() > 1 {
                        break;
                    }
                    cursor = p;
                }
                return Err(ConfigError::RoleInheritance {
                    reason: format!(
                        "cycle detected in `parent_role` graph starting at \
                         role `{start}`: {} (role `{parent}` is already in \
                         the chain)",
                        path.join(" -> ")
                    ),
                });
            }
            current = parent;
        }
    }

    // Invariant 4: at least one root. A root is a role whose
    // `parent_role` is `None`. Zero roots means every chain cycles
    // (already caught above; the explicit check is belt-and-braces
    // and gives a clearer error message if invariant 3 ever drifts).
    // Multiple roots are *legal* — PRODUCT.md P7's single-inheritance
    // rule is "no role has more than one parent," which a forest
    // satisfies just as well as a single rooted tree.
    let has_root = roles.values().any(|r| r.parent_role.value.is_none());
    if !has_root {
        return Err(ConfigError::RoleInheritance {
            reason: "no root role found (every role has a `parent_role`) — \
                     at least one role must have `parent_role = None` for \
                     the tree to terminate"
                .to_string(),
        });
    }

    // Invariant 5: child-parent attenuation. PRODUCT.md P7 commits
    // to "child can attenuate, never widen" — a role that declares
    // `capability_scopes` must declare a *subset* of its nearest
    // non-empty ancestor's declared scopes (under D4 prefix-
    // attenuation: every declared scope must be `is_granted_by`
    // some scope in that ancestor's set). Q5 resolution: enforce
    // at config-load time so a typo in a child role surfaces with
    // file context, not at the next capability check.
    //
    // **Empty `capability_scopes` is the unconstrained sentinel.**
    // A role with no declared scopes is saying "I add no
    // constraint — take whatever inheritance gives me, or the
    // binary's backcompat floor if nothing else applies." The
    // attenuation walk skips empty links: when looking for a
    // child's effective constraint, we walk up the parent chain
    // through empty roles until we find a non-empty ancestor.
    // If the entire chain to the root is empty, the child has no
    // constraint to validate against and any declared set is
    // legal at config-load time (the binary's backcompat floor
    // and the channel ceiling do the actual capping at runtime).
    //
    // **Why declared sets, not effective sets.** The binary's
    // backcompat floor lives in `aivyx-channel/src/bin/aivyx.rs`,
    // not in `aivyx-config`, and bleeding it into a leaf crate
    // would invert the workspace dep graph. Validating against
    // declared sets keeps `aivyx-config` self-contained: if an
    // operator wants two-level attenuation enforcement, they
    // must explicitly declare scopes on the parent. The
    // implicit-floor path goes one level deep only — which is
    // the right strictness for a backcompat hatch (Q6).
    for (name, role) in roles {
        if role.capability_scopes.value.is_empty() {
            // Empty role declares no constraint — nothing to
            // attenuate against the parent.
            continue;
        }
        // Walk up the parent chain through empty roles until we
        // hit a non-empty ancestor or run out of parents. Cycles
        // were rejected by invariant 3, so this loop terminates.
        let mut cursor = role.parent_role.value.as_deref();
        let constraining_ancestor: Option<&Role> = loop {
            let Some(parent_name) = cursor else {
                break None;
            };
            let Some(parent) = roles.get(parent_name) else {
                // Already caught by invariant 1; keeping the
                // pattern exhaustive for clarity.
                break None;
            };
            if !parent.capability_scopes.value.is_empty() {
                break Some(parent);
            }
            cursor = parent.parent_role.value.as_deref();
        };
        let Some(ancestor) = constraining_ancestor else {
            // Whole chain to root is empty (or this role is
            // itself a root). No constraint to enforce.
            continue;
        };
        let ancestor_name = ancestor.name.value.as_str();
        // Every declared scope on `role` must be granted by some
        // scope on `ancestor`.
        for child_scope in &role.capability_scopes.value {
            let granted = ancestor
                .capability_scopes
                .value
                .iter()
                .any(|parent_scope| child_scope.is_granted_by(parent_scope));
            if !granted {
                let ancestor_scope_strings: Vec<&str> = ancestor
                    .capability_scopes
                    .value
                    .iter()
                    .map(|s| s.as_str())
                    .collect();
                return Err(ConfigError::RoleInheritance {
                    reason: format!(
                        "role `{name}` declares capability scope \
                         {child:?} that is not granted by its \
                         constraining ancestor `{ancestor_name}` \
                         (PRODUCT.md P7: a child may attenuate but \
                         never widen its parent's envelope; \
                         `{ancestor_name}`'s declared scopes are \
                         {ancestor_scope_strings:?})",
                        child = child_scope.as_str(),
                    ),
                });
            }
        }
    }

    Ok(())
}

/// Load and parse the TOML file at `path`, if any.
///
/// Contract:
/// - `None` path → return default (empty) [`RawToml`].
/// - `Some(path)` with no file → return default; not an error.
/// - `Some(path)` with unreadable file → [`ConfigError::TomlIo`].
/// - `Some(path)` with parse failure → [`ConfigError::TomlParse`].
fn load_toml(path: Option<&Path>) -> Result<RawToml, ConfigError> {
    let Some(path) = path else {
        return Ok(RawToml::default());
    };
    if !path.exists() {
        return Ok(RawToml::default());
    }
    let contents = std::fs::read_to_string(path).map_err(|e| ConfigError::TomlIo {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    let parsed: RawToml = toml::from_str(&contents).map_err(|e| ConfigError::TomlParse {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    Ok(parsed)
}

#[cfg(test)]
mod tests;

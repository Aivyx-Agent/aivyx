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
use serde::{Deserialize, Serialize};

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

/// Default assistant name used by [`Profile`] when no `[profile]
/// assistant_name` is declared in TOML. Matches the product name —
/// operators who don't care about renaming get "Aivyx" by default;
/// operators who want a named assistant override it explicitly. Q5(b)
/// resolution at Phase 57 sign-off (PRODUCT.md P13 commit 5).
pub const DEFAULT_ASSISTANT_NAME: &str = "Aivyx";

/// Which LLM provider backend to use.
///
/// `Ollama` is config-level sugar for the OpenAI-compatible
/// provider with Ollama-specific defaults: `base_url` defaults
/// to `http://localhost:11434`, API key is not required, and
/// `stream_options` is omitted from requests (older Ollama
/// versions may reject unknown fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Anthropic,
    #[serde(alias = "openai")]
    OpenAi,
    Ollama,
}

impl ProviderKind {
    /// Returns `true` if this provider uses the OpenAI-compatible
    /// API (either cloud OpenAI or local Ollama).
    pub fn is_openai_compatible(&self) -> bool {
        matches!(self, ProviderKind::OpenAi | ProviderKind::Ollama)
    }

    /// Default context window size in tokens for this provider.
    /// Used by the planner's pruning layer (Phase 43) to decide
    /// when to drop old history messages.
    pub fn default_context_window(&self) -> usize {
        match self {
            ProviderKind::Anthropic => 200_000,
            ProviderKind::OpenAi => 128_000,
            // Ollama models vary widely; 8k is a conservative default
            // that works for most 7B/13B models. Operators can override
            // via config.
            ProviderKind::Ollama => 8_000,
        }
    }
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderKind::Anthropic => f.write_str("anthropic"),
            ProviderKind::OpenAi => f.write_str("openai"),
            ProviderKind::Ollama => f.write_str("ollama"),
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
    /// OpenAI-compatible base URL override. For `ProviderKind::OpenAi`,
    /// `None` means `https://api.openai.com`. For `ProviderKind::Ollama`,
    /// `None` means `http://localhost:11434`. Explicit values override
    /// both defaults.
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
    /// Phase 42 — optional TTL for memory entries, in seconds.
    /// `None` means no TTL (entries live forever). When set, the
    /// daemon periodically calls `gc_expired(now - ttl)` to remove
    /// entries older than this duration.
    pub memory_ttl_secs: Option<Sourced<u64>>,
    /// Phase 74 — per-topic-glob retention rules. First-match wins
    /// at GC time; topics with no matching rule fall through to
    /// the global `memory_ttl_secs` default (no behavior change
    /// for pre-Phase-74 configs).
    pub memory_retention: Vec<MemoryRetentionRule>,
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
    /// Phase 68 — shared SMTP configuration for the email notify
    /// backend. `None` when no `[email]` section is declared.
    /// Required when any `[[notify_target]] kind = "email"` exists;
    /// the loader rejects email targets without `[email]` at load
    /// time.
    pub email: Option<EmailConfig>,
    /// Phase 75 — `[embedding]` section. `None` when the
    /// section is absent: semantic memory search is disabled
    /// and `memory.search` stays keyword-only. When `Some`, the
    /// daemon embeds memory writes and serves
    /// `mode = "semantic"` searches.
    pub embedding: Option<EmbeddingConfig>,
    /// Phase 80 — `[proactive]` section. `None` when absent:
    /// proactive surfacing is off (the assistant never reaches
    /// out unprompted — pre-Phase-80 behavior). `Some` only
    /// arms the pass; it still no-ops unless `enabled = true`.
    pub proactive: Option<ProactiveConfig>,
    /// Phase 81 — `[persona_lifecycle]` section. `None` when
    /// absent: the Persona never self-consolidates or decays
    /// (pre-Phase-81 behavior — it only ever grows). `Some`
    /// only arms the pass; it still no-ops unless
    /// `enabled = true`.
    pub persona_lifecycle: Option<PersonaLifecycleConfig>,
    /// Phase 84 — `[recall_cluster]` section. `None` when
    /// absent: Phase 76 recall is unchanged (pre-Phase-84
    /// behaviour — only literal keyword/semantic hits). `Some`
    /// only arms cluster expansion; it still no-ops unless
    /// `enabled = true`.
    pub recall_cluster: Option<RecallClusterConfig>,
    /// Phase 87 — `[persona_consolidation]` section. `None`
    /// when absent: the Persona proposal pipeline is unchanged
    /// (pre-Phase-87 behaviour — no pattern-driven proposals).
    /// `Some` only arms the pass; it still no-ops unless
    /// `enabled = true`.
    pub persona_consolidation: Option<PersonaConsolidationConfig>,
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
    /// Operator-declared identity layer per **PRODUCT.md P13**
    /// (added by amendment A9, Phase 56). Phase 57 substrate.
    ///
    /// Always populated. Loaded from the `[profile]` TOML table
    /// when present; otherwise [`Profile::default()`] synthesizes
    /// a default carrying `assistant_name = `
    /// [`DEFAULT_ASSISTANT_NAME`] and every other category empty.
    ///
    /// Q1(a) at Phase 57 sign-off: Profile lives in `aivyx.toml`
    /// as a top-level `[profile]` table — single operator-facing
    /// config file, plain-text-inspectable per P13 commit 4.
    pub profile: Profile,
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
    /// Tool process configurations from `[[tool_process]]` entries.
    /// Phase 49 — PRODUCT.md P12. Empty when no entries are configured.
    pub tool_processes: Vec<ToolProcessConfig>,
    /// Scheduled execution entries from `[[schedule]]` entries.
    /// Empty when no entries are configured.
    pub schedules: Vec<ScheduleConfig>,
    /// Webhook trigger entries from `[[webhook]]` entries.
    /// Empty when no entries are configured.
    pub webhooks: Vec<WebhookConfig>,
    /// File-watch trigger entries from `[[file_watch]]` entries.
    /// Empty when no entries are configured.
    pub file_watches: Vec<FileWatchConfig>,
    /// Notification-target entries from `[[notify_target]]`
    /// entries. Phase 62 Task 3 — Reach Milestone phase 1. Empty
    /// when no entries are configured; the `notify.send` tool
    /// then dispatches with "unknown target" failures for any
    /// target name the agent provides.
    pub notify_targets: Vec<NotifyTargetConfig>,
    /// Reflection-schedule entries from `[[reflection_schedule]]`
    /// entries. Phase 70 — P14 self-learning closure. Each entry
    /// fires a periodic reflection turn that synthesizes pending
    /// Persona proposals from observed turn outcomes. Empty when
    /// no entries are configured (the agent only reflects when
    /// an operator explicitly prompts it).
    pub reflection_schedules: Vec<ReflectionScheduleConfig>,
    /// Webhook listener port override. `None` means use the default
    /// (7842). Loaded from `[daemon] webhook_port` in the TOML file.
    pub webhook_port: Option<u16>,
    /// Web UI port. `Some(port)` enables the web UI on that port.
    /// `None` means the web UI is disabled. Set via `[daemon] web_ui = true`
    /// (uses default 7843) or `[daemon] web_ui_port = <N>` (enables on
    /// that port). Phase 39.
    pub web_ui_port: Option<u16>,
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

/// Operator-declared identity layer per **PRODUCT.md P13**. Profile
/// is loaded once per daemon lifetime from the `[profile]` table in
/// `aivyx.toml` and injects into every turn's system prompt
/// regardless of active role. Profile is the role-orthogonal identity
/// layer — roles gate *what* the agent may do, Profile flavors *how*
/// it speaks and judges.
///
/// Phase 57 lands the substrate; Phase 58 lands the operator-facing
/// inspection surface (`aivyx profile show` / `edit`).
///
/// Profile carries no secrets per P13 commit 7 — it is plain-text-
/// inspectable, lives in the operator-facing `aivyx.toml`, and is
/// never used for API keys, passphrases, or tokens.
///
/// The agent **cannot** write to its own Profile (P13 commit 3).
/// Profile changes are operator-driven only. Reflection writes
/// (P8) shape Persona (P14), not Profile.
#[derive(Debug, Clone)]
pub struct Profile {
    /// What the operator calls this specific assistant. Distinct
    /// from the product name (*Aivyx*) and from role names. Always
    /// populated — falls through to [`DEFAULT_ASSISTANT_NAME`] if
    /// no source supplied one (tagged [`FieldSource::Default`]).
    pub assistant_name: Sourced<String>,
    /// Short description of who the operator is — role, expertise
    /// level, primary work context. Drives domain-specific
    /// language and assumed background knowledge in the
    /// assistant's responses. `None` means "not declared."
    pub operator_profile: Option<String>,
    /// Operator preferences on verbosity, formality, citation
    /// frequency, source referencing, list-vs-prose, etc. Free
    /// text — the wizard offers presets but the TOML is
    /// unstructured. `None` means "not declared."
    pub communication_style: Option<String>,
    /// The 1–3 use-case archetypes the assistant is being shaped
    /// around (e.g. *"Rust systems programming"*, *"personal-
    /// finance analysis"*). Drives default domain assumptions.
    /// Empty `Vec` means "not declared."
    pub primary_use_cases: Vec<String>,
    /// Non-capability defaults that flavor the agent's judgment
    /// (e.g. *"prefer integration tests over mocks"*, *"always
    /// cite sources when summarizing"*). Empty `Vec` means "not
    /// declared." Not the same thing as capability scopes — these
    /// are voice-layer preferences, not authority gates.
    pub behavioral_preferences: Vec<String>,
    /// Non-capability guardrails the agent should respect across
    /// every role (e.g. *"never autonomously commit code"*,
    /// *"always confirm destructive shell commands"*). Empty `Vec`
    /// means "not declared." Not the same thing as capability
    /// ceilings — these are voice-layer constraints, not
    /// authority gates.
    pub behavioral_constraints: Vec<String>,
}

impl Default for Profile {
    /// Q5(b) resolution at Phase 57 sign-off: synthesize a default
    /// Profile with [`DEFAULT_ASSISTANT_NAME`] populated and every
    /// other category empty. Matches the existing precedent
    /// ([`DEFAULT_MODEL`], [`DEFAULT_ROLE_NAME`],
    /// [`DEFAULT_SYSTEM_PROMPT`]) — every existing `aivyx.toml`
    /// keeps working without a `[profile]` section.
    fn default() -> Self {
        Self {
            assistant_name: Sourced::new(
                DEFAULT_ASSISTANT_NAME.to_string(),
                FieldSource::Default,
            ),
            operator_profile: None,
            communication_style: None,
            primary_use_cases: Vec::new(),
            behavioral_preferences: Vec::new(),
            behavioral_constraints: Vec::new(),
        }
    }
}

impl Profile {
    /// `true` if the operator declared any Profile content — i.e.
    /// either `assistant_name` was supplied (so its source is `Toml`,
    /// not `Default`) or any of the other five fields is non-empty.
    ///
    /// Phase 57 Task 3 consumer: when this returns `false`, the
    /// system-prompt assembly path skips the Profile section
    /// entirely and emits the role's `system_prompt` unchanged.
    /// This keeps the substrate non-invasive — every pre-Phase-57
    /// `aivyx.toml` sees zero behavior change unless it actually
    /// declares a `[profile]` section.
    pub fn is_operator_declared(&self) -> bool {
        self.assistant_name.source != FieldSource::Default
            || self.operator_profile.is_some()
            || self.communication_style.is_some()
            || !self.primary_use_cases.is_empty()
            || !self.behavioral_preferences.is_empty()
            || !self.behavioral_constraints.is_empty()
    }
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

/// Transport kind for an MCP server connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpTransportKind {
    /// Local child process over stdio (Phase 23).
    Stdio,
    /// Remote HTTP server over SSE (Phase 32).
    Sse,
}

/// One MCP server to connect to at daemon startup.
/// Loaded from `[[mcp_server]]` entries in `aivyx.toml`.
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransportKind,
    /// Command to spawn (stdio transport only).
    pub command: Option<String>,
    /// Command-line arguments (stdio transport only).
    pub args: Vec<String>,
    /// SSE endpoint URL (SSE transport only).
    pub url: Option<String>,
    pub enabled: bool,
    /// When `true`, the binary resolves `command` to `std::env::current_exe()`
    /// before spawning. Used for bundled MCP servers (Phase 46).
    pub bundled: bool,
    /// Phase 55 — optional sandbox wrapper for the stdio spawn.
    /// `None` for SSE transport (no local child to wrap).
    /// Reuses the same `SandboxConfig` type as
    /// `[[tool_process]]` — see `docs/TOOL_SDK.md` §9.
    pub sandbox: Option<SandboxConfig>,
}

/// One tool process to spawn at daemon startup. Phase 49 — delivers
/// PRODUCT.md P12 (Tools as Separate Processes Over Daemon IPC).
/// Loaded from `[[tool_process]]` entries in `aivyx.toml`.
///
/// `scope_overrides` lets the operator narrow (never widen) the scopes
/// the tool declares at handshake. Keys are tool names within the
/// tool process; values are scope strings that must `is_granted_by`
/// the declared scope. The daemon enforces the narrowing rule at
/// registration time — see `docs/TOOL_SDK.md` §6.
#[derive(Debug, Clone)]
pub struct ToolProcessConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Per-tool scope overrides keyed by tool name. Operator may only
    /// narrow what the tool declared; the daemon rejects widenings.
    pub scope_overrides: std::collections::HashMap<String, String>,
    pub enabled: bool,
    /// Phase 52 — optional sandbox wrapper. When present, the daemon
    /// spawns `wrapper wrapper_args... command command_args...`
    /// instead of `command command_args...`. Aivyx supplies the
    /// policy slot; the operator supplies the policy (bubblewrap,
    /// firejail, docker run, sandbox-exec — see `docs/TOOL_SDK.md`
    /// §9).
    pub sandbox: Option<SandboxConfig>,
}

/// Phase 52 — operator-supplied command wrapper that hardens a
/// `[[tool_process]]` spawn. Threaded into
/// `aivyx_tool::SandboxConfig` at daemon startup.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub wrapper: String,
    pub args: Vec<String>,
}

/// Phase 72 — conditional dispatch gate. When a trigger's
/// `notify_when` is anything other than `Always`, the daemon
/// evaluates the turn outcome (and, for
/// `OnCompletedNonEmpty`, the rendered response body) before
/// fanning out to the notify targets. A gate that returns
/// `false` records `AutoNotifyOutcomeSummary::SkippedByCondition`
/// in the audit chain so forensic searches can answer "why
/// didn't this trigger notify?" definitively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NotifyWhen {
    /// Today's behavior — dispatch unconditionally on every
    /// trigger fire. Empty responses still get
    /// `SkippedEmptyResponse` audit treatment per the Phase 63
    /// Q2(a) rule baked into the dispatch path.
    #[default]
    Always,
    /// Dispatch only when the turn's outcome is `Failed` or
    /// `TimedOut`. Completed / Cancelled / Escalated outcomes
    /// skip dispatch.
    OnFailed,
    /// Dispatch only when the turn completed AND the final
    /// response body is non-whitespace. The audit chain's
    /// existing `SkippedEmptyResponse` still records the
    /// empty-body case; this variant additionally skips
    /// `Failed | TimedOut | Cancelled | Escalated` outcomes
    /// (operators who want "only when something useful was
    /// produced").
    OnCompletedNonEmpty,
}

impl NotifyWhen {
    /// Stable label rendered into audit `condition` strings
    /// when a dispatch skips because of this gate.
    pub fn condition_label(self) -> &'static str {
        match self {
            NotifyWhen::Always => "always",
            NotifyWhen::OnFailed => "on_failed",
            NotifyWhen::OnCompletedNonEmpty => "on_completed_non_empty",
        }
    }
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
    /// Phase 63 Task 2 — kept as a singular alias for backwards
    /// compatibility with pre-Phase-72 configs. When set, the
    /// loader bridges it into `notify_targets` as a
    /// one-element vec so downstream consumers always read the
    /// vec. Declaring both `notify_target` and `notify_targets`
    /// on the same trigger is rejected at config-load time
    /// (Phase 72 Q1(a)).
    pub notify_target: Option<String>,
    /// Phase 72 — list of notify target names this trigger
    /// dispatches to on fire. The daemon fans out concurrently
    /// (Q4(a)); per-target outcomes are audited independently.
    /// When empty AND a `[[notify_target]]` is marked
    /// `default = true`, the loader resolves the default into
    /// this vec at config-load time so runtime dispatch never
    /// has to ask "which target is default?" again.
    pub notify_targets: Vec<String>,
    /// Phase 72 — conditional dispatch gate. Defaults to
    /// `Always` (today's behavior, no behavior change for
    /// pre-Phase-72 configs).
    pub notify_when: NotifyWhen,
}

/// One reflection-schedule entry loaded from
/// `[[reflection_schedule]]` in the TOML file. Phase 70 — P14
/// self-learning closure. Each entry fires a periodic reflection
/// turn that synthesizes pending Persona proposals from observed
/// turn outcomes for the configured lookback window.
///
/// The scheduler reuses the existing `[[schedule]]` cron
/// infrastructure under the hood; this is a distinct config
/// section because the reflection-turn semantics — canonical
/// reflection prompt, outcome-summary input context, persistent
/// proposal store — differ enough from a generic scheduled turn
/// that operator clarity wins over composability (Q1(a) at
/// Phase 70 sign-off).
#[derive(Debug, Clone)]
pub struct ReflectionScheduleConfig {
    /// Operator-chosen name, unique across reflection schedules
    /// and across regular `[[schedule]]` entries.
    pub name: String,
    /// Standard 5- or 6-field cron pattern, parsed by the same
    /// cron implementation `[[schedule]]` uses.
    pub cron: String,
    /// How far back to look when summarizing turn outcomes for
    /// the reflection prompt. Minimum 60 seconds, maximum 30
    /// days. Default 86400 (24 hours).
    pub lookback_window_secs: u64,
    /// Optional role override. When `Some(name)`, the reflection
    /// turn runs as that role instead of the default reflection
    /// envelope. The role must exist in the config.
    pub role_override: Option<String>,
    /// `true` when the entry is active; `false` keeps the entry
    /// in the config but skips scheduler registration.
    pub enabled: bool,
}

/// Phase 74 — retention policy for a `[[memory.retention]]` block.
/// Operators declare either `retention = "forever"` (entries never
/// expire by TTL) or `retention_days = N` (entries older than N days
/// are evicted by the GC pass). Exactly one form is set per block;
/// the loader rejects partial config naming the missing field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionPolicy {
    /// Topic entries are never evicted by the GC's TTL pass.
    /// Per-topic-cap LRU eviction (`memory_max_per_topic`) still
    /// applies.
    Forever,
    /// Topic entries older than N days are evicted by the GC pass.
    /// `0` is meaningless (would evict everything immediately) and
    /// rejects at load time.
    ForDays(u64),
}

/// Phase 74 — one `[[memory.retention]]` rule. The loader compiles
/// `topic_glob` into a `globset::GlobMatcher` at config-load time so
/// the runtime GC walk is a fast match-or-skip per entry; the
/// compiled matcher is held alongside the raw pattern string for
/// diagnostics. `GlobMatcher` is `Send + Sync + Clone`, which keeps
/// `MemoryRetentionRule` cheap to clone across the config-to-daemon
/// boundary.
#[derive(Debug, Clone)]
pub struct MemoryRetentionRule {
    /// Raw glob pattern as declared in TOML (e.g. `"project/*"`,
    /// `"notes/**"`, `"daily-*"`). Kept for diagnostics and the
    /// startup-banner render.
    pub topic_glob: String,
    /// Compiled matcher. Built once at config-load time. The
    /// runtime GC pass calls `is_match` per entry to find the
    /// first applicable rule.
    pub matcher: globset::GlobMatcher,
    /// What to do with matching entries.
    pub retention: RetentionPolicy,
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
    /// See [`ScheduleConfig::notify_target`] — singular alias.
    pub notify_target: Option<String>,
    /// Phase 72 — see [`ScheduleConfig::notify_targets`].
    pub notify_targets: Vec<String>,
    /// Phase 72 — see [`ScheduleConfig::notify_when`].
    pub notify_when: NotifyWhen,
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
    /// See [`ScheduleConfig::notify_target`] — singular alias.
    pub notify_target: Option<String>,
    /// Phase 72 — see [`ScheduleConfig::notify_targets`].
    pub notify_targets: Vec<String>,
    /// Phase 72 — see [`ScheduleConfig::notify_when`].
    pub notify_when: NotifyWhen,
}

/// One notification-target entry loaded from `[[notify_target]]` in
/// the TOML file. Phase 62 Task 3 — operator-feedback-shaped Reach
/// Milestone phase 1. The agent calls `notify.send` (Phase 62 Task
/// 7) to push a message to one of these targets.
///
/// Invalid combinations (e.g. `kind = "telegram"` without a
/// `chat_id`) are rejected at config-load time and never
/// represented in the runtime [`NotifyTargetConfig`] / [`NotifyTargetKind`]
/// pair — the kind enum carries kind-specific fields directly so
/// the runtime cannot observe an inconsistent state.
#[derive(Debug, Clone)]
pub struct NotifyTargetConfig {
    pub name: String,
    pub kind: NotifyTargetKind,
    pub enabled: bool,
    /// Phase 72 — when `true`, this target is the global default
    /// triggers fall through to when they omit `notify_targets`.
    /// At most one `[[notify_target]]` may set this; the loader
    /// rejects multiple defaults at config-load time.
    pub is_default: bool,
    /// Phase 73 — number of retry attempts after the initial
    /// dispatch fails with a transient error class
    /// (`Transport`, `Timeout`, or `Rejected` with HTTP status
    /// ≥ 500 per Q2(b)). Default `0` preserves Phase 62 behavior.
    /// Capped at 10 by the loader.
    pub retry_count: u32,
    /// Phase 73 — starting backoff for the first retry, in
    /// milliseconds. Each subsequent retry waits double the
    /// previous (`backoff * 2^attempt`). Default 500 ms; loader
    /// rejects values below 100 ms.
    pub retry_backoff_ms_start: u64,
    /// Phase 73 — when both this and
    /// [`Self::rate_limit_window_secs`] are `Some`, the daemon
    /// allows at most `rate_limit_max` dispatch attempts per
    /// `rate_limit_window_secs` per target. Excess attempts
    /// record `AutoNotifyOutcomeSummary::SkippedByRateLimit`
    /// in the audit chain and skip the backend call.
    pub rate_limit_max: Option<u32>,
    /// Phase 73 — sliding-window length for [`Self::rate_limit_max`].
    /// Both fields must be set together or neither — the loader
    /// rejects partial config naming the missing field.
    pub rate_limit_window_secs: Option<u64>,
}

/// Per-kind notification target configuration. Phase 62 ships two
/// kinds: Telegram (uses the operator's existing bot client to
/// push a message to the named `chat_id`) and Webhook (HTTP POST
/// with a small JSON body to the configured `url`). Additional
/// kinds (email SMTP, Web UI desktop notification, OS-level
/// notification) are recorded as Phase 62 deferrals.
#[derive(Debug, Clone)]
pub enum NotifyTargetKind {
    /// Telegram bot outbound. `chat_id` is the operator-owned chat
    /// the bot is already authorized to message — typically the
    /// same `chat_id` declared under `[telegram]` for the inbound
    /// path, but explicitly named here so multiple chats can be
    /// configured independently.
    Telegram { chat_id: String },
    /// Generic HTTP webhook. The dispatcher POSTs a JSON body of
    /// shape `{source, target, subject?, message, timestamp}` per
    /// Q5(a) at sign-off. Suitable for ntfy.sh, Pushover, IFTTT,
    /// and custom endpoints. Slack-flavored payload (`{text: ...}`)
    /// is a Phase 62 deferral.
    Webhook { url: String },
    /// Phase 68 — SMTP email outbound. `to` is the recipient
    /// address; the SMTP server, credentials, and `from` address
    /// live in the top-level `[email]` config section (shared
    /// across every email target per Q2(a) at sign-off). One
    /// `LettreEmailSender` is constructed at daemon startup and
    /// Arc-cloned into each email target's backend.
    Email { to: String },
    /// Phase 69 — Web UI desktop notification. Pushes onto a
    /// broadcast channel that the Web UI WebSocket connection
    /// handlers subscribe to; browser-side JS triggers the
    /// `Notification` API + an in-page toast. No per-target
    /// fields — one Web UI per daemon. `kind = "web-ui"`.
    WebUi,
}

/// Phase 68 — SMTP TLS mode discriminator. Defaults to
/// `Starttls` (modern submission standard supported by Gmail,
/// Fastmail, ProtonMail bridge, AWS SES, etc.). Operators with
/// legacy infrastructure can override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    /// Plain TCP that upgrades to TLS via STARTTLS. Port 587 by
    /// default.
    Starttls,
    /// TLS from byte zero. Port 465 by default.
    Implicit,
    /// No TLS. Always rejected at load time when paired with
    /// PLAIN/LOGIN auth — sending credentials in cleartext over
    /// the wire is a misconfiguration the loader refuses, not a
    /// runtime surprise.
    None,
}

/// Phase 68 — shared SMTP configuration. One per-deployment;
/// every `[[notify_target]] kind = "email"` reuses it. The
/// password is stored as `SourcedSecret` so it never lands in a
/// plain `String` field (matches the `[telegram] token` and
/// `[anthropic] api_key` patterns).
#[derive(Debug, Clone)]
pub struct EmailConfig {
    /// SMTP server hostname (e.g. `"smtp.gmail.com"`,
    /// `"smtp.fastmail.com"`).
    pub host: String,
    /// SMTP server port. Defaults to 587 for STARTTLS or 465 for
    /// implicit TLS; an explicit override wins.
    pub port: u16,
    /// TLS mode for the connection.
    pub tls_mode: TlsMode,
    /// SMTP username. Often the same as `from` but explicit so
    /// providers using account-id-as-username (some self-hosted
    /// setups) are supported.
    pub username: SourcedSecret,
    /// SMTP password. For Gmail and most cloud providers this is
    /// an "app password," not the operator's account password.
    pub password: SourcedSecret,
    /// Sender address. Appears in the `From:` header.
    pub from: String,
}

/// Phase 75 — `[embedding]` section. Configures the
/// OpenAI-compatible embedding backend that powers semantic
/// memory search. `None` on [`AivyxConfig`] means the section
/// was absent: semantic search is disabled and `memory.search`
/// keeps working in keyword mode (no behavior change for
/// pre-Phase-75 configs).
///
/// `base_url` is the privacy lever: point it at
/// `https://api.openai.com` and memory content is sent to
/// OpenAI; point it at a local OpenAI-compatible server
/// (ollama, llama.cpp, text-embeddings-inference) and nothing
/// leaves the box. The default is the OpenAI public endpoint —
/// the operator opts into locality explicitly.
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    /// Embeddings API base URL. Default
    /// [`DEFAULT_EMBEDDING_BASE_URL`]. The provider POSTs to
    /// `{base_url}/v1/embeddings`.
    pub base_url: String,
    /// Embedding model id. Default [`DEFAULT_EMBEDDING_MODEL`].
    pub model: String,
    /// API key. `Option` because a local server needs none.
    /// `SourcedSecret` so a stray `{:?}` never leaks it and the
    /// startup banner can show provenance — same pattern as the
    /// anthropic / openai keys (env > TOML > encrypted store).
    pub api_key: Option<SourcedSecret>,
    /// Expected vector dimensionality. Default
    /// [`DEFAULT_EMBEDDING_DIMENSIONS`] (text-embedding-3-small).
    /// The vector store uses this to detect a model swap:
    /// stored vectors with a different length are treated as
    /// unembedded and lazily re-embedded.
    pub dimensions: usize,
    /// Phase 76 — automatic-recall fan-out: how many of the
    /// top semantic hits the per-turn recall hook may inject.
    /// Default [`DEFAULT_RAG_TOP_K`]. Must be ≥ 1.
    pub rag_top_k: usize,
    /// Phase 76 — automatic-recall relevance floor: a hit whose
    /// cosine similarity is below this is dropped even when
    /// `rag_top_k` is not filled. This is what stops naive RAG
    /// from injecting weak/irrelevant memories on every
    /// unrelated prompt. Default [`DEFAULT_RAG_MIN_SIMILARITY`].
    /// Must be in `[0.0, 1.0]`.
    pub rag_min_similarity: f32,
    /// Phase 86 — conversational-window relevance: the number
    /// of recent turns (current user message included) that
    /// auto-recall and adaptive-Persona selection embed
    /// together as their relevance query. Default
    /// [`DEFAULT_RECALL_WINDOW_TURNS`] (`1`) is
    /// byte-identical to pre-Phase-86 (the latest message
    /// only). Must be `>= 1`.
    pub recall_window_turns: usize,
}

/// Default embeddings endpoint — the OpenAI public API. An
/// operator who wants on-device embedding overrides this with
/// a local OpenAI-compatible server URL.
pub const DEFAULT_EMBEDDING_BASE_URL: &str = "https://api.openai.com";
/// Default embedding model. `text-embedding-3-small` is the
/// cheap, widely-supported OpenAI default; local servers
/// generally accept an arbitrary model string.
pub const DEFAULT_EMBEDDING_MODEL: &str = "text-embedding-3-small";
/// Default vector dimensionality — the native size of
/// `text-embedding-3-small`.
pub const DEFAULT_EMBEDDING_DIMENSIONS: usize = 1536;
/// Phase 76 — default auto-recall top-K. Small on purpose: a
/// handful of highly-relevant memories beats a wall of
/// loosely-related ones for prompt quality and token cost.
pub const DEFAULT_RAG_TOP_K: usize = 5;
/// Phase 76 — default auto-recall similarity floor. Cosine
/// similarity runs `[-1.0, 1.0]`; 0.20 keeps clearly-related
/// hits while dropping the near-orthogonal noise that an
/// unrelated prompt would otherwise pull in.
pub const DEFAULT_RAG_MIN_SIMILARITY: f32 = 0.20;
/// Phase 86 — default conversational-window size: 1 means
/// "just the latest message" = byte-identical to pre-Phase-86
/// recall/Persona-selection. The operator opts into a larger
/// window by raising this; the project's behaviour-change-is-
/// opt-in discipline (recall context feeds model output).
pub const DEFAULT_RECALL_WINDOW_TURNS: usize = 1;

/// Phase 80 — which structural signal classes the proactive
/// pass is allowed to surface. All default `true`: an operator
/// who turns proactive on generally wants every conservative
/// signal, and can disable individual classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProactiveSignals {
    /// A memory entry within the warn window of TTL eviction.
    pub ttl_expiry: bool,
    /// A topic whose Phase-77 net helpfulness is strongly
    /// positive over the high threshold.
    pub recall_cluster: bool,
    /// Reminder-shaped memory whose due time has arrived.
    pub due_reminder: bool,
}

impl Default for ProactiveSignals {
    fn default() -> Self {
        ProactiveSignals {
            ttl_expiry: true,
            recall_cluster: true,
            due_reminder: true,
        }
    }
}

/// Phase 80 — operator-facing config for proactive surfacing
/// (the assistant reaching out unprompted). **Off unless an
/// `[proactive]` section is present *and* `enabled = true`** —
/// an unprompted outbound message is the highest-trust-stakes
/// action, so it is opt-in, hard-capped, and never a
/// surprise-on-upgrade.
#[derive(Debug, Clone)]
pub struct ProactiveConfig {
    /// Master switch. Default `false`; even with the section
    /// present the pass is a no-op until this is `true`.
    pub enabled: bool,
    /// Notify-target name the surfacing is dispatched to (must
    /// match a configured `[[notify_target]]`). Required when
    /// `enabled`.
    pub target: String,
    /// Hard cap on proactive sends per `window_secs`, on top of
    /// the per-target Phase 73 rate-limit. The total volume
    /// guard regardless of how much the signal fires.
    pub max_per_window: u32,
    /// The cap's window, in seconds. Default
    /// [`DEFAULT_PROACTIVE_WINDOW_SECS`].
    pub window_secs: u64,
    /// Which structural signal classes may surface.
    pub signals: ProactiveSignals,
}

/// Default proactive volume cap: at most this many unprompted
/// surfacings per [`DEFAULT_PROACTIVE_WINDOW_SECS`]. Small on
/// purpose — proactive is a scalpel, not a feed.
pub const DEFAULT_PROACTIVE_MAX_PER_WINDOW: u32 = 3;
/// Default proactive cap window — one day.
pub const DEFAULT_PROACTIVE_WINDOW_SECS: u64 = 86_400;

/// Phase 81 — which lifecycle action classes the persona-
/// lifecycle pass may propose. Both default `true`: an
/// operator who turns the lifecycle on generally wants the
/// Soul kept tidy, and can disable a class individually.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersonaLifecycleSignals {
    /// Propose merging near-duplicate facets in a soft list.
    pub consolidate: bool,
    /// Propose retiring a long-unreinforced facet.
    pub decay: bool,
}

impl Default for PersonaLifecycleSignals {
    fn default() -> Self {
        PersonaLifecycleSignals {
            consolidate: true,
            decay: true,
        }
    }
}

/// Phase 81 — operator-facing config for Persona lifecycle
/// (consolidation + decay of the learned soft-list facets).
/// **Off unless a `[persona_lifecycle]` section is present
/// *and* `enabled = true`.** The pass only ever *proposes*
/// (the operator approves/rejects and every action is
/// reversible) and never touches the always-on core, but
/// mutating identity is high-stakes, so it is opt-in and never
/// a surprise-on-upgrade.
#[derive(Debug, Clone)]
pub struct PersonaLifecycleConfig {
    /// Master switch. Default `false`; even with the section
    /// present the pass is a no-op until this is `true`.
    pub enabled: bool,
    /// Cosine threshold above which two facets in the same
    /// soft list are treated as near-duplicates and a merge is
    /// proposed. In `(0.0, 1.0]`; high by design.
    pub consolidation_similarity: f32,
    /// A soft-list facet whose originating delta is older than
    /// this many seconds, with no later reinforcing delta in
    /// its category, is proposed for decay.
    pub decay_max_age_secs: u64,
    /// Never act on a soft list with fewer than this many
    /// facets — a small Soul has nothing worth pruning.
    pub min_soft_facets: u32,
    /// Phase 85 — a recall-feedback-derived facet whose
    /// associated topic's durable (Phase 82) decayed
    /// helpfulness is **at or below** this (negative)
    /// value is treated as "sustained low": it may be
    /// proposed for decay before the age horizon, and a
    /// strongly-positive topic (>= the magnitude of this
    /// value) instead *protects* an age-old facet from
    /// age-decay. Reflection-authored facets (no topic
    /// linkage) ignore this and stay age-only.
    pub decay_unhelpful_threshold: f32,
    /// Phase 85 — confidence floor: a topic's helpfulness is
    /// only consulted once it has at least this many ledger
    /// samples. Identity is never decayed (or protected) on
    /// thin evidence.
    pub decay_min_samples: u32,
    /// Phase 88 — a `consolidate-pair:` facet whose pair's
    /// decayed Phase 83 affinity is **below** this floor is
    /// treated as "relationship no longer durable": the facet
    /// may be proposed for decay before the age horizon, and
    /// symmetrically, a pair whose affinity is **at or above**
    /// this floor *protects* its facet from age-decay. Mirrors
    /// the Phase 87 `[persona_consolidation].min_affinity`
    /// default (1.0) — a pair must be ≥ 1.0 to propose a facet
    /// (Phase 87), and staying ≥ 1.0 keeps the facet (Phase
    /// 88). Reflection-authored facets (no `consolidate-pair:`
    /// provenance) ignore this and stay age-only.
    pub decay_pair_below_affinity: f32,
    /// Which lifecycle action classes may be proposed.
    pub signals: PersonaLifecycleSignals,
}

/// Default near-duplicate cosine threshold. High on purpose —
/// only facets that are essentially the same should merge.
pub const DEFAULT_PL_CONSOLIDATION_SIMILARITY: f32 = 0.92;
/// Default decay horizon — ~90 days. A soft-list facet
/// untouched and unreinforced for a quarter is a stale-Soul
/// candidate.
pub const DEFAULT_PL_DECAY_MAX_AGE_SECS: u64 = 90 * 24 * 3600;
/// Default soft-list floor: never prune a list smaller than
/// this — a young Soul has nothing to tidy.
pub const DEFAULT_PL_MIN_SOFT_FACETS: u32 = 6;
/// Phase 85 — default "sustained low helpfulness" floor. A
/// recall topic whose durable decayed score sits at/below
/// -2.0 has, net, consistently hurt the turns it was recalled
/// into; symmetrically, >= +2.0 protects an age-old facet.
pub const DEFAULT_PL_DECAY_UNHELPFUL_THRESHOLD: f32 = -2.0;
/// Phase 85 — default confidence floor: don't consult a
/// topic's helpfulness for decay/protection until it has at
/// least this many ledger samples.
pub const DEFAULT_PL_DECAY_MIN_SAMPLES: u32 = 3;
/// Phase 88 — default pair-affinity floor for the decay /
/// protection arm. Mirrors the Phase 87
/// `DEFAULT_PC_MIN_AFFINITY` (= `1.0`) so the construction
/// floor and the decay floor coincide by default: a pair must
/// be ≥ 1.0 to propose a facet, and staying ≥ 1.0 keeps the
/// facet. An operator who wants explicit hysteresis can tune
/// this *below* `min_affinity` to widen the keep-zone.
pub const DEFAULT_PL_DECAY_PAIR_BELOW_AFFINITY: f32 = 1.0;

/// Phase 84 — operator-facing config for cluster-aware
/// co-recall (consuming the Phase 83 co-occurrence ledger
/// inside the Phase 76 recall path). **Off unless a
/// `[recall_cluster]` section is present *and* `enabled =
/// true`.** This is the first phase that acts on the learned
/// signal and changes what the model sees on the hot path, so
/// it is opt-in and never a surprise-on-upgrade.
#[derive(Debug, Clone)]
pub struct RecallClusterConfig {
    /// Master switch. Default `false`; even with the section
    /// present and a populated ledger, recall is unchanged
    /// until this is `true`.
    pub enabled: bool,
    /// Hard per-turn cap on injected sibling memories. They
    /// share the existing `rag_top_k` budget (displacing the
    /// weakest primary hits), so this also bounds how much of
    /// the budget cluster expansion may claim.
    pub max_siblings: u32,
    /// A sibling's decayed co-occurrence score must be at
    /// least this for the pair to be eligible — the bar that
    /// keeps weak/noisy affinities out of recall context.
    pub min_affinity: f32,
}

/// Default per-turn sibling cap — small on purpose; cluster
/// expansion is a scalpel, not a flood, and it shares the
/// `rag_top_k` budget.
pub const DEFAULT_RC_MAX_SIBLINGS: u32 = 3;
/// Default affinity floor: a pair must have accumulated at
/// least roughly one sustained helpful co-occurrence (after
/// decay) before it steers recall.
pub const DEFAULT_RC_MIN_AFFINITY: f32 = 1.0;

/// Phase 87 — `[persona_consolidation]` runtime config.
///
/// The actuator surface for pattern-driven Persona proposals:
/// when the Phase 83 co-occurrence ledger surfaces a durable
/// pair `(A, B)` whose endpoints are *both* helpful (Phase 82
/// ledger, Q1a's conservative double-gate), the reflection
/// cron asks the existing reflection LLM (Q2b) to phrase a
/// `learned_context` facet and files it through the existing
/// Phase 70 proposal chain. Same propose-only + edit-then-
/// approve + Revert + core-protected flow; opt-in (Q4a).
///
/// `None` (no section) → the pass never runs; the Persona
/// proposal pipeline is byte-identical to pre-Phase-87.
/// `Some` arms the pass; it still no-ops unless
/// `enabled = true`.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonaConsolidationConfig {
    /// Master switch. Default `false`; even with the section
    /// present and ledgers populated, no consolidation
    /// proposals are filed until this is `true`.
    pub enabled: bool,
    /// The decayed Phase 83 pair-affinity floor a candidate
    /// must clear — the same idea (and same default) as the
    /// Phase 84 `recall_cluster.min_affinity`, applied to the
    /// proposal-side of the symmetric arc.
    pub min_affinity: f32,
    /// Minimum observation count on the pair before it is
    /// proposal-eligible. Mirrors Phase 85's
    /// `decay_min_samples`: identity is never proposed on
    /// thin evidence.
    pub min_samples: u32,
    /// Both endpoints' Phase 82 helpfulness-ledger scores must
    /// be at least this value (Q1a's conservative double-gate).
    /// Default `0.0` enforces "non-negative" — a pattern made
    /// of topics that individually hurt is never proposed;
    /// raise it to require *positive* helpfulness on both
    /// sides.
    pub min_topic_helpfulness: f32,
    /// Hard cap on filings per reflection cycle. Mirrors the
    /// Phase 80 `max_per_cycle` precedent — actuators on the
    /// reflection cadence never flood the operator's queue.
    pub max_proposals_per_cycle: u32,
}

/// Default pair-affinity floor. Same value (and same
/// reasoning) as `DEFAULT_RC_MIN_AFFINITY` — the proposal-side
/// of the symmetric arc adopts the recall-side's already-tuned
/// floor.
pub const DEFAULT_PC_MIN_AFFINITY: f32 = 1.0;
/// Default sample-count floor on the pair. Same value as
/// Phase 85's `DEFAULT_DECAY_MIN_SAMPLES` — identity is never
/// proposed on thin evidence.
pub const DEFAULT_PC_MIN_SAMPLES: u32 = 3;
/// Default helpfulness floor on each endpoint: non-negative.
/// A pattern of consistently-hurting topics is never proposed;
/// "merely-not-harmful" is enough at the default.
pub const DEFAULT_PC_MIN_TOPIC_HELPFULNESS: f32 = 0.0;
/// Default per-cycle filing cap. Same value as the Phase 80
/// proactive cap — the operator's review queue is the
/// bottleneck, and a passive actuator should err on the side
/// of patience.
pub const DEFAULT_PC_MAX_PROPOSALS_PER_CYCLE: u32 = 3;

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
    email: RawEmail,
    /// `[embedding]` section. Phase 75 — semantic memory search.
    #[serde(default)]
    embedding: RawEmbedding,
    /// `[proactive]` section. Phase 80 — proactive surfacing.
    #[serde(default)]
    proactive: RawProactive,
    /// `[persona_lifecycle]` section. Phase 81 — Persona
    /// consolidation + decay.
    #[serde(default)]
    persona_lifecycle: RawPersonaLifecycle,
    /// `[recall_cluster]` section. Phase 84 — cluster-aware
    /// co-recall.
    #[serde(default)]
    recall_cluster: RawRecallCluster,
    /// `[persona_consolidation]` section. Phase 87 —
    /// pattern-driven Persona proposals.
    #[serde(default)]
    persona_consolidation: RawPersonaConsolidation,
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
    /// `[[tool_process]]` table-array. Phase 49 — PRODUCT.md P12.
    #[serde(default, rename = "tool_process")]
    tool_processes: Option<Vec<RawToolProcess>>,
    /// `[[schedule]]` table-array. Phase 26 Task 2.
    #[serde(default, rename = "schedule")]
    schedules: Option<Vec<RawSchedule>>,
    /// `[[webhook]]` table-array. Phase 27 Task 3.
    #[serde(default, rename = "webhook")]
    webhooks: Option<Vec<RawWebhook>>,
    /// `[[file_watch]]` table-array. Phase 27 Task 4.
    #[serde(default, rename = "file_watch")]
    file_watches: Option<Vec<RawFileWatch>>,
    /// `[[notify_target]]` table-array. Phase 62 Task 3 —
    /// operator-configured notification destinations the agent
    /// can reach via `notify.send`.
    #[serde(default, rename = "notify_target")]
    notify_targets: Option<Vec<RawNotifyTarget>>,
    /// `[[reflection_schedule]]` table-array. Phase 70 — P14
    /// self-learning closure.
    #[serde(default, rename = "reflection_schedule")]
    reflection_schedules: Option<Vec<RawReflectionSchedule>>,
    /// `[daemon]` section. Phase 28 Task 3.
    #[serde(default)]
    daemon: RawDaemon,
    /// `[profile]` section. Phase 57 (PRODUCT.md P13). Absent
    /// section deserializes via `Default` into an all-`None` /
    /// all-empty raw shape, which the loader then maps to
    /// [`Profile::default()`].
    #[serde(default)]
    profile: RawProfile,
}

/// `[daemon]` section in the TOML file. Phase 28 Task 3.
/// Phase 39 adds `web_ui` and `web_ui_port` for the web UI channel.
#[derive(Debug, Default, Deserialize)]
struct RawDaemon {
    webhook_port: Option<u16>,
    web_ui: Option<bool>,
    web_ui_port: Option<u16>,
}

/// `[profile]` section in the TOML file. Phase 57 (PRODUCT.md P13).
/// Every field optional — an absent section deserializes into the
/// all-`None`/all-empty shape via `Default`, which the loader then
/// maps to [`Profile::default()`].
///
/// The TOML keys match the six P13 commit-5 categories. Field names
/// in the operator-facing TOML are spelled out (e.g.
/// `behavioral_preferences`, not `preferences`) so the config file
/// is self-documenting without per-key comments.
#[derive(Debug, Default, Deserialize)]
struct RawProfile {
    #[serde(default)]
    assistant_name: Option<String>,
    #[serde(default)]
    operator_profile: Option<String>,
    #[serde(default)]
    communication_style: Option<String>,
    #[serde(default)]
    primary_use_cases: Option<Vec<String>>,
    #[serde(default)]
    behavioral_preferences: Option<Vec<String>>,
    #[serde(default)]
    behavioral_constraints: Option<Vec<String>>,
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

/// One `[[mcp_server]]` entry in the TOML file. Phase 24 Task 2,
/// extended in Phase 32 Task 4 for SSE transport.
#[derive(Debug, Default, Deserialize)]
struct RawMcpServer {
    name: String,
    /// Transport kind: `"stdio"` (default) or `"sse"`.
    #[serde(default = "default_stdio_transport")]
    transport: String,
    /// Command to spawn (stdio transport).
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Option<Vec<String>>,
    /// SSE endpoint URL (SSE transport).
    #[serde(default)]
    url: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
    /// When `true`, resolve `command` to the current binary path at runtime.
    /// Used for bundled MCP servers that ship inside the `aivyx` binary.
    #[serde(default)]
    bundled: bool,
    /// Phase 55 — optional `[mcp_server.sandbox]` nested block.
    /// Reuses `RawSandbox` from the `[[tool_process]]` schema.
    #[serde(default)]
    sandbox: Option<RawSandbox>,
}

/// One `[[tool_process]]` entry in the TOML file. Phase 49.
///
/// Layout:
///
/// ```toml
/// [[tool_process]]
/// name = "wordcount"
/// command = "python3"
/// args = ["/path/to/tool.py"]
///
/// # Optional environment additions.
/// env = { LOG_LEVEL = "info" }
///
/// # Optional per-tool scope narrowing. Keys are tool names declared
/// # in the process's ToolRegister; values are scope strings that
/// # must be granted by the declared scope.
/// [tool_process.scope_overrides]
/// wordcount = "memory.read:topic:wordcount/**"
///
/// enabled = true   # default
/// ```
#[derive(Debug, Default, Deserialize)]
struct RawToolProcess {
    name: String,
    command: String,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    env: Option<std::collections::HashMap<String, String>>,
    #[serde(default)]
    scope_overrides: Option<std::collections::HashMap<String, String>>,
    #[serde(default = "default_true")]
    enabled: bool,
    /// Phase 52 — optional `[tool_process.sandbox]` nested block.
    #[serde(default)]
    sandbox: Option<RawSandbox>,
}

/// `[tool_process.sandbox]` block. Phase 52.
#[derive(Debug, Default, Deserialize)]
struct RawSandbox {
    wrapper: String,
    #[serde(default)]
    args: Option<Vec<String>>,
}

fn default_stdio_transport() -> String {
    "stdio".into()
}

/// One `[[schedule]]` entry in the TOML file. Phase 26 Task 2.
/// Phase 63 Task 2 added the optional `notify_target` field.
/// Phase 72 added `notify_targets` (plural) + `notify_when`.
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
    /// Phase 63 Task 2 — singular alias. Kept for backwards
    /// compatibility; loader bridges into `notify_targets`.
    /// Declaring both `notify_target` and `notify_targets` on
    /// one trigger is rejected at load time (Phase 72 Q1(a)).
    #[serde(default)]
    notify_target: Option<String>,
    /// Phase 72 — explicit list of notify target names for
    /// multi-target fan-out. Empty + a default-marked
    /// `[[notify_target]]` exists → loader resolves the default.
    #[serde(default)]
    notify_targets: Vec<String>,
    /// Phase 72 — conditional dispatch gate. Default `"always"`.
    #[serde(default)]
    notify_when: Option<String>,
}

fn default_reflection_lookback_secs() -> u64 {
    86400 // 24 hours
}

/// Minimum and maximum lookback bounds enforced at config-load
/// time. Below 60s the reflection cadence becomes self-noisy;
/// above 30 days the outcome-summary list becomes unwieldy.
const MIN_REFLECTION_LOOKBACK_SECS: u64 = 60;
const MAX_REFLECTION_LOOKBACK_SECS: u64 = 30 * 86400;

/// One `[[reflection_schedule]]` entry in the TOML file.
/// Phase 70 Task 2 — P14 self-learning closure.
#[derive(Debug, Default, Deserialize)]
struct RawReflectionSchedule {
    name: String,
    cron: String,
    #[serde(default = "default_reflection_lookback_secs")]
    lookback_window_secs: u64,
    #[serde(default)]
    role_override: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
}

/// One `[[memory.retention]]` entry in the TOML file. Phase 74.
/// Operators declare:
///
/// ```toml
/// [[memory.retention]]
/// topic_glob = "project/*"
/// retention = "forever"
///
/// [[memory.retention]]
/// topic_glob = "notes/*"
/// retention_days = 30
/// ```
///
/// Exactly one of `retention` (literal `"forever"`) or
/// `retention_days` (numeric) must be set per block. The
/// loader rejects partial / mutually-exclusive config.
#[derive(Debug, Default, Deserialize)]
struct RawMemoryRetention {
    topic_glob: String,
    /// String discriminator. Today only `"forever"` is
    /// recognized; future variants land here.
    #[serde(default)]
    retention: Option<String>,
    /// Numeric retention period in days. Mutually exclusive
    /// with `retention`.
    #[serde(default)]
    retention_days: Option<u64>,
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
    /// Phase 63 Task 2 — see `RawSchedule::notify_target`.
    #[serde(default)]
    notify_target: Option<String>,
    /// Phase 72 — see `RawSchedule::notify_targets`.
    #[serde(default)]
    notify_targets: Vec<String>,
    /// Phase 72 — see `RawSchedule::notify_when`.
    #[serde(default)]
    notify_when: Option<String>,
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
    /// Phase 63 Task 2 — see `RawSchedule::notify_target`.
    #[serde(default)]
    notify_target: Option<String>,
    /// Phase 72 — see `RawSchedule::notify_targets`.
    #[serde(default)]
    notify_targets: Vec<String>,
    /// Phase 72 — see `RawSchedule::notify_when`.
    #[serde(default)]
    notify_when: Option<String>,
}

/// One `[[notify_target]]` entry in the TOML file. Phase 62 Task 3.
///
/// The shape is intentionally flat (all kind-specific fields are
/// optional at the deserialize layer) so that an operator can
/// declare any `[[notify_target]]` block and get a precise
/// load-time error if required fields are missing for the chosen
/// `kind`. The loader (in `AivyxConfig::from_sources_with_paths`)
/// validates the kind/field correspondence and emits
/// [`ConfigError::Invalid`] with a field name that points to the
/// offending entry.
#[derive(Debug, Default, Deserialize)]
struct RawNotifyTarget {
    name: String,
    /// Lowercase string discriminator. Accepted values:
    /// `"telegram"`, `"webhook"`, `"email"` (Phase 68).
    /// Anything else is rejected at load time.
    kind: String,
    /// Required when `kind = "telegram"`. The operator-owned
    /// Telegram chat the bot is authorized to message.
    #[serde(default)]
    chat_id: Option<String>,
    /// Required when `kind = "webhook"`. The endpoint to POST to.
    #[serde(default)]
    url: Option<String>,
    /// Phase 68 — required when `kind = "email"`. The recipient
    /// address; the shared SMTP credentials live in `[email]`.
    #[serde(default)]
    to: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
    /// Phase 72 — when `true`, this target is the global
    /// default triggers fall through to when they omit
    /// `notify_targets`. At most one notify_target may set
    /// this; loader rejects multiple defaults.
    #[serde(default)]
    default: bool,
    /// Phase 73 — see [`NotifyTargetConfig::retry_count`].
    /// `#[serde(default)]` returns 0 (no retry).
    #[serde(default)]
    retry_count: u32,
    /// Phase 73 — see
    /// [`NotifyTargetConfig::retry_backoff_ms_start`]. The
    /// `Option` distinguishes "not set" (use the default
    /// 500 ms) from "explicit value" so the loader's lower-
    /// bound check (≥ 100) only applies when the operator
    /// declared the field.
    #[serde(default)]
    retry_backoff_ms_start: Option<u64>,
    /// Phase 73 — see [`NotifyTargetConfig::rate_limit_max`].
    #[serde(default)]
    rate_limit_max: Option<u32>,
    /// Phase 73 — see
    /// [`NotifyTargetConfig::rate_limit_window_secs`].
    #[serde(default)]
    rate_limit_window_secs: Option<u64>,
}

/// Hard ceiling for `retry_count`. Beyond this we treat the
/// config as a footgun ("retry 100 times" means a single
/// transient outage produces a multi-minute hang per fire).
/// Phase 73 Task 2.
pub const MAX_RETRY_COUNT: u32 = 10;

/// Lower bound for `retry_backoff_ms_start`. Below this the
/// retry loop starts hammering the backend before it can
/// recover from the original failure. 100ms is enough that
/// tests with mocked backoffs run quickly while real
/// deployments don't pound the backend.
pub const MIN_RETRY_BACKOFF_MS_START: u64 = 100;

/// Default starting backoff when the operator declares
/// `retry_count > 0` without setting an explicit start.
pub const DEFAULT_RETRY_BACKOFF_MS_START: u64 = 500;

fn default_role_name() -> String {
    DEFAULT_ROLE_NAME.to_string()
}

/// Phase 72 — reconcile a trigger's singular `notify_target` +
/// plural `notify_targets` + string `notify_when` raw fields
/// into the public `(Vec<String>, NotifyWhen)` shape.
///
/// Rules:
/// - Singular + plural set on the same trigger → error.
/// - Singular only → singular-as-one-element vec.
/// - Plural only → vec passes through (empty allowed; the
///   loader's later default-resolution pass may fill it in).
/// - Neither set → empty vec.
/// - `notify_when` parses lowercase `"always" | "on_failed" |
///   "on_completed_non_empty"`; anything else → error.
fn resolve_trigger_notify_fields(
    trigger_kind: &str,
    trigger_name: &str,
    singular: Option<String>,
    plural: Vec<String>,
    raw_when: Option<&str>,
) -> Result<(Vec<String>, NotifyWhen), ConfigError> {
    let targets = match (singular, plural.is_empty()) {
        (Some(_), false) => {
            return Err(ConfigError::Invalid {
                field: "trigger.notify_targets",
                reason: format!(
                    "{trigger_kind} `{trigger_name}` declares both \
                     `notify_target` (singular) and `notify_targets` \
                     (plural) — pick one. The singular form is kept \
                     for backwards compatibility; new configs should \
                     use `notify_targets`.",
                ),
            });
        }
        (Some(name), true) => vec![name],
        (None, _) => plural,
    };
    let when = match raw_when {
        None => NotifyWhen::Always,
        Some(s) => match s {
            "always" => NotifyWhen::Always,
            "on_failed" => NotifyWhen::OnFailed,
            "on_completed_non_empty" => NotifyWhen::OnCompletedNonEmpty,
            other => {
                return Err(ConfigError::Invalid {
                    field: "trigger.notify_when",
                    reason: format!(
                        "{trigger_kind} `{trigger_name}` notify_when = \
                         `{other}` is not recognized. Supported: \
                         `always` (default), `on_failed`, \
                         `on_completed_non_empty`."
                    ),
                });
            }
        },
    };
    Ok((targets, when))
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
    /// Phase 42 — optional TTL for memory entries, in seconds.
    #[serde(default)]
    ttl_secs: Option<u64>,
    /// Phase 74 — `[[memory.retention]]` table-array. Each entry
    /// is a per-topic-glob retention rule (forever or N days).
    /// The loader compiles + validates each pattern and builds
    /// the `memory_retention: Vec<MemoryRetentionRule>` on the
    /// public type.
    #[serde(default)]
    retention: Vec<RawMemoryRetention>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTelegram {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    chat_id: Option<i64>,
}

/// Phase 68 — `[email]` section deserialize target.
///
/// All fields are optional at the TOML layer; the loader
/// validates required-when-present semantics and applies the
/// tls_mode → port default. `tls_mode` accepts the lowercase
/// string variants `"starttls"`, `"implicit"`, `"none"`.
#[derive(Debug, Default, Deserialize)]
struct RawEmail {
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    tls_mode: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    from: Option<String>,
}

/// Phase 75 — `[embedding]` section deserialize target. All
/// fields optional; an absent section deserializes via
/// `Default` into the all-`None` shape, which the loader maps
/// to `embedding: None` (semantic search disabled). When any
/// field is set the loader fills omitted fields from the
/// `DEFAULT_EMBEDDING_*` constants and validates the result.
#[derive(Debug, Default, Deserialize)]
struct RawEmbedding {
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    dimensions: Option<usize>,
    #[serde(default)]
    rag_top_k: Option<usize>,
    #[serde(default)]
    rag_min_similarity: Option<f32>,
    #[serde(default)]
    recall_window_turns: Option<usize>,
}

/// Phase 80 — `[proactive]` deserialize target. Absent section
/// → all-`None` via `Default` → the loader maps to
/// `proactive: None` (off). Signal toggles are `Option<bool>`
/// so an omitted key means "default on," set means explicit.
#[derive(Debug, Default, Deserialize)]
struct RawProactive {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    max_per_window: Option<u32>,
    #[serde(default)]
    window_secs: Option<u64>,
    #[serde(default)]
    signal_ttl_expiry: Option<bool>,
    #[serde(default)]
    signal_recall_cluster: Option<bool>,
    #[serde(default)]
    signal_due_reminder: Option<bool>,
}

/// Phase 81 — `[persona_lifecycle]` deserialize target. Absent
/// section → all-`None` via `Default` → the loader maps to
/// `persona_lifecycle: None` (off). Signal toggles are
/// `Option<bool>` so an omitted key means "default on,"
/// a set key means explicit.
#[derive(Debug, Default, Deserialize)]
struct RawPersonaLifecycle {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    consolidation_similarity: Option<f32>,
    #[serde(default)]
    decay_max_age_secs: Option<u64>,
    #[serde(default)]
    min_soft_facets: Option<u32>,
    #[serde(default)]
    decay_unhelpful_threshold: Option<f32>,
    #[serde(default)]
    decay_min_samples: Option<u32>,
    #[serde(default)]
    decay_pair_below_affinity: Option<f32>,
    #[serde(default)]
    signal_consolidate: Option<bool>,
    #[serde(default)]
    signal_decay: Option<bool>,
}

/// Phase 84 — `[recall_cluster]` deserialize target. Absent
/// section → all-`None` via `Default` → the loader maps to
/// `recall_cluster: None` (off; recall unchanged).
#[derive(Debug, Default, Deserialize)]
struct RawRecallCluster {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    max_siblings: Option<u32>,
    #[serde(default)]
    min_affinity: Option<f32>,
}

/// Phase 87 — `[persona_consolidation]` deserialize target.
/// Absent section → all-`None` via `Default` → the loader
/// maps to `persona_consolidation: None` (off; Persona
/// proposal pipeline unchanged).
#[derive(Debug, Default, Deserialize)]
struct RawPersonaConsolidation {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    min_affinity: Option<f32>,
    #[serde(default)]
    min_samples: Option<u32>,
    #[serde(default)]
    min_topic_helpfulness: Option<f32>,
    #[serde(default)]
    max_proposals_per_cycle: Option<u32>,
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
    /// Storage key for the embedding-backend API key (Phase 75).
    /// Value: UTF-8 string.
    pub const EMBEDDING_API_KEY: &[u8] = b"embedding_api_key";
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
const ENV_MEMORY_TTL_SECS: &str = "AIVYX_MEMORY_TTL_SECS";
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
/// Phase 75 — env override for the embedding-backend API key.
/// Highest priority in the env > TOML > encrypted-store
/// fall-through, matching the anthropic / openai key pattern.
const ENV_EMBEDDING_API_KEY: &str = "AIVYX_EMBEDDING_API_KEY";

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
                    "ollama" => ProviderKind::Ollama,
                    other => {
                        return Err(ConfigError::Invalid {
                            field: "provider",
                            reason: format!(
                                "{ENV_PROVIDER}={other:?} is not valid. \
                                 Supported: anthropic, openai, ollama"
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

        // --- memory_ttl_secs ---------------------------------------
        // Phase 42 — optional TTL for memory entries.
        let memory_ttl_secs = match env_string(ENV_MEMORY_TTL_SECS) {
            Some(s) => {
                let parsed = s.parse::<u64>().map_err(|e| ConfigError::Invalid {
                    field: "memory_ttl_secs",
                    reason: format!(
                        "{ENV_MEMORY_TTL_SECS}={s:?} is not a valid u64: {e}"
                    ),
                })?;
                Some(Sourced::new(parsed, FieldSource::Env))
            }
            None => toml.memory.ttl_secs.map(|n| Sourced::new(n, FieldSource::Toml)),
        };

        // --- memory.retention (Phase 74) ---------------------------
        // Each `[[memory.retention]]` block declares a topic-glob
        // pattern + a retention policy. The loader compiles each
        // glob, validates the policy discriminant (exactly one of
        // `retention = "forever"` or `retention_days = N`), and
        // builds the runtime `MemoryRetentionRule` vec. First-
        // match wins at GC time so operators put narrower globs
        // first.
        let mut memory_retention: Vec<MemoryRetentionRule> = Vec::new();
        for raw in toml.memory.retention {
            if raw.topic_glob.trim().is_empty() {
                return Err(ConfigError::Invalid {
                    field: "memory.retention.topic_glob",
                    reason: "memory.retention.topic_glob must be a non-empty \
                             glob pattern (e.g. \"project/*\" or \"notes/**\")"
                        .into(),
                });
            }
            let matcher = globset::Glob::new(&raw.topic_glob)
                .map_err(|e| ConfigError::Invalid {
                    field: "memory.retention.topic_glob",
                    reason: format!(
                        "memory.retention.topic_glob `{}` is not a valid \
                         glob pattern: {e}",
                        raw.topic_glob
                    ),
                })?
                .compile_matcher();
            let policy = match (
                raw.retention.as_deref(),
                raw.retention_days,
            ) {
                (Some("forever"), None) => RetentionPolicy::Forever,
                (None, Some(0)) => {
                    return Err(ConfigError::Invalid {
                        field: "memory.retention.retention_days",
                        reason: format!(
                            "memory.retention.retention_days = 0 is \
                             meaningless (entries would expire immediately). \
                             topic_glob = `{}`",
                            raw.topic_glob
                        ),
                    });
                }
                (None, Some(days)) => RetentionPolicy::ForDays(days),
                (Some(other), None) => {
                    return Err(ConfigError::Invalid {
                        field: "memory.retention.retention",
                        reason: format!(
                            "memory.retention.retention = `{other}` is not \
                             recognized. Supported: \"forever\". (For a \
                             numeric period use `retention_days = N` \
                             instead.) topic_glob = `{}`",
                            raw.topic_glob
                        ),
                    });
                }
                (Some(_), Some(_)) => {
                    return Err(ConfigError::Invalid {
                        field: "memory.retention",
                        reason: format!(
                            "memory.retention declares both `retention` and \
                             `retention_days` — pick one. topic_glob = `{}`",
                            raw.topic_glob
                        ),
                    });
                }
                (None, None) => {
                    return Err(ConfigError::Invalid {
                        field: "memory.retention",
                        reason: format!(
                            "memory.retention must declare either \
                             `retention = \"forever\"` or \
                             `retention_days = N`. topic_glob = `{}`",
                            raw.topic_glob
                        ),
                    });
                }
            };
            memory_retention.push(MemoryRetentionRule {
                topic_glob: raw.topic_glob,
                matcher,
                retention: policy,
            });
        }

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

        // --- Phase 68: email SMTP config ------------------------
        // The `[email]` section is opt-in. Present-but-incomplete
        // (e.g. host without password) is rejected; absent is fine.
        // Validation: TLS mode must be one of the three labels;
        // tls_mode=none combined with PLAIN/LOGIN auth is rejected
        // (Q4 sign-off — we always use auth, so cleartext over the
        // wire is a load-time error).
        let email = build_email_config(&toml.email)?;

        // Phase 75 — `[embedding]` section. Absent → None
        // (semantic search disabled). When present, the env
        // var beats the TOML key; a still-`None` key is filled
        // from the encrypted store in phase 2 of the load.
        let embedding = build_embedding_config(&toml.embedding)?;
        let proactive = build_proactive_config(&toml.proactive)?;
        let persona_lifecycle = build_persona_lifecycle_config(
            &toml.persona_lifecycle,
        )?;
        let recall_cluster = build_recall_cluster_config(
            &toml.recall_cluster,
        )?;
        let persona_consolidation =
            build_persona_consolidation_config(
                &toml.persona_consolidation,
            )?;

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
        let mut mcp_servers: Vec<McpServerConfig> = Vec::new();
        for r in toml.mcp_servers.unwrap_or_default() {
            if !r.enabled {
                continue;
            }
            let transport = match r.transport.as_str() {
                "stdio" => McpTransportKind::Stdio,
                "sse" => McpTransportKind::Sse,
                other => {
                    return Err(ConfigError::Invalid {
                        field: "mcp_server.transport",
                        reason: format!(
                            "server {:?}: unknown transport {:?} \
                             (expected \"stdio\" or \"sse\")",
                            r.name, other,
                        ),
                    });
                }
            };
            // Validate required fields per transport kind.
            if transport == McpTransportKind::Stdio && r.command.is_none() {
                return Err(ConfigError::Invalid {
                    field: "mcp_server.command",
                    reason: format!(
                        "server {:?}: stdio transport requires `command`",
                        r.name,
                    ),
                });
            }
            if transport == McpTransportKind::Sse && r.url.is_none() {
                return Err(ConfigError::Invalid {
                    field: "mcp_server.url",
                    reason: format!(
                        "server {:?}: sse transport requires `url`",
                        r.name,
                    ),
                });
            }
            // Phase 55 — sandbox is stdio-only; reject if declared
            // on an SSE entry, same shape as the empty-wrapper
            // validation in `[[tool_process]]`.
            let sandbox = match r.sandbox {
                Some(s) => {
                    if transport == McpTransportKind::Sse {
                        return Err(ConfigError::Invalid {
                            field: "mcp_server.sandbox",
                            reason: format!(
                                "server {:?}: sandbox is stdio-only \
                                 (no local child to wrap on SSE transport)",
                                r.name,
                            ),
                        });
                    }
                    if s.wrapper.trim().is_empty() {
                        return Err(ConfigError::Invalid {
                            field: "mcp_server.sandbox.wrapper",
                            reason: format!(
                                "server {:?}: `sandbox.wrapper` must be \
                                 non-empty",
                                r.name,
                            ),
                        });
                    }
                    Some(SandboxConfig {
                        wrapper: s.wrapper,
                        args: s.args.unwrap_or_default(),
                    })
                }
                None => None,
            };
            mcp_servers.push(McpServerConfig {
                name: r.name,
                transport,
                command: r.command,
                args: r.args.unwrap_or_default(),
                url: r.url,
                enabled: true,
                bundled: r.bundled,
                sandbox,
            });
        }

        // --- tool processes ---------------------------------------
        // Phase 49 — PRODUCT.md P12. One entry per `[[tool_process]]`
        // table-array. Disabled entries are filtered out at load
        // time (same pattern as schedules / mcp_servers).
        let mut tool_processes: Vec<ToolProcessConfig> = Vec::new();
        for r in toml.tool_processes.unwrap_or_default() {
            if !r.enabled {
                continue;
            }
            if r.command.trim().is_empty() {
                return Err(ConfigError::Invalid {
                    field: "tool_process.command",
                    reason: format!(
                        "tool process {:?}: `command` must be non-empty",
                        r.name,
                    ),
                });
            }
            // Phase 52 — validate and translate the optional sandbox
            // wrapper. Empty `wrapper` is rejected with the same
            // posture as empty `command`.
            let sandbox = match r.sandbox {
                Some(s) => {
                    if s.wrapper.trim().is_empty() {
                        return Err(ConfigError::Invalid {
                            field: "tool_process.sandbox.wrapper",
                            reason: format!(
                                "tool process {:?}: `sandbox.wrapper` must be non-empty",
                                r.name,
                            ),
                        });
                    }
                    Some(SandboxConfig {
                        wrapper: s.wrapper,
                        args: s.args.unwrap_or_default(),
                    })
                }
                None => None,
            };
            let env: Vec<(String, String)> = r
                .env
                .unwrap_or_default()
                .into_iter()
                .collect();
            let scope_overrides = r.scope_overrides.unwrap_or_default();
            tool_processes.push(ToolProcessConfig {
                name: r.name,
                command: r.command,
                args: r.args.unwrap_or_default(),
                env,
                scope_overrides,
                enabled: true,
                sandbox,
            });
        }

        // --- schedules ---------------------------------------------
        let mut schedules: Vec<ScheduleConfig> = Vec::new();
        for r in toml.schedules.unwrap_or_default() {
            if !r.enabled {
                continue;
            }
            let (notify_targets, notify_when) =
                resolve_trigger_notify_fields(
                    "schedule",
                    &r.name,
                    r.notify_target.clone(),
                    r.notify_targets.clone(),
                    r.notify_when.as_deref(),
                )?;
            schedules.push(ScheduleConfig {
                name: r.name,
                cron: r.cron,
                role: r.role,
                prompt: r.prompt,
                enabled: true,
                wrap_mission: r.wrap_mission,
                notify_target: r.notify_target,
                notify_targets,
                notify_when,
            });
        }

        // --- webhooks ----------------------------------------------
        let mut webhooks: Vec<WebhookConfig> = Vec::new();
        for r in toml.webhooks.unwrap_or_default() {
            if !r.enabled {
                continue;
            }
            let (notify_targets, notify_when) =
                resolve_trigger_notify_fields(
                    "webhook",
                    &r.name,
                    r.notify_target.clone(),
                    r.notify_targets.clone(),
                    r.notify_when.as_deref(),
                )?;
            webhooks.push(WebhookConfig {
                name: r.name,
                role: r.role,
                prompt: r.prompt,
                enabled: true,
                wrap_mission: r.wrap_mission,
                notify_target: r.notify_target,
                notify_targets,
                notify_when,
            });
        }

        // --- file watches ------------------------------------------
        let mut file_watches: Vec<FileWatchConfig> = Vec::new();
        for r in toml.file_watches.unwrap_or_default() {
            if !r.enabled {
                continue;
            }
            let (notify_targets, notify_when) =
                resolve_trigger_notify_fields(
                    "file_watch",
                    &r.name,
                    r.notify_target.clone(),
                    r.notify_targets.clone(),
                    r.notify_when.as_deref(),
                )?;
            file_watches.push(FileWatchConfig {
                name: r.name,
                path: r.path,
                role: r.role,
                prompt: r.prompt,
                enabled: true,
                debounce_ms: r.debounce_ms,
                wrap_mission: r.wrap_mission,
                notify_target: r.notify_target,
                notify_targets,
                notify_when,
            });
        }

        // --- notify targets (Phase 62 Task 3) ----------------------
        // Walk every [[notify_target]] entry. For each:
        //   1. Validate `kind` is a recognized discriminator.
        //   2. Validate kind-required fields are present.
        //   3. Build the typed `NotifyTargetKind`.
        // After the per-entry walk, validate name uniqueness across
        // the surviving set (disabled entries don't count — they
        // were never going to dispatch anyway).
        let mut notify_targets: Vec<NotifyTargetConfig> = Vec::new();
        for raw in toml.notify_targets.unwrap_or_default() {
            if !raw.enabled {
                continue;
            }
            if raw.name.trim().is_empty() {
                return Err(ConfigError::Invalid {
                    field: "notify_target.name",
                    reason: "name must be non-empty".into(),
                });
            }
            let kind = match raw.kind.as_str() {
                "telegram" => {
                    let chat_id = raw.chat_id.ok_or_else(|| ConfigError::Invalid {
                        field: "notify_target.chat_id",
                        reason: format!(
                            "kind = \"telegram\" requires `chat_id` \
                             (target `{}`)",
                            raw.name
                        ),
                    })?;
                    if chat_id.trim().is_empty() {
                        return Err(ConfigError::Invalid {
                            field: "notify_target.chat_id",
                            reason: format!(
                                "`chat_id` must be non-empty \
                                 (target `{}`)",
                                raw.name
                            ),
                        });
                    }
                    NotifyTargetKind::Telegram { chat_id }
                }
                "webhook" => {
                    let url = raw.url.ok_or_else(|| ConfigError::Invalid {
                        field: "notify_target.url",
                        reason: format!(
                            "kind = \"webhook\" requires `url` \
                             (target `{}`)",
                            raw.name
                        ),
                    })?;
                    if !url.starts_with("http://") && !url.starts_with("https://") {
                        return Err(ConfigError::Invalid {
                            field: "notify_target.url",
                            reason: format!(
                                "`url` must start with http:// or https:// \
                                 (target `{}`, got `{}`)",
                                raw.name, url
                            ),
                        });
                    }
                    NotifyTargetKind::Webhook { url }
                }
                "web-ui" => {
                    // Phase 69 — Web UI desktop notification.
                    // No per-target fields; one Web UI per
                    // daemon. Defensive: if the operator
                    // supplied `to`, `url`, or `chat_id`, that
                    // means they typed the wrong kind for the
                    // fields they were trying to use. We
                    // tolerate the unused fields silently
                    // because TOML doesn't strict-mode by
                    // default and the loader already accepts
                    // them as `Option`.
                    NotifyTargetKind::WebUi
                }
                "email" => {
                    // Phase 68 — email target. Requires `to` and
                    // the `[email]` section to be configured.
                    let to = raw.to.ok_or_else(|| ConfigError::Invalid {
                        field: "notify_target.to",
                        reason: format!(
                            "kind = \"email\" requires `to` \
                             (target `{}`)",
                            raw.name
                        ),
                    })?;
                    if !to.contains('@') {
                        return Err(ConfigError::Invalid {
                            field: "notify_target.to",
                            reason: format!(
                                "`to` must contain `@` (target `{}`, got `{}`)",
                                raw.name, to
                            ),
                        });
                    }
                    if email.is_none() {
                        return Err(ConfigError::Invalid {
                            field: "notify_target.to",
                            reason: format!(
                                "kind = \"email\" requires a top-level \
                                 [email] section with SMTP credentials \
                                 (target `{}`)",
                                raw.name
                            ),
                        });
                    }
                    NotifyTargetKind::Email { to }
                }
                other => {
                    return Err(ConfigError::Invalid {
                        field: "notify_target.kind",
                        reason: format!(
                            "unknown notify_target kind `{}` \
                             (target `{}`); supported: telegram, webhook, email, web-ui",
                            other, raw.name
                        ),
                    });
                }
            };
            // Reject duplicate names eagerly so the error names the
            // collision rather than letting the dispatcher pick one
            // silently at startup.
            if notify_targets.iter().any(|t| t.name == raw.name) {
                return Err(ConfigError::Invalid {
                    field: "notify_target.name",
                    reason: format!(
                        "duplicate notify_target name `{}` — names must \
                         be unique across all [[notify_target]] entries",
                        raw.name
                    ),
                });
            }
            // Phase 73 — validate retry + rate-limit fields.
            // retry_count is capped at MAX_RETRY_COUNT (10 by
            // default) so a misconfigured 100-retry policy
            // doesn't wedge a single fire for minutes.
            if raw.retry_count > MAX_RETRY_COUNT {
                return Err(ConfigError::Invalid {
                    field: "notify_target.retry_count",
                    reason: format!(
                        "notify_target `{}` retry_count = {} exceeds the \
                         hard cap of {MAX_RETRY_COUNT}. Lower retry_count \
                         or accept the failure quickly and surface it via \
                         the audit chain.",
                        raw.name, raw.retry_count,
                    ),
                });
            }
            // Backoff start has an explicit lower bound only
            // when the operator declared the field — defaults
            // (None → DEFAULT_RETRY_BACKOFF_MS_START) skip the
            // check.
            let retry_backoff_ms_start = match raw.retry_backoff_ms_start {
                Some(v) if v < MIN_RETRY_BACKOFF_MS_START => {
                    return Err(ConfigError::Invalid {
                        field: "notify_target.retry_backoff_ms_start",
                        reason: format!(
                            "notify_target `{}` retry_backoff_ms_start = \
                             {} ms is below the {MIN_RETRY_BACKOFF_MS_START} \
                             ms minimum. A short initial backoff hammers \
                             the failing backend before it can recover.",
                            raw.name, v,
                        ),
                    });
                }
                Some(v) => v,
                None => DEFAULT_RETRY_BACKOFF_MS_START,
            };
            // Rate limit: both fields must be set or both unset.
            match (raw.rate_limit_max, raw.rate_limit_window_secs) {
                (Some(_), None) => {
                    return Err(ConfigError::Invalid {
                        field: "notify_target.rate_limit_window_secs",
                        reason: format!(
                            "notify_target `{}` declares `rate_limit_max` \
                             without `rate_limit_window_secs`; both fields \
                             must be set together (or neither).",
                            raw.name,
                        ),
                    });
                }
                (None, Some(_)) => {
                    return Err(ConfigError::Invalid {
                        field: "notify_target.rate_limit_max",
                        reason: format!(
                            "notify_target `{}` declares `rate_limit_window_secs` \
                             without `rate_limit_max`; both fields must be \
                             set together (or neither).",
                            raw.name,
                        ),
                    });
                }
                (Some(0), _) => {
                    return Err(ConfigError::Invalid {
                        field: "notify_target.rate_limit_max",
                        reason: format!(
                            "notify_target `{}` rate_limit_max = 0 is \
                             meaningless (no dispatches would ever be \
                             allowed). Either remove the rate-limit fields \
                             or set max ≥ 1.",
                            raw.name,
                        ),
                    });
                }
                (_, Some(0)) => {
                    return Err(ConfigError::Invalid {
                        field: "notify_target.rate_limit_window_secs",
                        reason: format!(
                            "notify_target `{}` rate_limit_window_secs = 0 \
                             is meaningless. Set window_secs ≥ 1 or remove \
                             the rate-limit fields.",
                            raw.name,
                        ),
                    });
                }
                _ => {}
            }
            notify_targets.push(NotifyTargetConfig {
                name: raw.name,
                kind,
                enabled: true,
                is_default: raw.default,
                retry_count: raw.retry_count,
                retry_backoff_ms_start,
                rate_limit_max: raw.rate_limit_max,
                rate_limit_window_secs: raw.rate_limit_window_secs,
            });
        }

        // Phase 72 — at most one notify_target may set
        // `default = true`. Reject multiple defaults eagerly so
        // the loader error names the collision.
        {
            let defaults: Vec<&str> = notify_targets
                .iter()
                .filter(|t| t.is_default)
                .map(|t| t.name.as_str())
                .collect();
            if defaults.len() > 1 {
                return Err(ConfigError::Invalid {
                    field: "notify_target.default",
                    reason: format!(
                        "multiple notify_targets declare `default = true` \
                         ({}). At most one default is allowed.",
                        defaults.join(", "),
                    ),
                });
            }
        }

        // Phase 72 — resolve the default-target sugar into any
        // trigger that omitted `notify_targets`. Done at
        // config-load time so runtime dispatch never has to ask
        // "which target is default?" again.
        if let Some(default_name) = notify_targets
            .iter()
            .find(|t| t.is_default)
            .map(|t| t.name.clone())
        {
            for s in &mut schedules {
                if s.notify_targets.is_empty() {
                    s.notify_targets.push(default_name.clone());
                }
            }
            for w in &mut webhooks {
                if w.notify_targets.is_empty() {
                    w.notify_targets.push(default_name.clone());
                }
            }
            for f in &mut file_watches {
                if f.notify_targets.is_empty() {
                    f.notify_targets.push(default_name.clone());
                }
            }
        }

        // --- reflection_schedules (Phase 70 — P14 closure) --------
        // Each entry validates cron non-empty, lookback bounds,
        // name uniqueness (across reflection schedules AND
        // regular schedules to keep the operator mental model
        // single-namespace), and role_override existence when
        // declared.
        let mut reflection_schedules: Vec<ReflectionScheduleConfig> = Vec::new();
        for raw in toml.reflection_schedules.unwrap_or_default() {
            if !raw.enabled {
                continue;
            }
            if raw.name.trim().is_empty() {
                return Err(ConfigError::Invalid {
                    field: "reflection_schedule.name",
                    reason: "reflection_schedule.name must be non-empty".into(),
                });
            }
            if raw.cron.trim().is_empty() {
                return Err(ConfigError::Invalid {
                    field: "reflection_schedule.cron",
                    reason: format!(
                        "reflection_schedule `{}` has empty cron pattern",
                        raw.name
                    ),
                });
            }
            if raw.lookback_window_secs < MIN_REFLECTION_LOOKBACK_SECS
                || raw.lookback_window_secs > MAX_REFLECTION_LOOKBACK_SECS
            {
                return Err(ConfigError::Invalid {
                    field: "reflection_schedule.lookback_window_secs",
                    reason: format!(
                        "reflection_schedule `{}` lookback_window_secs = {} \
                         is outside the allowed range \
                         [{MIN_REFLECTION_LOOKBACK_SECS}, \
                         {MAX_REFLECTION_LOOKBACK_SECS}] (60s to 30 days)",
                        raw.name, raw.lookback_window_secs,
                    ),
                });
            }
            if reflection_schedules.iter().any(|s| s.name == raw.name) {
                return Err(ConfigError::Invalid {
                    field: "reflection_schedule.name",
                    reason: format!(
                        "duplicate reflection_schedule name `{}` — names must \
                         be unique across all [[reflection_schedule]] entries",
                        raw.name
                    ),
                });
            }
            if schedules.iter().any(|s| s.name == raw.name) {
                return Err(ConfigError::Invalid {
                    field: "reflection_schedule.name",
                    reason: format!(
                        "reflection_schedule `{}` collides with a [[schedule]] \
                         entry of the same name — names share a namespace",
                        raw.name
                    ),
                });
            }
            if let Some(ref role) = raw.role_override {
                if !roles.contains_key(role) {
                    return Err(ConfigError::Invalid {
                        field: "reflection_schedule.role_override",
                        reason: format!(
                            "reflection_schedule `{}` role_override = `{role}` \
                             references unknown role — declare a [[role]] \
                             with that name or remove role_override",
                            raw.name
                        ),
                    });
                }
            }
            reflection_schedules.push(ReflectionScheduleConfig {
                name: raw.name,
                cron: raw.cron,
                lookback_window_secs: raw.lookback_window_secs,
                role_override: raw.role_override,
                enabled: true,
            });
        }

        // --- Phase 63 Task 2: cross-validate trigger.notify_target
        //     against notify_targets + role envelopes. --------------
        //
        // Q5(a) at Phase 63 sign-off: validate at config-load time
        // rather than daemon startup. The operator gets the error
        // at `aivyx daemon run` startup, not at 9am the next
        // morning when the schedule fires silently. Mirrors the
        // load-time-not-runtime discipline of
        // `validate_role_inheritance` above.
        validate_trigger_notify_targets(
            &schedules,
            &webhooks,
            &file_watches,
            &notify_targets,
            &roles,
        )?;

        // --- profile (Phase 57, PRODUCT.md P13) -------------------
        // Map the raw `[profile]` section to a `Profile` struct.
        // Absent fields fall through to `Profile::default()` per
        // Q5(b) — `assistant_name` defaults to
        // `DEFAULT_ASSISTANT_NAME`, every other category defaults to
        // empty. No env-var override surface in Phase 57: Profile is
        // operator-declared via TOML only (Q1(a), Q6(a)).
        let profile = Profile {
            assistant_name: match toml.profile.assistant_name.clone() {
                Some(v) => Sourced::new(v, FieldSource::Toml),
                None => Sourced::new(
                    DEFAULT_ASSISTANT_NAME.to_string(),
                    FieldSource::Default,
                ),
            },
            operator_profile: toml.profile.operator_profile.clone(),
            communication_style: toml.profile.communication_style.clone(),
            primary_use_cases: toml
                .profile
                .primary_use_cases
                .clone()
                .unwrap_or_default(),
            behavioral_preferences: toml
                .profile
                .behavioral_preferences
                .clone()
                .unwrap_or_default(),
            behavioral_constraints: toml
                .profile
                .behavioral_constraints
                .clone()
                .unwrap_or_default(),
        };

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
            memory_ttl_secs,
            memory_retention,
            passphrase,
            telegram,
            email,
            embedding,
            proactive,
            persona_lifecycle,
            recall_cluster,
            persona_consolidation,
            roles,
            active_role,
            profile,
            warnings,
            mcp_servers,
            tool_processes,
            schedules,
            webhooks,
            file_watches,
            notify_targets,
            reflection_schedules,
            webhook_port: toml.daemon.webhook_port,
            web_ui_port: match (toml.daemon.web_ui, toml.daemon.web_ui_port) {
                // Explicit port always wins (and implicitly enables).
                (_, Some(port)) => Some(port),
                // `web_ui = true` without explicit port → default.
                (Some(true), None) => Some(7843),
                // Not configured or explicitly disabled.
                _ => None,
            },
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

        // Phase 75 — embedding API key store fall-through. Only
        // touched if the `[embedding]` section materialized an
        // `EmbeddingConfig`; we never fabricate one just because
        // the store holds a key (mirrors the telegram rule).
        if let Some(emb) = self.embedding.as_mut() {
            if emb.api_key.is_none() {
                if let Some(bytes) = secrets
                    .get(secret_keys::EMBEDDING_API_KEY)
                    .await
                    .map_err(|e| ConfigError::StoreRead {
                        field: "embedding.api_key",
                        reason: e.to_string(),
                    })?
                {
                    let s = String::from_utf8(bytes).map_err(|_| {
                        ConfigError::NonUtf8Secret {
                            field: "embedding.api_key",
                        }
                    })?;
                    emb.api_key = Some(SourcedSecret::new(
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
                ProviderKind::Ollama => {
                    // Ollama does not require an API key — it runs
                    // locally and ignores the Authorization header.
                    // The key is accepted if present (forwarded to
                    // the OpenAI provider) but never required.
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

/// Phase 63 Task 2 — cross-validate every trigger's
/// `notify_target` against the loaded `[[notify_target]]` set
/// and the trigger's role envelope.
///
/// Two failure modes per trigger with a non-`None`
/// `notify_target`:
///
/// 1. **Unknown target.** The string doesn't match any
///    configured `[[notify_target]] name = "..."`. Error
///    field: `<trigger-kind>.notify_target`.
///
/// 2. **Missing capability.** The trigger's role (the one its
///    turn runs under) doesn't declare `notify.send`
///    (qualified to the target, or unqualified) anywhere in
///    its parent chain — OR its `trust_ceiling` excludes
///    `notify.send` (it is `CEILING_TRUSTED` only at Phase
///    62 sign-off). Error field:
///    `<trigger-kind>.notify_target`.
///
/// Walks the role's parent chain accumulating declared scopes
/// into a `CapabilitySet`, then intersects with the role's
/// `trust_ceiling`. The intersection is the deceleration of
/// what `assemble_role_envelope` produces at runtime, modulo
/// the backcompat floor (which doesn't add `notify.send` and
/// so doesn't change the answer for this check). A role using
/// the empty `capability_scopes = []` sentinel does NOT get
/// `notify.send` from the floor — the floor predates the
/// scope.
///
/// `O(triggers * (targets + role_depth))`. Realistic configs
/// have a handful of each; cheap.
fn validate_trigger_notify_targets(
    schedules: &[ScheduleConfig],
    webhooks: &[WebhookConfig],
    file_watches: &[FileWatchConfig],
    notify_targets: &[NotifyTargetConfig],
    roles: &BTreeMap<String, Role>,
) -> Result<(), ConfigError> {
    use aivyx_capability::CapabilitySet;

    // Helper: walk the role chain accumulating declared scopes
    // into a CapabilitySet, intersect with trust ceiling, check
    // grant.
    let role_can_notify = |role_name: &str, target: &str| -> Result<bool, ConfigError> {
        let Some(start) = roles.get(role_name) else {
            // Caller (validate_role_inheritance / active-role
            // resolution) has already caught dangling role
            // names, but defend against being called before
            // those checks by erroring distinctly.
            return Err(ConfigError::Invalid {
                field: "trigger.role",
                reason: format!("trigger references unknown role `{role_name}`"),
            });
        };
        // Accumulate every ancestor's declared scopes.
        let mut accumulated: Vec<Scope> = Vec::new();
        let mut cursor: Option<&Role> = Some(start);
        let mut visited: std::collections::HashSet<&str> =
            std::collections::HashSet::new();
        while let Some(role) = cursor {
            let n = role.name.value.as_str();
            if !visited.insert(n) {
                // Cycle — already caught upstream; bail out
                // so we don't loop.
                break;
            }
            accumulated.extend(role.capability_scopes.value.iter().cloned());
            cursor = role
                .parent_role
                .value
                .as_deref()
                .and_then(|p| roles.get(p));
        }
        let declared = CapabilitySet::from_scopes(accumulated);
        let ceiling = start.trust_ceiling.value.default_ceiling();
        let effective = declared.intersect(ceiling);
        let needed = Scope::parse(&format!("notify.send:{target}")).ok_or_else(|| {
            ConfigError::Invalid {
                field: "notify_target.name",
                reason: format!(
                    "cannot construct capability scope for target `{target}`; \
                     target names must be bare identifiers compatible with \
                     Scope::parse"
                ),
            }
        })?;
        Ok(effective.grants(&needed))
    };

    let check =
        |role_name: &str, target: &str, kind: &str, name: &str, field: &'static str| -> Result<(), ConfigError> {
            if !notify_targets.iter().any(|t| t.name == target) {
                return Err(ConfigError::Invalid {
                    field,
                    reason: format!(
                        "{kind} `{name}` references unknown notify_target \
                         `{target}` — declare a matching [[notify_target]] \
                         entry or remove the field"
                    ),
                });
            }
            if !role_can_notify(role_name, target)? {
                return Err(ConfigError::Invalid {
                    field,
                    reason: format!(
                        "role `{role_name}` used by {kind} `{name}` lacks \
                         `notify.send` capability required for notify_target \
                         `{target}` — declare `notify.send` or \
                         `notify.send:{target}` in the role's \
                         `capability_scopes` (Trusted tier only)"
                    ),
                });
            }
            Ok(())
        };

    // Phase 72 — walk the full `notify_targets` vec on each
    // trigger. After load-time default-resolution this is the
    // authoritative target list; the singular `notify_target`
    // alias has already been bridged in. Per Q4(a) the dispatch
    // path fans out concurrently, so every named target must
    // pass both the existence + capability checks individually.
    for s in schedules {
        for target in &s.notify_targets {
            check(
                &s.role,
                target,
                "schedule",
                &s.name,
                "schedule.notify_targets",
            )?;
        }
    }
    for w in webhooks {
        for target in &w.notify_targets {
            check(
                &w.role,
                target,
                "webhook",
                &w.name,
                "webhook.notify_targets",
            )?;
        }
    }
    for f in file_watches {
        for target in &f.notify_targets {
            check(
                &f.role,
                target,
                "file_watch",
                &f.name,
                "file_watch.notify_targets",
            )?;
        }
    }
    Ok(())
}

/// Load and parse the TOML file at `path`, if any.
/// Phase 68 — build an [`EmailConfig`] from the parsed `[email]`
/// section. Returns `Ok(None)` if the section is absent (every
/// field unset); `Ok(Some(cfg))` on a complete + validated
/// declaration; `Err(ConfigError::Invalid)` on partial config or
/// tls_mode-vs-auth security mismatches.
///
/// Validations:
/// - If ANY email field is set, ALL required fields (`host`,
///   `username`, `password`, `from`) must be set.
/// - `tls_mode` must be one of `"starttls"`, `"implicit"`, `"none"`.
/// - `tls_mode = "none"` is rejected per Q4(a) — we always
///   send PLAIN/LOGIN credentials, which requires TLS.
/// - `from` must contain `@`.
/// - `port` defaults from tls_mode: 587 STARTTLS, 465 implicit.
fn build_email_config(raw: &RawEmail) -> Result<Option<EmailConfig>, ConfigError> {
    let any_set = raw.host.is_some()
        || raw.port.is_some()
        || raw.tls_mode.is_some()
        || raw.username.is_some()
        || raw.password.is_some()
        || raw.from.is_some();
    if !any_set {
        return Ok(None);
    }

    let host = raw.host.clone().ok_or(ConfigError::Invalid {
        field: "email.host",
        reason: "[email] section is present but `host` is missing".into(),
    })?;
    if host.trim().is_empty() {
        return Err(ConfigError::Invalid {
            field: "email.host",
            reason: "`host` must be non-empty".into(),
        });
    }

    let tls_mode = match raw.tls_mode.as_deref() {
        None | Some("starttls") => TlsMode::Starttls,
        Some("implicit") => TlsMode::Implicit,
        Some("none") => {
            return Err(ConfigError::Invalid {
                field: "email.tls_mode",
                reason: "tls_mode = \"none\" is rejected — Aivyx uses \
                         PLAIN/LOGIN auth which requires TLS to avoid \
                         sending credentials in cleartext"
                    .into(),
            });
        }
        Some(other) => {
            return Err(ConfigError::Invalid {
                field: "email.tls_mode",
                reason: format!(
                    "unknown tls_mode `{other}` — supported: \
                     \"starttls\" (default), \"implicit\""
                ),
            });
        }
    };

    let port = raw.port.unwrap_or(match tls_mode {
        TlsMode::Starttls => 587,
        TlsMode::Implicit => 465,
        TlsMode::None => 25,
    });

    let username_str = raw.username.clone().ok_or(ConfigError::Invalid {
        field: "email.username",
        reason: "[email] section requires `username`".into(),
    })?;
    if username_str.trim().is_empty() {
        return Err(ConfigError::Invalid {
            field: "email.username",
            reason: "`username` must be non-empty".into(),
        });
    }
    let username = SourcedSecret::new(
        secrecy::SecretString::from(username_str),
        FieldSource::Toml,
    );

    let password_str = raw.password.clone().ok_or(ConfigError::Invalid {
        field: "email.password",
        reason: "[email] section requires `password` (SMTP password / app password)".into(),
    })?;
    if password_str.trim().is_empty() {
        return Err(ConfigError::Invalid {
            field: "email.password",
            reason: "`password` must be non-empty".into(),
        });
    }
    let password = SourcedSecret::new(
        secrecy::SecretString::from(password_str),
        FieldSource::Toml,
    );

    let from = raw.from.clone().ok_or(ConfigError::Invalid {
        field: "email.from",
        reason: "[email] section requires `from`".into(),
    })?;
    if !from.contains('@') {
        return Err(ConfigError::Invalid {
            field: "email.from",
            reason: format!("`from` must contain `@` (got `{from}`)"),
        });
    }

    Ok(Some(EmailConfig {
        host,
        port,
        tls_mode,
        username,
        password,
        from,
    }))
}

/// Phase 75 — build the `[embedding]` config.
///
/// Absent section (every field `None`) → `Ok(None)`: semantic
/// search disabled, `memory.search` stays keyword-only. Any set
/// field opts in; omitted fields fall back to the
/// `DEFAULT_EMBEDDING_*` constants. The API key resolves
/// env > TOML here; a still-`None` key is filled from the
/// encrypted store in phase 2 ([`AivyxConfig::hydrate_secrets_from_store`]).
fn build_embedding_config(
    raw: &RawEmbedding,
) -> Result<Option<EmbeddingConfig>, ConfigError> {
    let any_set = raw.base_url.is_some()
        || raw.model.is_some()
        || raw.api_key.is_some()
        || raw.dimensions.is_some()
        || raw.rag_top_k.is_some()
        || raw.rag_min_similarity.is_some()
        || raw.recall_window_turns.is_some();
    if !any_set {
        return Ok(None);
    }

    let base_url = raw
        .base_url
        .clone()
        .unwrap_or_else(|| DEFAULT_EMBEDDING_BASE_URL.to_string());
    if base_url.trim().is_empty() {
        return Err(ConfigError::Invalid {
            field: "embedding.base_url",
            reason: "`base_url` must be non-empty".into(),
        });
    }

    let model = raw
        .model
        .clone()
        .unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_string());
    if model.trim().is_empty() {
        return Err(ConfigError::Invalid {
            field: "embedding.model",
            reason: "`model` must be non-empty".into(),
        });
    }

    let dimensions =
        raw.dimensions.unwrap_or(DEFAULT_EMBEDDING_DIMENSIONS);
    if dimensions == 0 {
        return Err(ConfigError::Invalid {
            field: "embedding.dimensions",
            reason: "`dimensions` must be >= 1".into(),
        });
    }

    let rag_top_k = raw.rag_top_k.unwrap_or(DEFAULT_RAG_TOP_K);
    if rag_top_k == 0 {
        return Err(ConfigError::Invalid {
            field: "embedding.rag_top_k",
            reason: "`rag_top_k` must be >= 1".into(),
        });
    }

    let rag_min_similarity = raw
        .rag_min_similarity
        .unwrap_or(DEFAULT_RAG_MIN_SIMILARITY);
    if !(0.0..=1.0).contains(&rag_min_similarity) {
        return Err(ConfigError::Invalid {
            field: "embedding.rag_min_similarity",
            reason: "`rag_min_similarity` must be in [0.0, 1.0]"
                .into(),
        });
    }

    let recall_window_turns = raw
        .recall_window_turns
        .unwrap_or(DEFAULT_RECALL_WINDOW_TURNS);
    if recall_window_turns == 0 {
        return Err(ConfigError::Invalid {
            field: "embedding.recall_window_turns",
            reason: "`recall_window_turns` must be >= 1 (1 = \
                     just the latest message, pre-Phase-86)"
                .into(),
        });
    }

    // env > TOML; encrypted-store fall-through happens in phase 2.
    let api_key = env_secret(ENV_EMBEDDING_API_KEY)
        .map(|s| SourcedSecret::new(s, FieldSource::Env))
        .or_else(|| {
            raw.api_key.as_ref().map(|s| {
                SourcedSecret::new(
                    SecretString::from(s.clone()),
                    FieldSource::Toml,
                )
            })
        });

    Ok(Some(EmbeddingConfig {
        base_url,
        model,
        api_key,
        dimensions,
        rag_top_k,
        rag_min_similarity,
        recall_window_turns,
    }))
}

/// Phase 80 — build the `[proactive]` config. Absent section
/// (every field `None`) → `Ok(None)` (proactive off, the
/// common case). Validation applies **only when `enabled`** —
/// a present-but-disabled section is allowed to be incomplete
/// so an operator can stage the config before arming it.
fn build_proactive_config(
    raw: &RawProactive,
) -> Result<Option<ProactiveConfig>, ConfigError> {
    let any_set = raw.enabled.is_some()
        || raw.target.is_some()
        || raw.max_per_window.is_some()
        || raw.window_secs.is_some()
        || raw.signal_ttl_expiry.is_some()
        || raw.signal_recall_cluster.is_some()
        || raw.signal_due_reminder.is_some();
    if !any_set {
        return Ok(None);
    }

    let enabled = raw.enabled.unwrap_or(false);
    let target = raw.target.clone().unwrap_or_default();
    let max_per_window = raw
        .max_per_window
        .unwrap_or(DEFAULT_PROACTIVE_MAX_PER_WINDOW);
    let window_secs =
        raw.window_secs.unwrap_or(DEFAULT_PROACTIVE_WINDOW_SECS);
    let signals = ProactiveSignals {
        ttl_expiry: raw.signal_ttl_expiry.unwrap_or(true),
        recall_cluster: raw.signal_recall_cluster.unwrap_or(true),
        due_reminder: raw.signal_due_reminder.unwrap_or(true),
    };

    // Only an *armed* config must be coherent — a staged
    // (enabled = false) section can be partial.
    if enabled {
        if target.trim().is_empty() {
            return Err(ConfigError::Invalid {
                field: "proactive.target",
                reason: "`target` is required when proactive is \
                         enabled (must name a [[notify_target]])"
                    .into(),
            });
        }
        if max_per_window == 0 {
            return Err(ConfigError::Invalid {
                field: "proactive.max_per_window",
                reason: "`max_per_window` must be >= 1".into(),
            });
        }
        if window_secs == 0 {
            return Err(ConfigError::Invalid {
                field: "proactive.window_secs",
                reason: "`window_secs` must be >= 1".into(),
            });
        }
        if !signals.ttl_expiry
            && !signals.recall_cluster
            && !signals.due_reminder
        {
            return Err(ConfigError::Invalid {
                field: "proactive.signals",
                reason: "at least one signal class must be enabled \
                         when proactive is enabled"
                    .into(),
            });
        }
    }

    Ok(Some(ProactiveConfig {
        enabled,
        target,
        max_per_window,
        window_secs,
        signals,
    }))
}

/// Phase 81 — build the `[persona_lifecycle]` config. Absent
/// section (every field `None`) → `Ok(None)` (lifecycle off,
/// the common case — the Persona only ever grows, pre-Phase-81
/// behavior). Validation applies **only when `enabled`** — a
/// present-but-disabled section may be incomplete so an
/// operator can stage it before arming.
fn build_persona_lifecycle_config(
    raw: &RawPersonaLifecycle,
) -> Result<Option<PersonaLifecycleConfig>, ConfigError> {
    let any_set = raw.enabled.is_some()
        || raw.consolidation_similarity.is_some()
        || raw.decay_max_age_secs.is_some()
        || raw.min_soft_facets.is_some()
        || raw.decay_unhelpful_threshold.is_some()
        || raw.decay_min_samples.is_some()
        || raw.decay_pair_below_affinity.is_some()
        || raw.signal_consolidate.is_some()
        || raw.signal_decay.is_some();
    if !any_set {
        return Ok(None);
    }

    let enabled = raw.enabled.unwrap_or(false);
    let consolidation_similarity = raw
        .consolidation_similarity
        .unwrap_or(DEFAULT_PL_CONSOLIDATION_SIMILARITY);
    let decay_max_age_secs = raw
        .decay_max_age_secs
        .unwrap_or(DEFAULT_PL_DECAY_MAX_AGE_SECS);
    let min_soft_facets =
        raw.min_soft_facets.unwrap_or(DEFAULT_PL_MIN_SOFT_FACETS);
    let decay_unhelpful_threshold = raw
        .decay_unhelpful_threshold
        .unwrap_or(DEFAULT_PL_DECAY_UNHELPFUL_THRESHOLD);
    let decay_min_samples = raw
        .decay_min_samples
        .unwrap_or(DEFAULT_PL_DECAY_MIN_SAMPLES);
    let decay_pair_below_affinity = raw
        .decay_pair_below_affinity
        .unwrap_or(DEFAULT_PL_DECAY_PAIR_BELOW_AFFINITY);
    let signals = PersonaLifecycleSignals {
        consolidate: raw.signal_consolidate.unwrap_or(true),
        decay: raw.signal_decay.unwrap_or(true),
    };

    // Only an *armed* config must be coherent — a staged
    // (enabled = false) section can be partial.
    if enabled {
        if !(consolidation_similarity > 0.0
            && consolidation_similarity <= 1.0)
        {
            return Err(ConfigError::Invalid {
                field: "persona_lifecycle.consolidation_similarity",
                reason: "`consolidation_similarity` must be in \
                         the range (0.0, 1.0]"
                    .into(),
            });
        }
        if decay_max_age_secs == 0 {
            return Err(ConfigError::Invalid {
                field: "persona_lifecycle.decay_max_age_secs",
                reason: "`decay_max_age_secs` must be >= 1".into(),
            });
        }
        if min_soft_facets == 0 {
            return Err(ConfigError::Invalid {
                field: "persona_lifecycle.min_soft_facets",
                reason: "`min_soft_facets` must be >= 1".into(),
            });
        }
        if !signals.consolidate && !signals.decay {
            return Err(ConfigError::Invalid {
                field: "persona_lifecycle.signals",
                reason: "at least one signal class must be \
                         enabled when persona_lifecycle is \
                         enabled"
                    .into(),
            });
        }
        // Phase 85 — the helpfulness-decay knobs are only
        // consulted by the decay signal, so validate them
        // only when decay is actually armed.
        if signals.decay {
            if decay_unhelpful_threshold >= 0.0 {
                return Err(ConfigError::Invalid {
                    field:
                        "persona_lifecycle.decay_unhelpful_threshold",
                    reason: "`decay_unhelpful_threshold` must \
                             be < 0.0 (it is a net-negative \
                             helpfulness floor)"
                        .into(),
                });
            }
            if decay_min_samples == 0 {
                return Err(ConfigError::Invalid {
                    field:
                        "persona_lifecycle.decay_min_samples",
                    reason: "`decay_min_samples` must be >= 1"
                        .into(),
                });
            }
            // Phase 88 — pair-affinity floor must be a sane
            // non-negative number. Zero is permitted (means
            // "never fire pair-driven decay / never protect"),
            // mirroring Phase 87's posture that a knob's
            // floor-of-zero is a valid no-op tuning.
            if !decay_pair_below_affinity.is_finite()
                || decay_pair_below_affinity < 0.0
            {
                return Err(ConfigError::Invalid {
                    field:
                        "persona_lifecycle.decay_pair_below_affinity",
                    reason:
                        "`decay_pair_below_affinity` must be \
                         a finite non-negative number"
                            .into(),
                });
            }
        }
    }

    Ok(Some(PersonaLifecycleConfig {
        enabled,
        consolidation_similarity,
        decay_max_age_secs,
        min_soft_facets,
        decay_unhelpful_threshold,
        decay_min_samples,
        decay_pair_below_affinity,
        signals,
    }))
}

/// Phase 84 — build the `[recall_cluster]` config. Absent
/// section (every field `None`) → `Ok(None)` (cluster
/// expansion off, the common case — recall is unchanged,
/// pre-Phase-84 behaviour). Validation applies **only when
/// `enabled`** — a present-but-disabled section may be
/// incomplete so an operator can stage it before arming.
fn build_recall_cluster_config(
    raw: &RawRecallCluster,
) -> Result<Option<RecallClusterConfig>, ConfigError> {
    let any_set = raw.enabled.is_some()
        || raw.max_siblings.is_some()
        || raw.min_affinity.is_some();
    if !any_set {
        return Ok(None);
    }

    let enabled = raw.enabled.unwrap_or(false);
    let max_siblings =
        raw.max_siblings.unwrap_or(DEFAULT_RC_MAX_SIBLINGS);
    let min_affinity =
        raw.min_affinity.unwrap_or(DEFAULT_RC_MIN_AFFINITY);

    // Only an *armed* config must be coherent — a staged
    // (enabled = false) section can be partial.
    if enabled {
        if max_siblings == 0 {
            return Err(ConfigError::Invalid {
                field: "recall_cluster.max_siblings",
                reason: "`max_siblings` must be >= 1".into(),
            });
        }
        if min_affinity <= 0.0 {
            return Err(ConfigError::Invalid {
                field: "recall_cluster.min_affinity",
                reason: "`min_affinity` must be > 0.0".into(),
            });
        }
    }

    Ok(Some(RecallClusterConfig {
        enabled,
        max_siblings,
        min_affinity,
    }))
}

/// Phase 87 — `[persona_consolidation]` → optional runtime
/// config. Absent section → `None`; partial section (any key
/// set) → fill defaults and, only if `enabled = true`, validate
/// the bounds (the Phase 80/81/84 staged-config pattern).
fn build_persona_consolidation_config(
    raw: &RawPersonaConsolidation,
) -> Result<Option<PersonaConsolidationConfig>, ConfigError> {
    let any_set = raw.enabled.is_some()
        || raw.min_affinity.is_some()
        || raw.min_samples.is_some()
        || raw.min_topic_helpfulness.is_some()
        || raw.max_proposals_per_cycle.is_some();
    if !any_set {
        return Ok(None);
    }

    let enabled = raw.enabled.unwrap_or(false);
    let min_affinity =
        raw.min_affinity.unwrap_or(DEFAULT_PC_MIN_AFFINITY);
    let min_samples =
        raw.min_samples.unwrap_or(DEFAULT_PC_MIN_SAMPLES);
    let min_topic_helpfulness = raw
        .min_topic_helpfulness
        .unwrap_or(DEFAULT_PC_MIN_TOPIC_HELPFULNESS);
    let max_proposals_per_cycle = raw
        .max_proposals_per_cycle
        .unwrap_or(DEFAULT_PC_MAX_PROPOSALS_PER_CYCLE);

    // Only an *armed* config must be coherent — a staged
    // (enabled = false) section can be partial.
    if enabled {
        if min_affinity <= 0.0 {
            return Err(ConfigError::Invalid {
                field: "persona_consolidation.min_affinity",
                reason: "`min_affinity` must be > 0.0".into(),
            });
        }
        if min_samples == 0 {
            return Err(ConfigError::Invalid {
                field: "persona_consolidation.min_samples",
                reason: "`min_samples` must be >= 1".into(),
            });
        }
        if !min_topic_helpfulness.is_finite() {
            return Err(ConfigError::Invalid {
                field:
                    "persona_consolidation.min_topic_helpfulness",
                reason: "`min_topic_helpfulness` must be finite"
                    .into(),
            });
        }
        if max_proposals_per_cycle == 0 {
            return Err(ConfigError::Invalid {
                field:
                    "persona_consolidation.max_proposals_per_cycle",
                reason:
                    "`max_proposals_per_cycle` must be >= 1"
                        .into(),
            });
        }
    }

    Ok(Some(PersonaConsolidationConfig {
        enabled,
        min_affinity,
        min_samples,
        min_topic_helpfulness,
        max_proposals_per_cycle,
    }))
}

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

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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use secrecy::SecretString;
use serde::Deserialize;

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
}

impl LoadOptions {
    /// Minimal options — no TOML, no required fields. Used by tests
    /// that want to exercise env-only precedence without touching disk.
    pub fn test_env_only() -> Self {
        Self {
            toml_path: None,
            require_api_key: false,
            require_telegram_token: false,
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
    /// Model id. Always populated — falls through to [`DEFAULT_MODEL`]
    /// if no source supplied one (tagged [`FieldSource::Default`]).
    pub model: Sourced<String>,
    /// System prompt. Always populated with the same default semantics
    /// as [`Self::model`].
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
}

#[derive(Debug, Default, Deserialize)]
struct RawAnthropic {
    #[serde(default)]
    api_key: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawAgent {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    system_prompt: Option<String>,
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

        Ok(Self {
            anthropic_api_key,
            model,
            system_prompt,
            fs_root,
            storage_path,
            memory_max_per_topic,
            passphrase,
            telegram,
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
        if opts.require_api_key && self.anthropic_api_key.is_none() {
            return Err(ConfigError::Missing {
                field: "anthropic_api_key",
            });
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

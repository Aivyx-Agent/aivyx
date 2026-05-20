//! Unit tests for [`crate::AivyxConfig`].
//!
//! Process-environment tests are the dangerous part: `std::env` is
//! process-wide, `cargo test` runs tests in parallel by default, and
//! two tests that both `set_var("FOO", ...)` will race. The
//! [`env_guard`] module below serializes every env-touching test
//! through one `Mutex` and restores the prior state on drop, so tests
//! are race-free but still express one-var-at-a-time mutations.
//!
//! Tests are structured around the three phases of
//! [`AivyxConfig::load_from_env_and_toml`] + `hydrate_secrets_from_store`
//! + `validate`:
//!
//! 1. **Env-only precedence.** Set env vars, no TOML, assert the
//!    resulting `Sourced<T>::source == FieldSource::Env`.
//! 2. **TOML-only precedence.** Clear env vars, write TOML, assert
//!    `source == FieldSource::Toml`.
//! 3. **Env-over-TOML.** Both sources hold the same field; assert the
//!    env value wins and `source == FieldSource::Env`.
//! 4. **Invalid parsing.** Set `AIVYX_MEMORY_MAX_PER_TOPIC=not-a-num`
//!    and `AIVYX_TELEGRAM_CHAT_ID=oops`; assert typed `Invalid`.
//! 5. **Missing required field.** Ask for `require_api_key = true`
//!    with no source supplying one; assert `Missing { field:
//!    "anthropic_api_key" }`.
//! 6. **Encrypted-store hydration.** Seed `KeyDomain::Secrets`
//!    manually, call `hydrate_secrets_from_store`, assert the api_key
//!    now reports `FieldSource::EncryptedStore`.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use secrecy::ExposeSecret;

use crate::{
    AivyxConfig, ConfigError, FieldSource, LoadOptions, McpTransportKind, NotifyTargetKind,
    NotifyWhen, ProviderKind, Role, TlsMode, ToolAllowlist, DEFAULT_ASSISTANT_NAME,
    DEFAULT_MEMORY_MAX_PER_TOPIC, DEFAULT_MODEL, DEFAULT_ROLE_NAME, DEFAULT_SYSTEM_PROMPT,
};

// ------------------------------------------------------------------
// env_guard — serialize env mutations across parallel tests
// ------------------------------------------------------------------

/// All env-touching tests hold this `Mutex` for their full duration so
/// `cargo test`'s parallel runner cannot cross-contaminate them. The
/// guard is RAII: drop = unlock.
fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

/// Snapshot + clear every env var this test crate might mutate, then
/// restore on drop. Prevents a flaky test from polluting the process
/// environment for any later test or for the rest of the cargo run.
///
/// `#[allow(unsafe_code)]` scoped to this impl is the whole reason
/// `lib.rs` uses `#![deny(unsafe_code)]` instead of `forbid`. See the
/// comment on that attribute.
struct EnvScope {
    saved: Vec<(&'static str, Option<String>)>,
    _guard: MutexGuard<'static, ()>,
}

impl EnvScope {
    fn new() -> Self {
        let guard = env_lock();
        let vars = [
            "ANTHROPIC_API_KEY",
            "AIVYX_MODEL",
            "AIVYX_SYSTEM_PROMPT",
            "AIVYX_FS_ROOT",
            "AIVYX_STORAGE_PATH",
            "XDG_DATA_HOME",
            "HOME",
            "AIVYX_MEMORY_MAX_PER_TOPIC",
            "AIVYX_PASSPHRASE",
            "AIVYX_TELEGRAM_TOKEN",
            "AIVYX_TELEGRAM_CHAT_ID",
            // Phase 11 Task 1: scope the role override env var into
            // the env-guard so role-tests don't leak state across
            // parallel cargo-test runs.
            "AIVYX_ROLE",
            "AIVYX_OPENAI_API_KEY",
            "AIVYX_OPENAI_BASE_URL",
            "AIVYX_PROVIDER",
        ];
        let saved: Vec<_> = vars
            .iter()
            .map(|&v| (v, std::env::var(v).ok()))
            .collect();
        // Start every test with a clean slate — no env vars set,
        // except HOME which we always keep pointed at a safe
        // directory so the path-resolving helpers don't trip a
        // spurious NoHome error.
        for (var, _) in &saved {
            // Safety: scoped env mutation under a process-wide mutex,
            // restored on drop. Test-only.
            #[allow(unsafe_code)]
            unsafe {
                std::env::remove_var(var);
            }
        }
        // HOME default: under $TMPDIR, which always exists and is a
        // directory. Tests that need a specific HOME override this.
        let tmp_home = std::env::temp_dir();
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("HOME", &tmp_home);
        }
        EnvScope {
            saved,
            _guard: guard,
        }
    }

    fn set(&self, var: &str, value: &str) {
        // Safety: scoped env mutation under a process-wide mutex.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var(var, value);
        }
    }

    fn clear(&self, var: &str) {
        // Safety: scoped env mutation under a process-wide mutex.
        #[allow(unsafe_code)]
        unsafe {
            std::env::remove_var(var);
        }
    }
}

impl Drop for EnvScope {
    fn drop(&mut self) {
        for (var, prior) in self.saved.drain(..) {
            match prior {
                Some(value) => {
                    // Safety: restoring prior state under the same mutex.
                    #[allow(unsafe_code)]
                    unsafe {
                        std::env::set_var(var, value);
                    }
                }
                None => {
                    // Safety: restoring prior state under the same mutex.
                    #[allow(unsafe_code)]
                    unsafe {
                        std::env::remove_var(var);
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------
// temp-dir RAII — matches aivyx-storage's `$TMPDIR` convention
// ------------------------------------------------------------------

/// Bare-bones unique dir under `$TMPDIR`, removed on drop. Same shape
/// as the helper in `crates/aivyx-storage/src/tests.rs` so a reader
/// grepping for temp-dir patterns finds one convention.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let unique = format!(
            "aivyx-config-test-{tag}-{}",
            uuid_like(),
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Cheap unique id — nanos since epoch + a counter. We deliberately
/// do not pull `uuid` into this test file; the existing convention
/// in `aivyx-storage`'s tests is the same.
fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let c = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{c}")
}

// ------------------------------------------------------------------
// Phase 1: env-only precedence
// ------------------------------------------------------------------

#[test]
fn env_only_populates_every_field_with_env_source() {
    let env = EnvScope::new();
    env.set("ANTHROPIC_API_KEY", "sk-test-env-key");
    env.set("AIVYX_MODEL", "claude-from-env");
    env.set("AIVYX_SYSTEM_PROMPT", "env prompt");
    env.set("AIVYX_FS_ROOT", "/tmp/env-fs-root");
    env.set("AIVYX_STORAGE_PATH", "/tmp/env-store.redb");
    env.set("AIVYX_MEMORY_MAX_PER_TOPIC", "1234");
    env.set("AIVYX_PASSPHRASE", "env-passphrase");
    env.set("AIVYX_TELEGRAM_TOKEN", "123:env-token");
    env.set("AIVYX_TELEGRAM_CHAT_ID", "42");

    let cfg = AivyxConfig::load_from_env_and_toml(&LoadOptions::test_env_only())
        .expect("load should succeed");

    let api_key = cfg.anthropic_api_key.as_ref().expect("api key set");
    assert_eq!(api_key.source, FieldSource::Env);
    assert_eq!(api_key.value.expose_secret(), "sk-test-env-key");

    assert_eq!(cfg.model.source, FieldSource::Env);
    assert_eq!(cfg.model.value, "claude-from-env");
    assert_eq!(cfg.system_prompt.source, FieldSource::Env);
    assert_eq!(cfg.system_prompt.value, "env prompt");
    assert_eq!(cfg.fs_root.source, FieldSource::Env);
    assert_eq!(cfg.fs_root.value, PathBuf::from("/tmp/env-fs-root"));
    assert_eq!(cfg.storage_path.source, FieldSource::Env);
    assert_eq!(cfg.memory_max_per_topic.source, FieldSource::Env);
    assert_eq!(cfg.memory_max_per_topic.value, 1234);

    let pw = cfg.passphrase.as_ref().expect("passphrase set");
    assert_eq!(pw.source, FieldSource::Env);
    assert_eq!(pw.value.expose_secret(), "env-passphrase");

    let tg = cfg.telegram.as_ref().expect("telegram set");
    let tok = tg.token.as_ref().expect("token set");
    assert_eq!(tok.source, FieldSource::Env);
    assert_eq!(tok.value.expose_secret(), "123:env-token");
    let chat = tg.chat_filter.as_ref().expect("chat set");
    assert_eq!(chat.source, FieldSource::Env);
    assert_eq!(chat.value, 42);

    // Phase 11 Task 1: with no `[[role]]` entries and no AIVYX_ROLE
    // override, the backwards-compat bridge must synthesize an
    // implicit `default` role whose system_prompt mirrors the
    // legacy env-sourced value. The test does not set AIVYX_ROLE, so
    // the active role falls through to DEFAULT_ROLE_NAME.
    assert_eq!(cfg.roles.len(), 1, "exactly one synthesized role");
    let default_role = cfg
        .roles
        .get(DEFAULT_ROLE_NAME)
        .expect("default role synthesized");
    // The synthesized role's prompt carries the original env source,
    // not `FieldSource::Default` — pre-Phase-11 provenance preserved.
    assert_eq!(default_role.system_prompt.source, FieldSource::Env);
    assert_eq!(default_role.system_prompt.value, "env prompt");
    assert_eq!(cfg.active_role.value, DEFAULT_ROLE_NAME);
    assert_eq!(cfg.active_role.source, FieldSource::Default);
    assert!(
        cfg.warnings.is_empty(),
        "no warning when config has no explicit roles: {:?}",
        cfg.warnings
    );

    drop(env);
}

#[test]
fn env_empty_string_is_treated_as_unset() {
    // Phase 8 behavior preserved: `export AIVYX_PASSPHRASE=` with no
    // value should not populate the passphrase field. Tests the
    // `env_string` helper's empty-is-unset rule.
    let env = EnvScope::new();
    env.set("AIVYX_PASSPHRASE", "");

    let cfg = AivyxConfig::load_from_env_and_toml(&LoadOptions::test_env_only())
        .expect("load should succeed");
    assert!(cfg.passphrase.is_none(), "empty env var treated as unset");

    drop(env);
}

// ------------------------------------------------------------------
// Phase 2: TOML-only precedence
// ------------------------------------------------------------------

#[test]
fn toml_only_populates_fields_with_toml_source() {
    let env = EnvScope::new();
    let tmp = TempDir::new("toml-only");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-from-toml"

[agent]
model = "claude-toml-model"
system_prompt = "toml prompt"

[fs]
root = "/tmp/toml-fs"

[storage]
path = "/tmp/toml-store.redb"

[memory]
max_per_topic = 777

[telegram]
token = "toml:123:abc"
chat_id = 99

[aivyx]
passphrase = "toml-passphrase"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path.clone()),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");

    assert_eq!(
        cfg.anthropic_api_key.as_ref().unwrap().source,
        FieldSource::Toml
    );
    assert_eq!(cfg.model.source, FieldSource::Toml);
    assert_eq!(cfg.model.value, "claude-toml-model");
    assert_eq!(cfg.system_prompt.source, FieldSource::Toml);
    assert_eq!(cfg.fs_root.source, FieldSource::Toml);
    assert_eq!(cfg.fs_root.value, PathBuf::from("/tmp/toml-fs"));
    assert_eq!(cfg.storage_path.source, FieldSource::Toml);
    assert_eq!(cfg.memory_max_per_topic.source, FieldSource::Toml);
    assert_eq!(cfg.memory_max_per_topic.value, 777);
    assert_eq!(cfg.passphrase.as_ref().unwrap().source, FieldSource::Toml);
    let tg = cfg.telegram.as_ref().unwrap();
    assert_eq!(tg.token.as_ref().unwrap().source, FieldSource::Toml);
    assert_eq!(tg.chat_filter.as_ref().unwrap().value, 99);

    drop(env);
}

// ------------------------------------------------------------------
// Phase 3: env-over-TOML precedence
// ------------------------------------------------------------------

#[test]
fn env_beats_toml_when_both_set() {
    let env = EnvScope::new();
    let tmp = TempDir::new("precedence");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[agent]
model = "toml-loses"
[telegram]
token = "toml-token"
chat_id = 1
"#,
    )
    .unwrap();

    env.set("AIVYX_MODEL", "env-wins");
    env.set("AIVYX_TELEGRAM_TOKEN", "env-token");
    env.set("AIVYX_TELEGRAM_CHAT_ID", "2");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");

    assert_eq!(cfg.model.source, FieldSource::Env);
    assert_eq!(cfg.model.value, "env-wins");
    let tg = cfg.telegram.as_ref().unwrap();
    assert_eq!(tg.token.as_ref().unwrap().source, FieldSource::Env);
    assert_eq!(
        tg.token.as_ref().unwrap().value.expose_secret(),
        "env-token"
    );
    assert_eq!(tg.chat_filter.as_ref().unwrap().source, FieldSource::Env);
    assert_eq!(tg.chat_filter.as_ref().unwrap().value, 2);

    drop(env);
}

// ------------------------------------------------------------------
// Phase 4: defaults
// ------------------------------------------------------------------

#[test]
fn defaults_win_when_no_source_supplies_value() {
    let env = EnvScope::new();
    // HOME is set by EnvScope to $TMPDIR, so fs_root and
    // storage_path's default branches both succeed.
    let cfg = AivyxConfig::load_from_env_and_toml(&LoadOptions::test_env_only())
        .expect("load should succeed with defaults");

    assert!(cfg.anthropic_api_key.is_none());
    assert_eq!(cfg.model.source, FieldSource::Default);
    assert_eq!(cfg.model.value, DEFAULT_MODEL);
    assert_eq!(cfg.system_prompt.source, FieldSource::Default);
    assert_eq!(cfg.system_prompt.value, DEFAULT_SYSTEM_PROMPT);
    assert_eq!(cfg.fs_root.source, FieldSource::Default);
    assert!(cfg.fs_root.value.ends_with("aivyx-sandbox"));
    assert_eq!(cfg.storage_path.source, FieldSource::Default);
    // Default path without XDG_DATA_HOME should end in .local/share/aivyx/store.redb
    assert!(cfg
        .storage_path
        .value
        .to_string_lossy()
        .ends_with(".local/share/aivyx/store.redb"));
    assert_eq!(
        cfg.memory_max_per_topic.source,
        FieldSource::Default
    );
    assert_eq!(
        cfg.memory_max_per_topic.value,
        DEFAULT_MEMORY_MAX_PER_TOPIC
    );
    assert!(cfg.passphrase.is_none());
    assert!(cfg.telegram.is_none());

    // Phase 11 Task 1: the synthesized `default` role's system_prompt
    // should inherit `FieldSource::Default` from the legacy field —
    // a brand-new config with no prompt source should fall through
    // to DEFAULT_SYSTEM_PROMPT wrapped inside the synthesized role.
    let default_role = cfg
        .roles
        .get(DEFAULT_ROLE_NAME)
        .expect("default role synthesized");
    assert_eq!(default_role.system_prompt.source, FieldSource::Default);
    assert_eq!(default_role.system_prompt.value, DEFAULT_SYSTEM_PROMPT);
    // The synthesized `default` role uses AllowAll so pre-Phase-11
    // behavior is preserved: every registered tool is available.
    assert!(matches!(
        default_role.tool_allowlist.value,
        crate::ToolAllowlist::AllowAll
    ));
    // No prefix in the backwards-compat path → identical memory
    // layout to Phase 8–10.
    assert!(default_role.memory_topic_prefix.value.is_none());
    assert_eq!(cfg.active_role.value, DEFAULT_ROLE_NAME);
    assert!(cfg.warnings.is_empty());

    drop(env);
}

#[test]
fn xdg_data_home_wins_over_home_for_storage_default() {
    let env = EnvScope::new();
    env.set("XDG_DATA_HOME", "/tmp/xdg");

    let cfg = AivyxConfig::load_from_env_and_toml(&LoadOptions::test_env_only()).unwrap();
    assert_eq!(
        cfg.storage_path.value,
        PathBuf::from("/tmp/xdg/aivyx/store.redb")
    );
    assert_eq!(cfg.storage_path.source, FieldSource::Default);

    drop(env);
}

#[test]
fn no_home_and_no_fs_root_override_is_typed_error() {
    let env = EnvScope::new();
    env.clear("HOME");
    // fs_root's default needs HOME; unset → NoHome error.
    let err = AivyxConfig::load_from_env_and_toml(&LoadOptions::test_env_only())
        .expect_err("should fail without HOME");
    match err {
        ConfigError::NoHome { field } => assert_eq!(field, "fs_root"),
        other => panic!("expected NoHome, got {other:?}"),
    }

    drop(env);
}

// ------------------------------------------------------------------
// Phase 5: invalid parsing
// ------------------------------------------------------------------

#[test]
fn unparseable_memory_cap_is_typed_invalid_error() {
    let env = EnvScope::new();
    env.set("AIVYX_MEMORY_MAX_PER_TOPIC", "not-a-number");

    let err = AivyxConfig::load_from_env_and_toml(&LoadOptions::test_env_only())
        .expect_err("should fail parsing");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "memory_max_per_topic");
            assert!(
                reason.contains("not a valid usize"),
                "reason should mention usize parse: {reason}"
            );
        }
        other => panic!("expected Invalid, got {other:?}"),
    }

    drop(env);
}

#[test]
fn unparseable_telegram_chat_id_is_typed_invalid_error() {
    let env = EnvScope::new();
    env.set("AIVYX_TELEGRAM_CHAT_ID", "oops");

    let err = AivyxConfig::load_from_env_and_toml(&LoadOptions::test_env_only())
        .expect_err("should fail parsing");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "telegram.chat_id");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }

    drop(env);
}

#[test]
fn malformed_toml_is_typed_parse_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("malformed");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(&toml_path, "this is : not : valid TOML ][").unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path.clone()),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("should fail");
    match err {
        ConfigError::TomlParse { path, .. } => assert_eq!(path, toml_path),
        other => panic!("expected TomlParse, got {other:?}"),
    }

    drop(env);
}

#[test]
fn missing_toml_file_is_not_an_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("missing");
    let toml_path = tmp.path().join("does-not-exist.toml");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts)
        .expect("missing TOML file should load cleanly");
    assert_eq!(cfg.model.source, FieldSource::Default);

    drop(env);
}

// ------------------------------------------------------------------
// Phase 6: validation
// ------------------------------------------------------------------

#[test]
fn validate_errors_when_required_api_key_missing() {
    let env = EnvScope::new();
    let cfg = AivyxConfig::load_from_env_and_toml(&LoadOptions::test_env_only()).unwrap();
    let opts = LoadOptions {
        toml_path: None,
        require_api_key: true,
        require_telegram_token: false,
        role_override: None,
    };
    let err = cfg.validate(&opts).expect_err("should require api key");
    match err {
        ConfigError::Missing { field } => assert_eq!(field, "anthropic_api_key"),
        other => panic!("expected Missing, got {other:?}"),
    }

    drop(env);
}

#[test]
fn validate_errors_when_required_telegram_token_missing() {
    let env = EnvScope::new();
    // Set only the chat_id — token missing everywhere.
    env.set("AIVYX_TELEGRAM_CHAT_ID", "1");
    let cfg = AivyxConfig::load_from_env_and_toml(&LoadOptions::test_env_only()).unwrap();
    let opts = LoadOptions {
        toml_path: None,
        require_api_key: false,
        require_telegram_token: true,
        role_override: None,
    };
    let err = cfg.validate(&opts).expect_err("should require token");
    match err {
        ConfigError::Missing { field } => assert_eq!(field, "telegram.token"),
        other => panic!("expected Missing, got {other:?}"),
    }

    drop(env);
}

#[test]
fn validate_succeeds_when_everything_required_is_set() {
    let env = EnvScope::new();
    env.set("ANTHROPIC_API_KEY", "sk-x");
    env.set("AIVYX_TELEGRAM_TOKEN", "t-x");
    let cfg = AivyxConfig::load_from_env_and_toml(&LoadOptions::test_env_only()).unwrap();
    let opts = LoadOptions {
        toml_path: None,
        require_api_key: true,
        require_telegram_token: true,
        role_override: None,
    };
    cfg.validate(&opts).expect("should validate cleanly");

    drop(env);
}

// ------------------------------------------------------------------
// Phase 7: encrypted-store hydration
// ------------------------------------------------------------------

/// End-to-end: env + TOML do not supply the api key, but the
/// encrypted store has a row for `secret_keys::ANTHROPIC_API_KEY`.
/// After `hydrate_secrets_from_store`, the api key is populated with
/// `FieldSource::EncryptedStore`.
#[tokio::test]
async fn encrypted_store_hydrates_missing_api_key() {
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{KeyDomain, RedbStorage, StorageConfig};

    let env = EnvScope::new();
    // Make sure no env source competes with the store.
    env.clear("ANTHROPIC_API_KEY");

    let tmp = TempDir::new("store-hydrate");
    let store_path = tmp.path().join("store.redb");

    // `MasterKey::from_raw` is the documented test-only fast path
    // (see `aivyx-crypto` lib.rs doc on `MasterKey::from_raw`). Using
    // a hard-coded 32-byte array keeps tests in milliseconds instead
    // of seconds — Argon2id even with `weak_for_tests()` is ~10ms per
    // derivation and we do not need any of its security properties
    // to round-trip a byte row through `KeyDomain::Secrets`.
    let master = MasterKey::from_raw([7u8; 32]);

    let storage = RedbStorage::open(StorageConfig::new(store_path), master)
        .await
        .expect("open store");

    // Seed the secrets domain with a known api key.
    let secrets = storage.domain(KeyDomain::Secrets);
    secrets
        .put(crate::secret_keys::ANTHROPIC_API_KEY, b"sk-from-store")
        .await
        .expect("put api key");

    // Load env+TOML → api key is None.
    let mut cfg =
        AivyxConfig::load_from_env_and_toml(&LoadOptions::test_env_only()).expect("load");
    assert!(cfg.anthropic_api_key.is_none());

    // Hydrate from the store → api key is now populated.
    cfg.hydrate_secrets_from_store(&storage)
        .await
        .expect("hydrate");
    let api_key = cfg
        .anthropic_api_key
        .as_ref()
        .expect("hydrated from store");
    assert_eq!(api_key.source, FieldSource::EncryptedStore);
    assert_eq!(api_key.value.expose_secret(), "sk-from-store");

    // Validate with required_api_key = true now succeeds.
    let opts = LoadOptions {
        toml_path: None,
        require_api_key: true,
        require_telegram_token: false,
        role_override: None,
    };
    cfg.validate(&opts).expect("api key present after hydration");

    drop(env);
}

/// Env already supplies the api key → hydrate_secrets_from_store must
/// not overwrite it even if the store has a different row. Precedence
/// is env > toml > store.
#[tokio::test]
async fn env_wins_over_encrypted_store_on_hydrate() {
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{KeyDomain, RedbStorage, StorageConfig};

    let env = EnvScope::new();
    env.set("ANTHROPIC_API_KEY", "sk-from-env");

    let tmp = TempDir::new("store-vs-env");
    let store_path = tmp.path().join("store.redb");
    let master = MasterKey::from_raw([8u8; 32]);
    let storage = RedbStorage::open(StorageConfig::new(store_path), master)
        .await
        .unwrap();
    let secrets = storage.domain(KeyDomain::Secrets);
    secrets
        .put(crate::secret_keys::ANTHROPIC_API_KEY, b"sk-store-loses")
        .await
        .unwrap();

    let mut cfg = AivyxConfig::load_from_env_and_toml(&LoadOptions::test_env_only()).unwrap();
    cfg.hydrate_secrets_from_store(&storage).await.unwrap();

    let api_key = cfg.anthropic_api_key.as_ref().unwrap();
    assert_eq!(api_key.source, FieldSource::Env);
    assert_eq!(api_key.value.expose_secret(), "sk-from-env");

    drop(env);
}

/// A non-UTF-8 secret row surfaces as typed `NonUtf8Secret`.
#[tokio::test]
async fn non_utf8_secret_in_store_is_typed_error() {
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{KeyDomain, RedbStorage, StorageConfig};

    let env = EnvScope::new();
    let tmp = TempDir::new("store-nonutf8");
    let store_path = tmp.path().join("store.redb");
    let master = MasterKey::from_raw([9u8; 32]);
    let storage = RedbStorage::open(StorageConfig::new(store_path), master)
        .await
        .unwrap();
    let secrets = storage.domain(KeyDomain::Secrets);
    // invalid UTF-8: a stray high byte without a valid continuation
    secrets
        .put(crate::secret_keys::ANTHROPIC_API_KEY, &[0xff, 0xfe, 0xfd])
        .await
        .unwrap();

    let mut cfg = AivyxConfig::load_from_env_and_toml(&LoadOptions::test_env_only()).unwrap();
    let err = cfg
        .hydrate_secrets_from_store(&storage)
        .await
        .expect_err("should reject non-UTF-8");
    match err {
        ConfigError::NonUtf8Secret { field } => assert_eq!(field, "anthropic_api_key"),
        other => panic!("expected NonUtf8Secret, got {other:?}"),
    }

    drop(env);
}

// ------------------------------------------------------------------
// Phase 11 Task 1 — Role primitive
// ------------------------------------------------------------------
//
// These tests cover the new `Role` / `ToolAllowlist` types, the
// `[[role]]` TOML schema, the implicit-`default`-role backwards-
// compatibility bridge, active-role resolution priority, and the
// `UnknownRole` / Q4 warning error-handling paths.
//
// The decisions pinned by these tests:
//   - Q3 resolution: absent `tool_allowlist` → `AllowAll`,
//     explicit empty `tool_allowlist = []` → `Only(vec![])` (deny all).
//   - Q4 resolution (Option B): a config with both legacy
//     `system_prompt` *and* explicit `[[role]]` entries loads
//     successfully with a non-fatal warning on `AivyxConfig::warnings`.
//   - Active-role priority: `LoadOptions::role_override` > `AIVYX_ROLE`
//     env var > `DEFAULT_ROLE_NAME`.
//   - Backwards compat: zero `[[role]]` entries synthesize an implicit
//     `default` role that inherits the legacy `system_prompt`'s
//     original `FieldSource` so provenance rendering is preserved.

/// Two explicit `[[role]]` entries in the TOML file load into the
/// `roles` map, keyed by name. Each field is parsed into the runtime
/// types with `FieldSource::Toml` wrappers; absent fields fall back
/// to defaults. Also checks the deterministic `BTreeMap` ordering
/// that the module docstring promises.
#[test]
fn explicit_roles_from_toml_parse_into_role_map() {
    let env = EnvScope::new();
    let tmp = TempDir::new("roles-toml");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "coder"
system_prompt = "You are a pair-programmer."
tool_allowlist = ["fs.read", "fs.write", "memory.read", "memory.write", "shell.exec"]
memory_topic_prefix = "coder/"

[[role]]
name = "researcher"
system_prompt = "You are a careful note-taker."
tool_allowlist = ["fs.read", "memory.read", "memory.write"]
memory_topic_prefix = "researcher/"
"#,
    )
    .unwrap();

    // Select `coder` explicitly via env var so the load succeeds;
    // neither `coder` nor `researcher` is the default role name so
    // omitting the selector would fail `UnknownRole`.
    env.set("AIVYX_ROLE", "coder");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");

    assert_eq!(cfg.roles.len(), 2);
    let coder = cfg.roles.get("coder").expect("coder role present");
    assert_eq!(coder.name.value, "coder");
    assert_eq!(coder.name.source, FieldSource::Toml);
    assert_eq!(coder.system_prompt.source, FieldSource::Toml);
    assert_eq!(coder.system_prompt.value, "You are a pair-programmer.");
    match &coder.tool_allowlist.value {
        ToolAllowlist::Only(tools) => {
            assert_eq!(
                tools,
                &vec![
                    "fs.read".to_string(),
                    "fs.write".to_string(),
                    "memory.read".to_string(),
                    "memory.write".to_string(),
                    "shell.exec".to_string(),
                ]
            );
        }
        other => panic!("expected Only(_), got {other:?}"),
    }
    assert_eq!(coder.memory_topic_prefix.value.as_deref(), Some("coder/"));

    let researcher = cfg
        .roles
        .get("researcher")
        .expect("researcher role present");
    assert_eq!(researcher.name.value, "researcher");
    match &researcher.tool_allowlist.value {
        ToolAllowlist::Only(tools) => assert_eq!(tools.len(), 3),
        other => panic!("expected Only(_), got {other:?}"),
    }
    assert_eq!(
        researcher.memory_topic_prefix.value.as_deref(),
        Some("researcher/")
    );

    // BTreeMap ordering — iteration is alphabetical by key.
    let names: Vec<&str> = cfg.roles.keys().map(String::as_str).collect();
    assert_eq!(names, vec!["coder", "researcher"]);

    // Active role reflects the AIVYX_ROLE env var.
    assert_eq!(cfg.active_role.value, "coder");
    assert_eq!(cfg.active_role.source, FieldSource::Env);

    drop(env);
}

/// Q3 resolution — a `[[role]]` entry with no `tool_allowlist` key
/// at all maps to `ToolAllowlist::AllowAll` (no filter, every
/// registered tool available).
#[test]
fn role_without_tool_allowlist_key_is_allow_all() {
    let env = EnvScope::new();
    let tmp = TempDir::new("role-allow-all");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "open"
system_prompt = "no allowlist key at all"
"#,
    )
    .unwrap();
    env.set("AIVYX_ROLE", "open");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let role = cfg.roles.get("open").unwrap();
    assert_eq!(role.tool_allowlist.value, ToolAllowlist::AllowAll);
    // `AllowAll` came from the default branch (field absent), not
    // from TOML-supplied source.
    assert_eq!(role.tool_allowlist.source, FieldSource::Default);

    drop(env);
}

/// Q3 resolution — a `[[role]]` entry with `tool_allowlist = []`
/// (explicit empty list) maps to `ToolAllowlist::Only(vec![])`,
/// meaning "deny every tool." This is a legal configuration
/// distinct from an absent field.
#[test]
fn role_with_empty_tool_allowlist_is_deny_all() {
    let env = EnvScope::new();
    let tmp = TempDir::new("role-deny-all");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "locked"
system_prompt = "deny-all role"
tool_allowlist = []
"#,
    )
    .unwrap();
    env.set("AIVYX_ROLE", "locked");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let role = cfg.roles.get("locked").unwrap();
    match &role.tool_allowlist.value {
        ToolAllowlist::Only(v) => assert!(v.is_empty(), "explicit empty list"),
        other => panic!("expected Only(empty), got {other:?}"),
    }
    // An explicit empty list is `Toml`-sourced, not `Default`.
    assert_eq!(role.tool_allowlist.source, FieldSource::Toml);

    drop(env);
}

/// Active-role priority: `LoadOptions::role_override` beats the
/// `AIVYX_ROLE` env var.
#[test]
fn role_override_beats_env_var() {
    let env = EnvScope::new();
    let tmp = TempDir::new("role-override");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "first"
system_prompt = "first"

[[role]]
name = "second"
system_prompt = "second"
"#,
    )
    .unwrap();
    env.set("AIVYX_ROLE", "first");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        // Override wins even though the env var says "first".
        role_override: Some("second".to_string()),
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.active_role.value, "second");

    drop(env);
}

/// Active-role priority: when `LoadOptions::role_override` is `None`
/// the `AIVYX_ROLE` env var is honored.
#[test]
fn env_var_sets_active_role_when_no_override() {
    let env = EnvScope::new();
    let tmp = TempDir::new("role-env");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "primary"
system_prompt = "primary"

[[role]]
name = "secondary"
system_prompt = "secondary"
"#,
    )
    .unwrap();
    env.set("AIVYX_ROLE", "secondary");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.active_role.value, "secondary");
    assert_eq!(cfg.active_role.source, FieldSource::Env);

    drop(env);
}

/// Selecting an active role that doesn't exist in the loaded config
/// is a typed `UnknownRole` error; the error includes the list of
/// known role names so operators can diagnose the typo.
#[test]
fn active_role_not_in_config_is_typed_unknown_role_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("role-unknown");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "real-role"
system_prompt = "real"
"#,
    )
    .unwrap();
    env.set("AIVYX_ROLE", "ghost-role");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("ghost-role should fail active-role validation");
    match err {
        ConfigError::UnknownRole { name, known } => {
            assert_eq!(name, "ghost-role");
            assert_eq!(known, vec!["real-role".to_string()]);
        }
        other => panic!("expected UnknownRole, got {other:?}"),
    }

    drop(env);
}

/// Q4 resolution — a config with both a legacy env-sourced
/// `system_prompt` *and* an explicit `[[role]]` entry loads
/// successfully. `AivyxConfig::warnings` accumulates a clear
/// message; the runtime behavior prefers the explicit role.
#[test]
fn legacy_system_prompt_with_explicit_roles_warns_but_loads() {
    let env = EnvScope::new();
    let tmp = TempDir::new("role-legacy-conflict");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "explicit"
system_prompt = "from the explicit role"
"#,
    )
    .unwrap();
    // Legacy env-level prompt — user forgot to remove it when
    // adopting roles.
    env.set("AIVYX_SYSTEM_PROMPT", "legacy value that will be shadowed");
    env.set("AIVYX_ROLE", "explicit");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load with conflict");
    // Legacy field is still populated for the banner's sake.
    assert_eq!(
        cfg.system_prompt.value,
        "legacy value that will be shadowed"
    );
    // The explicit role's prompt is what Task 4 will consume.
    let role = cfg.roles.get("explicit").unwrap();
    assert_eq!(role.system_prompt.value, "from the explicit role");
    // Exactly one warning was emitted, and it mentions the legacy
    // field so the operator can find it.
    assert_eq!(
        cfg.warnings.len(),
        1,
        "one warning about the legacy field: got {:?}",
        cfg.warnings
    );
    assert!(cfg.warnings[0].contains("system_prompt"));

    drop(env);
}

/// Q4 refinement — when the legacy `system_prompt` is sourced from
/// `FieldSource::Default` (no user source supplied it), adding an
/// explicit `[[role]]` entry must **not** fire the warning. A brand-
/// new role-using config should be silent.
#[test]
fn explicit_role_with_default_legacy_prompt_emits_no_warning() {
    let env = EnvScope::new();
    let tmp = TempDir::new("role-silent");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "alpha"
system_prompt = "brand new role, no legacy baggage"
"#,
    )
    .unwrap();
    // Deliberately DO NOT set AIVYX_SYSTEM_PROMPT. The legacy field
    // falls through to DEFAULT_SYSTEM_PROMPT with source `Default`,
    // which must not trip the warning.
    env.set("AIVYX_ROLE", "alpha");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.system_prompt.source, FieldSource::Default);
    assert!(
        cfg.warnings.is_empty(),
        "no warning when legacy prompt source is Default: {:?}",
        cfg.warnings
    );

    drop(env);
}

/// A `Role` parsed from TOML that omits both `system_prompt` and
/// `memory_topic_prefix` still loads. The omitted fields fall
/// through to `Default`-sourced values on the runtime `Role`.
#[test]
fn role_with_only_name_populates_defaults_for_optional_fields() {
    let env = EnvScope::new();
    let tmp = TempDir::new("role-minimal");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "bare"
"#,
    )
    .unwrap();
    env.set("AIVYX_ROLE", "bare");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let role = cfg.roles.get("bare").unwrap();
    assert_eq!(role.system_prompt.source, FieldSource::Default);
    assert_eq!(role.system_prompt.value, DEFAULT_SYSTEM_PROMPT);
    assert!(matches!(
        role.tool_allowlist.value,
        ToolAllowlist::AllowAll
    ));
    assert_eq!(role.tool_allowlist.source, FieldSource::Default);
    assert!(role.memory_topic_prefix.value.is_none());
    assert_eq!(role.memory_topic_prefix.source, FieldSource::Default);

    drop(env);
}

/// Public-API smoke test: the `Role` struct's fields are all public
/// and the `ToolAllowlist` variants are all constructible from
/// outside `aivyx-config`. Task 4 will consume these via the
/// `aivyx-channel` binary; this test proves the API surface supports
/// that consumption pattern without any unexposed internals.
///
/// Phase 13 Task 1 extended `Role` with three more fields
/// (`capability_scopes`, `trust_ceiling`, `parent_role`). This test
/// now constructs them inline as well, asserting the whole struct
/// remains exhaustively literal-constructible from outside the crate.
#[test]
fn role_struct_is_constructible_and_matchable_from_outside() {
    use aivyx_capability::TrustTier;

    let role = Role {
        name: crate::Sourced::new("test".to_string(), FieldSource::Default),
        system_prompt: crate::Sourced::new("sp".to_string(), FieldSource::Default),
        tool_allowlist: crate::Sourced::new(
            ToolAllowlist::Only(vec!["fs.read".to_string()]),
            FieldSource::Default,
        ),
        memory_topic_prefix: crate::Sourced::new(Some("x/".to_string()), FieldSource::Default),
        capability_scopes: crate::Sourced::new(Vec::new(), FieldSource::Default),
        trust_ceiling: crate::Sourced::new(TrustTier::Trusted, FieldSource::Default),
        parent_role: crate::Sourced::new(None, FieldSource::Default),
    };
    // The match is exhaustive against the public enum — if Task 3 or
    // a later task ever adds a variant, this test exists to catch
    // the surface change at the `_ => unreachable!()` alternative
    // missing.
    let behavior = match &role.tool_allowlist.value {
        ToolAllowlist::AllowAll => "no filter",
        ToolAllowlist::Only(_) => "filtered",
    };
    assert_eq!(behavior, "filtered");
    assert_eq!(role.name.value, "test");
    assert!(role.capability_scopes.value.is_empty());
    assert_eq!(role.trust_ceiling.value, TrustTier::Trusted);
    assert!(role.parent_role.value.is_none());
}

// ====================================================================
// Phase 13 Task 1 — per-role capability envelope fields
// ====================================================================
//
// These tests exercise the three fields added to `Role` in Phase 13
// Task 1 (`capability_scopes`, `trust_ceiling`, `parent_role`) plus
// the single-inheritance tree validator that runs at config-load
// time. Each test names the invariant it locks in so a future
// refactor that breaks one knows which contract it just violated.

/// Phase 11 backcompat — a TOML file with one explicit `[[role]]`
/// entry that touches none of the three new Phase 13 fields still
/// loads. The new fields populate from their absent-key defaults:
/// empty `capability_scopes`, `Trusted` ceiling, and `parent_role =
/// None` (because no `default` role exists in the same file to
/// implicit-parent against — Q4's "implicit-from-default only when
/// default is declared" rule).
#[test]
fn legacy_role_loads_with_default_capability_envelope() {
    use aivyx_capability::TrustTier;

    let env = EnvScope::new();
    let tmp = TempDir::new("phase13-legacy-role");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "coder"
system_prompt = "You are a pair-programmer."
"#,
    )
    .unwrap();
    env.set("AIVYX_ROLE", "coder");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");

    let role = cfg.roles.get("coder").expect("coder role present");
    assert!(
        role.capability_scopes.value.is_empty(),
        "absent capability_scopes key → empty Vec"
    );
    assert_eq!(role.capability_scopes.source, FieldSource::Default);
    assert_eq!(role.trust_ceiling.value, TrustTier::Trusted);
    assert_eq!(role.trust_ceiling.source, FieldSource::Default);
    assert!(
        role.parent_role.value.is_none(),
        "no `default` role declared → this role is its own tree root"
    );
    assert_eq!(role.parent_role.source, FieldSource::Default);

    drop(env);
}

/// An explicit `capability_scopes = ["fs.read", "shell.exec:git"]`
/// list parses through `Scope::parse` and lands as
/// `Sourced::new(Vec<Scope>, FieldSource::Toml)`. Locks in (a) the
/// scope-string-parsing-at-config-load-time decision from Q2,
/// (b) the `FieldSource::Toml` provenance for explicit lists, and
/// (c) the round-trip through `Scope::as_str` so the in-memory
/// `Scope` holds the original string verbatim.
#[test]
fn explicit_capability_scopes_parse_at_load_time() {
    let env = EnvScope::new();
    let tmp = TempDir::new("phase13-scopes");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "shellrunner"
system_prompt = "shell role"
capability_scopes = ["fs.read", "shell.exec:git", "memory.write"]
"#,
    )
    .unwrap();
    env.set("AIVYX_ROLE", "shellrunner");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");

    let role = cfg.roles.get("shellrunner").unwrap();
    assert_eq!(role.capability_scopes.source, FieldSource::Toml);
    let scope_strings: Vec<&str> = role
        .capability_scopes
        .value
        .iter()
        .map(|s| s.as_str())
        .collect();
    assert_eq!(scope_strings, vec!["fs.read", "shell.exec:git", "memory.write"]);

    drop(env);
}

/// An unknown scope base in `capability_scopes` (not in
/// `KNOWN_BASES`) fails loudly at config-load time, not at
/// capability-check time later. The error message names the role
/// and the bad scope string so the operator can grep their TOML.
#[test]
fn unknown_capability_scope_fails_loudly_at_load_time() {
    let env = EnvScope::new();
    let tmp = TempDir::new("phase13-bad-scope");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "broken"
system_prompt = "broken"
capability_scopes = ["fs.read", "this.is.not.a.real.base"]
"#,
    )
    .unwrap();
    env.set("AIVYX_ROLE", "broken");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("unknown scope should fail load");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "role.capability_scopes");
            assert!(reason.contains("broken"), "mentions role name: {reason}");
            assert!(
                reason.contains("this.is.not.a.real.base"),
                "mentions bad scope: {reason}"
            );
        }
        other => panic!("expected Invalid, got {other:?}"),
    }

    drop(env);
}

/// All four `TrustTier` variants parse from TOML strings (via
/// `serde::Deserialize` derived on `TrustTier` in
/// `aivyx-capability`), and an unknown variant fails with a
/// `TomlParse` error pointing at the offending file. Locks in that
/// trust-tier validation is a TOML-parse-time concern, not a
/// post-parse loader concern — typos surface with file+line context.
#[test]
fn trust_ceiling_parses_all_four_tiers_and_rejects_garbage() {
    use aivyx_capability::TrustTier;

    let env = EnvScope::new();
    for (tier_str, expected) in [
        ("Kernel", TrustTier::Kernel),
        ("Trusted", TrustTier::Trusted),
        ("SemiTrusted", TrustTier::SemiTrusted),
        ("Untrusted", TrustTier::Untrusted),
    ] {
        let tmp = TempDir::new(&format!("phase13-tier-{tier_str}"));
        let toml_path = tmp.path().join("aivyx.toml");
        std::fs::write(
            &toml_path,
            format!(
                r#"
[[role]]
name = "tiered"
system_prompt = "tier check"
trust_ceiling = "{tier_str}"
"#
            ),
        )
        .unwrap();
        env.set("AIVYX_ROLE", "tiered");

        let opts = LoadOptions {
            toml_path: Some(toml_path),
            require_api_key: false,
            require_telegram_token: false,
            role_override: None,
        };
        let cfg = AivyxConfig::load_from_env_and_toml(&opts)
            .unwrap_or_else(|e| panic!("load {tier_str}: {e:?}"));
        let role = cfg.roles.get("tiered").unwrap();
        assert_eq!(role.trust_ceiling.value, expected);
        assert_eq!(role.trust_ceiling.source, FieldSource::Toml);
    }

    // Garbage tier name surfaces as a TomlParse error (serde
    // rejects the unknown variant during `toml::from_str`).
    let tmp = TempDir::new("phase13-tier-garbage");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "tiered"
system_prompt = "tier check"
trust_ceiling = "Goat"
"#,
    )
    .unwrap();
    env.set("AIVYX_ROLE", "tiered");
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("garbage tier should fail");
    assert!(
        matches!(err, ConfigError::TomlParse { .. }),
        "expected TomlParse, got {err:?}"
    );

    drop(env);
}

/// `parent_role` must name an existing role. A typo (or a renamed
/// role that some other entry still points at) fails loudly with
/// `RoleInheritance`, naming the offending role *and* the missing
/// parent string.
#[test]
fn parent_role_pointing_at_unknown_role_fails_loudly() {
    let env = EnvScope::new();
    let tmp = TempDir::new("phase13-parent-typo");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "default"
system_prompt = "root"

[[role]]
name = "child"
system_prompt = "child"
parent_role = "no-such-role"
"#,
    )
    .unwrap();
    env.set("AIVYX_ROLE", "child");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("unknown parent should fail");
    match err {
        ConfigError::RoleInheritance { reason } => {
            assert!(reason.contains("child"), "mentions child: {reason}");
            assert!(
                reason.contains("no-such-role"),
                "mentions missing parent: {reason}"
            );
        }
        other => panic!("expected RoleInheritance, got {other:?}"),
    }

    drop(env);
}

/// A `parent_role` cycle (A → B → A) is detected at load time and
/// surfaces as `RoleInheritance`. Locks in invariant 3 of the
/// single-inheritance tree validator. Includes a self-cycle as a
/// sub-case because self-cycles are the degenerate path through
/// the same code.
#[test]
fn parent_role_cycle_is_detected_at_load_time() {
    // --- self-cycle (A → A) ---
    {
        let env = EnvScope::new();
        let tmp = TempDir::new("phase13-self-cycle");
        let toml_path = tmp.path().join("aivyx.toml");
        std::fs::write(
            &toml_path,
            r#"
[[role]]
name = "selfish"
system_prompt = "self-loop"
parent_role = "selfish"
"#,
        )
        .unwrap();
        env.set("AIVYX_ROLE", "selfish");
        let opts = LoadOptions {
            toml_path: Some(toml_path),
            require_api_key: false,
            require_telegram_token: false,
            role_override: None,
        };
        let err = AivyxConfig::load_from_env_and_toml(&opts)
            .expect_err("self-cycle should fail");
        match err {
            ConfigError::RoleInheritance { reason } => {
                assert!(reason.contains("selfish"), "mentions role: {reason}");
                assert!(
                    reason.contains("self-cycle") || reason.contains("itself"),
                    "names the failure mode: {reason}"
                );
            }
            other => panic!("expected RoleInheritance, got {other:?}"),
        }
        drop(env);
    }

    // --- two-hop cycle (A → B → A) ---
    {
        let env = EnvScope::new();
        let tmp = TempDir::new("phase13-two-hop-cycle");
        let toml_path = tmp.path().join("aivyx.toml");
        std::fs::write(
            &toml_path,
            r#"
[[role]]
name = "a"
system_prompt = "a"
parent_role = "b"

[[role]]
name = "b"
system_prompt = "b"
parent_role = "a"
"#,
        )
        .unwrap();
        env.set("AIVYX_ROLE", "a");
        let opts = LoadOptions {
            toml_path: Some(toml_path),
            require_api_key: false,
            require_telegram_token: false,
            role_override: None,
        };
        let err = AivyxConfig::load_from_env_and_toml(&opts)
            .expect_err("two-hop cycle should fail");
        match err {
            ConfigError::RoleInheritance { reason } => {
                assert!(reason.contains("cycle"), "mentions cycle: {reason}");
            }
            other => panic!("expected RoleInheritance, got {other:?}"),
        }
        drop(env);
    }
}

// ====================================================================
// Phase 13 Task 2 — child-parent attenuation invariant (invariant 5)
// ====================================================================

/// PRODUCT.md P7's "child can attenuate, never widen" rule fires
/// at config-load time when a child role declares a
/// `capability_scopes` entry that its constraining ancestor does
/// not grant. The error message names both the child role, the
/// offending scope string, and the constraining ancestor whose
/// declared scopes failed to grant it.
#[test]
fn child_role_widening_parent_envelope_fails_at_load_time() {
    let env = EnvScope::new();
    let tmp = TempDir::new("phase13t2-widen");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "default"
system_prompt = "root"
capability_scopes = ["fs.read"]

[[role]]
name = "rogue"
system_prompt = "tries to widen"
parent_role = "default"
capability_scopes = ["fs.read", "shell.exec"]
"#,
    )
    .unwrap();
    env.set("AIVYX_ROLE", "rogue");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("widening child should fail");
    match err {
        ConfigError::RoleInheritance { reason } => {
            assert!(reason.contains("rogue"), "names child: {reason}");
            assert!(
                reason.contains("shell.exec"),
                "names offending scope: {reason}"
            );
            assert!(
                reason.contains("default"),
                "names constraining ancestor: {reason}"
            );
        }
        other => panic!("expected RoleInheritance, got {other:?}"),
    }

    drop(env);
}

/// Empty `capability_scopes` is the unconstrained sentinel: a
/// role with no declared scopes adds no constraint, and the
/// attenuation walk skips through it to find the next non-empty
/// ancestor. This pins that a `grandparent → empty parent →
/// child` chain validates the child against the **grandparent's**
/// scopes, not the parent's empty set (which would otherwise
/// either pass everything or fail everything depending on edge
/// behavior).
#[test]
fn attenuation_walk_skips_empty_parent_to_grandparent() {
    let env = EnvScope::new();
    let tmp = TempDir::new("phase13t2-skip-empty");
    let toml_path = tmp.path().join("aivyx.toml");
    // grandparent: fs.read only
    // parent: empty (sentinel — adds nothing)
    // child: tries to declare net.fetch
    // Expected: fail, because grandparent doesn't grant net.fetch
    // and the walk skips through the empty parent.
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "grandparent"
system_prompt = "gp"
capability_scopes = ["fs.read"]

[[role]]
name = "parent"
system_prompt = "p"
parent_role = "grandparent"

[[role]]
name = "child"
system_prompt = "c"
parent_role = "parent"
capability_scopes = ["net.fetch"]
"#,
    )
    .unwrap();
    env.set("AIVYX_ROLE", "child");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("widening through empty parent should fail");
    match err {
        ConfigError::RoleInheritance { reason } => {
            assert!(reason.contains("child"), "names child: {reason}");
            assert!(reason.contains("net.fetch"), "names scope: {reason}");
            assert!(
                reason.contains("grandparent"),
                "constraining ancestor is grandparent, not parent: {reason}"
            );
        }
        other => panic!("expected RoleInheritance, got {other:?}"),
    }

    drop(env);
}

/// D4 prefix-attenuation under inheritance: a child declaring
/// `fs.read:/tmp/**` under a parent declaring unqualified
/// `fs.read` loads cleanly, because Rule 2 ("unqualified held
/// grants any qualified needed with the same base") makes the
/// parent's unqualified scope grant the child's narrow one. This
/// is the load-time analog of the runtime intersection behavior
/// — both routes (config validation, runtime envelope assembly)
/// agree on what counts as "child can attenuate."
#[test]
fn child_qualifier_under_unqualified_parent_loads_cleanly() {
    let env = EnvScope::new();
    let tmp = TempDir::new("phase13t2-qualifier-attenuation");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "default"
system_prompt = "broad parent"
capability_scopes = ["fs.read", "fs.write"]

[[role]]
name = "narrow"
system_prompt = "narrowed child"
parent_role = "default"
capability_scopes = ["fs.read:/etc/**"]
"#,
    )
    .unwrap();
    env.set("AIVYX_ROLE", "narrow");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts)
        .expect("qualifier attenuation should load");
    let role = cfg.roles.get("narrow").unwrap();
    assert_eq!(role.capability_scopes.value.len(), 1);
    assert_eq!(
        role.capability_scopes.value[0].as_str(),
        "fs.read:/etc/**"
    );
}

/// Q4 ergonomic — when an explicit `default` role is declared
/// alongside other roles, those other roles implicitly inherit
/// from `default` (with `FieldSource::Default` provenance, since
/// no operator wrote `parent_role = "default"` literally).
/// This locks in the "implicit-from-default *when default exists*"
/// half of Q4 — the other half (no-default → root) is locked in
/// by `legacy_role_loads_with_default_capability_envelope`.
#[test]
fn implicit_parent_default_kicks_in_when_default_role_is_declared() {
    let env = EnvScope::new();
    let tmp = TempDir::new("phase13-implicit-parent");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[role]]
name = "default"
system_prompt = "root prompt"

[[role]]
name = "coder"
system_prompt = "coder prompt"
"#,
    )
    .unwrap();
    env.set("AIVYX_ROLE", "coder");

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");

    let default_role = cfg.roles.get("default").unwrap();
    assert!(
        default_role.parent_role.value.is_none(),
        "default role is its own root"
    );

    let coder = cfg.roles.get("coder").unwrap();
    assert_eq!(
        coder.parent_role.value.as_deref(),
        Some("default"),
        "coder implicitly inherits from default"
    );
    assert_eq!(
        coder.parent_role.source,
        FieldSource::Default,
        "implicit parent has Default source — no operator typed it"
    );

    drop(env);
}

// ------------------------------------------------------------------
// Phase 24: [[mcp_server]] config entries
// ------------------------------------------------------------------

#[test]
fn mcp_server_entries_parse_from_toml() {
    let env = EnvScope::new();
    let tmp = TempDir::new("mcp-cfg");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[mcp_server]]
name = "github"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]

[[mcp_server]]
name = "disabled-one"
command = "echo"
enabled = false

[[mcp_server]]
name = "bare"
command = "/usr/bin/my-server"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");

    assert_eq!(cfg.mcp_servers.len(), 2, "disabled server filtered out");

    let gh = &cfg.mcp_servers[0];
    assert_eq!(gh.name, "github");
    assert_eq!(gh.transport, McpTransportKind::Stdio);
    assert_eq!(gh.command.as_deref(), Some("npx"));
    assert_eq!(gh.args, vec!["-y", "@modelcontextprotocol/server-github"]);
    assert!(gh.enabled);

    let bare = &cfg.mcp_servers[1];
    assert_eq!(bare.name, "bare");
    assert_eq!(bare.transport, McpTransportKind::Stdio);
    assert_eq!(bare.command.as_deref(), Some("/usr/bin/my-server"));
    assert!(bare.args.is_empty(), "absent args default to empty vec");
    assert!(bare.enabled);

    drop(env);
}

#[test]
fn no_mcp_server_section_gives_empty_vec() {
    let env = EnvScope::new();
    let tmp = TempDir::new("no-mcp");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert!(cfg.mcp_servers.is_empty());

    drop(env);
}

#[test]
fn mcp_server_sse_transport_parses() {
    let env = EnvScope::new();
    let tmp = TempDir::new("mcp-sse");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[mcp_server]]
name = "remote"
transport = "sse"
url = "http://example.com:8080/sse"

[[mcp_server]]
name = "local"
command = "npx"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");

    assert_eq!(cfg.mcp_servers.len(), 2);

    let remote = &cfg.mcp_servers[0];
    assert_eq!(remote.name, "remote");
    assert_eq!(remote.transport, McpTransportKind::Sse);
    assert_eq!(remote.url.as_deref(), Some("http://example.com:8080/sse"));
    assert!(remote.command.is_none());

    let local = &cfg.mcp_servers[1];
    assert_eq!(local.name, "local");
    assert_eq!(local.transport, McpTransportKind::Stdio);
    assert_eq!(local.command.as_deref(), Some("npx"));
    assert!(local.url.is_none());

    drop(env);
}

#[test]
fn mcp_server_sse_missing_url_is_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("mcp-sse-no-url");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[mcp_server]]
name = "broken"
transport = "sse"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("sse without url must fail");
    let msg = err.to_string();
    assert!(msg.contains("url"), "error must mention url: {msg}");

    drop(env);
}

// ------------------------------------------------------------------
// Phase 49 — [[tool_process]] config
// ------------------------------------------------------------------

#[test]
fn tool_process_basic_entry_loads() {
    let env = EnvScope::new();
    let tmp = TempDir::new("tool-process-basic");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[tool_process]]
name = "wordcount"
command = "python3"
args = ["/path/to/tool.py"]

[tool_process.env]
LOG_LEVEL = "info"

[tool_process.scope_overrides]
wordcount = "memory.read:topic:wc/**"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.tool_processes.len(), 1);
    let t = &cfg.tool_processes[0];
    assert_eq!(t.name, "wordcount");
    assert_eq!(t.command, "python3");
    assert_eq!(t.args, vec!["/path/to/tool.py"]);
    assert!(t.enabled);
    assert_eq!(t.env.len(), 1);
    assert_eq!(t.env[0].0, "LOG_LEVEL");
    assert_eq!(t.env[0].1, "info");
    assert_eq!(t.scope_overrides.len(), 1);
    assert_eq!(
        t.scope_overrides.get("wordcount").map(String::as_str),
        Some("memory.read:topic:wc/**"),
    );
    drop(env);
}

#[test]
fn tool_process_disabled_entries_filtered() {
    let env = EnvScope::new();
    let tmp = TempDir::new("tool-process-disabled");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[tool_process]]
name = "active"
command = "python3"

[[tool_process]]
name = "skipped"
command = "python3"
enabled = false
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.tool_processes.len(), 1);
    assert_eq!(cfg.tool_processes[0].name, "active");
    drop(env);
}

#[test]
fn tool_process_sandbox_block_loads() {
    let env = EnvScope::new();
    let tmp = TempDir::new("tool-process-sandbox");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[tool_process]]
name = "sandboxed"
command = "python3"
args = ["/path/to/tool.py"]

[tool_process.sandbox]
wrapper = "bwrap"
args = ["--ro-bind", "/", "/", "--proc", "/proc", "--unshare-all", "--die-with-parent", "--"]
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.tool_processes.len(), 1);
    let sandbox = cfg.tool_processes[0]
        .sandbox
        .as_ref()
        .expect("sandbox block must be Some");
    assert_eq!(sandbox.wrapper, "bwrap");
    assert!(sandbox.args.iter().any(|a| a == "--unshare-all"));
    drop(env);
}

#[test]
fn tool_process_sandbox_empty_wrapper_is_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("tool-process-sandbox-empty");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[tool_process]]
name = "broken-sandbox"
command = "python3"

[tool_process.sandbox]
wrapper = "   "
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("empty wrapper must fail");
    let msg = err.to_string();
    assert!(msg.contains("sandbox.wrapper"), "error must name field: {msg}");
    drop(env);
}

#[test]
fn tool_process_without_sandbox_is_none() {
    let env = EnvScope::new();
    let tmp = TempDir::new("tool-process-no-sandbox");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[tool_process]]
name = "plain"
command = "python3"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert!(
        cfg.tool_processes[0].sandbox.is_none(),
        "omitting [tool_process.sandbox] must yield None",
    );
    drop(env);
}

#[test]
fn tool_process_empty_command_is_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("tool-process-empty-cmd");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[tool_process]]
name = "broken"
command = "   "
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("empty command must fail");
    let msg = err.to_string();
    assert!(msg.contains("command"), "error must mention command: {msg}");
    drop(env);
}

// ------------------------------------------------------------------
// Phase 55 — [mcp_server.sandbox] schema
// ------------------------------------------------------------------

#[test]
fn mcp_server_sandbox_block_loads() {
    let env = EnvScope::new();
    let tmp = TempDir::new("mcp-sandbox-basic");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[mcp_server]]
name = "external-thing"
command = "/usr/local/bin/external-mcp"

[mcp_server.sandbox]
wrapper = "bwrap"
args = ["--ro-bind", "/", "/", "--proc", "/proc", "--unshare-all", "--die-with-parent", "--"]
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.mcp_servers.len(), 1);
    let sandbox = cfg.mcp_servers[0]
        .sandbox
        .as_ref()
        .expect("sandbox block must be Some");
    assert_eq!(sandbox.wrapper, "bwrap");
    assert!(sandbox.args.iter().any(|a| a == "--unshare-all"));
    drop(env);
}

#[test]
fn mcp_server_sandbox_empty_wrapper_is_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("mcp-sandbox-empty-wrapper");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[mcp_server]]
name = "broken"
command = "/usr/local/bin/mcp"

[mcp_server.sandbox]
wrapper = "   "
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("empty wrapper must fail");
    let msg = err.to_string();
    assert!(msg.contains("sandbox.wrapper"), "error must name field: {msg}");
    drop(env);
}

#[test]
fn mcp_server_sandbox_on_sse_transport_is_error() {
    // SSE has no local child to wrap; declaring a sandbox on it
    // is operator confusion the loader should call out.
    let env = EnvScope::new();
    let tmp = TempDir::new("mcp-sandbox-sse");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[mcp_server]]
name = "remote"
transport = "sse"
url = "http://localhost:9000"

[mcp_server.sandbox]
wrapper = "bwrap"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("sandbox on SSE transport must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("stdio-only") || msg.contains("sandbox"),
        "error must explain the stdio-only constraint: {msg}",
    );
    drop(env);
}

#[test]
fn mcp_server_without_sandbox_is_none() {
    let env = EnvScope::new();
    let tmp = TempDir::new("mcp-no-sandbox");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[mcp_server]]
name = "plain"
command = "/usr/local/bin/mcp"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert!(
        cfg.mcp_servers[0].sandbox.is_none(),
        "omitting [mcp_server.sandbox] must yield None",
    );
    drop(env);
}

#[test]
fn mcp_server_stdio_missing_command_is_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("mcp-stdio-no-cmd");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[mcp_server]]
name = "broken"
transport = "stdio"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("stdio without command must fail");
    let msg = err.to_string();
    assert!(msg.contains("command"), "error must mention command: {msg}");

    drop(env);
}

// ------------------------------------------------------------------
// Phase 25 Task 3 — provider selection + OpenAI config
// ------------------------------------------------------------------

#[test]
fn provider_defaults_to_anthropic() {
    let env = EnvScope::new();
    let opts = LoadOptions::test_env_only();
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.provider.value, ProviderKind::Anthropic);
    assert_eq!(cfg.provider.source, FieldSource::Default);
    drop(env);
}

#[test]
fn provider_from_env_var() {
    let env = EnvScope::new();
    env.set("AIVYX_PROVIDER", "openai");
    let opts = LoadOptions::test_env_only();
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.provider.value, ProviderKind::OpenAi);
    assert_eq!(cfg.provider.source, FieldSource::Env);
    drop(env);
}

#[test]
fn provider_invalid_env_var_is_error() {
    let env = EnvScope::new();
    env.set("AIVYX_PROVIDER", "gemini");
    let opts = LoadOptions::test_env_only();
    let err = AivyxConfig::load_from_env_and_toml(&opts).unwrap_err();
    assert!(matches!(err, ConfigError::Invalid { field: "provider", .. }));
    drop(env);
}

#[test]
fn provider_from_toml() {
    let env = EnvScope::new();
    let tmp = TempDir::new("provider-toml");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[agent]
provider = "openai"

[openai]
api_key = "sk-openai-test"
base_url = "http://localhost:11434/v1"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.provider.value, ProviderKind::OpenAi);
    assert_eq!(cfg.provider.source, FieldSource::Toml);
    assert!(cfg.openai_api_key.is_some());
    assert_eq!(cfg.openai_api_key.as_ref().unwrap().source, FieldSource::Toml);
    let base_url = cfg.openai_base_url.as_ref().expect("base_url set");
    assert_eq!(base_url.value, "http://localhost:11434/v1");
    assert_eq!(base_url.source, FieldSource::Toml);
    drop(env);
}

#[test]
fn openai_api_key_from_env_overrides_toml() {
    let env = EnvScope::new();
    env.set("AIVYX_OPENAI_API_KEY", "sk-env-wins");
    let tmp = TempDir::new("openai-env-over-toml");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[openai]
api_key = "sk-toml-loses"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.openai_api_key.as_ref().unwrap().source, FieldSource::Env);
    drop(env);
}

#[test]
fn validate_requires_openai_key_when_provider_is_openai() {
    let env = EnvScope::new();
    env.set("AIVYX_PROVIDER", "openai");
    let opts = LoadOptions {
        toml_path: None,
        require_api_key: true,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let err = cfg.validate(&opts).unwrap_err();
    assert!(matches!(err, ConfigError::Missing { field: "openai_api_key" }));
    drop(env);
}

#[test]
fn validate_does_not_require_anthropic_key_when_provider_is_openai() {
    let env = EnvScope::new();
    env.set("AIVYX_PROVIDER", "openai");
    env.set("AIVYX_OPENAI_API_KEY", "sk-test");
    let opts = LoadOptions {
        toml_path: None,
        require_api_key: true,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    cfg.validate(&opts).expect("should pass — openai key present");
    drop(env);
}

// ---- Ollama provider tests ----

#[test]
fn ollama_provider_from_env() {
    let env = EnvScope::new();
    env.set("AIVYX_PROVIDER", "ollama");
    let opts = LoadOptions {
        toml_path: None,
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.provider.value, ProviderKind::Ollama);
    assert_eq!(cfg.provider.source, FieldSource::Env);
    drop(env);
}

#[test]
fn ollama_provider_from_toml() {
    let env = EnvScope::new();
    let tmp = TempDir::new("ollama-toml");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[agent]
provider = "ollama"
model = "llama3.1"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.provider.value, ProviderKind::Ollama);
    assert_eq!(cfg.model.value, "llama3.1");
    drop(env);
}

#[test]
fn ollama_validate_does_not_require_api_key() {
    let env = EnvScope::new();
    env.set("AIVYX_PROVIDER", "ollama");
    let opts = LoadOptions {
        toml_path: None,
        require_api_key: true,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    cfg.validate(&opts)
        .expect("ollama must not require an API key even with require_api_key=true");
    drop(env);
}

#[test]
fn ollama_accepts_optional_api_key() {
    let env = EnvScope::new();
    env.set("AIVYX_PROVIDER", "ollama");
    env.set("AIVYX_OPENAI_API_KEY", "sk-optional");
    let opts = LoadOptions {
        toml_path: None,
        require_api_key: true,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    cfg.validate(&opts).expect("ollama with optional key must pass");
    assert!(cfg.openai_api_key.is_some());
    drop(env);
}

#[test]
fn provider_kind_is_openai_compatible() {
    assert!(!ProviderKind::Anthropic.is_openai_compatible());
    assert!(ProviderKind::OpenAi.is_openai_compatible());
    assert!(ProviderKind::Ollama.is_openai_compatible());
}

#[test]
fn provider_kind_display() {
    assert_eq!(ProviderKind::Anthropic.to_string(), "anthropic");
    assert_eq!(ProviderKind::OpenAi.to_string(), "openai");
    assert_eq!(ProviderKind::Ollama.to_string(), "ollama");
}

// ------------------------------------------------------------------
// [daemon] web_ui / web_ui_port — Phase 39
// ------------------------------------------------------------------

#[test]
fn daemon_web_ui_true_yields_default_port() {
    let env = EnvScope::new();
    let tmp = TempDir::new("web-ui-true");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[daemon]
web_ui = true
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.web_ui_port, Some(7843));
    drop(env);
}

#[test]
fn daemon_web_ui_port_overrides_default() {
    let env = EnvScope::new();
    let tmp = TempDir::new("web-ui-port");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[daemon]
web_ui_port = 9999
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.web_ui_port, Some(9999));
    drop(env);
}

#[test]
fn daemon_web_ui_false_disables() {
    let env = EnvScope::new();
    let tmp = TempDir::new("web-ui-false");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[daemon]
web_ui = false
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.web_ui_port, None);
    drop(env);
}

#[test]
fn daemon_web_ui_absent_means_none() {
    let env = EnvScope::new();
    let tmp = TempDir::new("web-ui-absent");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(&toml_path, "").unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.web_ui_port, None);
    drop(env);
}

// ------------------------------------------------------------------
// Phase 46: `bundled` flag on [[mcp_server]]
// ------------------------------------------------------------------

#[test]
fn bundled_flag_parses() {
    let env = EnvScope::new();
    let tmp = TempDir::new("mcp-bundled");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[mcp_server]]
name = "web-search"
command = "aivyx"
args = ["mcp-server", "web-search"]
bundled = true
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.mcp_servers.len(), 1);
    assert!(cfg.mcp_servers[0].bundled);
    assert_eq!(cfg.mcp_servers[0].name, "web-search");
    drop(env);
}

#[test]
fn bundled_default_false() {
    let env = EnvScope::new();
    let tmp = TempDir::new("mcp-no-bundled");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[[mcp_server]]
name = "github"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.mcp_servers.len(), 1);
    assert!(!cfg.mcp_servers[0].bundled);
    drop(env);
}

// ------------------------------------------------------------------
// Profile (Phase 57 — PRODUCT.md P13)
// ------------------------------------------------------------------

#[test]
fn profile_section_populates_all_fields_with_toml_source() {
    let env = EnvScope::new();
    let tmp = TempDir::new("profile-full");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[profile]
assistant_name = "Codex"
operator_profile = "Senior Rust engineer focused on systems and AI agents."
communication_style = "terse, conclusion-first, three-bullet lists"
primary_use_cases = ["Rust systems programming", "AI agent design"]
behavioral_preferences = [
    "prefer integration tests over mocks",
    "always cite sources when summarizing",
]
behavioral_constraints = [
    "never autonomously commit code",
    "always confirm destructive shell commands",
]
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");

    assert_eq!(cfg.profile.assistant_name.value, "Codex");
    assert_eq!(cfg.profile.assistant_name.source, FieldSource::Toml);
    assert_eq!(
        cfg.profile.operator_profile.as_deref(),
        Some("Senior Rust engineer focused on systems and AI agents."),
    );
    assert_eq!(
        cfg.profile.communication_style.as_deref(),
        Some("terse, conclusion-first, three-bullet lists"),
    );
    assert_eq!(
        cfg.profile.primary_use_cases,
        vec![
            "Rust systems programming".to_string(),
            "AI agent design".to_string(),
        ],
    );
    assert_eq!(cfg.profile.behavioral_preferences.len(), 2);
    assert!(cfg
        .profile
        .behavioral_preferences
        .iter()
        .any(|s| s.contains("integration tests")));
    assert_eq!(cfg.profile.behavioral_constraints.len(), 2);
    assert!(cfg
        .profile
        .behavioral_constraints
        .iter()
        .any(|s| s.contains("autonomously commit code")));

    drop(env);
}

#[test]
fn profile_section_absent_synthesizes_default_with_assistant_name() {
    let env = EnvScope::new();
    let tmp = TempDir::new("profile-absent");
    let toml_path = tmp.path().join("aivyx.toml");
    // No [profile] section at all — legacy aivyx.toml shape.
    std::fs::write(
        &toml_path,
        r#"
[agent]
provider = "anthropic"
model = "claude-haiku-4-5-20251001"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");

    assert_eq!(cfg.profile.assistant_name.value, DEFAULT_ASSISTANT_NAME);
    assert_eq!(cfg.profile.assistant_name.source, FieldSource::Default);
    assert!(cfg.profile.operator_profile.is_none());
    assert!(cfg.profile.communication_style.is_none());
    assert!(cfg.profile.primary_use_cases.is_empty());
    assert!(cfg.profile.behavioral_preferences.is_empty());
    assert!(cfg.profile.behavioral_constraints.is_empty());

    drop(env);
}

#[test]
fn profile_section_partial_provides_some_defaults_some_toml() {
    let env = EnvScope::new();
    let tmp = TempDir::new("profile-partial");
    let toml_path = tmp.path().join("aivyx.toml");
    // Only assistant_name + primary_use_cases declared. The other
    // four fields must remain at their unset defaults.
    std::fs::write(
        &toml_path,
        r#"
[profile]
assistant_name = "Mira"
primary_use_cases = ["personal-finance analysis"]
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");

    // Declared fields carry FieldSource::Toml.
    assert_eq!(cfg.profile.assistant_name.value, "Mira");
    assert_eq!(cfg.profile.assistant_name.source, FieldSource::Toml);
    assert_eq!(
        cfg.profile.primary_use_cases,
        vec!["personal-finance analysis".to_string()],
    );

    // Undeclared fields stay at default — Option::None for the
    // two free-text fields, empty Vec for the two list fields.
    assert!(cfg.profile.operator_profile.is_none());
    assert!(cfg.profile.communication_style.is_none());
    assert!(cfg.profile.behavioral_preferences.is_empty());
    assert!(cfg.profile.behavioral_constraints.is_empty());

    drop(env);
}

// ==============================================================
// Phase 62 Task 3 — `[[notify_target]]` entries
// ==============================================================

#[test]
fn notify_target_entries_parse_telegram_and_webhook() {
    let env = EnvScope::new();
    let tmp = TempDir::new("notify-cfg");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "phone"
kind = "telegram"
chat_id = "123456789"

[[notify_target]]
name = "ops-alerts"
kind = "webhook"
url = "https://ntfy.sh/aivyx-personal-2026"

[[notify_target]]
name = "disabled-one"
kind = "telegram"
chat_id = "987654321"
enabled = false
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");

    assert_eq!(cfg.notify_targets.len(), 2, "disabled target filtered out");

    let phone = &cfg.notify_targets[0];
    assert_eq!(phone.name, "phone");
    assert!(phone.enabled);
    match &phone.kind {
        NotifyTargetKind::Telegram { chat_id } => assert_eq!(chat_id, "123456789"),
        other => panic!("expected Telegram, got {other:?}"),
    }

    let webhook = &cfg.notify_targets[1];
    assert_eq!(webhook.name, "ops-alerts");
    match &webhook.kind {
        NotifyTargetKind::Webhook { url } => {
            assert_eq!(url, "https://ntfy.sh/aivyx-personal-2026");
        }
        other => panic!("expected Webhook, got {other:?}"),
    }

    drop(env);
}

#[test]
fn no_notify_target_section_gives_empty_vec() {
    let env = EnvScope::new();
    let tmp = TempDir::new("no-notify");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert!(cfg.notify_targets.is_empty());
    drop(env);
}

#[test]
fn notify_target_telegram_missing_chat_id_is_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("notify-bad-telegram");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "phone"
kind = "telegram"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "notify_target.chat_id");
            assert!(
                reason.contains("requires `chat_id`"),
                "reason was: {reason}"
            );
            assert!(reason.contains("phone"), "reason should name target: {reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn notify_target_webhook_missing_url_is_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("notify-bad-webhook");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "alerts"
kind = "webhook"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    assert!(matches!(err, ConfigError::Invalid { field, .. } if field == "notify_target.url"));
    drop(env);
}

#[test]
fn notify_target_webhook_rejects_non_http_url() {
    let env = EnvScope::new();
    let tmp = TempDir::new("notify-bad-scheme");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "weird"
kind = "webhook"
url = "ftp://example.com/notify"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "notify_target.url");
            assert!(
                reason.contains("must start with http:// or https://"),
                "reason was: {reason}"
            );
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn notify_target_unknown_kind_is_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("notify-bad-kind");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "phone"
kind = "signal"
chat_id = "x"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "notify_target.kind");
            assert!(
                reason.contains("unknown notify_target kind"),
                "reason was: {reason}"
            );
            assert!(reason.contains("signal"));
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn notify_target_duplicate_names_are_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("notify-dup");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "phone"
kind = "telegram"
chat_id = "1"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "notify_target.name");
            assert!(
                reason.contains("duplicate"),
                "reason was: {reason}"
            );
            assert!(reason.contains("phone"));
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn notify_target_empty_name_is_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("notify-empty-name");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = ""
kind = "webhook"
url = "https://example.com/x"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    assert!(matches!(err, ConfigError::Invalid { field, .. } if field == "notify_target.name"));
    drop(env);
}

// ==============================================================
// Phase 63 Task 2 — trigger.notify_target field + cross-validation
// ==============================================================

#[test]
fn schedule_notify_target_loads_when_role_has_capability() {
    let env = EnvScope::new();
    let tmp = TempDir::new("schedule-notify-ok");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send"]
trust_ceiling = "Trusted"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"

[[schedule]]
name = "morning-summary"
cron = "0 0 9 * * * *"
prompt = "Summarize my day"
notify_target = "phone"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.schedules.len(), 1);
    assert_eq!(cfg.schedules[0].notify_target.as_deref(), Some("phone"));
    drop(env);
}

#[test]
fn schedule_notify_target_unknown_target_is_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("schedule-notify-unknown");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send"]
trust_ceiling = "Trusted"

[[schedule]]
name = "morning-summary"
cron = "0 0 9 * * * *"
prompt = "Summarize my day"
notify_target = "phone"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "schedule.notify_targets");
            assert!(reason.contains("unknown notify_target"), "reason: {reason}");
            assert!(reason.contains("phone"), "reason: {reason}");
            assert!(reason.contains("morning-summary"), "reason: {reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn schedule_notify_target_role_lacks_capability_is_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("schedule-notify-noscope");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["memory.read"]
trust_ceiling = "Trusted"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"

[[schedule]]
name = "morning-summary"
cron = "0 0 9 * * * *"
prompt = "Summarize my day"
notify_target = "phone"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "schedule.notify_targets");
            assert!(reason.contains("lacks `notify.send`"), "reason: {reason}");
            assert!(reason.contains("default"), "reason: {reason}");
            assert!(reason.contains("morning-summary"), "reason: {reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn schedule_notify_target_qualified_scope_grants_named_target() {
    // Role declares `notify.send:phone` (qualified). The
    // schedule with notify_target = "phone" passes; if the
    // schedule named a different target it would fail.
    let env = EnvScope::new();
    let tmp = TempDir::new("schedule-notify-qualified");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send:phone"]
trust_ceiling = "Trusted"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"

[[schedule]]
name = "morning-summary"
cron = "0 0 9 * * * *"
prompt = "Summarize my day"
notify_target = "phone"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.schedules[0].notify_target.as_deref(), Some("phone"));
    drop(env);
}

#[test]
fn schedule_notify_target_semitrusted_role_is_error() {
    // notify.send is in CEILING_TRUSTED only; a SemiTrusted
    // role declaring notify.send loses it after intersection.
    let env = EnvScope::new();
    let tmp = TempDir::new("schedule-notify-semi");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send"]
trust_ceiling = "SemiTrusted"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"

[[schedule]]
name = "morning-summary"
cron = "0 0 9 * * * *"
prompt = "Summarize my day"
notify_target = "phone"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    assert!(matches!(err, ConfigError::Invalid { field, .. } if field == "schedule.notify_targets"));
    drop(env);
}

#[test]
fn webhook_notify_target_validated_the_same_way() {
    let env = EnvScope::new();
    let tmp = TempDir::new("webhook-notify");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send"]
trust_ceiling = "Trusted"

[[notify_target]]
name = "ops"
kind = "webhook"
url = "https://example.com/x"

[[webhook]]
name = "ci-events"
prompt = "Process CI event"
notify_target = "ops"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.webhooks[0].notify_target.as_deref(), Some("ops"));
    drop(env);
}

#[test]
fn file_watch_notify_target_validated_the_same_way() {
    let env = EnvScope::new();
    let tmp = TempDir::new("filewatch-notify");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send"]
trust_ceiling = "Trusted"

[[notify_target]]
name = "alerts"
kind = "webhook"
url = "https://example.com/x"

[[file_watch]]
name = "notes-dir"
path = "/tmp/notes"
prompt = "React to note change"
notify_target = "alerts"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.file_watches[0].notify_target.as_deref(), Some("alerts"));
    drop(env);
}

#[test]
fn trigger_without_notify_target_loads_normally() {
    // Phase 63 doesn't change behavior for triggers that don't
    // opt in to notify_target. Regression test.
    let env = EnvScope::new();
    let tmp = TempDir::new("no-notify-target");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[schedule]]
name = "plain-old-schedule"
cron = "0 0 9 * * * *"
prompt = "Do the thing"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.schedules[0].notify_target, None);
    drop(env);
}

#[test]
fn notify_send_via_parent_role_grants_inherited_capability() {
    // Inheritance: child role doesn't declare notify.send, but
    // its parent does. Should be granted via the parent chain.
    let env = EnvScope::new();
    let tmp = TempDir::new("notify-inherited");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send"]
trust_ceiling = "Trusted"

[[role]]
name = "child"
capability_scopes = []
trust_ceiling = "Trusted"
parent_role = "default"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"

[[schedule]]
name = "morning-summary"
cron = "0 0 9 * * * *"
role = "child"
prompt = "Summarize my day"
notify_target = "phone"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.schedules[0].role, "child");
    assert_eq!(cfg.schedules[0].notify_target.as_deref(), Some("phone"));
    drop(env);
}

// ==============================================================
// Phase 68 — [email] section + kind = "email" notify_target
// ==============================================================

#[test]
fn email_section_with_kind_email_target_loads_cleanly() {
    use secrecy::ExposeSecret;
    let env = EnvScope::new();
    let tmp = TempDir::new("email-happy");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[email]
host = "smtp.fastmail.com"
username = "alice@example.com"
password = "app-password-xyz"
from = "aivyx@example.com"

[[notify_target]]
name = "self"
kind = "email"
to = "alice@example.com"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let email = cfg.email.expect("[email] populated");
    assert_eq!(email.host, "smtp.fastmail.com");
    assert_eq!(email.port, 587);
    assert_eq!(email.tls_mode, TlsMode::Starttls);
    assert_eq!(email.from, "aivyx@example.com");
    assert_eq!(email.password.value.expose_secret(), "app-password-xyz");
    assert_eq!(cfg.notify_targets.len(), 1);
    match &cfg.notify_targets[0].kind {
        NotifyTargetKind::Email { to } => assert_eq!(to, "alice@example.com"),
        other => panic!("expected Email, got {other:?}"),
    }
    drop(env);
}

#[test]
fn email_kind_target_without_email_section_is_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("email-no-section");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "self"
kind = "email"
to = "alice@example.com"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "notify_target.to");
            assert!(
                reason.contains("[email] section"),
                "reason: {reason}"
            );
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn email_tls_mode_none_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("email-tls-none");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[email]
host = "smtp.example.com"
tls_mode = "none"
username = "u"
password = "p"
from = "a@b.c"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "email.tls_mode");
            assert!(reason.contains("cleartext"), "reason: {reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn email_implicit_tls_picks_port_465_default() {
    let env = EnvScope::new();
    let tmp = TempDir::new("email-implicit");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[email]
host = "smtp.example.com"
tls_mode = "implicit"
username = "u"
password = "p"
from = "a@example.com"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let email = cfg.email.expect("[email] populated");
    assert_eq!(email.port, 465);
    assert_eq!(email.tls_mode, TlsMode::Implicit);
    drop(env);
}

#[test]
fn email_explicit_port_override_wins() {
    let env = EnvScope::new();
    let tmp = TempDir::new("email-explicit-port");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[email]
host = "smtp.example.com"
port = 2525
username = "u"
password = "p"
from = "a@example.com"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.email.unwrap().port, 2525);
    drop(env);
}

#[test]
fn email_section_partial_config_is_error() {
    // [email] declared with host but no password.
    let env = EnvScope::new();
    let tmp = TempDir::new("email-partial");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[email]
host = "smtp.example.com"
username = "u"
from = "a@example.com"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    assert!(matches!(err, ConfigError::Invalid { field, .. } if field == "email.password"));
    drop(env);
}

#[test]
fn email_from_without_at_sign_is_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("email-bad-from");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[email]
host = "smtp.example.com"
username = "u"
password = "p"
from = "notanemail"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    assert!(matches!(err, ConfigError::Invalid { field, .. } if field == "email.from"));
    drop(env);
}

#[test]
fn email_to_without_at_sign_is_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("email-bad-to");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[email]
host = "smtp.example.com"
username = "u"
password = "p"
from = "a@example.com"

[[notify_target]]
name = "broken"
kind = "email"
to = "no-at-sign"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    assert!(matches!(err, ConfigError::Invalid { field, .. } if field == "notify_target.to"));
    drop(env);
}

#[test]
fn email_unknown_tls_mode_is_error() {
    let env = EnvScope::new();
    let tmp = TempDir::new("email-unknown-tls");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[email]
host = "smtp.example.com"
tls_mode = "bogus"
username = "u"
password = "p"
from = "a@example.com"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "email.tls_mode");
            assert!(reason.contains("bogus"), "reason: {reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

// ==============================================================
// Phase 69 — kind = "web-ui" notify_target
// ==============================================================

#[test]
fn web_ui_notify_target_parses_with_no_extra_fields() {
    let env = EnvScope::new();
    let tmp = TempDir::new("webui-target");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "desktop"
kind = "web-ui"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.notify_targets.len(), 1);
    assert_eq!(cfg.notify_targets[0].name, "desktop");
    assert!(matches!(
        cfg.notify_targets[0].kind,
        NotifyTargetKind::WebUi
    ));
    drop(env);
}

#[test]
fn unknown_notify_target_kind_error_mentions_web_ui() {
    // Regression: the helpful "supported kinds" list in the
    // unknown-kind error message must include web-ui.
    let env = EnvScope::new();
    let tmp = TempDir::new("notify-unknown-kind");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "bogus"
kind = "carrier-pigeon"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { reason, .. } => {
            assert!(reason.contains("web-ui"), "reason: {reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

// ==============================================================
// Phase 70 — [[reflection_schedule]] config block
// ==============================================================

#[test]
fn reflection_schedule_with_defaults_parses() {
    let env = EnvScope::new();
    let tmp = TempDir::new("reflection-default");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[reflection_schedule]]
name = "nightly"
cron = "0 0 23 * * *"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.reflection_schedules.len(), 1);
    let s = &cfg.reflection_schedules[0];
    assert_eq!(s.name, "nightly");
    assert_eq!(s.cron, "0 0 23 * * *");
    assert_eq!(s.lookback_window_secs, 86400); // 24h default
    assert!(s.role_override.is_none());
    assert!(s.enabled);
    drop(env);
}

#[test]
fn reflection_schedule_disabled_entries_are_skipped() {
    let env = EnvScope::new();
    let tmp = TempDir::new("reflection-disabled");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[reflection_schedule]]
name = "nightly"
cron = "0 0 23 * * *"
enabled = false
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert!(cfg.reflection_schedules.is_empty());
    drop(env);
}

#[test]
fn reflection_schedule_empty_cron_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("reflection-empty-cron");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[reflection_schedule]]
name = "nightly"
cron = ""
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "reflection_schedule.cron");
            assert!(reason.contains("nightly"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn reflection_schedule_lookback_below_min_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("reflection-lookback-low");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[reflection_schedule]]
name = "fast"
cron = "* * * * * *"
lookback_window_secs = 30
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "reflection_schedule.lookback_window_secs");
            assert!(reason.contains("fast"), "{reason}");
            assert!(reason.contains("60"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn reflection_schedule_lookback_above_max_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("reflection-lookback-high");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[reflection_schedule]]
name = "slow"
cron = "0 0 * * * *"
lookback_window_secs = 999999999
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "reflection_schedule.lookback_window_secs");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn reflection_schedule_duplicate_name_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("reflection-dup");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[reflection_schedule]]
name = "nightly"
cron = "0 0 23 * * *"

[[reflection_schedule]]
name = "nightly"
cron = "0 0 1 * * *"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "reflection_schedule.name");
            assert!(reason.contains("duplicate"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn reflection_schedule_collision_with_schedule_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("reflection-vs-schedule");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[schedule]]
name = "nightly"
cron = "0 0 23 * * *"
prompt = "do stuff"

[[reflection_schedule]]
name = "nightly"
cron = "0 0 1 * * *"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "reflection_schedule.name");
            assert!(reason.contains("collides"), "{reason}");
            assert!(reason.contains("[[schedule]]"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn reflection_schedule_unknown_role_override_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("reflection-bad-role");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[reflection_schedule]]
name = "nightly"
cron = "0 0 23 * * *"
role_override = "ghost-role"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "reflection_schedule.role_override");
            assert!(reason.contains("ghost-role"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

// ==============================================================
// Phase 72 — multi-target, default-target, conditional notify
// ==============================================================

#[test]
fn singular_notify_target_bridges_into_plural() {
    let env = EnvScope::new();
    let tmp = TempDir::new("singular-alias");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send"]
trust_ceiling = "Trusted"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"

[[schedule]]
name = "daily"
cron = "0 0 9 * * *"
prompt = "morning summary"
notify_target = "phone"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let sched = cfg
        .schedules
        .iter()
        .find(|s| s.name == "daily")
        .expect("schedule loaded");
    assert_eq!(sched.notify_targets, vec!["phone".to_string()]);
    assert_eq!(sched.notify_target.as_deref(), Some("phone"));
    assert_eq!(sched.notify_when, NotifyWhen::Always);
    drop(env);
}

#[test]
fn plural_notify_targets_loads_full_list() {
    let env = EnvScope::new();
    let tmp = TempDir::new("plural-list");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send"]
trust_ceiling = "Trusted"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/p"

[[notify_target]]
name = "desktop"
kind = "web-ui"

[[schedule]]
name = "daily"
cron = "0 0 9 * * *"
prompt = "morning summary"
notify_targets = ["phone", "desktop"]
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let sched = &cfg.schedules[0];
    assert_eq!(
        sched.notify_targets,
        vec!["phone".to_string(), "desktop".to_string()]
    );
    drop(env);
}

#[test]
fn both_singular_and_plural_declared_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("both-forms");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send"]
trust_ceiling = "Trusted"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"

[[schedule]]
name = "daily"
cron = "0 0 9 * * *"
prompt = "morning summary"
notify_target = "phone"
notify_targets = ["phone"]
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "trigger.notify_targets");
            assert!(reason.contains("both"), "{reason}");
            assert!(reason.contains("daily"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn default_target_resolves_into_empty_trigger_list() {
    let env = EnvScope::new();
    let tmp = TempDir::new("default-resolve");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send"]
trust_ceiling = "Trusted"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"
default = true

[[schedule]]
name = "daily"
cron = "0 0 9 * * *"
prompt = "morning summary"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    // The default flag flows through to NotifyTargetConfig.
    let phone = &cfg.notify_targets[0];
    assert!(phone.is_default);
    // The schedule's empty notify_targets gets filled with the
    // default at load time.
    let sched = &cfg.schedules[0];
    assert_eq!(sched.notify_targets, vec!["phone".to_string()]);
    drop(env);
}

#[test]
fn default_target_does_not_overwrite_explicit_list() {
    let env = EnvScope::new();
    let tmp = TempDir::new("default-no-overwrite");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send"]
trust_ceiling = "Trusted"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/p"
default = true

[[notify_target]]
name = "desktop"
kind = "web-ui"

[[schedule]]
name = "daily"
cron = "0 0 9 * * *"
prompt = "morning summary"
notify_targets = ["desktop"]
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let sched = &cfg.schedules[0];
    // The explicit list survives unchanged — default doesn't merge.
    assert_eq!(sched.notify_targets, vec!["desktop".to_string()]);
    drop(env);
}

#[test]
fn multiple_default_targets_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("dup-default");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send"]
trust_ceiling = "Trusted"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/p"
default = true

[[notify_target]]
name = "desktop"
kind = "web-ui"
default = true
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "notify_target.default");
            assert!(reason.contains("multiple"), "{reason}");
            assert!(reason.contains("phone"), "{reason}");
            assert!(reason.contains("desktop"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn notify_when_variants_parse() {
    for (input, expected) in [
        ("always", NotifyWhen::Always),
        ("on_failed", NotifyWhen::OnFailed),
        ("on_completed_non_empty", NotifyWhen::OnCompletedNonEmpty),
    ] {
        let env = EnvScope::new();
        let tmp = TempDir::new(&format!("notify-when-{input}"));
        let toml_path = tmp.path().join("aivyx.toml");
        std::fs::write(
            &toml_path,
            format!(
                r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send"]
trust_ceiling = "Trusted"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"

[[schedule]]
name = "daily"
cron = "0 0 9 * * *"
prompt = "morning"
notify_targets = ["phone"]
notify_when = "{input}"
"#,
            ),
        )
        .unwrap();
        let opts = LoadOptions {
            toml_path: Some(toml_path),
            require_api_key: false,
            require_telegram_token: false,
            role_override: None,
        };
        let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
        assert_eq!(cfg.schedules[0].notify_when, expected);
        drop(env);
    }
}

#[test]
fn notify_when_unknown_value_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("notify-when-bad");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send"]
trust_ceiling = "Trusted"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"

[[schedule]]
name = "daily"
cron = "0 0 9 * * *"
prompt = "morning"
notify_targets = ["phone"]
notify_when = "if_blue_moon"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "trigger.notify_when");
            assert!(reason.contains("if_blue_moon"), "{reason}");
            assert!(reason.contains("always"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn multi_target_with_one_unknown_name_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("multi-unknown");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[role]]
name = "default"
capability_scopes = ["notify.send"]
trust_ceiling = "Trusted"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"

[[schedule]]
name = "daily"
cron = "0 0 9 * * *"
prompt = "morning"
notify_targets = ["phone", "ghost"]
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "schedule.notify_targets");
            assert!(reason.contains("ghost"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

// ==============================================================
// Phase 73 — retry + rate-limit fields on notify_target
// ==============================================================

#[test]
fn retry_fields_default_to_zero_count_and_default_backoff() {
    let env = EnvScope::new();
    let tmp = TempDir::new("retry-defaults");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let t = &cfg.notify_targets[0];
    assert_eq!(t.retry_count, 0);
    assert_eq!(t.retry_backoff_ms_start, 500); // DEFAULT_RETRY_BACKOFF_MS_START
    assert!(t.rate_limit_max.is_none());
    assert!(t.rate_limit_window_secs.is_none());
    drop(env);
}

#[test]
fn retry_count_above_cap_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("retry-too-high");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"
retry_count = 100
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "notify_target.retry_count");
            assert!(reason.contains("100"), "{reason}");
            assert!(reason.contains("hard cap"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn retry_backoff_below_floor_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("backoff-too-low");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"
retry_count = 3
retry_backoff_ms_start = 50
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "notify_target.retry_backoff_ms_start");
            assert!(reason.contains("50"), "{reason}");
            assert!(reason.contains("100"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn retry_explicit_values_parse() {
    let env = EnvScope::new();
    let tmp = TempDir::new("retry-explicit");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"
retry_count = 5
retry_backoff_ms_start = 200
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let t = &cfg.notify_targets[0];
    assert_eq!(t.retry_count, 5);
    assert_eq!(t.retry_backoff_ms_start, 200);
    drop(env);
}

#[test]
fn rate_limit_partial_max_without_window_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("rate-no-window");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"
rate_limit_max = 10
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "notify_target.rate_limit_window_secs");
            assert!(reason.contains("both"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn rate_limit_partial_window_without_max_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("rate-no-max");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"
rate_limit_window_secs = 3600
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "notify_target.rate_limit_max");
            assert!(reason.contains("both"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn rate_limit_zero_max_is_rejected() {
    let env = EnvScope::new();
    let tmp = TempDir::new("rate-zero-max");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"
rate_limit_max = 0
rate_limit_window_secs = 60
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "notify_target.rate_limit_max");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn rate_limit_both_set_parses() {
    let env = EnvScope::new();
    let tmp = TempDir::new("rate-both");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[notify_target]]
name = "phone"
kind = "webhook"
url = "https://example.com/x"
rate_limit_max = 10
rate_limit_window_secs = 3600
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let t = &cfg.notify_targets[0];
    assert_eq!(t.rate_limit_max, Some(10));
    assert_eq!(t.rate_limit_window_secs, Some(3600));
    drop(env);
}

// ==============================================================
// Phase 74 — [[memory.retention]] config blocks
// ==============================================================

#[test]
fn memory_retention_forever_parses() {
    let env = EnvScope::new();
    let tmp = TempDir::new("retention-forever");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[memory.retention]]
topic_glob = "project/*"
retention = "forever"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.memory_retention.len(), 1);
    let rule = &cfg.memory_retention[0];
    assert_eq!(rule.topic_glob, "project/*");
    assert!(matches!(rule.retention, crate::RetentionPolicy::Forever));
    // Glob matcher works as expected.
    assert!(rule.matcher.is_match("project/x"));
    assert!(!rule.matcher.is_match("notes/today"));
    drop(env);
}

#[test]
fn memory_retention_days_parses() {
    let env = EnvScope::new();
    let tmp = TempDir::new("retention-days");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[memory.retention]]
topic_glob = "notes/*"
retention_days = 30
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.memory_retention.len(), 1);
    assert!(matches!(
        cfg.memory_retention[0].retention,
        crate::RetentionPolicy::ForDays(30)
    ));
    drop(env);
}

#[test]
fn memory_retention_multiple_rules_preserve_first_match_order() {
    let env = EnvScope::new();
    let tmp = TempDir::new("retention-multi");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[memory.retention]]
topic_glob = "project/critical/*"
retention = "forever"

[[memory.retention]]
topic_glob = "project/*"
retention_days = 90

[[memory.retention]]
topic_glob = "notes/*"
retention_days = 30
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert_eq!(cfg.memory_retention.len(), 3);
    // Order preserved — operator put narrower glob first.
    assert_eq!(cfg.memory_retention[0].topic_glob, "project/critical/*");
    assert_eq!(cfg.memory_retention[1].topic_glob, "project/*");
    assert_eq!(cfg.memory_retention[2].topic_glob, "notes/*");
    drop(env);
}

#[test]
fn memory_retention_empty_glob_rejects() {
    let env = EnvScope::new();
    let tmp = TempDir::new("retention-empty-glob");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[memory.retention]]
topic_glob = ""
retention = "forever"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "memory.retention.topic_glob");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn memory_retention_unknown_retention_value_rejects() {
    let env = EnvScope::new();
    let tmp = TempDir::new("retention-bad-value");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[memory.retention]]
topic_glob = "project/*"
retention = "until-summer"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "memory.retention.retention");
            assert!(reason.contains("until-summer"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn memory_retention_zero_days_rejects() {
    let env = EnvScope::new();
    let tmp = TempDir::new("retention-zero-days");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[memory.retention]]
topic_glob = "notes/*"
retention_days = 0
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "memory.retention.retention_days");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn memory_retention_neither_form_rejects() {
    let env = EnvScope::new();
    let tmp = TempDir::new("retention-no-policy");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[memory.retention]]
topic_glob = "notes/*"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "memory.retention");
            assert!(reason.contains("must declare"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn memory_retention_both_forms_rejects() {
    let env = EnvScope::new();
    let tmp = TempDir::new("retention-both-forms");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[memory.retention]]
topic_glob = "notes/*"
retention = "forever"
retention_days = 30
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "memory.retention");
            assert!(reason.contains("both"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

#[test]
fn memory_retention_invalid_glob_rejects() {
    let env = EnvScope::new();
    let tmp = TempDir::new("retention-bad-glob");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[anthropic]
api_key = "sk-test"

[[memory.retention]]
topic_glob = "[unclosed"
retention = "forever"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts).expect_err("must error");
    match err {
        ConfigError::Invalid { field, reason } => {
            assert_eq!(field, "memory.retention.topic_glob");
            assert!(reason.contains("not a valid glob"), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

// ------------------------------------------------------------------
// Phase 75 — [embedding] section
// ------------------------------------------------------------------

/// No `[embedding]` section → `embedding: None`. Semantic search
/// is disabled; pre-Phase-75 configs are unaffected.
#[test]
fn embedding_absent_section_is_none() {
    let env = EnvScope::new();
    let cfg = AivyxConfig::load_from_env_and_toml(&LoadOptions::test_env_only())
        .expect("load");
    assert!(cfg.embedding.is_none());
    drop(env);
}

/// A `[embedding]` section with only the api_key set: the three
/// non-secret fields fall back to the `DEFAULT_EMBEDDING_*`
/// constants, the key is `FieldSource::Toml`.
#[test]
fn embedding_partial_section_applies_defaults() {
    let env = EnvScope::new();
    env.clear("AIVYX_EMBEDDING_API_KEY");
    let tmp = TempDir::new("embedding-defaults");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[embedding]
api_key = "sk-emb-toml"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let emb = cfg.embedding.expect("section present");
    assert_eq!(emb.base_url, crate::DEFAULT_EMBEDDING_BASE_URL);
    assert_eq!(emb.model, crate::DEFAULT_EMBEDDING_MODEL);
    assert_eq!(emb.dimensions, crate::DEFAULT_EMBEDDING_DIMENSIONS);
    // Phase 76 — RAG knobs default when unspecified.
    assert_eq!(emb.rag_top_k, crate::DEFAULT_RAG_TOP_K);
    assert_eq!(
        emb.rag_min_similarity,
        crate::DEFAULT_RAG_MIN_SIMILARITY
    );
    // Phase 86 — recall-window default is 1 (= byte-identical
    // to pre-Phase-86 single-message behaviour).
    assert_eq!(
        emb.recall_window_turns,
        crate::DEFAULT_RECALL_WINDOW_TURNS
    );
    assert_eq!(emb.recall_window_turns, 1);
    let key = emb.api_key.expect("key from toml");
    assert_eq!(key.source, FieldSource::Toml);
    assert_eq!(key.value.expose_secret(), "sk-emb-toml");
    drop(env);
}

/// Explicit values override every default; a local base_url
/// keeps embedding on-device.
#[test]
fn embedding_explicit_fields_win() {
    let env = EnvScope::new();
    env.clear("AIVYX_EMBEDDING_API_KEY");
    let tmp = TempDir::new("embedding-explicit");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[embedding]
base_url = "http://localhost:11434"
model = "nomic-embed-text"
dimensions = 768
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let emb = cfg.embedding.expect("section present");
    assert_eq!(emb.base_url, "http://localhost:11434");
    assert_eq!(emb.model, "nomic-embed-text");
    assert_eq!(emb.dimensions, 768);
    // No key set anywhere — a local server needs none.
    assert!(emb.api_key.is_none());
    drop(env);
}

/// `AIVYX_EMBEDDING_API_KEY` beats the TOML `api_key` (env >
/// TOML), matching the anthropic / openai key precedence.
#[test]
fn embedding_env_key_beats_toml_key() {
    let env = EnvScope::new();
    env.set("AIVYX_EMBEDDING_API_KEY", "sk-emb-env");
    let tmp = TempDir::new("embedding-env-wins");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[embedding]
api_key = "sk-emb-toml-loses"
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let key = cfg.embedding.unwrap().api_key.expect("key present");
    assert_eq!(key.source, FieldSource::Env);
    assert_eq!(key.value.expose_secret(), "sk-emb-env");
    drop(env);
}

/// A blank `base_url` is a load-time `Invalid`.
#[test]
fn embedding_blank_base_url_is_invalid() {
    let env = EnvScope::new();
    let tmp = TempDir::new("embedding-blank-url");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[embedding]
base_url = "   "
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "embedding.base_url");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

/// `dimensions = 0` is a load-time `Invalid`.
#[test]
fn embedding_zero_dimensions_is_invalid() {
    let env = EnvScope::new();
    let tmp = TempDir::new("embedding-zero-dims");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[embedding]
dimensions = 0
"#,
    )
    .unwrap();

    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "embedding.dimensions");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

/// Phase 76 — explicit RAG knobs override the defaults.
#[test]
fn embedding_rag_knobs_explicit_win() {
    let env = EnvScope::new();
    env.clear("AIVYX_EMBEDDING_API_KEY");
    let tmp = TempDir::new("embedding-rag-explicit");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[embedding]
base_url = "http://localhost:11434"
rag_top_k = 12
rag_min_similarity = 0.55
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    let emb = cfg.embedding.expect("section present");
    assert_eq!(emb.rag_top_k, 12);
    assert!((emb.rag_min_similarity - 0.55).abs() < 1e-6);
}

/// Phase 76 — `rag_top_k = 0` is a load-time `Invalid`.
#[test]
fn embedding_rag_top_k_zero_is_invalid() {
    let env = EnvScope::new();
    let tmp = TempDir::new("embedding-rag-topk-zero");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[embedding]
rag_top_k = 0
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "embedding.rag_top_k");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

/// Phase 76 — `rag_min_similarity` outside `[0.0, 1.0]` is a
/// load-time `Invalid`.
#[test]
fn embedding_rag_min_similarity_out_of_range_is_invalid() {
    let env = EnvScope::new();
    let tmp = TempDir::new("embedding-rag-sim-oor");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[embedding]
rag_min_similarity = 1.5
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "embedding.rag_min_similarity");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

/// Phase 86 — explicit `recall_window_turns` wins; the
/// default-when-absent is asserted in
/// `embedding_partial_section_applies_defaults`.
#[test]
fn embedding_recall_window_turns_explicit_wins() {
    let env = EnvScope::new();
    let tmp = TempDir::new("embedding-window-explicit");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[embedding]
recall_window_turns = 5
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let cfg = AivyxConfig::load_from_env_and_toml(&opts)
        .expect("load");
    assert_eq!(
        cfg.embedding.expect("section").recall_window_turns,
        5
    );
    drop(env);
}

/// Phase 86 — `recall_window_turns = 0` is a load-time
/// `Invalid` (1 is the byte-identical-to-pre-Phase-86 floor).
#[test]
fn embedding_recall_window_turns_zero_is_invalid() {
    let env = EnvScope::new();
    let tmp = TempDir::new("embedding-window-zero");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[embedding]
recall_window_turns = 0
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(
                field,
                "embedding.recall_window_turns"
            );
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

/// Phase 90 — `recall_gate_min_chars` defaults to `0`
/// (gate disabled = byte-identical to pre-Phase-90).
#[test]
fn embedding_recall_gate_min_chars_default_is_zero() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[embedding]\nmodel = \"any\"\n",
        "embed-gate-default",
    );
    let emb = cfg.embedding.expect("section present");
    assert_eq!(
        emb.recall_gate_min_chars,
        crate::DEFAULT_RECALL_GATE_MIN_CHARS,
    );
    assert_eq!(emb.recall_gate_min_chars, 0);
    drop(env);
}

/// Phase 90 — explicit `recall_gate_min_chars` wins.
#[test]
fn embedding_recall_gate_min_chars_explicit_wins() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[embedding]\nrecall_gate_min_chars = 6\n",
        "embed-gate-explicit",
    );
    let emb = cfg.embedding.expect("section present");
    assert_eq!(emb.recall_gate_min_chars, 6);
    drop(env);
}

/// Phase 90 — explicit `recall_gate_min_chars = 0` is
/// honored (the operator can express the default explicitly
/// without changing behaviour). Any value is legal —
/// large thresholds gate aggressively, the operator's call.
#[test]
fn embedding_recall_gate_min_chars_explicit_zero_honored() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[embedding]\nrecall_gate_min_chars = 0\n",
        "embed-gate-explicit-zero",
    );
    let emb = cfg.embedding.expect("section present");
    assert_eq!(emb.recall_gate_min_chars, 0);
    drop(env);
}

/// Env + TOML do not supply the embedding key, but the
/// encrypted store has a `secret_keys::EMBEDDING_API_KEY` row.
/// After hydration the key is populated with
/// `FieldSource::EncryptedStore`.
#[tokio::test]
async fn embedding_key_hydrates_from_store() {
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{KeyDomain, RedbStorage, StorageConfig};

    let env = EnvScope::new();
    env.clear("AIVYX_EMBEDDING_API_KEY");

    let tmp = TempDir::new("embedding-store-hydrate");
    let store_path = tmp.path().join("store.redb");
    let master = MasterKey::from_raw([11u8; 32]);
    let storage = RedbStorage::open(StorageConfig::new(store_path), master)
        .await
        .expect("open store");
    let secrets = storage.domain(KeyDomain::Secrets);
    secrets
        .put(crate::secret_keys::EMBEDDING_API_KEY, b"sk-emb-from-store")
        .await
        .expect("put embedding key");

    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        r#"
[embedding]
model = "text-embedding-3-small"
"#,
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let mut cfg =
        AivyxConfig::load_from_env_and_toml(&opts).expect("load");
    assert!(cfg.embedding.as_ref().unwrap().api_key.is_none());

    cfg.hydrate_secrets_from_store(&storage)
        .await
        .expect("hydrate");
    let key = cfg
        .embedding
        .unwrap()
        .api_key
        .expect("hydrated from store");
    assert_eq!(key.source, FieldSource::EncryptedStore);
    assert_eq!(key.value.expose_secret(), "sk-emb-from-store");
    drop(env);
}

/// Hydration must not materialize an `EmbeddingConfig` when the
/// `[embedding]` section was absent, even if the store holds a
/// key row (mirrors the telegram-token rule).
#[tokio::test]
async fn embedding_store_key_without_section_stays_none() {
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{KeyDomain, RedbStorage, StorageConfig};

    let env = EnvScope::new();
    env.clear("AIVYX_EMBEDDING_API_KEY");
    let tmp = TempDir::new("embedding-no-section");
    let store_path = tmp.path().join("store.redb");
    let master = MasterKey::from_raw([12u8; 32]);
    let storage = RedbStorage::open(StorageConfig::new(store_path), master)
        .await
        .unwrap();
    storage
        .domain(KeyDomain::Secrets)
        .put(crate::secret_keys::EMBEDDING_API_KEY, b"orphan-key")
        .await
        .unwrap();

    let mut cfg =
        AivyxConfig::load_from_env_and_toml(&LoadOptions::test_env_only())
            .expect("load");
    cfg.hydrate_secrets_from_store(&storage).await.unwrap();
    assert!(cfg.embedding.is_none());
    drop(env);
}

// ------------------------------------------------------------------
// Phase 80 — [proactive] section
// ------------------------------------------------------------------

fn load_with_toml(body: &str, tag: &str) -> AivyxConfig {
    let tmp = TempDir::new(tag);
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(&toml_path, body).unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    AivyxConfig::load_from_env_and_toml(&opts).expect("load")
}

/// No `[proactive]` section → `proactive: None` (off; the
/// assistant never reaches out unprompted, pre-Phase-80).
#[test]
fn proactive_absent_section_is_none() {
    let env = EnvScope::new();
    let cfg = AivyxConfig::load_from_env_and_toml(
        &LoadOptions::test_env_only(),
    )
    .expect("load");
    assert!(cfg.proactive.is_none());
    drop(env);
}

/// A present-but-disabled section may be partial (staged
/// config): it builds with `enabled = false`, defaults
/// elsewhere, and is NOT validated.
#[test]
fn proactive_present_disabled_is_allowed_partial() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[proactive]\nenabled = false\n",
        "proactive-staged",
    );
    let p = cfg.proactive.expect("section present");
    assert!(!p.enabled);
    assert_eq!(p.target, "");
    assert_eq!(
        p.max_per_window,
        crate::DEFAULT_PROACTIVE_MAX_PER_WINDOW
    );
    assert_eq!(
        p.window_secs,
        crate::DEFAULT_PROACTIVE_WINDOW_SECS
    );
    // Signals default on.
    assert!(p.signals.ttl_expiry);
    assert!(p.signals.recall_cluster);
    assert!(p.signals.due_reminder);
    drop(env);
}

/// Enabled + valid: explicit fields win; an explicitly-off
/// signal is respected while the others default on.
#[test]
fn proactive_enabled_valid_with_signal_toggle() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[proactive]\nenabled = true\ntarget = \"ops\"\n\
         max_per_window = 5\nwindow_secs = 3600\n\
         signal_due_reminder = false\n",
        "proactive-valid",
    );
    let p = cfg.proactive.expect("section present");
    assert!(p.enabled);
    assert_eq!(p.target, "ops");
    assert_eq!(p.max_per_window, 5);
    assert_eq!(p.window_secs, 3600);
    assert!(p.signals.ttl_expiry);
    assert!(p.signals.recall_cluster);
    assert!(!p.signals.due_reminder);
    drop(env);
}

/// Enabled without a target → load-time `Invalid`.
#[test]
fn proactive_enabled_requires_target() {
    let env = EnvScope::new();
    let tmp = TempDir::new("proactive-no-target");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(&toml_path, "\n[proactive]\nenabled = true\n")
        .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "proactive.target");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

/// Enabled with `max_per_window = 0` → `Invalid`.
#[test]
fn proactive_enabled_zero_cap_is_invalid() {
    let env = EnvScope::new();
    let tmp = TempDir::new("proactive-zero-cap");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        "\n[proactive]\nenabled = true\ntarget = \"ops\"\n\
         max_per_window = 0\n",
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "proactive.max_per_window");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

/// Enabled with every signal class off → `Invalid`.
#[test]
fn proactive_enabled_all_signals_off_is_invalid() {
    let env = EnvScope::new();
    let tmp = TempDir::new("proactive-no-signals");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        "\n[proactive]\nenabled = true\ntarget = \"ops\"\n\
         signal_ttl_expiry = false\n\
         signal_recall_cluster = false\n\
         signal_due_reminder = false\n",
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "proactive.signals");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

// ------------------------------------------------------------------
// Phase 81 — [persona_lifecycle] section
// ------------------------------------------------------------------

/// No `[persona_lifecycle]` section → `persona_lifecycle: None`
/// (off; the Persona only ever grows, pre-Phase-81).
#[test]
fn persona_lifecycle_absent_section_is_none() {
    let env = EnvScope::new();
    let cfg = AivyxConfig::load_from_env_and_toml(
        &LoadOptions::test_env_only(),
    )
    .expect("load");
    assert!(cfg.persona_lifecycle.is_none());
    drop(env);
}

/// A present-but-disabled section may be partial (staged
/// config): it builds with `enabled = false`, defaults
/// elsewhere, and is NOT validated.
#[test]
fn persona_lifecycle_present_disabled_is_allowed_partial() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[persona_lifecycle]\nenabled = false\n",
        "pl-staged",
    );
    let p = cfg.persona_lifecycle.expect("section present");
    assert!(!p.enabled);
    assert!(
        (p.consolidation_similarity
            - crate::DEFAULT_PL_CONSOLIDATION_SIMILARITY)
            .abs()
            < 1e-6
    );
    assert_eq!(
        p.decay_max_age_secs,
        crate::DEFAULT_PL_DECAY_MAX_AGE_SECS
    );
    assert_eq!(
        p.min_soft_facets,
        crate::DEFAULT_PL_MIN_SOFT_FACETS
    );
    // Phase 85 — helpfulness-decay knobs default.
    assert!(
        (p.decay_unhelpful_threshold
            - crate::DEFAULT_PL_DECAY_UNHELPFUL_THRESHOLD)
            .abs()
            < 1e-6
    );
    assert_eq!(
        p.decay_min_samples,
        crate::DEFAULT_PL_DECAY_MIN_SAMPLES
    );
    // Phase 88 — pair-affinity decay floor default.
    assert!(
        (p.decay_pair_below_affinity
            - crate::DEFAULT_PL_DECAY_PAIR_BELOW_AFFINITY)
            .abs()
            < 1e-6
    );
    // Signals default on.
    assert!(p.signals.consolidate);
    assert!(p.signals.decay);
    drop(env);
}

/// Phase 85 — explicit helpfulness-decay knobs win; the two
/// validation cases fire only when decay is armed.
#[test]
fn persona_lifecycle_helpfulness_decay_knobs() {
    let env = EnvScope::new();

    // Valid override.
    let cfg = load_with_toml(
        "\n[persona_lifecycle]\nenabled = true\n\
         decay_unhelpful_threshold = -5.0\n\
         decay_min_samples = 8\n",
        "pl-help-valid",
    );
    let p = cfg.persona_lifecycle.expect("section present");
    assert!(
        (p.decay_unhelpful_threshold - (-5.0)).abs() < 1e-6
    );
    assert_eq!(p.decay_min_samples, 8);

    // Non-negative threshold (armed) → Invalid.
    let tmp = TempDir::new("pl-help-bad-threshold");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        "\n[persona_lifecycle]\nenabled = true\n\
         decay_unhelpful_threshold = 1.0\n",
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    match AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error")
    {
        ConfigError::Invalid { field, .. } => assert_eq!(
            field,
            "persona_lifecycle.decay_unhelpful_threshold"
        ),
        other => panic!("expected Invalid, got {other:?}"),
    }

    // Zero min-samples (armed) → Invalid.
    let tmp2 = TempDir::new("pl-help-zero-samples");
    let toml2 = tmp2.path().join("aivyx.toml");
    std::fs::write(
        &toml2,
        "\n[persona_lifecycle]\nenabled = true\n\
         decay_min_samples = 0\n",
    )
    .unwrap();
    let opts2 = LoadOptions {
        toml_path: Some(toml2),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    match AivyxConfig::load_from_env_and_toml(&opts2)
        .expect_err("must error")
    {
        ConfigError::Invalid { field, .. } => assert_eq!(
            field,
            "persona_lifecycle.decay_min_samples"
        ),
        other => panic!("expected Invalid, got {other:?}"),
    }

    // Decay disarmed → the knobs are NOT validated even if
    // nonsensical (staged config).
    let cfg2 = load_with_toml(
        "\n[persona_lifecycle]\nenabled = true\n\
         signal_consolidate = true\nsignal_decay = false\n\
         decay_unhelpful_threshold = 9.0\n\
         decay_min_samples = 0\n",
        "pl-help-disarmed",
    );
    assert!(cfg2.persona_lifecycle.is_some());

    drop(env);
}

/// Phase 88 — explicit `decay_pair_below_affinity` wins;
/// validation fires only when decay is armed; non-finite +
/// negative are rejects.
#[test]
fn persona_lifecycle_pair_affinity_decay_knob() {
    let env = EnvScope::new();

    // Valid override.
    let cfg = load_with_toml(
        "\n[persona_lifecycle]\nenabled = true\n\
         decay_pair_below_affinity = 0.4\n",
        "pl-pair-valid",
    );
    let p = cfg.persona_lifecycle.expect("section present");
    assert!(
        (p.decay_pair_below_affinity - 0.4).abs() < 1e-6
    );

    // Negative pair floor (armed) → Invalid.
    let tmp = TempDir::new("pl-pair-neg");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        "\n[persona_lifecycle]\nenabled = true\n\
         decay_pair_below_affinity = -1.0\n",
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    match AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error")
    {
        ConfigError::Invalid { field, .. } => assert_eq!(
            field,
            "persona_lifecycle.decay_pair_below_affinity"
        ),
        other => panic!("expected Invalid, got {other:?}"),
    }

    // Non-finite pair floor (armed) → Invalid.
    let tmp2 = TempDir::new("pl-pair-nan");
    let toml2 = tmp2.path().join("aivyx.toml");
    std::fs::write(
        &toml2,
        "\n[persona_lifecycle]\nenabled = true\n\
         decay_pair_below_affinity = nan\n",
    )
    .unwrap();
    let opts2 = LoadOptions {
        toml_path: Some(toml2),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    match AivyxConfig::load_from_env_and_toml(&opts2)
        .expect_err("must error")
    {
        ConfigError::Invalid { field, .. } => assert_eq!(
            field,
            "persona_lifecycle.decay_pair_below_affinity"
        ),
        other => panic!("expected Invalid, got {other:?}"),
    }

    // Decay disarmed → the knob is NOT validated even if
    // nonsensical (staged config — same Phase 85 posture).
    let cfg2 = load_with_toml(
        "\n[persona_lifecycle]\nenabled = true\n\
         signal_consolidate = true\nsignal_decay = false\n\
         decay_pair_below_affinity = -42.0\n",
        "pl-pair-disarmed",
    );
    assert!(cfg2.persona_lifecycle.is_some());

    drop(env);
}

/// Enabled + valid: explicit fields win; an explicitly-off
/// signal is respected while the other defaults on.
#[test]
fn persona_lifecycle_enabled_valid_with_signal_toggle() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[persona_lifecycle]\nenabled = true\n\
         consolidation_similarity = 0.85\n\
         decay_max_age_secs = 1209600\nmin_soft_facets = 4\n\
         signal_decay = false\n",
        "pl-valid",
    );
    let p = cfg.persona_lifecycle.expect("section present");
    assert!(p.enabled);
    assert!((p.consolidation_similarity - 0.85).abs() < 1e-6);
    assert_eq!(p.decay_max_age_secs, 1_209_600);
    assert_eq!(p.min_soft_facets, 4);
    assert!(p.signals.consolidate);
    assert!(!p.signals.decay);
    drop(env);
}

/// Enabled with an out-of-range similarity → load-time
/// `Invalid` (the `(0.0, 1.0]` bound; `0.0` is excluded).
#[test]
fn persona_lifecycle_enabled_bad_similarity_is_invalid() {
    let env = EnvScope::new();
    let tmp = TempDir::new("pl-bad-sim");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        "\n[persona_lifecycle]\nenabled = true\n\
         consolidation_similarity = 1.5\n",
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(
                field,
                "persona_lifecycle.consolidation_similarity"
            );
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

/// Enabled with `min_soft_facets = 0` → `Invalid`.
#[test]
fn persona_lifecycle_enabled_zero_min_facets_is_invalid() {
    let env = EnvScope::new();
    let tmp = TempDir::new("pl-zero-floor");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        "\n[persona_lifecycle]\nenabled = true\n\
         min_soft_facets = 0\n",
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "persona_lifecycle.min_soft_facets");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

/// Enabled with every signal class off → `Invalid`.
#[test]
fn persona_lifecycle_enabled_all_signals_off_is_invalid() {
    let env = EnvScope::new();
    let tmp = TempDir::new("pl-no-signals");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        "\n[persona_lifecycle]\nenabled = true\n\
         signal_consolidate = false\nsignal_decay = false\n",
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "persona_lifecycle.signals");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

/// No `[recall_cluster]` section → `recall_cluster: None`
/// (off; Phase 76 recall is unchanged, pre-Phase-84).
#[test]
fn recall_cluster_absent_section_is_none() {
    let env = EnvScope::new();
    let cfg = AivyxConfig::load_from_env_and_toml(
        &LoadOptions::test_env_only(),
    )
    .expect("load");
    assert!(cfg.recall_cluster.is_none());
    drop(env);
}

/// A present-but-disabled section may be partial (staged
/// config): builds with `enabled = false`, defaults
/// elsewhere, and is NOT validated.
#[test]
fn recall_cluster_present_disabled_is_allowed_partial() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[recall_cluster]\nenabled = false\n",
        "rc-staged",
    );
    let r = cfg.recall_cluster.expect("section present");
    assert!(!r.enabled);
    assert_eq!(
        r.max_siblings,
        crate::DEFAULT_RC_MAX_SIBLINGS
    );
    assert!(
        (r.min_affinity - crate::DEFAULT_RC_MIN_AFFINITY)
            .abs()
            < 1e-6
    );
    drop(env);
}

/// Enabled + valid: explicit fields win.
#[test]
fn recall_cluster_enabled_valid() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[recall_cluster]\nenabled = true\n\
         max_siblings = 5\nmin_affinity = 2.5\n",
        "rc-valid",
    );
    let r = cfg.recall_cluster.expect("section present");
    assert!(r.enabled);
    assert_eq!(r.max_siblings, 5);
    assert!((r.min_affinity - 2.5).abs() < 1e-6);
    drop(env);
}

/// Enabled with `max_siblings = 0` → load-time `Invalid`.
#[test]
fn recall_cluster_enabled_zero_siblings_is_invalid() {
    let env = EnvScope::new();
    let tmp = TempDir::new("rc-zero-siblings");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        "\n[recall_cluster]\nenabled = true\n\
         max_siblings = 0\n",
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "recall_cluster.max_siblings");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

/// Enabled with non-positive `min_affinity` → `Invalid`.
#[test]
fn recall_cluster_enabled_nonpositive_affinity_is_invalid() {
    let env = EnvScope::new();
    let tmp = TempDir::new("rc-bad-affinity");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        "\n[recall_cluster]\nenabled = true\n\
         min_affinity = 0.0\n",
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(field, "recall_cluster.min_affinity");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

// ---- Phase 87 — [persona_consolidation] ---------------------

/// No `[persona_consolidation]` section →
/// `persona_consolidation: None` (off; the Persona proposal
/// pipeline is byte-identical to pre-Phase-87).
#[test]
fn persona_consolidation_absent_section_is_none() {
    let env = EnvScope::new();
    let cfg = AivyxConfig::load_from_env_and_toml(
        &LoadOptions::test_env_only(),
    )
    .expect("load");
    assert!(cfg.persona_consolidation.is_none());
    drop(env);
}

/// A present-but-disabled section may be partial (staged
/// config): builds with `enabled = false`, defaults elsewhere,
/// and is NOT validated.
#[test]
fn persona_consolidation_present_disabled_is_allowed_partial() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[persona_consolidation]\nenabled = false\n",
        "pc-staged",
    );
    let p = cfg.persona_consolidation.expect("section present");
    assert!(!p.enabled);
    assert!(
        (p.min_affinity - crate::DEFAULT_PC_MIN_AFFINITY).abs()
            < 1e-6
    );
    assert_eq!(
        p.min_samples,
        crate::DEFAULT_PC_MIN_SAMPLES
    );
    assert!(
        (p.min_topic_helpfulness
            - crate::DEFAULT_PC_MIN_TOPIC_HELPFULNESS)
            .abs()
            < 1e-6
    );
    assert_eq!(
        p.max_proposals_per_cycle,
        crate::DEFAULT_PC_MAX_PROPOSALS_PER_CYCLE
    );
    drop(env);
}

/// Enabled + valid: explicit fields win.
#[test]
fn persona_consolidation_enabled_valid() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[persona_consolidation]\nenabled = true\n\
         min_affinity = 2.5\nmin_samples = 7\n\
         min_topic_helpfulness = 0.5\n\
         max_proposals_per_cycle = 5\n",
        "pc-valid",
    );
    let p = cfg.persona_consolidation.expect("section present");
    assert!(p.enabled);
    assert!((p.min_affinity - 2.5).abs() < 1e-6);
    assert_eq!(p.min_samples, 7);
    assert!((p.min_topic_helpfulness - 0.5).abs() < 1e-6);
    assert_eq!(p.max_proposals_per_cycle, 5);
    drop(env);
}

/// Enabled with non-positive `min_affinity` → `Invalid`.
#[test]
fn persona_consolidation_enabled_nonpositive_affinity_is_invalid()
{
    let env = EnvScope::new();
    let tmp = TempDir::new("pc-bad-affinity");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        "\n[persona_consolidation]\nenabled = true\n\
         min_affinity = 0.0\n",
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(
                field,
                "persona_consolidation.min_affinity"
            );
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

/// Enabled with `min_samples = 0` → `Invalid`.
#[test]
fn persona_consolidation_enabled_zero_samples_is_invalid() {
    let env = EnvScope::new();
    let tmp = TempDir::new("pc-zero-samples");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        "\n[persona_consolidation]\nenabled = true\n\
         min_samples = 0\n",
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(
                field,
                "persona_consolidation.min_samples"
            );
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

/// Enabled with `max_proposals_per_cycle = 0` → `Invalid`.
#[test]
fn persona_consolidation_enabled_zero_cap_is_invalid() {
    let env = EnvScope::new();
    let tmp = TempDir::new("pc-zero-cap");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        "\n[persona_consolidation]\nenabled = true\n\
         max_proposals_per_cycle = 0\n",
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(
                field,
                "persona_consolidation.max_proposals_per_cycle"
            );
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

/// Phase 92 — `enable_supersession` defaults to `false` even
/// when the block is present and `enabled = true`. Operators
/// running Phase 87 consolidation must explicitly opt into
/// supersession.
#[test]
fn persona_consolidation_supersession_defaults_false() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[persona_consolidation]\nenabled = true\n",
        "pc-supersede-default",
    );
    let p = cfg.persona_consolidation.expect("section present");
    assert!(p.enabled);
    assert!(
        !p.enable_supersession,
        "supersession is opt-in; default false even when \
         consolidation itself is enabled"
    );
    drop(env);
}

/// Phase 92 — explicit `enable_supersession = true` wins.
#[test]
fn persona_consolidation_supersession_explicit_true_wins() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[persona_consolidation]\nenabled = true\n\
         enable_supersession = true\n",
        "pc-supersede-on",
    );
    let p = cfg.persona_consolidation.expect("section present");
    assert!(p.enable_supersession);
    drop(env);
}

/// Phase 92 — a staged-disabled section can carry the
/// supersession key partially (the staged-config flexibility
/// from Phase 85 / 87). Setting only the supersession key (no
/// other persona_consolidation fields) still builds Some(...)
/// because the "any field set" predicate fires.
#[test]
fn persona_consolidation_supersession_only_builds_some() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[persona_consolidation]\nenable_supersession = true\n",
        "pc-supersede-only",
    );
    let p = cfg.persona_consolidation.expect("section present");
    assert!(!p.enabled, "enabled defaults to false");
    assert!(p.enable_supersession);
    drop(env);
}

// ---- Phase 89 — [memory].canonicalize_topics ----------------

/// No `[memory]` block (or no `canonicalize_topics` key) →
/// `memory_canonicalize_topics = false` (the memory layer is
/// byte-identical to pre-Phase-89).
#[test]
fn memory_canonicalize_topics_absent_defaults_false() {
    let env = EnvScope::new();
    let cfg = AivyxConfig::load_from_env_and_toml(
        &LoadOptions::test_env_only(),
    )
    .expect("load");
    assert!(!cfg.memory_canonicalize_topics.value);
    assert_eq!(
        cfg.memory_canonicalize_topics.source,
        crate::FieldSource::Default,
    );
    drop(env);
}

/// Explicit `canonicalize_topics = true` wins; source records
/// it came from the TOML.
#[test]
fn memory_canonicalize_topics_explicit_true_wins() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[memory]\ncanonicalize_topics = true\n",
        "mem-canon-on",
    );
    assert!(cfg.memory_canonicalize_topics.value);
    assert_eq!(
        cfg.memory_canonicalize_topics.source,
        crate::FieldSource::Toml,
    );
    drop(env);
}

/// Explicit `canonicalize_topics = false` is honored (the
/// operator can express the default explicitly without
/// changing behaviour).
#[test]
fn memory_canonicalize_topics_explicit_false_wins() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[memory]\ncanonicalize_topics = false\n",
        "mem-canon-off",
    );
    assert!(!cfg.memory_canonicalize_topics.value);
    assert_eq!(
        cfg.memory_canonicalize_topics.source,
        crate::FieldSource::Toml,
    );
    drop(env);
}

// ---- Phase 91 — [recall_judgment] -----------------------------

/// No `[recall_judgment]` section → `recall_judgment: None`
/// (off; the Phase 77 structural recall-feedback signal is
/// the only signal — byte-identical to pre-Phase-91).
#[test]
fn recall_judgment_absent_section_is_none() {
    let env = EnvScope::new();
    let cfg = AivyxConfig::load_from_env_and_toml(
        &LoadOptions::test_env_only(),
    )
    .expect("load");
    assert!(cfg.recall_judgment.is_none());
    drop(env);
}

/// A present-but-disabled section may be partial (staged
/// config). It builds with `enabled = false`, defaults
/// elsewhere, and is NOT validated.
#[test]
fn recall_judgment_present_disabled_is_allowed_partial() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[recall_judgment]\nenabled = false\n",
        "rj-staged",
    );
    let rj = cfg.recall_judgment.expect("section present");
    assert!(!rj.enabled);
    assert_eq!(
        rj.max_recalls_per_cycle,
        crate::DEFAULT_RJ_MAX_RECALLS_PER_CYCLE,
    );
    drop(env);
}

/// Enabled + valid: explicit field wins.
#[test]
fn recall_judgment_enabled_valid() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[recall_judgment]\nenabled = true\n\
         max_recalls_per_cycle = 7\n",
        "rj-valid",
    );
    let rj = cfg.recall_judgment.expect("section present");
    assert!(rj.enabled);
    assert_eq!(rj.max_recalls_per_cycle, 7);
    drop(env);
}

/// Enabled with `max_recalls_per_cycle = 0` → `Invalid`.
#[test]
fn recall_judgment_enabled_zero_cap_is_invalid() {
    let env = EnvScope::new();
    let tmp = TempDir::new("rj-zero-cap");
    let toml_path = tmp.path().join("aivyx.toml");
    std::fs::write(
        &toml_path,
        "\n[recall_judgment]\nenabled = true\n\
         max_recalls_per_cycle = 0\n",
    )
    .unwrap();
    let opts = LoadOptions {
        toml_path: Some(toml_path),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    let err = AivyxConfig::load_from_env_and_toml(&opts)
        .expect_err("must error");
    match err {
        ConfigError::Invalid { field, .. } => {
            assert_eq!(
                field,
                "recall_judgment.max_recalls_per_cycle",
            );
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    drop(env);
}

/// Decay disarmed (the `enabled = false` staged-config
/// posture) → the cap knob is NOT validated even if
/// nonsensical (mirrors the Phase 85 / Phase 87 staged-
/// config behaviour).
#[test]
fn recall_judgment_disabled_partial_allows_nonsense() {
    let env = EnvScope::new();
    let cfg = load_with_toml(
        "\n[recall_judgment]\nenabled = false\n\
         max_recalls_per_cycle = 0\n",
        "rj-disabled-nonsense",
    );
    let rj = cfg.recall_judgment.expect("section present");
    assert!(!rj.enabled);
    assert_eq!(rj.max_recalls_per_cycle, 0);
    drop(env);
}

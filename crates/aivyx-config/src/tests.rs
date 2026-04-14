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
    AivyxConfig, ConfigError, FieldSource, LoadOptions, DEFAULT_MEMORY_MAX_PER_TOPIC,
    DEFAULT_MODEL, DEFAULT_SYSTEM_PROMPT,
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

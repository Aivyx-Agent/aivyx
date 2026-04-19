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
    AivyxConfig, ConfigError, FieldSource, LoadOptions, McpTransportKind, ProviderKind, Role,
    ToolAllowlist, DEFAULT_MEMORY_MAX_PER_TOPIC, DEFAULT_MODEL, DEFAULT_ROLE_NAME,
    DEFAULT_SYSTEM_PROMPT,
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

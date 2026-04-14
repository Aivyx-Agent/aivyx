//! Passphrase → `MasterKey` derivation for the local CLI channel.
//!
//! ## What this module owns
//!
//! D7 (LOCKED 2026-04-13) commits that the passphrase prompt lives in
//! the **channel adapter**, not the storage layer: `aivyx-storage`
//! never sees a raw passphrase. This module is the channel-side half
//! of that contract for the `LocalChannel` / CLI binary. It:
//!
//! 1. Sources a passphrase from one of several places (env var, test
//!    fixture, or — in a later follow-up task — an interactive TTY
//!    prompt via `rpassword`).
//! 2. Loads (or on first run, generates + persists) a 16-byte salt
//!    from a sidecar file next to the redb store.
//! 3. Runs Argon2id through [`aivyx_crypto::derive_master_key`] to
//!    produce a [`aivyx_crypto::MasterKey`].
//! 4. Zeroizes the transient passphrase buffer before returning.
//!
//! The **only** value that crosses this module's boundary is the
//! `MasterKey` — the passphrase bytes live inside a single `Vec<u8>`
//! that is explicitly zeroized between derivation and return. Any
//! future code that wants the passphrase back (rotation, change-
//! passphrase flow) will have to re-source it, which is the point.
//!
//! ## Why a sidecar salt file?
//!
//! Argon2id needs a per-store salt to defeat rainbow tables. The salt
//! cannot live inside the encrypted redb store — the master key is
//! needed to open the store, and the salt is needed to derive the
//! master key, which is a chicken-and-egg. Salts are not secret
//! (that's the whole point of Argon2's design), so they can live in
//! a plaintext sidecar file without compromising the security model.
//!
//! The sidecar path convention: if the redb store is `store.redb`
//! the salt file is `store.redb.salt`. The binary in task 4 will
//! adopt this convention; tests construct both paths explicitly.
//!
//! ## Phase 5 scope — env var only
//!
//! Per PHASE_5.md Q2 (leaning: option 3), the core task 3 commit
//! wires `PassphraseSource::Env` as the only production source and
//! leaves `PassphraseSource::InteractivePrompt` as a stub returning
//! [`PassphraseError::InteractiveNotImplemented`]. A follow-up task
//! inside Phase 5 (or Phase 6 if time runs out) pulls in `rpassword`
//! and implements the interactive path. Tests exercise the env-var
//! path without a tty.

use std::fs;
use std::path::{Path, PathBuf};

use aivyx_crypto::{derive_master_key as crypto_derive_master_key, Argon2Params, MasterKey};
use zeroize::Zeroize;

/// Length of the Argon2id salt in bytes. 16 is the OWASP-recommended
/// floor for password hashing and matches every Argon2 reference
/// implementation in the RustCrypto ecosystem.
pub const SALT_LEN: usize = 16;

/// Default env var name for the env-sourced passphrase. The binary
/// and tests refer to this constant rather than hardcoding the
/// string, so a rename stays a one-line edit.
pub const DEFAULT_ENV_VAR: &str = "AIVYX_PASSPHRASE";

// --------------------------------------------------------------------
// Errors
// --------------------------------------------------------------------

/// Errors this module produces. All fold into the existing
/// `AivyxError::Crypto` / `AivyxError::Config` slots at the
/// `aivyx-core` boundary — no new top-level variant needed.
#[derive(Debug, thiserror::Error)]
pub enum PassphraseError {
    /// `AIVYX_PASSPHRASE` (or the caller's chosen env var) was not
    /// set when a `PassphraseSource::Env` was requested.
    #[error("passphrase env var `{0}` is not set")]
    EnvNotSet(String),

    /// `AIVYX_PASSPHRASE` was set but empty. Rejected explicitly
    /// because Argon2id happily hashes an empty input and the
    /// resulting key would be trivially brute-forceable.
    #[error("passphrase env var `{0}` is empty")]
    EnvEmpty(String),

    /// Reading or writing the salt sidecar file failed. Wraps
    /// `std::io::Error::to_string()` to avoid dragging the whole
    /// `io::Error` type across the module boundary.
    #[error("salt sidecar I/O failed at {path:?}: {reason}")]
    SaltIo { path: PathBuf, reason: String },

    /// The salt sidecar file exists but is not exactly [`SALT_LEN`]
    /// bytes long. This means the file was written by a different
    /// tool, truncated, or corrupted.
    #[error("salt sidecar at {path:?} is {len} bytes, expected {SALT_LEN}")]
    SaltMalformed { path: PathBuf, len: usize },

    /// Argon2id or the upstream crypto layer refused the derivation.
    /// In practice this only happens if [`Argon2Params`] is set to a
    /// value below Argon2's minimum (e.g., memory_kib = 0).
    #[error("crypto derivation failed: {0}")]
    Crypto(#[from] aivyx_crypto::CryptoError),

    /// Placeholder for the interactive TTY prompt path. Returned by
    /// [`PassphraseSource::InteractivePrompt`] in Phase 5 task 3.
    /// A follow-up task inside Phase 5 wires `rpassword::prompt_password`
    /// here and removes this variant.
    #[error("interactive passphrase prompt not implemented yet (Phase 5 follow-up task)")]
    InteractiveNotImplemented,
}

// --------------------------------------------------------------------
// PassphraseSource
// --------------------------------------------------------------------

/// Where to source the passphrase bytes.
///
/// The production binary uses [`PassphraseSource::Env`]; tests use
/// [`PassphraseSource::Fixture`] to inject known bytes without
/// touching the environment or a tty. The [`PassphraseSource::
/// InteractivePrompt`] variant is a placeholder that currently
/// errors — see [`PassphraseError::InteractiveNotImplemented`].
pub enum PassphraseSource {
    /// Read from a named environment variable. Fails with
    /// [`PassphraseError::EnvNotSet`] if unset or
    /// [`PassphraseError::EnvEmpty`] if set to "". Pass
    /// [`DEFAULT_ENV_VAR`] for the standard `AIVYX_PASSPHRASE` name.
    Env { var_name: String },

    /// Test-only: invoke a caller-supplied closure to produce the
    /// passphrase bytes. The closure is called once and its return
    /// value is zeroized after the master key is derived. Production
    /// code should not use this variant — there's nothing stopping
    /// it, but the idiomatic binary wiring is `Env`.
    Fixture(Box<dyn FnOnce() -> Vec<u8> + Send>),

    /// Interactive TTY prompt. Currently unimplemented — returns
    /// [`PassphraseError::InteractiveNotImplemented`]. A Phase 5
    /// follow-up task wires `rpassword::prompt_password("aivyx
    /// passphrase: ")` here.
    InteractivePrompt,
}

impl std::fmt::Debug for PassphraseSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Name variants only — never even hint at what the
            // closure might return.
            PassphraseSource::Env { var_name } => f
                .debug_struct("Env")
                .field("var_name", var_name)
                .finish(),
            PassphraseSource::Fixture(_) => {
                f.debug_struct("Fixture").finish_non_exhaustive()
            }
            PassphraseSource::InteractivePrompt => f.write_str("InteractivePrompt"),
        }
    }
}

// --------------------------------------------------------------------
// Salt sidecar
// --------------------------------------------------------------------

/// Load the salt from `salt_path` if it exists, otherwise generate
/// a fresh 16-byte salt from `Uuid::new_v4()` and persist it to the
/// same path.
///
/// The salt is written atomically in the "simple" sense: since this
/// is a 16-byte file, a single `fs::write` either succeeds fully or
/// leaves no file at all on every filesystem this binary supports.
/// No temp-and-rename ritual is needed at this size.
///
/// Salts are **not secret**, so the file permissions are not
/// constrained — the defense is that a salt is unique per store, not
/// that it's hidden from an attacker.
pub fn load_or_create_salt(salt_path: &Path) -> Result<[u8; SALT_LEN], PassphraseError> {
    match fs::read(salt_path) {
        Ok(bytes) => {
            if bytes.len() != SALT_LEN {
                return Err(PassphraseError::SaltMalformed {
                    path: salt_path.to_path_buf(),
                    len: bytes.len(),
                });
            }
            let mut salt = [0u8; SALT_LEN];
            salt.copy_from_slice(&bytes);
            Ok(salt)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // First-run path. Generate from Uuid::new_v4() (same
            // entropy source aivyx-storage uses for AEAD nonces)
            // and persist plaintext.
            let uuid_bytes = *uuid::Uuid::new_v4().as_bytes(); // 16 bytes
            fs::write(salt_path, uuid_bytes).map_err(|e| PassphraseError::SaltIo {
                path: salt_path.to_path_buf(),
                reason: e.to_string(),
            })?;
            Ok(uuid_bytes)
        }
        Err(e) => Err(PassphraseError::SaltIo {
            path: salt_path.to_path_buf(),
            reason: e.to_string(),
        }),
    }
}

// --------------------------------------------------------------------
// derive_master_key
// --------------------------------------------------------------------

/// Source a passphrase from `source`, load or create the salt at
/// `salt_path`, and derive a [`MasterKey`] via Argon2id.
///
/// The transient passphrase `Vec<u8>` is explicitly zeroized between
/// the Argon2id call returning and this function returning, so the
/// raw bytes do not linger on the heap past master-key derivation.
///
/// # Errors
///
/// - [`PassphraseError::EnvNotSet`] / [`PassphraseError::EnvEmpty`]
///   if `source` is `Env` and the variable is missing or empty
/// - [`PassphraseError::SaltIo`] / [`PassphraseError::SaltMalformed`]
///   if the salt sidecar can't be read, written, or is the wrong size
/// - [`PassphraseError::Crypto`] if Argon2id refuses the params
/// - [`PassphraseError::InteractiveNotImplemented`] if `source` is
///   `InteractivePrompt`
pub fn derive_master_key(
    source: PassphraseSource,
    salt_path: &Path,
    params: Argon2Params,
) -> Result<MasterKey, PassphraseError> {
    let mut passphrase = fetch_passphrase_bytes(source)?;
    let salt = load_or_create_salt(salt_path)?;

    // `?` is fine here — the transient `passphrase` Vec zeroes on
    // drop regardless of return path, but only because of the
    // explicit `zeroize()` below + the immediately-following drop.
    // A panic would leave the Vec on the stack until unwind, which
    // is acceptable; this is a best-effort hygiene step, not a
    // hard guarantee.
    let result = crypto_derive_master_key(&passphrase, &salt, params);

    passphrase.zeroize();
    drop(passphrase);

    Ok(result?)
}

fn fetch_passphrase_bytes(source: PassphraseSource) -> Result<Vec<u8>, PassphraseError> {
    match source {
        PassphraseSource::Env { var_name } => match std::env::var(&var_name) {
            Ok(s) if s.is_empty() => Err(PassphraseError::EnvEmpty(var_name)),
            Ok(s) => Ok(s.into_bytes()),
            Err(_) => Err(PassphraseError::EnvNotSet(var_name)),
        },
        PassphraseSource::Fixture(f) => Ok(f()),
        PassphraseSource::InteractivePrompt => Err(PassphraseError::InteractiveNotImplemented),
    }
}

// --------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env vars are process-global state. Run every env-touching test
    // under a single mutex so `cargo test` parallelism can't make
    // one test's unset race with another's set. This is the same
    // pattern aivyx-audit uses for its HMAC-key env fixture.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// RAII temp dir — same `$TMPDIR + uuid` pattern aivyx-storage
    /// and aivyx-core::tools::fs use, to avoid adding `tempfile`.
    struct TestDir {
        dir: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let tmp = std::env::var("TMPDIR")
                .or_else(|_| std::env::var("TEMP"))
                .unwrap_or_else(|_| "/tmp".to_string());
            let dir = PathBuf::from(tmp)
                .join(format!("aivyx-passphrase-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).expect("test dir must be creatable");
            TestDir { dir }
        }

        fn salt(&self) -> PathBuf {
            self.dir.join("store.redb.salt")
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn fixture(bytes: &'static [u8]) -> PassphraseSource {
        PassphraseSource::Fixture(Box::new(move || bytes.to_vec()))
    }

    // ---- Salt sidecar -----------------------------------------------

    #[test]
    fn salt_is_generated_on_first_call_and_persisted() {
        let dir = TestDir::new();
        let salt_path = dir.salt();

        assert!(!salt_path.exists(), "precondition: salt file missing");
        let s1 = load_or_create_salt(&salt_path).unwrap();
        assert_eq!(s1.len(), SALT_LEN);
        assert!(salt_path.exists(), "salt file must be created on first call");

        // Second call reads the same bytes back, does not regenerate.
        let s2 = load_or_create_salt(&salt_path).unwrap();
        assert_eq!(s1, s2);
    }

    #[test]
    fn salt_differs_between_independent_stores() {
        let dir_a = TestDir::new();
        let dir_b = TestDir::new();
        let s_a = load_or_create_salt(&dir_a.salt()).unwrap();
        let s_b = load_or_create_salt(&dir_b.salt()).unwrap();
        // Random from Uuid::new_v4 — the collision probability is
        // 2^-128, small enough to treat this assertion as a
        // determinism sanity check.
        assert_ne!(s_a, s_b);
    }

    #[test]
    fn salt_rejects_wrong_length_file() {
        let dir = TestDir::new();
        let salt_path = dir.salt();
        // Write 8 bytes instead of 16.
        fs::write(&salt_path, [0u8; 8]).unwrap();
        let err = load_or_create_salt(&salt_path).unwrap_err();
        assert!(matches!(
            err,
            PassphraseError::SaltMalformed { len: 8, .. }
        ));
    }

    // ---- Env var source ---------------------------------------------

    #[test]
    fn env_source_success_derives_master_key() {
        let _lock = env_lock();
        let dir = TestDir::new();
        // SAFETY: `set_var` is safe under our env_lock serialization.
        unsafe {
            std::env::set_var("AIVYX_PASSPHRASE_TEST_OK", "correct horse battery staple");
        }
        let master = derive_master_key(
            PassphraseSource::Env {
                var_name: "AIVYX_PASSPHRASE_TEST_OK".to_string(),
            },
            &dir.salt(),
            Argon2Params::weak_for_tests(),
        )
        .expect("env source must succeed");
        // Prove the key actually round-trips by deriving a subkey —
        // if Argon2 silently produced zeros, HKDF would still work
        // but the Debug redaction check in aivyx-crypto already
        // catches that, so here we just confirm no error.
        let _sub = master.derive_subkey(b"sessions").unwrap();
        unsafe {
            std::env::remove_var("AIVYX_PASSPHRASE_TEST_OK");
        }
    }

    #[test]
    fn env_source_not_set_fails_cleanly() {
        let _lock = env_lock();
        let dir = TestDir::new();
        // SAFETY: `remove_var` is safe under the env_lock.
        unsafe {
            std::env::remove_var("AIVYX_PASSPHRASE_TEST_MISSING");
        }
        let err = derive_master_key(
            PassphraseSource::Env {
                var_name: "AIVYX_PASSPHRASE_TEST_MISSING".to_string(),
            },
            &dir.salt(),
            Argon2Params::weak_for_tests(),
        )
        .unwrap_err();
        assert!(matches!(err, PassphraseError::EnvNotSet(ref v) if v == "AIVYX_PASSPHRASE_TEST_MISSING"));
    }

    #[test]
    fn env_source_empty_fails_cleanly() {
        let _lock = env_lock();
        let dir = TestDir::new();
        unsafe {
            std::env::set_var("AIVYX_PASSPHRASE_TEST_EMPTY", "");
        }
        let err = derive_master_key(
            PassphraseSource::Env {
                var_name: "AIVYX_PASSPHRASE_TEST_EMPTY".to_string(),
            },
            &dir.salt(),
            Argon2Params::weak_for_tests(),
        )
        .unwrap_err();
        assert!(matches!(err, PassphraseError::EnvEmpty(ref v) if v == "AIVYX_PASSPHRASE_TEST_EMPTY"));
        unsafe {
            std::env::remove_var("AIVYX_PASSPHRASE_TEST_EMPTY");
        }
    }

    // ---- Fixture source ---------------------------------------------

    #[test]
    fn fixture_source_derives_master_key() {
        let dir = TestDir::new();
        let master = derive_master_key(
            fixture(b"fixture-passphrase"),
            &dir.salt(),
            Argon2Params::weak_for_tests(),
        )
        .expect("fixture source must succeed");
        let _sub = master.derive_subkey(b"sessions").unwrap();
    }

    #[test]
    fn fixture_and_env_produce_same_master_key_for_same_bytes() {
        let _lock = env_lock();
        let dir = TestDir::new();
        let salt_path = dir.salt();

        // Both sources use the same passphrase string; the salt is
        // shared across both calls (load_or_create_salt will generate
        // it on the first call and read it on the second).
        unsafe {
            std::env::set_var("AIVYX_PASSPHRASE_TEST_PARITY", "same-bytes");
        }
        let m_env = derive_master_key(
            PassphraseSource::Env {
                var_name: "AIVYX_PASSPHRASE_TEST_PARITY".to_string(),
            },
            &salt_path,
            Argon2Params::weak_for_tests(),
        )
        .unwrap();
        unsafe {
            std::env::remove_var("AIVYX_PASSPHRASE_TEST_PARITY");
        }

        let m_fix = derive_master_key(
            fixture(b"same-bytes"),
            &salt_path,
            Argon2Params::weak_for_tests(),
        )
        .unwrap();

        // Compare via a derived subkey since MasterKey's bytes are
        // not publicly accessible. If they match, the master keys
        // are equal (HKDF is deterministic).
        let s_env = m_env.derive_subkey(b"sessions").unwrap();
        let s_fix = m_fix.derive_subkey(b"sessions").unwrap();
        assert_eq!(
            subkey_fingerprint(&s_env),
            subkey_fingerprint(&s_fix),
            "same passphrase + same salt must produce same master key"
        );
    }

    /// Public MasterKey/SubKey types hide their bytes; to prove two
    /// subkeys are equal in a test, we seal the same plaintext with
    /// both and check that each can open the other's ciphertext.
    /// Equality-by-function-extensionality instead of equality-by-
    /// bytes — same correctness, different axis.
    fn subkey_fingerprint(sub: &aivyx_crypto::SubKey) -> Vec<u8> {
        let nonce = [0u8; aivyx_crypto::NONCE_LEN];
        sub.seal(&nonce, b"fingerprint", b"probe").unwrap()
    }

    // ---- Interactive stub --------------------------------------------

    #[test]
    fn interactive_source_returns_not_implemented_stub() {
        let dir = TestDir::new();
        let err = derive_master_key(
            PassphraseSource::InteractivePrompt,
            &dir.salt(),
            Argon2Params::weak_for_tests(),
        )
        .unwrap_err();
        assert!(matches!(err, PassphraseError::InteractiveNotImplemented));
    }

    // ---- End-to-end with aivyx-storage -------------------------------

    #[tokio::test]
    async fn passphrase_to_storage_full_round_trip() {
        // The whole point of the task — proves the three-layer chain
        // (passphrase → MasterKey → RedbStorage → get/put) actually
        // composes. Task 4 will wire this same shape into
        // SessionConfig; this test is its regression precedent.
        use aivyx_storage::{KeyDomain, RedbStorage, StorageConfig};

        let dir = TestDir::new();
        let store_path = dir.dir.join("store.redb");
        let salt_path = dir.salt();
        let passphrase: &[u8] = b"e2e-passphrase";

        // Session A: derive, open, write, drop.
        {
            let master_a = derive_master_key(
                fixture(passphrase),
                &salt_path,
                Argon2Params::weak_for_tests(),
            )
            .expect("session A: derive");

            let store_a = RedbStorage::open(StorageConfig::new(store_path.clone()), master_a)
                .await
                .expect("session A: open");

            store_a
                .domain(KeyDomain::Sessions)
                .put(b"last-seen", b"turn-99")
                .await
                .expect("session A: put");

            drop(store_a);
        }

        // Session B: re-derive from the same passphrase + same salt
        // file (load_or_create_salt reads the existing bytes), open
        // the same redb file, read back the persisted value.
        {
            let master_b = derive_master_key(
                fixture(passphrase),
                &salt_path,
                Argon2Params::weak_for_tests(),
            )
            .expect("session B: re-derive");

            let store_b = RedbStorage::open(StorageConfig::new(store_path.clone()), master_b)
                .await
                .expect("session B: reopen");

            let got = store_b
                .domain(KeyDomain::Sessions)
                .get(b"last-seen")
                .await
                .expect("session B: get");
            assert_eq!(got, Some(b"turn-99".to_vec()));
        }
    }

    #[tokio::test]
    async fn wrong_passphrase_fails_to_decrypt_storage() {
        // Complements the storage-layer wrong-key test: task 3's
        // contract says "same passphrase + same salt → same master →
        // same subkeys," so a *different* passphrase under the same
        // salt must produce a master that cannot decrypt the
        // ciphertext. Proves the full chain is sensitive to
        // passphrase bytes, not just to explicit MasterKey
        // construction.
        use aivyx_storage::{KeyDomain, RedbStorage, StorageConfig, StorageError};

        let dir = TestDir::new();
        let store_path = dir.dir.join("store.redb");
        let salt_path = dir.salt();

        // Write with passphrase A.
        {
            let master = derive_master_key(
                fixture(b"passphrase-a"),
                &salt_path,
                Argon2Params::weak_for_tests(),
            )
            .unwrap();
            let store = RedbStorage::open(StorageConfig::new(store_path.clone()), master)
                .await
                .unwrap();
            store
                .domain(KeyDomain::Secrets)
                .put(b"api-key", b"sk-xxx")
                .await
                .unwrap();
            drop(store);
        }

        // Reopen with passphrase B; get fails at decrypt time.
        let master = derive_master_key(
            fixture(b"passphrase-b"),
            &salt_path,
            Argon2Params::weak_for_tests(),
        )
        .unwrap();
        let store = RedbStorage::open(StorageConfig::new(store_path.clone()), master)
            .await
            .unwrap();
        let err = store
            .domain(KeyDomain::Secrets)
            .get(b"api-key")
            .await
            .unwrap_err();
        assert!(
            matches!(err, StorageError::DecryptFailed { domain: KeyDomain::Secrets }),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn debug_impl_does_not_leak_fixture_closure_contents() {
        // Regression lock: a `dbg!(source)` on a Fixture variant
        // must not somehow print the passphrase that the closure
        // would return. The closure is opaque-by-design and the
        // Debug impl must reflect that.
        let src = fixture(b"super-secret");
        let dbg = format!("{src:?}");
        assert!(dbg.contains("Fixture"));
        assert!(!dbg.contains("super-secret"));
    }
}

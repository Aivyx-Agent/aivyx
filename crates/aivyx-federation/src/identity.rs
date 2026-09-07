//! Identity (FED.1 / Chapter Passport PP.1) — *who is this agent?*
//!
//! An operator-owned **Ed25519 keypair per instance** (sovereignty: the
//! identity is the operator's), and the **signed, replay-guarded request
//! envelope** every cross-boundary request carries.
//!
//! Lifted from the archived `auth.rs` and modernized onto the new core (PP.0):
//! - errors map to [`crate::FederationError`] (no shared `AivyxError`);
//! - **key-at-rest is sealed with a subkey derived from the storage
//!   [`MasterKey`](aivyx_crypto::MasterKey)** via the new core's audited HKDF +
//!   ChaCha20-Poly1305 ([`SubKey::seal`](aivyx_crypto::SubKey::seal)), replacing
//!   the salvage's hand-rolled AEAD.
//!
//! Preserved invariants: the manual [`Debug`] that **redacts the signing key**,
//! `0o600` key-file permissions, and `instance_id` charset validation — *key
//! material is never logged* (FED.0 §7).

use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use aivyx_crypto::{MasterKey, NONCE_LEN};

use crate::FederationError;

/// Maximum age of a signed request before it is considered stale.
const MAX_REQUEST_AGE_SECS: u64 = 60;

/// HKDF `info` that derives the federation identity-key-wrapping subkey from the
/// storage [`MasterKey`]. Versioned so a future rotation is a new label, never a
/// silent reinterpretation of old bytes.
const KEY_WRAP_INFO: &[u8] = b"aivyx-federation-identity-key-v1";

/// AEAD additional-authenticated-data binding the sealed key to its purpose.
const KEY_WRAP_AAD: &[u8] = b"aivyx-federation-identity";

/// Encrypted key-file layout: `nonce(12) || ciphertext+tag(48)`.
const ENCRYPTED_KEY_LEN: usize = NONCE_LEN + 48;

/// A signed federation request header (FED.0 §2). Travels with every
/// cross-boundary request; verified against the peer's known public key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedHeader {
    /// The sender's instance id.
    pub instance_id: String,
    /// Unix timestamp (secs) when the request was signed.
    pub timestamp: u64,
    /// Base64 Ed25519 signature over `"{instance_id}:{timestamp}:{body_hash}"`.
    pub signature: String,
}

/// Replay guard — remembers recently seen `instance_id:timestamp:signature`
/// nonces and rejects duplicates within the freshness window. Call
/// [`check_and_record`](ReplayGuard::check_and_record) **after** signature
/// verification succeeds.
pub struct ReplayGuard {
    seen: Mutex<std::collections::HashSet<String>>,
    last_evict: Mutex<u64>,
}

impl ReplayGuard {
    pub fn new() -> Self {
        Self {
            seen: Mutex::new(std::collections::HashSet::new()),
            last_evict: Mutex::new(now_secs()),
        }
    }

    /// Record this header's nonce; `Err` if it was already seen (a replay).
    pub fn check_and_record(&self, header: &SignedHeader) -> Result<(), FederationError> {
        let nonce = format!(
            "{}:{}:{}",
            header.instance_id, header.timestamp, header.signature
        );
        let now = now_secs();
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut last = self.last_evict.lock().unwrap_or_else(|e| e.into_inner());
            if now.saturating_sub(*last) >= MAX_REQUEST_AGE_SECS {
                seen.clear();
                *last = now;
            }
        }
        if !seen.insert(nonce) {
            return Err(FederationError::Identity("replayed federation request".into()));
        }
        Ok(())
    }
}

impl Default for ReplayGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Abstraction over "something that can hardware-sign like a YubiKey".
///
/// Exists purely as an internal storage/dispatch seam so this crate's own
/// tests can exercise [`IdentitySigner::Hardware`]'s dispatch and
/// [`Identity::load_hardware`]'s serial-mismatch rejection without real
/// YubiKey hardware. `aivyx_yubi::YubiKeySigner`'s own public API has no way
/// to construct a hardware-backed instance without either real PC/SC
/// hardware discovery (`YubiKeySigner::new`) or `aivyx-yubi`'s private,
/// crate-internal-only test seams (`from_open_card`/`discover_and_construct`
/// — not `pub`, not reachable from outside that crate even under its
/// `test-util` feature) — see that crate's `src/sign.rs`. `Identity::
/// load_hardware`'s public signature still takes a concrete
/// `aivyx_yubi::YubiKeySigner` directly (matching what Task 8's CLI
/// constructs); this trait never appears in this crate's public API.
trait HardwareSigner: Send + Sync {
    fn sign(&self, message: &[u8]) -> Result<[u8; 64], String>;
    fn public_key(&self) -> [u8; 32];
    fn card_serial(&self) -> &str;
}

impl HardwareSigner for aivyx_yubi::YubiKeySigner {
    fn sign(&self, message: &[u8]) -> Result<[u8; 64], String> {
        aivyx_yubi::YubiKeySigner::sign(self, message).map_err(|e| e.to_string())
    }

    fn public_key(&self) -> [u8; 32] {
        aivyx_yubi::YubiKeySigner::public_key(self)
    }

    fn card_serial(&self) -> &str {
        aivyx_yubi::YubiKeySigner::card_serial(self)
    }
}

/// The two backends an [`Identity`] can sign with. Never exposed publicly —
/// callers only ever see [`Identity`]'s own methods.
enum IdentitySigner {
    /// A software-generated key, sealed at rest under the storage
    /// [`MasterKey`] (see [`Identity::load_or_generate`]). Boxed: `SigningKey`
    /// is >200 bytes, and without this the whole enum pays that size for
    /// every `Hardware` instance too (`clippy::large_enum_variant`).
    Software(Box<SigningKey>),
    /// A hardware-backed signer (a YubiKey's OpenPGP card applet, or, in
    /// this crate's own tests, a fake — see [`HardwareSigner`]'s doc
    /// comment).
    Hardware(Box<dyn HardwareSigner>),
}

/// An operator-owned Ed25519 federation identity: `instance_id` + keypair,
/// backed by either a software key or a hardware signer (see
/// [`IdentitySigner`]).
///
/// `Debug` is implemented manually so the signing key is **never** rendered.
pub struct Identity {
    instance_id: String,
    signer: IdentitySigner,
    verifying_key: VerifyingKey,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Redact just as thoroughly for the hardware path as for software:
        // no PIN state or key material is ever printed. `card_serial()`
        // isn't itself secret, but the conservative posture this file
        // documents (FED.0 §7) applies to the signer as a whole, not just
        // the software case — never print anything from `self.signer` here.
        f.debug_struct("Identity")
            .field("instance_id", &self.instance_id)
            .field("signer", &"[redacted]")
            .field("verifying_key", &self.public_key_base64())
            .finish()
    }
}

impl Identity {
    /// Build from an existing signing key. Validates `instance_id`.
    pub fn new(instance_id: String, signing_key: SigningKey) -> Result<Self, FederationError> {
        validate_instance_id(&instance_id)?;
        let verifying_key = signing_key.verifying_key();
        Ok(Self {
            instance_id,
            signer: IdentitySigner::Software(Box::new(signing_key)),
            verifying_key,
        })
    }

    /// Generate a fresh random keypair for `instance_id`.
    pub fn generate(instance_id: String) -> Result<Self, FederationError> {
        let mut rng = rand::thread_rng();
        Self::new(instance_id, SigningKey::generate(&mut rng))
    }

    /// Build an `Identity` backed by a YubiKey's OpenPGP card applet instead
    /// of a software-generated key. `signer` must already be constructed —
    /// obtaining the User PIN and calling `aivyx_yubi::YubiKeySigner::new`
    /// is the caller's responsibility (Task 8's CLI, not this crate); this
    /// keeps `aivyx-federation` from needing to know anything about PIN
    /// acquisition UX. `expected_serial` is the card serial this identity
    /// was originally provisioned against (persisted by that same caller) —
    /// if the currently connected card's serial doesn't match, this fails
    /// loudly rather than silently trusting a different physical device.
    pub fn load_hardware(
        instance_id: String,
        signer: aivyx_yubi::YubiKeySigner,
        expected_serial: &str,
    ) -> Result<Self, FederationError> {
        Self::from_hardware_signer(instance_id, Box::new(signer), expected_serial)
    }

    /// The shared core of [`Self::load_hardware`], generic over
    /// [`HardwareSigner`] so this crate's own tests can exercise it against
    /// a fake (see [`HardwareSigner`]'s doc comment for why a real
    /// `aivyx_yubi::YubiKeySigner` can't be fabricated in a test).
    fn from_hardware_signer(
        instance_id: String,
        signer: Box<dyn HardwareSigner>,
        expected_serial: &str,
    ) -> Result<Self, FederationError> {
        validate_instance_id(&instance_id)?;
        if signer.card_serial() != expected_serial {
            return Err(FederationError::Hardware(format!(
                "wrong YubiKey inserted: expected card serial {expected_serial}, found {}",
                signer.card_serial()
            )));
        }
        let public_key_bytes = signer.public_key();
        let verifying_key = VerifyingKey::from_bytes(&public_key_bytes).map_err(|e| {
            FederationError::Hardware(format!("invalid Ed25519 key from card: {e}"))
        })?;
        Ok(Self {
            instance_id,
            signer: IdentitySigner::Hardware(signer),
            verifying_key,
        })
    }

    /// Test-only mirror of [`Self::load_hardware`] that accepts any
    /// [`HardwareSigner`] (a fake, in practice) instead of a concrete
    /// `aivyx_yubi::YubiKeySigner` — see [`HardwareSigner`]'s doc comment
    /// for why the real type can't be constructed in a test.
    #[cfg(test)]
    fn load_hardware_for_test(
        instance_id: String,
        signer: impl HardwareSigner + 'static,
        expected_serial: &str,
    ) -> Result<Self, FederationError> {
        Self::from_hardware_signer(instance_id, Box::new(signer), expected_serial)
    }

    /// This instance's public key as base64 — what a peer records to verify us.
    pub fn public_key_base64(&self) -> String {
        BASE64.encode(self.verifying_key.as_bytes())
    }

    /// This instance's id.
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Sign a request body, producing a [`SignedHeader`]. Async because the
    /// hardware-backed path can legitimately block for several seconds
    /// waiting on a physical touch — callers must `.await` this even though
    /// the software-backed path completes synchronously in practice. Takes
    /// `&self` (not `&mut self`): neither backend mutates any state on
    /// `self` while signing (`aivyx_yubi::YubiKeySigner::sign` is itself
    /// `&self` for exactly this reason — see its own doc comment), so
    /// there's no need to force callers into a `Mutex<Identity>` to hold a
    /// `&mut` across a multi-second `.await`.
    pub async fn sign_request(&self, body: &[u8]) -> Result<SignedHeader, FederationError> {
        let timestamp = now_secs();
        let body_hash = sha256_hex(body);
        let message = format!("{}:{}:{}", self.instance_id, timestamp, body_hash);
        let signature_bytes = match &self.signer {
            IdentitySigner::Software(k) => k.sign(message.as_bytes()).to_bytes(),
            IdentitySigner::Hardware(h) => {
                h.sign(message.as_bytes()).map_err(FederationError::Hardware)?
            }
        };
        Ok(SignedHeader {
            instance_id: self.instance_id.clone(),
            timestamp,
            signature: BASE64.encode(signature_bytes),
        })
    }

    /// Verify a peer's signed request: fresh timestamp + valid signature over
    /// `id:timestamp:body_hash` under `peer_public_key` (base64). Pair with a
    /// [`ReplayGuard`] to reject replays within the freshness window.
    pub fn verify_request(
        peer_public_key: &str,
        header: &SignedHeader,
        body: &[u8],
    ) -> Result<(), FederationError> {
        if now_secs().saturating_sub(header.timestamp) > MAX_REQUEST_AGE_SECS {
            return Err(FederationError::Identity("federation request expired".into()));
        }
        let key_bytes = BASE64
            .decode(peer_public_key)
            .map_err(|e| FederationError::Identity(format!("invalid peer public key: {e}")))?;
        let key_array: [u8; 32] = key_bytes
            .try_into()
            .map_err(|_| FederationError::Identity("peer public key must be 32 bytes".into()))?;
        let verifying_key = VerifyingKey::from_bytes(&key_array)
            .map_err(|e| FederationError::Identity(format!("invalid Ed25519 key: {e}")))?;
        let sig_bytes = BASE64
            .decode(&header.signature)
            .map_err(|e| FederationError::Identity(format!("invalid signature encoding: {e}")))?;
        let sig_array: [u8; 64] = sig_bytes
            .try_into()
            .map_err(|_| FederationError::Identity("signature must be 64 bytes".into()))?;
        let signature = ed25519_dalek::Signature::from_bytes(&sig_array);
        let body_hash = sha256_hex(body);
        let message = format!("{}:{}:{}", header.instance_id, header.timestamp, body_hash);
        verifying_key
            .verify(message.as_bytes(), &signature)
            .map_err(|_| FederationError::Identity("federation signature verification failed".into()))
    }

    /// Load the identity from an encrypted key file, or generate + seal + save
    /// it if absent. The key at rest is sealed with a subkey derived from
    /// `master` (the storage [`MasterKey`]) — same root secret as the redb
    /// store, so the operator's identity is as protected as their data. The
    /// file is `nonce(12) || ciphertext+tag(48)`, mode `0o600`.
    pub fn load_or_generate(
        instance_id: String,
        key_path: &Path,
        master: &MasterKey,
    ) -> Result<Self, FederationError> {
        let subkey = master
            .derive_subkey(KEY_WRAP_INFO)
            .map_err(|e| FederationError::Identity(format!("derive identity wrap key: {e}")))?;

        if key_path.exists() {
            let data = std::fs::read(key_path)
                .map_err(|e| FederationError::Identity(format!("read federation key: {e}")))?;
            if data.len() != ENCRYPTED_KEY_LEN {
                return Err(FederationError::Identity(format!(
                    "federation key file has unexpected size {} (expected {ENCRYPTED_KEY_LEN})",
                    data.len()
                )));
            }
            let plaintext = subkey
                .open(&data[..NONCE_LEN], KEY_WRAP_AAD, &data[NONCE_LEN..])
                .map_err(|_| FederationError::Identity("federation key decryption failed".into()))?;
            let key_bytes: [u8; 32] = plaintext
                .try_into()
                .map_err(|_| FederationError::Identity("decrypted key is not 32 bytes".into()))?;
            Self::new(instance_id, SigningKey::from_bytes(&key_bytes))
        } else {
            let identity = Self::generate(instance_id)?;
            identity.save_sealed(key_path, &subkey)?;
            Ok(identity)
        }
    }

    /// Seal the signing key under `subkey` and write `nonce||ciphertext` at
    /// `0o600`. Only ever called from [`Self::load_or_generate`] right after
    /// [`Self::generate`], so `self.signer` is always [`IdentitySigner::
    /// Software`] in practice — a hardware-backed `Identity` has no software
    /// key to seal, so that case returns an error rather than panicking.
    fn save_sealed(
        &self,
        key_path: &Path,
        subkey: &aivyx_crypto::SubKey,
    ) -> Result<(), FederationError> {
        let IdentitySigner::Software(signing_key) = &self.signer else {
            return Err(FederationError::Identity(
                "cannot seal a hardware-backed identity's key to disk (no software key exists)"
                    .into(),
            ));
        };
        if let Some(parent) = key_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| FederationError::Identity(format!("create key dir: {e}")))?;
        }
        let mut nonce = [0u8; NONCE_LEN];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);
        let ciphertext = subkey
            .seal(&nonce, KEY_WRAP_AAD, &signing_key.to_bytes())
            .map_err(|e| FederationError::Identity(format!("seal federation key: {e}")))?;
        let mut file_data = Vec::with_capacity(ENCRYPTED_KEY_LEN);
        file_data.extend_from_slice(&nonce);
        file_data.extend_from_slice(&ciphertext);
        std::fs::write(key_path, &file_data)
            .map_err(|e| FederationError::Identity(format!("write federation key: {e}")))?;
        set_file_permissions_600(key_path)?;
        Ok(())
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(data))
}

/// Owner-only (`0o600`) on Unix; no-op elsewhere — keeps the private key
/// unreadable by other users on the host.
#[cfg(unix)]
fn set_file_permissions_600(path: &Path) -> Result<(), FederationError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| FederationError::Identity(format!("set key file permissions: {e}")))
}

#[cfg(not(unix))]
fn set_file_permissions_600(_path: &Path) -> Result<(), FederationError> {
    Ok(())
}

/// Instance ids appear in signed headers, audit payloads, and logs — restrict
/// to ASCII alphanumeric + `-`/`_` so they can't inject into any of those.
fn validate_instance_id(id: &str) -> Result<(), FederationError> {
    if id.is_empty() {
        return Err(FederationError::Validation(
            "federation instance_id must not be empty".into(),
        ));
    }
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(FederationError::Validation(format!(
            "federation instance_id contains invalid characters: '{id}' \
             (only ASCII alphanumeric, hyphens, and underscores allowed)"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_key_path(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("aivyx-fed-id-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("federation.key")
    }

    /// A fake [`HardwareSigner`] standing in for a real `aivyx_yubi::
    /// YubiKeySigner` — see that trait's doc comment for why the real type
    /// can't be fabricated from outside `aivyx-yubi`.
    struct FakeHardwareSigner {
        card_serial: String,
        public_key: [u8; 32],
        signing_key: SigningKey,
        fail_with: Option<String>,
    }

    impl FakeHardwareSigner {
        fn new(card_serial: &str) -> Self {
            let mut rng = rand::thread_rng();
            let signing_key = SigningKey::generate(&mut rng);
            let public_key = signing_key.verifying_key().to_bytes();
            Self {
                card_serial: card_serial.to_string(),
                public_key,
                signing_key,
                fail_with: None,
            }
        }

        /// Make `sign()` fail with `message`, simulating a `YubiError`
        /// (e.g. a touch timeout or PIN rejection) instead of producing a
        /// real signature.
        fn failing(mut self, message: &str) -> Self {
            self.fail_with = Some(message.to_string());
            self
        }
    }

    impl HardwareSigner for FakeHardwareSigner {
        fn sign(&self, message: &[u8]) -> Result<[u8; 64], String> {
            if let Some(err) = &self.fail_with {
                return Err(err.clone());
            }
            Ok(self.signing_key.sign(message).to_bytes())
        }

        fn public_key(&self) -> [u8; 32] {
            self.public_key
        }

        fn card_serial(&self) -> &str {
            &self.card_serial
        }
    }

    #[tokio::test]
    async fn sign_and_verify_roundtrips() {
        let id = Identity::generate("test-instance".into()).unwrap();
        let body = b"hello federation";
        let header = id.sign_request(body).await.unwrap();
        Identity::verify_request(&id.public_key_base64(), &header, body)
            .expect("verification should pass");
    }

    #[tokio::test]
    async fn rejects_tampered_body() {
        let id = Identity::generate("test-instance".into()).unwrap();
        let header = id.sign_request(b"original body").await.unwrap();
        assert!(Identity::verify_request(&id.public_key_base64(), &header, b"tampered").is_err());
    }

    #[tokio::test]
    async fn rejects_expired_request() {
        let id = Identity::generate("test-instance".into()).unwrap();
        let mut header = id.sign_request(b"body").await.unwrap();
        header.timestamp -= MAX_REQUEST_AGE_SECS + 10;
        assert!(Identity::verify_request(&id.public_key_base64(), &header, b"body").is_err());
    }

    #[tokio::test]
    async fn rejects_wrong_peer_key() {
        let a = Identity::generate("instance-a".into()).unwrap();
        let b = Identity::generate("instance-b".into()).unwrap();
        let header = a.sign_request(b"secret").await.unwrap();
        // verifying A's signature with B's key must fail
        assert!(Identity::verify_request(&b.public_key_base64(), &header, b"secret").is_err());
    }

    #[tokio::test]
    async fn signature_is_64_bytes_and_header_is_well_formed() {
        let id = Identity::generate("struct-test".into()).unwrap();
        let header = id.sign_request(b"payload").await.unwrap();
        assert_eq!(header.instance_id, "struct-test");
        assert_eq!(BASE64.decode(&header.signature).unwrap().len(), 64);
    }

    #[tokio::test]
    async fn replay_guard_rejects_second_use() {
        let id = Identity::generate("replay".into()).unwrap();
        let header = id.sign_request(b"once").await.unwrap();
        let guard = ReplayGuard::new();
        assert!(guard.check_and_record(&header).is_ok());
        assert!(guard.check_and_record(&header).is_err(), "replay must be rejected");
    }

    #[test]
    fn debug_redacts_the_signing_key() {
        let id = Identity::generate("redact-test".into()).unwrap();
        let dbg = format!("{id:?}");
        assert!(dbg.contains("[redacted]"));
        assert!(dbg.contains("redact-test"));
        // the raw signing-key bytes must never appear
        let IdentitySigner::Software(signing_key) = &id.signer else {
            panic!("Identity::generate should always produce a software-backed signer");
        };
        let raw = format!("{:?}", signing_key.to_bytes());
        assert!(!dbg.contains(&raw));
    }

    #[test]
    fn debug_redacts_the_hardware_signer() {
        let fake = FakeHardwareSigner::new("0006:00112233");
        let id = Identity::load_hardware_for_test("hw-redact".into(), fake, "0006:00112233")
            .unwrap();
        let dbg = format!("{id:?}");
        assert!(dbg.contains("[redacted]"));
        assert!(dbg.contains("hw-redact"));
        // the card serial (not secret, but conservatively redacted anyway
        // per FED.0 §7 -- see `Identity`'s manual `Debug` impl) must not
        // leak through the signer field.
        assert!(!dbg.contains("0006:00112233"));
    }

    #[tokio::test]
    async fn hardware_backed_sign_request_verifies_like_the_software_path() {
        let fake = FakeHardwareSigner::new("0006:00112233");
        let id =
            Identity::load_hardware_for_test("hw-instance".into(), fake, "0006:00112233").unwrap();
        let header = id.sign_request(b"hello from hardware").await.unwrap();
        Identity::verify_request(&id.public_key_base64(), &header, b"hello from hardware")
            .expect("a hardware-backed signature must verify identically to a software one");
    }

    #[test]
    fn load_hardware_rejects_a_mismatched_card_serial() {
        let fake = FakeHardwareSigner::new("0006:00112233");
        let err = Identity::load_hardware_for_test(
            "hw-mismatch".into(),
            fake,
            "0006:99999999", // a different serial than the fake reports
        )
        .expect_err("a mismatched card serial must be rejected");
        assert!(
            matches!(err, FederationError::Hardware(_)),
            "expected FederationError::Hardware, got {err:?}"
        );
    }

    #[tokio::test]
    async fn hardware_signing_failure_maps_to_federation_error_hardware() {
        let fake = FakeHardwareSigner::new("0006:00112233").failing("touch timeout");
        let id =
            Identity::load_hardware_for_test("hw-fails".into(), fake, "0006:00112233").unwrap();
        let err = id
            .sign_request(b"anything")
            .await
            .expect_err("a hardware signing failure must propagate as an Err, not a panic");
        match err {
            FederationError::Hardware(msg) => assert_eq!(msg, "touch timeout"),
            other => panic!("expected FederationError::Hardware, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn encrypted_key_roundtrips_via_masterkey() {
        let master = MasterKey::from_raw([7u8; 32]);
        let path = tmp_key_path("rt");
        // generate + seal
        let a = Identity::load_or_generate("enc".into(), &path, &master).unwrap();
        let pubkey = a.public_key_base64();
        // file is exactly nonce(12)+ciphertext+tag(48)
        assert_eq!(std::fs::read(&path).unwrap().len(), ENCRYPTED_KEY_LEN);
        // reload yields the same identity + signing still verifies
        let b = Identity::load_or_generate("enc".into(), &path, &master).unwrap();
        assert_eq!(b.public_key_base64(), pubkey);
        let header = b.sign_request(b"x").await.unwrap();
        Identity::verify_request(&pubkey, &header, b"x").unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn wrong_masterkey_cannot_open_the_identity() {
        let master = MasterKey::from_raw([7u8; 32]);
        let wrong = MasterKey::from_raw([9u8; 32]);
        let path = tmp_key_path("wrong");
        Identity::load_or_generate("enc".into(), &path, &master).unwrap();
        assert!(Identity::load_or_generate("enc".into(), &path, &wrong).is_err());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[cfg(unix)]
    #[test]
    fn key_file_is_600() {
        use std::os::unix::fs::PermissionsExt;
        let master = MasterKey::from_raw([7u8; 32]);
        let path = tmp_key_path("perms");
        Identity::load_or_generate("perms".into(), &path, &master).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "key file must be 0o600, got {mode:o}");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn rejects_empty_and_malformed_instance_ids() {
        assert!(Identity::generate(String::new()).is_err());
        assert!(Identity::generate("bad id/slash".into()).is_err());
        assert!(Identity::generate("good-id_42".into()).is_ok());
    }
}

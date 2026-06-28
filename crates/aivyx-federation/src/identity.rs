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

/// An operator-owned Ed25519 federation identity: `instance_id` + keypair.
///
/// `Debug` is implemented manually so the signing key is **never** rendered.
pub struct Identity {
    instance_id: String,
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("instance_id", &self.instance_id)
            .field("signing_key", &"[redacted]")
            .field("verifying_key", &self.public_key_base64())
            .finish()
    }
}

impl Identity {
    /// Build from an existing signing key. Validates `instance_id`.
    pub fn new(instance_id: String, signing_key: SigningKey) -> Result<Self, FederationError> {
        validate_instance_id(&instance_id)?;
        let verifying_key = signing_key.verifying_key();
        Ok(Self { instance_id, signing_key, verifying_key })
    }

    /// Generate a fresh random keypair for `instance_id`.
    pub fn generate(instance_id: String) -> Result<Self, FederationError> {
        let mut rng = rand::thread_rng();
        Self::new(instance_id, SigningKey::generate(&mut rng))
    }

    /// This instance's public key as base64 — what a peer records to verify us.
    pub fn public_key_base64(&self) -> String {
        BASE64.encode(self.verifying_key.as_bytes())
    }

    /// This instance's id.
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Sign a request body, producing a [`SignedHeader`].
    pub fn sign_request(&self, body: &[u8]) -> SignedHeader {
        let timestamp = now_secs();
        let body_hash = sha256_hex(body);
        let message = format!("{}:{}:{}", self.instance_id, timestamp, body_hash);
        let signature = self.signing_key.sign(message.as_bytes());
        SignedHeader {
            instance_id: self.instance_id.clone(),
            timestamp,
            signature: BASE64.encode(signature.to_bytes()),
        }
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
    /// `0o600`.
    fn save_sealed(
        &self,
        key_path: &Path,
        subkey: &aivyx_crypto::SubKey,
    ) -> Result<(), FederationError> {
        if let Some(parent) = key_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| FederationError::Identity(format!("create key dir: {e}")))?;
        }
        let mut nonce = [0u8; NONCE_LEN];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);
        let ciphertext = subkey
            .seal(&nonce, KEY_WRAP_AAD, &self.signing_key.to_bytes())
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

    #[test]
    fn sign_and_verify_roundtrips() {
        let id = Identity::generate("test-instance".into()).unwrap();
        let body = b"hello federation";
        let header = id.sign_request(body);
        Identity::verify_request(&id.public_key_base64(), &header, body)
            .expect("verification should pass");
    }

    #[test]
    fn rejects_tampered_body() {
        let id = Identity::generate("test-instance".into()).unwrap();
        let header = id.sign_request(b"original body");
        assert!(Identity::verify_request(&id.public_key_base64(), &header, b"tampered").is_err());
    }

    #[test]
    fn rejects_expired_request() {
        let id = Identity::generate("test-instance".into()).unwrap();
        let mut header = id.sign_request(b"body");
        header.timestamp -= MAX_REQUEST_AGE_SECS + 10;
        assert!(Identity::verify_request(&id.public_key_base64(), &header, b"body").is_err());
    }

    #[test]
    fn rejects_wrong_peer_key() {
        let a = Identity::generate("instance-a".into()).unwrap();
        let b = Identity::generate("instance-b".into()).unwrap();
        let header = a.sign_request(b"secret");
        // verifying A's signature with B's key must fail
        assert!(Identity::verify_request(&b.public_key_base64(), &header, b"secret").is_err());
    }

    #[test]
    fn signature_is_64_bytes_and_header_is_well_formed() {
        let id = Identity::generate("struct-test".into()).unwrap();
        let header = id.sign_request(b"payload");
        assert_eq!(header.instance_id, "struct-test");
        assert_eq!(BASE64.decode(&header.signature).unwrap().len(), 64);
    }

    #[test]
    fn replay_guard_rejects_second_use() {
        let id = Identity::generate("replay".into()).unwrap();
        let header = id.sign_request(b"once");
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
        let raw = format!("{:?}", id.signing_key.to_bytes());
        assert!(!dbg.contains(&raw));
    }

    #[test]
    fn encrypted_key_roundtrips_via_masterkey() {
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
        let header = b.sign_request(b"x");
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

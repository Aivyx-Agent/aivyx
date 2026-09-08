//! `aivyx federation` CLI surface — hardware-backed federation identity
//! provisioning (Chapter Passport / `docs/FEDERATION.md`, Task 8).
//!
//! This whole module is compiled only under the `yubikey` Cargo feature
//! (see `aivyx.rs`'s `mod federation;` declaration and its dispatch arm's
//! `#[cfg(not(feature = "yubikey"))]` fallback): both dependencies this
//! module names directly (`aivyx-yubi`, and `aivyx-federation` built with
//! its own `yubikey` feature) transitively need `libpcsclite` (via
//! `pcsc-sys`) at build time, which isn't installed in the default CI job
//! or on every contributor's machine — a plain `cargo build -p aivyx-cli`
//! must never require it. See `aivyx-cli`'s own `Cargo.toml` for the
//! feature wiring.
//!
//! # Provisioning flow (`run_yubikey_init`)
//!
//! 1. Discover the connected YubiKey's OpenPGP card
//!    (`aivyx_yubi::discovery::discover_real_card`). Requires `pcscd`
//!    running and a card inserted.
//! 2. Refuse if the card's User and/or Admin PIN is still the OpenPGP-card
//!    factory default (`aivyx_yubi::pin::require_pin_changed`). This
//!    command **never** changes a PIN on the operator's behalf — Chapter
//!    Passport's design spec's Global Constraints require refusing to
//!    proceed instead, and `pin::require_pin_changed` must only be called
//!    once per attempt (see its own doc comment on why: repeated calls
//!    burn real PIN retry attempts, and the Admin PIN has no self-recovery
//!    once blocked).
//! 3. Verify the (already-changed) Admin PIN, prompted interactively via
//!    `rpassword` (input hidden, never passed as a CLI argument or
//!    logged).
//! 4. Generate a fresh Ed25519 keypair in the Signature slot
//!    (`aivyx_yubi::provision::generate_signature_key`) — **destructive**
//!    if the slot already holds a key; see that function's own doc
//!    comment. This command assumes a freshly-reset or never-before-
//!    provisioned card, per the design spec.
//! 5. Set the Signature slot's touch-policy to `Fixed`
//!    (`aivyx_yubi::provision::set_signature_touch_policy_fixed`) — every
//!    future signature requires a physical touch, with no PIN-only
//!    fast-path.
//! 6. Build the binding record (`{instance_id, card_serial,
//!    public_key_base64}`) from data already in hand and write it to
//!    `key_binding_path` as plain (non-secret) JSON.
//! 7. As a closing sanity check, round-trip through
//!    `aivyx_federation::Identity::load_hardware` — the exact production
//!    load path a daemon will use later — confirming the freshly
//!    provisioned card's public key and serial are accepted by
//!    `aivyx-federation`'s own validation. This re-discovers the card via
//!    a fresh `aivyx_yubi::YubiKeySigner::new`, which never presents a PIN
//!    to the card at construction time (only a later `sign()` call would
//!    — see that type's own doc comment), so an empty placeholder PIN is
//!    used here and is never sent to the card. The binding record file
//!    from step 6 is already written by this point — a failure here is
//!    reported as an error, but does not un-write it (provisioning is
//!    already real and irreversible on the card by this point).

use std::path::Path;

use aivyx_yubi::{SecretString, YubiError, YubiKeySigner, discovery, pin, provision};
use base64::Engine as _;
use serde::Serialize;

/// The on-disk binding record `yubikey-init` produces. Plain JSON — no
/// secret or private key material, per the design spec (contrast with
/// `aivyx_federation::Identity`'s software-backed key file, which *is*
/// sealed at rest).
#[derive(Debug, Serialize)]
struct KeyBindingRecord {
    instance_id: String,
    card_serial: String,
    public_key_base64: String,
}

/// Entry point for `aivyx federation yubikey-init <instance-id>
/// <key-binding-path>`. See this module's doc comment for the full flow.
pub fn run_yubikey_init(instance_id: &str, key_binding_path: &Path) -> Result<(), String> {
    // Fail fast on an obviously-bad instance id before touching the card
    // at all (card discovery + the PIN-factory-default check below both
    // cost real, limited PIN retry attempts on a card whose PINs are
    // already correctly changed — see `pin::require_pin_changed`'s doc
    // comment). `aivyx_federation::Identity::load_hardware` re-validates
    // this authoritatively at the end regardless (step 7); this is just a
    // cheap early exit for the common "forgot the argument" mistake.
    if instance_id.is_empty() {
        return Err("aivyx federation yubikey-init: instance-id must not be empty".to_string());
    }

    eprintln!("aivyx federation yubikey-init: discovering YubiKey (requires pcscd running)...");
    let mut card = discovery::discover_real_card()
        .map_err(|e| format!("aivyx federation yubikey-init: {e}"))?;

    let mut tx = card.transaction().map_err(YubiError::from).map_err(|e| {
        format!("aivyx federation yubikey-init: failed to open a card transaction: {e}")
    })?;

    // Refuse on a still-factory-default PIN rather than changing it
    // ourselves — see this module's doc comment (step 2) and
    // `pin::require_pin_changed`'s own doc comment for why this is called
    // exactly once here, not in a retry loop.
    pin::require_pin_changed(&mut tx).map_err(|e| {
        format!(
            "aivyx federation yubikey-init: {e}\n\n\
             This command never changes a PIN on your behalf — change both the \
             User and Admin PIN first via the standard OpenPGP-card PIN-change \
             command (e.g. `gpg --card-edit`, then `admin`, then `passwd`), then \
             retry `aivyx federation yubikey-init`."
        )
    })?;

    let admin_pin = rpassword::prompt_password("Admin PIN (input hidden): ")
        .map_err(|e| format!("aivyx federation yubikey-init: failed to read Admin PIN: {e}"))?;
    let mut admin = tx
        .as_admin_card(SecretString::from(admin_pin))
        .map_err(YubiError::from)
        .map_err(|e| {
            format!("aivyx federation yubikey-init: Admin PIN verification failed: {e}")
        })?;

    eprintln!(
        "aivyx federation yubikey-init: generating an Ed25519 keypair in the Signature slot \
         (this overwrites any existing key in that slot)..."
    );
    let public_key = provision::generate_signature_key(&mut admin).map_err(|e| {
        format!("aivyx federation yubikey-init: Signature-slot key generation failed: {e}")
    })?;

    eprintln!(
        "aivyx federation yubikey-init: setting the Signature slot's touch policy to fixed \
         (every future signature will require a physical touch)..."
    );
    provision::set_signature_touch_policy_fixed(&mut admin).map_err(|e| {
        format!("aivyx federation yubikey-init: setting the touch policy failed: {e}")
    })?;

    // `admin`'s last use was the call directly above -- NLL ends its
    // mutable borrow of `tx` here, freeing `tx` to read the serial
    // directly (same pattern `aivyx-yubi`'s own
    // `sets_the_signature_touch_policy_to_fixed` test uses).
    let card_serial = discovery::read_serial(&mut tx).map_err(|e| {
        format!("aivyx federation yubikey-init: failed to read the card's serial: {e}")
    })?;

    let public_key_base64 = base64::engine::general_purpose::STANDARD.encode(public_key.as_ref());

    let record = KeyBindingRecord {
        instance_id: instance_id.to_string(),
        card_serial: card_serial.clone(),
        public_key_base64,
    };
    write_binding_record(key_binding_path, &record)?;
    eprintln!(
        "aivyx federation yubikey-init: wrote binding record ({{instance_id: {}, card_serial: \
         {}}}) to {}",
        record.instance_id,
        record.card_serial,
        key_binding_path.display(),
    );

    // Closing sanity check (step 7 in this module's doc comment): confirm
    // the exact production load path (`Identity::load_hardware`) accepts
    // what we just provisioned. The binding record above is already
    // written by this point regardless of this check's outcome — the
    // card's state is already real and irreversible.
    let verifying_signer = YubiKeySigner::new(SecretString::from(String::new())).map_err(|e| {
        format!(
            "aivyx federation yubikey-init: wrote {} but a fresh re-discovery for verification \
             failed: {e}",
            key_binding_path.display(),
        )
    })?;
    let identity = aivyx_federation::identity::Identity::load_hardware(
        instance_id.to_string(),
        verifying_signer,
        &card_serial,
    )
    .map_err(|e| {
        format!(
            "aivyx federation yubikey-init: wrote {} but the provisioned identity failed \
             verification against aivyx-federation's own load path: {e}",
            key_binding_path.display(),
        )
    })?;
    if identity.public_key_base64() != record.public_key_base64 {
        return Err(format!(
            "aivyx federation yubikey-init: wrote {} but the verification pass read back a \
             different public key than provisioning reported -- this should not happen; please \
             report this as a bug",
            key_binding_path.display(),
        ));
    }

    eprintln!(
        "aivyx federation yubikey-init: verified — instance `{}` is bound to card {} and \
         loads correctly via aivyx-federation's own Identity::load_hardware.",
        identity.instance_id(),
        card_serial,
    );
    Ok(())
}

/// Write `record` as pretty-printed JSON to `path`, creating parent
/// directories if needed. Plain permissions (not `0600`) — the record is
/// deliberately non-secret (see this module's doc comment).
fn write_binding_record(path: &Path, record: &KeyBindingRecord) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create parent dir {}: {e}", parent.display()))?;
        }
    }
    let json = serde_json::to_string_pretty(record)
        .map_err(|e| format!("failed to serialize the key binding record: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_binding_record_creates_parent_dirs_and_pretty_json() {
        let dir = std::env::temp_dir().join(format!(
            "aivyx-federation-cli-test-{}",
            uuid::Uuid::new_v4()
        ));
        let path = dir.join("nested").join("binding.json");
        let record = KeyBindingRecord {
            instance_id: "test-instance".to_string(),
            card_serial: "0006:00112233".to_string(),
            public_key_base64: "abc123==".to_string(),
        };

        write_binding_record(&path, &record).expect("write should succeed");

        let contents = std::fs::read_to_string(&path).expect("read back");
        assert!(contents.contains("\"instance_id\": \"test-instance\""));
        assert!(contents.contains("\"card_serial\": \"0006:00112233\""));
        assert!(contents.contains("\"public_key_base64\": \"abc123==\""));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_instance_id_is_rejected_before_touching_any_card() {
        let path = std::env::temp_dir().join("aivyx-federation-cli-test-unused.json");
        let err = run_yubikey_init("", &path).expect_err("an empty instance id must be rejected");
        assert!(
            err.contains("instance-id must not be empty"),
            "error: {err}"
        );
        // Must not have written anything -- the empty-id check runs
        // before any card I/O or file write.
        assert!(!path.exists());
    }
}

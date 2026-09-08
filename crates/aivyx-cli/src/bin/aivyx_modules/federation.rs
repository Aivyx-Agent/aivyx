//! `aivyx-pa federation` CLI surface — hardware-backed federation identity
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
//!
//!    **This step opens a second, independent PC/SC connection to the same
//!    physical reader.** The provisioning transaction and card handle from
//!    steps 1-6 are explicitly `drop`ped before this step runs (Finding
//!    C-1) — `SCardBeginTransaction` blocks indefinitely (it does not fail
//!    fast) if another exclusive transaction is still held on the same
//!    reader, so failing to release the first transaction first would hang
//!    this command forever right after the card has already been
//!    irreversibly re-keyed.
//!
//!    **What this step does NOT verify**: whether the touch-policy setting
//!    from step 5 actually took effect live on the card. `YubiKeySigner::
//!    sign` hard-refuses if the live touch policy isn't `Fixed`, but doing
//!    that check here would require collecting the User PIN and a real
//!    physical touch, which isn't this provisioning command's job (Finding
//!    I-1) — see this step's own user-facing message for the accurate,
//!    non-overclaiming description of what was and wasn't confirmed.

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

/// Entry point for `aivyx-pa federation yubikey-init <instance-id>
/// <key-binding-path>`. See this module's doc comment for the full flow.
pub fn run_yubikey_init(instance_id: &str, key_binding_path: &Path) -> Result<(), String> {
    // Fail fast on an invalid instance id before touching the card at all
    // (card discovery + the PIN-factory-default check below both cost
    // real, limited PIN retry attempts on a card whose PINs are already
    // correctly changed — see `pin::require_pin_changed`'s doc comment —
    // and key generation below is destructive and irreversible on the
    // card itself). Reuses `aivyx_federation::identity::validate_instance_id`
    // directly (Finding I-2) rather than duplicating its character-class
    // rule inline, so this early check can never drift out of sync with
    // the authoritative rule `Identity::load_hardware` re-applies at the
    // end regardless (step 7).
    aivyx_federation::identity::validate_instance_id(instance_id)
        .map_err(|e| format!("aivyx-pa federation yubikey-init: {e}"))?;

    eprintln!("aivyx-pa federation yubikey-init: discovering YubiKey (requires pcscd running)...");
    let mut card = discovery::discover_real_card()
        .map_err(|e| format!("aivyx-pa federation yubikey-init: {e}"))?;

    let mut tx = card.transaction().map_err(YubiError::from).map_err(|e| {
        format!("aivyx-pa federation yubikey-init: failed to open a card transaction: {e}")
    })?;

    // Refuse on a still-factory-default PIN rather than changing it
    // ourselves — see this module's doc comment (step 2) and
    // `pin::require_pin_changed`'s own doc comment for why this is called
    // exactly once here, not in a retry loop.
    pin::require_pin_changed(&mut tx).map_err(|e| {
        format!(
            "aivyx-pa federation yubikey-init: {e}\n\n\
             This command never changes a PIN on your behalf — change both the \
             User and Admin PIN first via the standard OpenPGP-card PIN-change \
             command (e.g. `gpg --card-edit`, then `admin`, then `passwd`), then \
             retry `aivyx-pa federation yubikey-init`."
        )
    })?;

    let admin_pin = rpassword::prompt_password("Admin PIN (input hidden): ")
        .map_err(|e| format!("aivyx-pa federation yubikey-init: failed to read Admin PIN: {e}"))?;
    let mut admin = tx
        .as_admin_card(SecretString::from(admin_pin))
        .map_err(YubiError::from)
        .map_err(|e| {
            // `as_admin_card`'s failure routes through `YubiError`'s
            // generic, context-free `From<openpgp_card::Error>` blanket
            // conversion (see that impl's own doc comment in `aivyx-yubi`)
            // -- a blocked PIN comes back labeled `pin_kind: "A"` ("A PIN
            // is blocked..."), even though this call site unambiguously
            // knows it's the ADMIN PIN that just failed. Rewrite that
            // specific case with an unambiguous, correctly-labeled message
            // and accurate (non-circular) recovery guidance -- never
            // "verify with the admin PIN", since that's exactly what's
            // blocked here (Finding I-4) -- instead of relying on the
            // generic fallback's ambiguous wording.
            let description = if matches!(&e, YubiError::PinBlocked { .. }) {
                "Admin PIN is blocked (too many failed attempts). This cannot be recovered \
                 with the Admin PIN itself -- only a pre-configured Reset Code (if one was \
                 set up) or a full card reset (TERMINATE+ACTIVATE, which erases all keys) can \
                 recover from this state."
                    .to_string()
            } else {
                format!("Admin PIN verification failed: {e}")
            };
            // Finding I-3: `pin::require_pin_changed` above and this
            // verification attempt each burn one of the Admin PIN's
            // limited real retries (`aivyx-yubi`'s own `pin.rs` documents
            // this at length) -- two mistakes, not three, can permanently
            // block it, with no self-recovery short of a full card wipe.
            // Nothing else in this command's output warns the operator of
            // that before they retry blindly.
            format!(
                "aivyx-pa federation yubikey-init: {description}\n\n\
                 Warning: this failed attempt just consumed one of the Admin PIN's limited \
                 real retry attempts, and the factory-default-PIN check that already ran \
                 earlier in this same command consumed one too. A blocked Admin PIN has NO \
                 self-recovery path short of a full card wipe (TERMINATE+ACTIVATE, which \
                 erases all existing keys) -- check your retry counter (e.g. `gpg \
                 --card-status`) before retrying blindly."
            )
        })?;

    eprintln!(
        "aivyx-pa federation yubikey-init: generating an Ed25519 keypair in the Signature slot \
         (this overwrites any existing key in that slot)..."
    );
    let public_key = provision::generate_signature_key(&mut admin).map_err(|e| {
        format!("aivyx-pa federation yubikey-init: Signature-slot key generation failed: {e}")
    })?;

    eprintln!(
        "aivyx-pa federation yubikey-init: setting the Signature slot's touch policy to fixed \
         (every future signature will require a physical touch)..."
    );
    provision::set_signature_touch_policy_fixed(&mut admin).map_err(|e| {
        format!("aivyx-pa federation yubikey-init: setting the touch policy failed: {e}")
    })?;

    // `admin`'s last use was the call directly above -- NLL ends its
    // mutable borrow of `tx` here, freeing `tx` to read the serial
    // directly (same pattern `aivyx-yubi`'s own
    // `sets_the_signature_touch_policy_to_fixed` test uses).
    let card_serial = discovery::read_serial(&mut tx).map_err(|e| {
        format!("aivyx-pa federation yubikey-init: failed to read the card's serial: {e}")
    })?;

    // Finding C-1 (CRITICAL): explicitly close the exclusive PC/SC
    // transaction (`tx`) and disconnect the underlying card handle
    // (`card`) now that all real card I/O for this provisioning attempt
    // is done -- BEFORE the verification pass below opens a *second*,
    // independent PC/SC connection to the same physical reader via
    // `YubiKeySigner::new`. `SCardBeginTransaction` (what that second
    // connection's own transaction call performs internally) blocks
    // indefinitely -- it does not fail fast -- if another exclusive
    // transaction is still held on the same reader. Without this, the
    // command would hang forever right here, after the card has already
    // been irreversibly re-keyed and the binding record already written,
    // with no way for the operator to tell whether provisioning actually
    // succeeded. `tx` borrows `card` mutably (`Card<Transaction<'_>>`), so
    // it must be dropped first.
    drop(tx);
    drop(card);

    let public_key_base64 = base64::engine::general_purpose::STANDARD.encode(public_key.as_ref());

    let record = KeyBindingRecord {
        instance_id: instance_id.to_string(),
        card_serial: card_serial.clone(),
        public_key_base64,
    };
    write_binding_record(key_binding_path, &record)?;
    eprintln!(
        "aivyx-pa federation yubikey-init: wrote binding record ({{instance_id: {}, card_serial: \
         {}}}) to {}",
        record.instance_id,
        record.card_serial,
        key_binding_path.display(),
    );

    // Closing sanity check (step 7 in this module's doc comment): confirm
    // the exact production load path (`Identity::load_hardware`) accepts
    // what we just provisioned. The binding record above is already
    // written by this point regardless of this check's outcome — the
    // card's state is already real and irreversible. Safe to open a fresh
    // PC/SC connection here: `tx`/`card` were already dropped above
    // (Finding C-1), so no exclusive transaction is still held on this
    // reader.
    let verifying_signer = YubiKeySigner::new(SecretString::from(String::new())).map_err(|e| {
        format!(
            "aivyx-pa federation yubikey-init: wrote {} but a fresh re-discovery for verification \
             failed: {e}",
            key_binding_path.display(),
        )
    })?;
    // NB (Finding I-1): `load_hardware`'s serial check compares the
    // freshly re-discovered card's serial against `card_serial` -- which
    // was itself just read from this exact same card, seconds earlier, in
    // this exact same run. That comparison can never meaningfully fail in
    // this flow; it isn't a "wrong card" check here (unlike in `sign()`'s
    // long-lived-signer use case where it genuinely guards against a card
    // swap). What this whole pass *does* meaningfully confirm is narrower:
    // the card is discoverable again, its serial and public key match what
    // provisioning itself just reported, and `Identity::load_hardware`
    // (the real production load path) accepts all of it end to end. It
    // does NOT confirm the touch policy set in step 5 is being enforced
    // live -- see the success message below for the honest, non-
    // overclaiming summary of what was and wasn't checked.
    let identity = aivyx_federation::identity::Identity::load_hardware(
        instance_id.to_string(),
        verifying_signer,
        &card_serial,
    )
    .map_err(|e| {
        format!(
            "aivyx-pa federation yubikey-init: wrote {} but the provisioned identity failed \
             verification against aivyx-federation's own load path: {e}",
            key_binding_path.display(),
        )
    })?;
    if identity.public_key_base64() != record.public_key_base64 {
        return Err(format!(
            "aivyx-pa federation yubikey-init: wrote {} but the verification pass read back a \
             different public key than provisioning reported -- this should not happen; please \
             report this as a bug",
            key_binding_path.display(),
        ));
    }

    eprintln!(
        "aivyx-pa federation yubikey-init: post-provisioning check passed — a fresh re-discovery \
         of card {1} confirms its serial and Signature-slot public key match what provisioning \
         just wrote for instance `{0}`, and aivyx-federation's own Identity::load_hardware \
         (the real production load path) accepts them. This does NOT confirm the touch policy \
         is being enforced live on the card -- that is confirmed the first time this identity \
         actually signs a real federation request, not by this init command.",
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
        // Finding M-3: a fixed temp filename, asserted not to exist, is
        // flaky on a shared `/tmp` (a leftover from a prior interrupted
        // run or another user could make this spuriously fail) -- mirror
        // `write_binding_record_creates_parent_dirs_and_pretty_json`'s own
        // UUID-based unique filename instead.
        let path = std::env::temp_dir().join(format!(
            "aivyx-federation-cli-test-{}.json",
            uuid::Uuid::new_v4()
        ));
        let err = run_yubikey_init("", &path).expect_err("an empty instance id must be rejected");
        assert!(err.contains("must not be empty"), "error: {err}");
        // Must not have written anything -- the instance-id check runs
        // before any card I/O or file write (Finding I-2).
        assert!(!path.exists());
    }

    #[test]
    fn invalid_instance_id_is_rejected_before_touching_any_card() {
        // Finding I-2: the fuller validation rule (only ASCII
        // alphanumeric, `-`, and `_`) must run up front too, not just the
        // empty-string case -- an id containing e.g. a space must never
        // reach card discovery/I/O.
        let path = std::env::temp_dir().join(format!(
            "aivyx-federation-cli-test-{}.json",
            uuid::Uuid::new_v4()
        ));
        let err = run_yubikey_init("bad id with spaces", &path)
            .expect_err("an instance id with invalid characters must be rejected");
        assert!(err.contains("invalid characters"), "error: {err}");
        // Must not have written anything -- the instance-id check runs
        // before any card I/O or file write.
        assert!(!path.exists());
    }
}

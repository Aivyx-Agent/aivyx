//! # aivyx-contacts
//!
//! Google Contacts (People API) third-party tool process for
//! Aivyx. **Chapter Contacts — the first Broaden-track domain**
//! (closes the contacts/CRM slice of backend-audit F4). Ships as
//! a separate binary the operator installs and wires into
//! `aivyx.toml` via `[[tool_process]]`. Per PRODUCT.md P10 (the
//! substrate is closed at thirteen tools forever; contacts is
//! third-party territory, like Gmail / Calendar / Drive).
//!
//! ## Layout
//!
//! - **OAuth substrate consumed from [`aivyx_google_oauth`]** —
//!   no in-tree OAuth copy. Contacts is the fifth Google
//!   integration; the lifted substrate is long paid for.
//! - [`auth_cli`] — `aivyx-contacts auth init / status /
//!   revoke` operator-facing CLI subcommand surface.
//! - [`contacts_client`] — Google People API v1 client
//!   (token-authenticated reqwest wrapper around
//!   `https://people.googleapis.com/v1/...`).
//! - [`tools`] — six `aivyx_core::Tool` impls (CT.3 + CT.4):
//!   - `contacts.search` (CT.3; `contacts.read`)
//!   - `contacts.list` (CT.3; `contacts.read`)
//!   - `contacts.get` (CT.3; `contacts.read`)
//!   - `contacts.create` (CT.4; `contacts.write`, CEILING_TRUSTED)
//!   - `contacts.update` (CT.4; `contacts.write`, CEILING_TRUSTED)
//!   - `contacts.delete` (CT.4; `contacts.write`, CEILING_TRUSTED;
//!     confirm-first, irreversible)
//!
//! Multi-tool harness consumed from `aivyx_tool::multi_harness`.
//!
//! ## Auth model
//!
//! Operator-provided OAuth app — same posture as Gmail /
//! Calendar / Drive. Most operators reuse their existing Aivyx
//! GCP project + client_id + client_secret; they enable the
//! People API alongside the others, register
//! `https://www.googleapis.com/auth/contacts` on the OAuth
//! consent screen, and run `aivyx-contacts auth init` to grant
//! the contacts scope independently.

pub mod auth_cli;
pub mod contacts_client;
pub mod tools;

pub use contacts_client::{ContactsClient, ContactsClientError};

// Re-export the shared OAuth substrate so consumers (main.rs +
// downstream) use a single import path, identical to aivyx-drive.
pub use aivyx_google_oauth::{
    exchange_code, load_tokens, refresh_access_token, save_tokens, ExchangeError,
    OAuthConfig, OAuthError, StorageError, TokenSet, GOOGLE_AUTH_ENDPOINT,
    GOOGLE_TOKEN_ENDPOINT,
};

// Re-export the lifted multi-tool harness so consumers use the
// same import surface as aivyx-gmail / aivyx-calendar / aivyx-drive.
pub use aivyx_tool::multi_harness::{run_multi_tool_subprocess, HarnessError};

/// Default Contacts OAuth scope covering the CT.3 + CT.4 tool
/// surface (six tools: 3 read / 3 write).
///
/// The chapter ships the write surface (create / update /
/// delete), so the default is the **read-write** People API
/// scope `https://www.googleapis.com/auth/contacts`. Operators
/// wanting read-only access can narrow to
/// `https://www.googleapis.com/auth/contacts.readonly` via the
/// `scopes` key in their `config.toml`; the write tools then
/// fail with a clear "scope not granted" message.
pub const DEFAULT_CONTACTS_SCOPES: &[&str] =
    &["https://www.googleapis.com/auth/contacts"];

/// Service-specific token storage path
/// (`$HOME/.aivyx/tool-processes/contacts/tokens.json`).
/// Returns `None` when `$HOME` is unset.
pub fn default_token_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        std::path::PathBuf::from(home)
            .join(".aivyx")
            .join("tool-processes")
            .join("contacts")
            .join("tokens.json"),
    )
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    #[test]
    fn default_scopes_use_read_write_contacts() {
        assert_eq!(
            DEFAULT_CONTACTS_SCOPES,
            &["https://www.googleapis.com/auth/contacts"]
        );
    }

    #[test]
    fn default_scope_is_not_the_readonly_variant() {
        // Regression pin: the chapter ships create/update/delete,
        // so the default must be the read-write scope. A future
        // narrowing to `.readonly` would silently break the write
        // tools — this test surfaces that.
        assert!(!DEFAULT_CONTACTS_SCOPES
            .contains(&"https://www.googleapis.com/auth/contacts.readonly"));
    }

    #[test]
    fn token_path_lands_under_contacts_tool_process_dir() {
        // Read whatever HOME the test environment provides (do not
        // mutate process env — unsafe under edition 2024). If HOME
        // is unset the path is None and there is nothing to assert.
        if let Some(p) = default_token_path() {
            assert!(p.ends_with("tool-processes/contacts/tokens.json"));
        }
    }
}

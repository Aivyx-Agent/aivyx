//! Operator-facing CLI for the Contacts OAuth flow.
//!
//! Phase 123 Task 3. Three subcommands:
//!
//! - `aivyx-contacts auth init` — runs the auth-code flow. Loads
//!   the operator-supplied OAuth client config; spins up a
//!   local-loopback HTTP listener on the redirect_uri's port;
//!   prints the Google consent URL; waits for the browser
//!   redirect; exchanges the captured code for tokens; saves
//!   them via [`crate::storage::save_tokens`].
//!
//! - `aivyx-contacts auth status` — loads tokens (if any) and
//!   prints a human-readable health report: granted scopes,
//!   expiry, refresh-available?, time-since-init.
//!
//! - `aivyx-contacts auth revoke` — calls Google's revoke
//!   endpoint (best-effort; logs but doesn't fail if the
//!   network call doesn't reach Google) and deletes the local
//!   token file. After revoke, `auth init` must be re-run
//!   before any Contacts tool will work.
//!
//! ## Module layout
//!
//! - [`cli`] — argument parsing (subcommand dispatch).
//! - [`config_file`] — load the operator-supplied OAuth client
//!   config from `~/.aivyx/tool-processes/contacts/config.toml`.
//! - [`init`] — `auth init` implementation (loopback +
//!   consent URL + token exchange).
//! - [`status`] — `auth status` implementation.
//! - [`revoke`] — `auth revoke` implementation.

pub mod cli;
pub mod config_file;
pub mod init;
pub mod revoke;
pub mod status;

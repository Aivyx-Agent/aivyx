//! # aivyx-gmail
//!
//! Gmail third-party tool process for Aivyx. Chapter F #1 — Phase
//! 123. Ships as a separate binary the operator installs and wires
//! into `aivyx.toml` via `[[tool_process]]`. Per PRODUCT.md P10
//! (substrate is closed at fifteen tools forever; email is third-
//! party territory).
//!
//! ## Layout (Phase 123 Task 2 — this commit)
//!
//! - [`oauth`] — OAuth 2.0 substrate. Operator-provided Google
//!   Cloud OAuth app; auth-code exchange; refresh-token flow;
//!   per-tool-process token file storage at
//!   `~/.aivyx/tool-processes/gmail/tokens.json` (0600 perms).
//!
//! ## Layout (Phase 123 Tasks 3-7 — future commits)
//!
//! - `auth_cli` — Task 3: `aivyx-gmail auth init / status / revoke`
//!   operator-facing CLI subcommand surface.
//! - `tools` — Tasks 4-7: `gmail.search`, `gmail.read`,
//!   `gmail.draft`, `gmail.send` implementations of the
//!   `aivyx_core::Tool` trait.
//! - `harness` — Task 4: multi-tool harness (inline; documented
//!   as Phase 123 SDK-validation finding — single-tool
//!   `aivyx_tool::run_tool_as_subprocess` doesn't fit a four-tool
//!   process; substrate generalization is follow-on substrate work
//!   for a future phase).
//!
//! ## Auth model
//!
//! Operator-provided OAuth app (Q1a Recommended at Phase 123 sign-
//! off). The operator creates an OAuth client in their own Google
//! Cloud project, configures the Gmail API + consent screen, and
//! supplies `client_id` + `client_secret` via the tool-process
//! config file at `~/.aivyx/tool-processes/gmail/config.toml`.
//! Aivyx never sees an Aivyx-published OAuth app. Aligns with
//! G6 (Local execution, privacy non-negotiable) + Phase 99
//! local-builds posture.

pub mod auth_cli;
pub mod gmail_client;
pub mod harness;
pub mod mime;
pub mod oauth;
pub mod tools;

pub use gmail_client::{GmailClient, GmailClientError};
pub use harness::{run_multi_tool_subprocess, HarnessError};
pub use oauth::{
    OAuthConfig, OAuthError, TokenSet, DEFAULT_GMAIL_SCOPES,
    GOOGLE_AUTH_ENDPOINT, GOOGLE_TOKEN_ENDPOINT,
};

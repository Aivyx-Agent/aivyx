//! # aivyx-n8n
//!
//! n8n workflow-automation third-party tool process for
//! Aivyx. Chapter F #7 — Phase 131.
//!
//! ## Substrate shape: bearer-token with operator-base
//!
//! Unlike the Notion / Gmail / Calendar / Drive
//! integrations (which all hard-code a fixed service base
//! URL), n8n is **self-hosted**. Operators point at their
//! own instance via `n8n_base_url` in config.toml. The
//! crate constructs every request as
//! `{n8n_base_url}/api/v1/{path}` with the n8n-specific
//! `X-N8N-API-KEY` header.
//!
//! ## Why no OAuth substrate
//!
//! n8n uses **API key** auth — same shape as Notion's
//! Integration Token but with a different header name.
//! Operators create an API key in n8n's Settings → API
//! → Create API key, copy it, paste it into config.toml.
//! No callback flow, no token refresh.
//!
//! ## Layout (Phase 131)
//!
//! - [`n8n_client`] — token-authenticated REST client.
//! - [`auth_cli`] — minimal CLI: `auth status` (offline)
//!   plus `auth check` (online ping against
//!   `/api/v1/workflows?limit=1`).
//! - [`tools`] — ten `aivyx_core::Tool` impls per Phase
//!   131 Q1c (Tasks 3-12).

pub mod auth_cli;
pub mod n8n_client;
pub mod tools;

pub use n8n_client::{N8nClient, N8nClientError, N8nConfig};

pub use aivyx_tool::multi_harness::{run_multi_tool_subprocess, HarnessError};

/// Default config path
/// `$HOME/.aivyx/tool-processes/n8n/config.toml`.
pub fn default_config_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        std::path::PathBuf::from(home)
            .join(".aivyx")
            .join("tool-processes")
            .join("n8n")
            .join("config.toml"),
    )
}

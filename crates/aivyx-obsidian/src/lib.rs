//! # aivyx-obsidian
//!
//! Obsidian vault third-party tool process for Aivyx.
//! Chapter F #6 — Phase 130. Ships as a separate binary
//! the operator installs and wires into `aivyx-pa.toml` via
//! `[[tool_process]]`.
//!
//! ## Why no API client
//!
//! Obsidian is a **local Markdown vault** — there's no
//! REST API. The integration is filesystem operations
//! against the operator's vault directory (set in
//! config.toml). This is fundamentally different from
//! the other Chapter F integrations (Gmail / Calendar /
//! Drive / Notion all have external APIs).
//!
//! ## The path-traversal guard is load-bearing
//!
//! Every tool operation MUST resolve operator-supplied
//! paths via [`VaultClient::resolve_under_vault`] before
//! any I/O. See [`vault_client`] for the full safety
//! model. Without the guard a `..` segment in operator
//! input would let the agent escape the vault root and
//! read/write arbitrary filesystem locations.
//!
//! ## Layout (Phase 130)
//!
//! - [`vault_client`] — vault root resolution + path-
//!   traversal guard.
//! - [`markdown`] — frontmatter splitter + wikilink and
//!   tag extractors.
//! - [`auth_cli`] — minimal CLI (just `auth check` to
//!   verify the vault path is configured and readable).
//! - [`tools`] — six `aivyx_core::Tool` impls per Phase
//!   130 Q2a (Tasks 11-16).

pub mod auth_cli;
pub mod markdown;
pub mod tools;
pub mod vault_client;

pub use vault_client::{VaultClient, VaultConfig, VaultError, MARKDOWN_EXT};

pub use aivyx_tool::multi_harness::{run_multi_tool_subprocess, HarnessError};

/// Default config file path:
/// `$HOME/.aivyx-pa/tool-processes/obsidian/config.toml`.
pub fn default_config_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        std::path::PathBuf::from(home)
            .join(".aivyx-pa")
            .join("tool-processes")
            .join("obsidian")
            .join("config.toml"),
    )
}

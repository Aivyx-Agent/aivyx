//! Minimal CLI for `aivyx-obsidian` — even slimmer than
//! `aivyx-notion`'s. Obsidian has no auth at all (vault
//! is just a directory), so the only "auth" subcommand is
//! `auth check` — verify the configured vault path
//! exists and is readable.

pub mod cli;
pub mod config_file;
pub mod check;

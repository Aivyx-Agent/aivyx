//! `aivyx-kitchen-toolkit` — the kitchen vertical's tool process (Chapter
//! Brigade).
//!
//! A [Chapter F/G](../../../docs/VERTICAL_PACKS.md) substrate-pattern tool
//! process: the daemon spawns it via `[[tool_process]]` in `aivyx.toml`, and it
//! registers the `kitchen.*` tools through the multi-tool harness
//! ([`run_multi_tool_subprocess`]). The tools call the operator's **KitchenDB**
//! (Postgres + PostgREST) — the system of record — via [`KitchenClient`]; the
//! agent never reimplements domain logic.
//!
//! The `kitchen.*` capability bases already live in `aivyx-capability`'s
//! `KNOWN_BASES`, so this crate adds none and is not a P10 amendment — it is
//! the third-party tool-process tier.
//!
//! BG.1 ships the read surface (`kitchen.read`): inventory list / low-stock /
//! value, recipe search, supplier list. Gated writes, `kitchen.order.send`
//! (confirm-first), and `kitchen.haccp.log` land in later phases.

pub mod client;
pub mod config;
pub mod tools;

pub use aivyx_tool::run_multi_tool_subprocess;
pub use client::{KitchenClient, KitchenError};
pub use config::{default_config_path, load_config, KitchenConfig, KitchenDbConfig};

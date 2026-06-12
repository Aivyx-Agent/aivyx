//! The daemon ↔ client IPC protocol — the wire envelope + frame codec.
//!
//! Moved wholesale to the wasm-clean [`aivyx_ipc`] crate (Chapter M.2f) so the
//! browser Mission-Control app (a `wasm32` Dioxus client) and the daemon
//! serialize from one source of wire truth. Re-exported here so every
//! `crate::daemon_ipc::…` reference across the daemon — and external
//! `aivyx_channel::daemon_ipc::…` users (the CLI/TUI clients) — are unchanged.
//!
//! `FrontendMessage` / `DaemonMessage` / `QueryPayload` /
//! `QueryResponsePayload`, the `*Summary` wire structs, the lifecycle + stream
//! events, and `encode_frame` / `decode_frame` all live in
//! [`aivyx_ipc::protocol`]; the daemon-side handlers that *produce* them stay
//! in `daemon_server` / `daemon_client`.
pub use aivyx_ipc::protocol::*;

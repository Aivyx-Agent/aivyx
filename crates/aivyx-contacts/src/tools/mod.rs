//! Tool-trait implementations registered by the IPC harness.
//!
//! Chapter Contacts. Each tool implements [`aivyx_core::Tool`]
//! so it can be served by the multi-tool IPC harness (see
//! [`aivyx_tool::multi_harness`]). All six tools share the
//! [`crate::ContactsClient`].
//!
//! ## Module layout
//!
//! - `search` / `list` / `get` — CT.3: `contacts.search`,
//!   `contacts.list`, `contacts.get` (`contacts.read`).
//! - `create` / `update` / `delete` — CT.4: `contacts.create`,
//!   `contacts.update`, `contacts.delete` (`contacts.write`,
//!   Trusted-tier only; `delete` is confirm-first).
//!
//! ## Output shaping (`person`)
//!
//! The People API returns deeply-nested `Person` objects. The
//! shared [`person::trim_person`] helper flattens one to the
//! snake-cased shape every read + write tool returns, so the
//! cross-tool response shape stays operator-predictable (the
//! `web.search` / Gmail precedent).

pub mod person;

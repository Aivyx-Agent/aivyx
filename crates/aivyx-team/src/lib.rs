//! `aivyx-team` — Nonagon multi-agent teams (Chapter J).
//!
//! A **lead** agent convenes up to **9 attenuated specialists** inside the
//! one daemon, on the one HMAC audit chain — preserving the single-agent
//! ethos (one lead; ephemeral, subordinate, least-privileged specialists).
//! The engine is **free core**; **verticals customise the team** (the
//! kitchen pack ships a Back-of-House Nonagon). See `docs/NONAGON.md`.
//!
//! ## Phase J.1 (this module set)
//!
//! - [`config`] — the [`TeamConfig`] schema verticals supply (a `lead`
//!   plus members, each a persona + scoped role) + validation.
//! - [`roster`] — the default general-purpose 9-role Nonagon
//!   ([`default_nonagon`]) that ships in the free core.
//! - [`attenuation`] — [`attenuate_for_member`], the **NT-02** safety
//!   primitive: a specialist's capabilities are the declared scopes the
//!   lead actually grants, so a specialist can never exceed its lead.
//!
//! The `SpecialistPool`, delegation/message tools, `MissionPlan` DAG, and
//! `TeamRuntime` arrive in J.2–J.5.

pub mod attenuation;
pub mod config;
pub mod factory;
pub mod message_bus;
pub mod message_tools;
pub mod pool;
pub mod roster;
pub mod tools;

#[cfg(test)]
mod testutil;

pub use attenuation::{attenuate_for_member, effective_trust};
pub use config::{DialogueConfig, TeamConfig, TeamError, TeamMember, MAX_SPECIALISTS};
pub use factory::{filter_tools, SpecialistFactory};
pub use message_bus::{Drained, MessageBus, Recipient, Subscription, TeamMessage};
pub use message_tools::{ReadMessagesTool, SendMessageTool};
pub use pool::{SpecialistChannel, SpecialistPool};
pub use roster::default_nonagon;
pub use tools::{DelegateTaskTool, QueryAgentTool};

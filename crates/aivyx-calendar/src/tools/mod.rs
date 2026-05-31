//! `aivyx_core::Tool` implementations for the Calendar
//! tool process.
//!
//! Phase 128 Q3b operator-picked surface (5 tools):
//!
//! - [`list_events`] — `calendar.list_events` (Task 4;
//!   capability `calendar.read`)
//! - [`get_event`] — `calendar.get_event` (Task 5;
//!   capability `calendar.read`)
//! - [`create_event`] — `calendar.create_event` (Task 6;
//!   capability `calendar.write`, CEILING_TRUSTED)
//! - [`update_event`] — `calendar.update_event` (Task 7;
//!   capability `calendar.write`, CEILING_TRUSTED)
//! - [`delete_event`] — `calendar.delete_event` (Task 8;
//!   capability `calendar.write`, CEILING_TRUSTED)
//!
//! Per-tool modules land in tasks 4-8; this `mod.rs` is
//! the entry-point that `main.rs` reaches into to build
//! the harness's `Vec<Arc<dyn Tool>>`.
//!
//! ## Capability bases
//!
//! Phase 128 Task 3 registers `calendar.read` and
//! `calendar.write` in `aivyx-capability`. The read base
//! defaults to OPERATOR (visible to any role with
//! Operator-or-higher tier); the write base defaults to
//! CEILING_TRUSTED (write tools require an explicit grant
//! per role).

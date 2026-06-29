//! Chapter Deckhand — the `aivyx-apps` tool process.
//!
//! An opt-in (`[applications]`, default off) third-party tool process that lets
//! the agent **use the GUI applications already open on the operator's own
//! machine** — list windows, focus one, send keystrokes / clicks, and capture
//! a screenshot. It runs as a separate process behind the multi-tool harness
//! (like `aivyx-toolkit`), isolating the desktop integration from the core.
//!
//! Trust model: all `app.*` bases are Trusted-tier only and `app.input` is
//! confirm-first (see `docs/APPLICATIONS.md` + `aivyx_capability`). Linux/X11 +
//! Xwayland in v1; native Wayland windows are a documented limitation.

pub mod backend;
pub mod tools;

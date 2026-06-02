//! `whisper-cpp-plus` ASR backend — **deferred to Phase 136+**.
//!
//! ## Phase 135 scope adjustment from Q2c
//!
//! Phase 135's Q-block picked Q2c (both whisper-rs +
//! whisper-cpp-plus shipping as alternatives).
//! Implementation pass for Task 3 surfaced that
//! `whisper-cpp-plus = "0.1.4"` (the only published
//! version on crates.io at Phase 135 sign-off) does
//! **not build** against the current whisper.cpp it
//! tries to link — 40 compile errors against the
//! current `whisper_full_params` struct shape. The
//! crate appears to be upstream-stale.
//!
//! Rather than fork the crate or ship a non-building
//! feature flag, Phase 135 keeps the
//! `asr-whisper-cpp-plus` feature flag wired for
//! future re-enablement and ships this module as a
//! stub. Operators who want the alternative engine in
//! the meantime use `whisper-rs` (default, working).
//!
//! Phase 136+ candidate revisits this: either
//! upstream fixes land + we promote to a real impl,
//! or we swap the alternative engine for a different
//! crate (e.g. `rwhisper` / kalosm, `whisper-stream-rs`).
//!
//! See the Phase 135 exit doc's "What landed cleanly
//! + what bent" section for the honest framing.

use crate::asr::AsrConfig;

/// Placeholder type — see module docs. Constructable
/// so the feature gate compiles, but does not load a
/// model or implement `AsrEngine`. Phase 136+ promotes
/// this to a real impl once a working binding exists.
pub struct WhisperCppPlusEngine {
    #[allow(dead_code)]
    config: AsrConfig,
}

impl WhisperCppPlusEngine {
    pub fn new(config: AsrConfig) -> Self {
        WhisperCppPlusEngine { config }
    }
}

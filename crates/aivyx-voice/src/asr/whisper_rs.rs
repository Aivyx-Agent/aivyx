//! `whisper-rs` ASR backend (default).
//!
//! Phase 135 Task 2 — placeholder stub so the feature
//! gate plumbing compiles. The real `AsrEngine` impl
//! lands in Task 3.

use crate::asr::AsrConfig;

/// Placeholder type for Task 3's `AsrEngine` impl.
/// Task 3 replaces this with the real model handle +
/// trait impl.
pub struct WhisperRsEngine {
    #[allow(dead_code)]
    config: AsrConfig,
}

impl WhisperRsEngine {
    /// Construct from operator config. Task 3 wires the
    /// actual `whisper_rs::WhisperContext::new_with_params`
    /// call.
    pub fn new(config: AsrConfig) -> Self {
        WhisperRsEngine { config }
    }
}

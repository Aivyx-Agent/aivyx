//! `whisper-cpp-plus` ASR backend (alternative).
//!
//! Phase 135 Task 2 — placeholder stub. Task 3 wires
//! the real `AsrEngine` impl, taking advantage of
//! whisper-cpp-plus's built-in Silero VAD + real-time
//! PCM streaming surface.

use crate::asr::AsrConfig;

/// Placeholder type for Task 3's `AsrEngine` impl.
pub struct WhisperCppPlusEngine {
    #[allow(dead_code)]
    config: AsrConfig,
}

impl WhisperCppPlusEngine {
    pub fn new(config: AsrConfig) -> Self {
        WhisperCppPlusEngine { config }
    }
}

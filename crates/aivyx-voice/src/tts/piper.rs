//! Piper TTS backend (via `piper1-rs`).
//!
//! Phase 135 Task 2 — placeholder stub. Task 4 wires
//! the real `TtsEngine` impl. Build prereq: ONNX
//! runtime headers must be installed
//! (`ONNX_RUNTIME_DIR`, `ONNX_INCLUDE_PATH` env vars
//! point at them). See INSTALL.md.

use crate::tts::TtsConfig;

/// Placeholder type for Task 4's `TtsEngine` impl.
pub struct PiperEngine {
    #[allow(dead_code)]
    config: TtsConfig,
}

impl PiperEngine {
    pub fn new(config: TtsConfig) -> Self {
        PiperEngine { config }
    }
}

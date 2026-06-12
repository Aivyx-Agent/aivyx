//! Longitudinal learning-ledger views (Phase 78) — moved to `aivyx-ipc` in
//! M.2d.
//!
//! Decayed, cross-session helpfulness / correction / co-occurrence summaries
//! shipped over IPC (`GetLearningInsights`). The persistent ledgers that
//! compute them (encrypted storage + EWMA decay) stay in `aivyx-channel`.

use serde::{Deserialize, Serialize};

/// One topic's decayed accumulated helpfulness, for the Phase
/// 78 longitudinal surface. `samples` is the confidence proxy
/// (one EWMA point is not a trend).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicScore {
    pub topic: String,
    pub score: f32,
    pub samples: u32,
}

/// The durable, decayed top-helpful / top-unhelpful per-topic
/// view (the longitudinal picture Phase 78 deferred — distinct
/// from the windowed `LearningDigest.top_helpful`).
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize,
)]
pub struct AccumulatedHelpfulness {
    pub top_helpful: Vec<TopicScore>,
    pub top_unhelpful: Vec<TopicScore>,
}

/// One affined topic pair, for the Phase 78 surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairScore {
    pub a: String,
    pub b: String,
    pub score: f32,
    pub samples: u32,
}

/// The durable, decayed top co-occurring topic pairs (the
/// cross-session pattern view — "topics that consistently help
/// together").
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize,
)]
pub struct CooccurrencePatterns {
    pub top_pairs: Vec<PairScore>,
}

/// One topic's decayed accumulated correction pressure, for the
/// Phase 78 longitudinal surface. `samples` is the confidence
/// proxy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicCorrections {
    pub topic: String,
    pub count: f32,
    pub samples: u32,
}

/// The durable, decayed most-corrected-per-topic view for the
/// Phase 78 learning surface.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize,
)]
pub struct AccumulatedCorrections {
    pub top_corrected: Vec<TopicCorrections>,
}

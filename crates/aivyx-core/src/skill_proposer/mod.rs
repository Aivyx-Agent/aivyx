//! Phase 112 — Skill Auto-Proposer substrate.
//!
//! Closes the Phase 110 named follow-on: agent-side auto-
//! proposer heuristic that fires after complex turns to draft
//! `LearnedSkill` proposals. Turns Phase 110's operator-driven
//! propose-approve-render-invoke loop into the genuinely
//! self-learning propose-judge-accept-render-invoke loop the
//! project vision requires.
//!
//! ## Module layout
//!
//! - `heuristic` (Task 2, this commit) — cheap deterministic
//!   gate. `TurnSignals` + `HeuristicConfig` + `is_candidate`.
//!   Zero LLM cost; reads only the post-turn signals the
//!   caller assembles from `TurnOutcome` + audit log.
//! - `judge` (Task 3, follow-on commit) — LLM-judge prompt and
//!   invocation surface. Takes candidate turn summary + the
//!   existing skill list and returns
//!   `{is_worth_proposing, confidence, proposed_skill,
//!   is_duplicate_of}`.
//!
//! The Q1b two-stage shape (heuristic → judge) is the Phase
//! 91/95 precedent — cheap deterministic signal filters to a
//! small candidate set; LLM-judge confirms each candidate.
//! Cost-efficient because the heuristic rejects the bulk of
//! simple turns (chats, single-tool turns) before any LLM
//! call fires.

pub mod heuristic;

pub use heuristic::{HeuristicConfig, MatchMode, TurnSignals, is_candidate};

# Phase 0 — Design (FROZEN)

**Status:** Closed 2026-04-13
**Exit commit:** `1b4f271` — *"Phase 0: agent-first rebuild — design locked, skeleton green"*
**Successor:** [PHASE_1.md](PHASE_1.md)

This document is a historical record. The contract it produced lives
in [`../DESIGN.md`](../../../DESIGN.md); this file explains *how* that
contract came to be and what was deliberately left out.

## Goal

Rebuild Aivyx from scratch with an **agent-first, ecosystem-second**
architecture. Produce a single coherent design document and a
compiling (but empty) workspace skeleton. **No runtime code.**

The driving motivation was that the archived pre-rebuild codebase had
Alfred (the PA) bolted onto a drifting core — channel adapters
bypassed on the chat path, tools lived in a second crate and had to be
bridged via a wrapper, and capability checks had gaps. Phase 0's job
was to make those classes of bug *impossible to re-introduce by
construction*, not just fixable.

## Deliverables (all LOCKED)

| #  | Deliverable                              | Status |
|----|------------------------------------------|--------|
| D1 | Turn loop contract (north star)          | LOCKED |
| D2 | Open-core line (MIT surface + 11 crates) | LOCKED |
| D3 | Core traits (Agent, Tool, ChannelContext, StreamEvent, TurnOutcome, ToolOutcome, Verification) | LOCKED |
| D4 | Capability system (21 active scopes, prefix attenuation, audit events) | LOCKED |
| D5 | Trust tiers (Untrusted < SemiTrusted < Trusted < Kernel, per-tier ceilings) | LOCKED |
| D6 | `AivyxError` (14 variants, 15-variant cap) | LOCKED |
| D7 | Storage stack (redb + HKDF per `KeyDomain`, 5 domains) | LOCKED |
| D8 | Repo skeleton (9-crate workspace, `cargo check` green in 0.03s) | LOCKED |

See [`../DESIGN.md`](../../../DESIGN.md) for the full contract text.

## Decisions made during Phase 0 that aren't in DESIGN.md

These are the judgment calls and close-run choices that shaped the
contract but don't appear as contract text. Recording them here so
Phase 1 has the reasoning, not just the result.

### 9 crates over 11

Earlier drafts had `aivyx-tool` (tool registry) and `aivyx-turn` (turn
loop) as separate crates. Collapsed both into `aivyx-core` once it was
clear the registry has no state beyond what the turn loop already
holds, and a separate turn-loop crate would need to re-export almost
everything from core anyway. Fewer crates = fewer `pub use` boundaries
= less drift.

### "No bypass path" as deliverable 1, paragraph 1

The north-star paragraph of D1 was originally drafted as *"agents run
in a turn loop."* Rewrote it to state explicitly: **every agent turn
is delivered through a `ChannelContext` — there is no direct
`agent.turn()` on the public API.** This is the single most important
sentence in the document: it's the structural fix for the chat-path
bypass bug that Alfred shipped with.

### `Verification` enum on `ToolOutcome`

Added after the `feedback_tool_success_vs_intent` memory record was
surfaced. A tool returning `Ok` does not mean the intent succeeded —
a `key_combo` tool call can return `Ok` while the keystroke landed in
the wrong window. Every tool now declares
`Verified | Unverified | NotApplicable` on its outcome, and the turn
loop surfaces that to the agent so it can't silently narrate success.

### `TrustTier` variant ordering (footgun caught mid-draft)

First draft declared `Kernel, Untrusted, SemiTrusted, Trusted` — which
under derived `Ord` makes `Kernel < Untrusted`, the wrong direction.
Reordered to `Untrusted, SemiTrusted, Trusted, Kernel` so that
`tier_a < tier_b` correctly means "a is less trusted than b." Exactly
the class of thing Phase 0 is supposed to catch before any code relies
on it.

### Audit as inline-synchronous, not async-batched

Considered batching audit events on a background writer for
throughput. Rejected: if the process crashes between a
scope-denied event and its flush, the forensic record is gone. Audit
must complete before the tool call returns, even at a latency cost.
Recorded under D4 as a hard commitment.

### Passphrase lives in the channel adapter, not storage

`aivyx-storage` never prompts for a passphrase. The channel adapter
collects it (CLI asks on stdin, GUI pops a dialog, remote channels use
a config file with fs perms) and hands it to storage as bytes. Keeps
storage deterministic and testable; keeps channels free to do their
own UX. See D7.

### Versioned HKDF salt

The salt string `"aivyx-v1-storage"` has a version prefix from day
one. When key rotation ever becomes necessary, bumping to
`"aivyx-v2-storage"` gives a clean break without touching the master
key. Cheap to add now, painful to retrofit later.

## Decisions explicitly deferred to Phase 1

These came up during Phase 0 sanity checks but are not contract
material — they belong to Phase 1's implementation phase.

- **`Tool::required_scope` signature.** The current D3 shape
  `required_scope() -> Scope` can't express input-dependent scope
  requirements (e.g., `memory.read` needing different scopes based on
  session filter). Phase 1 will change it to
  `required_scope(&self, input: &Value) -> Scope`. This is an
  extension, not a contract break.
- **Window-control scopes.** D1's Scenario 2 (close terminal window)
  requires `display.window_close` and related scopes. Left under the
  Reserved section of D4 rather than promoted to active — the concrete
  scope taxonomy for display control belongs to the Phase 1 tool crate
  that implements it.
- **`aivyx-tool` tool crate split.** D8 collapsed tools into core, but
  once there are more than ~5 built-in tools, a separate
  `aivyx-tool-fs`, `aivyx-tool-net` etc. layout may be reasonable.
  Phase 1 decision, driven by actual tool count.

## Lessons carried forward

- **Design documents must be readable end-to-end by a stranger.** A
  working developer on a fresh session needs to understand the whole
  contract without access to conversation history. The D1 paragraph is
  the test: if it doesn't land on first read, nothing downstream does.
- **`git init` *after* the skeleton compiles, not before.** Every file
  in the root commit was intentional Phase 0 output. No migration
  history, no experimental branches, no fix-up commits. Clean slate.
- **Flag every known refinement at the moment it's spotted.** D3's
  `required_scope` gap and D4's window-control gap were both caught by
  walking D1's scenarios against D4's active scope list. Neither
  blocked Phase 0, but both are now queued against Phase 1 in this
  document — they won't be forgotten.

## Exit criteria (all met)

- [x] All 8 deliverables LOCKED in `DESIGN.md`
- [x] Document reads end-to-end as a coherent design
- [x] 9-crate workspace scaffolded
- [x] `cargo check --workspace` green
- [x] No runtime code written
- [x] Root commit created (`1b4f271`) with no remote
- [x] Archive of pre-rebuild codebase complete (see user memory
  `project_aivyx_rebuild.md`)

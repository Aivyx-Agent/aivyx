# Following the Thread — conversation-history replay (Chapter Thread)

> **Status: CORE COMPLETE (TR.1–TR.3, 2026-07-05); TR.4 rig verification
> pending.** Interactive-session turns now replay the session's recent
> user/assistant messages into the model's context as real conversation
> history, so follow-ups like *"did you find the correct code?"* resolve
> against what was actually said. Fresh-context turns remain the
> substrate — replay is a bounded, ephemeral prefix, not a persistent
> transcript.

## 1. Why — the Vitrine P1 that named the tension

The operator's first Studio chat session (2026-07-05) hit the wall in
five minutes: *"Whats the ICAO for Jandakot?"* → web_search → *(empty
completion)* → *"Did you find the correct code?"* → the agent guessed
"code" meant a source-code snippet. The audit chain confirmed both
turns shared one session; the cause was the documented WI.2 design —
turns are fresh-context, and the Phase 86 conversation window feeds
only recall *relevance*, never the model prompt.

That design is right for automation (cron, loop, reflection — synthetic
turns with no conversation to remember) and wrong for a chat surface:
pronouns, ellipsis, and answer-to-my-question turns are the *normal
texture* of conversation, and every one of them fails without the prior
turn in context. The operator decided (2026-07-05, explicitly weighing
the block-injection alternative): **full history replay, on by
default** — the model should see the conversation the operator sees.

## 2. Architecture & decisions (locked)

- **Source = the Phase 86 window, unchanged.** The per-session
  `ConversationWindow` (16 messages / 4,000-char cap, ephemeral, daemon
  lifetime) already records exactly what replay needs: the operator's
  text and the assistant's final text, for interactive-session turns
  only. Thread adds readers, not state. Tool calls and tool results
  are **not** replayed — what was *said*, not what was *done*.
- **Core hook mirrors `ContextProvider`.** A `ConversationSeeder`
  trait in `llm_planner` (implemented in the channel layer, wired by
  the binary); `begin_turn` seeds normalized prior messages into the
  turn history before the current user message. No seeder → byte-
  identical fresh-context turns.
- **Provider-safe normalization** (`seeded_history_messages`): blanks
  dropped; leading assistant entries dropped (first message must be
  user); consecutive same-role entries coalesced (strict-alternation
  providers); a trailing user entry — the fingerprint of a prior
  empty completion — gets an honest `(no response was produced that
  turn)` assistant filler, which both preserves alternation and lets
  the model SEE that it never answered.
- **Knob: `[agent] conversation_history_turns`**, default **8**
  messages (four exchanges), `0` disables. Default-ON is a deliberate,
  operator-decided exception to the behaviour-change-is-opt-in
  discipline — recorded here and at the constant.
- **Trigger turns are structurally excluded.** Cron / loop /
  reflection fires run under fresh per-fire sessions that never get a
  window entry; the seeder returns empty and the planner no-ops. No
  flag needed.
- **The windows are now built unconditionally** (previously iff
  `[embedding]` was configured) — replay must work on embedding-free
  (Ember/lite) installs.
- **WI.2 amendment:** Chapter Wire's "sessions do not add transcript
  injection" statement is revised by this chapter. Piped
  `aivyx --headless` multi-turn sessions get replay too — a batch of
  related steps now reads as one conversation, which is what Wire's
  operators wanted anyway. One-shot `--headless "task"` turns have no
  prior window and are unchanged.
- **Out of scope:** the in-process (`--no-daemon`) REPL — its turns
  don't flow through the daemon's window write site, so it keeps
  fresh-context turns for now; role-switch child planners (specialist
  sub-turns keep fresh context deliberately); durable cross-restart
  transcripts (the window stays ephemeral by design — memory remains
  the durable layer, the Etch charter line stays true).

## 3. Phase plan

| Phase | What | Proof |
|---|---|---|
| **TR.0** | This doc. | Reviewed. |
| **TR.1** ✅ | Core: `PriorTurn` + `ConversationSeeder` trait + `seeded_history_messages` normalization + `begin_turn` seeding + config field/builder. | 8 new unit tests; planner suite green. |
| **TR.2** ✅ | Channel: `WindowConversationSeeder` over the shared windows (newest-first budget trim, oldest-first replay) + `[agent] conversation_history_turns` (default 8, 0 disables). | 4 new window tests; config check green. |
| **TR.3** ✅ | Binary wiring: windows built unconditionally; seeder attached to the daemon planner when the knob > 0. | Full workspace clippy + tests green. |
| **TR.4** | Rig live-verify: piped two-turn session re-running the exact ICAO scenario — the follow-up must resolve "the correct code" against the prior turn. | Pending. |

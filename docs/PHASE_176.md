# Phase 176 — Loop Token-Budget Cap (the last cap in the trio)

**Closes the loop's cap trio.** Phase 173 gave the autonomous
loop a `max_iterations` cap; Phase 174 added a wall-clock cap +
gate verification; Phase 175 made it accumulate knowledge. The
one operational risk still uncapped is **spend**: an autonomous
LLM loop that runs for many iterations can quietly burn a lot of
tokens. Phase 176 adds a **per-run token budget** — the run
stops once total turn token-usage during the run exceeds a
configured cap.

## The semantic (deliberately simple + honest)

The audit chain already records `TokenUsage` (input + output)
on every `TurnEnded`. There is **no** trigger-kind on a turn and
**no** trigger-fired audit event, so the chain can't cheaply
say "this turn was a loop iteration." Rather than change
`TriggerDispatch::fire`'s signature (5 callers) or touch the
core `TurnOutcome` (which would break the `lib.rs` streak),
Phase 176 takes the lean, honest framing:

> **`max_run_tokens` caps the total token usage of every turn
> that completes during the run window** — snapshot the audit
> chain length at run start, then sum `TurnEnded` input+output
> tokens after it.

In the overwhelmingly common case (an autonomous run is the
only thing firing turns) this *is* the loop's spend. If the
operator also chats, or a reflection cron fires mid-run, those
turns count too — which is the **safe** direction for a cap (it
stops sooner) and is arguably the more useful guard anyway
("don't let the daemon spend more than X tokens while this
autonomous run is going"). Precise loop-only attribution
(via a `fire()` that returns the iteration's session id) is a
documented Phase 177 refinement.

It's a **token** cap, not a dollar cap — no per-model pricing
table is introduced. Input + output tokens are summed; cache
tokens are not separately weighted.

## How it fits the existing driver

The driver already checks `decide()` at the top of every
iteration (operator-stop → wall-clock → iteration-cap →
backlog-drain). Phase 176 slots a **`StopBudget`** decision in
after the wall-clock check: before each iteration the driver
sums turn usage since the run's start-seq (best-effort over the
audit chain) and passes the total to `decide()`. No audit log
configured → no budget enforcement (graceful degrade, like the
gate when `gate_command` is unset).

## Tasks

1. **Open doc + README.** This doc, README Active row, backfill
   Phase 175's frozen hash (`5cc78eb`).

2. **Config + usage-sum helper.** `[loop].max_run_tokens:
   Option<u64>` (validated `>= 1` when set on an armed section).
   A pure `sum_turn_usage(entries) -> u64` that sums
   `input_tokens + output_tokens` over `AuditEvent::TurnEnded`
   in a slice of audit entries — unit-testable without a daemon.

3. **`decide()` budget cap.** New
   `LoopDecision::StopBudget { tokens }`. `decide()` gains
   `tokens_used: u64` + `max_run_tokens: Option<u64>`, checked
   after the wall-clock cap (`None` disables). Update the
   existing decision tests; add budget-precedence tests.

4. **Driver budget wiring.** Thread the audit-log `Arc` +
   `max_run_tokens` into `run_loop_driver`. At run start,
   snapshot `start_seq = audit_log.len()`; before each
   `decide()`, read `entries_range(start_seq, len)` and
   `sum_turn_usage` (best-effort: no audit → `0`, never breaks
   the run). Wire at the daemon spawn site.

5. **Status surface + INSTALL + exit + Frozen.** `aivyx loop
   status` shows the token cap (and, when a run is active, the
   tokens used so far). INSTALL section (the new knob + the
   total-spend-during-run framing); exit doc; README Frozen
   flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak: 12 → **13**. This phase
  adds **no new capability scope and no new tool** — just a
  config knob + driver logic reading the existing audit chain.
  No A3 amendment needed; the thirteen-tool core is untouched.
- **PRODUCT.md** — **Will hold.** Streak: 65 → **67**. A cap on
  an existing capability, not a new commitment.
- **`aivyx-core/src/lib.rs`** — **Will hold.** Streak: 12 →
  **13**. `TurnOutcome` is deliberately *not* changed; work
  lands in `aivyx-channel` + `aivyx-config`.

## Exit criteria

- [ ] `docs/PHASE_176.md` + README row + Phase 175 backfill —
  Task 1.
- [ ] `[loop].max_run_tokens` + validation; pure
  `sum_turn_usage` — Task 2.
- [ ] `decide()` `StopBudget` after wall-clock; `None` disables
  — Task 3.
- [ ] `run_loop_driver` snapshots start-seq + sums turn usage
  each iteration; no-audit degrades to no cap — Task 4.
- [ ] `aivyx loop status` shows the token cap — Task 5.
- [ ] DESIGN.md / PRODUCT.md / `aivyx-core/src/lib.rs` HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+10` to `+16`. *(The Phase 175 retro
  pinned the empirical band for glue-heavy loop phases at
  `+10..+16`; this one is the pure helper + `decide()` + config,
  with driver wiring covered transitively.)*

## Honest scope risks at sign-off

- **Window-sum, not loop-only attribution.** Turns from other
  channels / reflection crons that complete during the run
  count toward `max_run_tokens` (stops the run sooner — safe).
  Loop-only precision is a Phase 177 refinement.
- **Token cap, not cost.** No per-model pricing; a 1k-token
  Opus turn and a 1k-token Haiku turn count equally. A real
  cost model would need a pricing table (Phase 177+).
- **Checked at iteration boundaries.** A single runaway
  iteration is bounded by the agent's own max-steps + the gate
  timeout, not the token cap (which fires before the *next*
  iteration).
- **No audit → no budget.** When no audit log is configured the
  cap silently does nothing (the audit chain is the only usage
  source). Documented; `max_iterations` still applies.
- **Sixty-fifth consecutive deferral of the Channel Activation
  Milestone** — intentional hold.

## Direction after Phase 176

With the cap trio complete (iterations + wall-clock + tokens)
plus gate verification + the progress log, the loop arc is
substantially done. Natural next steps:

- **Phase 177 — loop precision/polish:** loop-only token
  attribution (a `fire()` that returns the session id), and/or
  a **Web UI loop pane** (backlog + iterations + gate/cap state
  + progress log + live spend + stop control).
- The post-172 roster carries forward: LLM-judged correction
  classification, tool/topic surfacing in `OutcomeSummary`,
  cryptographic PRNG, PDF full-compression page count, and the
  long-deferred **Channel Activation Milestone**.

## Prediction vs reality

_(Filled at exit.)_

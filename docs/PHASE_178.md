# Phase 178 — LLM-Judged Correction Classification

**Closing the Phase 172 honest debt.** Phase 172 shipped the
correction-signal learning loop with one foregrounded weakness:
the signal is **structural-only**. "The operator came back
within 60 s of a completed turn" counts a genuine rework, a
"thanks, perfect," and an unrelated new request all the same.
Phase 178 adds the LLM judgment that distinguishes them, so only
genuine reworks feed the correction ledger — the same
augment-an-existing-structural-signal move Phase 91 made for
recall feedback.

## The crux + the lean solution

A real praise-vs-rework classifier needs the operator's
**follow-up message text** — and the audit chain stores outcome
summaries, not transcripts (Phase 91's own judge ships with a
*placeholder* `response_text` for exactly this reason). Rather
than build a whole new turn-capture substrate, Phase 178 uses
the one place a per-turn operator message already correlates to
the correction signal: the **recall log**. The Phase 76 recall
provider already writes a `RecallEvent` (session + time + hits)
for every recall-firing turn, and it has the operator's
`user_message` at that moment. Adding a truncated `query_text`
field to `RecallEvent` gives the judge the follow-up message
with **perfect correlation** and **graceful degradation** — a
follow-up turn that fired no recall simply has no captured
query, so it gets no judgment and the structural signal stands
(the Phase 91/93 augment posture).

> **Privacy note.** This extends the recall log to hold a
> truncated copy of the operator's query text. The recall log is
> HKDF-domain-encrypted at rest, and the text is truncated, but
> it is a deliberate posture shift (the *audit* chain stays
> transcript-free; the *recall* log gains query content). Opt-in
> via the same condition as auto-recall, documented at sign-off.

## Design

- **`RecallEvent.query_text`** — truncated operator query,
  `#[serde(default)]` (old events decode with `""`). Populated
  by the recall provider.
- **Detailed correction events** — a `detect_corrections_detailed`
  that, per correction event, exposes the corrected turn's
  distinct topics **and** the follow-up turn's `query_text`
  (found by matching the follow-up outcome to its recall event).
- **`CorrectionJudgment`** — a 3-way verdict: `Rework` /
  `Praise` / `Unrelated`. A `CorrectionJudge` trait +
  `LlmCorrectionJudge` (one batched LLM call per reflection
  cycle, parser-tolerant), mirroring `recall_judgment`.
- **Judged fold** — behind a new `[correction_judgment]` config
  (off by default, like `[recall_judgment]`). When on, the
  Phase 172 correction fold judges each detailed event and folds
  **only `Rework`** events into the correction ledger;
  `Praise` / `Unrelated` / un-judged-no-query drop. When off,
  the structural fold is byte-identical to Phase 172.

## Tasks

1. **Open doc + README.** This doc, README Active row, backfill
   Phase 177's frozen hash (`3996fc1`).

2. **Query capture + detailed events.** Add
   `RecallEvent.query_text`; the recall provider populates it
   (truncated). `detect_corrections_detailed` yields per-event
   `{ corrected_topics, follow_up_query }`. Tests pin the
   capture + the follow-up correlation + the no-recall-degrade
   path.

3. **The judge.** `CorrectionJudgment` enum + `CorrectionJudge`
   trait + `LlmCorrectionJudge` (LLM request + a tolerant
   `parse_response`), mirroring `recall_judgment`'s shape. Tests
   pin the parser (Rework/Praise/Unrelated, casing, padding,
   unparseable → `None`).

4. **Judged fold + config.** `[correction_judgment]` config
   (`enabled`, `max_corrections_per_cycle`). Thread a
   `CorrectionJudgmentDeps` into the reflection pass; when armed,
   judge the detailed events and fold only `Rework` into the
   ledger. Off → Phase 172 structural fold unchanged. Tests for
   the only-Rework-folds path with a fake judge.

5. **Surface + INSTALL + exit + Frozen.** A last-cycle judgment
   stat in `aivyx learning` (judged / rework / praise / unrelated
   counts); INSTALL section; exit doc; README Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak: 14 → **15**. A
  wire-compat field on `RecallEvent`, a new `[correction_judgment]`
  config section, and a channel-tier judge — no new capability
  scope, no new tool, no new `KeyDomain`. No A3 amendment.
- **PRODUCT.md** — **Will hold.** Streak: 68 → **69**. Refining
  an existing self-learning signal, exactly as Phase 91 did —
  not a new commitment.
- **`aivyx-core/src/lib.rs`** — **Will hold.** Streak: 14 →
  **15**. Work lands in `aivyx-channel` + `aivyx-config`;
  `aivyx-core` untouched.

## Exit criteria

- [ ] `docs/PHASE_178.md` + README row + Phase 177 backfill —
  Task 1.
- [ ] `RecallEvent.query_text` captured + `detect_corrections_detailed`
  exposes the follow-up query — Task 2.
- [ ] `CorrectionJudge` + `LlmCorrectionJudge` + tolerant parser
  — Task 3.
- [ ] `[correction_judgment]` armed fold folds only `Rework`;
  off → Phase 172 byte-identical — Task 4.
- [ ] `aivyx learning` judgment stat — Task 5.
- [ ] DESIGN.md / PRODUCT.md / `aivyx-core/src/lib.rs` HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+15` to `+28`. *(A self-learning
  substrate phase like Phase 172, not a loop-polish phase —
  the judge parser + detailed detection + fold test denser. NOT
  the `+5..+12` loop band.)*

## Honest scope risks at sign-off

- **No-recall follow-ups are unjudged.** A follow-up turn that
  fired no auto-recall has no `query_text`, so it gets no
  judgment — it falls back to the structural signal (counted as
  a correction). Short follow-ups ("no.") are the likely
  unjudged case. Documented; the alternative (a universal
  per-turn capture) is heavier and deferred.
- **Recall log now holds query text.** The privacy posture shift
  above. Truncated + encrypted, but real.
- **One LLM call per reflection cycle.** Bounded cost (the Phase
  91 cadence); off by default.
- **Judgment is best-effort.** Parse failure / LLM outage → the
  event stays structural (no drop), same as Phase 91/93.
- **Sixty-seventh consecutive deferral of the Channel
  Activation Milestone** — intentional hold.

## Direction after Phase 178

The self-learning correction loop is now structurally + LLM-
judged. Remaining roster: tool/topic surfacing in
`OutcomeSummary` (broadens correction attribution beyond
recalled topics), cryptographic PRNG / PDF parser (both add a
dep), and the long-deferred **Channel Activation Milestone**.

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 15 | Untouched (a wire-compat `RecallEvent` field + a config section + a channel-tier judge; no scope/tool/`KeyDomain`) | ✅ |
| PRODUCT.md HOLD → 69 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 15 | Untouched | ✅ |
| Zero new workspace deps | Reused the recall log, the LLM provider, the reflection fold | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean | ✅ |
| Test count delta `+15` to `+28` | **`+21`** (capture/detailed 7 + judge 8 + fold 6 (4 helper + 1 integration + 1 config) — net across crates); workspace ~4,110 → ~4,131 | ✅ **in band** |

**Prediction note — the band discipline worked.** After three
loop-polish phases I over-predicted (175 +11 vs +12–22, 176 +5
vs +10–16, 177 +6 vs +8–14), I deliberately used a *different*
band here — `+15..+28`, the self-learning-substrate band (Phase
172 was +33) — and called out in the open doc that this was NOT
a loop phase. Landed `+21`, mid-band. The lesson held: pick the
band from the phase's *kind* (substrate-dense vs glue-thin), not
a single global prior.

The debt closed end-to-end:

1. **Capture + detailed events** (Task 2). `RecallEvent.query_text`
   (wire-compat, truncated, encrypted) + `detect_corrections_detailed`
   exposing each correction's follow-up query with perfect
   session/time correlation; graceful `None` when the follow-up
   fired no recall.
2. **The judge** (Task 3). `CorrectionJudgment` (Rework / Praise
   / Unrelated) + `CorrectionJudge` + `LlmCorrectionJudge`,
   mirroring Phase 91 — batched, tolerant parser, best-effort.
3. **Judged fold** (Task 4). `judged_correction_counts` folds
   only `Rework` (structural fallback for no-query / failed /
   over-cap); `[correction_judgment]` config; wired into the
   Phase 172 fold (off → byte-identical).
4. **Surface** (Task 5). `aivyx learning` shows the last cycle's
   `rework / praise / unrelated / structural` counts; INSTALL
   documents the knob + the privacy posture.

### Honest-debt status carried forward

- **No-recall follow-ups are unjudged** (structural fallback) —
  the universal-capture alternative stays deferred.
- **The recall log now holds truncated query text** — the
  documented privacy posture shift.
- **Corrections are still recall-topic-attributed** — tool/topic
  surfacing in `OutcomeSummary` is the next roster item.
- Sixty-seventh consecutive deferral of the Channel Activation
  Milestone.

### The result

The self-learning correction loop, opened at the very start of
this session as structural-only, is now **structurally
detected AND LLM-judged**: a genuine rework folds, a "thanks,
perfect" doesn't — built on the recall log + the LLM provider
the agent already uses, with no new dependency and an unbroken
DESIGN / PRODUCT / `lib.rs` streak.

# The Edge Between Uses — grading skill effectiveness (Chapter Strop)

> **Status: PLANNED (scoped 2026-07-04).** A whetstone sharpens the blade;
> a strop hones the edge between uses. Chapter Whetstone built the
> refinement loop (ledger → underperformer → LLM draft → governed
> supersession) and the 2026-07-04 self-learning dogfood proved it live —
> but only with an artificially raised floor, because the effectiveness
> signal it folds is **outcome-only** and turn outcomes are almost always
> `Completed`. Strop makes the signal honest: negative evidence flows in
> from the deterministic checks the platform already runs (Candor's
> claim check, the correction ledger), so "underperformer = net negative
> evidence at floor 0.0" becomes *reachable* without becoming *eager*.
> Refinement stays propose-only and operator-governed. No new agent tool,
> no new capability base, no P10 amendment, no new dependency.

## 1. Why — a score that can only go up measures usage, not service

The per-turn fold (`record_turn_skills`, wired at the daemon's
post-finalize hook) folds each invoked skill **+1.0 when the turn
`Completed`, −1.0 otherwise**. Live observation from the dogfood: local
models complete nearly every turn, including turns where the skill's
result was hollow, wrong, or claimed-but-never-done. So every skill's
decayed EWMA drifts positive, and at the default
`[skill_refinement] floor = 0.0` the refinement pass never finds an
underperformer organically. The WHETSTONE design intended the signal to
be "did invoking this skill lead to a turn that went well,
**cross-referenced with the correction ledger**" — the cross-reference
was deferred out of WH.3b and never landed. Consequences today:

- Whetstone is live-proven but organically dormant (the dogfood fired it
  only at `floor = 2.0`, which classifies *healthy* skills as
  underperformers — wrong as a default posture).
- Repertoire's Skills screen renders EWMA + samples that mean "how often
  is this skill used," not "how well does it serve."

## 2. Architecture & decisions (locked at scoping)

### A graded per-turn fold — Candor is the missing deterministic judge
`TurnOutcome::Completed` stops being an automatic +1. At the existing
fold site (the detached task in `daemon_server`, ~L2094), grade:

- **−1.0** — turn `Failed` / `Looping` (today's negative, unchanged), OR
  the turn Completed but **Candor flagged an unfulfilled claim**. This
  targets the dominant local-model failure: invoke the skill, claim the
  result, never do the work.
- **+1.0** — Completed, no unfulfilled claim (today's positive).

**ST.1 implementation decision (built):** the fold reads Candor's
verdict from the *annotation* the turn loop already embedded in
`final_message` ("⚠ {note}", matched against the exact finite `RULES`
note strings via `claim_check::has_unfulfilled_claim_annotation`) rather
than re-running the detection. Two reasons: the audit slice **cannot**
reconstruct `called_tools` (`ToolCall` entries carry `tool_id`, not the
name — the registry lives in the turn loop), and the annotation *is* the
turn loop's own registry-accurate computation, so reading it shares one
verdict instead of maintaining two. Exact-note matching means organic
model text (or a bare ⚠ glyph) can't false-positive. The grade itself is
the pure `skill_effectiveness::turn_folds_helpful(&TurnOutcome)`.

### Correction cross-reference at reflection cadence — the original design
`correction_detect::detect_tool_corrections` already identifies
**corrected turns** (a `completed` turn followed within the window by
another same-session turn) and folds `tool:`-namespaced keys into the
correction ledger. Strop adds the per-skill analog on the same cadence,
inside the existing reflection-scheduler pass (no new task):

1. Walk the lookback audit window once, building
   `turn_id → {invoked skills}` from `SkillInvocation` entries.
2. For each corrected turn (the `followup_outcome` definition, reused
   verbatim) whose turn id maps to invoked skills, fold **−1.0 per
   distinct skill** into the *skill-effectiveness* ledger.
3. Same-cycle dedup: a turn already graded −1 by the per-turn fold
   (Failed/Candor) must not double-count — track folded turn ids in the
   pass; the per-turn fold and the retro-fold are disjoint by
   construction only for Completed-and-unflagged turns, so the retro-fold
   skips turns whose outcome wasn't `completed` (already true of
   `detect_tool_corrections`) and accepts the small overlap where a
   Candor-flagged turn *also* gets corrected (two independent pieces of
   negative evidence about one use — folding both is defensible; decide
   at ST-OQ1 with a test either way).

Gating: the retro-fold arms only when `[skill_refinement]` is configured
(the ledger exists) — the same `Some(ledger)` gate as the per-turn fold.
It is **not** gated on the correction-attribution flag
(`attribute_tool_corrections`): that flag governs what enters the
*correction* ledger; Strop reads turn summaries directly and writes the
*skill* ledger.

### What deliberately does NOT change
- **`floor` stays 0.0.** The whole point: with real negative evidence,
  "net negative" is reachable; without incidents, nothing proposes. No
  relative ranking ("refine the bottom skill") — that manufactures
  underperformers from healthy rosters.
- **No LLM judge on skill turns.** Candor + corrections give targeted,
  deterministic negatives; an LLM grader adds cost and variance for
  little marginal signal. (If ever wanted, it slots into the same fold
  seam later.)
- **Propose-only + governance untouched.** Strop changes what the ledger
  *means*, not what Whetstone *does* with it.
- **Ledger format untouched.** Same decayed-EWMA rows, same
  `KeyDomain::SkillHelpfulnessLedger`; only the fold values feeding it
  are graded. Existing rows keep decaying — no migration.

## 3. Scope

**In:** the graded per-turn fold (Candor + outcome); the reflection-
cadence correction→skill retro-fold; tests for both (incl. the exact
dogfood shape: a Completed turn with a skill invocation and an
unfulfilled claim must fold negative); a live rig verification pass.

**Out:** LLM-judged skill grading; floor/threshold semantics changes;
Studio surfacing beyond what Repertoire already renders (the numbers
just start meaning something); the loop's CompletionJudge as a signal
source (loop stories rarely invoke skills; revisit if dogfood says
otherwise); sharing Candor's computation between the turn hook and the
fold (micro-refactor, only if it falls out naturally).

## 4. Phase plan (docs-first, small phases per convention)

| Phase | What | Proof |
|---|---|---|
| **ST.0** | This doc — scope + decisions locked. | Reviewed. |
| **ST.1** ✅ | **DONE.** Graded per-turn fold via `turn_folds_helpful` (pure, in `skill_effectiveness`) reading Candor's embedded annotation (`claim_check::has_unfulfilled_claim_annotation` — see the implementation decision above); `record_turn_skills` keeps its bool signature (the caller grades). Tests: the dogfood shape (Candor-annotated Completed folds unhelpful, with the fixture built through the real detect+append path), clean Completed folds helpful, Failed/Looping stay unhelpful, annotation detection rejects raw claim prose + organic ⚠ text. | `cargo test` green. |
| **ST.2** | Correction retro-fold in the reflection scheduler: `turn_id → skills` map from the lookback audit walk + `followup_outcome` reuse; fold −1 per corrected skill turn. Unit tests over canned summaries/audit entries; ST-OQ1 resolved with a test. | `cargo test`. |
| **ST.3** | Live rig verification: drive a skill turn that claims-but-doesn't (Candor negative), a skill turn the "operator" corrects next turn (retro-fold negative), and clean skill turns (positive); watch the ledger cross floor 0.0 and Whetstone file a refinement **at default thresholds** — the organic-firing proof the dogfood couldn't give. | Journal + `persona proposals list` on the rig; findings folded back here. |

## 5. Open questions (resolve in-phase)

- **ST-OQ1** — a turn graded −1 per-turn (Candor) that is *also*
  corrected next turn: fold once or twice? Leaning twice (independent
  evidence), capped by the existing per-window fold semantics; decide
  with a test in ST.2.
- **ST-OQ2** — should `SKILL_UNHELPFUL_NET` outweigh the positive
  (e.g. −2.0 vs +1.0) so one real incident isn't erased by two routine
  uses? Leaning keep 1:1 for ST.1 (simplest honest scale; decay already
  privileges recency) and let ST.3's live numbers decide.
- **ST-OQ3** — does the retro-fold need its own breadcrumb
  (`aivyx skill-effectiveness: retro-folded N …`) for observability?
  Leaning yes, one line, only when N > 0 (matches the house pattern).

# Phase 172 — Correction-Signal Learning Loop (self-improvement closure)

**First phase after the extended review.** The Aivyx Agent
Review (2026-06-05) named, as the headline unrealized gap
(§5.8, unrealized-potential #2), that *"the agent doesn't yet
self-improve in production."* The persona/profile substrate
(P13–P14) has every piece — reflection proposals, operator
approval, decayed ledgers, the proposal chain — but the one
signal a self-improving assistant most obviously needs is
absent: **the agent never notices when the operator corrects
it.** Phase 172 adds that signal and wires it end-to-end to a
Pending Persona proposal, so the agent can finally surface
*"you keep reworking my responses about X — want a Profile
note?"* on its own cadence.

## Why this, why now

- **The correction proxy already exists, unspent.** Phase 77's
  `recall_feedback` computes the structural "operator came
  right back within 60 s of a completed turn" event — the
  rapid-re-ask correction proxy — but spends it only as a
  `−1` toward *memory retention*. It is never surfaced as a
  first-class correction signal, never made durable, never
  drives a preference proposal. Phase 172 gives that exact
  event a second actuator.

- **Every adjacent signal is already a durable decayed
  ledger.** Helpfulness (Phase 82), co-occurrence (Phase 83).
  The correction signal is the one missing ledger in the set,
  and it is the most directly "self-improving" of them all.

- **The full loop is the point.** Per the operator framing,
  Phase 172 ships the *whole* loop, not just the foundation:
  signal → durable decayed ledger → consolidation → Pending
  Persona proposal through the existing Phase-70 chain. This
  mirrors how Phase 87 consumed the Phase 83 ledger into
  proposals; the correction path is symmetric.

- **Zero new workspace deps.** Pure substrate, reusing
  `aivyx_storage`, `serde`, the Phase-70 proposal chain, and
  the Phase-87 LLM-phraser pattern.

## What "correction" means here (the honest definition)

A **correction** is exactly the Phase-77 structural event:
a turn that **`completed`** but was **followed within
`CORRECTION_WINDOW_MS` by another turn in the same session**
(the operator immediately came back). Failed / timed-out
turns are *agent failures*, not operator corrections, and are
deliberately excluded — the correction signal is about "the
answer wasn't what you wanted," not "the tool broke."

Each correction is attributed to the **recalled topics
injected into the corrected turn** — the only structural
"what was this turn about" surface available (`OutcomeSummary`
carries no tool/topic). This is honest and coarse: a turn
with no recall contributes no correction signal (documented
scope risk), and a follow-up that *praises* rather than
corrects counts the same (the Phase-77 no-self-judgement
trade — refined by LLM judgement is a Phase 173+ candidate,
mirroring Phase 91's recall-judgment augmentation).

The correction ledger is a **distinct actuator on the same
raw event** as helpfulness: helpfulness nets `+1/−1` and
drives memory retention; the correction ledger counts only
the rework events and drives *operator-preference* proposals.
A topic can net positive helpfulness while still accumulating
corrections — those are precisely the topics worth a Profile
note.

## Tasks

1. **Open doc + ROADMAP + README.** This doc, roadmap entry,
   README status row, backfill Phase 171's frozen hash
   (`03fad6f`).

2. **Correction signal type + structural detector.** New
   `correction_detect` module: pure, no LLM, no I/O. Given the
   window's `RecallEvent`s + `OutcomeSummary`s, emit a
   `CorrectionTally` of per-topic correction counts (only
   `completed`-then-rapid-followup turns; excludes
   failed/timed-out/escalated/cancelled). Reuses the Phase-77
   `followed_quickly` proxy semantics. Tests pin the window
   boundary, the same-session gate, the completed-only gate,
   and the no-signal cases.

3. **Durable decayed correction ledger.** New
   `correction_ledger` module symmetric with
   `helpfulness_ledger` (Phase 82): EWMA half-life decay,
   prune epsilon + horizon, HKDF-isolated via a new
   `KeyDomain::CorrectionLedger` in `aivyx-storage`. Folded on
   the reflection cadence in `run_recall_feedback_pass` after
   the Phase-82/83 folds (so recall-feedback + both existing
   ledgers stay byte-identical). A corrupt/pruned row degrades
   only the longitudinal correction view.

4. **Consolidation → Pending Persona proposal.** New
   `correction_consolidation` module mirroring
   `persona_consolidation` (Phase 87): a `select_corrections`
   selector double-gating on `min_corrections` (decayed
   magnitude) + `min_samples`, dedup against the proposal
   chain under a canonical `correction:{topic}` id, capped per
   cycle; a `TopicPhraser` trait + `LlmTopicPhraser`
   production impl (reusing the existing `LlmProvider`); and a
   `consolidate_corrections` actuator filing each as a Pending
   `LearnedContext` proposal through the Phase-70 chain. Wired
   into the reflection pass behind a new
   `CorrectionConsolidationDeps`.

5. **Config + Learning surface + INSTALL + exit + Frozen.**
   New `[correction_consolidation]` config block (off unless
   present + `enabled = true`, like `[persona_consolidation]`),
   a `CorrectionLedgerStat` + cycle stat in the `aivyx
   learning` CLI and Web UI Learning digest, INSTALL note,
   exit doc with prediction-vs-reality, README/ROADMAP Frozen
   flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak: 8 → **9**. A
  channel-tier learning signal + a new `KeyDomain` variant are
  not a core-substrate contract change; the thirteen-tool core
  (amendment A12) is untouched, the audit/capability/persona
  contracts are unchanged.
- **PRODUCT.md** — **Will hold.** Streak: 62 → **63**. This is
  a new *source* of proposals consistent with the existing
  P8/P14 self-learning commitments — never a new commitment
  and never a new way of resolving proposals (the operator
  gate stays the sole authority). Same posture that held
  PRODUCT.md across Phases 82/83/87/91/92.
- **`aivyx-core/src/lib.rs`** — **Will hold.** Streak: 8 →
  **9**. All work lands in `aivyx-channel`, `aivyx-config`,
  and `aivyx-storage`; `aivyx-core` is untouched.

## Exit criteria

- [ ] `docs/PHASE_172.md` + ROADMAP entry + `docs/README.md`
  status row + Phase 171 hash backfill — Task 1.
- [ ] `correction_detect` emits per-topic correction counts;
  completed-then-rapid-followup only — Task 2.
- [ ] `correction_ledger` + `KeyDomain::CorrectionLedger`;
  decay + prune; folded on the reflection cadence — Task 3.
- [ ] `correction_consolidation` files a Pending
  `correction:{topic}` proposal through the Phase-70 chain;
  double-gated, deduped, capped, LLM-phrased — Task 4.
- [ ] `[correction_consolidation]` config + `aivyx learning`
  surface — Task 5.
- [ ] DESIGN.md / PRODUCT.md / `aivyx-core/src/lib.rs` HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+30` to `+50`.

## Honest scope risks at sign-off

- **Corrections are attributed to recalled topics only.** A
  corrected turn that injected no recall contributes no
  correction signal — invisible to the ledger. This is the
  same structural-surface limitation `OutcomeSummary` imposes
  on the whole reflection family; enriching it with tool/topic
  names is a Phase 173+ candidate.

- **The signal is structural, not semantic.** "Operator came
  back within the window" is coarse: a follow-up that praises,
  thanks, or asks an unrelated new question counts identically
  to a genuine rework. Phase-77 no-self-judgement ethos;
  LLM-judged correction classification (à la Phase 91) is the
  documented refinement.

- **Overlap with the helpfulness `−1`.** The same raw event
  feeds both ledgers. This is intentional and matches the
  established multi-actuator pattern (Phases 82/83/84/87 all
  consume overlapping recall signal), but operators reading
  both surfaces will see the correction count and the
  helpfulness dip move together.

- **Off by default.** Like `[persona_consolidation]`, the
  proposal-filing pass is opt-in. With the block absent the
  ledger still accumulates passively (visible in `aivyx
  learning`) but files nothing — no surprise-on-upgrade
  proposals.

- **Sixty-first consecutive deferral of the Channel
  Activation Milestone.** Per operator framing — intentional
  hold.

## Direction after Phase 172

- **Phase 173 candidate — tool/topic surfacing in
  `OutcomeSummary`.** Broaden the correction signal beyond
  recall-injected turns by carrying the corrected turn's tool
  names into the summary.
- **LLM-judged correction classification** (distinguish
  genuine rework from praise / unrelated follow-up), mirroring
  Phase 91's recall-judgment augmentation.
- The post-171 roster carries forward: cryptographic PRNG
  (Phase 169), full-document-compression PDF page count (Phase
  168), `arboard` clipboard (Phase 170), Silero VAD, streaming
  ASR, `access_role` deprecation, `build_agent_stack`
  promotion, secret-store integration, and the Channel
  Activation Milestone.

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 9 | Untouched | ✅ |
| PRODUCT.md HOLD → 63 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 9 | Untouched | ✅ |
| Zero new workspace deps | All work used existing primitives (`aivyx_storage`, `serde`, the Phase-70 chain, the Phase-87 phraser pattern) | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean (one `manual_contains` reworded in Task 2) | ✅ |
| Test count delta `+30` to `+50` | `+33` (detect 12 + ledger 6 + fold 1 + consolidation 9 + config-parse 4 + render 1); workspace ~4,002 → ~4,035 | ✅ (low end of band) |

The full loop closed end-to-end:

1. **Correction signal** (Task 2, commit `…`). New
   `correction_detect` counts, per recalled topic, the
   `completed`-then-rapid-followup reworks — reusing Phase
   77's `followed_quickly` + `match_outcome` (promoted to
   `pub(crate)` so the two consumers can't drift). Narrower
   than the helpfulness `−1`: agent failures and ambiguous
   outcomes are excluded.
2. **Durable decayed ledger** (Task 3). New
   `correction_ledger` symmetric with the Phase-82 helpfulness
   ledger; ~30-day half-life (half of helpfulness, since a
   correction is a transient signal); HKDF-isolated via the
   new `KeyDomain::CorrectionLedger`; folded + pruned on the
   reflection cadence after the Phase 82/83 folds.
3. **Consolidation → Pending proposal** (Task 4). New
   `correction_consolidation` mirroring Phase 87:
   double-gated, deduped, capped, LLM-phrased, filed under a
   canonical `correction:{topic}` id through the Phase-70
   chain behind the opt-in `[correction_consolidation]`
   block.
4. **Surface** (Task 5). `accumulated_corrections` +
   `correction_consolidation` threaded through the
   `GetLearningInsights` IPC → `aivyx learning` CLI +
   Web UI Learning tab; INSTALL section.

### What landed beyond the open

Nothing material. The surface plumbing touched more files
than a typical learning add (the `LearningInsights` payload
accretes ~16 optional fields now), but the change shape is
the established per-phase accretion.

### Honest-debt status carried forward

- Correction signal is structural-only (no LLM-judged
  classification yet) and recall-attributed (turns with no
  recall are invisible). Both are documented Phase 173+
  candidates.
- Sixty-first consecutive deferral of the Channel Activation
  Milestone — intentional hold.

### Self-improvement closure

Phase 172 is the first phase to wire the §5.8 gap the Agent
Review named: the agent now has a durable, operator-legible
signal for *what it keeps getting reworked on*, and an
opt-in path that turns that into a gated Profile proposal.
The substrate the review said was "looking for the
orchestration push" got one of its named pushes.

# Skills That Sharpen — the refinement loop (Chapter Whetstone)

> **Status:** 🪨 **WH.3 — the refinement engine shipped.** `skill_refinement::
> propose_skill_refinements` reads the WH.2 ledger's confidence-gated
> `underperformers`, has a `RefinementDrafter` (production `LlmRefinementDrafter`)
> draft a sharper procedure, and files a **governed supersession pair** —
> `AppendList(v2, provenance: agent, refined_from, version+1)` + `RemoveList`(the
> exact stored v1 JSON), cross-linked — into the existing persona-proposal log
> (the Agents approve/edit/reject UI). Deterministic ids dedup re-runs; only ever
> *proposes*. The daemon write-side fold + reflection-cadence scheduling + the
> `[skill_refinement]` config land in **WH.3b**. The locked reference for the
> chapter that turns Aivyx's skills from a *static list* into something
> that **gets better through use**. Skills already exist as first-class,
> governed parts of the agent's identity — authored by the operator
> (`skills.teach`) or proposed by the agent (the auto-proposer) — and a
> per-skill success/failure signal is already captured. What's missing is
> the **loop**: nothing measures how a skill *actually performs* over time
> and proposes a **refinement** when it underperforms. Whetstone builds
> that loop — for the operator's skills *and* the agent's own — closing
> the "agent refines your skills" gap. A refinement chapter: it reuses the
> persona-proposal governance, so **no new capability, base, amendment, or
> agent tool** — refinements surface in the existing Agents approve / edit
> / reject UI.

## 1. Why — skills don't learn from how they play out

A `LearnedSkill { name, trigger, procedure }` lives on the HMAC-chained
persona log; every turn its *trigger* is in the prompt and the agent pulls
the full *procedure* via `skills.invoke`. Two creation paths exist
(operator-authored + agent-proposed), and the Phase 116
`ToolRelevanceLedger` already records per-`(keyword, skill)` **success /
failure** counts. But:

- **A skill never improves once written.** A `code-review` skill the
  operator taught — or the agent proposed — stays exactly as authored,
  even if the turns that use it keep getting **reworked** by the operator
  (a clear "this procedure is wrong" signal that already lands in the
  correction ledger).
- **The effectiveness signal is unused for refinement.** Success/failure
  is captured but only feeds a prompt hint — nothing acts on a skill that
  is consistently *failing*.
- **The agent can't refine the operator's skills.** It can *propose new*
  skills, but there is no path for "your skill X is underperforming —
  here's a sharper version."

The fix is the same shape the memory stack already uses: **measure →
propose**. A decayed per-skill effectiveness ledger, and a reflection-
cadence pass that drafts a refinement for an underperforming skill and
files it as a governed persona proposal.

## 2. Architecture & decisions (locked)

### A richer skill model — provenance + lineage (backward-compatible)
`LearnedSkill` is stored as JSON in a persona-list delta, so it extends
**without churning the chain format** (old entries decode with defaults).
Add the fields a refinement needs to be *traceable and governable*:

- **`provenance`** — who authored this version (`operator` / `agent`) and,
  for a refinement, *why* (a one-line reason).
- **`refined_from`** + **`version`** — lineage: a refinement records the
  name/version it sharpened, so the operator sees "v2, refined from your
  v1 because turns using it were reworked."
- **`domain`** (optional) — a specialization tag, laying groundwork for
  the deferred knowledge-derived specialization chapter.

Today's free-text skills become `version 1`, `provenance: operator`,
unchanged in behavior. The `{ name, trigger, procedure }` core is
untouched.

### A per-skill effectiveness ledger — the proven EWMA pattern
A new `PersistentSkillHelpfulnessLedger` (a sibling of the Phase 82
per-topic helpfulness ledger): one row per skill name, a **time-decayed
EWMA** of "did invoking this skill lead to a turn that went well." Folded
on the existing reflection cadence from the per-turn signal already
captured (a skill was `skills.invoke`d + the turn's outcome, cross-
referenced with the **correction ledger** — turns the operator reworked).
Self-pruning, HKDF-isolated in a new `KeyDomain::SkillHelpfulnessLedger`;
a corrupt row degrades only the refinement signal, never skills, persona,
or recall. (Decay matters: a skill the operator already fixed shouldn't
keep triggering refinements.)

### The refinement loop — measure, draft, propose (governed)
On the reflection cadence, when a skill's decayed effectiveness falls
below a floor (or its correction rate is high) **and** it has enough
samples to be confident, the pass:

1. Asks the LLM to **draft a sharper version** of that skill's procedure,
   given the skill, its trigger, and the kind of turns that went wrong.
2. Files it as a **persona proposal** — a `LearnedSkill` delta that
   *supersedes* the original (the Phase 92 linked-supersession shape:
   retire v1, append v2 with `refined_from`/`version`), with
   `provenance: agent` + the reason.
3. The operator **approves / edits / rejects** it in the **existing**
   Agents persona-proposal UI — no new governance surface. Refinement
   works identically on **operator-authored** and **agent-authored**
   skills (both are just `LearnedSkill`s on the chain).

This reuses `skills.propose` / `reflection.propose` and the persona
governance wholesale: a refinement is operator-approved, on the signed
chain, audited, and revertible (`aivyx persona revert`) — the persona
invariant (every applied delta is operator-approved) holds.

### Opt-in, best-effort, byte-identical default
The refinement pass is **off by default** (a config knob alongside the
other reflection passes), arms only when auto-recall/reflection is
configured, and is best-effort — a draft failure, a ledger miss, or no
underperformers simply files nothing. Nothing about a turn changes; the
loop runs on the reflection cadence and only ever *proposes*.

### Governance: a refinement, nothing new
No new agent tool, capability base, P10 amendment. **One new storage
domain** (`SkillHelpfulnessLedger`, routine growth like every prior
ledger). One backward-compatible persona-format extension. The behavior
(propose a persona delta on the reflection cadence) is squarely inside
PRODUCT.md **P8**'s outcome-driven-audited-reflection envelope and **P14**
(persona governance) — the same envelope the skill *auto-proposer*
already lives in.

## 3. Scope

**In:** the `LearnedSkill` provenance/lineage/domain extension; the
per-skill effectiveness ledger + its reflection-cadence fold; the
refinement pass (detect underperformer → LLM draft → governed
supersession proposal) for operator *and* agent skills; the `[…]` opt-in
knob; tests.

**Out:** a dedicated **Studio Skills library** (refinements already
surface in the Agents proposal UI; a richer skill-management screen is a
follow-on); **knowledge-derived specialization** (the agent *authoring*
specialized skills from the wiki/graph — the deferred ask-(c) chapter); a
structured step/parameter DSL for procedures (the body stays free-text in
v1); auto-*applying* refinements without operator approval (they stay
proposals).

## 4. Phase plan (docs-first, small phases per convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **WH.0** | **This design contract** | locked reference; banner flips per phase |
| **WH.1** ✅ | **Richer skill model** | DONE. `LearnedSkill` gains `version: u32` (`#[serde(default = "1")]`), `provenance: SkillProvenance { author: SkillAuthor (Operator/Agent), reason: Option<String> }`, `refined_from: Option<String>`, `domain: Option<String>` in `aivyx-ipc::persona`, all `#[serde(default)]` + a manual `Default` (so a `..Default::default()` spread = v1/operator). Pre-Whetstone entries decode unchanged; the `skills.update` builder preserves lineage via `..existing.clone()`; the seven construction sites updated; the prompt render + `skills.invoke` ignore the new fields. **Inert**. Tests (fresh-skill defaults; a pre-Whetstone JSON decodes v1/operator; a refined v2 round-trips its lineage + provenance). |
| **WH.2** ✅ | **Skill-effectiveness ledger** | DONE. `KeyDomain::SkillHelpfulnessLedger` (domains 23→24) + `skill_effectiveness::SkillEffectivenessLedger` — a thin skill-named wrapper over the Phase 82 `PersistentHelpfulnessLedger` (same EWMA decay / half-life / prune), with `record_window`, `skill_score`, `ranked`, and the confidence-gated `underperformers(floor, min_samples, now)` (worst-first, `samples ≥ min`). `record_turn_skills(ledger, audit_entries, helpful, now)` folds the **distinct** `SkillInvocation` skills of a finished turn by its outcome (`±1`), failure-isolated. 3 tests (fold + decay; underperformers confidence-gate; turn-fold dedups by outcome). Storage 36 / channel 1001 + clippy green. The daemon **write-side wiring** (call it at the turn-finalize spawn) lands with WH.3, where the ledger is also read. |
| **WH.3** ✅ | **The refinement engine** | DONE. `skill_refinement.rs`: `propose_skill_refinements(ledger, learned_skills_raw, drafter, proposal_log, config, …)` reads `underperformers`, drafts a sharper procedure (`RefinementDrafter` trait + production `LlmRefinementDrafter`), and files the **linked supersession pair** — `AppendList(v2)` (`provenance: agent` + reason, `refined_from`, `version+1`) + `RemoveList(raw v1 JSON)`, cross-linked via `supersedes_proposal_id` — through `append_pending`. Takes the **raw** stored JSON so the retire matches even a pre-Whetstone entry; deterministic ids dedup re-runs; `enabled=false` default. 3 tests (underperformer → cross-linked pair w/ agent provenance + lineage + exact-v1 retire; healthy/disabled/missing → nothing; re-run dedups). Channel 1004 + clippy green. |
| **WH.3b** | **Wire it live** | plumb `Option<Arc<SkillEffectivenessLedger>>` through `DaemonConfig`/`ConnectionContext` + construct in `aivyx.rs`; call `record_turn_skills` at the turn-finalize spawn (helpful = `TurnOutcome::Completed`); schedule `propose_skill_refinements` on the reflection cadence (alongside the consolidation passes) reading the effective persona's `learned_skills`; the `[skill_refinement]` config section. |
| **WH.4** | **Finalize** | full suite + clippy + `cargo deny` green; affordance/docs (the refinement loop in the example config + a note in the Agents proposal docs); README domain count 23→24; status flip; record. |

**Discipline:** WH.1 ships the model substrate **inert** (a backward-
compatible format extension, no behavior). WH.2 adds the *signal*
(measurement only — no proposals). WH.3 is the single behavior phase, and
it only ever *proposes* (the operator approves), default-off, on the
reflection cadence — never touching a live turn. The chapter adds a
**loop**, not a capability: skills the operator and agent already create
now *sharpen*.

## 5. Open questions (resolve in-phase)

- **Underperformer threshold** — decayed-EWMA floor vs. correction-rate
  vs. both, and the minimum sample count before a refinement is
  considered. Pick conservative (don't nag); tune against the WH.3 test
  (WH.2/WH.3).
- **Supersession vs. in-place update** — file a linked retire-v1 / add-v2
  pair (Phase 92 shape, preserves lineage) vs. an update that mutates the
  skill. Default supersession — lineage is the point (WH.3).
- **Refinement cadence + cap** — every reflection cycle, capped at N
  proposals so the operator isn't flooded. Default 1–2 per cycle (WH.3).
- **Source of the effectiveness signal** — fold a fresh per-skill EWMA
  from turn outcomes vs. aggregate the existing `ToolRelevanceLedger`
  per-keyword skill counts. Default a fresh decayed fold (decay + a clean
  per-skill key); reuse the ToolRelevanceLedger capture as the input
  (WH.2).

---

*Chapter Whetstone is the stone the agent's skills are sharpened on: not
new skills, but the loop that watches how the skills it and the operator
already wrote actually play out, and proposes a better version when one
keeps failing — governed, traceable, and reversible, exactly like every
other change to the agent's identity.*

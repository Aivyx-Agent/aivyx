# Knowledge → Capability — specialized skills (Chapter Praxis)

> **Status:** ✅ **COMPLETE (PX.0–PX.3).** The agent now **authors its own
> specialized skills from its consolidated knowledge**: on the reflection
> cadence it reads a knowledge-rich, skill-less topic's wiki page + graph
> neighbourhood and proposes a grounded specialized skill (`domain`-tagged,
> agent provenance) that the operator approves/edits/rejects in the existing
> Agents UI. Opt-in (`[skill_authoring]`), default byte-identical,
> propose-only; **no new agent tool, capability base, P10 amendment, or storage
> domain** — it reuses the wiki/graph stores + persona governance. Full suite +
> clippy + `cargo deny` green; zero new deps. The locked reference for the
> chapter where the agent **authors its own specialized skills from its
> consolidated knowledge**. The memory stack already turns experience into
> *what the agent knows*: the [[CODEX]] wiki (per-topic consolidated
> `WikiPage`s) and the [[LATTICE]] typed graph (directed relations). Praxis
> closes the last loop — turning that declarative knowledge into
> *procedural capability*: when a topic has rich, connected knowledge but
> no skill covering it, the agent **synthesizes a specialized skill** from
> the wiki page + the graph neighbourhood and proposes it. *Praxis* is
> knowledge put into practice — **the wiki is what the agent knows; the
> skill is how it acts.** Like [[WHETSTONE]] (which *sharpens* existing
> skills), this is a governed, propose-only reflection pass — **no new
> agent tool, capability base, or P10 amendment.**

## 1. Why — the agent knows more than it can do

After Codex + Lattice, the agent accumulates real structured knowledge: a
`deploy` wiki page that consolidates everything it's learned about
deploys, and a graph saying `deploy —depends-on→ ci`, `ci —runs→ tests`,
`rollback —reverts→ deploy`. But that knowledge is **inert as
capability** — the next time the operator asks the agent to *do* a
deploy, it re-derives the procedure from scratch. Meanwhile skills (the
named procedures it *follows*) are only born two ways:

- **operator-authored** (`skills.teach`, seed), or
- the **auto-proposer** — which is *reactive*: it watches a *single good
  turn* and proposes generalizing it. It never sits back and notices "I
  have deep, connected knowledge about `deploy` and no skill for it — I
  should write one."

That's the gap: the agent's **declarative** knowledge (wiki + graph) never
crystallizes into **procedural** skill. Praxis is the deliberate,
knowledge-driven authoring path — the procedural counterpart to the wiki's
declarative consolidation. ([[WHETSTONE]] then keeps the authored skill
sharp; the WH.1 `LearnedSkill.domain` field — shipped as "groundwork for
the deferred knowledge-derived specialization chapter" — is the
specialization tag this chapter finally fills.)

## 2. Architecture & decisions (locked)

### A reflection-cadence authoring pass (sibling of Whetstone)
A pass on the existing reflection cadence — structurally a sibling of
Whetstone's refinement pass, reusing the same plumbing shape (a deps
bundle + a per-pass runner threaded through `run_reflection_scheduler`).
It **only ever files Pending persona proposals**; the operator approves /
edits / rejects them in the **existing** Agents UI. Off by default
(`[skill_authoring]`), byte-identical when absent, propose-only.

### Candidates: knowledge-rich **and** skill-less
A topic is a specialization candidate when it has **consolidated
knowledge** but **no skill yet**:

1. a `WikiPage` whose summary clears a substance floor (not a stub), AND
2. a non-trivial **graph neighbourhood** (the topic is a graph entity with
   relations — evidence it's a *connected, procedural* subject, not an
   isolated fact), AND
3. **no existing skill already covers it** — deduped against the effective
   persona's `learned_skills` by `domain == topic` (and name), so Praxis
   never competes with an operator-taught or already-authored skill.

Conservative by construction: a topic the agent barely knows, or one it
already has a skill for, is skipped. Capped per cycle so the operator's
review queue is never flooded.

### Source: the wiki page **+** the graph neighbourhood
The synthesis prompt is built from the agent's *own* consolidated
knowledge — never invented:

- the **`WikiPage.summary`** (what the agent knows about the topic), and
- the topic's **typed graph edges** (`out_edges` rendered as
  `subject —predicate→ object` lines — how it connects / depends / flows).

The LLM is asked to turn that into a **specialized skill**:
`{ name, trigger, procedure, domain = topic }` — a concrete, step-wise
procedure grounded in the relations, with `provenance = agent` and a
reason citing the source page. Best-effort: a draft failure / empty /
ungrounded output simply files nothing for that candidate.

### A *new* skill (single proposal), provenance + domain set
Unlike a Whetstone refinement (a v1→v2 *supersession pair*), an authored
skill is **new** — a single `AppendList(LearnedSkill)` persona proposal,
`version = 1`, `provenance: agent` (+ reason), and **`domain` set to the
source topic** (the first real use of the WH.1 field — the specialization
tag). Deterministic proposal id keyed by topic dedups re-runs and a prior
rejected/pending proposal (no nagging).

### Governance & reuse: a refinement, nothing new
No new agent tool, capability base, P10 amendment, or storage domain.
Praxis reads the **existing** wiki + graph stores (already in the daemon
for the Codex/Lattice sweeps), files through the **existing**
persona-proposal governance, and rides the **existing** reflection
cadence. It sits squarely in PRODUCT.md **P8** (outcome-/knowledge-driven
audited reflection) + **P14** (persona governance) — the same envelope as
the auto-proposer and Whetstone. Opt-in, default byte-identical,
propose-only, reversible (`aivyx-pa persona revert`).

## 3. Scope

**In:** the specialization engine (knowledge-rich + skill-less candidate
selection → wiki+graph synthesis → governed `AppendList` proposal with
`domain`/provenance); a `SpecializationDrafter` trait + production LLM
impl; the `[skill_authoring]` opt-in config + the reflection-cadence pass
wiring; tests.

**Out:** ~~a dedicated **Studio Skills library**~~ *(done — Chapter
Repertoire)*; ~~pulling in **raw memory** beyond the consolidated
wiki+graph~~ *(done — pre-v0.4.0: the topic's recent memory entries now
widen the synthesis context alongside the wiki summary + graph relations,
threaded through `SkillAuthoringDeps.memory`)*; a structured step/parameter
DSL for the procedure (free-text body, as today); auto-*applying* an
authored skill without operator approval (it stays a proposal).

## 4. Phase plan (docs-first, small phases per convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **PX.0** | **This design contract** | locked reference; banner flips per phase |
| **PX.1** ✅ | **The specialization engine** | DONE. `skill_authoring.rs`: `propose_specialized_skills(wiki_store, graph_store, learned_skills_raw, drafter, proposal_log, config, …)` selects knowledge-rich + skill-less topics (`min_summary_chars` floor + `min_edges` neighbourhood via `out_edges` + dedup vs `learned_skills` name/`domain`), renders the wiki summary + `subject predicate object` edges, drafts `{trigger, procedure}` via a `SpecializationDrafter` (trait + production `LlmSpecializationDrafter` with a tolerant `parse_drafted` JSON), and files a governed `AppendList` proposal (`version 1`, `provenance: agent` + reason, `domain = topic`). `SkillAuthoringConfig` added to `aivyx-config`. 4 tests (rich+skill-less → proposal w/ domain + agent provenance; thin page / sparse graph / already-skilled / disabled → none; dedup on re-run; JSON parse tolerance). Channel 1008 + clippy green. |
| **PX.2** ✅ | **Wire it live** | DONE. `[skill_authoring]` config (`RawSkillAuthoring` + `build_skill_authoring_config` in `aivyx-config`, default `None`). A `SkillAuthoringDeps` bundle + `run_skill_authoring_pass` (reads the effective persona's `learned_skills` + the wiki/graph stores → `propose_specialized_skills`), threaded as a new `Option<SkillAuthoringDeps>` through `run_reflection_scheduler`/`fire_reflection` beside Whetstone's pass. `rs_skill_authoring` assembled in `daemon_server` from the existing `DaemonConfig` `wiki_store`/`graph_store` + proposal/persona logs + a production `LlmSpecializationDrafter` built in `aivyx.rs`, armed only when `[skill_authoring].enabled`. 1 config test + the daemon/e2e literals. Channel 1008 / e2e 30 / cli 470 / config 351 + clippy green. |
| **PX.3** ✅ | **Finalize** | DONE. Full workspace suite + `cargo clippy --workspace` (0) + `cargo deny` (licenses + advisories) green; zero new deps. README (Praxis in phases; Rust tests; the "skills sharpen" highlight extended to "and authors new ones"), CHANGELOG entry, and a `[skill_authoring]` section in `examples/aivyx-pa.toml` (propose-only, surfaces in Agents UI). Status flipped to COMPLETE; recorded. |

**Discipline:** PX.1 is the testable engine (reads the stores, files
proposals — no scheduling). PX.2 only schedules it (no behaviour the engine
doesn't already have). Default stays off / byte-identical, and the pass is
**propose-only** — the agent authors a *candidate* specialized skill; the
operator decides if it joins the identity. Praxis adds an authoring *path*,
not a capability — the agent's own knowledge becomes its own skills.

## 5. Open questions (resolve in-phase)

- **Substance + neighbourhood thresholds** — the wiki-summary length floor
  and the minimum graph-edge count that make a topic "skill-worthy."
  Conservative defaults (a real paragraph + ≥2 typed relations); tune
  against the PX.1 test (PX.1).
- **Dedup key** — `domain == topic` only, or also fuzzy name match against
  existing skills? Default `domain`/exact-name; revisit if near-duplicate
  skills appear (PX.1).
- **Per-cycle cap** — how many specialized skills to author per reflection
  cycle. Default 1 (more conservative than Whetstone's 2 — authoring a new
  skill is a bigger ask of the operator than a refinement) (PX.1/PX.2).
- **Shared vs separate config** — a distinct `[skill_authoring]` section
  vs. folding into `[skill_refinement]`. Default separate (distinct pass,
  distinct knobs, distinct opt-in) (PX.2).

---

*Chapter Praxis is where the agent stops only knowing and starts being
able: it reads its own wiki and graph about a subject it understands well,
and writes itself a skill for it — a specialized procedure, grounded in
what it has actually learned, offered to the operator to make part of who
the agent is.*

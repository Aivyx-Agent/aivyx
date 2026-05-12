# Phase 56 — Profile + Persona Amendments (P13 + P14)

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Docs-only contract-amendment phase, same shape as Phase 22.
File the **Profile** (P13) and **Persona** (P14) amendments
against `PRODUCT.md`, reframe the locked pitch to the
operator's stated vision, and refresh the PRODUCT.md Delivery
Status section to current state. Zero code changes — the
implementation phases for P13 (Phases 57–58) and P14 (Phases
59–60) follow, sequenced through `ROADMAP.md`.

## Why now

1. **The pitch no longer reflects the project's vision.** The
   PRODUCT.md pitch was locked at Phase 12.5 (2026-04-15) as
   *"a personal autonomous agent platform that runs on your
   hardware …"*. The operator restated the vision post-
   Phase-55 as *"a self-learning, self-improving AI-personal
   assistant that has a user-defined Profile and Persona
   based on the end-user use-case"*. Both can be true (the
   second narrows the first to a more specific shape) but
   the contract should reflect the narrower target, not the
   broader one. The amendment process is how the contract
   absorbs vision shifts without silent drift.

2. **The Profile + Persona arc needs contract commitments
   before it has implementation phases.** Phases 57–60 are
   already scaffolded in `ROADMAP.md` (committed 2026-05-12).
   Opening any of those phases against an absent contract
   commitment would be the same shape failure Phase 22 was
   built to prevent — implementation drifting ahead of
   contract. P13 and P14 must land in `PRODUCT.md` before
   Phase 57 opens.

3. **The Delivery Status is stale.** PRODUCT.md's Delivery
   Status currently reads *"as of Phase 50 exit, 2026-05-12"*.
   Phases 51–55 have shipped since. The same opportunity
   that touches PRODUCT.md to file the amendments should
   refresh the status block, the way Phase 38's substrate-
   tool amendment also refreshed the status.

4. **Amendment process is well-exercised.** Seven amendments
   exist (A1–A7). The mechanism is no longer novel, the
   tooling is established, and a three-amendment batch in
   one phase has precedent (Phase 22 filed four).

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 56 is docs-only and
  touches only `PRODUCT.md` and amendment files. DESIGN.md
  remains untouched. Prediction: streak **extends to three**
  consecutive phases (currently at 2 since Phase 54's A3
  addendum break).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will break.** Three amendments + the
  pitch reframe + Delivery Status refresh all edit
  PRODUCT.md. Same precedent as Phase 22 Task 6's intentional
  break. The streak's purpose (detecting silent drift) is
  satisfied — breaking it through the formal amendment
  process is the mechanism working as designed. Prediction:
  streak ends at **six consecutive phases** (Phases 51–55
  held).
  Hash at entry: `b5beb16ef0014e32c4e805cd842380ffa68f7e8a79ec40d979e04628a375530b`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  Phase 56 is docs-only and touches zero `.rs` files.
  Prediction: streak **extends to five** consecutive phases
  (currently at 4 since Phase 51's deliberate D6 break).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

## Tasks

### Task 1 — Open commit + PHASE_56.md scaffold

This file. Update `docs/README.md` to show Phase 56 as Open.
Commit Q-block resolutions before proceeding to Task 2.

### Task 2 — Amendment A8: Pitch Reframe

Create `docs/amendments/2026-05-12-pitch-reframe.md`. The
amendment supersedes the LOCKED pitch sentence in PRODUCT.md
(line 27–29) with the operator's vision statement. Update
PRODUCT.md inline to reference the amendment per the existing
amendment-pointer convention (A5's substrate-tool-count
amendment is the precedent — the affected section gains a
blockquote pointer to the amendment file).

The new pitch (subject to Q1 resolution):

> **Aivyx is a self-learning, self-improving AI-personal
> assistant with a user-defined Profile and Persona, running
> on your hardware, talking to cloud or local LLMs under
> your own credentials, and never compromising privacy or
> auditability for the sake of a feature.**

Preserves the existing privacy / auditability / local-first
constraints (the original pitch's spine) while narrowing the
agent's identity from *"autonomous agent platform"* to
*"AI-personal assistant with user-defined Profile and
Persona"*.

## Task 2 ship record

**Files modified:**
- `docs/amendments/2026-05-12-pitch-reframe.md` (+78, new
  file): amendment narrowing the locked pitch from "personal
  autonomous agent platform" to "AI-personal assistant with
  user-defined Profile and Persona." Three substantive
  shifts documented (platform→assistant, cloud LLMs→cloud
  or local, addition of Profile/Persona). Privacy /
  auditability / local-first spine preserved.
- `PRODUCT.md` (+9, -3): pitch section header updated to
  *"LOCKED 2026-04-15, amended 2026-05-12"*; pitch sentence
  replaced; blockquote-style amendment pointer added in the
  same A5/A8 convention.

**PRODUCT.md byte-identity streak ended at 6 phases
(intentional).**

### Task 3 — Amendment A9: Product Commitment P13 — Assistant Profile

Create `docs/amendments/2026-05-12-product-commitment-p13-profile.md`.
Add Product Commitment 13 to PRODUCT.md as a new section
between P12 and the Status section. P13 establishes Profile
as the operator-declared, mostly-static identity layer —
distinct from the role envelope (capability gating) and from
Persona (P14, dynamic identity layer).

P13 should pin (subject to Q2 resolution):

- **The Rule:** Profile is the operator-declared identity
  layer specifying who this Aivyx instance is *for* and what
  use cases it serves. Profile carries operator-declared
  static fields including (at minimum) operator description,
  communication style preferences, primary use cases,
  behavioral preferences, and behavioral constraints.
  Profile is loaded once and injects into the system prompt
  at turn start, composed alongside (not inside) the
  role-derived envelope description.

- **What this commits us to:**
  1. Profile is a contract-level concept distinct from the
     role envelope. Roles gate *what the agent may do*;
     Profile shapes *how it speaks and judges*.
  2. There is exactly one Profile per Aivyx instance, scoped
     to the operator's identity (per P6).
  3. Profile is operator-declared at install/init time and
     mutable thereafter only by the operator (CLI or Web
     UI — not by the agent, not by reflection, not by any
     in-turn process).
  4. Profile is plain-text-inspectable. The operator can
     read it without unlocking the redb store.
  5. Profile injects into every turn's system prompt
     regardless of active role.

- **What this commitment deliberately does not say:**
  - Does not pin the exact field names or TOML/JSON shape
    (per P9 precedent — implementation choices for Phase 57).
  - Does not say Profile is encrypted (it carries no
    secrets; preferences are not credentials).
  - Does not say `aivyx init` is the only entry point for
    Profile creation; future channels may bootstrap it.
  - Does not preclude Profile from referencing external
    facts (memory entries, role names) — Profile may
    *point at* other state, it just isn't *that* state.

## Task 3 ship record

**Files modified:**
- `docs/amendments/2026-05-12-product-commitment-p13-profile.md`
  (+199, new file): A9 amendment adding P13 to PRODUCT.md as
  the operator-declared static identity layer distinct from
  role envelope (P7+P9), memory (G3), and Persona (P14).
  Pins seven commitments and six "deliberately does not say"
  carve-outs. Documents composition with roles, capabilities,
  memory, and Persona.
- `PRODUCT.md` (+109): new P13 section inserted between P12
  and Status. Rule sentence, seven numbered commitments
  (categories pinned per Q2(b) without field-naming), five
  "deliberately does not say" entries, composition note,
  inline amendment pointer in the same A5/A8 convention.

### Task 4 — Amendment A10: Product Commitment P14 — Persona

Create `docs/amendments/2026-05-12-product-commitment-p14-persona.md`.
Add Product Commitment 14 to PRODUCT.md as a new section
between P13 and the Status section. P14 establishes Persona
as the reflection-written, dynamic identity layer that grows
from Profile over the assistant's lifetime under the existing
P8 operator-gated reflection machinery.

P14 should pin (subject to Q3 + Q4 resolution):

- **The Rule:** Persona is the reflection-written dynamic
  identity layer that grows from Profile over the
  assistant's lifetime. Every Persona delta is proposed by
  the agent via the P8 reflection layer, approved by the
  operator through the P2 mission-gate machinery, and
  recorded in an HMAC-chained delta log audit-verifiable
  like the existing audit chain. There is no silent
  Persona modification.

- **Terminology note:** The operator's vision uses "Soul"
  for the same concept. The contract spelling is "Persona."
  Both refer to the evolving character-layer; contract
  docs, phase journals, and code use "Persona."

- **What this commits us to:**
  1. Persona modifications are P8-gated. Every delta is
     proposed, audited, and operator-approved. Silent
     drift is structurally impossible.
  2. Persona deltas form an append-only HMAC-chained log
     parallel to the existing audit chain. Operator can
     verify Persona evolution offline.
  3. The effective-identity layer at turn start is
     `Profile + sum(approved deltas)` — Persona augments
     Profile, never overwrites it.
  4. Operator can revert Persona to a prior state (delta
     log is append-only but the agent's *effective*
     Persona is computable up to any point in the chain).
  5. Persona is scoped to the operator's identity (per
     P6). One Profile, one Persona, one operator.

- **What this commitment deliberately does not say:**
  - Does not pin the delta categories beyond requiring
    them to exist (implementation choice for Phase 59).
  - Does not say reflection *must* propose Persona deltas;
    the operator may run Aivyx for years with empty
    Persona delta logs.
  - Does not preclude future delta export/import; the
    contract pins the security property (auditable,
    operator-gated) not the locality.
  - Does not commit Persona deltas to redb specifically;
    storage shape is implementation choice.

## Task 4 ship record

**Files modified:**
- `docs/amendments/2026-05-12-product-commitment-p14-persona.md`
  (+227, new file): A10 amendment adding P14 to PRODUCT.md
  as the reflection-written dynamic identity layer growing
  from Profile under P8-gated, P2-approved, HMAC-chained
  operator-reversible delta accumulation. Includes a
  prominent Terminology Note mapping operator's "Soul" to
  contract's "Persona" per Q4(b). Pins seven commitments and
  seven "deliberately does not say" carve-outs. Documents
  composition with Profile, roles, memory, and reflection
  and the "most differentiating commitment after P8" framing.
- `PRODUCT.md` (+118): new P14 section inserted between P13
  and Status. Terminology Note at the section head. Rule
  sentence, seven numbered commitments (both HMAC-chained
  *and* gate-threading properties pinned per Q3(c)), seven
  "deliberately does not say" entries, composition note,
  "most differentiating after P8" rationale, inline
  amendment pointer.

### Task 5 — PRODUCT.md Delivery Status refresh

Refresh the Delivery Status section (currently *"as of Phase
50 exit"*) to *"as of Phase 55 exit"*. Update P5/P11/P12
entries with their post-Phase-50 status (mostly mechanical —
the substrate is unchanged, just the dates roll forward).
Add P13 and P14 entries with **Forward** status and the
Phase 57–60 pointers from ROADMAP.md.

This is the same shape as Phase 22 Task 6 and Phase 38's
post-amendment status refresh — additive, no existing text
edited.

## Task 5 ship record

**Files modified:**
- `PRODUCT.md` (+71, -10): Delivery Status header refreshed
  from *"as of Phase 50 exit"* to *"as of Phase 56,
  2026-05-12"*. Intro paragraph rewritten to reflect five
  amendments (A5/A6/A7/A8/A9/A10) and 55-phase span. "All
  twelve P1-P12 fully shipped + P13/P14 forward" status
  statement. Forward (Not Yet Started) section gains P13 and
  P14 entries with phase pointers. New "Phase 51–55 —
  Chapter A Foundation Closeout + MCP sandbox" subsection
  documents Phases 51–55 with one-paragraph each. New
  "Phase 56 — Profile + Persona contract amendments"
  subsection records this phase's three amendments inline.

### Task 6 — docs/README.md amendment + phase pointer

Update `docs/README.md`:
- Phase 56 row moves from open to a Frozen row at exit.
- Amendment-process section gains a one-line note that
  the directory now holds ten amendments (A1–A10).

### Task 7 — Exit freeze

Standard exit procedure. Update `docs/README.md` Phase 56
row with the exit commit hash, draft prediction-vs-reality
block, fill exit criteria check marks.

## Deferrals

**Rolling deferrals carried into Phase 56:**
- None — Chapter A closed the backlog at zero load-bearing
  items.

**Net-new deferrals from Phase 56:** none expected. The
phase is contract-only; implementation deferrals belong in
Phase 57 onward.

## Prediction vs. reality

*(Filled at exit.)*

## Exit criteria

*(Filled at exit.)*

- [ ] Three amendments created in `docs/amendments/`:
  - A8: Pitch Reframe (supersedes PRODUCT.md pitch line)
  - A9: P13 — Assistant Profile
  - A10: P14 — Persona
- [ ] PRODUCT.md edited with inline amendment references at
  the pitch section, between P12 and Status (P13), and
  between P13 and Status (P14).
- [ ] PRODUCT.md Delivery Status section refreshed to Phase
  55 exit; P13 and P14 added with Forward status.
- [ ] docs/README.md updated to reflect Phase 56 Frozen and
  amendment count update.
- [ ] All Q-block questions resolved.
- [ ] DESIGN.md streak extends to three.
- [ ] PRODUCT.md streak ends at six (intentional).
- [ ] Production-core streak extends to five.
- [ ] Test count unchanged (docs-only phase).
- [ ] Prediction-vs-reality block filled.

## Open questions

**Q1 — Amendment batching strategy.** Three contract changes
need to land: (i) pitch reframe, (ii) P13 Profile addition,
(iii) P14 Persona addition. Options:

  - **(a)** Three separate amendments (A8/A9/A10), one file
    per change. Cleanest traceability — each amendment file
    maps to exactly one contract decision. Phase 22's
    A1–A4 four-amendment batch is the precedent.
  - **(b)** Two amendments — A8 covers pitch + P13 (the
    pitch reframe is the same vision-shift motivation as
    P13's identity-layer addition), A9 covers P14
    standalone (Persona is the more novel commitment and
    deserves its own file).
  - **(c)** One omnibus amendment — A8 covers all three
    changes in one file. Smaller file count but worse for
    future-archaeology (someone wanting to know "when did
    Persona land in the contract" reads three things at once).

  **Recommendation: (a).** Three amendments. Each contract
  decision deserves its own file. Phase 22's four-amendment
  batch proves the cardinality scales. The diff-cost is
  identical; the readability cost of (c) is worse.

**Q2 — P13 Profile field-set specificity.** How tightly
should P13 lock the Profile field set?

  - **(a)** P13 enumerates every field name (e.g. `name`,
    `operator_profile`, `communication_style`, etc.).
    Lock the shape now.
  - **(b)** P13 enumerates required field *categories*
    (operator description, communication style, use cases,
    preferences, constraints) without pinning field
    names. Defer field-naming to Phase 57.
  - **(c)** P13 pins only the principle ("Profile is
    operator-declared static identity") and defers the
    field set entirely to Phase 57.

  **Recommendation: (b).** Per P9 precedent (*"It does not
  pin the field names or the exact TOML shape"*), the
  contract should commit to the **categories and
  properties**, not the field-naming. (b) protects against
  having to amend P13 every time Phase 57 refines a field
  name. (c) is too loose — the categories are part of the
  contract's substance, not implementation.

**Q3 — P14 Persona storage commitment.** Should P14 commit
to a specific storage shape for the Persona delta log?

  - **(a)** P14 commits to "HMAC-chained, audit-verifiable
    like the existing audit chain" (pin the security
    property without pinning the storage backend). The
    deltas could share the audit chain or live in a
    parallel chain.
  - **(b)** P14 commits to "operator-approved through P2's
    mission-gate machinery" (pin the gate threading
    without pinning the storage property).
  - **(c)** Both — pin gate threading AND HMAC-chained
    storage as load-bearing security properties.
  - **(d)** Neither — defer to Phase 59 entirely. Pin only
    that Persona modifications are P8-reflection-based.

  **Recommendation: (c).** Both properties matter for the
  "no silent self-modification" invariant P8 already
  establishes for memory. HMAC-chained storage prevents
  tampering; gate threading prevents unilateral agent
  action. Pinning both prevents Phase 59 from drifting
  toward an unaudited or unapproved variant.

**Q4 — Persona vs Soul terminology in P14.** The operator
uses "Soul" for what contract docs call "Persona." How
should P14 handle this?

  - **(a)** Use "Persona" exclusively in P14 — no "Soul"
    mention. Cleaner contract reading.
  - **(b)** Use "Persona" in the rule + commitments, but
    include a short Terminology Note in P14 explaining
    the operator's "Soul" framing maps to the contract's
    "Persona." Preserves the connection.
  - **(c)** Use both interchangeably throughout.

  **Recommendation: (b).** The contract must be
  authoritative ("Persona" is the term in code and contract
  docs), but a one-line note saves future-session readers
  from confusion when they encounter "Soul" in
  operator-facing material or memory.

**Q5 — Delivery Status refresh in this phase.** Should
Phase 56 also refresh the PRODUCT.md Delivery Status block
(currently dated Phase 50 exit) to Phase 55 exit?

  - **(a)** Yes — folded into Task 5. The Delivery Status
    section is already out of sync, and refreshing it in
    the same commit batch as the amendments keeps PRODUCT.md
    in a consistent state at Phase 56 exit. Same precedent
    as Phase 38 (substrate amendment + status refresh in
    the same phase).
  - **(b)** No — leave the refresh for a dedicated "docs
    sweep" phase. Phase 56 stays narrowly scoped to the
    three new amendments.

  **Recommendation: (a).** The refresh is mechanical
  (Phases 51–55 are already fully documented in their own
  phase docs and ROADMAP entries) and avoids leaving
  PRODUCT.md inconsistent at phase exit.

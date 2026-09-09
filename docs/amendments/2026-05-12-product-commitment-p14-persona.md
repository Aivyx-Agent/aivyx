# Amendment A10 — Product Commitment P14: Persona

**Date:** 2026-05-12
**Phase:** 56
**Adds to:** PRODUCT.md — new Product Commitment 14
("Persona") inserted between P13 (Assistant Profile) and the
Status section.
**Implementing phases:** 59 (Foundation), 60 (Visualization)
— see ROADMAP.md.

---

## Terminology note

The operator's restated vision (post-Phase-55) uses **"Soul"**
as the evocative term for the assistant's evolving
character-layer. The PRODUCT.md contract, every phase
journal, and every code identifier uses **"Persona"** for
the same concept. Both refer to the dynamic counterpart of
Profile (P13) — the voice the assistant becomes over time
under operator-gated reflection.

When this amendment, P14 itself, or any future phase journal
says "Persona," it means the same thing the operator's
vision statement means by "Soul." Operator-facing surfaces
may use either term; contract docs, code identifiers,
amendment titles, and capability scope names use "Persona."

---

## What's new

PRODUCT.md gains a fourteenth Product Commitment: **P14 —
Persona**. P14 establishes Persona as the reflection-written
dynamic identity layer that grows from Profile (P13) over the
assistant's lifetime. Every Persona modification is
operator-gated through the existing P2 mission-gate
machinery, audit-logged in an HMAC-chained append-only delta
log parallel to the existing audit chain, and reversible by
operator action at any point.

Persona is the **most differentiating** commitment after P8
(Outcome-Driven Audited Reflection). Where P8 commits to
audited self-improvement at the *behavior* level (memory,
role overrides), P14 extends that posture to the *identity*
level — the assistant's voice itself evolves, but every
evolution step is operator-approved and audit-verifiable.

---

## The new rule

> **Persona is the reflection-written dynamic identity layer
> that grows from Profile (P13) over the assistant's
> lifetime. Every Persona modification is a structured delta
> proposed by the agent via the P8 reflection layer,
> approved by the operator through the P2 mission-gate
> machinery, and recorded in an HMAC-chained append-only
> delta log audit-verifiable like the existing audit chain.
> There is no silent Persona modification, no unaudited
> Persona modification, and no operator-bypassed Persona
> modification.**

---

## What this commits us to

1. **Persona modifications are P8-gated and P2-approved.**
   The agent proposes Persona deltas through the same
   `reflection.propose` surface that proposes memory and
   role-config edits today. The operator approves them
   through the same mission-gate machinery (`/approve`,
   `/reject`, Web UI buttons) that approves missions today.
   No new gating mechanism is introduced. Silent agent-
   driven Persona evolution is structurally impossible.

2. **Persona deltas form an append-only HMAC-chained log
   audit-verifiable like the existing audit chain.** The
   chain may share the existing audit chain or live as a
   parallel chain (implementation choice for Phase 59); the
   contract pins the *property* (HMAC-chained,
   append-only, offline-verifiable). The operator can run
   `aivyx-pa --verify-only` (or its successor) and confirm
   that the Persona delta log has not been tampered with
   since it was last verified.

3. **Effective Persona at turn start is `Profile + sum(approved
   deltas up to now)`.** Persona augments Profile — it never
   overwrites it. The effective-identity layer the daemon
   composes at the start of every turn is Profile (static,
   operator-declared) plus the deterministic application of
   every approved delta from the log. This makes the
   *effective Persona at any past moment* reproducible by
   replaying the chain.

4. **Persona is operator-reversible.** Because the delta log
   is append-only but the effective Persona is computed by
   *applying* deltas, the operator can revert to a prior
   effective Persona by appending a structured "revert
   delta" that nullifies one or more previous approved
   deltas. The audit chain remains immutable; the agent's
   effective voice returns to a prior state. The operator
   can also choose to grant a future role the capability to
   propose revert deltas, or keep that scope to themselves.

5. **Persona is single-instance per operator.** Per **P1**
   and **P6**, there is exactly one Persona delta log per
   Aivyx PA daemon, parallel to the single Profile. There is
   no "switch persona" gesture parallel to "switch role."

6. **Persona writes are capability-secured.** The agent
   needs an explicit capability (e.g., `persona.propose`,
   `persona.write`, exact base name TBD by the implementing
   phase) to propose a Persona delta. Operators who do not
   want autonomous voice-evolution simply do not grant this
   scope to any role; the substrate degrades to "Profile
   drives the assistant's voice, Persona delta log stays
   empty forever, all behavior remains operator-declared."

7. **Persona is plain-text-inspectable in its effective
   form.** The operator can view the current effective
   Persona (the result of Profile + applied deltas) without
   unlocking the redb store, the same way they read
   Profile today. The delta log itself may be HMAC-chained
   and stored encrypted-at-rest in the existing redb
   substrate, but the *effective* state is operator-
   readable.

---

## What this commitment deliberately does not say

- **It does not pin the delta categories.** Phase 59 chooses
  whether Persona deltas cover communication adaptations,
  learned context, character traits, relationship
  milestones, or other categories. The contract pins the
  *existence of structured deltas*, not the *enumeration of
  delta kinds*.

- **It does not say reflection *must* propose Persona
  deltas.** An operator may run Aivyx PA for years with an
  empty delta log. The contract pins what *happens when*
  Persona evolves, not that Persona *must* evolve.

- **It does not pin the storage backend.** Whether the
  delta log lives in a new `KeyDomain::Persona`, shares
  the audit chain's domain, or uses a separate file is an
  implementation choice. The contract pins HMAC-chaining
  and append-only-ness as load-bearing security properties.

- **It does not preclude future delta export/import.** A
  future feature may let an operator back up or transfer
  their Persona delta log to another machine they own. The
  contract pins the security property (auditable,
  operator-gated), not the locality.

- **It does not commit to a specific approval-gate UX.**
  The P2 mission-gate substrate already exists; Persona
  delta approvals reuse it. Whether the gate prompt
  renders deltas as diffs, as before/after voice samples,
  or in some other shape is implementation choice for
  Phase 59–60.

- **It does not say Persona changes are autonomous by
  default.** Per Commit 6, the capability gate is the
  operator's lever. Persona is opt-in per role.

- **It does not commit to a global Persona scope.** Per
  **P7**'s attenuation rules, a child role's Persona
  proposal writes are scoped to the child's own state, not
  the parent's. (Whether that means per-role Personas, or
  a single Persona with role-scoped proposal authority, is
  a Phase 59 decision.)

---

## Why this is the most differentiating commitment after P8

P8 already commits Aivyx PA to outcome-driven audited
self-improvement at the *behavior* level: memory writes
and role-config updates. P14 extends that posture to the
*identity* level: the assistant's *voice* itself can
evolve, but every evolution step is operator-approved and
audit-verifiable.

This is the second time the contract pins a property that
distinguishes Aivyx PA from "Claude with a memory store" or
"AutoGPT with self-improvement." P8 was the first — *outcome-
driven evolution with full audit legibility*. P14 is the
second — *identity evolution with full audit legibility*.
Together they define a single property: **the assistant
can become more useful over time, and every step of that
becoming is legible to the operator and reversible by the
operator.** No other agent product in the lane offers both
properties.

---

## How Persona composes with Profile, roles, memory, and reflection

- **Profile (P13)** is the static, operator-declared seed.
  Persona is the dynamic, reflection-written growth on top
  of Profile. Profile is *who the operator declared the
  assistant to be*; Persona is *who the assistant becomes*.

- **Roles (P7 + P9)** gate *what the agent may do*. Persona
  has no envelope of its own — it doesn't grant or
  restrict capabilities. A Persona delta can shape *how
  the agent communicates within a role's authority*; it
  cannot expand authority.

- **Memory (G3)** captures *observations* (facts the agent
  has read or derived). Persona captures *voice
  refinements* (how the agent has learned to speak with
  this operator). Memory is dynamic substrate; Persona is
  dynamic identity. A future implementation may have the
  reflection layer propose a memory write and a Persona
  delta in the same proposal — but they remain separate
  surfaces.

- **Reflection (P8)** is the *engine* that proposes Persona
  deltas. P14 commits to nothing new in the reflection
  machinery beyond extending the proposal surface with a
  new delta category (`persona`, or however Phase 59 names
  it). The gate-threading, audit-logging, and operator-
  approval semantics are all inherited from P8 + P2.

- **Effective system prompt at turn start** = Profile +
  sum(approved Persona deltas) + active-role
  `system_prompt` + role-derived envelope description. The
  four flow into one prompt; the four are conceptually
  distinct surfaces; the operator can inspect each
  separately.

---

## Forward — what Phase 59 and Phase 60 will land

Per ROADMAP.md (committed 2026-05-12):

- **Phase 59 — Persona Foundation.** Structured
  `PersonaDelta` records (HMAC-chained, possibly sharing
  the existing audit chain or parallel — Q-block
  decision). `reflection.propose` extended with a new
  delta category for Persona proposals. Gate threading
  reuses Phase 21 / 28–30 mission-gate machinery — the
  operator approves Persona deltas the same way they
  approve missions today. Effective-identity assembly at
  turn start composes Profile + accumulated Persona deltas
  into a single voice layer for the system prompt.

- **Phase 60 — Persona Visualization (closes P14).** Web
  UI Persona pane visualizing the delta log over time
  (timeline view of how the assistant's voice has
  evolved). Possibly identity export/import for
  backup/transfer (deferred decision; import re-binds the
  HMAC chain). After Phase 60 the project sits at: every
  PRODUCT.md commitment (P1–P14) delivered, identity layer
  fully shaped, operator-feedback posture re-established.

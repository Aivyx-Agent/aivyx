# Chapter W — Persona/Skills onboarding seed

## Why

`aivyx init` (the production first-launch wizard) captures the operator-declared
**Profile** and writes `[profile]` to `aivyx.toml`, which the agent adopts at
load (verified in the post-V audit). But the agent's **Persona** and **Skills**
are, by design, *self-learned* — they start empty and grow from reflection +
operator-gated proposals (PRODUCT.md P14 / P8). There is **no path to plant an
initial Persona/Skills set at onboarding**, and the richer "Genesis Wizard"
(`aivyx-tui/examples/genesis.rs`) that hints at "a Persona seed (which then grows
from use)" is a visual mockup, not wired to production.

Chapter W closes that gap: let the end user **seed** an initial Persona + starter
Skills at first launch, persisted and adopted by the agent **from turn one** —
while preserving the append-only, HMAC-signed integrity of the persona chain and
the learned-from-there-on model.

## The model this preserves

| Layer | Authoring | After onboarding |
|---|---|---|
| **Profile** | operator-**declared** (`[profile]`) | edited via Agents/Settings; load-time |
| **Persona** | now **seedable** at onboarding (`[persona_seed]`) → then **self-learned** | grows via reflection → gated proposals; revertable |
| **Skills** | now **seedable** at onboarding → then taught/auto-proposed | `skills.teach` / auto-proposer |

The seed is the **operator's** first authored content on the chain — it is
*pre-approved* (the operator wrote it), so it is appended directly as approved
`PersonaDelta`s, exactly like the `ImportPersonaChain` replay path, **not** routed
through the proposal/approval gate. From the seed onward, the chain grows only by
the normal learned + gated path.

## Hard design facts

1. **The persona chain needs the encrypted store + `persona_chain_key`**, which
   is only open once the daemon is running — but `aivyx init` runs *before* the
   daemon. So seeding is **config-driven**, not an init-time write to the chain:
   the wizard writes a declarative `[persona_seed]` section to `aivyx.toml`; the
   **daemon seeds the chain at boot**, the same place it already opens
   `PersistentPersonaLog` and folds `shared_persona` (`aivyx.rs` ~4375).
2. **Seed once, never overwrite.** The boot-seed fires **iff the persona chain is
   empty**. Once the chain has any entry (seeded or learned), the section is inert
   — onboarding can never clobber a grown persona. (Same "refuse on non-empty"
   posture as `ImportPersonaChain` without `force`.)
3. **Append-only + signed is preserved.** Seed entries go through the existing
   `PersistentPersonaLog::append`, so each is MAC-chained against the local key
   like any other delta. The seed is just the first real deltas.
4. **Live adoption.** After seeding, the daemon recomputes `shared_persona`
   (`Arc<RwLock>`), so the very next turn's `assemble_session_prompt` includes the
   seeded Persona/Skills — turn-one, no extra restart.

## What gets seeded (scope)

The **persona-specific** categories (the ones that are *learned*, not the
Profile-mirror scalars) + starter Skills:

- `learned_context` — facts about the operator/domain the agent should start with.
- `communication_adaptations` — voice refinements beyond the Profile's style.
- `character_traits` — emergent voice properties to start with.
- `relationship_milestones` — seed continuity ("we started building Aivyx today").
- `skills` — starter `LearnedSkill { name, trigger, procedure }` entries
  (`LearnedSkill` category, `AppendList` of the JSON payload).

Profile-mirror categories (`assistant_name`, `operator_profile`, …) stay in
`[profile]` — they are *declared*, not seeded, so the declared/learned boundary
stays clean.

## Config schema (`[persona_seed]`)

Joins the existing `[persona_lifecycle]` / `[persona_consolidation]` /
`[persona_auto_propose]` sections.

```toml
[persona_seed]
learned_context = ["operator is building a Rust agent platform called Aivyx"]
communication_adaptations = ["leads with code, minimal preamble"]
character_traits = ["pragmatic", "precise"]
relationship_milestones = ["genesis: first launch"]

[[persona_seed.skill]]
name = "rust-review"
trigger = "when asked to review Rust for safety"
procedure = "check unwraps, lifetimes, and error propagation; cite file:line"
```

Parsed into a `PersonaSeed` (all fields optional/empty-default). Absent section ⇒
`None` ⇒ no seeding.

## Mechanism

- **Primitive** (`aivyx-channel`): `seed_persona_chain_if_empty(log, shared,
  audit, &seed) -> Result<u64>` — returns `0` (no-op) when the chain is non-empty
  or the seed is empty; otherwise appends one operator-authored approved
  `PersonaDelta` per facet/skill (`proposal_id = "genesis-seed"` sentinel so the
  V.4 Change-History viewer can badge seeded deltas), recomputes `shared`, appends
  one `AuditEvent::PersonaSeeded { entries, categories }`, returns the count.
- **Startup hook** (`aivyx.rs`): right after `shared_persona` is built, call the
  primitive with the loaded `config.persona_seed`. (Adds a new `AuditEvent`
  variant → run the full e2e suite for count-assertion fallout, per the
  established discipline — see the e2e audit-chain note.)

## Phases

| Phase | Deliverable |
|---|---|
| **W.0** | This contract. |
| **W.1** | `[persona_seed]` config schema + `PersonaSeed` parse in `aivyx-config` (incl. `[[persona_seed.skill]]`); unit tests. |
| **W.2** | `AuditEvent::PersonaSeeded` + the `seed_persona_chain_if_empty` primitive in `aivyx-channel` (append operator-authored deltas, recompute, audit, refuse-on-non-empty); unit tests. |
| **W.3** | Wire the boot-seed into `aivyx.rs` startup; e2e + live-verify the seeded Persona/Skills fold into the system prompt turn-one; full suite for audit-count fallout. |
| **W.4** | `aivyx init` wizard capture → writes `[persona_seed]` via the config writer; optionally badge seeded deltas in the Agents Change History. |
| **W.5** | Finalize: e2e, docs, memory, push. |

## Out of scope (follow-ons)

- A **web** onboarding surface that authors the seed (the config-driven mechanism
  makes this a later, additive UI — write `[persona_seed]` + restart, or a live
  `SeedPersona` IPC).
- LLM-assisted seed drafting (the Phase 181 "describe it in words" idea from the
  genesis mockup).
- Promoting `genesis.rs` from a TUI example into the production first-run flow.

## Invariants

- **Chain integrity is never bypassed** — seed deltas use the same signed
  `append`; no raw unsigned writes.
- **Never overwrite a grown persona** — seed only when the chain is empty.
- **Declared vs learned stays clean** — Profile in `[profile]`; Persona/Skills
  seed in `[persona_seed]`, adopted onto the *learned* chain.
- **Every seed is audited** — one `PersonaSeeded` entry on the HMAC audit chain.

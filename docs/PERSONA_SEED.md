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

**Status: W.0–W.5 COMPLETE + live-verified.** `aivyx init` captures an optional
seed → `[persona_seed]` → the daemon plants it on the signed chain at first boot
(iff empty) → the agent adopts it turn-one. Live run: a 4-delta seed (traits +
context + skill) appeared on `ListPersonaDeltas` all stamped `genesis-seed`,
folded into `GetEffectivePersona` (`is_non_empty = true`), recorded one
`PersonaSeeded` audit entry, and **did not re-seed on restart**. The Agents
Change-History view badges `genesis-seed` deltas with a `seed` chip. The
generated `[persona_seed]` round-trips through the config loader (a wizard test
asserts it). Deferred (still): a **web** onboarding surface that authors the
seed, LLM-assisted seed drafting, and promoting `genesis.rs` into production.

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

---

# Chapter X — Persona seed: web authoring + LLM-assisted drafting

Closes the two Chapter-W follow-ons: a **web onboarding surface** that authors
the seed live (no restart), and **LLM-assisted drafting** ("describe your
assistant in words and Aivyx drafts the seed"), in both the Studio and the CLI.

## What's new vs. W

W was config-driven + boot-time (`[persona_seed]` → seed at boot iff empty). X
adds the **live runtime path**: a fresh agent (empty persona chain) can be seeded
from the Studio while the daemon runs, with **immediate adoption** (the daemon
recomputes `shared_persona` — same next-turn liveness as the persona governance
writes). The boot-seed (W) and the live-seed (X) share **one primitive**,
`seed_persona_chain_if_empty`, so both honor "never overwrite a grown persona."

## New IPC (wasm-clean `aivyx-ipc`)

Modeled on the persona-governance writes (`ResolvePersonaProposal` /
`RevertPersonaDelta`): `FrontendMessage` requests with `DaemonEnvelope` acks.

- **`SeedPersona { id, seed: PersonaSeedWire }`** → `PersonaSeedResolved { id,
  ok, appended, error }`. The daemon maps `PersonaSeedWire` → `aivyx_config::
  PersonaSeed`, calls the W.2 primitive **with the (now-open) audit log**, and
  recomputes shared state. Refuses (`ok = false`) when the chain is non-empty.
- **`DraftPersonaSeed { id, description }`** → `PersonaSeedDrafted { id, draft:
  Option<PersonaSeedWire>, error }`. The daemon runs a **one-shot LLM draft** of
  a seed from the operator's free-text description.
- `PersonaSeedWire { learned_context, communication_adaptations,
  character_traits, relationship_milestones, skills: Vec<SeedSkillWire> }`;
  `SeedSkillWire { name, trigger, procedure }` — plain-field mirror of
  `aivyx_config::PersonaSeed`, no config/llm dep.

## Shared drafting (`aivyx-channel`)

One implementation both the daemon handler and the CLI wizard call:
`persona_seed_draft::draft_persona_seed(provider, model, description) ->
Option<aivyx_config::PersonaSeed>`. Reuses the Phase-181 identity-draft pattern
(`chat_stream` → `finish` → parse labeled `KEY: value` lines, lists
comma-split). The **operator is always the author of record** — the draft only
pre-fills an editable form; nothing is planted until they confirm.

## Threading the LLM into the query handler

The daemon's query path has the `agent` but no plain `LlmProvider`. X threads
`Option<Arc<dyn LlmProvider>>` through `DaemonConfig → ConnectionContext →
handle_query` (the Chapter-U `config_toml_path` pattern), sourced from the
provider `aivyx.rs` already builds. `None` (no model) → `DraftPersonaSeed` returns
a typed "no model available" error; seeding still works (it's LLM-free).

## Web (Studio)

The Agents screen gains a **"Seed your assistant"** onboarding card, shown only
when the persona is empty (`is_non_empty == false` **and** the delta chain is
empty — a fresh agent). It offers: a description box + **Draft with AI**
(→ `DraftPersonaSeed`, fills the form), editable traits / context / one starter
skill, and **Plant seed** (→ `SeedPersona`). On success the existing
`refresh_tick` re-queries and the normal governance view replaces the card.

## Phases

| Phase | Deliverable |
|---|---|
| **X.0** | This contract. |
| **X.1** | `SeedPersona` IPC + `PersonaSeedWire` + daemon handler (live seed via the W.2 primitive, audited, refuse-on-non-empty); round-trip + handler tests. |
| **X.2** | `persona_seed_draft` shared drafter + `DraftPersonaSeed` IPC + thread `LlmProvider` into the query handler + daemon handler; tests. |
| **X.3** | Web: the Agents "Seed your assistant" card (describe → draft → edit → plant); `ws_task` arms; `stitch.css`. |
| **X.4** | CLI: an LLM-assisted draft option in `collect_persona_seed` reusing the shared drafter. |
| **X.5** | Finalize: bundle, e2e, live-verify, docs, memory, push. |

**Status: X.0–X.5 COMPLETE + live-verified.** Both W follow-ons shipped: a
fresh agent can be seeded live from the Studio (`SeedPersona`) and from a
free-text description the model drafts (`DraftPersonaSeed`), in both Studio and
CLI. Live run (Ollama up): the model **drafted** traits + a communication
adaptation + a `code-review-summary` skill from a one-line description;
`SeedPersona` planted 4 deltas (LLM-free); `GetEffectivePersona` flipped to
`is_non_empty = true` (the web swaps the Seed card for the governance view); a
second seed was **refused** ("already has content"). Boot-seed (W) and live-seed
(X) share the one `seed_persona_chain_if_empty` primitive — same integrity both
ways. Remaining deferred: promoting `genesis.rs` into the production first-run
flow.

## Invariants (carried from W)

- **One primitive** — boot-seed and live-seed both go through
  `seed_persona_chain_if_empty`: signed append, never overwrite a grown persona,
  always audited.
- **Operator is the author** — the LLM only drafts a form; the operator edits and
  confirms before anything is planted.
- **LLM-optional** — drafting degrades to a typed error with no model; seeding is
  LLM-free and always available.
- **Studio only / local-first** — same scope + offline rules as R–W.

## Chapter Outfit — default starter skills

W/X let the **operator** seed skills. Outfit gives a *fresh* agent a small,
curated repertoire even when the operator declares none — so a brand-new agent
can do real work on turn one instead of arriving with zero skills. It is the
skills analogue of the default cron routines and the default system-prompt
charter (Chapter Keel): an opinionated, default-on starting posture.

**The five starter skills** (`aivyx_config::default_starter_skills`):
`summarize-document`, `research-and-summarize`, `draft-reply`, `daily-briefing`,
`capture-note`. Each is a lightweight `{name, trigger, procedure}` recipe whose
procedure composes Aivyx's own tools and pillars (the Sheaf readers, `web.search`
/ `web.extract`, memory, workspace, persona) — the connective tissue that
activates the capabilities the charter tells the agent to use.

**Mechanism (compiled-in + genesis-planted).** The set is compiled into
`aivyx-config`, not written into `aivyx.toml`. At config-load the loader merges
it into `[persona_seed].skills` (`merge_starter_skills`), and the **existing**
one-time `seed_persona_chain_if_empty` plants it onto the signed chain at first
boot — so the seeding path is unchanged. A fresh agent (empty chain) gets the
skills; an already-running agent (non-empty chain) is **never** retro-injected.
Once planted they are ordinary `LearnedSkill`s: visible in the Studio Skills
library, scored by Whetstone, and removable via `skills.forget`.

**Config (`[skills]`).**

```toml
[skills]
starter = false   # opt out of the default repertoire (default: on)
```

**Precedence.** Operator-declared `[[persona_seed.skill]]` entries win on a
`name` collision; the remaining defaults are still appended.

### Invariants (Outfit)

- **Default-on, fully opt-out** — `[skills] starter = false` is byte-identical to
  a pre-Outfit build.
- **Genesis-only** — starter skills are a first-run gift, never injected into a
  grown persona chain (the `seed_persona_chain_if_empty` guard).
- **Operator wins** — a declared skill of the same name overrides the default.
- **No new tool/base** — skills orchestrate existing capabilities; the lightweight
  `{name, trigger, procedure}` model, not Anthropic's SKILL.md filesystem format.

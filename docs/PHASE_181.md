# Phase 181 — Guided First-Launch Identity Builder

**Chapter H, phase 2.** This is the moment the End User stops
configuring a tool and starts shaping a *relationship* — a
personal agentic assistant, potentially a confidant. The Phase
179 backend review named the gap: `aivyx init` collects three
free-text Profile fields with single-line prompts and writes the
config. The substrate is richer than that; the first launch
should be too.

## What exists, and the gap

The **Profile (P13)** substrate already supports **six** fields —
the declared identity layer that shapes the agent's voice and
judgment:

| Field | What it shapes | Collected today? |
| --- | --- | --- |
| `assistant_name` | What the operator calls it | ✅ |
| `operator_profile` | *Who the operator is* — the other half of the relationship | ❌ TOML-only |
| `communication_style` | How it talks — tone, verbosity, warmth | ✅ |
| `primary_use_cases` | 1–3 archetypes it's shaped around | ✅ (single) |
| `behavioral_preferences` | Voice-layer judgment defaults | ❌ TOML-only |
| `behavioral_constraints` | What it should *never* do — the trust boundaries | ❌ TOML-only |

The three uncollected fields — `operator_profile`,
`behavioral_preferences`, `behavioral_constraints` — are exactly
the **confidant dimension**: who you are to each other, how it
should behave, and the lines it must not cross. Today they
require hand-editing TOML, so almost no operator sets them.

## Design principles (the fixed constraints)

- **Local-first — the LLM is enrichment, never required.** The
  builder must fully function offline with guided prompts. An LLM
  is reachable at init (the wizard already calls
  `verify_provider_credentials` with the chosen provider/model/
  key), so the assisted path uses *that same* provider — and
  degrades to the manual path on any LLM error or a declined
  offer.
- **Profile, not Persona.** The builder enriches the *declared*
  layer (Profile). It does **not** pre-seed the **Persona/Soul**
  (P14) — that character is *earned* through reflection over time
  (HMAC-chained, gated proposals). Declaring it at launch would
  violate the P13/P14 separation. The builder shapes who the
  agent *starts* as; the Soul grows from there.
- **Consensual + skippable.** The builder *offers* to help; the
  press-Enter-to-skip minimal path is preserved, so an operator
  who wants the old terse flow keeps it.
- **Backward-compatible.** Existing configs and the non-guided
  path are unchanged; templates still pre-fill.

## The flow (operator-facing)

1. **The invitation.** After provider setup verifies, the builder
   asks — warmly — whether the operator wants help shaping their
   assistant ("I can ask you a few questions and draft an identity
   you can edit"). Decline → the guided manual prompts (all six
   fields, with examples/presets). No LLM reachable → same manual
   path, no dead end.

2. **The relationship conversation** (the confidant framing). A
   short, guided set of questions whose answers feed the draft:
   - *In your own words, what do you want this assistant to be
     for you?* (free text)
   - *What role should it play?* — collaborator / coach /
     confidant / assistant / (your own words)
   - *How should it talk to you?* — tone & warmth
   - *Is there anything it should never do?* — the hard lines

3. **The draft** (LLM-assisted). The chosen LLM drafts all six
   Profile fields from the conversation. A tolerant parser maps
   the response into the fields; any field the model omits falls
   back to the operator's own words or stays empty (never a hard
   failure).

4. **Review & edit.** Each drafted field is shown for the
   operator to accept or rewrite — the operator is always the
   author of record.

5. **Meet your assistant.** A warm rendered summary — *"Here's
   who I'll be: <name>. You're <operator_profile>. I'll talk
   <style>. I'm here to <use-cases>. I'll never <constraints>."*
   — that the operator **confirms / edits / restarts**. The first
   real moment of the relationship, before anything is written.

6. **Write.** The full six-field `[profile]` block (plus the
   Phase 180 `[sandbox]` default) is written.

## Architecture

- **`InitConfig` six-field expansion.** Carry all six Profile
  fields through the wizard (today: three). `render_toml` and
  `render_with_template` emit all six (lists as TOML arrays).
- **Identity-draft helper** (`identity_draft.rs` or in `init`):
  `compose_identity_prompt(answers) -> String` + a **tolerant**
  `parse_identity_draft(text) -> DraftedProfile` (the
  recall/correction-judgment parser pattern — labeled fields,
  case/whitespace tolerant, missing → fallback). The provider
  call reuses the verify-path construction; behind a trait seam
  so tests inject a fake provider.
- **The builder flow** in `run_init_wizard_inner`, using the
  existing injectable `prompt_line(reader, writer)` so the whole
  flow (offer → conversation → draft → review → preview →
  confirm/restart) is scripted-stdin testable.
- **The preview renderer** — pure `render_identity_summary(cfg)
  -> String`, unit-testable.

## Tasks

1. **Open doc + README.** This doc, README Active row, backfill
   Phase 180's frozen hash (`eec1893`).
2. **Six-field `InitConfig` + render.** Expand the wizard's
   Profile model to all six fields; `render_toml` +
   `render_with_template` emit them (arrays for the two lists +
   `operator_profile`). Round-trip tests (render → parse → all
   six survive).
3. **Identity-draft helper.** `compose_identity_prompt` + tolerant
   `parse_identity_draft` + the provider seam. Parser tests
   (clean, missing fields, list-splitting, garbage → fallback).
4. **The guided builder flow.** The invitation + relationship
   conversation + LLM-draft path + offline-manual fallback +
   review/edit, wired into the wizard. Scripted-stdin tests for
   the manual path + the fake-provider draft path + the
   LLM-error-degrades-to-manual path.
5. **Meet-your-assistant preview.** `render_identity_summary` +
   the confirm / edit / restart loop. Renderer + flow tests.
6. **INSTALL + exit + Frozen.** INSTALL section (the guided
   builder, the six fields, the offline behaviour); exit doc;
   README Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak: 17 → **18**. A richer
  *collection* UX over the existing P13 Profile fields + an
  optional LLM-draft step — no change to the Profile/Persona
  contract, no new scope/tool/`KeyDomain`. The Persona boundary
  is *respected*, not changed.
- **PRODUCT.md** — **Will hold.** Streak: 71 → **72**. Delivers
  the P13 first-launch experience more fully; not a new
  commitment.
- **`aivyx-core/src/lib.rs`** — **Will hold.** Streak: 17 →
  **18**. Work lands in `aivyx-channel` (the `aivyx` binary's
  init module) + `aivyx-config` (Profile already supports the
  fields); `aivyx-core` untouched.

## Exit criteria

- [ ] `docs/PHASE_181.md` + README row + Phase 180 backfill — T1.
- [ ] All six Profile fields collected + rendered + round-trip — T2.
- [ ] LLM identity draft with a tolerant parser + provider seam — T3.
- [ ] Guided flow with a working offline-manual fallback — T4.
- [ ] Meet-your-assistant preview + confirm/edit/restart — T5.
- [ ] DESIGN.md / PRODUCT.md / `aivyx-core/src/lib.rs` HOLD.
- [ ] Zero new workspace dependencies (reuse `aivyx-llm`).
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+14` to `+24`. *(Dense components: the
  identity-draft parser + the six-field render round-trip +
  scripted-stdin flow tests. Wizard UX with a parser, no IPC
  surface.)*

## Honest scope risks at sign-off

- **The LLM draft quality is model-dependent.** A weak local
  model may draft thin fields; the review/edit step is the
  backstop (the operator is always the author), and the parser
  degrades missing fields gracefully.
- **Interactive flow coverage is path-sampled, not exhaustive.**
  Scripted-stdin tests cover the key branches (manual, drafted,
  LLM-error→manual, restart); the full combinatorial space of
  edits is not enumerated.
- **No Persona seeding** — a deliberate boundary, not a gap. An
  operator wanting a strong starting character expresses it
  through `operator_profile` + the behavioral fields; the Soul
  earns the rest.
- **The draft call adds one LLM round-trip at init** — only on
  the opted-in assisted path, bounded, and skipped offline.

## Prediction vs reality

_(Filled at exit.)_

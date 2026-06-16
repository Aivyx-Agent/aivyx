# Agent Creation & Onboarding (Chapter Genesis)

> **Status:** 🟡 **design contract** (Chapter Genesis opening, GE.0). The locked
> contract for **unifying agent creation/onboarding across the CLI and the web
> Studio** onto one shared model. Scope: the **single assistant** identity
> (Profile + Persona + Soul); **team creation is deferred** (§3). This is a
> *consolidation* chapter — it extends a pattern already proven in one half
> rather than inventing a new mechanism.

## 1. The gap — onboarding is split, and the reusable half is trapped

Aivyx already creates a capable agent at first run, but the machinery is split
across two surfaces that don't share it evenly:

| Step | CLI (`aivyx init`) | Web Studio |
|---|---|---|
| Provider / model select | ✅ full wizard (`init.rs`) | ❌ none |
| **Profile** draft (declared identity) | ✅ `identity_draft.rs` (LLM) | ❌ none |
| **Persona seed** draft (learned voice) | ✅ shared `persona_seed_draft` | ✅ shared `persona_seed_draft` |
| Access level | ✅ wizard | ✅ Settings (`SetAccessLevel`) |
| Persona governance / Profile edit | — | ✅ Agents screen (`SetProfile`) |

Two problems fall out of that table:

1. **The Profile drafter is trapped in the CLI binary.** `identity_draft.rs`
   (`IdentityAnswers → DraftedProfile`, six P13 fields, LLM-assisted) is a clean,
   surface-agnostic module — but it lives in `crates/aivyx-cli/src/bin/aivyx_modules/`,
   so the daemon and the web **literally cannot call it.** The web has no
   Profile-drafting onboarding at all.

2. **There is no guided first-run flow in the web.** The Studio has piecemeal
   panels (the Agents persona-seed card, the Settings access editor) but nothing
   that *sequences* a brand-new user from "empty daemon" to "agent that feels
   like mine."

## 2. The key insight — Chapter X already proved the pattern

The **Persona-seed** half is **already unified**: `persona_seed_draft.rs`
(`aivyx-channel`) is, per its own module doc, *"shared by the daemon's
`DraftPersonaSeed` IPC handler (the Studio) and the `aivyx init` wizard (the
CLI), so there is one drafting implementation."* Chapter X built exactly the
unification this chapter needs — one drafter, two surfaces, daemon-side LLM
behind IPC, local-first fallback, operator-as-author-of-record.

Genesis does **not** invent a mechanism. It **applies the Chapter-X pattern to
the Profile half** and then **sequences** the (now-shared) steps into a real web
flow, reusing the IPC that already exists:

| Persona-seed half (Chapter X — done) | → Profile half + sequence (Genesis) |
|---|---|
| `persona_seed_draft` in `aivyx-channel` | lift `identity_draft` into `aivyx-channel` (GE.1) |
| `DraftPersonaSeed` IPC + `PersonaSeedWire` | new `DraftProfile` IPC + `ProfileDraftWire` (GE.2) |
| `SeedPersona` plants the seed | `SetProfile` already writes the Profile (Chapter V) |
| Studio Agents persona-seed card | a sequenced Studio **onboarding flow** (GE.3) |
| `aivyx init` calls the shared drafter | `aivyx init` calls it from its new home (GE.1) |

After Genesis, both surfaces drive the **same** Profile drafter, the **same**
Persona-seed drafter, and the **same** config writers — the split in §1's table
is gone.

## 3. Scope — single assistant now; teams deferred (on purpose)

**In scope:** onboarding the user's one assistant — its **Profile** (declared),
**Persona** seed (learned), provider/model, and access level.

**Out of scope — team creation/onboarding.** The Nonagon ships today as a
hardcoded `default_nonagon()` roster (lead + 8 specialists) or a vertical pack's
`TeamConfig`; the Studio Teams screen is read-only and editing was deferred
(Chapter Y). Building team *creation* is greenfield and much larger — and it
would **reuse** the single-assistant onboarding primitives (per-member Profile/
Soul drafting, the shared drafter, the sequenced flow) once they exist. So
Genesis is the **foundation** a future "Chapter Roster" (team creation) builds
on, not a detour from it. Recorded here so the boundary is explicit, not
forgotten.

## 4. The daemon chicken-and-egg — and how Genesis resolves it

`aivyx init` writes `aivyx.toml` **before** the daemon boots — true cold-start.
The web Studio only talks to an **already-running** daemon. So the web cannot be
the *very first* surface a user touches without a daemon that boots unconfigured
and serves an onboarding wizard (a larger change).

**Resolution for this chapter:**

- `aivyx init` **remains the canonical cold-start path** (no daemon required).
- The **web onboarding flow targets a running daemon** — the *complete-your-
  agent* / *re-onboard* / *refine-identity* path. This is the common real case:
  the daemon is installed and started with a minimal config (provider + model,
  the one thing that genuinely must precede boot), then the user does the
  identity-shaping (Profile + Persona + access) in the Studio against live IPC.
- Both surfaces drive the **same shared drafters and the same config writers**,
  so they never diverge.
- **Explicitly out of scope:** an "unconfigured daemon serves a browser
  onboarding wizard" boot mode. Noted as a possible future (it would make the
  web a true cold-start surface), but it is not required to close the split.

## 5. Provider/model from the web (open — GE.4)

Provider+model is the one setting that must exist *before* the daemon boots, so
it's the awkward edge of the web flow. Two options, decided at GE.4:

- **(a) Assume provider/model is CLI/installer-set**, and the web flow starts at
  the Profile step. Smaller; honest about the chicken-and-egg.
- **(b) Add a provider/model config writer** (extend the Chapter-U `toml_edit`
  `write_*_section` family with a `[provider]`/`[model]` writer + IPC + a web
  step), restart-required like the other config writes. Larger, but makes the
  web flow complete end-to-end. *Lean: (a) for GE.3, revisit (b) as GE.4 once
  the rest of the flow is proven.*

## 6. What's deliberately *not* here

- **Not a new drafting mechanism.** Genesis reuses the Chapter-X drafter shape;
  the LLM stays daemon-side behind IPC, local-first, operator-as-author.
- **Not team creation** (§3).
- **Not a daemon cold-start web mode** (§4).
- **Not Soul authoring UI.** The Soul is part of identity but its dedicated
  editor is out of scope; Genesis sequences Profile + Persona + access.

## 7. Phase plan

| Phase | Deliverable |
|---|---|
| **GE.0** | This contract. |
| **GE.1** | Lift `identity_draft` (Profile drafting) out of the CLI bin into `aivyx-channel`, next to `persona_seed_draft`; `aivyx init` consumes it from the new home (pure move — no behavior change, existing tests green). Now reusable by the daemon. |
| **GE.2** | `DraftProfile` IPC + a wasm-clean `ProfileDraftWire` in `aivyx-ipc` (mirrors `DraftPersonaSeed`/`PersonaSeedWire`); daemon handler calls the lifted drafter. `SetProfile` (Chapter V) already persists the result — no new write path. |
| **GE.3** | The Studio **onboarding flow**: a sequenced first-run view that chains Profile (DraftProfile → review/edit → SetProfile) → Persona seed (DraftPersonaSeed → SeedPersona, exist) → access level (SetAccessLevel, exists), against a running daemon (§4). Reuses the existing Stitch component kit. |
| **GE.4** | Decide §5: either document the CLI/installer-set provider assumption, or add a provider/model config writer + IPC + a leading web step. |
| **GE.5** | Finalize — CLI ↔ web parity check (both produce the same Profile/Persona artifacts via the same code), full build + test, docs, chapter memory. |

## 8. Open questions (resolved at GE.N)

- **F-1 (GE.1):** does `identity_draft` move into `aivyx-channel` as a new module,
  or a small new `aivyx-onboarding` crate that `aivyx-channel` re-exports? *Lean:
  a module in `aivyx-channel` next to `persona_seed_draft` — same crate the CLI
  and daemon already share, no new crate to justify.*
- **F-2 (GE.3):** is the onboarding flow a distinct Studio `View` (its own nav
  entry + first-run detection), or a modal launched from the Agents screen?
  *Lean: a distinct first-run View that the Command Center routes to when the
  Profile is empty, falling back to manual nav otherwise.*
- **F-3 (GE.4):** provider/model web write — §5 (a) vs (b).

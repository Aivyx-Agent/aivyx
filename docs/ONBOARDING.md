# Agent Creation & Onboarding (Chapter Genesis)

> **Status:** ✅ **shipped** (Chapter Genesis complete, GE.0–GE.5). This began as
> the design contract and is now implemented: the Profile drafter lifted into
> `aivyx-channel` (`profile_draft.rs`, GE.1) so one drafter backs both surfaces;
> the `DraftProfile` IPC + wasm-clean `ProfileDraftWire` (GE.2); the Studio
> **"Create your agent"** flow sequencing Profile → Persona seed → access over
> existing IPC (GE.3); provider/model kept CLI-set and shown read-only (GE.4,
> §5). Scope: the **single assistant** identity (Profile + Persona + Soul);
> **team creation was deferred at the time** (§3, to what this doc called "a
> future 'Chapter Roster'") — **that chapter has since shipped**
> (`docs/ROSTER.md`, status COMPLETE): the operator can now create and edit a
> team from the Studio's Teams screen, and a starter-team choice is folded
> into this very onboarding flow (`OnboardingTeamStep`). §3/§6 below are kept
> as a historical record of Genesis's own original scope, not a current
> statement that team creation is still unbuilt. A *consolidation* chapter — it extended the Chapter-X persona-seed
> unification to the Profile half rather than inventing a mechanism. After it,
> both the CLI wizard and the web Studio drive the **same** drafters and the
> **same** config writers (the §1 split is gone).

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

**As Genesis originally shipped:** onboarding the user's one assistant — its
**Profile** (declared), **Persona** seed (learned), provider/model, and access
level — was in scope; team creation/onboarding was out of scope, on the theory
that a future "Chapter Roster" would build on Genesis's primitives once they
existed. **That's since happened** — see `docs/ROSTER.md` (status COMPLETE):
the Studio Teams screen is no longer read-only, an operator can create/edit a
full team from the GUI, and a starter-team step now runs as part of this same
onboarding sequence. The paragraph below is kept verbatim as the original
design rationale, not a current "still not built" statement.

**Out of scope [at Genesis's own original ship] — team creation/onboarding.**
The Nonagon ships today as a hardcoded `default_nonagon()` roster (lead + 8
specialists) or a vertical pack's `TeamConfig`; the Studio Teams screen is
read-only and editing was deferred (Chapter Y). Building team *creation* is
greenfield and much larger — and it would **reuse** the single-assistant
onboarding primitives (per-member Profile/Soul drafting, the shared drafter,
the sequenced flow) once they exist. So Genesis is the **foundation** a future
"Chapter Roster" (team creation) builds on, not a detour from it. Recorded
here so the boundary is explicit, not forgotten.

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

## 5. Provider/model from the web — RESOLVED (GE.4): option (a)

Provider+model is the one setting that must exist *before* the daemon boots, so
it's the awkward edge of the web flow. Two options were on the table:

- **(a) Assume provider/model is CLI/installer-set**, and the web flow starts at
  the Profile step. Smaller; honest about the chicken-and-egg.
- **(b) Add a provider/model config writer** (extend the Chapter-U `toml_edit`
  `write_*_section` family with a `[provider]`/`[model]` writer + IPC + a web
  step), restart-required like the other config writes. Larger, but makes the
  web flow complete end-to-end.

**Decision (GE.4): (a).** The web flow does **not** write provider/model. The
daemon can't be talking to the browser at all unless a provider/model is already
configured (it's load-time and precedes boot), so writing it from the live web
session is the wrong layer — that's `aivyx init`'s job (the cold-start path,
§4). The onboarding intro instead **shows** the configured provider/model
read-only (from the existing `GetSettings` snapshot, which already carries both),
with a one-line pointer to `aivyx init` / the config for changing it. Option (b)
remains a clean future add if a true browser cold-start mode (§4) ever lands —
at that point the daemon boots unconfigured and a provider/model *write* step
becomes necessary rather than redundant.

## 6. What's deliberately *not* here

- **Not a new drafting mechanism.** Genesis reuses the Chapter-X drafter shape;
  the LLM stays daemon-side behind IPC, local-first, operator-as-author.
- **Not team creation** (§3) — **at the time**; `docs/ROSTER.md`'s Chapter
  Roster has since shipped this.
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

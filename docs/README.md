# Aivyx Docs

This directory holds **phase records** and **amendments**. It is
deliberately separate from the two contract documents in the repo
root:

- **`DESIGN.md`** is the **technical contract** — the locked
  architectural decisions ("how the agent works") that every phase
  must respect. It is edited rarely, and only through the amendment
  process described below.
- **`PRODUCT.md`** is the **product contract** — the locked
  product-shape decisions ("who the agent is for, what it commits
  to do, where the line is") that every phase must respect. It is
  the sibling of `DESIGN.md`, edited under the same amendment
  process. Drafted at the Phase 12.5 product-shape review.
- **`docs/PHASE_N.md`** are **phase journals** — working documents for
  one phase at a time. They churn freely during the phase and are
  frozen at phase exit.
- **`docs/ROADMAP.md`** is the **technical roadmap** — one-paragraph
  intents for upcoming numbered phases.
- **`docs/PRODUCT_ROADMAP.md`** is the **product roadmap** — the
  sibling of `ROADMAP.md`, listing forward product-shape milestones
  derived from `PRODUCT.md`'s commitments. Milestones are named, not
  numbered, and may span one or more technical phases.
- **`docs/amendments/`** (created when first needed) holds one file per
  contract change. An amendment is the *only* legal way to modify a
  locked `DESIGN.md` *or* `PRODUCT.md` decision from inside a later
  phase.

## Why the split?

The failure mode this layout defends against is **silent contract
drift** — a later phase quietly editing a Phase 0 decision because it
became inconvenient. With the contract in one file and the phase work
in another, any such edit shows up as a `DESIGN.md` diff in a phase
commit, which is an obvious review flag.

Rule of thumb: if you're writing something that future-you in a fresh
session needs to rely on, it goes in `DESIGN.md` (technical) or
`PRODUCT.md` (product). If it's a decision log, task list, or "why we
chose A over B today," it goes in the phase doc. If it's a one-paragraph
intent for a future phase or milestone, it goes in `ROADMAP.md` or
`PRODUCT_ROADMAP.md`.

## Cross-phase reference docs

Living documents that span multiple phases and capture patterns
learned across the phase sequence. Unlike phase journals, these are
edited freely when a new adapter or subsystem teaches us something new.

- [`ADAPTER_PATTERN.md`](ADAPTER_PATTERN.md) — future-proof checklist
  for adding a new `ChannelContext` adapter, grounded in the two
  adapters in tree (`LocalChannel`, `TelegramChannel`). Read this
  first if you're about to add a third.

## Phase status

| Phase    | Status  | Doc                        | Commit    |
|----------|---------|----------------------------|-----------|
| Phase 0  | Frozen  | [PHASE_0.md](PHASE_0.md)   | `1b4f271` |
| Phase 1  | Frozen  | [PHASE_1.md](PHASE_1.md)   | `33012be` |
| Phase 2  | Frozen  | [PHASE_2.md](PHASE_2.md)   | `2b6f876` |
| Phase 3  | Frozen  | [PHASE_3.md](PHASE_3.md)   | `fa0f4ea` |
| Phase 4  | Frozen  | [PHASE_4.md](PHASE_4.md)   | `999ce87` |
| Phase 5  | Frozen  | [PHASE_5.md](PHASE_5.md)   | `6dab2a7` |
| Phase 6  | Frozen  | [PHASE_6.md](PHASE_6.md)   | `912f022` |
| Phase 7  | Frozen  | [PHASE_7.md](PHASE_7.md)   | `8164317` |
| Phase 8  | Frozen  | [PHASE_8.md](PHASE_8.md)   | `0484606` |
| Phase 9  | Frozen  | [PHASE_9.md](PHASE_9.md)   | `7052ecc` |
| Phase 10 | Frozen  | [PHASE_10.md](PHASE_10.md) | `f8f4d28` |
| Phase 11 | Frozen  | [PHASE_11.md](PHASE_11.md) | `16422e2` |
| Phase 12 | Frozen  | [PHASE_12.md](PHASE_12.md) | `16e618c` |
| Phase 13 | Frozen  | [PHASE_13.md](PHASE_13.md) | `25a09de` |
| Phase 14 | Frozen  | [PHASE_14.md](PHASE_14.md) | `0d94d32` |
| Phase 15 | Frozen  | [PHASE_15.md](PHASE_15.md) | `06dfdfd` |
| Phase 16 | Frozen  | [PHASE_16.md](PHASE_16.md) | `1ed3f90` |
| Phase 17 | Frozen  | [PHASE_17.md](PHASE_17.md) | `277d910` |
| Phase 18 | Frozen  | [PHASE_18.md](PHASE_18.md) | `6dd4f23` |
| Phase 19 | Frozen  | [PHASE_19.md](PHASE_19.md) | `PENDING` |
| Phase 20+ | Planned | [ROADMAP.md](ROADMAP.md)   | —         |

Frozen means the phase doc is no longer edited except through commits
with a message starting `docs(phase-N):` — a convention, not an
enforcement, but it makes drift visible in `git log`.

**Planned phases** (6 and beyond) live in [`ROADMAP.md`](ROADMAP.md)
as one-paragraph intents rather than as separate PHASE_N.md files.
A phase gets its own doc only when it opens — that way there are no
stale task lists sitting in files for phases we haven't started yet.

## Amendment process (not yet used)

When a Phase N decision needs to override a Phase 0 contract:

1. Create `docs/amendments/<date>-<short-slug>.md` — one file per
   amendment, describing what changed, why, and which `DESIGN.md`
   section it supersedes.
2. Edit the relevant section of `DESIGN.md` to reference the amendment
   inline (e.g., *"See amendment `2026-05-12-tool-required-scope.md`"*).
3. Commit both files together with message
   `docs(amendment): <slug>`.

The directory does not yet exist. It will be created the first time
an amendment is needed.

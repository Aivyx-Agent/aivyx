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
| Phase 19 | Frozen  | [PHASE_19.md](PHASE_19.md) | `986c519` |
| Phase 20 | Frozen  | [PHASE_20.md](PHASE_20.md) | `8e77075` |
| Phase 21 | Frozen  | [PHASE_21.md](PHASE_21.md) | `05cc349` |
| Phase 22 | Frozen  | [PHASE_22.md](PHASE_22.md) | `549bc6e` |
| Phase 23 | Frozen  | [PHASE_23.md](PHASE_23.md) | `8f104e8` |
| Phase 24 | Frozen  | [PHASE_24.md](PHASE_24.md) | `84962d1` |
| Phase 25 | Frozen  | [PHASE_25.md](PHASE_25.md) | `5e4144f` |
| Phase 26 | Frozen  | [PHASE_26.md](PHASE_26.md) | `e550229` |
| Phase 27 | Frozen  | [PHASE_27.md](PHASE_27.md) | `426bcc2` |
| Phase 28 | Frozen  | [PHASE_28.md](PHASE_28.md) | `2cd41d9`  |
| Phase 29 | Frozen  | [PHASE_29.md](PHASE_29.md) | `03a804b`  |
| Phase 30 | Frozen  | [PHASE_30.md](PHASE_30.md) | `8b93180` |
| Phase 31 | Frozen  | [PHASE_31.md](PHASE_31.md) | `7f16f7a` |
| Phase 32 | Frozen  | [PHASE_32.md](PHASE_32.md) | `4aabea8` |
| Phase 33 | Frozen  | [PHASE_33.md](PHASE_33.md) | `d02dda6` |
| Phase 34 | Frozen  | [PHASE_34.md](PHASE_34.md) | `0004cf2` |
| Phase 35 | Frozen  | [PHASE_35.md](PHASE_35.md) | `5786449` |
| Phase 36 | Frozen  | [PHASE_36.md](PHASE_36.md) | `1f726f9`  |
| Phase 37 | Frozen  | [PHASE_37.md](PHASE_37.md) | `b2f3bda`  |
| Phase 38 | Frozen  | [PHASE_38.md](PHASE_38.md) | `b596207`  |
| Phase 39 | Frozen  | [PHASE_39.md](PHASE_39.md) | `6f7eaf1`  |
| Phase 40 | Frozen  | [PHASE_40.md](PHASE_40.md) | `d995afd`  |
| Phase 41 | Frozen  | [PHASE_41.md](PHASE_41.md) | `87d87c2`  |
| Phase 42 | Frozen  | [PHASE_42.md](PHASE_42.md) | `0478461`  |
| Phase 43 | Frozen  | [PHASE_43.md](PHASE_43.md) | `066e633`  |

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

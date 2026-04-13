# Aivyx Docs

This directory holds **phase records** and **amendments**. It is
deliberately separate from `DESIGN.md` in the repo root:

- **`DESIGN.md`** is the **contract document** — the locked decisions
  that every phase must respect. It is edited rarely, and only through
  the amendment process described below.
- **`docs/PHASE_N.md`** are **phase journals** — working documents for
  one phase at a time. They churn freely during the phase and are
  frozen at phase exit.
- **`docs/amendments/`** (created when first needed) holds one file per
  contract change. An amendment is the *only* legal way to modify a
  locked `DESIGN.md` decision from inside a later phase.

## Why the split?

The failure mode this layout defends against is **silent contract
drift** — a later phase quietly editing a Phase 0 decision because it
became inconvenient. With the contract in one file and the phase work
in another, any such edit shows up as a `DESIGN.md` diff in a phase
commit, which is an obvious review flag.

Rule of thumb: if you're writing something that future-you in a fresh
session needs to rely on, it goes in `DESIGN.md`. If it's a decision
log, task list, or "why we chose A over B today," it goes in the phase
doc.

## Phase status

| Phase    | Status  | Doc                        | Commit    |
|----------|---------|----------------------------|-----------|
| Phase 0  | Frozen  | [PHASE_0.md](PHASE_0.md)   | `1b4f271` |
| Phase 1  | Frozen  | [PHASE_1.md](PHASE_1.md)   | —         |
| Phase 2  | Active  | [PHASE_2.md](PHASE_2.md)   | —         |
| Phase 3+ | Planned | [ROADMAP.md](ROADMAP.md)   | —         |

Frozen means the phase doc is no longer edited except through commits
with a message starting `docs(phase-N):` — a convention, not an
enforcement, but it makes drift visible in `git log`.

**Planned phases** (2 and beyond) live in [`ROADMAP.md`](ROADMAP.md)
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

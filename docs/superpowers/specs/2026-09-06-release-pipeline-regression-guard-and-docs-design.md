# Chapter N, Phase 3 (global Phase 197): Regression Guard + Doc Corrections

**Status: approved, ready for implementation planning.**

## Context

Chapter N (Release & Distribution Integrity) opened 2026-09-05 after
discovering `v0.9.0` — the version `README.md`'s own "Status" line
named as current — was never actually published as a real GitHub
Release. Phase 192 fixed the root cause (three private git
dependencies) and built `scripts/check-git-deps-public.sh`, a
reusable anonymous-clone probe, but never wired it into CI. Phase 193
cut and fully verified a real release, `v0.9.4` — now genuinely
`Latest`, with every documented install path (release binaries, GHCR
image, WSL appliance, build-from-source) proven to work for a
credential-less outsider.

This phase closes Chapter N: wire the regression guard into CI so a
future recurrence of Phase 192's root cause fails in seconds with a
specific message, correct every stale pre-launch version reference
still sitting in `README.md`/`docs/INSTALL.md`, and backfill
`CHANGELOG.md` with the one real release (`v0.9.4`) that's currently
undocumented there.

## Scope

Three independent changes, one phase:

1. Wire `scripts/check-git-deps-public.sh` into
   `.github/workflows/quality-gate.yml`.
2. Correct five stale version-reference locations across `README.md`
   and `docs/INSTALL.md`.
3. Add a `[0.9.4]` entry to `CHANGELOG.md`.

None of these three depend on each other — they can be implemented
and reviewed as independent tasks.

## 1. CI regression guard

Add one new step to `.github/workflows/quality-gate.yml`'s single
`test-and-clippy` job, immediately after the `Checkout` step and
before `Free disk space on the runner`:

```yaml
      - name: Confirm git dependencies are anonymously cloneable
        run: ./scripts/check-git-deps-public.sh
```

**Why this placement:** `quality-gate.yml` is a reusable
(`workflow_call`) workflow with exactly one job. It's called by both
`.github/workflows/ci.yml` (every push to `main` + every PR) and,
indirectly, `.github/workflows/release.yml` (via `dist-workspace.toml`'s
`plan-jobs = ["./quality-gate"]` hook) — so this single line covers
both call sites. Placing it as the second step (right after checkout,
before the disk-cleanup/toolchain-install/cache-restore steps that
exist solely to support the expensive `cargo test`/`cargo clippy`
steps) means a regression fails in the time it takes to provision a
runner and run one `git ls-remote` per dependency (script already
proven at ~1-3s per URL against real GitHub state) — not after
~10-20 minutes of disk cleanup, toolchain setup, and dependency
compilation that was always going to fail anyway once `cargo` tries
to fetch the same unreachable git dependency.

**Why a step in the existing job, not a separate job:** A separate
preceding job (`needs:`) would fail marginally faster (no toolchain
install step to even reach) and show as its own named check, but adds
a second job definition, a `needs:` edge, and duplicate checkout for
one script that already runs in under 5 seconds total. Not worth the
structural overhead for this workflow's scale.

**Out of scope, deliberately:** the script's three known gaps from
Phase 192 (grep's `|| true` swallowing exit code 2 as well as exit
code 1; `https://`-only URL matching, missing `ssh://`/`git@` forms;
root-`Cargo.toml`-only scanning) are unchanged. None of them block
this phase's goal (catching the exact failure mode that broke
`v0.9.0`), and Phase 192 already logged them as follow-ups.

**Documentation:** extend `quality-gate.yml`'s existing top-of-file
comment block (which already explains the workflow's two callers and
the "defense in depth" rationale) with one line noting the new step
and pointing at `docs/archive/phases/PHASE_192.md` for why it exists.

## 2. Doc corrections

All five locations carry the same category of bug: version claims
that were accurate once (either at initial pre-launch drafting, or as
of `v0.9.0`) and were never updated as real releases shipped past
them. Two files, five locations:

### `README.md`

**Location A — line 22, the `## Status` header:**
```
## Status (v0.9.0 — source-available, BUSL-1.1, 2026-09-03)
```
→
```
## Status (v0.9.4 — source-available, BUSL-1.1, 2026-09-05)
```

**Location B — line 28, the status table's "Release pipeline" row.**
Current text describes `v0.9.0` as "the Interface Polish phase
capstone" with a list of what that release contained. Replace with a
description of what `v0.9.4` actually is: the release-pipeline
integrity fixes (git-dependency visibility, the `aivyx-confine`
Landlock/musl fixes) plus Chapter Picket's active prompt-injection
tripwire. Keep the same table-row format and the three links
(shell installer / desktop app / WSL distro) unchanged — those paths
themselves didn't change, only which release they currently point at.

**Location C — the "Release pipeline status" section (lines
228-260).** Update the opening paragraph's `v0.9.0` reference to
`v0.9.4` and drop the "Interface Polish phase capstone" parenthetical
(no longer the relevant framing for what's currently latest). The
workflow-by-workflow technical list below it (`release.yml`,
`desktop-release.yml`, `docker-publish.yml`, `wsl-release.yml`,
`ci.yml`, `quality-gate.yml`) is still accurate and needs no
structural changes — add one new bullet (or extend the existing
`quality-gate.yml` bullet) noting it now also confirms git
dependencies are anonymously cloneable before running tests/clippy,
referencing Phase 192/197.

### `docs/INSTALL.md`

**Location D — the "Current install state" section (lines 12-22).**
Current text frames `v0.1.0` as "the first public release
(pre-release)" with a conditional note that the installer URL is
"live only once the `v0.1.0` tag has been pushed." Both claims are
false today — several real releases have shipped since, the pipeline
is mature and active, and `v0.1.0` was never actually tagged (verified
directly: no local or remote `v0.1.0` tag exists — this was always
aspirational pre-launch copy, never corrected after the real first
release). Rewrite to state plainly: the recommended install path is
the shell installer (downloads a prebuilt binary for your platform),
build-from-source is the alternative, both install the same single
`aivyx` binary — and drop the pre-release/conditional-availability
framing entirely. Point to `CHANGELOG.md` for release history instead
of asserting a specific "current" version inline (this section
shouldn't need editing on every future release).

**Location E — lines 283-289, the "for a specific version" installer
example.** Current text hardcodes `v0.1.0` in a copy-pasteable
`curl` command, plus a worked example showing `aivyx --version #
aivyx 0.1.0`, plus a note that "the URL resolves only once `v0.1.0`
has been tagged." Replace the hardcoded tag with a generic `vX.Y.Z`
placeholder in both the command and an inline instruction ("replace
`vX.Y.Z` with a real tag from the Releases page") rather than the
current actual latest (`v0.9.4`) — hardcoding today's latest here
would silently go stale again on the very next release, which is
exactly the bug this phase exists to stop happening. Drop the
now-false "resolves only once tagged" conditional note entirely (the
pipeline is active; every tagged release already has this asset).

## 3. CHANGELOG.md backfill

Real tag/release history confirms only `v0.9.0` and `v0.9.4` were
ever actually published as GitHub Releases — `v0.9.1`/`v0.9.2`/`v0.9.3`
were intermediate local workspace-version bumps on the now-merged,
now-deleted Phase 193 worktree branch, chasing two real
`aivyx-confine` bugs found while trying to cut a working release;
`v0.9.3` was briefly pushed as a tag and later deleted (it pointed at
a pre-merge commit and never had a successful release attached),
`v0.9.1`/`v0.9.2` were never pushed at all, and all three dangling
local tags have since been deleted. So this backfill adds exactly one
new entry, `[0.9.4]`, between `[Unreleased]` and `[0.9.0]` — not one
entry per intermediate version number, since three of those four
numbers never existed as real releases.

Content (matching the existing milestone-paragraph + `### Added`/
`### Fixed` format used by the `[0.9.0]` entry):

```markdown
## [0.9.4] — 2026-09-05

**Milestone: the first real, working GitHub Release since `v0.8.3`.**
Closes Chapter N (Release & Distribution Integrity, opened this same
day after discovering `v0.9.0` never actually published — see
`docs/archive/phases/PHASE_192.md` and `PHASE_193.md`) and Chapter
Picket (the prompt-injection tripwire, `PHASE_194.md`-`PHASE_196.md`)
in one release.

### Added

- **Active prompt-injection tripwire (Chapter Picket).**
  `aivyx-injection-guard`, a new shared crate extracted from
  `aivyx-coder`'s existing production tripwire, scans untrusted tool
  output for known injection phrasings and escalates the turn via the
  existing `TurnOutcome::Escalated` → `ApprovalGate`/`HeadlessRefusal`
  path — a side-channel signal checked after the tool's real outcome
  is recorded, so a mutating tool's audit trail always reflects what
  actually executed. Complements, doesn't replace, Bulwark's existing
  passive `fence_untrusted_output` labeling. See `docs/THREAT_MODEL.md`
  and `PHASE_196.md`.

### Fixed

- **`aivyx-confine`'s `require_enforcement` check incorrectly required
  `RulesetStatus::FullyEnforced`.** GitHub's own hosted-runner kernel
  only ever achieves `PartiallyEnforced` at the crate's hardcoded
  Landlock ABI version, which made every release build fail its own
  quality gate before reaching the real cross-compile work. Now only
  `NotEnforced` fails closed.
- **`aivyx-confine`'s syscall blocklist referenced
  `libc::SYS_kexec_file_load`, absent from libc's musl bindings for
  aarch64/riscv64,** breaking the `aarch64-unknown-linux-musl` release
  target's build.
- **The WSL release workflow's retry budget for pulling the appliance
  base image (10 minutes) was too short for real-world timing;**
  raised to 30 minutes to match the sibling wait-for-release retry
  loop in the same workflow.
- **The three private git dependencies (`aivyx-confine`,
  `aivyx-checkpoint`, `aivyx-kvcache`) that silently broke every
  `v0.9.0` release workflow — and any outside contributor's
  build-from-source path — are now public,** with a new CI regression
  guard (`scripts/check-git-deps-public.sh`, wired into
  `quality-gate.yml`) that fails fast and by name if this ever
  recurs.
```

## Self-review

- **Placeholder scan:** none — every location, every replacement
  string, and the full CHANGELOG entry text are given verbatim above.
- **Internal consistency:** Section 1's regression guard and
  Section 3's CHANGELOG entry both reference the same fix
  (git-dependency visibility); Section 2 Location C's new bullet
  points at the same guard Section 1 adds — all three sections agree
  on what shipped and when.
- **Scope check:** three independent, narrowly-bounded changes to
  three files (`quality-gate.yml`, `README.md` + `docs/INSTALL.md`,
  `CHANGELOG.md`) — small enough for one implementation plan, no
  decomposition needed.
- **Ambiguity check:** Location D deliberately avoids asserting a
  specific "current" version inline (pointing to `CHANGELOG.md`
  instead) specifically so it doesn't need editing on the next
  release; Location E's placeholder choice (`vX.Y.Z`, not the current
  `v0.9.4`) is the same anti-staleness decision applied consistently.

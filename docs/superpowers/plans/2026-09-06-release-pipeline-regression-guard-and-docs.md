# Chapter N, Phase 3 (global Phase 197) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close Chapter N (Release & Distribution Integrity) by wiring
the existing git-dependency regression guard into CI, correcting every
stale pre-launch version reference in `README.md`/`docs/INSTALL.md`,
and backfilling `CHANGELOG.md` with the one real release (`v0.9.4`)
currently missing from it.

**Architecture:** Three independent, single-file-family changes — a
one-line CI workflow addition, a doc-text correction pass across two
files, and one new CHANGELOG entry. No code, no new dependencies, no
shared interfaces between tasks. A fourth task closes out the phase
and the chapter with a retrospective and roadmap updates.

**Tech Stack:** GitHub Actions YAML, Markdown. No Rust changes.

## Global Constraints

- Full design: `docs/superpowers/specs/2026-09-06-release-pipeline-regression-guard-and-docs-design.md`.
  Parent chapter design: `docs/superpowers/specs/2026-09-05-release-distribution-integrity-design.md`.
- `scripts/check-git-deps-public.sh` itself must NOT be modified in
  this phase. Its three known gaps (grep's `|| true` swallowing exit
  code 2 as well as exit code 1; `https://`-only URL matching; scans
  only the root `Cargo.toml`) stay logged as follow-ups, unchanged
  from Phase 192's disposition.
- The new CHANGELOG entry must match the exact format of the existing
  `[0.9.0]` entry: `## [X.Y.Z] — DATE` header, a milestone paragraph,
  then `### Added` / `### Fixed` subsections with bullet points.
- `docs/INSTALL.md`'s installer-example fix (Task 2, Location E) must
  use a generic `vX.Y.Z` placeholder — NOT the current actual latest
  version (`v0.9.4`). This is a deliberate anti-staleness decision:
  hardcoding today's latest here would silently go stale again on the
  very next release, which is the exact bug this phase exists to stop.
- Real tag/release history: only `v0.9.0` and `v0.9.4` were ever
  published as real GitHub Releases. `v0.9.1`/`v0.9.2`/`v0.9.3` never
  shipped as their own releases (confirmed and their dangling local
  tags already deleted) — do not add CHANGELOG entries for them.

---

## Task 1: Wire the git-dependency regression guard into CI

**Files:**
- Modify: `.github/workflows/quality-gate.yml`

**Interfaces:**
- Consumes: `scripts/check-git-deps-public.sh` (already exists,
  already executable, already proven working against real GitHub
  state in Phase 192 — do not modify it). Exit code `0` on success,
  non-zero (naming every offending URL on stderr/stdout) on failure.
- Produces: nothing consumed by later tasks in this plan.

- [ ] **Step 1: Add the new step to the workflow**

Open `.github/workflows/quality-gate.yml`. Find this exact block
(currently lines 29-38):

```yaml
      - name: Checkout
        uses: actions/checkout@v4
        with:
          persist-credentials: false
      # `cargo test --workspace` builds every crate + every test/example binary
      # for 40 crates; combined with the restored dep cache it overflowed the
      # hosted runner's ~14 GB root disk ("No space left on device" mid-test,
      # which also manifested as truncated/again-missing logs). Reclaim ~20 GB of
      # preinstalled toolchains we never use before building. Dependency-free.
      - name: Free disk space on the runner
```

Replace it with (inserting the new step between `Checkout` and `Free
disk space on the runner`):

```yaml
      - name: Checkout
        uses: actions/checkout@v4
        with:
          persist-credentials: false
      # Fails fast (a few seconds) and by name if a workspace git
      # dependency has silently become private — the exact failure
      # mode that broke every v0.9.0 release workflow (see
      # docs/archive/phases/PHASE_192.md). Deliberately placed before
      # the disk-cleanup/toolchain/cache steps below: those exist only
      # to support cargo test/clippy, which would fail anyway (just
      # 10-20 minutes later, via an opaque auth backtrace) if this
      # check would have failed.
      - name: Confirm git dependencies are anonymously cloneable
        run: ./scripts/check-git-deps-public.sh
      # `cargo test --workspace` builds every crate + every test/example binary
      # for 40 crates; combined with the restored dep cache it overflowed the
      # hosted runner's ~14 GB root disk ("No space left on device" mid-test,
      # which also manifested as truncated/again-missing logs). Reclaim ~20 GB of
      # preinstalled toolchains we never use before building. Dependency-free.
      - name: Free disk space on the runner
```

- [ ] **Step 2: Extend the file's top-of-file comment block**

Find this exact text at the top of the same file (lines 1-16):

```yaml
# Phase 61 Task 4 — Quality Gate (reusable workflow).
#
# Mirrors the local pre-commit hook discipline (the workspace has
# held at zero clippy warnings since the Phase 9 hook). Runs
# `cargo clippy --workspace --all-targets -- -D warnings` and
# `cargo test --workspace`. Consumed by two callers:
#
#   - `.github/workflows/ci.yml` — every push to main + every PR.
#   - `.github/workflows/release.yml` — wired in via the
#     `plan-jobs = ["./quality-gate"]` hook in
#     `dist-workspace.toml`. dist makes the plan job depend on
#     this workflow, which means NO release artifact gets built
#     unless `cargo test --workspace` and clippy both pass.
#
# Defense in depth: ci.yml catches failures at commit time;
# release.yml's wiring catches them at tag time even if the
# operator skipped the PR-time check.
```

Replace it with:

```yaml
# Phase 61 Task 4 — Quality Gate (reusable workflow).
#
# Mirrors the local pre-commit hook discipline (the workspace has
# held at zero clippy warnings since the Phase 9 hook). Runs
# `cargo clippy --workspace --all-targets -- -D warnings` and
# `cargo test --workspace`. Consumed by two callers:
#
#   - `.github/workflows/ci.yml` — every push to main + every PR.
#   - `.github/workflows/release.yml` — wired in via the
#     `plan-jobs = ["./quality-gate"]` hook in
#     `dist-workspace.toml`. dist makes the plan job depend on
#     this workflow, which means NO release artifact gets built
#     unless `cargo test --workspace` and clippy both pass.
#
# Defense in depth: ci.yml catches failures at commit time;
# release.yml's wiring catches them at tag time even if the
# operator skipped the PR-time check.
#
# Phase 197 — also runs scripts/check-git-deps-public.sh before
# anything else: a private workspace git dependency silently broke
# every v0.9.0 release workflow (Phase 192) and would otherwise only
# surface ~10-20 minutes later, as an opaque cargo auth failure. See
# docs/archive/phases/PHASE_192.md and PHASE_197.md.
```

- [ ] **Step 3: Validate the YAML is well-formed**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/quality-gate.yml'))" && echo "valid YAML"`

Expected: `valid YAML` (no exception).

- [ ] **Step 4: Prove the new step actually catches the failure it exists for**

This is a local, throwaway test of the script's real behavior — the
same probe the workflow step now runs, pointed at a URL that must
fail so you can confirm the failure path is real, then reverted.

Run:
```bash
git -C /tmp rm -rf check-guard-test 2>/dev/null; true
mkdir -p /tmp/check-guard-test
cp Cargo.toml /tmp/check-guard-test/Cargo.toml.bak
# Temporarily point one real dependency URL at a private-repo-shaped
# placeholder that cannot be anonymously cloned, to prove the script
# (unmodified) still catches this class of failure today.
sed -i.orig 's#https://github.com/Aivyx-Agent/aivyx-confine#https://github.com/Aivyx-Agent/this-repo-does-not-exist-guard-test#' Cargo.toml
./scripts/check-git-deps-public.sh; echo "exit code: $?"
mv Cargo.toml.orig Cargo.toml
rm -rf /tmp/check-guard-test
```

Expected: the run before restoring `Cargo.toml` prints a `FAIL: could
not be cloned without credentials` line naming the placeholder URL and
exits non-zero (`exit code: 1`). After `mv Cargo.toml.orig Cargo.toml`,
confirm `git diff Cargo.toml` shows no changes (the file is back to
its committed state).

- [ ] **Step 5: Confirm the real (unmodified) dependencies still pass**

Run: `./scripts/check-git-deps-public.sh; echo "exit code: $?"`

Expected: every real dependency (`aivyx-confine`, `aivyx-checkpoint`,
`aivyx-kvcache`, `aivyx-injection-guard`) reports `OK: anonymously
cloneable`, ending in `All git dependencies are anonymously
cloneable.` and `exit code: 0`.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/quality-gate.yml
git commit -m "ci: fail fast on a private workspace git dependency

Wires scripts/check-git-deps-public.sh (Phase 192) into quality-gate.yml
as the second step, right after checkout — the exact failure mode that
broke every v0.9.0 release workflow now surfaces in seconds with a
named URL, not ~10-20 minutes later via an opaque cargo auth backtrace.
quality-gate.yml is the shared reusable workflow both ci.yml and (via
dist-workspace.toml's plan-jobs) release.yml call, so this one line
covers both."
```

---

## Task 2: Correct five stale version references in README.md and docs/INSTALL.md

**Files:**
- Modify: `README.md`
- Modify: `docs/INSTALL.md`

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: nothing consumed by later tasks (Task 3's CHANGELOG entry
  is independently authored, not derived from these edits).

- [ ] **Step 1: Fix Location A — README.md's Status header**

Find this exact line in `README.md`:

```markdown
## Status (v0.9.0 — source-available, BUSL-1.1, 2026-09-03)
```

Replace with:

```markdown
## Status (v0.9.4 — source-available, BUSL-1.1, 2026-09-05)
```

- [ ] **Step 2: Fix Location B — README.md's status table "Release pipeline" row**

Find this exact table row in `README.md` (it is one long line — match
it verbatim):

```markdown
| Release pipeline | **Active** — on each version tag, cargo-dist builds the CLI (Linux x86_64/aarch64 musl + macOS x86_64/aarch64) and a separate workflow builds the **desktop app** (`.deb` + macOS `.app`); both attach to the GitHub Release. Latest is **`v0.9.0`** (the Interface Polish phase capstone — Gatehouse, Freight, the Vitrine walkthrough, the resulting 8-sub-project polish backlog, Chapter Mission Control, and Phase 185's Terminal TUI foundation) via the [shell installer](docs/INSTALL.md#shell-installer-recommended), the [desktop app](docs/INSTALL.md#desktop-app), or the [WSL distro](docs/INSTALL.md#windows-wsl2-or-docker) |
```

Replace with:

```markdown
| Release pipeline | **Active** — on each version tag, cargo-dist builds the CLI (Linux x86_64/aarch64 musl + macOS x86_64/aarch64) and a separate workflow builds the **desktop app** (`.deb` + macOS `.app`); both attach to the GitHub Release. Latest is **`v0.9.4`** (Chapter N's release-pipeline integrity fixes — the git-dependency visibility fix and the `aivyx-confine` Landlock/musl fixes that had been silently breaking releases — plus Chapter Picket's active prompt-injection tripwire) via the [shell installer](docs/INSTALL.md#shell-installer-recommended), the [desktop app](docs/INSTALL.md#desktop-app), or the [WSL distro](docs/INSTALL.md#windows-wsl2-or-docker) |
```

- [ ] **Step 3: Fix Location C — README.md's "Release pipeline status" section**

Find this exact block in `README.md`:

```markdown
## Release pipeline status

The release pipeline is **active** on the public repo. The latest
release is `v0.9.0` — the Interface Polish phase capstone (Gatehouse,
Freight, the Vitrine walkthrough, the 8-sub-project polish backlog,
Chapter Mission Control, Phase 185's Terminal TUI foundation; see the
CHANGELOG):

- `.github/workflows/release.yml` (cargo-dist-generated) cross-compiles
  the CLI for x86_64/aarch64 Linux musl + x86_64/aarch64 macOS on every
  `v*.*.*` tag push, then publishes a GitHub Release with the
  binaries, checksums, and the one-line shell installer.
- `.github/workflows/desktop-release.yml` builds the **desktop app**
  bundles (a `.deb` on Linux, a `.app` on macOS) and uploads them to that
  same release — it waits for cargo-dist to create the release first, so
  the two never race on creation.
- `.github/workflows/docker-publish.yml` builds + pushes the **server
  appliance image** to GHCR on the same tag.
- `.github/workflows/wsl-release.yml` reuses that appliance image to export
  a **WSL distribution** (`Aivyx.wsl`) — the daemon pre-installed for
  Windows/WSL2 users — and attaches it to the release. This is the
  cheapest real "Aivyx on Windows" path: it sidesteps the deferred native
  Windows port (the daemon's Unix-socket IPC just works inside WSL2's Linux
  kernel). See [docs/INSTALL.md](docs/INSTALL.md#windows-wsl2-or-docker).
- `.github/workflows/ci.yml` runs `cargo clippy --workspace --all-targets
  -- -D warnings` and `cargo test --workspace` on every push to
  main and every PR.
- `.github/workflows/quality-gate.yml` is the shared reusable
  workflow both CI and release pipelines call — the release
  short-circuits if the gate fails.

Cutting a release is a single step: `git tag vX.Y.Z && git push
origin vX.Y.Z`, and the workflow publishes the binaries + installer.
```

Replace with:

```markdown
## Release pipeline status

The release pipeline is **active** on the public repo. The latest
release is `v0.9.4` (see the CHANGELOG for what shipped):

- `.github/workflows/release.yml` (cargo-dist-generated) cross-compiles
  the CLI for x86_64/aarch64 Linux musl + x86_64/aarch64 macOS on every
  `v*.*.*` tag push, then publishes a GitHub Release with the
  binaries, checksums, and the one-line shell installer.
- `.github/workflows/desktop-release.yml` builds the **desktop app**
  bundles (a `.deb` on Linux, a `.app` on macOS) and uploads them to that
  same release — it waits for cargo-dist to create the release first, so
  the two never race on creation.
- `.github/workflows/docker-publish.yml` builds + pushes the **server
  appliance image** to GHCR on the same tag.
- `.github/workflows/wsl-release.yml` reuses that appliance image to export
  a **WSL distribution** (`Aivyx.wsl`) — the daemon pre-installed for
  Windows/WSL2 users — and attaches it to the release. This is the
  cheapest real "Aivyx on Windows" path: it sidesteps the deferred native
  Windows port (the daemon's Unix-socket IPC just works inside WSL2's Linux
  kernel). See [docs/INSTALL.md](docs/INSTALL.md#windows-wsl2-or-docker).
- `.github/workflows/ci.yml` runs `cargo clippy --workspace --all-targets
  -- -D warnings` and `cargo test --workspace` on every push to
  main and every PR.
- `.github/workflows/quality-gate.yml` is the shared reusable
  workflow both CI and release pipelines call — the release
  short-circuits if the gate fails, and it also confirms every
  workspace git dependency is anonymously cloneable before running
  tests/clippy (the exact check that would have caught the bug behind
  `v0.9.0`'s failed release; see `docs/archive/phases/PHASE_192.md`).

Cutting a release is a single step: `git tag vX.Y.Z && git push
origin vX.Y.Z`, and the workflow publishes the binaries + installer.
```

- [ ] **Step 4: Fix Location D — docs/INSTALL.md's "Current install state" section**

Find this exact block in `docs/INSTALL.md`:

```markdown
## Current install state

The first public release is **`v0.1.0` (pre-release)**. The
recommended install path is the [shell installer](#shell-installer-recommended),
which downloads a prebuilt binary for your platform; you can also
[build from source](#build-from-source). Both install the same
single `aivyx` binary.

> `v0.1.0` is an early pre-release — see the [CHANGELOG](../CHANGELOG.md).
> The shell-installer URL is live only once the `v0.1.0` tag has been
> pushed and GitHub Actions has finished building the release.
```

Replace with:

```markdown
## Current install state

The recommended install path is the [shell installer](#shell-installer-recommended),
which downloads a prebuilt binary for your platform; you can also
[build from source](#build-from-source). Both install the same
single `aivyx` binary. The release pipeline is active — see the
[CHANGELOG](../CHANGELOG.md) for release history.
```

- [ ] **Step 5: Fix Location E — docs/INSTALL.md's "for a specific version" installer example**

Find this exact block in `docs/INSTALL.md`:

```markdown
For a specific version, replace `latest` with the tag:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/Aivyx-Agent/aivyx/releases/download/v0.1.0/aivyx-cli-installer.sh \
  | sh
```

> The URL resolves only once `v0.1.0` has been tagged and the
> GitHub Actions release build has completed. Until then, use
> [build from source](#build-from-source).
```

Replace with:

```markdown
For a specific version, replace `latest` with the tag (see the
[Releases page](https://github.com/Aivyx-Agent/aivyx/releases) for
available tags):

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/Aivyx-Agent/aivyx/releases/download/vX.Y.Z/aivyx-cli-installer.sh \
  | sh
```
```

(Note: this removes the trailing "URL resolves only once..." blockquote
entirely — the pipeline is active and every tagged release already has
this asset, so the conditional no longer applies.)

- [ ] **Step 6: Confirm no other stale version references were missed**

Run: `grep -n "v0\.1\.0\|v0\.9\.0" README.md docs/INSTALL.md`

Expected: no output (empty) — Steps 1-5 above are the exhaustive list
of locations found during this phase's design. If this command prints
any match, stop and report it before continuing — it means a sixth
location exists that the plan didn't account for.

- [ ] **Step 7: Commit**

```bash
git add README.md docs/INSTALL.md
git commit -m "docs: correct five stale pre-launch version references

README.md's Status header, its status-table release-pipeline row, and
its 'Release pipeline status' section all still named v0.9.0 as
latest; docs/INSTALL.md's 'Current install state' section and its
'for a specific version' installer example both still referenced
v0.1.0, which was never actually tagged (verified: no local or remote
v0.1.0 tag exists — this was aspirational pre-launch copy, never
corrected after the real first release).

The installer example now uses a generic vX.Y.Z placeholder instead
of hardcoding the current latest (v0.9.4) — hardcoding today's latest
would silently go stale again on the next release, which is the exact
bug this phase exists to stop."
```

---

## Task 3: Backfill CHANGELOG.md with the v0.9.4 entry

**Files:**
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: nothing from Tasks 1-2.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Insert the new entry**

Find this exact block at the top of `CHANGELOG.md`:

```markdown
## [Unreleased]

## [0.9.0] — 2026-09-03
```

Replace with:

```markdown
## [Unreleased]

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

## [0.9.0] — 2026-09-03
```

- [ ] **Step 2: Confirm the file is still well-formed and only one entry was added**

Run: `grep -n "^## \[" CHANGELOG.md | head -5`

Expected:
```
6:## [Unreleased]
8:## [0.9.4] — 2026-09-05
XX:## [0.9.0] — 2026-09-03
```
(line numbers for `[0.9.0]` and everything below will have shifted
down by the length of the new entry — that's expected; just confirm
there is exactly one new `## [` header between `[Unreleased]` and
`[0.9.0]`, and that no other version header was duplicated or
removed.)

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: add the missing v0.9.4 CHANGELOG entry

Only v0.9.0 and v0.9.4 were ever actually published as real GitHub
Releases (v0.9.1/v0.9.2/v0.9.3 were intermediate local workspace-version
bumps on the now-merged, now-deleted Phase 193 worktree branch that
never shipped as their own releases), so this adds exactly one new
entry covering everything that landed in v0.9.4: Chapter Picket's
injection-guard tripwire, plus the aivyx-confine/WSL/git-dependency
fixes from Chapter N."
```

---

## Task 4: Close out the phase and Chapter N

**Files:**
- Create: `docs/archive/phases/PHASE_197.md`
- Modify: `docs/ROADMAP.md`
- Modify: `/home/julian/Projects/Rust/aivyx-ecosystem/ROADMAP.md`

**Interfaces:**
- Consumes: the real outcomes of Tasks 1-3 (commit hashes, confirmed
  test/verification results) to write an accurate retrospective —
  do not write this task's content until Tasks 1-3 are complete and
  their steps' expected outputs have been confirmed.
- Produces: nothing (terminal task).

- [ ] **Step 1: Write the retrospective**

Create `docs/archive/phases/PHASE_197.md` following the structure of
`docs/archive/phases/PHASE_192.md` and `PHASE_193.md` (both already
exist in this repo — read them first for the exact heading style,
tone, and level of detail expected: a `# Phase NNN — <title>` heading,
a `**Chapter N, phase 3 — [SHIPPED] <date>. Chapter N is now
COMPLETE.**` status line, then `## Goal (carried from the chapter
design)`, `## What shipped`, `## The result`, and `## Known
follow-ups` sections). Content for `## What shipped` must cover, with
real commit hashes filled in from `git log --oneline -10`:

- The new `check-git-deps-public.sh` CI step in `quality-gate.yml`,
  what it catches, and why it's placed where it is (Task 1).
- The five corrected doc locations across `README.md` and
  `docs/INSTALL.md`, and the real finding that `v0.1.0` was never
  actually tagged (Task 2).
- The `CHANGELOG.md` backfill, and the real finding that only `v0.9.0`
  and `v0.9.4` were ever actually released — `v0.9.1`/`v0.9.2`/`v0.9.3`
  never shipped as their own releases (Task 3).

`## Known follow-ups` must carry forward, unchanged, the three
`check-git-deps-public.sh` gaps already logged in `PHASE_192.md`
(grep's `|| true` swallowing exit code 2; `https://`-only URL
matching; root-`Cargo.toml`-only scanning) — this phase didn't touch
the script, so these remain open. Also carry forward Phase 193's two
open questions (why `aivyx` reverted to private; thin audit-log
access at this account tier).

- [ ] **Step 2: Update docs/ROADMAP.md**

Open `docs/ROADMAP.md` and find the Chapter N section (search for
`## Chapter N`). Update the phase list entry that currently reads
(from Phase 193's close-out edit):

```markdown
- **Phase 194** (Chapter N's own numbering — since reassigned; see
  below) **— Regression guard + doc correction.** Not yet started.
  Wire `check-git-deps-public.sh` into `quality-gate.yml` so a future
  regression fails in seconds with a specific message instead of ~20
  minutes in via an opaque `cargo clippy` backtrace. Correct
  `README.md`'s "Release pipeline status" section (currently overclaims
  `v0.9.0` as active/latest) and `docs/INSTALL.md`'s "Current install
  state" section (separately stale in the *opposite* direction — still
  says pre-`v0.1.0`).
```

Replace with:

```markdown
- **Phase 197 — Regression guard + doc correction.** Shipped
  2026-09-06 — see [PHASE_197.md](archive/phases/PHASE_197.md). Wired
  `check-git-deps-public.sh` into `quality-gate.yml`; corrected five
  stale pre-launch version references across `README.md` and
  `docs/INSTALL.md` (not just the two originally scoped — a broader
  sweep found three more of the same category, including a literal
  installer command pointing at a `v0.1.0` tag that was never actually
  created); backfilled `CHANGELOG.md` with the one real release
  (`v0.9.4`) it was missing. **Chapter N is now complete.**
```

Also find the paragraph immediately below the phase list (the "Known
follow-up, not yet scheduled to a phase" line about auditing
`aivyx-confine`/`aivyx-checkpoint`/`aivyx-kvcache` git history) and
leave it as-is — that follow-up is still open and unrelated to this
phase's scope.

- [ ] **Step 3: Update aivyx-ecosystem/ROADMAP.md**

Open `/home/julian/Projects/Rust/aivyx-ecosystem/ROADMAP.md` and find
the same Chapter N narrative (search for `Phase 192`). Add one new
paragraph immediately after the existing "Corrected 2026-09-05 — Phase
193 shipped" paragraph, stating Chapter N is now complete: what Phase
197 shipped (one sentence each for the three tasks), and a pointer to
`aivyx/docs/archive/phases/PHASE_197.md` for the full account. Match
the prose style already used in the surrounding paragraphs in that
file (dense, single-paragraph-per-phase, real specifics not vague
summaries).

- [ ] **Step 4: Commit both roadmap updates and the retrospective together in the aivyx repo**

```bash
cd /home/julian/Projects/Rust/aivyx
git add docs/archive/phases/PHASE_197.md docs/ROADMAP.md
git commit -m "docs: close out Phase 197 — Chapter N is now complete

Phase 197 wired the git-deps regression guard into CI, corrected five
stale pre-launch version references (README.md + docs/INSTALL.md),
and backfilled the missing v0.9.4 CHANGELOG entry. This was Chapter
N's last planned phase."
```

- [ ] **Step 5: Commit the ecosystem roadmap update in its own repo**

```bash
cd /home/julian/Projects/Rust/aivyx-ecosystem
git add ROADMAP.md
git commit -m "docs: record Phase 197 — Chapter N (Release & Distribution Integrity) is complete

See aivyx/docs/archive/phases/PHASE_197.md for the full account."
```

## Self-review notes (for whoever executes this plan)

- **Spec coverage:** Task 1 implements spec Section 1 verbatim (same
  YAML, same placement rationale). Task 2 implements spec Section 2's
  five locations verbatim (same before/after text, same `vX.Y.Z`
  placeholder decision for Location E). Task 3 implements spec Section
  3 verbatim (same single-entry decision, same CHANGELOG content).
  Task 4 covers the spec's implicit "this closes Chapter N" framing,
  which the spec's Context section states but doesn't assign to a
  numbered section — added as its own task since it has a real,
  independently-reviewable deliverable (the retrospective + two
  roadmap files) distinct from Tasks 1-3's content changes.
- **No placeholders:** every task's before/after text, every commit
  message, and the full CHANGELOG entry are given verbatim — copied
  directly from the spec (which itself was written from real file
  reads, not reconstructed from memory).
- **Type/interface consistency:** not applicable — no code, no shared
  types between tasks. Confirmed each task's "Consumes" is genuinely
  empty (all three of Tasks 1-3 are independent) except Task 4, which
  explicitly depends on Tasks 1-3's real outcomes for its retrospective
  content.

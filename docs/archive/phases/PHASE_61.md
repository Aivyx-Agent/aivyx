# Phase 61 — Release Pipeline & Prebuilt Binaries

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../../../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

**Original goal** (at phase open): Cut the first published release
(`v0.1.0`) on GitHub Releases and replace the `cargo run --release
--bin aivyx` quickstart with a one-line `curl … | sh && aivyx
init` installer path. First phase of the **Distribution
Milestone** — operator-feedback-shaped work on the post-Phase-60
substrate-ergonomics axis.

**Revised outcome** (at phase close, 2026-05-13): Phase 61 ships
the release-pipeline **substrate** — `--version` flag, cargo-dist
config, CI gates, release workflow, install matrix docs — but
does **not** publish a release. Mid-phase the operator pivoted
to a "VPS-private-first, GitHub-later" posture, holding the
public publication step (Task 7) indefinitely. Publication
re-opens as a focused micro-phase whenever public hosting goes
live.

Target matrix when the pipeline does fire:

- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`

Native Windows, Homebrew, crates.io publishing, Docker images,
and macOS code-signing/notarization remain explicitly **deferred
to follow-up phases** per the Phase 61 scope sign-off (Windows is
a real engineering project — daemon IPC is Unix-domain-socket-only
today). Windows-via-WSL is documented but not packaged.

The fixup commit reverting the premature "v0.1.0 published"
README/INSTALL.md claims (commit `7ac8cd8`) sits between Task 6
and Task 8 and is recorded as a phase commit, not a `chore:`
chore, because it materially changes the operator-facing framing
of the phase outcome.

## Why now

1. **Phase 60 closed the forward-commitment ledger.** P1–P14 are
   all Fully Delivered. The next pressure axis is end-user
   reach, not substrate.
2. **The codebase review surfaced distribution as the largest
   adoption-shape gap.** Today's quickstart requires a Rust
   toolchain (`cargo run --release --bin aivyx`); no prebuilt
   binaries exist on any platform. This locks out non-developers
   and front-loads ~10 minutes of toolchain install before the
   operator sees the wizard.
3. **The cross-compile surface is clean.** Reqwest uses
   `rustls-tls` (no system openssl), and `libc` is the only
   native dep. cargo-dist's standard four-target matrix should
   build without per-platform special-casing.
4. **No contract surface is touched.** Distribution is pure
   substrate ergonomics: zero changes to DESIGN.md, PRODUCT.md,
   or `aivyx-core`. All three byte-identity streaks should
   extend.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 61 ships CI infrastructure
  (`.github/workflows/release.yml`, `dist-workspace.toml`), one
  binary-file edit (`--version` flag), and docs. Zero contract
  touch. Prediction: streak **extends to eight** consecutive
  phases (currently at 7 after Phase 60).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** No commitment-text edits, no
  Delivery Status refresh (P1–P14 stay Fully Delivered). The
  forthcoming "Distribution" entry lands in
  `docs/PRODUCT_ROADMAP.md`, not in the contract document.
  Prediction: streak **recovers to one** (Phase 60 broke at
  Task 7).
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  Task 2 (`--version` flag) lands in
  `crates/aivyx-channel/src/bin/aivyx.rs`. Every other task
  lands in CI yaml, docs, `Cargo.toml`, or new files. No path
  touches `aivyx-core`. Prediction: streak **extends to nine**
  consecutive phases (new record, beating Phase 60's 8).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

## Tasks

### Task 1 — Open commit + PHASE_61.md scaffold

This file. Update `docs/README.md` to show Phase 61 as Open.
Commit Q-block resolutions before proceeding to Task 2.

### Task 2 — `aivyx --version` CLI flag

Add a top-level `--version` / `-V` flag to the hand-rolled CLI
parser in `crates/aivyx-channel/src/bin/aivyx.rs`. Prints
`aivyx <CARGO_PKG_VERSION>` and exits 0. Standard hygiene for
binaries shipped via package managers; cargo-dist's installer-
test smoke step expects it. One parser test (positive case).

### Task 3 — Release pipeline initialization

Generate the GitHub Actions release workflow at
`.github/workflows/release.yml` plus the cargo-dist
`dist-workspace.toml`. See **Q1** for cargo-dist vs hand-written.
Targets: the four listed in Goal. Linux targets use
musl-static per **Q2**. Triggered on git tags matching
`v[0-9]+.[0-9]+.[0-9]+`.

### Task 4 — CI gates

The release workflow must run `cargo test --workspace` and
`cargo clippy --workspace --all-targets -- -D warnings` BEFORE
uploading any artifact. Mirrors the local pre-commit hook
discipline. A failing test or a clippy warning prevents a bad
release from shipping.

### Task 5 — README rewrite

- Replace the "Five-minute setup" `cargo run --release --bin
  aivyx` block with the install-script path (`curl … | sh &&
  aivyx init`).
- Refresh stale `Status (Phase 54 exit, 2026-05-12)` table to
  current Phase 60 numbers (1071 Rust tests, 12 crates, 43
  scope bases, 10 storage domains, 10 amendments).
- Add macOS Gatekeeper workaround inline (xattr command or
  right-click-open).
- Source-build path stays as a documented fallback for
  contributors; not the primary.

### Task 6 — `docs/INSTALL.md`

New file. Full install matrix in one place:

- Shell installer (Linux + macOS, recommended).
- Build from source (Rust toolchain fallback path).
- Where the binary lands (per **Q4**).
- macOS quarantine workaround (full version).
- First-run checklist (`aivyx init`, then `aivyx`).
- Windows-via-WSL note; native Windows deferred.

### Task 7 — Cut `v0.1.0` + push (operator-side)

**Operator-side action, not engineering.** Listed for
completeness; Phase 61's engineering tasks complete at Task 8
exit.

1. Create the GitHub repository (no `git remote` is configured
   today).
2. `git remote add origin <repo-url>`
3. `git push -u origin main`
4. `git tag v0.1.0` (annotated, with a one-line message)
5. `git push --tags`

The release workflow fires on the tag push and publishes the
artifacts. Phase 61's exit criteria includes "v0.1.0 release
artifacts visible on GitHub Releases" — but the tag and push
are operator decisions, not Claude's.

### Task 8 — Exit commit

- ROADMAP.md gets the Phase 61 frozen entry.
- PRODUCT_ROADMAP.md gets a new "Distribution" milestone entry
  with Phase 61 listed as phase 1 of N.
- `docs/README.md` status table flips to Frozen with exit
  commit hash (back-filled in a final `docs(phase-61):
  backfill` commit per project convention).
- Prediction-vs-reality block filled.
- Exit-criteria block completed.

## Open questions

**Q1 — cargo-dist vs hand-rolled release CI?**

  - **(a)** Install `cargo-dist`, run `cargo dist init`, accept
    the generated workflow + `dist-workspace.toml`. Standard,
    well-tested, includes installer-script generation,
    release-notes generation, and is future-proof for Windows
    and signing additions in follow-up phases. cargo-dist
    itself lives in `~/.cargo/bin/` (developer-local, not a
    project-level dep).
  - **(b)** Hand-write `.github/workflows/release.yml`. Minimal
    dep surface (no cargo-dist install). More yaml to maintain
    long-term; no auto-generated installer script (would need a
    hand-written one).

  **Recommendation: (a).** cargo-dist is the de-facto standard
  for Rust release tooling and absorbs a lot of boilerplate. The
  installer-script generation alone is worth the dep — it
  handles arch detection, checksum verification, and the "where
  to put the binary" question. Hand-writing is reasonable but
  front-loads work better done once by the cargo-dist authors.

**Q2 — Linux: musl-static or glibc-dynamic?**

  - **(a)** musl-static. One binary works on every Linux distro
    (Debian 8 to Arch latest to Alpine), no glibc version
    drift. Slightly larger binary; slightly slower allocator on
    some workloads. Targets: `*-unknown-linux-musl`.
  - **(b)** glibc-dynamic. Smaller binary, faster allocator,
    but binds to a specific minimum glibc version (typically
    pinned by the CI runner's distro). Older distros break.

  **Recommendation: (a).** End-user distribution wants "works
  everywhere"; musl-static is the standard answer for portable
  Linux Rust binaries. Size/allocator overhead is negligible
  for our workload (tokio + reqwest dwarf any musl difference).

**Q3 — Versioning cadence?**

  - **(a)** Every phase exit bumps a version. Mechanical rule;
    every phase produces a release. Many releases, including
    docs-only.
  - **(b)** Only user-visible changes bump. Docs-only phases
    (22, 38, 54) skip releases. Substrate phases (51, 52, 55)
    likely bump. Phase exits decide at the time.
  - **(c)** Calendar-based. Release weekly or biweekly,
    independent of phase boundaries.

  **Recommendation: (b).** Phase-driven but selective. Matches
  the "phase exit is the natural decision point" rhythm. Each
  tag tells operators something they can see shipped. Phase 61
  tags v0.1.0 *because* an end user can now install, not just
  because a phase ended.

**Q4 — Install location and PATH handling?**

  - **(a)** cargo-dist default — `~/.cargo/bin/aivyx`. Existing
    Rust users have it on `PATH` already; everyone else gets a
    printed `export PATH=...` instruction from the installer.
  - **(b)** XDG-standard `~/.local/bin/aivyx`. Distribution-
    standard for non-Rust tools; widely on `PATH` via
    `.profile`.
  - **(c)** `/usr/local/bin/aivyx` via sudo. System-wide; on
    `PATH` out of the box; requires sudo and aggressive
    system-state changes.

  **Recommendation: (a).** cargo-dist's default. Per-user (no
  sudo), respected by convention, and the installer prints
  PATH guidance. Matches "single-operator personal-agent
  platform" (P1 + P6) — each operator's binary, not a shared
  system binary.

## Deferrals

**Net-new deferrals from Phase 61:**

- **v0.1.0 publication (Task 7).** The operator-side action of
  creating a public GitHub repo, pushing history, and tagging
  `v0.1.0` is held indefinitely under the operator's
  VPS-private-first posture. The substrate (CI gates, dist
  config, release workflow) sits ready and fires the first time
  a `v*.*.*` tag is pushed to a public GitHub remote. Re-opens
  as a focused micro-phase when public hosting is configured.
- **Native Windows port** (out of Phase 61 scope per sign-off).
  Daemon IPC is Unix-domain-socket-only (`UnixListener` /
  `UnixStream` throughout `daemon_server.rs` / `daemon_client.rs`,
  `#[cfg(unix)]` across `aivyx-channel`). A Windows port needs
  a `NamedPipe` replacement layer, platform-conditional spawn
  paths for `shell.exec`, and a parallel CI matrix.
- **Homebrew tap** (out of Phase 61 scope per sign-off). Adds a
  `homebrew-aivyx` repository plus formula maintenance overhead.
- **crates.io publishing** (out of Phase 61 scope per sign-off).
  Twelve workspace crates need namespace-availability checks
  plus a workspace-wide versioning policy.
- **Docker image** (out of Phase 61 scope per sign-off). Adds
  a Dockerfile + image build job in CI.
- **macOS signing + notarization** (out of Phase 61 scope per
  sign-off). Requires an Apple Developer account ($99/yr) plus
  a cert pipeline in CI.

## Prediction vs. reality

- **DESIGN.md** — Predicted: streak **extends to eight**.
  **Reality: correct.** Hash unchanged at entry and exit:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Phase 61 shipped under existing D-deliverables; zero
  contract touch.

- **PRODUCT.md** — Predicted: streak **recovers to one**.
  **Reality: correct.** Hash unchanged at entry and exit:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  No commitment-text edits, no Delivery Status refresh (P1–P14
  stay Fully Delivered). The forthcoming "Distribution"
  milestone landed in `docs/PRODUCT_ROADMAP.md`, not in the
  contract document.

- **Production-core `aivyx-core/src/lib.rs`** — Predicted: streak
  **extends to nine** (new record). **Reality: correct.** Hash
  unchanged at entry and exit:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Task 2's `--version` flag landed in `aivyx-channel`'s binary
  file as predicted; every other task landed in CI yaml, dist
  config, root Cargo.toml, or docs. Nine consecutive phases
  beats the prior Phase 60 record of eight.

- **Test count** — Predicted: small positive (~+1). **Reality:
  +3** (1071 → 1074). Three parser tests for `--version`:
  long-flag, short-flag (`-V`), and the extra-args rejection
  path. Zero clippy warnings throughout.

- **Pipeline-vs-publication outcome** — Not predicted at open.
  Phase 61's stated goal was to publish v0.1.0; the actual
  outcome is pipeline-ready-but-not-firing under the VPS-first
  pivot. This is the most material deviation from the open
  doc's framing and is recorded in the Goal section's
  "Revised outcome" addendum.

## Exit criteria

- [x] `aivyx --version` flag wired and tested — Task 2,
  commit `aa198f7`.
- [x] `.github/workflows/release.yml` + `dist-workspace.toml`
  generated — Task 3, commit `05bf044`.
- [x] CI gates run `cargo test --workspace` +
  `cargo clippy --workspace --all-targets -- -D warnings`
  before artifact upload — Task 4, commit `8b2c6eb`.
- [x] README quickstart refreshed; status table updated; macOS
  Gatekeeper workaround documented — Task 5, commit `cde239a`
  + Task 5/6 fixup `7ac8cd8`.
- [x] `docs/INSTALL.md` covers the install matrix — Task 6,
  commit `93ec379` + Task 5/6 fixup `7ac8cd8`.
- [ ] v0.1.0 tagged + pushed; release workflow fires; install
  script downloads and runs on at least one verified host —
  **Task 7, deferred** to a focused micro-phase under the
  VPS-first posture.
- [x] ROADMAP.md + PRODUCT_ROADMAP.md + docs/README.md
  refreshed — Task 8 (this commit).
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 3 (Q1(a) cargo-dist, Q2(a) musl-static,
  Q3(b) phase-driven user-visible cadence, Q4(a)
  `~/.cargo/bin/`).
- [x] DESIGN.md streak extends to eight.
- [x] PRODUCT.md streak recovers to one.
- [x] Production-core streak extends to nine (new record).
- [x] Test count delta: +3 (1071 → 1074).
- [x] Prediction-vs-reality block filled.

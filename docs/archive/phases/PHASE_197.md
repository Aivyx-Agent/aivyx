# Phase 197 — Chapter N, Phase 3: Regression Guard + Doc Correction

**Chapter N, phase 3 — [SHIPPED] 2026-09-06. Chapter N is now
COMPLETE.**

## Goal (carried from the chapter design)

Phase 192 fixed the root cause (three private git dependencies
blocking every `v0.9.0` release workflow) and Phase 193 cut and
verified a real release (`v0.9.4`), but two things from the original
chapter design were still open: `scripts/check-git-deps-public.sh`
(Phase 192) existed only as a standalone script an operator would have
to remember to run by hand, and `README.md`/`docs/INSTALL.md` still
carried stale version claims in opposite directions — `README.md`
overclaimed `v0.9.0` as current/latest, while `docs/INSTALL.md` still
described a pre-`v0.1.0` state that was never real to begin with.
Phase 197 (Chapter N's phase 3 — see PHASE_193.md's follow-ups for why
the number isn't 194) was scoped to close both gaps and, with them,
close the chapter.

## What shipped

- **`check-git-deps-public.sh` wired into `quality-gate.yml`**
  (`ed48caec`, cherry-picked onto this branch after an
  `isolation: "worktree"` dispatch mistake put the original commit —
  `98d4b875` — in a disconnected worktree; the diff content is
  unchanged). Added as a new step immediately after `Checkout` and
  before the disk-cleanup/toolchain/cache steps that exist only to
  support `cargo test`/`clippy` — deliberately first, since those
  later steps would fail anyway (just ~10-20 minutes later, via an
  opaque cargo auth backtrace) if this check would have failed.
  `quality-gate.yml` is the shared reusable workflow both `ci.yml` and
  `release.yml` (via `dist-workspace.toml`'s plan-jobs) call, so one
  step covers both entry points — the exact failure mode that broke
  every `v0.9.0` release workflow now surfaces in seconds with a named
  URL instead of an opaque `cargo` backtrace deep into the run.
  Verified two ways before landing: pointing `Cargo.toml`'s
  `aivyx-confine` dependency at a nonexistent placeholder repo made the
  unmodified script fail with exit code 1 and a named URL; restoring
  `Cargo.toml` and re-running against the real, unmodified dependencies
  passed clean (`aivyx-confine`, `aivyx-checkpoint`, `aivyx-kvcache`,
  `aivyx-injection-guard` all `OK: anonymously cloneable`). The script
  itself was not modified — its three known gaps (see Known
  follow-ups) are unchanged.
- **Six stale pre-launch version references corrected across two
  files** (`b787c6d2`, plus a sixth caught by the final whole-branch
  review and fixed in `214843f0` — see below) — four more than the
  chapter design's original two-location scope: an initial sweep
  during implementation found three more instances of the same
  category of error, and the final review found one more that the
  sweep's own verification grep couldn't have caught:
  - `README.md`'s "Status" header (`v0.9.0 — 2026-09-03` →
    `v0.9.4 — 2026-09-05`).
  - `README.md`'s status-table "Release pipeline" row (same version
    bump, plus its description of what `v0.9.0` shipped replaced with
    what `v0.9.4` actually shipped — Chapter N's own fixes plus
    Chapter Picket's injection tripwire).
  - `README.md`'s "Release pipeline status" section (same version bump,
    condensed to point at the CHANGELOG rather than re-describing the
    release inline, plus a new line documenting the git-dependency
    check this same phase just wired in).
  - `docs/INSTALL.md`'s "Current install state" section, which still
    named `v0.1.0` as "the first public release (pre-release)" — real
    finding: `v0.1.0` was never actually tagged (`git tag -l v0.1.0`
    returns nothing locally or on the remote). This was aspirational
    pre-launch copy that was never corrected after the real first
    release shipped.
  - `docs/INSTALL.md`'s "for a specific version" installer example,
    which hardcoded a `v0.1.0` download URL. Replaced with a generic
    `vX.Y.Z` placeholder rather than hardcoding today's actual latest
    (`v0.9.4`) — hardcoding the current version would silently go
    stale again on the very next release, which is the exact bug this
    phase exists to stop.

  One `v0.9.0` string deliberately remains in `README.md` after this
  change (line 257, in the new git-dependency-check description) — it
  is a historical reference to the incident Phase 192 fixed, not a
  claim about the current latest, and was confirmed as such rather
  than flagged as a miss.
- **`CHANGELOG.md` backfilled with the missing `v0.9.4` entry**
  (`aaa218bf`). Real finding driving this task's scope: `gh release
  list` shows only `v0.9.0` and `v0.9.4` were ever actually published
  as real GitHub Releases — `v0.9.1`, `v0.9.2`, and `v0.9.3` were
  intermediate local workspace-version bumps on the original Phase 193
  worktree branch (chasing two genuine `aivyx-confine` bugs and a WSL
  retry-budget fix along the way) that got superseded by the merge to
  `main` and never shipped as their own releases — see PHASE_193.md's
  "Reconciled a long-diverged branch" entry. The new entry covers
  everything that actually landed in `v0.9.4`: Chapter Picket's active
  prompt-injection tripwire, the two `aivyx-confine` fixes
  (`require_enforcement`'s `FullyEnforced` requirement failing on
  GitHub's own hosted-runner kernel; `SYS_kexec_file_load` missing from
  musl's aarch64/riscv64 libc bindings), the WSL image-pull retry
  budget raise (10 → 30 minutes), and the three git dependencies going
  public. This phase's own new CI regression guard (Task 1, above)
  hadn't shipped yet when `v0.9.4` was tagged the day before — it's
  recorded in `[Unreleased]` instead, correctly attributed to this
  phase rather than backdated onto an already-published release.

Along the way, the first review round was briefly slowed by a stale
leftover report file from an unrelated earlier phase sitting in
`.superpowers/sdd/`, which was identified and corrected before it
affected the actual review outcome — noted here since it's a real
process detail of how this phase went, not because it changed what
shipped.

The final whole-branch review (Opus) caught three real, worth-fixing
issues that no task-level review could have, since each only shows up
viewing the branch as a whole: (1) the CHANGELOG mis-attribution just
described; (2) a **sixth** stale version reference Task 2 missed —
`docs/INSTALL.md`'s shell-installer worked example still showed
`aivyx --version` producing `# aivyx 0.1.0`, which Task 2's Step 6
verification grep (`v0\.1\.0`) structurally couldn't catch since the
line has no leading `v`; fixed to a generic `# aivyx x.y.z`; (3)
`docs/ROADMAP.md`'s Chapter N section heading itself was never marked
`[COMPLETE]`, unlike the file's dominant convention for other finished
chapters (a few, like Chapter J, instead use `✅ COMPLETE` — either
form is established, but Chapter N's heading used neither) — the
phase-list bullet said so, the heading didn't. All three fixed in one commit (`214843f0`), plus two Minor
polish items from the same review (a "Chapter N" naming collision with
README.md's own already-in-use, unrelated "Chapter N" — Operator
Access Levels — and a cosmetic backtick-vs-link inconsistency).

## The result

Chapter N is complete. All three of its phases are now closed: Phase
192 fixed the root cause (private git dependencies), Phase 193 cut and
verified the first real release since `v0.8.3`, and Phase 197 makes
the fix self-enforcing (a future regression fails CI in seconds,
by name, instead of silently shipping another `v0.9.0`-style broken
release) and brings the two most user-facing docs (`README.md`,
`docs/INSTALL.md`) back in line with what's actually true today. The
chapter's original goal — an unaffiliated outside end user can get a
current, working `aivyx` binary through every documented path, and the
pipeline can't silently regress into shipping stale or broken
artifacts again without being caught immediately — is met.

## Known follow-ups (not done here, logged for whenever they matter)

- **`check-git-deps-public.sh`'s `|| true` also swallows `grep`'s exit
  code 2** (a genuine read error, e.g. an unreadable — not missing —
  `Cargo.toml`), which would currently report a false "OK, nothing to
  check" instead of failing loudly. Low severity (stderr still carries
  the real cause; contrived in practice since CI's checkout is always
  readable) — this phase wired the script into CI but did not touch
  its internals, so this gap is unchanged and still open.
- **The script only matches `https://` git URLs**, not `ssh://` or
  `git@github.com:` forms, and only scans the root `Cargo.toml` (every
  workspace member currently inherits via `workspace = true`, so this
  is exhaustive today but not structurally enforced). Both remain real
  gaps, unchanged by this phase.
- **Why did `aivyx` revert to private?** Still genuinely unresolved
  (carried from PHASE_193.md). Worth a quick check of
  collaborator/webhook activity or GitHub's own audit-log UI next time
  someone is in the repo's Settings, in case it recurs.
- **Audit trail for the visibility flip is thin at this account tier**
  (carried from PHASE_193.md). If this recurs, it's worth checking
  whether a scoped PAT, a GitHub App installation, or a
  branch-protection/ruleset change has permission to alter repo
  visibility, since nothing in Phase 193's investigation found a
  specific actor or trigger.

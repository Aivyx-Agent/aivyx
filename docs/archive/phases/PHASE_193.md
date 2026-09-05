# Phase 193 — Chapter N, Phase 2: Cut a Real Release, Prove Build-From-Source

**Chapter N, phase 2 — [SHIPPED] 2026-09-05.**

## Goal (carried from the chapter design)

Phase 192 fixed the actual root cause (three private git dependencies
blocking every `v0.9.0` release workflow) but never produced a real
release — `gh release list` still showed `v0.8.3` as Latest. Phase 193
was scoped to close that loop: tag a genuinely new version, watch all
four release workflows (`release.yml`, `desktop-release.yml`,
`docker-publish.yml`, `wsl-release.yml`) actually succeed end-to-end,
and separately prove the documented `git clone && cargo build`
sequence works for a real, credential-less outsider — not just inside
GitHub's own CI context.

## What shipped

- **Reconciled a long-diverged branch.** The original Phase 193
  worktree (six real commits: three version bumps chasing two genuine
  `aivyx-confine` bugs found along the way, plus a WSL retry-budget
  fix) had been built off the commit right after Phase 192, then sat
  unmerged while Chapter Picket's three phases were designed, built,
  and merged into `main` from that same ancestor. Neither line of work
  knew about the other. Merged cleanly via `git merge
  worktree-release-distribution-integrity-phase193 --no-edit` (no
  conflicts — `Cargo.lock`/`Cargo.toml` both auto-merged), re-verified
  with `cargo check`/`cargo test` (120 result blocks, 0 failures)/
  `cargo clippy --all-targets -- -D warnings` (clean) on the merged
  result before cleaning up the now-redundant branch, worktree, and the
  stale `v0.9.3` tag (which pointed at the pre-merge commit and never
  had a successful release attached).
- **Bumped to `v0.9.4`** (not `v0.9.1`–`v0.9.3`, since `main` now
  contains strictly more than any of those versions represented —
  Chapter Picket's injection-guard adoption is genuinely new content on
  top of the release-pipeline fixes) and cut a real, tagged release.
- **Found and fixed a second real regression, independent of the code:
  `Aivyx-Agent/aivyx` had silently reverted from public back to
  private** sometime after Phase 192 set it public. This produced two
  simultaneous, superficially unrelated symptoms that took direct
  investigation to connect: (1) a credential-less Docker build-from-
  source attempt failed with `could not read Username for
  'https://github.com'` — a private-repo auth prompt, not a missing-
  repo error; (2) all four release workflows and the plain `CI` run on
  `main` instant-failed (`runner_id: 0`, 2–4 seconds, zero steps) —
  the exact signature of a private repo with no Actions budget
  available, not the GitHub secondary-rate-limit throttle this session
  had assumed was still in effect from an earlier bulk-API-deletion
  incident. Ruled out the throttle theory directly: primary rate limit
  was full (5000/5000), githubstatus.com showed Actions fully
  operational, the account wasn't suspended, and — most tellingly —
  the four sibling extraction repos (`aivyx-confine`,
  `aivyx-checkpoint`, `aivyx-kvcache`, `aivyx-injection-guard`) were
  all still correctly `PUBLIC`; only `aivyx` itself had flipped. Fixed
  with the user's explicit go-ahead via `gh repo edit
  Aivyx-Agent/aivyx --visibility public`, then re-verified anonymous
  clone access and re-ran (`gh run rerun`) all five previously-failed
  runs, which then genuinely built and passed. **Root cause of the
  revert itself was not found** — no audit-log access at this account
  tier, and nothing in Dependabot alerts or `security_and_analysis`
  settings pointed to an automated cause. Logged as an open question,
  not guessed at.
- **Verified the real release**: `v0.9.4` is `Latest`, not a draft or
  prerelease, with all four cross-compiled binaries
  (`x86_64`/`aarch64` × `unknown-linux-musl`/`apple-darwin`), the shell
  installer, `.deb`/macOS `.zip` desktop bundles, both WSL artifacts
  (`Aivyx.wsl`, `aivyx-wsl-rootfs-x86_64-0.9.4.tar.gz`), and a source
  tarball. The GHCR image (`ghcr.io/aivyx-agent/aivyx:0.9.4`) is
  independently pullable via `docker manifest inspect`.
- **Verified build-from-source for a genuinely credential-less
  outsider**: a throwaway `rust:1.85-slim` container, confirmed to have
  no inherited `~/.gitconfig`, `/etc/gitconfig`, or git/GitHub/SSH
  environment variables, anonymously cloned `Aivyx-Agent/aivyx`,
  checked out the `v0.9.4` tag, and ran the exact sequence
  `docs/INSTALL.md` documents (`cargo build --release --bin aivyx`,
  no `--target`). Exit code `0`, `Finished \`release\` profile`, and
  the built binary reported `aivyx 0.9.4`. Evidence saved to
  `.superpowers/sdd/task-3-build-from-source.log`; the container was
  discarded afterward.

## The result

`aivyx` now has a real, current, working GitHub Release for the first
time since `v0.8.3` — every path `docs/INSTALL.md` documents (release
binaries, GHCR image, WSL appliance, build-from-source) is proven to
work for someone with zero special access, not assumed. Chapter N's
root-cause phase (192) and its release-cutting phase (193) are both
now closed; only the regression-guard-in-CI + doc-correction phase
remains, still unstarted.

## Known follow-ups (not done here, logged for whenever they matter)

- **Why did `aivyx` revert to private?** Genuinely unresolved. Worth a
  quick check of collaborator/webhook activity or GitHub's own
  audit-log UI (not reachable via this session's API access) next time
  someone is in the repo's Settings, in case it recurs.
- **Chapter N's remaining phase** (wire `check-git-deps-public.sh` into
  `quality-gate.yml`; correct `README.md`'s "Release pipeline status"
  and `docs/INSTALL.md`'s "Current install state" sections, both stale
  in opposite directions) was tentatively "Phase 194" in the original
  chapter design, but that number was reassigned to Chapter Picket
  once it was scoped as its own chapter. Needs a fresh phase number
  whenever picked up.
- **Audit trail for the visibility flip is thin.** If this recurs, it's
  worth checking whether a scoped PAT, a GitHub App installation, or a
  branch-protection/ruleset change on the account has permission to
  alter repo visibility, since nothing in this phase's investigation
  found a specific actor or trigger.

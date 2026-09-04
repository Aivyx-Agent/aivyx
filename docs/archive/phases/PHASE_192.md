# Phase 192 — Chapter N, Phase 1: Unblock the Release Pipeline

**Chapter N, phase 1 — [SHIPPED] 2026-09-05.**

## Goal (carried from the chapter design)

Investigating "how would an end user deploy Aivyx on their own bare
metal" (this session, 2026-09-05) surfaced that `v0.9.0` — the version
`README.md`'s own "Status" line named as current — was never actually
published as a real GitHub Release: `gh release list` showed `v0.8.3`
(2026-07-08) as the true "Latest," over 7 weeks stale relative to
`main`. Root cause, confirmed against real GitHub/CI state rather than
documentation claims: `Cargo.toml`'s `[workspace.dependencies]` table
declared three git dependencies, each pinned by commit SHA, each
pointing at a private `Aivyx-Agent` repo — `aivyx-confine`,
`aivyx-checkpoint`, `aivyx-kvcache` — all wired into `aivyx-core` as
non-optional dependencies (the crate whose own Cargo.toml comment calls
it "a dependency of every shipped binary"). A CI runner's ambient
`GITHUB_TOKEN` is scoped only to the repo it's executing in and cannot
authenticate against a different, private repo — so `v0.9.0`'s tag push
made all four release workflows (`release.yml`, `desktop-release.yml`,
`docker-publish.yml`, `wsl-release.yml`) fail identically and fast
(1m50s–11m40s vs. 11–24 minutes historically), before any of them
reached their real cross-compile/package/publish work. The same failure
almost certainly blocks the documented "build from source" path for any
real outside contributor too, not just CI.

Phase 192 is Chapter N's first phase: the root-cause fix that Phases
193 (cut + verify a real release; credential-less build-from-source
verification) and 194 (regression guard + doc corrections) both depend
on.

## What shipped

- **`scripts/check-git-deps-public.sh`** — a new, reusable, standalone
  regression-guard script. Extracts every `git = "https://..."` URL
  from the workspace `Cargo.toml` and probes each with a credential-less
  `git ls-remote` (`-c credential.helper=` clears any configured
  helper for that invocation only; `GIT_TERMINAL_PROMPT=0` fails fast
  instead of hanging on an interactive prompt). Chosen over a `gh repo
  view --json isPrivate` check specifically because it tests the
  literal property that broke ("can this be fetched with zero special
  access"), needs no token, and isn't GitHub-specific — a design
  decision made during this chapter's brainstorm, not an implementation
  afterthought. Proven against real, live GitHub state before the fix
  (all three dependencies genuinely failed) and after (all three
  genuinely passed) — not synthetic fixtures. Phase 194 will wire this
  same script into `quality-gate.yml`.
- **`Aivyx-Agent/aivyx-confine`, `Aivyx-Agent/aivyx-checkpoint`,
  `Aivyx-Agent/aivyx-kvcache` are now public GitHub repositories.**
  Their privacy was never intentional — confirmed with the operator
  during design: they're infrastructure utilities (Landlock+seccomp
  process confinement, git-ref checkpoint/rollback, KV-cache
  persistence), adopted from the sibling `aivyx-coder` repo and never
  revisited, not business logic or secrets. A CI-only credential (a
  scoped PAT) was considered and rejected during design — it would have
  fixed the release pipeline but left "build from source" permanently
  broken for anyone without repo access, which is exactly the failure
  this chapter exists to close. This was the one action in the phase
  the controller executed directly rather than delegating to a
  subagent, since it changes real, externally-visible state for three
  organization repositories — the user's explicit go-ahead was obtained
  immediately before running the three `gh repo edit --visibility
  public` commands, verified via `gh repo view` and, independently, via
  a second read-only verification subagent re-running both checks from
  scratch.
- **`Cargo.toml`'s three dependency comment blocks now document the
  constraint** that these repos must stay public, referencing the real
  incident (created 2026-08-16/2026-08-17; broke the `v0.9.0` release
  workflow on 2026-09-02; found and fixed as Phase 192 on 2026-09-05)
  and the verification script, so a future maintainer can't
  re-privatize one of these repos without understanding the
  consequence.
- **A real, independently-verified fix wave**: the final whole-branch
  review (Opus) found two factual/mechanism issues a task-level review
  had no way to catch — the plan's own literal replacement text for
  the `Cargo.toml` comment claimed the repos had been "privately-hosted
  from 2026-06" and that their privacy "silently broke the entire
  v0.9.0 release pipeline... for 7+ weeks before anyone noticed," both
  wrong (repos were created 2026-08-16/17; the break-to-discovery
  window was three days, 2026-09-02 to 2026-09-05 — the true "7+ weeks"
  figure the plan's author had in mind is *release staleness* since
  v0.8.3, a real but different fact that predates these dependencies
  existing at all); and `scripts/check-git-deps-public.sh`'s own
  zero-git-deps path was dead code — `set -euo pipefail` combined with
  `grep` exiting 1 on no match meant the script silently aborted before
  ever reaching its own intended "nothing to check" / exit-0 branch.
  Both fixed in one dispatch, both independently re-verified correct by
  a second Opus re-review that gathered its own fresh evidence (real
  `gh repo view --json createdAt` timestamps, a live `set -e` repro of
  the grep bug, a synthetic zero-git-deps Cargo.toml to prove the fix
  branch is now reachable) rather than trusting the fix report's word.

## The result

Chapter N's first phase closes the actual root cause: the three git
dependencies that made every `v0.9.0` release workflow fail are now
anonymously cloneable, proven with a real, reusable verification tool
rather than assumed. `docs/superpowers/specs/2026-09-05-release-
distribution-integrity-design.md` grounds the rest of the chapter;
Phase 193 (cut and verify a real release, then prove build-from-source
works for a genuinely credential-less outside user) and Phase 194
(wire the regression guard into CI, correct the stale README/INSTALL.md
sections) are next.

## Known follow-ups (not done here, logged for whenever they matter)

- **Audit the three newly-public repos' git history for accidentally-
  committed secrets.** They were private until this phase; nothing in
  the design, plan, or review process checked their history before the
  visibility flip. The operator's rationale (infrastructure utilities,
  not business logic) is reasonable but isn't the same as having
  checked. Cheap to do now and still actionable if anything turns up —
  a real candidate for immediate follow-up, not deferred to a future
  chapter.
- **`check-git-deps-public.sh`'s `\|\| true` also swallows `grep`'s
  exit code 2** (a genuine read error, e.g. an unreadable — not
  missing — `Cargo.toml`), which would currently report a false "OK,
  nothing to check" instead of failing loudly. Low severity (not
  silent — stderr still carries the real cause — and contrived in
  practice, since CI's checkout is always readable), left as optional
  polish for whoever wires the script into CI in Phase 194.
- **The script only matches `https://` git URLs**, not `ssh://` or
  `git@github.com:` forms, and only scans the root `Cargo.toml` (every
  workspace member currently inherits via `workspace = true`, so this
  is exhaustive today but not structurally enforced). Both are real
  gaps to harden in Phase 194, not blockers for this phase's own scope.

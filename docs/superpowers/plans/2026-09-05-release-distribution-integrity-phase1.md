# Phase 192 — Chapter N, Phase 1: Unblock the Release Pipeline — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `aivyx-confine`, `aivyx-checkpoint`, and `aivyx-kvcache` — currently-private `Aivyx-Agent` repos wired into `aivyx-core` as pinned-rev git dependencies — anonymously cloneable, and prove it with a reusable, credential-less verification script rather than assuming it worked.

**Architecture:** A small standalone shell script (`scripts/check-git-deps-public.sh`) parses every `git = "https://..."` dependency out of the workspace `Cargo.toml` and probes each with a credential-less `git ls-remote`. Run once now to *prove the bug is real* (all three currently fail), then the three GitHub repos are switched to public, then the same script is run again to *prove the fix worked* (all three now pass). This script is a real, reusable deliverable — Phase 194 (the chapter's regression-guard phase) wires it into CI directly rather than re-deriving the same check.

**Tech Stack:** Bash (POSIX-ish; matches the rest of `scripts/` in this repo, e.g. `scripts/dev-run.sh`, `scripts/install-hooks.sh`), `git`, `gh` CLI.

## Global Constraints

- The three repos (`aivyx-confine`, `aivyx-checkpoint`, `aivyx-kvcache`) become **public**, not gated behind a CI-only credential — this was decided during design specifically because a CI-only fix would leave "build from source" permanently broken for real outside contributors. Source: `docs/superpowers/specs/2026-09-05-release-distribution-integrity-design.md`.
- The verification mechanism is an **anonymous-clone probe** (`git ls-remote` with no credential helper), not a `gh repo view --json isPrivate` check — it tests the literal property that broke ("can this be fetched with zero special access"), needs no token, and isn't GitHub-specific. Same source.
- No change to what `aivyx-confine`/`aivyx-checkpoint`/`aivyx-kvcache` actually *do* — this phase only changes their GitHub visibility and this repo's documentation of them, never their internal implementation (they're separate repos with their own `CLAUDE.md`).
- Do not touch `Cargo.lock`, the pinned `rev` values, or any consuming crate's code — the dependency *declarations* are already correct; only their reachability is broken.
- Out of scope for this phase specifically (later phases in the same chapter): cutting a real release (Phase 193), credential-less build-from-source verification in a throwaway container (also Phase 193 or later), wiring the probe script into `quality-gate.yml` and fixing the stale README/INSTALL.md sections (Phase 194).

---

### Task 1: Write the anonymous-clone-check script and confirm it detects today's real failure

**Files:**
- Create: `scripts/check-git-deps-public.sh`

**Interfaces:**
- Produces: a standalone, dependency-free shell script, runnable as `./scripts/check-git-deps-public.sh` from the repo root, exit code `0` if every `git = "https://..."` dependency in `Cargo.toml` is anonymously cloneable, exit code `1` (with each failing URL named on stderr) otherwise. Phase 194 will call this exact script from CI — its interface (path, exit code contract, no required arguments) must not change without updating that later phase.

- [ ] **Step 1: Write the script**

```bash
cat > scripts/check-git-deps-public.sh << 'SCRIPT_EOF'
#!/usr/bin/env bash
# Confirms every git dependency declared in the workspace Cargo.toml is
# anonymously cloneable — no credential helper, no cached token, no
# interactive prompt fallback. This is the exact property whose silent
# failure broke the v0.9.0 release pipeline (see
# docs/superpowers/specs/2026-09-05-release-distribution-integrity-design.md):
# a git dependency pinned to a private repo passes `cargo check` on any
# machine that already has cached access to it, but fails for CI and for
# any real outside contributor. Exits non-zero, naming every offending
# URL, if any dependency requires authentication to fetch.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TOML="$REPO_ROOT/Cargo.toml"

if [ ! -f "$CARGO_TOML" ]; then
  echo "error: $CARGO_TOML not found" >&2
  exit 2
fi

urls=$(grep -oE 'git = "https://[^"]+"' "$CARGO_TOML" | sed -E 's/git = "(.*)"/\1/' | sort -u)

if [ -z "$urls" ]; then
  echo "No git dependencies found in $CARGO_TOML — nothing to check."
  exit 0
fi

failed=0
while IFS= read -r url; do
  echo "Checking anonymous clone access: $url"
  # -c credential.helper= clears any configured credential helper for
  # this invocation only (an empty value resets the helper list — this
  # is documented git behavior, not a no-op). GIT_TERMINAL_PROMPT=0 makes
  # git fail immediately instead of hanging on an interactive
  # username/password prompt when a repo needs auth. Together these make
  # the check genuinely credential-less regardless of what's cached on
  # the machine running it.
  if GIT_TERMINAL_PROMPT=0 git -c credential.helper= ls-remote "$url" > /dev/null 2>&1; then
    echo "  OK: anonymously cloneable"
  else
    echo "  FAIL: could not be cloned without credentials" >&2
    failed=1
  fi
done <<< "$urls"

echo ""
if [ "$failed" -ne 0 ]; then
  echo "One or more git dependencies require authentication to clone." >&2
  echo "This breaks CI (the default GITHUB_TOKEN can't authenticate against" >&2
  echo "a different repo) and any outside contributor's build-from-source path." >&2
  exit 1
fi

echo "All git dependencies are anonymously cloneable."
SCRIPT_EOF
chmod +x scripts/check-git-deps-public.sh
```

- [ ] **Step 2: Run it and confirm it correctly detects today's real, live failure**

Run: `./scripts/check-git-deps-public.sh`

Expected output (order of the three URLs may vary — `sort -u` orders them alphabetically, so the real order is `aivyx-checkpoint`, `aivyx-confine`, `aivyx-kvcache`):

```
Checking anonymous clone access: https://github.com/Aivyx-Agent/aivyx-checkpoint
  FAIL: could not be cloned without credentials
Checking anonymous clone access: https://github.com/Aivyx-Agent/aivyx-confine
  FAIL: could not be cloned without credentials
Checking anonymous clone access: https://github.com/Aivyx-Agent/aivyx-kvcache
  FAIL: could not be cloned without credentials

One or more git dependencies require authentication to clone.
This breaks CI (the default GITHUB_TOKEN can't authenticate against
a different repo) and any outside contributor's build-from-source path.
```

Expected exit code: `1` (check with `echo $?` immediately after).

This is the "red" state — a real, live demonstration that the script correctly identifies the actual bug this whole chapter exists to fix, not a synthetic test fixture. If any of the three unexpectedly shows `OK` here, stop and report it (it would mean the repo's visibility already changed since this plan was written, or the machine running this has cached credentials that leaked through — check with `git -c credential.helper= -c http.https://github.com/.extraheader= ls-remote <url>` to rule out an inherited header-based token before concluding the script itself is wrong).

- [ ] **Step 3: Commit**

```bash
git add scripts/check-git-deps-public.sh
git commit -s -m "scripts: add anonymous-clone probe for workspace git dependencies

Confirms every git = \"https://...\" dependency in Cargo.toml is
fetchable with zero credentials. Run now to prove the aivyx-confine /
aivyx-checkpoint / aivyx-kvcache private-repo bug (the one that broke
the v0.9.0 release pipeline) is real; Phase 194 wires this same
script into quality-gate.yml as a fast, specific regression guard.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: Make the three repositories public and verify the fix with the same script

**Files:** None — this task changes real external GitHub repository state, not repo-local files.

**Interfaces:**
- Consumes: `scripts/check-git-deps-public.sh` from Task 1 (unchanged) as the verification tool.
- Produces: three GitHub repos (`Aivyx-Agent/aivyx-confine`, `Aivyx-Agent/aivyx-checkpoint`, `Aivyx-Agent/aivyx-kvcache`) with `isPrivate: false`, confirmed both via `gh repo view` and via the anonymous-clone script actually succeeding — later phases (193, 194) depend on this being genuinely done, not merely attempted.

**Execution note — read before running this task:** this task changes real, externally-visible GitHub repository visibility for three organization repositories outside this repo. It is **not** a code change and should **not** be delegated to an autonomous subagent with no human checkpoint — the controller running this plan should execute these commands directly and get an explicit go-ahead immediately before running them, even though the approach itself (make public, not a CI-only credential) was already confirmed with the user during this chapter's design. Confirming *the decision* at design time is not the same as confirming *the moment of execution* for an action this externally visible.

- [ ] **Step 1: Confirm current state (all three still private) before changing anything**

Run:
```bash
for repo in aivyx-confine aivyx-checkpoint aivyx-kvcache; do
  gh repo view "Aivyx-Agent/$repo" --json isPrivate --jq ".isPrivate"
done
```

Expected: `true` printed three times. If any prints `false` already, stop and report which one — do not proceed assuming the plan's premise still holds unverified.

- [ ] **Step 2: Get explicit go-ahead, then make each repository public**

After confirming with the user this specific action should proceed now, run:

```bash
for repo in aivyx-confine aivyx-checkpoint aivyx-kvcache; do
  gh repo edit "Aivyx-Agent/$repo" --visibility public --accept-visibility-change-consequences
done
```

- [ ] **Step 3: Verify via GitHub's own API that all three are now public**

Run:
```bash
for repo in aivyx-confine aivyx-checkpoint aivyx-kvcache; do
  gh repo view "Aivyx-Agent/$repo" --json isPrivate --jq ".isPrivate"
done
```

Expected: `false` printed three times.

- [ ] **Step 4: Verify via the anonymous-clone script — the real proof this fixes the actual bug**

Run: `./scripts/check-git-deps-public.sh`

Expected output:
```
Checking anonymous clone access: https://github.com/Aivyx-Agent/aivyx-checkpoint
  OK: anonymously cloneable
Checking anonymous clone access: https://github.com/Aivyx-Agent/aivyx-confine
  OK: anonymously cloneable
Checking anonymous clone access: https://github.com/Aivyx-Agent/aivyx-kvcache
  OK: anonymously cloneable

All git dependencies are anonymously cloneable.
```

Expected exit code: `0`.

Do not consider this task complete until this exact script — not a manual eyeball of the GitHub UI — reports success. `isPrivate: false` and "actually fetchable with zero credentials" are related but distinct claims; Step 4 is the one that matters for what this chapter is actually fixing.

- [ ] **Step 5: Confirm the real build still resolves dependencies cleanly on this machine**

Run: `cargo metadata --format-version=1 > /dev/null && echo "metadata resolved cleanly"`

Expected: `metadata resolved cleanly` — this doesn't prove the outside-contributor path yet (that's Phase 193's job, in a genuinely credential-less container), but it's a cheap sanity check that nothing about the dependency declarations themselves broke.

There is no commit for this task — it changes no repo-local files. Record the outcome (all three now public, script passing) in the task report for the reviewer.

---

### Task 3: Document why these three repos must stay public

**Files:**
- Modify: `Cargo.toml:233-261` (the three dependency comment blocks for `aivyx-confine`, `aivyx-checkpoint`, `aivyx-kvcache`)

**Interfaces:**
- Consumes: nothing new — this is a documentation-only change to existing comment blocks.
- Produces: nothing later tasks depend on programmatically; this is the human-facing record of *why* these three entries must never be re-privatized without also reintroducing the Phase 192 fix.

The existing comments (reproduced below from the real file) explain the *dependency* choice ("adopted from the sibling aivyx-coder repo," "not on crates.io," pinned-SHA rationale) but never mention that these repos silently broke the entire release pipeline for being private, or that they must stay public. Add that missing context.

- [ ] **Step 1: Read the current three comment blocks to confirm line numbers haven't shifted**

Run: `grep -n "aivyx-confine\|aivyx-checkpoint\|aivyx-kvcache" Cargo.toml`

If the line numbers differ meaningfully from `Cargo.toml:233-261`, use the real numbers reported here for the edits below — the content to match against is the exact comment text quoted in Step 2, not the line numbers.

- [ ] **Step 2: Add a one-line addendum to each of the three comment blocks**

Find this block (currently ending just above `aivyx-confine = { git = ...`):

```toml
# Shared Landlock+seccomp process confinement for shell.exec/git.rs,
# adopted from the sibling aivyx-coder repo rather than reimplemented here.
# Not on crates.io (see aivyx-confine's own README) — pinned by commit SHA.
```

Replace with:

```toml
# Shared Landlock+seccomp process confinement for shell.exec/git.rs,
# adopted from the sibling aivyx-coder repo rather than reimplemented here.
# Not on crates.io (see aivyx-confine's own README) — pinned by commit SHA.
# MUST stay a public repo: this was privately-hosted from 2026-06 until
# Phase 192 (Chapter N), and its privacy silently broke the entire
# v0.9.0 release pipeline (CI's ambient GITHUB_TOKEN can't authenticate
# against a different, private repo) for 7+ weeks before anyone noticed.
# `./scripts/check-git-deps-public.sh` verifies this reachability and is
# wired into CI (Phase 194) specifically to catch a re-privatization fast.
```

Find:

```toml
# Shared git-ref checkpoint/rollback for fs_root's mutating tools
# (fs.write, fs.delete, shell.exec), adopted from the sibling aivyx-coder
# repo (via aivyx-checkpoint) rather than reimplemented here. Not on
# crates.io — pinned by commit SHA. No platform-specific backend (pure
# git plumbing, unlike aivyx-confine) so no default-features/target-gating
# split is needed.
```

Replace with:

```toml
# Shared git-ref checkpoint/rollback for fs_root's mutating tools
# (fs.write, fs.delete, shell.exec), adopted from the sibling aivyx-coder
# repo (via aivyx-checkpoint) rather than reimplemented here. Not on
# crates.io — pinned by commit SHA. No platform-specific backend (pure
# git plumbing, unlike aivyx-confine) so no default-features/target-gating
# split is needed.
# MUST stay a public repo — see the aivyx-confine entry above for why;
# this was the specific dependency whose private-repo auth failure was
# traced and fixed in Phase 192 (Chapter N).
```

Find:

```toml
# KV-cache persistence against a llama-server backend (LlmPlanner/
# SpecialistFactory `with_kv_cache`), adopted from the sibling aivyx-coder
# repo rather than reimplemented here. Not on crates.io — pinned by commit
# SHA. No platform-specific backend, same as aivyx-checkpoint, so no
# default-features/target-gating split is needed. Centralized here
# 2026-08-27 — previously duplicated verbatim across aivyx-core, aivyx-team,
# aivyx-cli, and aivyx-channel's own Cargo.toml files.
```

Replace with:

```toml
# KV-cache persistence against a llama-server backend (LlmPlanner/
# SpecialistFactory `with_kv_cache`), adopted from the sibling aivyx-coder
# repo rather than reimplemented here. Not on crates.io — pinned by commit
# SHA. No platform-specific backend, same as aivyx-checkpoint, so no
# default-features/target-gating split is needed. Centralized here
# 2026-08-27 — previously duplicated verbatim across aivyx-core, aivyx-team,
# aivyx-cli, and aivyx-channel's own Cargo.toml files.
# MUST stay a public repo — see the aivyx-confine entry above for why.
```

- [ ] **Step 3: Confirm the file still parses as valid TOML and the workspace still resolves**

Run: `cargo metadata --format-version=1 > /dev/null && echo "Cargo.toml still valid"`

Expected: `Cargo.toml still valid` — comment-only edits shouldn't be able to break parsing, but this catches a stray unmatched `#` or accidental deletion of the `[dependencies]`-table line above/below the edited blocks.

- [ ] **Step 4: Re-run the anonymous-clone script one more time as a final sanity check**

Run: `./scripts/check-git-deps-public.sh`

Expected: same all-`OK` output and exit code `0` as Task 2 Step 3 — this task only touched comments, so nothing about reachability should have changed.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml
git commit -s -m "docs(cargo): note that aivyx-confine/checkpoint/kvcache must stay public

These three workspace git dependencies were privately-hosted until
Phase 192 (Chapter N), which found their privacy had silently broken
the entire v0.9.0 release pipeline — CI's ambient GITHUB_TOKEN can't
authenticate against a different, private repo. Documents the
constraint directly at the dependency declaration so a future
re-privatization isn't made without realizing the consequence.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Self-review notes (for whoever executes this plan)

- **Spec coverage:** Phase 192's spec scope was "make the three repos public; confirm — not assume — each is anonymously cloneable; update the dependency comments if the reasoning no longer matches reality." Task 1 builds the confirmation tool and proves the bug is real; Task 2 performs the fix and confirms it with that same tool; Task 3 updates the comments — not because the existing reasoning became *false*, but because it never disclosed the risk that just caused a real 7-week-silent outage, which is worth documenting directly at the point future maintainers will read it.
- **No placeholders:** every step has literal, runnable commands and exact expected output; the script in Task 1 is complete, not sketched.
- **Type/interface consistency:** Task 2 and Task 3 both re-invoke the exact script path (`./scripts/check-git-deps-public.sh`) and exit-code contract Task 1 defines — no drift.

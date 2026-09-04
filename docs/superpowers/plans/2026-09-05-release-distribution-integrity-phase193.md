# Phase 193 — Chapter N, Phase 2: Cut a Real Release + Prove Build-From-Source — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut and verify a genuinely working release (`v0.9.1`) now that Phase 192 unblocked the pipeline, then prove — not assume — that the documented "build from source" path works for a real outside contributor with zero cached credentials.

**Architecture:** Bump the workspace version (the `v0.9.0` tag already exists and its release never succeeded, so a new tag is needed — and cargo-dist requires the tag to match a real `Cargo.toml` version), push the new tag, and watch all four tag-triggered release workflows (`release.yml`, `desktop-release.yml`, `docker-publish.yml`, `wsl-release.yml`) actually succeed end-to-end. Then, independently of CI, build the exact same tag inside a throwaway Docker container stripped of every credential a real GitHub-hosted runner might still incidentally have, to rule out "it worked because of residual access" as a false positive.

**Tech Stack:** Bash, `git`, `gh` CLI, Docker.

## Global Constraints

- The new release tag is `v0.9.1` — `v0.9.0`'s tag already exists on `origin` (pushed 2026-09-02) and cannot be reused; its own release run failed, so nothing safely occupies that name.
- `[workspace.package] version` in the root `Cargo.toml` (currently `"0.9.0"`, line 102) must be bumped to `"0.9.1"` *before* tagging — cargo-dist's `release.yml` (auto-generated, in `.github/workflows/release.yml`) expects the pushed tag's version to match a real dist-able package's `Cargo.toml` version; `aivyx-cli` (the `aivyx` binary) inherits its version via `version.workspace = true`.
- Pushing the tag is a real, externally-visible action — it triggers real public CI runs, publishes a real GitHub Release, and pushes a real image to GHCR. Per the precedent set in Phase 192 (there: changing GitHub repo visibility), this step is **controller-executed with the user's explicit go-ahead immediately before running**, not delegated to an autonomous subagent.
- The build-from-source verification container must have **no mounted host git config, no forwarded SSH agent, no injected git/GitHub credentials of any kind** — this is what makes it a stronger proof than trusting the GitHub Actions run from Task 2, per the chapter design spec's own decision.
- Success for the build-from-source check is the full build log plus a working `aivyx --version` from the built binary — not just a zero exit code.
- No `CHANGELOG.md` exists in this repo; cargo-dist will generate a default release title/body. Not a gap to fix here.
- Out of scope for this phase (belongs to Phase 194, not yet started): wiring `scripts/check-git-deps-public.sh` into `quality-gate.yml`, and correcting `README.md`'s "Release pipeline status" / `docs/INSTALL.md`'s "Current install state" sections.

---

### Task 1: Bump the workspace version to 0.9.1

**Files:**
- Modify: `Cargo.toml:102` (`[workspace.package] version = "0.9.0"` → `"0.9.1"`)
- Modify: `Cargo.lock` (regenerated, not hand-edited)

**Interfaces:**
- Produces: every workspace member crate (all of which inherit via `version.workspace = true`) now resolves to version `0.9.1`, confirmed via `cargo pkgid -p aivyx-cli`. Task 2 tags exactly this commit.

- [ ] **Step 1: Bump the version**

In `Cargo.toml`, find:
```toml
[workspace.package]
version = "0.9.0"
```

Change to:
```toml
[workspace.package]
version = "0.9.1"
```

- [ ] **Step 2: Regenerate Cargo.lock**

Run: `cargo check`

This is a normal dependency-resolution pass — no code changed, only the version string — so it should complete without recompiling much beyond version metadata. It updates every workspace-member entry in `Cargo.lock` to `0.9.1`.

- [ ] **Step 3: Verify the version actually took**

Run: `cargo pkgid -p aivyx-cli`

Expected output (path prefix will vary by machine, the version suffix is what matters): a line ending in `#0.9.1`, e.g. `path+file:///home/.../aivyx#aivyx-cli@0.9.1`.

Also run: `grep -A1 'name = "aivyx-cli"' Cargo.lock`

Expected:
```
name = "aivyx-cli"
version = "0.9.1"
```

- [ ] **Step 4: Confirm the workspace still builds clean**

Run: `cargo build`

Expected: `Finished` with no errors (this only touches default-members, matching this repo's own `CLAUDE.md` build convention — don't add `--workspace`, which would also try to build `aivyx-desktop` and fail on missing system webview libs unrelated to this change).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -s -m "chore(release): bump workspace version to 0.9.1

v0.9.0's tag already exists on origin but its release run failed
before Phase 192 fixed the private-git-dependency root cause -- no
real GitHub Release was ever produced for it. Cutting a genuinely
new release needs a version cargo-dist hasn't already claimed.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: Tag, push, and verify all four release workflows actually succeed

**Files:** None — this task pushes a git tag and observes real GitHub Actions runs; no repo-local files change.

**Interfaces:**
- Consumes: the commit from Task 1 (workspace version `0.9.1`).
- Produces: a real GitHub Release `v0.9.1` with real build artifacts attached, a real GHCR image `ghcr.io/aivyx-agent/aivyx:0.9.1`, a real `Aivyx.wsl` release asset — the concrete precondition Task 3 verifies build-from-source against (the exact tagged commit, not an arbitrary later one).

**Execution note — read before running this task:** pushing this tag triggers four real GitHub Actions workflow runs on the public `aivyx` repo, publishes a real public GitHub Release, and pushes a real image to GHCR. This is **not** a code change and should **not** be delegated to an autonomous subagent — the controller running this plan should execute the tag push directly and get an explicit go-ahead from the user immediately before running `git push origin v0.9.1`, the same way Phase 192's repo-visibility change was handled.

- [ ] **Step 1: Confirm the local commit is what will be tagged**

Run: `git log -1 --format='%H %s'`

Expected: the Task 1 commit (`chore(release): bump workspace version to 0.9.1 ...`) is `HEAD`.

- [ ] **Step 2: Confirm `v0.9.1` isn't already taken**

Run: `git ls-remote --tags origin | grep 'v0.9.1'`

Expected: no output (empty). If a `v0.9.1` tag already exists, stop and report it — do not overwrite an existing tag.

- [ ] **Step 3: Get explicit go-ahead, then create and push the tag**

After confirming with the user this specific action should proceed now, run:

```bash
git tag v0.9.1
git push origin v0.9.1
```

- [ ] **Step 4: Find the four triggered workflow runs**

Run: `gh run list --limit 10 --json databaseId,workflowName,status,headBranch,createdAt --jq '.[] | select(.headBranch=="v0.9.1")'`

Expected: four JSON objects, one each for `Release`, `desktop-release`, `Docker Publish`, `WSL Distribution` (workflow display names may differ slightly from filenames — match whatever `gh run list --workflow=release.yml`, `--workflow=desktop-release.yml`, `--workflow=docker-publish.yml`, `--workflow=wsl-release.yml` each report for `headBranch v0.9.1`), all with `status` initially `queued` or `in_progress`. Record each run's `databaseId` for the next step.

- [ ] **Step 5: Watch each run to completion**

For each of the four `databaseId`s from Step 4, run:

```bash
gh run watch <databaseId> --exit-status
```

Historical successful runs took 11–25 minutes each; a single Bash tool call is capped at 10 minutes. If a `gh run watch` call hits that local timeout before the run finishes, the real GitHub Actions run is unaffected (it keeps running server-side) — simply re-run the same `gh run watch <databaseId> --exit-status` command again; it resumes polling the same run. Repeat until each of the four returns a real exit code. Consider running the four watches as separate background processes (e.g. `run_in_background: true` in this harness, or `&` + `wait` in a plain shell) rather than waiting on them fully sequentially, since they run concurrently on GitHub's side.

Expected: all four exit `0` (success). If any exits non-zero, run `gh run view <databaseId> --log-failed` on the failing one and stop — do not proceed to Step 6 with a failed release.

- [ ] **Step 6: Verify the real GitHub Release exists with real assets**

Run: `gh release view v0.9.1 --json name,tagName,isDraft,isPrerelease,assets --jq '{name, tagName, isDraft, isPrerelease, assetNames: [.assets[].name]}'`

Expected: `isDraft: false`, `tagName: "v0.9.1"`, and `assetNames` including entries for each of the 4 build targets from `dist-workspace.toml` (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `x86_64-apple-darwin`, `aarch64-apple-darwin`), a shell installer script, `.deb`/`.app` bundle names (from `desktop-release.yml`), and both `Aivyx.wsl` and `aivyx-wsl-rootfs-x86_64-v0.9.1.tar.gz` (from `wsl-release.yml`).

- [ ] **Step 7: Verify the real Docker image is pullable**

Run: `docker manifest inspect ghcr.io/aivyx-agent/aivyx:0.9.1 > /dev/null && echo "GHCR image exists and is pullable"`

Expected: `GHCR image exists and is pullable`. (The tag is `0.9.1`, without the `v` — `docker-publish.yml`'s `type=semver,pattern={{version}}` metadata rule strips it.)

- [ ] **Step 8: Verify `gh release list` now shows this as Latest**

Run: `gh release list --limit 3`

Expected: `v0.9.1` is the top row, marked `Latest`.

There is no commit for this task — it changes no repo-local files. Record the outcome (all four workflows succeeded, release/image/assets all verified) in the task report for the reviewer.

---

### Task 3: Prove build-from-source works for a real, credential-less outsider

**Files:** None — this task builds inside a throwaway Docker container; no repo-local files change.

**Interfaces:**
- Consumes: the `v0.9.1` tag from Task 2 (must exist and have a real, successful release before this task runs).
- Produces: a captured build log and a verified `aivyx --version` output, proving the exact sequence `docs/INSTALL.md`'s "Build from source" section documents works for someone with zero special access — the first time this specific claim will have been verified rather than assumed.

This runs as a sequence of plain, non-interactive shell commands (no interactive TTY session) — start a detached container that just idles, then run each step against it with `docker exec`, so every step is a normal, individually-inspectable command.

- [ ] **Step 1: Start a clean, detached container with no host credentials mounted**

```bash
docker run -d --name aivyx-build-verify rust:1.85-slim sleep infinity
```

Do not add `-v`, `--mount`, or any flag that shares host files, SSH agent, or environment variables — a bare `docker run` with a fresh public image has none of those by default, which is the point. Confirm this directly before doing anything else (git isn't installed yet at this point — Step 2 installs it — so check for credential *sources* via the filesystem and environment rather than running `git` itself):

```bash
docker exec aivyx-build-verify bash -c '
  test -f ~/.gitconfig && echo "FOUND host gitconfig (unexpected)" || echo "no ~/.gitconfig (expected)"
  test -f /etc/gitconfig && echo "FOUND system gitconfig (unexpected)" || echo "no /etc/gitconfig (expected)"
  env | grep -iE "GIT|GH_TOKEN|GITHUB_TOKEN|SSH_AUTH_SOCK" || echo "no git/github/ssh env vars (expected)"
'
```

Expected: all three checks report the "(expected)" branch — no inherited gitconfig, no injected token or SSH agent socket.

- [ ] **Step 2: Install the documented prerequisites**

```bash
docker exec aivyx-build-verify bash -c 'apt-get update && apt-get install -y --no-install-recommends git gcc'
```

(`rust:1.85-slim` already provides the Rust 1.85+ toolchain per `docs/INSTALL.md`'s stated prerequisite; `gcc` provides the C linker prerequisite; `musl-tools` is not needed since the documented default sequence below builds for the container's native `x86_64-unknown-linux-gnu` target, not musl.)

- [ ] **Step 3: Run the exact documented build-from-source sequence, pinned to the v0.9.1 tag**

The literal sequence in `docs/INSTALL.md` clones the default branch; to verify the exact tagged release rather than whatever `main` has drifted to since, clone and then check out the tag explicitly. Everything here runs as one `docker exec` so `cd` state carries between commands (each separate `docker exec` call otherwise starts fresh in the container's default working directory):

```bash
docker exec aivyx-build-verify bash -c '
  git clone https://github.com/Aivyx-Agent/aivyx
  cd aivyx
  git checkout v0.9.1
  cargo build --release --bin aivyx 2>&1 | tee /tmp/build.log
  echo "build exit code: ${PIPESTATUS[0]}"
'
```

(`${PIPESTATUS[0]}`, not `$?` — this is a `bash -c` script, and `$?` after a `| tee` pipeline would report `tee`'s exit status, not `cargo build`'s, silently masking a real build failure as success.)

Expected: the clone succeeds anonymously (this is what Phase 192 fixed — if it fails here with an authentication error against `aivyx-confine`/`aivyx-checkpoint`/`aivyx-kvcache`, Phase 192's fix did not actually hold and this task should stop and report it), the checkout succeeds, and the output ends with `build exit code: 0` and a line matching `Finished \`release\` profile`.

- [ ] **Step 4: Confirm the built binary actually runs**

```bash
docker exec -w /aivyx aivyx-build-verify ./target/release/aivyx --version
```

Expected: output containing `0.9.1` (the version this binary was built from).

- [ ] **Step 5: Capture the evidence, then discard the container**

```bash
docker cp aivyx-build-verify:/tmp/build.log .superpowers/sdd/task-3-build-from-source.log
docker rm -f aivyx-build-verify
```

Removing the container is deliberate — nothing about this verification should persist or be reused; a later run should start from an equally clean state.

- [ ] **Step 6: Record the result**

No commit for this task (nothing in the repo changed). In the task report, include: the full clone-to-run transcript or a reference to the saved `task-3-build-from-source.log`, the exact `aivyx --version` output from Step 4, and explicit confirmation that Step 1's credential check showed no inherited git config.

---

## Self-review notes (for whoever executes this plan)

- **Spec coverage:** the chapter design's Phase N.2 ("cut and verify a real release... all four workflows actually go green end-to-end") is Task 2; Phase N.3 ("prove build-from-source works... deliberately stripped of any cached credentials") is Task 3. Task 1 is the version-bump precondition the design spec didn't call out explicitly but that grounding against the real `release.yml`/`dist-workspace.toml` found is required before Task 2 can produce a real, distinct release.
- **No placeholders:** every step has literal commands and exact expected output, including the real image name (`ghcr.io/aivyx-agent/aivyx`) and asset naming derived directly from the workflow files, not guessed.
- **Type/interface consistency:** Task 2 and Task 3 both anchor on the literal tag `v0.9.1`; Task 3 explicitly checks out that tag rather than trusting `main`, so it verifies the exact commit Task 2's release was cut from.

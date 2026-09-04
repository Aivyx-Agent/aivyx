# Chapter N — Release & Distribution Integrity — Design

> **For agentic workers:** this is a chapter-level design covering four
> phases. Each phase gets its own implementation plan via
> superpowers:writing-plans when it's picked up — this document is the
> shared grounding all four plans draw from, not itself a task list.

## Goal

An unaffiliated outside end user can get a current, working `aivyx`
binary through every path `docs/INSTALL.md` documents — shell installer,
build from source, desktop app, Docker appliance, WSL distro — and the
release pipeline cannot silently regress into shipping stale or broken
artifacts again without being caught immediately, not discovered weeks
later by a human asking "does this actually work."

## How this was found

Investigating "how would an end user deploy this on their own bare
metal" (this session, 2026-09-05) surfaced that `v0.9.0` — the version
`README.md`'s own "Status" line names as current — was never actually
published as a real GitHub Release. `gh release list` shows `v0.8.3`
(2026-07-08) as the true "Latest," over 7 weeks stale relative to `main`.

Root cause, confirmed against real GitHub/CI state, not documentation
claims:

- `Cargo.toml`'s `[workspace.dependencies]` table declares three git
  dependencies, each pinned by commit SHA, each pointing at a private
  `Aivyx-Agent` repo: `aivyx-confine`, `aivyx-checkpoint`, `aivyx-kvcache`.
  All three were adopted from the sibling `aivyx-coder` repo rather than
  reimplemented, per their own Cargo.toml comments.
- All three are wired into `aivyx-core` as non-optional dependencies —
  the crate whose own Cargo.toml comment calls it "a dependency of every
  shipped binary, including the two macOS targets."
- `gh repo view --json isPrivate` confirms all three are private. A CI
  runner's ambient `GITHUB_TOKEN` is scoped only to the repo it's
  executing in and cannot authenticate against a different, private repo.
- `v0.9.0`'s tag push triggered all four release workflows
  (`release.yml`, `desktop-release.yml`, `docker-publish.yml`,
  `wsl-release.yml`); all four completed with status `failure`, all
  dramatically faster than any prior successful run (1m50s–11m40s vs.
  11–24 minutes historically) — consistent with every one of them
  failing at the same early dependency-resolution step rather than
  reaching their real cross-compile/package/publish work.
- `gh run view <release-run-id> --log-failed` shows the literal error:
  `failed to authenticate when downloading repository` while resolving
  `aivyx-checkpoint`, with the underlying "revision not found" message
  being a downstream symptom of the failed clone (the SHA itself is
  real and fetchable with authenticated access — confirmed directly via
  `gh api repos/Aivyx-Agent/aivyx-checkpoint/commits/<sha>`).

The same failure almost certainly blocks the documented "build from
source" path for a genuine outside contributor — this session's own
successful local builds throughout are not evidence otherwise, since
this machine already has cached, authenticated access to all three
private repos.

`aivyx-recall` (a fourth private `Aivyx-Agent` repo, also adopted by
`aivyx-coder`) is confirmed **not** referenced anywhere in `aivyx`'s own
`Cargo.toml` files — out of scope, not part of this problem.

## Decisions made during design

- **The three repos will be made public**, not kept private behind a
  CI-only credential. Confirmed with the operator: their privacy was
  never intentional — they're infrastructure utilities (process
  confinement, git-ref checkpoint/rollback, KV-cache persistence), not
  business logic or secrets, adopted from `aivyx-coder` and never
  revisited. Making them public fixes CI, the release pipeline, AND
  build-from-source for real outside users in one move, with no ongoing
  credential to provision or rotate. A CI-only PAT was considered and
  rejected — it would fix the pipeline but leave "build from source"
  permanently broken for anyone without repo access, which is precisely
  the failure this chapter exists to close.
- **Regression guard: an anonymous-clone probe, not a GitHub-API
  visibility check.** A CI step that extracts every
  `git = "https://github.com/..."` URL from the workspace `Cargo.toml`
  and runs `git ls-remote <url>` against each with no credentials
  configured, placed early in `quality-gate.yml` so a future regression
  fails in seconds with a specific message, not ~20 minutes in via an
  opaque `cargo clippy` backtrace. Chosen over a `gh repo view
  --json isPrivate` check because it tests the literal property that
  broke ("can this be fetched with zero special access") rather than a
  proxy for it, needs no token, and isn't GitHub-specific.
- **Build-from-source verification uses a throwaway Docker container**
  with no mounted host git config, no forwarded SSH agent, no injected
  credentials of any kind — stricter than trusting the GitHub Actions
  CI run from Phase N.2, since a GitHub-hosted runner still sits inside
  org network/token context in subtle ways even when nothing is
  deliberately shared. The container runs the exact sequence
  `docs/INSTALL.md` documents, against the release tag cut in Phase
  N.2, and the evidence captured is the full build log plus a working
  `aivyx --version` — not just a green exit code.

## Phases

### Phase N.1 — Unblock the pipeline

Make `aivyx-confine`, `aivyx-checkpoint`, `aivyx-kvcache` public on
GitHub. Confirm — not assume — each is now anonymously cloneable
(`git ls-remote` with no credentials configured, from an environment
that has no cached access). Update the three dependency comments in the
root `Cargo.toml` (each currently says "Not on crates.io" and explains
the private-repo pinned-SHA pattern) if the reasoning they document no
longer matches reality post-fix. This phase is the root-cause fix;
Phases N.2–N.4 all depend on it.

### Phase N.2 — Cut and verify a real release

Tag a new version (`v0.9.1`, since `v0.9.0`'s tag already exists and
never produced a real release) and push it. Watch all four release
workflows run to completion:

- `release.yml` — cargo-dist cross-compiles Linux x86_64/aarch64 musl +
  macOS x86_64/aarch64, publishes the real GitHub Release object.
- `desktop-release.yml` — builds the `.deb`/`.app` bundles.
- `docker-publish.yml` — builds and pushes the server-appliance image to
  GHCR.
- `wsl-release.yml` — exports `Aivyx.wsl` from the Docker appliance
  image.

Success criteria: all four show `completed`/`success` in
`gh run list`, `gh release list` shows the new tag as "Latest," and the
shell installer one-liner from `docs/INSTALL.md` actually installs it.

### Phase N.3 — Prove build-from-source works for a real outsider

In a throwaway Docker container (fresh base image matching
`docs/INSTALL.md`'s stated prerequisites: Rust 1.85+, a C linker,
`musl-tools` for the Linux-musl target; no volume mounts, no forwarded
SSH agent, no injected git/GitHub credentials of any kind), run the
exact sequence `docs/INSTALL.md`'s "Build from source" section
documents, against the tag cut in Phase N.2:

```sh
git clone https://github.com/Aivyx-Agent/aivyx
cd aivyx
cargo build --release --bin aivyx
```

Capture the full build log and confirm both a zero exit code and a
working `./target/release/aivyx --version`. This is the first time this
path will have been verified against a genuinely credential-less
environment rather than assumed to work because it works on a
maintainer's machine.

### Phase N.4 — Regression guard + doc correction

- Add the anonymous-clone probe (see "Decisions made during design")
  to `quality-gate.yml`, early enough that a future regression fails
  fast with a specific message.
- Correct `README.md`'s "Release pipeline status" section, which
  currently claims `v0.9.0` is the active/latest release (it wasn't —
  see "How this was found" above) — update to reflect whatever the
  real latest release is after Phase N.2.
- Correct `docs/INSTALL.md`'s "Current install state" section, which is
  separately stale in the *opposite* direction — it still says
  "`v0.1.0` (pre-release)... shell-installer URL is live only once the
  `v0.1.0` tag has been pushed," when the pipeline has in fact been
  real and producing installable releases since well before `v0.8.0`.

## Out of scope

- `aivyx-recall` — not referenced by `aivyx`'s build; no action needed.
- Any change to what `aivyx-confine`/`aivyx-checkpoint`/`aivyx-kvcache`
  actually do — this chapter only changes their GitHub visibility and
  fixes how `aivyx` depends on them, not their internal implementation.
- Publishing any of the three to crates.io — making them public on
  GitHub is sufficient to fix anonymous cloning; a crates.io release is
  a separate, independent improvement not required to close this gap.
- Re-auditing `aivyx-coder`'s own build (the sibling repo that also
  depends on these three) — likely benefits from the same fix
  incidentally, but verifying that is that repo's own concern, not this
  chapter's.

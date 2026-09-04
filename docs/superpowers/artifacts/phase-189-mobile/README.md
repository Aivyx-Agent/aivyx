# Phase 189 mobile-verification environment runbook

If the dev daemon isn't reachable at `http://127.0.0.1:7843`, bring it back
up with these steps (see `docs/superpowers/plans/2026-09-04-studio-mobile-verification.md`
Task 1 for full detail and expected output at each step):

1. Start `llama-server` on the rig (if not already running):
   `ssh julian@10.80.80.148 'nohup /usr/bin/llama-server --model /home/julian/models/Qwen3.5-9B-Q4_K_M.gguf --host 127.0.0.1 --port 8080 --parallel 1 --ctx-size 8192 -ngl 99 > /tmp/llama-server-phase189.log 2>&1 < /dev/null &'`
2. Tunnel it locally (background): `ssh -N -L 8080:127.0.0.1:8080 julian@10.80.80.148`
3. Rebuild if CSS/markup changed: `just build-web && cargo build --bin aivyx`
   (needs `dx` and `just` on `PATH`: `export PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH"`
   — both `dx` and `just` actually resolve from `~/.cargo/bin` on this
   machine, not the rustup toolchain dir; `just` wasn't preinstalled here and
   was added via `cargo install just --locked`.
   **Run this from the main checkout (`/home/julian/Projects/Rust/aivyx`),
   not from this worktree** — `wasm-bindgen` bakes the build's absolute
   output path into the emitted `.js`, and prior `dist/` commits (and this
   one) all embed the main checkout's path. Building from the worktree
   instead embeds `.../.claude/worktrees/studio-mobile-verification/...`,
   which is dead code at runtime but a build-hygiene defect and a diff
   noise generator. After building in the main checkout, `rsync -a --delete`
   the resulting `crates/aivyx-web/dist/` into this worktree's
   `crates/aivyx-web/dist/` before committing here.)
4. Launch the daemon (background, from `.dev-run-189/`):
   ```
   mkdir -p .dev-run-189/sandbox
   cd .dev-run-189
   export AIVYX_PROVIDER=llamacpp AIVYX_OPENAI_BASE_URL=http://127.0.0.1:8080 AIVYX_MODEL=qwen3.5-9b-q4_k_m AIVYX_FS_ROOT="$PWD/sandbox" AIVYX_STORAGE_PATH="$PWD/store.redb" AIVYX_PASSPHRASE=aivyx-dev-throwaway
   ../target/debug/aivyx daemon run --web-ui
   ```
   (use `export ... ; command`, not `env VAR=... command` — the latter form
   is blocked by this environment's sandbox for backgrounded commands.
   The `mkdir -p .dev-run-189/sandbox` + `cd .dev-run-189` step is required
   before launching — `AIVYX_FS_ROOT`/`AIVYX_STORAGE_PATH` above are
   relative to that directory via `$PWD`, and the daemon needs `sandbox/`
   to already exist.)
5. Verify: `curl -fsS http://127.0.0.1:7843/ | head -c 200` should print HTML.

`.dev-run-189/` is disposable scratch state (git-ignored), same convention
as `.dev-run/` from `scripts/dev-run.sh` — safe to `rm -rf` and restart from
Step 4 if the store gets into a bad state.

## Notes from the initial stand-up (2026-09-04)

- `just` was not installed on this machine; it was added with
  `cargo install just --locked` (installs to `~/.cargo/bin`, same place
  `dx` already lived). Both need `~/.cargo/bin` on `PATH`.
- The smoke-test screenshot command in the plan hardcodes the screenshot's
  output path as `/home/julian/Projects/Rust/aivyx/docs/superpowers/artifacts/phase-189-mobile/_smoke-command-1280.png`,
  which is the **main** checkout, not this worktree
  (`.../aivyx/.claude/worktrees/studio-mobile-verification/...`). Since
  Task 1's commit happens from the worktree, the file was moved into the
  worktree's `docs/superpowers/artifacts/phase-189-mobile/` after capture.
  Later tasks running Playwright screenshots from this worktree should
  point `page.screenshot({ path: ... })` at the worktree's own
  `docs/superpowers/artifacts/phase-189-mobile/` directly rather than the
  main checkout's copy.
- An earlier draft of this runbook had `just build-web` run from this
  worktree, which changed nothing semantically but re-embedded a
  worktree-specific absolute path into the built `.js` (wasm-bindgen bakes
  the output dir path in) — a build-hygiene defect, not a functional break,
  but it made `dist/`'s diff noisy and didn't match the canonical path
  prior phases' `dist/` commits use. Fixed by rebuilding from the main
  checkout instead (see Step 3 above) — confirmed the rebuilt files are
  now byte-identical to what was already committed on this branch before
  Task 1 touched `dist/` at all, so no `dist/` change ended up being
  necessary for Task 1's own commit.

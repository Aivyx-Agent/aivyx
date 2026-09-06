# Phase 202 — First-Launch Store Safety

**First-launch audit follow-through — [SHIPPED] 2026-09-06.**

## Goal (carried from a direct code audit, not a prior phase's own follow-up)

A thorough audit of what happens when an End User launches Aivyx for the
first time — traced directly against real code, not inferred from docs —
found a genuine bug chain: running bare `aivyx` before ever running
`aivyx init` on a fresh machine silently creates a permanent, unconfirmed
encrypted store (real `mkdir`s, an unconfirmed interactive passphrase
prompt, a real `RedbStorage::open`) *before* config validation ever runs,
and a later, real `aivyx init` run can then write a config pointing at
that same orphaned store under a different passphrase, producing an
opaque, undiagnosable decrypt failure with no indication of the real
cause. Phase 202 was scoped to close this.

## What shipped

- **Confirmed via direct code tracing that the documented, README-
  recommended paths (shell installer → `aivyx init`; the manual
  quickstart's hand-written TOML, which pre-supplies a passphrase) never
  hit this bug** — it specifically triggers for a user who runs bare
  `aivyx` first, skipping the documented next step. Also confirmed
  Docker/systemd deployments always bake an `aivyx.toml` even when using
  env overrides, so "no `aivyx.toml` exists" is a safe, low-false-positive
  signal rather than one that would misfire on a legitimate env-only
  production setup.
- **An early-validate gate in `run()`** (`crates/aivyx-cli/src/bin/aivyx.rs`):
  when the target store doesn't exist yet, `config.validate()` now runs
  *before* any directory creation, passphrase prompt, or `RedbStorage::open`
  — not after, as it did before. If validation fails, the operator gets
  an interactive offer to run the setup wizard inline (TTY) or a clean,
  immediate failure naming `aivyx init` (non-TTY) — zero side effects
  either way. When a store already exists, this new check is skipped
  entirely and the pre-existing mkdir → passphrase → open →
  hydrate-secrets-from-store → validate order runs completely unchanged,
  preserving the store-can-supply-a-secret fallback every returning
  install relies on.
- **A store-collision guard in `aivyx init`**: before writing a config,
  the wizard now warns and requires explicit confirmation if a store
  already exists at the target storage path — mirroring the wizard's
  own existing `aivyx.toml`-overwrite guard exactly.
- **Confirm-reentry on the first-time interactive passphrase prompt**,
  scoped to new-store creation only: `PassphraseSource::InteractivePrompt`
  gained a `confirm: bool` field. Creating a brand-new store now prompts
  twice and requires a match (mirroring `aivyx keyring set`'s existing
  shape); unlocking an existing store is completely unchanged — one
  prompt, no confirmation, since a wrong guess there just fails cleanly
  and the user retries.
- **A real, unavoidable cross-task compile dependency surfaced mid-branch**:
  the passphrase-enum change broke `aivyx-cli`'s compilation transitively
  (a second, independent task in a different crate needed it to compile
  just to test its own unrelated changes). Fixed with a one-line,
  behaviorally-neutral placeholder that the branch's own later work
  cleanly replaced with the real threaded value — no design change, pure
  build-ordering reality of a Cargo workspace.
- **Two design corrections were caught and fixed during planning itself**,
  before any code was written: a test design that assumed non-existent
  "full wizard" tests to mirror (`run_init_wizard_inner` hardcodes real
  stdin/stderr internally and has no such tests anywhere in the file) was
  fixed by extracting a pure `decide_storage_collision` function instead,
  matching the file's own established `decide_service_install` pattern;
  and the confirm-reentry function was designed around a single
  `FnMut(&str)` closure rather than two separate closures specifically to
  avoid a real borrow-checker conflict (two closures capturing the same
  test reader/writer can't coexist as separate function arguments).
- **Task 3's own review caught a real, reproducible bug by actually
  building and running the binary**, not just reading the diff: the gate
  printed the same error+hint text to stderr twice on the most common
  real-world trigger (a fresh, unconfigured install run non-interactively)
  — the gate's own `eprintln!` plus the same text re-embedded in the
  returned `Err`, which `main()`'s generic handler printed again. Fixed
  with a one-line change before the branch ever reached final review.
- **The final whole-branch review (Opus) went further than reading the
  diff — it built and ran the real binary for both code paths.** It
  confirmed via `find` that zero files are created on the new-gate
  failure path, and confirmed (by creating a real store and re-running)
  that the existing-store path is byte-for-byte behaviorally identical to
  before this branch — the single regression guarantee the whole phase
  rests on. It independently judged a TOCTOU concern an earlier task
  review had flagged and confirmed the design is self-correcting in both
  directions regardless of which way a hypothetical race resolves (not a
  flaw). It confirmed no second store-open bypass path exists anywhere in
  the workspace — `derive_master_key` has exactly one production call
  site in the entire codebase, unlike the multi-site landmine Phase 200
  found in its own domain.
- **The review found 3 Important + 1 Minor issue, all fixed**:
  - `read_interactive_password_with_confirm` (this phase's own new
    function) dropped both passphrase copies unzeroized on every call and
    every mismatch retry, contradicting the module's own documented
    zeroize invariant. Fixed to zeroize both copies on every path except
    the one real return value.
  - `aivyx init`'s `default_paths()` hardcoded `$HOME/.local/share/aivyx`,
    ignoring `$XDG_DATA_HOME` — a real, **pre-existing** divergence from
    `aivyx-config`'s own loader resolution that this phase's own approved
    design spec incorrectly claimed didn't exist ("resolve identically").
    On a machine with `$XDG_DATA_HOME` set, the new store-collision guard
    would check the wrong path and silently miss an orphaned store living
    at the XDG path. Fixed to mirror the loader's resolution exactly.
  - Two stale `#[allow(dead_code)]` attributes, left over from the point
    mid-branch where the decision function existed but had no caller yet
    — verified empirically that removing them still compiles and lints
    clean.
  - A missing `CHANGELOG.md` entry for this phase's operator-visible
    behavior changes — added.
- Full `cargo test` (120 result blocks across default-members, 0
  failures) and `cargo clippy --all-targets -- -D warnings` (clean)
  independently re-verified by the controller after the final-review fix
  and again on merged `main`.

## The result

A genuinely first-time user who runs bare `aivyx` before `aivyx init` on a
fresh machine no longer has a permanent, unconfirmed encrypted store
created out from under them — they get a clean nudge toward `aivyx init`
instead, with zero side effects on the failure path. `aivyx init` itself
now protects against writing a config into an existing orphaned store, and
the one interactive passphrase entry point that creates a brand-new store
now has the same confirm-reentry safety net every other passphrase-setting
path in the product already had.

## Known follow-ups (not done here, logged for whenever they matter)

- **`aivyx --verify-only` / `aivyx audit export` / `aivyx cost` are
  deliberately excluded from the early-validate gate** and can still
  create a real, permanent store on an otherwise completely unconfigured
  machine (confirmed empirically by the final review: a fresh
  `--verify-only` invocation with no API key configured anywhere created
  a real store and exited 0). This is intentional per the design (those
  modes never require an API key), and Task 1's confirm-reentry safety
  net still applies regardless of mode — but it's a real, narrower version
  of the same "a store gets created before the operator meant to set one
  up" shape this whole phase exists to close. Worth an explicit
  accept-or-close decision at some point rather than staying an
  implicit gap.
- **A doubled `aivyx:`-prefixed line remains on the non-interactive
  failure path** (the gate's own context line, then `main()`'s generic
  `"aivyx: setup required — see above"`) — cosmetic only, the duplicate
  *content* bug is fixed, but two separately-prefixed lines still read a
  little oddly.
- **The passphrase-mismatch retry message
  (`"Passphrases didn't match. Try again."`) bypasses the injected writer
  seam** (`eprintln!` directly, rather than through the same `writer`
  parameter `decide_storage_collision`'s and `prompt_yes_no`'s equivalent
  messages already go through) — makes it untestable and prints raw
  during `cargo test`. Not worth restructuring the closure signature for
  on its own; noted for whenever this file's I/O seams get revisited.

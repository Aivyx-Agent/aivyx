# Phase 187 — Service Install + First-Run Launch

**Chapter I, phase 3 — [SHIPPED] 2026-09-03.**

## Goal (carried from the roadmap entry)

`aivyx service install` generates a systemd/launchd unit (run at
login/boot); the `init` wizard ends by offering to install + launch.
Turnkey "install once, always there." No new dependency.

## Where this actually stood going in

Grounding against the real code before scoping found this phase was
already ~90% shipped. The systemd/launchd unit-generation half wasn't
Phase 187 work at all — it was **Chapter Anchor**, which shipped in
`v0.7.6` (2026-06-28), over two months before Chapter I's own roadmap
entry existed. `aivyx daemon install`/`uninstall`
(`crates/aivyx-cli/src/bin/aivyx_modules/daemon_service.rs`) was already
complete and production-grade: a real systemd **user** unit + `loginctl
enable-linger` on Linux (no root), a real launchd `LaunchAgent` on
macOS, idempotent re-install, unattended-passphrase capture kept
owner-only at rest, a `--web-ui` flag, a `--no-start` flag, `aivyx
doctor` integration, and full documentation. This is the same class of
gap Phase 186 hit — a chapter shipping without the numbered phase
sequence's own forward-looking doc ever being told about it.

The genuinely missing piece was narrow: `aivyx init`'s wizard only
*printed* the `aivyx daemon install` command as a "next steps" hint —
nothing offered to run it. Full design reasoning:
`docs/superpowers/specs/2026-09-03-init-wizard-service-offer-design.md`.

## What shipped

- **`ServiceInstallDecision` + `decide_service_install` +
  `offer_service_install`** (`crates/aivyx-cli/src/bin/aivyx_modules/
  init.rs`) — a pure decision function (no OS calls, fully unit-testable
  with the same scripted-`Cursor` idiom this file's `prompt_yes_no`
  tests already used) wrapped by a function that takes the actual
  install call as an injected closure, so its success/failure print
  paths are also unit-testable without ever touching real systemd/
  launchd. Wired into the wizard's existing ending, in the same list
  position the old passive hint occupied.
- **Skips the offer entirely if a service is already installed** — no
  re-nagging an operator who re-runs `aivyx init`.
- **Real, live verification beyond the plan's own required bar**: rather
  than trusting only the mocked unit tests, the controller took the
  branch to a real host with systemd (the GPU test rig used earlier this
  session for the `aivyx-kvcache` fix) and ran the new code path against
  the REAL, non-mocked `daemon_service::run_install` — twice. The first
  run (no controlling TTY, no `AIVYX_PASSPHRASE`) hit a genuine failure
  in `run_install`'s own passphrase-capture step and confirmed the new
  code's error handling prints cleanly and never propagates — proving
  the "never fail the wizard" constraint against a real failure, not
  just a mocked one. The second run succeeded fully: a genuine systemd
  user unit was created, enabled for boot via a real symlink, an env
  file written, the service started, and `aivyx doctor`'s own
  pre-existing "Service:" section correctly reported it — independently
  confirming the design's choice to point the success message at
  `aivyx doctor` rather than duplicating unit-path/check-command text
  inline. Everything was fully uninstalled and cleaned up afterward.
- **A second real gap found by the final review, invisible to both the
  mocked tests and both real-host runs**: `run_install`'s own
  `resolve_passphrase()` silently prompts for a NEW, permanent, no-echo
  store passphrase whenever `AIVYX_PASSPHRASE` is unset — a genuine
  third interactive prompt the design spec, the plan, all 9 original
  tests, and *both* real-host verification runs structurally missed
  (one had no TTY so the prompt errored before firing; the other had
  `AIVYX_PASSPHRASE` set so it was skipped). Fixed with a one-line
  lead-in printed before the prompt fires, explaining what's coming and
  that `AIVYX_PASSPHRASE` skips it.
- **A third real gap**: `install_linux`'s non-atomic unit+env-file
  writes (no rollback on failure) can leave a partial install that the
  wizard's own `already_installed` check would silently mistake for a
  successful one on the next `aivyx init` run. Fixed by mentioning
  `aivyx daemon uninstall` as the recovery path in the failure message.

## The result

Chapter I's third phase closes with the "turnkey install once, always
there" promise now literally happening inside the wizard, not requiring
a manual follow-up command — while the actual unit-generation machinery
underneath it (Chapter Anchor) stayed untouched, since it was already
solid. Two more genuine, non-obvious bugs surfaced and were fixed inside
this one phase, both caught only by the final whole-branch review after
per-task review AND real-host verification had both passed clean —
reinforcing, a fifth time this session, that a final review on the most
capable model finds real issues a task-by-task process structurally
can't see.

## Known follow-ups (not done here, logged for whenever they matter)

- **`docs/INSTALL.md` doesn't yet mention the wizard's new offer.** The
  first-run checklist and the "Running as a service" section both still
  describe only the standalone `aivyx daemon install` command. Cheap,
  deferred to a documentation pass rather than blocking this phase.
- **The `aivyx daemon run --web-ui` "next steps" line can go stale on
  the happy path.** It's printed before the service-install offer runs;
  if the operator says yes, running that command manually afterward
  would collide with the daemon the offer just started on the same IPC
  socket. Minor, not fixed here.
- **The success and failure branches' output visually interrupts the
  "Next steps:" list** — `run_install`'s own multi-line confirmation
  block (and, on failure, the two-line error message) prints mid-list
  rather than as a single list item, unlike the unchanged `Declined`
  branch. Cosmetic, deferred.
- **Ctrl-D at the new offer prompt reads as "yes"** (pre-existing
  wizard-wide EOF-defaults-to-default-answer semantics, not new to this
  phase) — degrades safely (an empty passphrase is then rejected by
  `resolve_passphrase`, printing an error and continuing), so this was
  deliberately left unchanged rather than special-cased.

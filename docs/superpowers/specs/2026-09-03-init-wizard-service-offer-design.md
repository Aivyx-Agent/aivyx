# Phase 187 — init wizard service-install offer — design

**Status:** Approved, ready for planning.

## Motivation

`docs/ROADMAP.md`'s Chapter I "Expected phases" list names Phase 187 as:
"`aivyx service install` generates a systemd/launchd unit (run at
login/boot); the `init` wizard ends by offering to install + launch.
Turnkey 'install once, always there.' No new dependency."

**Grounding against the real code found this phase is overwhelmingly
already shipped.** The systemd/launchd unit-generation half isn't a
Phase 187 deliverable at all — it's **Chapter Anchor**, which shipped in
`v0.7.6` (2026-06-28), over two months before Chapter I's own roadmap
entry existed. `aivyx daemon install`/`uninstall`
(`crates/aivyx-cli/src/bin/aivyx_modules/daemon_service.rs`) is complete
and production-grade: a real systemd **user** unit + `loginctl
enable-linger` on Linux (no root), a real launchd `LaunchAgent` on macOS,
idempotent re-install, unattended-passphrase capture kept owner-only at
rest, a `--web-ui` flag, a `--no-start` flag, `aivyx doctor` integration,
and full documentation (`docs/INSTALL.md`'s "Running as a service"
section). This is exactly the same class of gap Phase 186 hit — a later
chapter shipping without the phase-sequence's own forward-looking doc
ever being told about it.

**The genuinely missing half is much narrower than the roadmap entry
implies.** Direct inspection of `crates/aivyx-cli/src/bin/aivyx_modules/
init.rs`'s wizard ending (the last ~25 lines of `run_init_wizard_inner`,
around line 2213) found it only *prints* `aivyx daemon install` as one
line in a static "Next steps" hint list — there is no interactive offer,
and nothing calls the install logic. The operator has to come back later
and run the command themselves. This is the entire remaining scope of
Phase 187: wire a real yes/no offer into that ending that calls the
already-existing `daemon_service::run_install` directly, so first-run can
genuinely end with a running background service instead of a printed
suggestion.

**No new dependency, matching the roadmap's own stated constraint** — this
is new callers of existing, already-shipped, already-tested install logic,
not new install machinery.

## A. Where this lands

`crates/aivyx-cli/src/bin/aivyx_modules/init.rs`, inside
`run_init_wizard_inner` (an `async fn`), replacing the current
unconditional print of the `aivyx daemon install` hint line
(around line 2220-2226 today):

```rust
    // Chapter Anchor — the runs-for-days path: a real service so the agent keeps
    // running (and its scheduled routines keep firing) across logout + reboot.
    if matches!(crate::daemon_service::Platform::detect(), crate::daemon_service::Platform::Linux | crate::daemon_service::Platform::MacOs) {
        eprintln!(
            "  aivyx daemon install       — run it as a background service (survives logout/reboot)"
        );
    }
```

`run_init_wizard_inner` already bails out at its very top
(`if !io::stdin().is_terminal() { return Err(...) }`) if stdin isn't a
real terminal — so by the time execution reaches this point, an
interactive terminal is already guaranteed. **No separate
non-interactive-stdin branch is needed for the new offer** (an earlier
draft of this design assumed one; grounding against the function's own
top-of-body guard found it unnecessary). `reader`/`writer` (`&mut
reader: BufRead`, `&mut writer: io::Stderr`, both already `mut` locals in
scope since the top of the function) are the same handles every other
`prompt_yes_no` call in this wizard already uses.

## B. The offer flow

Only on Linux/macOS (the existing platform gate, unchanged — Windows and
other platforms keep exactly today's passive behavior, pointing at the
Docker appliance / desktop autostart instead per
`daemon_service::run_install`'s own `Platform::Unsupported` error text).

1. **Check for an existing install first.**
   `crate::daemon_service::installed_unit_path()` returns `Some(path)` if
   the service is already installed on this host. If so, **skip the
   offer entirely** — print a short status line instead (using
   `crate::daemon_service::is_active()` to say whether it's currently
   running), so a re-run of `aivyx init` by an operator who already has
   this set up is never nagged to reinstall.

2. **Otherwise, ask.**
   ```rust
   let install_now = prompt_yes_no(
       "Install as a background service now? (survives logout/reboot)",
       true,
       &mut reader,
       &mut writer,
   )?;
   ```
   Default **`true`** — matches the runs-for-days framing this wizard
   already leans toward at other decision points, and is the whole point
   of the feature.

3. **If yes, a nested follow-up:**
   ```rust
   let with_web_ui = prompt_yes_no(
       "Also serve the Studio web UI?",
       false,
       &mut reader,
       &mut writer,
   )?;
   ```
   Default **`false`** — matches `daemon install`'s own `--web-ui` flag
   being opt-in today, not the default.

4. **Call the existing install function directly** — no new install
   logic:
   ```rust
   match crate::daemon_service::run_install(with_web_ui, true) {
       Ok(()) => { /* see Step 5 */ }
       Err(e) => { /* see Step 6 */ }
   }
   ```
   `run_install(web_ui: bool, start: bool)` is the exact function `aivyx
   daemon install` itself calls (`crates/aivyx-cli/src/bin/
   aivyx_modules/daemon_service.rs:147`) — `start: true` always, since
   the offer's whole premise is "install AND launch," not install-only
   (the wizard has no reason to expose `daemon install`'s `--no-start`
   flag here; an operator who wants that already has the CLI command for
   it).

5. **On success**, print a short confirmation instead of the old passive
   hint — the installed unit/plist path (`installed_unit_path()`, now
   `Some` after a successful install) and the same check-it commands
   `docs/INSTALL.md`'s own "Running as a service" section already
   documents (`systemctl --user status aivyx-daemon` / `launchctl print
   gui/$(id -u)/com.aivyx.daemon`), so the operator has a next step for
   *checking* it, not just having installed it.

6. **On failure** (`run_install` returns `Err(String)`), print the error
   message directly (it's already operator-readable — see
   `run_install`'s own `Platform::Unsupported` text as the existing
   style) and continue. **This must never fail the wizard overall** — by
   this point `aivyx.toml` has already been written successfully; a
   service-install failure is a separate, later concern, not a config
   problem.

7. **If the operator answers no** at step 2, fall back to exactly
   today's existing behavior — print the `aivyx daemon install` hint
   line unchanged. No behavior change for an operator who isn't ready to
   commit to a service yet.

## Testing

- `daemon_service::installed_unit_path`/`is_active`/`run_install` are all
  already tested in their own module — this plan adds no new coverage
  requirement there, only new callers.
- `init.rs`'s own wizard logic is normally exercised via its existing
  scripted-stdin integration-test pattern (feeding `prompt_yes_no`
  answers through a `BufRead` fixture) — the new offer's three outcomes
  (skip-because-already-installed, yes-then-success, yes-then-failure,
  no) should each get one such test, following whatever fixture pattern
  the wizard's existing `prompt_yes_no`-driven tests already use.
- A real end-to-end confirmation (actually running the updated wizard on
  a real Linux/macOS host and confirming a real service gets installed)
  is optional but valuable given this repo's own established lesson from
  today's separate `aivyx-kvcache` finding: a mocked-only test can't see
  everything a real OS-level side effect can — if a GPU-rig-style real
  host is available for this plan's own final verification, use it.

## Out of scope

- Any change to `aivyx daemon install`/`uninstall`/`run_install` itself —
  this plan adds callers, not new install machinery.
- Windows or container support — unchanged, already correctly out of
  `daemon_service`'s own scope (Docker appliance / desktop autostart are
  the documented alternatives there).
- Exposing `--no-start` in the wizard's offer — the offer's premise is
  install-and-launch; an operator who wants install-only already has the
  CLI command.

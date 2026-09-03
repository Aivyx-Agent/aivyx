# Phase 187 — Init Wizard Service-Install Offer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `aivyx init`'s wizard offers to install and launch a background
service right at the end, instead of only printing the command to run
later.

**Architecture:** A new pure decision function (`decide_service_install`)
picks what to do given whether a service is already installed and the
operator's yes/no answers — fully unit-testable with the same
scripted-`Cursor` idiom this file's existing `prompt_yes_no` tests already
use. A second function (`offer_service_install`) wraps it, taking the
actual install call as an injected closure parameter so its success/
failure print paths are also unit-testable without ever touching real
systemd/launchd. `run_init_wizard_inner`'s ending becomes a thin call site
that supplies the real `daemon_service::installed_unit_path`/`is_active`/
`run_install` — no new install logic anywhere.

**Tech Stack:** Rust, the existing `aivyx-cli` binary crate (`aivyx_modules::init`), the existing `aivyx_modules::daemon_service` module (untouched).

## Global Constraints

- No new crate dependencies (this plan adds callers of existing,
  already-shipped install logic — `daemon_service::run_install`/
  `installed_unit_path`/`is_active` are untouched).
- Windows/other platforms keep exactly today's behavior (no offer;
  `daemon_service::run_install` already returns a
  `Platform::Unsupported` error there, and the wizard's own platform
  gate already excludes them from ever reaching this code).
- A failed service install must never fail the wizard overall —
  `aivyx.toml` is already written successfully by this point in
  `run_init_wizard_inner`.
- `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D
  warnings` (no `-p`, matching this repo's own default-members
  convention — `aivyx-cli` is inside default-members) must stay green
  throughout.

---

### Task 1: Pure, testable decision + offer logic

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx_modules/init.rs` — add two new
  functions and one enum, placed directly after `prompt_yes_no`'s own
  definition (around line 513, right after its closing `}`) since they
  build directly on it.
- Test: same file, `mod tests` block (the existing test module starting
  around line 2238's `// Tests` comment) — add tests directly after
  `prompt_yes_no_accepts_yes` (around line 2698).

**Interfaces:**
- Consumes: `prompt_yes_no(prompt: &str, default: bool, reader: &mut dyn
  BufRead, writer: &mut dyn IoWrite) -> Result<bool, String>` (already
  defined in this file, unchanged).
- Produces: `ServiceInstallDecision` enum and `offer_service_install(
  already_installed: bool, active: Option<bool>, reader: &mut dyn
  BufRead, writer: &mut dyn IoWrite, run_install: impl FnOnce(bool, bool)
  -> Result<(), String>) -> Result<(), String>` — Task 2's call site
  consumes this exact signature, passing `crate::daemon_service::
  run_install` as the last argument (its real signature, `fn(bool, bool)
  -> Result<(), String>`, coerces directly to the `impl FnOnce` bound).

- [ ] **Step 1: Write the failing tests**

Add these tests directly after `prompt_yes_no_accepts_yes` (around line
2698, before the `// -- TOML generation --` comment):

```rust
    #[test]
    fn decide_service_install_skips_the_offer_when_already_installed() {
        let mut input = Cursor::new(b"" as &[u8]);
        let mut output = Vec::new();
        let decision =
            decide_service_install(true, Some(true), &mut input, &mut output).unwrap();
        assert!(matches!(
            decision,
            ServiceInstallDecision::AlreadyInstalled { active: Some(true) }
        ));
        // No prompt was printed -- nothing was asked.
        assert!(output.is_empty());
    }

    #[test]
    fn decide_service_install_declined_falls_back() {
        let mut input = Cursor::new(b"n\n" as &[u8]);
        let mut output = Vec::new();
        let decision =
            decide_service_install(false, None, &mut input, &mut output).unwrap();
        assert!(matches!(decision, ServiceInstallDecision::Declined));
    }

    #[test]
    fn decide_service_install_yes_then_no_web_ui() {
        let mut input = Cursor::new(b"y\nn\n" as &[u8]);
        let mut output = Vec::new();
        let decision =
            decide_service_install(false, None, &mut input, &mut output).unwrap();
        assert!(matches!(
            decision,
            ServiceInstallDecision::Install { web_ui: false }
        ));
    }

    #[test]
    fn decide_service_install_yes_then_yes_web_ui() {
        let mut input = Cursor::new(b"y\ny\n" as &[u8]);
        let mut output = Vec::new();
        let decision =
            decide_service_install(false, None, &mut input, &mut output).unwrap();
        assert!(matches!(
            decision,
            ServiceInstallDecision::Install { web_ui: true }
        ));
    }

    #[test]
    fn decide_service_install_defaults_to_yes_on_bare_enter() {
        // Confirms the "install now?" prompt defaults true (bare Enter =
        // yes) -- the second bare Enter then hits the web-ui follow-up,
        // which defaults false.
        let mut input = Cursor::new(b"\n\n" as &[u8]);
        let mut output = Vec::new();
        let decision =
            decide_service_install(false, None, &mut input, &mut output).unwrap();
        assert!(matches!(
            decision,
            ServiceInstallDecision::Install { web_ui: false }
        ));
    }

    #[test]
    fn offer_service_install_prints_status_when_already_installed() {
        let mut input = Cursor::new(b"" as &[u8]);
        let mut output = Vec::new();
        offer_service_install(true, Some(false), &mut input, &mut output, |_, _| {
            panic!("run_install must not be called when already installed")
        })
        .unwrap();
        let out = String::from_utf8(output).unwrap();
        assert!(out.contains("already installed"));
        assert!(out.contains("not currently running"));
    }

    #[test]
    fn offer_service_install_prints_todays_hint_when_declined() {
        let mut input = Cursor::new(b"n\n" as &[u8]);
        let mut output = Vec::new();
        offer_service_install(false, None, &mut input, &mut output, |_, _| {
            panic!("run_install must not be called when declined")
        })
        .unwrap();
        let out = String::from_utf8(output).unwrap();
        assert!(out.contains("aivyx daemon install"));
        assert!(out.contains("survives logout/reboot"));
    }

    #[test]
    fn offer_service_install_calls_run_install_with_the_right_args_on_yes() {
        let mut input = Cursor::new(b"y\ny\n" as &[u8]);
        let mut output = Vec::new();
        let mut captured: Option<(bool, bool)> = None;
        offer_service_install(false, None, &mut input, &mut output, |web_ui, start| {
            captured = Some((web_ui, start));
            Ok(())
        })
        .unwrap();
        assert_eq!(captured, Some((true, true)));
        let out = String::from_utf8(output).unwrap();
        assert!(out.contains("Installed as a background service"));
    }

    #[test]
    fn offer_service_install_prints_the_error_and_does_not_fail_on_install_failure() {
        let mut input = Cursor::new(b"y\nn\n" as &[u8]);
        let mut output = Vec::new();
        let result = offer_service_install(false, None, &mut input, &mut output, |_, _| {
            Err("no supported service manager on this platform".to_string())
        });
        // Must return Ok -- a failed install must never fail the wizard.
        assert!(result.is_ok());
        let out = String::from_utf8(output).unwrap();
        assert!(out.contains("Couldn't install"));
        assert!(out.contains("no supported service manager on this platform"));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-cli decide_service_install`
Expected: FAIL to compile — `decide_service_install`, `ServiceInstallDecision`, and `offer_service_install` don't exist yet.

- [ ] **Step 3: Implement `ServiceInstallDecision`, `decide_service_install`, and `offer_service_install`**

Add directly after `prompt_yes_no`'s closing `}` (the function ending
around line 514):

```rust
/// What the wizard should do about installing a background service,
/// decided from whether one's already installed and (if not) the
/// operator's answers. Pure with respect to OS calls -- doesn't itself
/// call `daemon_service::run_install`/`installed_unit_path`/`is_active`,
/// so it's testable with a scripted reader/writer like every other
/// `prompt_yes_no`-driven decision in this file.
enum ServiceInstallDecision {
    /// Already installed -- nothing to ask. `active` mirrors
    /// `daemon_service::is_active()`'s own `None` = "couldn't
    /// determine" convention.
    AlreadyInstalled { active: Option<bool> },
    /// Operator declined -- fall back to the existing passive hint.
    Declined,
    /// Operator wants it installed, with or without the Studio.
    Install { web_ui: bool },
}

fn decide_service_install(
    already_installed: bool,
    active: Option<bool>,
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
) -> Result<ServiceInstallDecision, String> {
    if already_installed {
        return Ok(ServiceInstallDecision::AlreadyInstalled { active });
    }
    let install_now = prompt_yes_no(
        "Install as a background service now? (survives logout/reboot)",
        true,
        reader,
        writer,
    )?;
    if !install_now {
        return Ok(ServiceInstallDecision::Declined);
    }
    let web_ui = prompt_yes_no("Also serve the Studio web UI?", false, reader, writer)?;
    Ok(ServiceInstallDecision::Install { web_ui })
}

/// Runs the full service-install offer: decide, then (if the operator
/// said yes) actually install via `run_install`. `run_install` is
/// injected so tests can exercise the success/failure print paths
/// without a real systemd/launchd call -- production passes
/// `crate::daemon_service::run_install` itself, whose `fn(bool, bool)
/// -> Result<(), String>` signature matches this parameter directly.
///
/// Never returns `Err` for an install failure -- by the time this runs,
/// `aivyx.toml` is already written; a failed service install is a
/// separate, later concern, not a wizard failure. Only genuinely
/// propagates `Err` if writing the prompt/output itself fails (matches
/// `prompt_yes_no`'s own convention elsewhere in this file).
fn offer_service_install(
    already_installed: bool,
    active: Option<bool>,
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
    run_install: impl FnOnce(bool, bool) -> Result<(), String>,
) -> Result<(), String> {
    match decide_service_install(already_installed, active, reader, writer)? {
        ServiceInstallDecision::AlreadyInstalled { active } => {
            let status = match active {
                Some(true) => "running",
                Some(false) => "not currently running",
                None => "status unknown",
            };
            writeln!(
                writer,
                "  (background service already installed — {status}; \
                 `aivyx doctor` has details)"
            )
            .map_err(|write_err| format!("write error: {write_err}"))?;
        }
        ServiceInstallDecision::Declined => {
            writeln!(
                writer,
                "  aivyx daemon install       — run it as a background service (survives logout/reboot)"
            )
            .map_err(|write_err| format!("write error: {write_err}"))?;
        }
        ServiceInstallDecision::Install { web_ui } => match run_install(web_ui, true) {
            Ok(()) => {
                writeln!(
                    writer,
                    "  Installed as a background service — `aivyx doctor` has details"
                )
                .map_err(|write_err| format!("write error: {write_err}"))?;
            }
            Err(e) => {
                writeln!(writer, "  Couldn't install as a background service: {e}")
                    .map_err(|write_err| format!("write error: {write_err}"))?;
            }
        },
    }
    Ok(())
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p aivyx-cli decide_service_install offer_service_install`
Expected: all 9 new tests PASS.

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p aivyx-cli --all-targets -- -D warnings`
Expected: zero warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-cli/src/bin/aivyx_modules/init.rs
git commit -m "feat(Phase 187): pure, testable service-install offer decision logic

ServiceInstallDecision + decide_service_install + offer_service_install
-- fully unit-testable (scripted Cursor reader, injected run_install
closure) without any real systemd/launchd side effects. Not wired into
the wizard yet -- that's Task 2."
```

---

### Task 2: Wire the offer into the wizard's ending

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx_modules/init.rs:2220-2226` (the
  existing Chapter Anchor hint block inside `run_init_wizard_inner`).

**Interfaces:**
- Consumes: `offer_service_install` (Task 1, exact signature above),
  `crate::daemon_service::{Platform, installed_unit_path, is_active,
  run_install}` (all pre-existing, unchanged — `crates/aivyx-cli/src/bin/
  aivyx_modules/daemon_service.rs`).
- Produces: nothing further downstream — this is the wizard's own
  terminal step.

- [ ] **Step 1: Replace the existing hint block**

Find this block (currently lines 2220-2226):

```rust
    // Chapter Anchor — the runs-for-days path: a real service so the agent keeps
    // running (and its scheduled routines keep firing) across logout + reboot.
    if matches!(crate::daemon_service::Platform::detect(), crate::daemon_service::Platform::Linux | crate::daemon_service::Platform::MacOs) {
        eprintln!(
            "  aivyx daemon install       — run it as a background service (survives logout/reboot)"
        );
    }
```

Replace with:

```rust
    // Chapter Anchor — the runs-for-days path: a real service so the agent keeps
    // running (and its scheduled routines keep firing) across logout + reboot.
    // Phase 187 — offer to install it right here instead of just printing the
    // command, so first-run can genuinely end with a running service.
    if matches!(
        crate::daemon_service::Platform::detect(),
        crate::daemon_service::Platform::Linux | crate::daemon_service::Platform::MacOs
    ) {
        let already_installed = crate::daemon_service::installed_unit_path().is_some();
        let active = if already_installed {
            crate::daemon_service::is_active()
        } else {
            None
        };
        offer_service_install(
            already_installed,
            active,
            &mut reader,
            &mut writer,
            crate::daemon_service::run_install,
        )?;
    }
```

This sits in the exact same position in the "Next steps:" list as
before (between the `aivyx daemon run --web-ui` line and the `aivyx
doctor` line), so every branch's output — the unchanged decline-hint,
the already-installed status note, or the install confirmation/error —
reads as one more line in that same list rather than an interrupting
paragraph.

`reader`/`writer` are the same `&mut reader`/`&mut writer` locals
already in scope from the top of `run_init_wizard_inner` (used by every
other `prompt_yes_no` call in this function) — no new variables needed.

- [ ] **Step 2: Run the full `aivyx-cli` test suite**

Run: `cargo test -p aivyx-cli`
Expected: all pass, including Task 1's 9 new tests and every pre-existing
`init.rs` test (this step only added a new call site — no existing
function's behavior changed).

- [ ] **Step 3: Run clippy**

Run: `cargo clippy -p aivyx-cli --all-targets -- -D warnings`
Expected: zero warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/aivyx-cli/src/bin/aivyx_modules/init.rs
git commit -m "feat(Phase 187): wizard offers to install + launch a service

Wires offer_service_install into run_init_wizard_inner's ending,
replacing the old passive 'aivyx daemon install' print with a real
yes/no offer (install now? also serve the Studio?) that calls the
already-existing daemon_service::run_install directly. Skips the
offer entirely if a service is already installed (no re-nagging a
repeat aivyx init run). A failed install prints the error and does
not fail the wizard -- aivyx.toml is already written by this point."
```

---

### Task 3: Final sweep

**Files:** none new — verification only.

- [ ] **Step 1: Full default-members build, test, and clippy sweep**

Run: `cargo build`
Expected: compiles clean.

Run: `cargo test`
Expected: all pass, zero failures.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: zero warnings.

- [ ] **Step 2: Manually exercise the wizard's ending, if a real
  Linux/macOS host is available**

This step is optional but valuable — Task 1/2's tests prove the decision
logic and the print paths, but not that `daemon_service::run_install`
itself, called from this new site, actually produces a real, working
systemd/launchd unit end to end (a mocked-only unit test structurally
cannot see this, the same lesson this session's `aivyx-kvcache` e2e-test
finding already established). If a real Linux or macOS host is
reachable (e.g. the GPU test rig used for `aivyx-kvcache`'s own
verification):

```sh
# on the real host, in a scratch directory
./target/release/aivyx init
# answer through to the service-install prompt; say yes, then yes to
# the Studio follow-up
systemctl --user status aivyx-daemon   # Linux — expect "active (running)"
# or: launchctl print gui/$(id -u)/com.aivyx.daemon   # macOS
aivyx daemon uninstall                 # clean up afterward
```

Expected: the service installs and starts for real, `aivyx doctor`'s
own "Service:" section reports it, and `aivyx daemon uninstall` removes
it cleanly afterward (matches `docs/INSTALL.md`'s own documented
uninstall behavior — untouched by this plan).

If no such host is reachable, skip this step — Task 1/2's unit tests are
the required bar; this step is a bonus real-world confirmation, not a
blocker.

- [ ] **Step 3: Confirm no stray behavior change to the untouched
  "Next steps" lines**

Run: `grep -n "chat with your agent\|open the Studio at\|re-check your setup" crates/aivyx-cli/src/bin/aivyx_modules/init.rs`
Expected: the three unrelated "Next steps" lines (`aivyx`, `aivyx daemon
run --web-ui`, `aivyx doctor`) are still present, unchanged, exactly as
before this plan.

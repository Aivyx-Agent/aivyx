# Phase 202 Follow-Ups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close all 3 follow-ups Phase 202's own final review logged: a
real gap (`verify-only`/`audit export`/`cost` can still create a store on
an unconfigured machine) and two cosmetic issues (doubled `aivyx:`-prefixed
output lines, an untestable passphrase-retry message).

**Architecture:** Two independent, small fixes. Issue 1+2 land together in
`aivyx.rs`'s early-validate gate (they touch overlapping lines) via two new
pure, directly-testable functions mirroring this file's own established
`should_early_validate`/`decide_*` extraction pattern. Issue 3 is a
one-function fix in `aivyx-channel`'s passphrase module, folding a
diagnostic message into an existing prompt string rather than adding a new
I/O parameter.

**Tech Stack:** Rust, the same `Cursor`/`rpassword::prompt_password_from_bufread`
test seams already established in these two files.

## Global Constraints

- Both branches of the new mode-check (diagnostic vs. default) must route
  through the exact same `decide_unconfigured_first_run` wizard-offer-or-fail
  mechanism — no new mechanism, no special-casing beyond message selection.
- The TTY-declined path's behavior is completely unchanged (full message
  before the prompt, short `"setup required — see above"` follow-up after
  decline) — only the non-TTY path changes (no early print; the full
  message is returned once instead).
- The passphrase confirm-reentry function's closure signature
  (`F: FnMut(&str) -> std::io::Result<String>`) does not change — only the
  prompt text passed to it varies between the first attempt and a retry.

---

### Task 1: Early-validate gate — close the diagnostic-mode gap and fix the doubled output

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs`

**Interfaces:**
- Produces: `should_early_validate(storage_path: &Path) -> bool` (signature
  change — drops its 3 `bool` parameters), `early_validate_message(...)  ->
  Result<(), String>`, `early_validate_fail_output(is_tty: bool,
  full_message: String) -> (Option<String>, String)`. All 3 are used only
  within this same file's `run()` function — no cross-task dependency.

- [ ] **Step 1: Write the failing tests for the simplified `should_early_validate`**

Find the existing `#[cfg(test)] mod early_validate_gate_tests` block (near
the end of the file) and replace its entire contents:

```rust
#[cfg(test)]
mod early_validate_gate_tests {
    use super::should_early_validate;
    use std::path::Path;

    #[test]
    fn fires_when_store_absent() {
        assert!(should_early_validate(Path::new(
            "/nonexistent/path/for/this/test"
        )));
    }

    #[test]
    fn skipped_when_store_already_exists() {
        // `tempfile` is not a dev-dependency of this crate; `uuid`
        // already is, so build a throwaway path the same way this
        // session's own aivyx-telegram/-discord/-slack checkpoint tests
        // already do.
        let path = std::env::temp_dir().join(format!(
            "aivyx-early-validate-gate-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, b"").expect("create dummy store file");
        assert!(!should_early_validate(&path));
        let _ = std::fs::remove_file(&path);
    }
}
```

(This deletes `skipped_for_verify_only_mode`/`skipped_for_audit_export_mode`/
`skipped_for_cost_mode` — they tested a mode-based exclusion that no longer
exists in this function; mode-awareness moves entirely to
`early_validate_message` in Step 3 below.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-cli --bin aivyx early_validate_gate_tests -- --nocapture`
Expected: FAIL — `should_early_validate` still takes 4 arguments.

- [ ] **Step 3: Simplify `should_early_validate`**

Find:

```rust
/// Whether `run()`'s early-validate gate should fire at all. Pure —
/// takes the already-computed facts, no I/O. Separated from the gate's
/// own side-effecting body (the prompt + wizard invocation) so the
/// *condition* is unit-testable without needing to drive a full `run()`
/// call.
fn should_early_validate(
    verify_only: bool,
    audit_export_mode: bool,
    cost_mode: bool,
    storage_path: &std::path::Path,
) -> bool {
    !verify_only && !audit_export_mode && !cost_mode && !storage_path.exists()
}
```

Replace with:

```rust
/// Whether `run()`'s early-validate gate should fire at all. Pure —
/// takes the already-computed facts, no I/O. Separated from the gate's
/// own side-effecting body (the prompt + wizard invocation) so the
/// *condition* is unit-testable without needing to drive a full `run()`
/// call.
///
/// Mode-independent: it used to also exclude `verify_only`/
/// `audit_export_mode`/`cost_mode`, but that exclusion meant those 3
/// modes could still create a real store on a totally unconfigured
/// machine (they never require an API key, so `config.validate()` can
/// never catch it for them). Mode-awareness now lives entirely in
/// `early_validate_message` below — this function only asks "does a
/// store exist yet," for every mode alike.
fn should_early_validate(storage_path: &std::path::Path) -> bool {
    !storage_path.exists()
}
```

(Only the simplification — the two brand-new functions below are
introduced test-first, in Steps 5-7, not here.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p aivyx-cli --bin aivyx early_validate_gate_tests -- --nocapture`
Expected: PASS (2 passed; 0 failed).

- [ ] **Step 5: Write the failing tests for `early_validate_message` and `early_validate_fail_output`**

Add a new test module immediately after `early_validate_gate_tests`:

```rust
#[cfg(test)]
mod early_validate_message_tests {
    use super::early_validate_message;
    use std::path::Path;

    #[test]
    fn diagnostic_mode_errors_even_when_validate_would_pass() {
        let result = early_validate_message(
            true, // verify_only
            false,
            false,
            Path::new("/nonexistent/path"),
            || Ok(()),
        );
        let err = result.expect_err("diagnostic mode must always error when the gate fires");
        assert!(
            err.contains("nothing to verify/export/report on"),
            "{err}"
        );
    }

    #[test]
    fn audit_export_mode_errors_even_when_validate_would_pass() {
        let result = early_validate_message(false, true, false, Path::new("/nonexistent/path"), || Ok(()));
        assert!(result.is_err());
    }

    #[test]
    fn cost_mode_errors_even_when_validate_would_pass() {
        let result = early_validate_message(false, false, true, Path::new("/nonexistent/path"), || Ok(()));
        assert!(result.is_err());
    }

    #[test]
    fn default_mode_passes_through_validate_ok() {
        let result = early_validate_message(false, false, false, Path::new("/nonexistent/path"), || Ok(()));
        assert!(result.is_ok());
    }

    #[test]
    fn default_mode_passes_through_validate_err() {
        let result = early_validate_message(
            false,
            false,
            false,
            Path::new("/nonexistent/path"),
            || {
                Err(aivyx_config::ConfigError::Missing {
                    field: "anthropic_api_key",
                })
            },
        );
        let err = result.expect_err("default mode must surface validate()'s own error");
        assert!(err.contains("anthropic_api_key"), "{err}");
    }
}

#[cfg(test)]
mod early_validate_fail_output_tests {
    use super::early_validate_fail_output;

    #[test]
    fn tty_shows_full_message_early_and_short_message_on_fail() {
        let (early, fail) = early_validate_fail_output(true, "aivyx: full message".to_string());
        assert_eq!(early, Some("aivyx: full message".to_string()));
        assert_eq!(fail, "setup required — see above");
    }

    #[test]
    fn non_tty_shows_nothing_early_and_full_message_on_fail() {
        let (early, fail) = early_validate_fail_output(false, "aivyx: full message".to_string());
        assert_eq!(early, None);
        assert_eq!(fail, "aivyx: full message");
    }
}
```

- [ ] **Step 6: Run the tests to verify they fail**

Run: `cargo test -p aivyx-cli --bin aivyx early_validate_message_tests early_validate_fail_output_tests -- --nocapture`
Expected: FAIL to compile — `early_validate_message` and
`early_validate_fail_output` don't exist yet.

- [ ] **Step 7: Implement `early_validate_message` and `early_validate_fail_output`**

Add both functions immediately after `should_early_validate` (from Step 3):

```rust
/// The validation-failure message for the early-validate gate, given the
/// mode and whether a store exists. The 3 diagnostic modes
/// (`verify_only`/`audit_export_mode`/`cost_mode`) never require an API
/// key (`LoadOptions.require_api_key` is `false` for them), so
/// `config.validate()` can never detect "nothing is configured yet" for
/// them — the absence of a store IS the signal for those modes,
/// independent of whatever `validate` would otherwise say. Takes
/// `validate` as an injected closure so this is testable without a real
/// `AivyxConfig`/`LoadOptions`.
fn early_validate_message(
    verify_only: bool,
    audit_export_mode: bool,
    cost_mode: bool,
    storage_path: &std::path::Path,
    validate: impl FnOnce() -> Result<(), aivyx_config::ConfigError>,
) -> Result<(), String> {
    if verify_only || audit_export_mode || cost_mode {
        Err(format!(
            "no store exists yet at {storage_path:?} — nothing to verify/export/report on"
        ))
    } else {
        validate().map_err(|e| e.to_string())
    }
}

/// What the early-validate gate should print immediately (if anything)
/// and what it should return if the operator declines the wizard offer
/// (or stdin isn't a terminal to ask in the first place). Pure — no I/O.
/// TTY: shows the full message before the wizard-offer prompt, then a
/// short follow-up on decline (a prompt already happened in between, so
/// the short follow-up doesn't read as a duplicate). Non-TTY: nothing
/// was shown yet, so print nothing early and return the full message
/// once, letting `main()`'s generic handler print it exactly one time.
fn early_validate_fail_output(is_tty: bool, full_message: String) -> (Option<String>, String) {
    if is_tty {
        (Some(full_message), "setup required — see above".to_string())
    } else {
        (None, full_message)
    }
}
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p aivyx-cli --bin aivyx early_validate_message_tests early_validate_fail_output_tests -- --nocapture`
Expected: PASS (7 passed; 0 failed).

- [ ] **Step 9: Wire both new functions into the gate body**

Find:

```rust
    if should_early_validate(verify_only, audit_export_mode, cost_mode, &storage_path) {
        if let Err(e) = config.validate(&load_opts) {
            let hint = "\n\nRun `aivyx init` to set this up (or `aivyx init \
                         --template coder|researcher|personal` for a quick start).";
            eprintln!("aivyx: {e}{hint}");
            let is_tty = io::stdin().is_terminal();
            let stdin = io::stdin();
            let mut reader = stdin.lock();
            let mut stderr_writer = io::stderr();
            match init::decide_unconfigured_first_run(
                is_tty,
                &mut reader,
                &mut stderr_writer,
            )? {
                init::UnconfiguredFirstRunDecision::RunWizardInline => {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
                    rt.block_on(init::run_init_wizard(None))?;
                    eprintln!("\nNow run `aivyx` again to start.");
                    return Ok(());
                }
                init::UnconfiguredFirstRunDecision::Fail => {
                    return Err("setup required — see above".to_string());
                }
            }
        }
    }
```

Replace with:

```rust
    if should_early_validate(&storage_path) {
        let validation_result = early_validate_message(
            verify_only,
            audit_export_mode,
            cost_mode,
            &storage_path,
            || config.validate(&load_opts),
        );
        if let Err(e) = validation_result {
            let hint = "\n\nRun `aivyx init` to set this up (or `aivyx init \
                         --template coder|researcher|personal` for a quick start).";
            let is_tty = io::stdin().is_terminal();
            let (early_print, fail_message) =
                early_validate_fail_output(is_tty, format!("aivyx: {e}{hint}"));
            if let Some(msg) = early_print {
                eprintln!("{msg}");
            }
            let stdin = io::stdin();
            let mut reader = stdin.lock();
            let mut stderr_writer = io::stderr();
            match init::decide_unconfigured_first_run(
                is_tty,
                &mut reader,
                &mut stderr_writer,
            )? {
                init::UnconfiguredFirstRunDecision::RunWizardInline => {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
                    rt.block_on(init::run_init_wizard(None))?;
                    eprintln!("\nNow run `aivyx` again to start.");
                    return Ok(());
                }
                init::UnconfiguredFirstRunDecision::Fail => {
                    return Err(fail_message);
                }
            }
        }
    }
```

- [ ] **Step 10: Run `cargo check` to confirm it compiles**

Run: `cargo check -p aivyx-cli`
Expected: PASS, zero errors. (`config`/`load_opts` are borrowed, not moved,
by the `|| config.validate(&load_opts)` closure — both remain valid for
their later uses further down in `run()`.)

- [ ] **Step 11: Run the full crate test suite**

Run: `cargo test -p aivyx-cli --bin aivyx`
Expected: PASS, all tests green — no regressions elsewhere in this large
file's test suite.

- [ ] **Step 12: Run clippy**

Run: `cargo clippy -p aivyx-cli --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 13: Commit**

```bash
git add crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "fix(aivyx-cli): close the verify-only/audit-export/cost store gap + doubled output

should_early_validate simplifies to a pure store-existence check,
mode-independent. The mode awareness moves into a new
early_validate_message: the 3 diagnostic modes (verify_only/
audit_export/cost) never require an API key, so config.validate()
could never detect 'nothing is configured yet' for them -- the
absence of a store is now the signal instead, routed through the
same wizard-offer-or-fail mechanism the default chat path already
uses.

Also fixes the non-interactive failure path printing two separately
aivyx:-prefixed lines for what was really one message -- a new
early_validate_fail_output only prints the full context early when
interactive (where a prompt happens in between, making a short
follow-up sensible); non-interactively it now prints the full
message exactly once.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: Passphrase confirm-reentry — route the retry message through the testable seam

**Files:**
- Modify: `crates/aivyx-channel/src/passphrase.rs`

**Interfaces:** none — self-contained to this one function, no dependency
on Task 1.

- [ ] **Step 1: Extend the existing mismatch test to assert on the retry wording**

Find:

```rust
    #[test]
    #[allow(deprecated)]
    fn confirm_reentry_mismatch_reprompts_until_matching() {
        // First pair ("typo-a" / "typo-b") mismatches and must be
        // silently discarded, not returned or mixed with the second
        // pair. Second pair ("real-pass" / "real-pass") matches.
        let mut reader = &b"typo-a\ntypo-b\nreal-pass\nreal-pass\n"[..];
        let mut sink: Vec<u8> = Vec::new();
        let bytes = read_interactive_password_with_confirm(|prompt| {
            rpassword::prompt_password_from_bufread(&mut reader, &mut sink, prompt)
        })
        .expect("second, matching pair must eventually succeed");
        assert_eq!(bytes, b"real-pass");
    }
```

Replace with:

```rust
    #[test]
    #[allow(deprecated)]
    fn confirm_reentry_mismatch_reprompts_until_matching() {
        // First pair ("typo-a" / "typo-b") mismatches and must be
        // silently discarded, not returned or mixed with the second
        // pair. Second pair ("real-pass" / "real-pass") matches.
        let mut reader = &b"typo-a\ntypo-b\nreal-pass\nreal-pass\n"[..];
        let mut sink: Vec<u8> = Vec::new();
        let bytes = read_interactive_password_with_confirm(|prompt| {
            rpassword::prompt_password_from_bufread(&mut reader, &mut sink, prompt)
        })
        .expect("second, matching pair must eventually succeed");
        assert_eq!(bytes, b"real-pass");
        let written = String::from_utf8_lossy(&sink);
        assert!(
            written.contains("Passphrases didn't match"),
            "retry prompt must carry the mismatch notice: {written:?}"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p aivyx-channel confirm_reentry_mismatch_reprompts_until_matching -- --nocapture`
Expected: FAIL — the mismatch message is currently a bare `eprintln!`, so
`sink` (which only receives text passed to
`rpassword::prompt_password_from_bufread`) does not contain it.

- [ ] **Step 3: Fold the retry message into the next prompt string**

Find:

```rust
fn read_interactive_password_with_confirm<F>(mut read: F) -> Result<Vec<u8>, PassphraseError>
where
    F: FnMut(&str) -> std::io::Result<String>,
{
    loop {
        let first = map_read_result(read(
            "aivyx passphrase (new store — you'll need this every time): ",
        ))?;
        let mut confirm = map_read_result(read("Confirm passphrase: "))?;
        if first == confirm {
            confirm.zeroize();
            return Ok(first);
        }
        // Mismatch: neither copy is going anywhere near a master key,
        // so zeroize both before looping — same discipline the
        // invariant above requires for every passphrase byte that
        // ever touches the heap.
        let mut first = first;
        first.zeroize();
        confirm.zeroize();
        eprintln!("Passphrases didn't match. Try again.");
    }
}
```

Replace with:

```rust
fn read_interactive_password_with_confirm<F>(mut read: F) -> Result<Vec<u8>, PassphraseError>
where
    F: FnMut(&str) -> std::io::Result<String>,
{
    const FIRST_PROMPT: &str = "aivyx passphrase (new store — you'll need this every time): ";
    const RETRY_PROMPT: &str = "Passphrases didn't match. Try again.\n\
                                 aivyx passphrase (new store — you'll need this every time): ";
    let mut prompt = FIRST_PROMPT;
    loop {
        let first = map_read_result(read(prompt))?;
        let mut confirm = map_read_result(read("Confirm passphrase: "))?;
        if first == confirm {
            confirm.zeroize();
            return Ok(first);
        }
        // Mismatch: neither copy is going anywhere near a master key,
        // so zeroize both before looping — same discipline the
        // invariant above requires for every passphrase byte that
        // ever touches the heap.
        let mut first = first;
        first.zeroize();
        confirm.zeroize();
        prompt = RETRY_PROMPT;
    }
}
```

(The bare `eprintln!` is gone entirely — the mismatch notice now travels
through the exact same `read(prompt)` seam every other prompt in this
function already uses, so it flows into the test's own `sink` naturally,
and into the real `/dev/tty` prompt in production exactly where the
operator is looking.)

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p aivyx-channel confirm_reentry_mismatch_reprompts_until_matching -- --nocapture`
Expected: PASS (1 passed; 0 failed).

- [ ] **Step 5: Run the full crate test suite**

Run: `cargo test -p aivyx-channel`
Expected: PASS, all tests green — no regressions in the other 3
`confirm_reentry_*` tests or any `interactive_source_*` test.

- [ ] **Step 6: Run clippy**

Run: `cargo clippy -p aivyx-channel --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-channel/src/passphrase.rs
git commit -m "fix(aivyx-channel): route the passphrase-mismatch retry message through the testable seam

read_interactive_password_with_confirm's mismatch notice was a bare
eprintln! that bypassed the injected read closure entirely --
untestable, and printed raw during cargo test. Now folded into the
next prompt string itself (no new function parameter needed), so it
flows through the exact same seam every other prompt in this function
already uses.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

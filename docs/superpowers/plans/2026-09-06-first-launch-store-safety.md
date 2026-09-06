# First-Launch Store Safety Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the first-launch bug chain where running bare `aivyx` before
`aivyx init` silently creates a permanent, unconfirmed encrypted store that
can later collide with a real `init` run and produce an undiagnosable
failure.

**Architecture:** Three small, independent-until-the-last-mile mechanisms:
a confirm-reentry step on the passphrase prompt (scoped to new-store
creation only), a store-collision guard in `aivyx init`, and an
early-validate gate in `aivyx`'s main dispatch that only activates when no
store exists yet — preserving every existing behavior for returning
installs untouched.

**Tech Stack:** Rust, `rpassword` (interactive passphrase reads),
`toml_edit`-adjacent config loader (`aivyx-config`), existing
`prompt_yes_no`/decision-enum testability pattern already used by
`init.rs`'s `decide_service_install`.

## Global Constraints

- All three mechanisms must produce **zero behavior change** for any install
  where a store already exists on disk at `storage_path` — the single most
  important regression guard across every task's tests.
- The confirm-reentry passphrase path and the store-collision guard are both
  scoped to **new-store creation only** — the unlock-an-existing-store path
  keeps its existing single-prompt UX, unchanged.
- Every new user-facing message text below is exact and approved — use it
  verbatim, do not paraphrase.

---

### Task 1: Confirm-reentry on the new-store passphrase prompt

**Files:**
- Modify: `crates/aivyx-channel/src/passphrase.rs`

**Interfaces:**
- Produces: `PassphraseSource::InteractivePrompt` becomes
  `PassphraseSource::InteractivePrompt { confirm: bool }`. `fetch_passphrase_bytes`
  branches on `confirm`. Task 3 constructs this variant with
  `confirm: new_store`.

- [ ] **Step 1: Change the `PassphraseSource` enum**

In `crates/aivyx-channel/src/passphrase.rs`, the enum currently ends:

```rust
    /// Interactive TTY prompt via `rpassword::prompt_password`.
    /// Opens `/dev/tty` directly on Unix (so it works even when
    /// stdin is piped, as long as a controlling terminal exists),
    /// prints `aivyx passphrase: `, reads a single line with echo
    /// disabled, strips the trailing newline, and zeroizes the
    /// internal buffer on the way out. Returns
    /// [`PassphraseError::InteractiveIo`] if `/dev/tty` is not
    /// reachable, or [`PassphraseError::InteractiveEmpty`] if the
    /// user pressed enter on an empty line.
    InteractivePrompt,
}
```

Change to:

```rust
    /// Interactive TTY prompt via `rpassword::prompt_password`.
    /// Opens `/dev/tty` directly on Unix (so it works even when
    /// stdin is piped, as long as a controlling terminal exists),
    /// reads a single line with echo disabled, strips the trailing
    /// newline, and zeroizes the internal buffer on the way out.
    /// Returns [`PassphraseError::InteractiveIo`] if `/dev/tty` is
    /// not reachable, or [`PassphraseError::InteractiveEmpty`] if
    /// the user pressed enter on an empty line.
    ///
    /// `confirm` — when `true` (creating a brand-new store), prompts
    /// twice and requires a match before returning, re-prompting the
    /// whole pair on mismatch; mirrors `aivyx keyring set`'s existing
    /// prompt+confirm+match-check shape. When `false` (unlocking an
    /// existing store), behavior is unchanged from before this field
    /// existed: one prompt, no confirmation — a wrong guess there
    /// fails cleanly at decrypt time and the user just retries the
    /// command, so there's nothing to protect against a typo for.
    InteractivePrompt { confirm: bool },
}
```

- [ ] **Step 2: Update the `Debug` impl**

Find:

```rust
            PassphraseSource::InteractivePrompt => f.write_str("InteractivePrompt"),
```

Replace with:

```rust
            PassphraseSource::InteractivePrompt { confirm } => f
                .debug_struct("InteractivePrompt")
                .field("confirm", confirm)
                .finish(),
```

- [ ] **Step 3: Run `cargo check` to confirm it fails only on the known sites**

Run: `cargo check -p aivyx-channel --tests`
Expected: FAIL — errors at `fetch_passphrase_bytes`'s match (non-exhaustive
pattern) and at the one existing test constructing
`PassphraseSource::InteractivePrompt` without the new field.

- [ ] **Step 4: Extract the shared read-and-validate helper, update the existing single-prompt path**

Find:

```rust
fn fetch_passphrase_bytes(source: PassphraseSource) -> Result<Vec<u8>, PassphraseError> {
    match source {
        PassphraseSource::Env { var_name } => match std::env::var(&var_name) {
            Ok(s) if s.is_empty() => Err(PassphraseError::EnvEmpty(var_name)),
            Ok(s) => Ok(s.into_bytes()),
            Err(_) => Err(PassphraseError::EnvNotSet(var_name)),
        },
        PassphraseSource::FromConfig(secret) => {
            // Read the secret into an owned Vec<u8>. The SecretString
            // itself zeroizes on drop; we additionally zeroize the
            // intermediate clone via the standard derive path
            // (`derive_master_key` already zeroizes the Vec after
            // hashing).
            let bytes = secret.expose_secret().as_bytes().to_vec();
            if bytes.is_empty() {
                // Same posture as EnvEmpty: empty passphrase is
                // refused outright. Argon2id would happily hash it.
                Err(PassphraseError::EnvEmpty("config:[aivyx]passphrase".into()))
            } else {
                Ok(bytes)
            }
        }
        PassphraseSource::Fixture(f) => Ok(f()),
        PassphraseSource::InteractivePrompt => {
            // `rpassword::prompt_password` opens `/dev/tty` on Unix,
            // echoes the prompt, reads one line with echo disabled,
            // and returns a `String`. Under `cargo test` there's no
            // controlling tty, so the unit test path routes through
            // `read_interactive_password_inner` with a `BufRead` +
            // `Write` seam instead (see
            // `interactive_source_reads_password_from_bufread`).
            read_interactive_password_inner(|| {
                rpassword::prompt_password("aivyx passphrase: ")
            })
        }
    }
}
```

Replace with:

```rust
fn fetch_passphrase_bytes(source: PassphraseSource) -> Result<Vec<u8>, PassphraseError> {
    match source {
        PassphraseSource::Env { var_name } => match std::env::var(&var_name) {
            Ok(s) if s.is_empty() => Err(PassphraseError::EnvEmpty(var_name)),
            Ok(s) => Ok(s.into_bytes()),
            Err(_) => Err(PassphraseError::EnvNotSet(var_name)),
        },
        PassphraseSource::FromConfig(secret) => {
            // Read the secret into an owned Vec<u8>. The SecretString
            // itself zeroizes on drop; we additionally zeroize the
            // intermediate clone via the standard derive path
            // (`derive_master_key` already zeroizes the Vec after
            // hashing).
            let bytes = secret.expose_secret().as_bytes().to_vec();
            if bytes.is_empty() {
                // Same posture as EnvEmpty: empty passphrase is
                // refused outright. Argon2id would happily hash it.
                Err(PassphraseError::EnvEmpty("config:[aivyx]passphrase".into()))
            } else {
                Ok(bytes)
            }
        }
        PassphraseSource::Fixture(f) => Ok(f()),
        PassphraseSource::InteractivePrompt { confirm: false } => {
            // `rpassword::prompt_password` opens `/dev/tty` on Unix,
            // echoes the prompt, reads one line with echo disabled,
            // and returns a `String`. Under `cargo test` there's no
            // controlling tty, so the unit test path routes through
            // `read_interactive_password_inner` with a `BufRead` +
            // `Write` seam instead (see
            // `interactive_source_reads_password_from_bufread`).
            read_interactive_password_inner(|| {
                rpassword::prompt_password("aivyx passphrase: ")
            })
        }
        PassphraseSource::InteractivePrompt { confirm: true } => {
            // Creating a brand-new store — prompt twice and require a
            // match, the same shape `aivyx keyring set` already uses.
            // See `read_interactive_password_with_confirm`.
            read_interactive_password_with_confirm(|prompt| {
                rpassword::prompt_password(prompt)
            })
        }
    }
}

/// Shared empty-check + I/O-error-mapping step for a single
/// interactive read. Factored out of `read_interactive_password_inner`
/// so `read_interactive_password_with_confirm` (below) can reuse it
/// for each of its two reads without duplicating the mapping logic.
fn map_read_result(result: std::io::Result<String>) -> Result<Vec<u8>, PassphraseError> {
    let pass = result.map_err(|e| PassphraseError::InteractiveIo { reason: e.to_string() })?;
    if pass.is_empty() {
        return Err(PassphraseError::InteractiveEmpty);
    }
    // `String::into_bytes` hands over the existing heap allocation
    // — no copy — so the outer zeroize path owns the one-and-only
    // persistent copy of the passphrase bytes.
    Ok(pass.into_bytes())
}
```

- [ ] **Step 5: Update `read_interactive_password_inner` to use the shared helper**

Find:

```rust
fn read_interactive_password_inner<F>(read: F) -> Result<Vec<u8>, PassphraseError>
where
    F: FnOnce() -> std::io::Result<String>,
{
    let pass = read()
        .map_err(|e| PassphraseError::InteractiveIo { reason: e.to_string() })?;
    if pass.is_empty() {
        return Err(PassphraseError::InteractiveEmpty);
    }
    // `String::into_bytes` hands over the existing heap allocation
    // — no copy — so the outer zeroize path owns the one-and-only
    // persistent copy of the passphrase bytes.
    Ok(pass.into_bytes())
}
```

Replace with:

```rust
fn read_interactive_password_inner<F>(read: F) -> Result<Vec<u8>, PassphraseError>
where
    F: FnOnce() -> std::io::Result<String>,
{
    map_read_result(read())
}
```

(This is a pure refactor — `read_interactive_password_inner`'s own
signature, callers, and the 3 existing tests exercising it are unaffected.)

- [ ] **Step 6: Add `read_interactive_password_with_confirm`**

Add this function immediately after `read_interactive_password_inner`:

```rust
/// New-store passphrase entry: prompts, prompts again, and requires a
/// match before returning — the confirm-reentry counterpart to
/// `read_interactive_password_inner`'s single unconfirmed prompt.
/// Loops (re-prompting both) on mismatch, matching this codebase's
/// existing "loop forever on invalid input" idiom
/// (`aivyx_modules::init::prompt_yes_no`).
///
/// Takes one `FnMut(&str) -> io::Result<String>` closure (parameterized
/// by the prompt text) rather than two separate closures — two closures
/// each capturing the same test reader/writer would violate the borrow
/// checker, since both would need to exist simultaneously as function
/// arguments. One closure invoked twice sequentially avoids that.
fn read_interactive_password_with_confirm<F>(mut read: F) -> Result<Vec<u8>, PassphraseError>
where
    F: FnMut(&str) -> std::io::Result<String>,
{
    loop {
        let first = map_read_result(read(
            "aivyx passphrase (new store — you'll need this every time): ",
        ))?;
        let confirm = map_read_result(read("Confirm passphrase: "))?;
        if first == confirm {
            return Ok(first);
        }
        eprintln!("Passphrases didn't match. Try again.");
    }
}
```

- [ ] **Step 7: Run `cargo check` — expect one remaining error (the test)**

Run: `cargo check -p aivyx-channel --tests`
Expected: FAIL — one error, the existing test at
`debug_interactive_prompt_renders_without_side_effects` still constructs
the bare unit-variant form.

- [ ] **Step 8: Fix the existing test**

Find:

```rust
    #[test]
    fn debug_interactive_prompt_renders_without_side_effects() {
        // InteractivePrompt has no payload — the tripwire here is
        // that `format!` must not touch the tty or block on a read.
        let rendered = format!("{:?}", PassphraseSource::InteractivePrompt);
        assert_eq!(rendered, "InteractivePrompt");
    }
```

Replace with:

```rust
    #[test]
    fn debug_interactive_prompt_renders_without_side_effects() {
        // The tripwire here is that `format!` must not touch the tty
        // or block on a read.
        let rendered = format!(
            "{:?}",
            PassphraseSource::InteractivePrompt { confirm: false }
        );
        assert_eq!(rendered, "InteractivePrompt { confirm: false }");
    }
```

- [ ] **Step 9: Run `cargo check` to confirm the crate compiles**

Run: `cargo check -p aivyx-channel --tests`
Expected: PASS, zero errors.

- [ ] **Step 10: Add tests for the confirm-reentry path**

Add these tests immediately after `debug_interactive_prompt_renders_without_side_effects`:

```rust
    #[test]
    #[allow(deprecated)] // test seam: rpassword 7.5 deprecated prompt_password_from_bufread; prod uses prompt_password
    fn confirm_reentry_matching_pair_succeeds() {
        let mut reader = &b"same-pass\nsame-pass\n"[..];
        let mut sink: Vec<u8> = Vec::new();
        let bytes = read_interactive_password_with_confirm(|prompt| {
            rpassword::prompt_password_from_bufread(&mut reader, &mut sink, prompt)
        })
        .expect("matching pair must succeed");
        assert_eq!(bytes, b"same-pass");
    }

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

    #[test]
    #[allow(deprecated)]
    fn confirm_reentry_empty_first_entry_is_rejected() {
        let mut reader = &b"\n"[..];
        let mut sink: Vec<u8> = Vec::new();
        let err = read_interactive_password_with_confirm(|prompt| {
            rpassword::prompt_password_from_bufread(&mut reader, &mut sink, prompt)
        })
        .unwrap_err();
        assert!(matches!(err, PassphraseError::InteractiveEmpty));
    }

    #[test]
    #[allow(deprecated)]
    fn confirm_reentry_wording_differs_from_unconfirmed_prompt() {
        // The new-store prompt must name the stakes; the existing
        // unlock prompt (confirm: false) must stay exactly as it was.
        let mut reader = &b"x\nx\n"[..];
        let mut sink: Vec<u8> = Vec::new();
        read_interactive_password_with_confirm(|prompt| {
            rpassword::prompt_password_from_bufread(&mut reader, &mut sink, prompt)
        })
        .expect("must succeed");
        let written = String::from_utf8_lossy(&sink);
        assert!(
            written.contains("new store"),
            "new-store prompt must mention it's a new store: {written:?}"
        );
    }
```

- [ ] **Step 11: Run the new tests**

Run: `cargo test -p aivyx-channel confirm_reentry -- --nocapture`
Expected: PASS (4 passed; 0 failed).

- [ ] **Step 12: Run the full crate test suite**

Run: `cargo test -p aivyx-channel`
Expected: PASS, all tests green (no regressions in the 3 existing
`interactive_source_*` tests or `debug_interactive_prompt_renders_without_side_effects`).

- [ ] **Step 13: Run clippy**

Run: `cargo clippy -p aivyx-channel --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 14: Commit**

```bash
git add crates/aivyx-channel/src/passphrase.rs
git commit -m "feat(aivyx-channel): confirm-reentry on the new-store passphrase prompt

PassphraseSource::InteractivePrompt gains a confirm: bool field.
confirm: true (creating a brand-new store) now prompts twice and
requires a match, re-prompting the whole pair on mismatch -- mirroring
aivyx keyring set's existing shape. confirm: false (unlocking an
existing store) is completely unchanged: one prompt, no confirmation.

Closes part of the first-launch audit finding: a typo on the very
first passphrase entry previously locked a fresh install out of its
own just-created store with zero recovery path.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: `aivyx init`'s store-collision guard + the unconfigured-first-run decision

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx_modules/init.rs`

**Interfaces:**
- Produces: `pub(crate) enum UnconfiguredFirstRunDecision { RunWizardInline, Fail }`
  and `pub(crate) fn decide_unconfigured_first_run(is_tty: bool, reader: &mut dyn BufRead, writer: &mut dyn IoWrite) -> Result<UnconfiguredFirstRunDecision, String>`.
  Task 3 calls both from `aivyx.rs` via `init::decide_unconfigured_first_run(...)`
  and `init::UnconfiguredFirstRunDecision::{RunWizardInline, Fail}`.

**No dependency on Task 1** — this task touches only `init.rs`, a different
file with no shared types.

**Correction from initial grounding**: `run_init_wizard_inner` hardcodes
`io::stdin()`/`io::stderr()` internally rather than accepting them as
parameters — confirmed by reading its signature (`async fn
run_init_wizard_inner(template_defaults: TemplateDefaults) -> Result<(),
String>`, no reader/writer parameters) and confirming (via grep) that
**no test in this file calls `run_init_wizard`/`run_init_wizard_inner`
directly at all** — every existing test instead targets one of the small
`decide_*`/`prompt_*`-style pure functions the wizard is built from. So
this guard is designed the same way from the start: extract its decision
into a small pure function (mirroring `decide_service_install` exactly),
test that directly, and keep the call site inside `run_init_wizard_inner`
a thin, untested wrapper — consistent with every other piece of this
file's own logic.

- [ ] **Step 1: Write the failing test for `decide_storage_collision`**

Add these tests near `decide_service_install_skips_the_offer_when_already_installed`
and its siblings (same `mod tests` block, same `Cursor`/`Vec::new()` style):

```rust
    #[test]
    fn decide_storage_collision_no_aborts() {
        let mut input = Cursor::new(b"n\n" as &[u8]);
        let mut output = Vec::new();
        let proceed =
            decide_storage_collision("/tmp/fake-store.redb", &mut input, &mut output)
                .unwrap();
        assert!(!proceed);
        let written = String::from_utf8_lossy(&output);
        assert!(
            written.contains("already exists"),
            "warning must mention the collision: {written:?}"
        );
    }

    #[test]
    fn decide_storage_collision_yes_proceeds() {
        let mut input = Cursor::new(b"y\n" as &[u8]);
        let mut output = Vec::new();
        let proceed =
            decide_storage_collision("/tmp/fake-store.redb", &mut input, &mut output)
                .unwrap();
        assert!(proceed);
    }

    #[test]
    fn decide_storage_collision_defaults_to_no_on_bare_enter() {
        let mut input = Cursor::new(b"\n" as &[u8]);
        let mut output = Vec::new();
        let proceed =
            decide_storage_collision("/tmp/fake-store.redb", &mut input, &mut output)
                .unwrap();
        assert!(!proceed, "default must be No — continuing is the riskier choice");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-cli decide_storage_collision -- --nocapture`
Expected: FAIL with "cannot find function `decide_storage_collision`".

- [ ] **Step 3: Add `decide_storage_collision` and wire it into the wizard**

Add this function immediately after `decide_service_install`:

```rust
/// Whether to proceed writing a config that points at a storage path
/// where a store already exists (e.g. from an earlier accidental bare
/// `aivyx` run, or any prior install). Pure with respect to OS calls,
/// mirroring `decide_service_install`'s own testability shape — the
/// caller does the actual `Path::exists()` check and only calls this
/// when it's already true.
fn decide_storage_collision(
    storage_path: &str,
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
) -> Result<bool, String> {
    writeln!(
        writer,
        "\n⚠ A store already exists at {storage_path}\n  \
         (possibly from an earlier run). Continuing will NOT delete it — your\n  \
         passphrase on first launch must match the one it was created under.\n  \
         If you don't know that passphrase, delete the file first."
    )
    .map_err(|e| format!("write error: {e}"))?;
    prompt_yes_no("Continue anyway?", false, reader, writer)
}
```

In `crates/aivyx-cli/src/bin/aivyx_modules/init.rs`, find (inside
`run_init_wizard_inner`):

```rust
    let storage_path = prompt_line(
        &format!("Storage path [{default_storage}]: "),
        &mut reader,
        &mut writer,
    )?;
    let storage_path = if storage_path.is_empty() {
        default_storage
    } else {
        storage_path
    };

    // 5b. Web search — bundled MCP server (Phase 46).
```

Replace with:

```rust
    let storage_path = prompt_line(
        &format!("Storage path [{default_storage}]: "),
        &mut reader,
        &mut writer,
    )?;
    let storage_path = if storage_path.is_empty() {
        default_storage
    } else {
        storage_path
    };

    // Store-collision guard — closes the compounding half of the
    // first-launch audit finding: writing a fresh aivyx.toml that
    // points at a storage path where a store already exists (e.g.
    // from an earlier accidental bare `aivyx` run) would let a
    // different passphrase on first launch silently fail to decrypt
    // it later, with no indication why.
    if Path::new(&storage_path).exists() {
        let proceed = decide_storage_collision(&storage_path, &mut reader, &mut writer)?;
        if !proceed {
            eprintln!("Aborted.");
            return Ok(());
        }
    }

    // 5b. Web search — bundled MCP server (Phase 46).
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p aivyx-cli decide_storage_collision -- --nocapture`
Expected: PASS (3 passed; 0 failed).

- [ ] **Step 5: Write the failing tests for `decide_unconfigured_first_run`**

Add these tests near `decide_service_install_skips_the_offer_when_already_installed`
and its siblings (same `mod tests` block), matching their exact style
(`Cursor::new(b"..." as &[u8])` for the reader, `Vec::new()` for the writer):

```rust
    #[test]
    fn decide_unconfigured_first_run_fails_immediately_when_not_a_tty() {
        let mut input = Cursor::new(b"" as &[u8]);
        let mut output = Vec::new();
        let decision =
            decide_unconfigured_first_run(false, &mut input, &mut output).unwrap();
        assert!(matches!(decision, UnconfiguredFirstRunDecision::Fail));
        // No prompt was printed -- nothing was asked when there's no
        // one to ask.
        assert!(output.is_empty());
    }

    #[test]
    fn decide_unconfigured_first_run_yes_runs_the_wizard() {
        let mut input = Cursor::new(b"y\n" as &[u8]);
        let mut output = Vec::new();
        let decision =
            decide_unconfigured_first_run(true, &mut input, &mut output).unwrap();
        assert!(matches!(
            decision,
            UnconfiguredFirstRunDecision::RunWizardInline
        ));
    }

    #[test]
    fn decide_unconfigured_first_run_no_fails() {
        let mut input = Cursor::new(b"n\n" as &[u8]);
        let mut output = Vec::new();
        let decision =
            decide_unconfigured_first_run(true, &mut input, &mut output).unwrap();
        assert!(matches!(decision, UnconfiguredFirstRunDecision::Fail));
    }

    #[test]
    fn decide_unconfigured_first_run_defaults_to_yes_on_bare_enter() {
        let mut input = Cursor::new(b"\n" as &[u8]);
        let mut output = Vec::new();
        let decision =
            decide_unconfigured_first_run(true, &mut input, &mut output).unwrap();
        assert!(matches!(
            decision,
            UnconfiguredFirstRunDecision::RunWizardInline
        ));
    }
```

- [ ] **Step 6: Run the tests to verify they fail**

Run: `cargo test -p aivyx-cli decide_unconfigured_first_run -- --nocapture`
Expected: FAIL with "cannot find function/type `decide_unconfigured_first_run`/`UnconfiguredFirstRunDecision`".

- [ ] **Step 7: Add `UnconfiguredFirstRunDecision` and `decide_unconfigured_first_run`**

Add these immediately after `ServiceInstallDecision`'s own definition (near
`decide_service_install`), mirroring that pattern exactly:

```rust
/// Whether `run()` should run the setup wizard inline or fail, when it
/// finds itself about to create a brand-new store for a config that
/// would fail validation anyway. Pure with respect to OS calls,
/// mirroring `ServiceInstallDecision`'s own testability shape — no I/O
/// beyond the injected `reader`/`writer`.
pub(crate) enum UnconfiguredFirstRunDecision {
    /// Run `run_init_wizard` inline, then exit — the operator said yes.
    RunWizardInline,
    /// Fail with the original validation error — the operator said no,
    /// or stdin isn't a terminal to ask in the first place.
    Fail,
}

/// Decides `UnconfiguredFirstRunDecision` from whether stdin is a
/// terminal and, if so, the operator's answer to "run the wizard now?".
/// When `is_tty` is `false` (scripts, CI, a misconfigured service unit),
/// fails immediately without prompting — there's no one to ask, and
/// blocking on a read that will never come would hang the process.
pub(crate) fn decide_unconfigured_first_run(
    is_tty: bool,
    reader: &mut dyn BufRead,
    writer: &mut dyn IoWrite,
) -> Result<UnconfiguredFirstRunDecision, String> {
    if !is_tty {
        return Ok(UnconfiguredFirstRunDecision::Fail);
    }
    let run_now = prompt_yes_no(
        "Run the setup wizard now?",
        true,
        reader,
        writer,
    )?;
    Ok(if run_now {
        UnconfiguredFirstRunDecision::RunWizardInline
    } else {
        UnconfiguredFirstRunDecision::Fail
    })
}
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p aivyx-cli decide_unconfigured_first_run -- --nocapture`
Expected: PASS (4 passed; 0 failed).

- [ ] **Step 9: Run the full crate test suite**

Run: `cargo test -p aivyx-cli`
Expected: PASS, all tests green (no regressions in
`decide_service_install_*` or any existing `run_init_wizard` test).

- [ ] **Step 10: Run clippy**

Run: `cargo clippy -p aivyx-cli --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 11: Commit**

```bash
git add crates/aivyx-cli/src/bin/aivyx_modules/init.rs
git commit -m "feat(aivyx-cli): aivyx init store-collision guard + first-run decision

aivyx init now warns and requires confirmation before writing a config
that points at a storage path where a store already exists (e.g. from
an earlier accidental bare 'aivyx' run) -- continuing doesn't delete
the existing store, so a mismatched passphrase on first real launch
would otherwise fail to decrypt with no indication why.

Also adds UnconfiguredFirstRunDecision + decide_unconfigured_first_run,
mirroring decide_service_install's existing pure-decision-function
pattern -- consumed by aivyx.rs's early-validate gate (next task).

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 3: `aivyx.rs`'s early-validate gate

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs`

**Interfaces:**
- Consumes: `PassphraseSource::InteractivePrompt { confirm: bool }` (Task 1);
  `init::UnconfiguredFirstRunDecision`, `init::decide_unconfigured_first_run`
  (Task 2).

**Depends on:** Tasks 1 and 2 (needs Task 1's enum shape to construct
correctly, and Task 2's new decision function to call).

- [ ] **Step 1: Move the `storage_path` binding earlier**

In `crates/aivyx-cli/src/bin/aivyx.rs`'s `run()` function, find:

```rust
    if let Some(name) = print_role {
        let rendered = render_role_envelope(&name, &config, channel_kind)?;
        print!("{rendered}");
        return Ok(());
    }

    // Sandbox root: create the directory if it does not exist so a
    // fresh install "just works" the same way Phase 4 promised. The
    // config layer returns a `PathBuf` with source provenance; we do
    // not mkdir inside the config layer because "create side effects
    // on load" is exactly the ambient-behavior trap the
    // AGENTS.md-equivalent hygiene rules in this repo try to avoid.
    //
    // Verify-only mode skips this — no session, no tools, no sandbox.
    // Phase 105 — audit-export shares the same skip: no fs sandbox
    // is touched by a read-only chain dump.
    if !verify_only && !audit_export_mode && !cost_mode {
        let root = &config.fs_root.value;
        std::fs::create_dir_all(root)
            .map_err(|e| format!("failed to create fs sandbox root {root:?}: {e}"))?;
    }

    // Resolve the encrypted store path + its sidecar salt file. The
    // config layer handled path *resolution* (env → toml → XDG → HOME
    // default); we only need to mkdir the parent directory and build
    // the sidecar path here.
    let storage_path = config.storage_path.value.clone();
    if let Some(parent) = storage_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!("failed to create storage parent directory {parent:?}: {e}")
        })?;
    }
    let salt_path = salt_path_for(&storage_path);
```

Replace with:

```rust
    if let Some(name) = print_role {
        let rendered = render_role_envelope(&name, &config, channel_kind)?;
        print!("{rendered}");
        return Ok(());
    }

    // Resolve the encrypted store path early — needed both for the
    // pre-flight check immediately below and the mkdir further down.
    let storage_path = config.storage_path.value.clone();

    // First-launch store safety — before touching disk at all, catch
    // the case where nothing has ever configured this install: no
    // store exists yet at the target path, and validation would fail
    // anyway. Moving `validate()` this early is only safe when the
    // store doesn't exist yet — an *existing* store can still supply
    // a missing secret via `hydrate_secrets_from_store` further down,
    // so this check is skipped once a prior store is on disk (the
    // unchanged order
    // further down handles that case, exactly as before this change).
    if !verify_only && !audit_export_mode && !cost_mode && !storage_path.exists() {
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
                    return Err(format!("{e}{hint}"));
                }
            }
        }
    }

    // Sandbox root: create the directory if it does not exist so a
    // fresh install "just works" the same way Phase 4 promised. The
    // config layer returns a `PathBuf` with source provenance; we do
    // not mkdir inside the config layer because "create side effects
    // on load" is exactly the ambient-behavior trap the
    // AGENTS.md-equivalent hygiene rules in this repo try to avoid.
    //
    // Verify-only mode skips this — no session, no tools, no sandbox.
    // Phase 105 — audit-export shares the same skip: no fs sandbox
    // is touched by a read-only chain dump.
    if !verify_only && !audit_export_mode && !cost_mode {
        let root = &config.fs_root.value;
        std::fs::create_dir_all(root)
            .map_err(|e| format!("failed to create fs sandbox root {root:?}: {e}"))?;
    }

    // Storage path's sidecar salt file. `storage_path` itself was
    // already resolved above.
    if let Some(parent) = storage_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!("failed to create storage parent directory {parent:?}: {e}")
        })?;
    }
    let salt_path = salt_path_for(&storage_path);
```

- [ ] **Step 2: Run `cargo check` to confirm it compiles**

Run: `cargo check -p aivyx-cli`
Expected: PASS. (`storage_path` is used identically to before at every
downstream site — only its binding moved earlier and its comment changed;
`init::decide_unconfigured_first_run`/`init::UnconfiguredFirstRunDecision`
already exist from Task 2, and `init::run_init_wizard` already existed
before this plan.)

- [ ] **Step 3: Update `select_passphrase_source`'s signature**

Find:

```rust
fn select_passphrase_source(
    passphrase: Option<&aivyx_config::SourcedSecret>,
) -> Result<PassphraseSource, String> {
    if let Some(secret) = passphrase {
        return match secret.source {
            aivyx_config::FieldSource::Env => Ok(PassphraseSource::Env {
                var_name: DEFAULT_ENV_VAR.to_string(),
            }),
            _ => Ok(PassphraseSource::FromConfig(secret.value.clone())),
        };
    }
    // Chapter Keyring — no explicit env/TOML passphrase: prefer the OS keyring
    // (encrypted at rest) over an interactive prompt. An unavailable/locked
    // keyring is not fatal — fall through to the prompt.
    match aivyx_channel::keyring_store::retrieve() {
        Ok(Some(secret)) => return Ok(PassphraseSource::FromConfig(secret)),
        Ok(None) => {}
        Err(e) => eprintln!(
            "aivyx: OS keyring not usable ({e}); trying other passphrase sources"
        ),
    }
    if io::stdin().is_terminal() {
        Ok(PassphraseSource::InteractivePrompt)
    } else {
        Err(format!(
            "no passphrase available: `{DEFAULT_ENV_VAR}` is not set, \
             no `[aivyx] passphrase` in the TOML config, nothing in the OS \
             keyring (`aivyx keyring set`), and stdin is not a terminal. \
             Export the env var, set the TOML field, store it in the keyring, \
             or run aivyx from an interactive shell."
        ))
    }
}
```

Replace with (only the signature and the `InteractivePrompt` construction
change):

```rust
fn select_passphrase_source(
    passphrase: Option<&aivyx_config::SourcedSecret>,
    new_store: bool,
) -> Result<PassphraseSource, String> {
    if let Some(secret) = passphrase {
        return match secret.source {
            aivyx_config::FieldSource::Env => Ok(PassphraseSource::Env {
                var_name: DEFAULT_ENV_VAR.to_string(),
            }),
            _ => Ok(PassphraseSource::FromConfig(secret.value.clone())),
        };
    }
    // Chapter Keyring — no explicit env/TOML passphrase: prefer the OS keyring
    // (encrypted at rest) over an interactive prompt. An unavailable/locked
    // keyring is not fatal — fall through to the prompt.
    match aivyx_channel::keyring_store::retrieve() {
        Ok(Some(secret)) => return Ok(PassphraseSource::FromConfig(secret)),
        Ok(None) => {}
        Err(e) => eprintln!(
            "aivyx: OS keyring not usable ({e}); trying other passphrase sources"
        ),
    }
    if io::stdin().is_terminal() {
        Ok(PassphraseSource::InteractivePrompt { confirm: new_store })
    } else {
        Err(format!(
            "no passphrase available: `{DEFAULT_ENV_VAR}` is not set, \
             no `[aivyx] passphrase` in the TOML config, nothing in the OS \
             keyring (`aivyx keyring set`), and stdin is not a terminal. \
             Export the env var, set the TOML field, store it in the keyring, \
             or run aivyx from an interactive shell."
        ))
    }
}
```

- [ ] **Step 4: Update the one real call site**

Find:

```rust
    let passphrase_source = select_passphrase_source(config.passphrase.as_ref())?;
```

Replace with:

```rust
    let new_store = !storage_path.exists();
    let passphrase_source = select_passphrase_source(config.passphrase.as_ref(), new_store)?;
```

(`storage_path` is unmoved and still holds the same value bound in Step 1 —
confirmed by reading the code between the two points: every intervening use
is `.exists()`, `.parent()`, or a borrow, never a move.)

- [ ] **Step 5: Run `cargo check` for the whole workspace**

Run: `cargo check -p aivyx-cli`
Expected: PASS, zero errors.

- [ ] **Step 6: Write a regression test proving existing-store behavior is unchanged**

`run()` itself has no existing tests and is not practically end-to-end
testable (confirmed: it's a 250+-line function that spawns real tokio
runtimes and opens a real encrypted store; grepping this file's own `mod
tests` block for any call to `run()` found none — every other piece of its
logic is tested the same way this task tests this one: by extracting a
small, pure decision function). Extract the early-validate gate's own
condition into a small, directly-testable function:

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

Use this in Step 1's `if` condition (`if should_early_validate(verify_only,
audit_export_mode, cost_mode, &storage_path) { ... }`) in place of the
inline boolean expression, then test the pure function directly:

```rust
#[cfg(test)]
mod early_validate_gate_tests {
    use super::should_early_validate;
    use std::path::Path;

    #[test]
    fn fires_when_store_absent_and_no_special_mode_active() {
        assert!(should_early_validate(false, false, false, Path::new("/nonexistent/path/for/this/test")));
    }

    #[test]
    fn skipped_when_store_already_exists() {
        // `tempfile` is not a dev-dependency of this crate (confirmed —
        // grep `Cargo.toml`); `uuid` already is, so build a throwaway
        // path the same way this session's own aivyx-telegram/-discord/
        // -slack checkpoint tests already do.
        let path = std::env::temp_dir().join(format!(
            "aivyx-early-validate-gate-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, b"").expect("create dummy store file");
        assert!(!should_early_validate(false, false, false, &path));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn skipped_for_verify_only_mode() {
        assert!(!should_early_validate(true, false, false, Path::new("/nonexistent/path")));
    }

    #[test]
    fn skipped_for_audit_export_mode() {
        assert!(!should_early_validate(false, true, false, Path::new("/nonexistent/path")));
    }

    #[test]
    fn skipped_for_cost_mode() {
        assert!(!should_early_validate(false, false, true, Path::new("/nonexistent/path")));
    }
}
```


- [ ] **Step 7: Run the new tests**

Run: `cargo test -p aivyx-cli early_validate_gate -- --nocapture`
Expected: PASS (5 passed; 0 failed).

- [ ] **Step 8: Run the full workspace test suite**

Run: `cargo test` (default-members)
Expected: PASS, 0 failures.

- [ ] **Step 9: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 10: Commit**

```bash
git add crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "feat(aivyx-cli): early-validate gate closes the silent-orphaned-store bug

run() now checks whether the target store exists BEFORE creating any
directories or prompting for a passphrase. If it doesn't exist yet and
config validation would fail anyway (the 'ran bare aivyx before aivyx
init' case), it either offers to run the setup wizard inline (TTY) or
fails immediately with a clear aivyx-init pointer (non-TTY) -- zero
side effects either way. If the store already exists, this new check
is skipped entirely and the existing mkdir/passphrase/open/hydrate/
validate order runs completely unchanged, preserving the
store-can-supply-a-secret fallback for every returning install.

Also threads a new_store: bool into select_passphrase_source, so the
interactive passphrase prompt only requires confirm-reentry (Task 1)
when genuinely creating a new store.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

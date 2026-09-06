# Closing Phase 202's Three Logged Follow-Ups

**Status: approved, ready for implementation planning.**

## Context

Phase 202 (first-launch store safety) shipped with 3 follow-ups explicitly
logged in its own retrospective (`docs/archive/phases/PHASE_202.md`), all
found by that phase's final whole-branch review — one real, two cosmetic.
This spec closes all three.

## 1. `aivyx --verify-only`/`audit export`/`cost` can still create a store on an unconfigured machine

**Grounding.** `should_early_validate` (`crates/aivyx-cli/src/bin/aivyx.rs`)
currently excludes these 3 modes from the early-validate gate:

```rust
fn should_early_validate(
    verify_only: bool,
    audit_export_mode: bool,
    cost_mode: bool,
    storage_path: &std::path::Path,
) -> bool {
    !verify_only && !audit_export_mode && !cost_mode && !storage_path.exists()
}
```

The exclusion exists because `LoadOptions.require_api_key` is `false` for
these 3 modes (`require_api_key: !verify_only && !print_role_mode &&
!audit_export_mode && !cost_mode`, confirmed at the one real call site) —
they never talk to an LLM provider, so `config.validate()` correctly never
fails for them regardless of configuration state. **This means simply
removing the 3-mode exclusion would do nothing**: the gate would fire, call
`config.validate(&load_opts)`, get `Ok(())` back even on a totally fresh
machine, and fall through to the exact same mkdir/passphrase/store-open
sequence as today. The real signal for these 3 modes isn't "config
validation fails" — it's "there's no store yet to verify/export/report on,"
which `validate()` was never designed to detect.

**Approach (confirmed with the user: extend the same mechanism, not a
separate one.)** Simplify `should_early_validate` to a pure existence
check — the 3 mode parameters are no longer needed:

```rust
fn should_early_validate(storage_path: &std::path::Path) -> bool {
    !storage_path.exists()
}
```

Inside the gate, branch on mode to decide what "validation failure" means:

```rust
if should_early_validate(&storage_path) {
    let validation_result: Result<(), String> = if verify_only || audit_export_mode || cost_mode {
        Err(format!(
            "no store exists yet at {storage_path:?} — nothing to verify/export/report on"
        ))
    } else {
        config.validate(&load_opts).map_err(|e| e.to_string())
    };
    if let Err(e) = validation_result {
        // ... unchanged from here: hint, TTY check, decide_unconfigured_first_run, etc.
    }
}
```

Both branches route through the same `decide_unconfigured_first_run`
wizard-offer-or-fail mechanism already built. An interactive
`aivyx --verify-only` on a fresh machine gets the same "run the setup
wizard now? [Y/n]" offer the default chat path already gets; a
non-interactive one fails immediately with a message naming the real
problem plus the `aivyx init` pointer.

**Type note:** `config.validate(&load_opts)` returns `Result<(),
ConfigError>`; the diagnostic-mode branch returns `Result<(), String>`
directly. Both arms of the `if/else` must produce the same type, so the
`ConfigError` arm needs `.map_err(|e| e.to_string())` (shown above) to
unify to `Result<(), String>` — `e` downstream (in the `eprintln!`/`hint`/
`Err` uses) is then always a `String`, not a `ConfigError`.

## 2. Doubled `aivyx:`-prefixed lines on the non-interactive failure path

**Grounding.** The gate currently always `eprintln!`s the full
context+hint before checking `is_tty`, then on the `Fail` decision returns
a short, distinct `Err("setup required — see above")`. In the TTY-declined
case this is sensible (full context shown, then a terse follow-up after
the user says no to the wizard offer). In the non-TTY case it's genuinely
redundant — nothing happened in between the eprintln and the return, so
the user sees two separately-`aivyx:`-prefixed lines carrying what could
have been one message.

**Fix:** only print the early context when interactive; let the
non-interactive path return the full message once, letting `main()`'s
generic handler print it exactly once:

```rust
if let Err(e) = validation_result {
    let hint = "\n\nRun `aivyx init` to set this up (or `aivyx init \
                 --template coder|researcher|personal` for a quick start).";
    let is_tty = io::stdin().is_terminal();
    if is_tty {
        eprintln!("aivyx: {e}{hint}");
    }
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut stderr_writer = io::stderr();
    match init::decide_unconfigured_first_run(is_tty, &mut reader, &mut stderr_writer)? {
        init::UnconfiguredFirstRunDecision::RunWizardInline => {
            // unchanged
        }
        init::UnconfiguredFirstRunDecision::Fail => {
            if is_tty {
                return Err("setup required — see above".to_string());
            }
            return Err(format!("{e}{hint}"));
        }
    }
}
```

Non-TTY now sees exactly one `aivyx:`-prefixed line, in full. TTY-declined
still sees the full context, then a short confirmation after declining —
unchanged from today, since that already reads sensibly.

## 3. Passphrase-mismatch retry message bypasses the testable writer seam

**Grounding.** `read_interactive_password_with_confirm`
(`crates/aivyx-channel/src/passphrase.rs`) takes one closure,
`F: FnMut(&str) -> std::io::Result<String>`, invoked with a literal prompt
string each call. Production passes `|prompt| rpassword::prompt_password(prompt)`;
tests pass `|prompt| rpassword::prompt_password_from_bufread(&mut reader,
&mut sink, prompt)`, which writes `prompt` into `sink` — this is the only
seam either caller has for observing what text was shown. The current
`eprintln!("Passphrases didn't match. Try again.")` bypasses that seam
entirely (writes straight to real stderr, untestable, and prints raw
during `cargo test`).

**Fix (no signature change):** fold the retry message into the *next*
prompt string instead of a separate print call:

```rust
fn read_interactive_password_with_confirm<F>(mut read: F) -> Result<Vec<u8>, PassphraseError>
where
    F: FnMut(&str) -> std::io::Result<String>,
{
    let mut prompt = "aivyx passphrase (new store — you'll need this every time): ";
    loop {
        let first = map_read_result(read(prompt))?;
        let mut confirm = map_read_result(read("Confirm passphrase: "))?;
        if first == confirm {
            confirm.zeroize();
            return Ok(first);
        }
        let mut first = first;
        first.zeroize();
        confirm.zeroize();
        prompt = "Passphrases didn't match. Try again.\n\
                  aivyx passphrase (new store — you'll need this every time): ";
    }
}
```

The retry text now flows through the exact same seam the existing
`confirm_reentry_wording_differs_from_unconfirmed_prompt`-style tests
already assert against (`sink.contains(...)`), with zero new parameters.

## Testing

- **Issue 1**: `should_early_validate`'s existing 5 tests
  (`fires_when_store_absent_and_no_special_mode_active`,
  `skipped_when_store_already_exists`, `skipped_for_verify_only_mode`,
  `skipped_for_audit_export_mode`, `skipped_for_cost_mode`) all need
  updating for the new 1-parameter signature — the 3 mode-exclusion tests
  are no longer meaningful (there's no mode-based exclusion left to test)
  and should be replaced with tests confirming the gate now fires
  identically regardless of mode. New tests needed at the call-site level
  (or via a small extracted pure function, matching this file's own
  established pattern) confirming: diagnostic-mode + no store → the
  "nothing to verify/export/report on" message; diagnostic-mode + store
  exists → skipped entirely, unchanged behavior.
- **Issue 2**: needs a way to assert on the non-TTY output distinctly from
  the TTY-declined output — ground at plan time whether `run()`'s current
  structure allows testing this directly, or whether (matching this
  file's now-repeated pattern for exactly this kind of problem) the
  eprintln/return decision should be extracted into a small pure function
  first.
- **Issue 3**: extend the existing `confirm_reentry_mismatch_reprompts_until_matching`
  test (or add a sibling) to assert the retry prompt's sink output
  contains `"Passphrases didn't match"` — the mismatch test already drives
  exactly this code path, so this is likely a one-line addition to an
  existing test rather than a wholly new one.

## Self-review

- **Placeholder scan:** none — all 3 fixes are given as complete,
  compiling code.
- **Internal consistency:** Issue 1's `validation_result` type
  (`Result<(), String>`) is used consistently through Issue 2's own
  `if let Err(e) = validation_result` — the two issues touch the exact
  same code block and are shown composed together, not as if they were
  independent edits to different copies of the same lines.
- **Scope check:** one phase, 3 small, independent fixes across 2 files
  (`aivyx.rs`, `passphrase.rs`) — smaller than any prior phase this
  session.
- **Ambiguity check:** Issue 1's approach (extend the shared mechanism
  vs. a separate simpler check) was confirmed with the user directly,
  not assumed.

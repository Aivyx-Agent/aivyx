# First-Launch Store Safety: Closing the Silent-Orphaned-Store Bug

**Status: approved, ready for implementation planning.**

## Context

A first-launch audit (this session, `docs/BACKEND_AUDIT_2026-06-16.md`-style
direct code tracing, not doc inference) found a real bug chain triggered by
the plausible, undocumented-but-common path of running bare `aivyx` before
ever running `aivyx init`:

1. No `aivyx.toml` exists. `crates/aivyx-cli/src/bin/aivyx.rs`'s `run()` has
   no check anywhere for this — confirmed by a full-file grep, the only
   `aivyx init` mention in the whole binary is an unrelated arg-parsing
   error string. `aivyx-config/src/lib.rs:5692` shows `provider` silently
   defaults to `ProviderKind::Anthropic` (`FieldSource::Default`) when unset
   everywhere.
2. `fs_root`/`storage_path` resolve to their HOME-derived defaults —
   confirmed identical to `init.rs`'s own `default_paths()` (line
   1512-1517): `$HOME/.local/share/aivyx/store.redb`. Both directories get
   created (`std::fs::create_dir_all`) unconditionally.
3. `select_passphrase_source` (`aivyx.rs:4943-4975`) finds nothing
   configured anywhere and, seeing a real TTY, returns
   `PassphraseSource::InteractivePrompt`.
4. `derive_master_key` → `fetch_passphrase_bytes`
   (`aivyx-channel/src/passphrase.rs:341-352`) calls
   `rpassword::prompt_password("aivyx passphrase: ")` — **one unconfirmed
   entry**, generic wording, no indication this creates a new store vs.
   unlocks an existing one, no confirm-reentry (unlike `aivyx keyring set`,
   which prompts + confirms + checks match, `aivyx.rs:4982-4991`).
5. `RedbStorage::open` (`aivyx-storage/src/lib.rs:567-597`,
   `Database::create(&path)`) genuinely creates a **permanent, 0600-chmod'd
   `.redb` file on disk** — confirmed real cold-start file creation, not
   lazy — before any config validation runs.
6. Only then does `config.validate(&load_opts)` (`aivyx.rs:1320`, called
   *after* `hydrate_secrets_from_store` and the store-open above) finally
   fire and return `ConfigError::Missing { field: "anthropic_api_key" }`,
   surfaced via `main()`'s bare `eprintln!("aivyx: {e}")` — never mentioning
   `aivyx init`.

**The compounding half:** `init.rs`'s own overwrite guard
(`run_init_wizard_inner`, ~line 1962-1975) only checks `config_path.exists()`
(`aivyx.toml`), never whether a store already exists at the target
`storage_path`. Since both paths resolve identically, a later, genuine
`aivyx init` run happily writes a fresh `aivyx.toml` pointing at the store
the accidental run already sealed under an unconfirmed, likely-forgotten
passphrase. A different passphrase on the "real" first launch afterward
means `RedbStorage::open`'s key derivation won't match, and it fails to
decrypt — an opaque `"failed to open encrypted store at {storage_path:?}:
{e}"` with zero indication of the real cause.

**Verified NOT already covered**: grepped `docs/*.md`/`README.md` for
`orphaned`/`store already exists`/`passphrase mismatch`/`decrypt fail` — the
2 hits found were unrelated to this scenario. Not logged in
`ROADMAP.md`/`PHASE_*.md` history either. `docs/LOCAL_FIRST_RUN.md`
(Chapter P) hardens a different, adjacent first-run risk (the Ollama
empty-reply problem for users who *did* complete `init`) — this bug lives
one step earlier, for users who skip `init` altogether.

**Confirmed NOT a real risk for documented paths**: the README's own
shell-installer → `aivyx init` path, and its manual-quickstart TOML snippet
(which includes `[aivyx] passphrase = "set-a-real-passphrase"`), both either
run the wizard or pre-supply a passphrase before ever reaching the
interactive prompt. Docker/systemd deployments were checked and confirmed
to always bake an `aivyx.toml` (`docs/DOCKER.md:251`) even when using env
overrides — so "no `aivyx.toml` exists" is a safe signal, not a false
positive on a legitimate env-only production deployment.

## Approach

Three related, small mechanisms, one phase. All three touch the same root
cause (the interactive-first-run path getting insufficient care) and share
context (mechanism 1 and 3 both need to know whether the store already
exists), so they're scoped together rather than split.

### 1. Early-validate gate (closes the root cause)

The real bug isn't that `validate()`'s check is wrong — it's that it runs
*after* side effects that should never have happened yet. `validate()` is
deliberately placed after `hydrate_secrets_from_store` so an *existing*
store's saved secret can satisfy a required field — that's correct for a
returning user, and must not regress.

The fix: in `run()`, right where `config.storage_path.value` is resolved
(currently line 1120, immediately before the sandbox/storage `mkdir` calls),
check `storage_path.exists()` *first*:

- **Store doesn't exist yet** (this would be a first-ever store on this
  machine): call `config.validate(&load_opts)` immediately, before any
  `mkdir`, before any passphrase prompt, before `RedbStorage::open`.
  - Validate passes → fall through, proceed exactly as today (a fully
    env-configured deployment creating its first store sees zero behavior
    change).
  - Validate fails → this is the "unconfigured first run" case:
    - **stdin is a TTY**: print the validation error plus an explicit
      `aivyx init` pointer, then ask *"Run the setup wizard now? [Y/n]"*
      (default yes, reusing `init.rs`'s existing `prompt_yes_no` shape —
      made `pub(crate)` for this one new caller). On yes: build a
      lightweight one-shot tokio runtime (the same pattern
      `InitMode::Interactive` already uses at `aivyx.rs:565-569`), run
      `init::run_init_wizard(None)`, then print `"Now run \`aivyx\` again
      to start."` and return `Ok(())` — not attempting to resume the
      interrupted session in the same process. On no: fail with the
      validation error plus the `aivyx init` pointer.
    - **not a TTY** (scripts, CI, a misconfigured systemd unit): fail
      immediately with the same error-plus-pointer text — no prompt.
- **Store already exists**: skip this entire new branch. The existing
  order (mkdir → passphrase select/derive → `RedbStorage::open` → hydrate
  secrets from store → `validate`) runs completely unchanged — zero
  behavior change for every returning install, including ones that rely on
  a store-sourced secret.

This composes cleanly with existing mode flags without new special-casing:
`verify_only`/`audit_export_mode`/`cost_mode`/`print_role` already either
set `require_api_key: false` in their `LoadOptions` or return before this
point in `run()`, so `validate()` is naturally a no-op or unreached for
them.

### 2. `aivyx init`'s store-collision guard

At the point `storage_path` is finalized in the wizard (`init.rs`, right
after the "Storage path [default]:" prompt resolves, ~line 2195-2204,
before config is rendered/written), add a check mirroring the existing
`aivyx.toml`-overwrite guard's shape: if `Path::new(&storage_path).exists()`,
print a warning and require explicit confirmation (`prompt_yes_no(...,
false, ...)` — default **No**, since continuing doesn't delete anything but
a later passphrase mismatch means a silent lockout):

```
⚠ A store already exists at {storage_path}
  (possibly from an earlier run). Continuing will NOT delete it — your
  passphrase on first launch must match the one it was created under.
  If you don't know that passphrase, delete the file first.
```

Answering no aborts the wizard cleanly, the same way declining the
`aivyx.toml`-overwrite prompt already does.

### 3. Passphrase confirm-reentry, new-store-only

`PassphraseSource::InteractivePrompt` (a unit variant today,
`aivyx-channel/src/passphrase.rs:174`) becomes
`InteractivePrompt { confirm: bool }`. `select_passphrase_source`
(`aivyx.rs:4943`) gains a `new_store: bool` parameter — its one real caller
in `run()` already has `storage_path.exists()` available at the call site
(same fact mechanism 1 needs), so this is a one-line threading change, not
new detection logic.

`fetch_passphrase_bytes`'s `InteractivePrompt { confirm }` arm
(`passphrase.rs:341-352`) branches:
- `confirm: false` (unlocking an existing store) — unchanged: one prompt,
  no confirmation. A wrong guess here fails cleanly at decrypt time and the
  user just retries the command; nothing is silently created or locked in.
- `confirm: true` (creating a brand-new store) — prompts, prompts again
  ("Confirm passphrase: "), loops back to re-prompt the whole pair on
  mismatch, mirroring `aivyx keyring set`'s existing shape exactly
  (`aivyx.rs:4982-4991`). The initial prompt's wording also changes to make
  the stakes clear: `"aivyx passphrase (new store — you'll need this every
  time): "` instead of the plain `"aivyx passphrase: "` used for unlocking.

The `Debug` impl for `PassphraseSource` (`passphrase.rs:193`) and the one
existing unit test constructing `PassphraseSource::InteractivePrompt`
(`passphrase.rs:890`) both need updating for the new field — grounded at
plan time, not expected to be more than mechanical churn (the whole-codebase
grep found exactly these 2 non-production references plus the 1 production
construction site, no others).

## Testing

- **`aivyx-config`**: no change needed — `validate()`'s own behavior is
  unchanged; only *when* it's called moves.
- **`aivyx-channel`**: update the existing `PassphraseSource::InteractivePrompt`
  test construction for the new field; add tests for `fetch_passphrase_bytes`
  under `confirm: true` — mismatched pair re-prompts (using the existing
  `prompt_password_from_bufread` test seam), matching pair succeeds,
  wording differs from the `confirm: false` case.
- **`aivyx-cli`**: this is the integration-heavy crate —
  - A test driving `run()`'s (or a suitably extracted pure decision
    function's) early-validate branch: store doesn't exist + validate
    fails + non-TTY → clean error mentioning `aivyx init`, zero directories
    or files created (assert on a fresh temp `HOME`).
  - Store doesn't exist + validate passes (e.g. full env config) → falls
    through unchanged, confirmed by diffing behavior against the current
    (pre-fix) code path on the same fixture.
  - Store already exists → the new branch is skipped entirely; existing
    tests covering the current mkdir/passphrase/open/hydrate/validate order
    must still pass unmodified — this is the regression guard proving
    returning-user behavior is untouched.
  - `init.rs`'s new store-collision guard: a test writing a dummy file at
    the target storage path before running the wizard through a scripted
    reader confirms the warning fires and a "n" answer aborts cleanly; a
    "y" answer proceeds and writes the config.
  - The interactive-offer-to-run-init branch is harder to drive through a
    full `run()` test (it spawns a nested wizard) — ground at plan time
    whether to test it via a scripted end-to-end run or extract the
    yes/no decision into a small pure function (mirroring `init.rs`'s own
    `ServiceInstallDecision` pattern, `init.rs:521-533`) that's unit-tested
    directly, with the actual wizard invocation left as a thin, untested
    wrapper. The latter is preferred if it doesn't distort the control
    flow — matches this codebase's established pattern for exactly this
    kind of "OS-call-adjacent decision" testability problem.

## Self-review

- **Placeholder scan**: none — every touch point is grounded against real,
  current line numbers and function signatures read this session. The one
  deliberately-deferred decision (pure-decision-function extraction vs.
  end-to-end test for the interactive-offer branch) is flagged explicitly
  as a plan-time grounding question, not a vague TODO.
- **Internal consistency**: mechanisms 1 and 3 both key off the same
  `storage_path.exists()` fact, checked once at the same point in `run()`
  and threaded to both — no risk of the two mechanisms disagreeing about
  whether a store is "new."
- **Scope check**: one phase, 3 small mechanisms across 3 files
  (`aivyx.rs`, `aivyx-channel/src/passphrase.rs`, `init.rs`) — smaller than
  the channel-session-turnsafety-config phase that immediately preceded
  this one in the same session.
- **Ambiguity check**: all three design decisions (interactive-offer vs.
  fail-fast; bundling the store-collision guard; bundling the confirm-
  reentry fix) were confirmed with the user via explicit questions during
  brainstorming, not assumed.

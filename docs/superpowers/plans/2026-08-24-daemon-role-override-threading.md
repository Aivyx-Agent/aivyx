# Daemon `role_override` Threading Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Thread the daemon's already-computed real `--role` value through
`DaemonConfig` so both config-re-read call sites (`channel_trigger_authz`
and Chapter U's `settings_applied`) resolve against it instead of a
hardcoded `None`, closing the deferred Piece C finding.

**Architecture:** One new `Option<String>` field, threaded through the
exact same 3-layer path `config_toml_path` already uses today
(`DaemonConfig` → `ConnectionContext` → `handle_query`'s own parameter),
plus one new parameter on the two functions that build `LoadOptions`
(`load_settings_config`, called from both real sites). No new resolution
logic — the value already exists in `aivyx.rs`; this only carries it
further.

**Tech Stack:** Rust, `aivyx-channel`/`aivyx-cli`/`aivyx-config` crates
already in the workspace. No new dependencies.

## Global Constraints

- No new resolution logic — `role_override`'s value at the one real
  production site is the exact same local variable already used to build
  the daemon's primary `LoadOptions` in `aivyx.rs`.
- The 8 non-production `DaemonConfig` construction sites (the compat
  wrapper + 7 test fixtures) get `role_override: None` — mechanical, no
  behavior change, since none of them exercise a non-default role today.
- All 7 existing tests in `crates/aivyx-channel/tests/daemon_roundtrip_e2e.rs`
  must keep passing **unchanged**.
- Every new test must be a genuine mutation-proof, shown to actually fail
  against the code as it exists at the start of this plan (hardcoded
  `None` in `load_settings_config`).
- `cargo build -p aivyx-channel -p aivyx-cli` clean. Do **not** run `cargo
  build --workspace`/`cargo test --workspace` — the full workspace has an
  unrelated, pre-existing, out-of-scope build failure (`javascriptcoregtk-4.1`
  missing system library in a GUI crate).

---

### Task 1: Thread `role_override` through `DaemonConfig` to both re-read call sites

**Files:**
- Modify: `crates/aivyx-channel/src/daemon_server.rs` — `DaemonConfig`
  (struct field + all 9 construction/destructure sites),
  `ConnectionContext` (struct field + construction + destructure),
  `handle_query` (new parameter + call-site argument), `load_settings_config`
  (new parameter), `settings_applied` (new parameter, threaded to its own
  call to `load_settings_config`), the `channel_trigger_authz` build (pass
  the new value instead of relying on the hardcoded default inside
  `load_settings_config`).
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` — the one real
  `DaemonConfig { ... }` construction site, passing the existing
  `role_override` local variable.
- Modify: `crates/aivyx-channel/tests/daemon_roundtrip_e2e.rs` — 7
  construction sites, each gets `role_override: None,` added.
- Test: new tests in `crates/aivyx-channel/src/daemon_server.rs`'s own
  `#[cfg(test)] mod tests` block.

Re-run these greps before editing — this plan's own line numbers were
current as of this plan's writing but may have drifted:

```bash
cd /home/julian/Projects/Rust/aivyx
grep -n "pub struct DaemonConfig\|pub config_toml_path: Option<PathBuf>\|pub team_config_write_path" crates/aivyx-channel/src/daemon_server.rs
grep -n "let DaemonConfig {\|config_toml_path,\|team_config_write_path,\|config_toml_path: config_toml_path.clone()" crates/aivyx-channel/src/daemon_server.rs
grep -n "struct ConnectionContext\|config_toml_path: Option<PathBuf>,\|fn handle_query\|fn load_settings_config\|fn settings_applied\|handle_query(" crates/aivyx-channel/src/daemon_server.rs
grep -n "config_toml_path.as_deref()\|team_config_write_path.as_deref()" crates/aivyx-channel/src/daemon_server.rs
grep -n "DaemonConfig {" crates/aivyx-channel/tests/daemon_roundtrip_e2e.rs crates/aivyx-cli/src/bin/aivyx.rs
grep -n "let load_opts = LoadOptions\|role_override: print_role" crates/aivyx-cli/src/bin/aivyx.rs
grep -n "fn test_dir" crates/aivyx-channel/src/daemon_server.rs
```

Confirm the shapes below still match; use whatever's actually current for
every step if they've drifted.

**Interfaces:**
- Produces: `DaemonConfig.role_override: Option<String>` — a new public
  field, no other signature changes to the struct.
- Produces: `load_settings_config(toml_path: &Path, role_override: Option<&str>) -> Result<aivyx_config::AivyxConfig, String>`
  — the two existing callers (`channel_trigger_authz`'s build and
  `settings_applied`) both gain this second argument.
- Produces: `settings_applied(toml_path: &Path, embeddings_available: bool, role_override: Option<&str>) -> QueryResponsePayload`
  — its 4 existing call sites (inside `handle_query`) each gain this third
  argument.
- Produces: `handle_query`'s own parameter list gains one new parameter,
  `role_override: Option<&str>`, inserted immediately after
  `config_toml_path: Option<&Path>` (matching that field's own position,
  for readability — exact position doesn't matter functionally since
  these are named-in-order positional args, but keep it adjacent since
  it's used for the same purpose).

- [ ] **Step 1: Write the failing unit test for `load_settings_config`**

Add this test to `daemon_server.rs`'s own `#[cfg(test)] mod tests` block,
near the existing `fn test_dir` helper (search for it):

```rust
    #[test]
    fn load_settings_config_resolves_a_non_default_role_when_overridden() {
        let dir = test_dir("role-override-threading");
        let toml_path = dir.join("aivyx.toml");
        std::fs::write(
            &toml_path,
            r#"
[[role]]
name = "custom"
system_prompt = "You are a custom role."
"#,
        )
        .unwrap();

        // Without the override, active-role resolution defaults to
        // "default", which this config doesn't declare -- UnknownRole.
        let without_override = load_settings_config(&toml_path, None);
        assert!(
            without_override.is_err(),
            "a config with only a non-default-named role must fail to \
             load without an override naming it"
        );

        // With the override, the real bug this task fixes: the daemon's
        // own re-read must resolve against the SAME role the primary
        // load used, not silently fall back to the "default" name.
        let with_override = load_settings_config(&toml_path, Some("custom"));
        assert!(
            with_override.is_ok(),
            "load_settings_config must accept a role_override and use it: {:?}",
            with_override.err()
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd /home/julian/Projects/Rust/aivyx
cargo test -p aivyx-channel --lib load_settings_config_resolves_a_non_default_role -- --test-threads=1
```

Expected: FAIL — compile error, `load_settings_config` doesn't yet take a
second argument.

- [ ] **Step 3: Add the field to `DaemonConfig` and thread it through `run_daemon`'s own destructure**

In `crates/aivyx-channel/src/daemon_server.rs`, immediately after the
`pub config_toml_path: Option<PathBuf>,` field's own doc comment and
declaration inside `DaemonConfig` (search for
`pub config_toml_path: Option<PathBuf>,` inside the `DaemonConfig` struct,
not the later `ConnectionContext` one), add:

```rust
    /// Piece C follow-up — the same value the daemon's own primary
    /// config load resolved its active role from (`LoadOptions::role_override`
    /// at the binary's own startup call). Threaded through so the
    /// re-reads below (`channel_trigger_authz`, Chapter U's
    /// `settings_applied`) resolve against the SAME role, instead of
    /// hardcoding `None` and risking `ConfigError::UnknownRole` for any
    /// operator running a non-default `--role`. `None` is correct for an
    /// env-only launch or a genuinely-default-role deployment — this
    /// field is not itself a resolution mechanism, only a carried value.
    pub role_override: Option<String>,
```

In `run_daemon`'s own `let DaemonConfig { ... } = config;` destructure,
add `role_override,` immediately after `config_toml_path,` (search for
`config_toml_path,` inside that destructure pattern — the one right after
`pub async fn run_daemon`, not any other destructure later in the file).

- [ ] **Step 4: Update the `channel_trigger_authz` build to pass the new value**

Change the `channel_trigger_authz` build's call to `load_settings_config`
(search for `let channel_trigger_authz = match config_toml_path.as_deref()`)
from:

```rust
    let channel_trigger_authz = match config_toml_path.as_deref() {
        Some(p) => match load_settings_config(p) {
```

to:

```rust
    let channel_trigger_authz = match config_toml_path.as_deref() {
        Some(p) => match load_settings_config(p, role_override.as_deref()) {
```

- [ ] **Step 5: Thread through `ConnectionContext`**

In `ConnectionContext`'s own struct definition, immediately after its
`config_toml_path: Option<PathBuf>,` field (search for `struct
ConnectionContext` first to confirm you're editing the right struct — this
one has module-private fields, not `pub`, unlike `DaemonConfig`'s), add:

```rust
    /// Piece C follow-up — see `DaemonConfig::role_override`'s own doc
    /// comment; threaded here so `handle_query`'s Settings handlers can
    /// resolve against the daemon's own real active role too.
    role_override: Option<String>,
```

In the `ConnectionContext { ... }` construction inside `run_daemon` (search
for `config_toml_path: config_toml_path.clone(),` — the construction, not
the destructure), add immediately after it:

```rust
            role_override: role_override.clone(),
```

In `handle_connection`'s own `let ConnectionContext { ... } = ctx;`
destructure, add `role_override,` immediately after `config_toml_path,`.

- [ ] **Step 6: Thread through `handle_query`'s call and signature**

At `handle_connection`'s call to `handle_query(...)` (search for
`let response_payload = handle_query(`), add
`role_override.as_deref(),` immediately after the existing
`config_toml_path.as_deref(),` argument.

In `handle_query`'s own signature (search for `async fn handle_query(`),
add a new parameter immediately after `config_toml_path: Option<&Path>,`:

```rust
    // Piece C follow-up — see `DaemonConfig::role_override`'s own doc
    // comment.
    role_override: Option<&str>,
```

- [ ] **Step 7: Thread through `settings_applied` and its 4 call sites**

Change `settings_applied`'s own signature from:

```rust
fn settings_applied(toml_path: &Path, embeddings_available: bool) -> QueryResponsePayload {
    match load_settings_config(toml_path) {
```

to:

```rust
fn settings_applied(
    toml_path: &Path,
    embeddings_available: bool,
    role_override: Option<&str>,
) -> QueryResponsePayload {
    match load_settings_config(toml_path, role_override) {
```

Then update each of its 4 call sites inside `handle_query`'s own body
(search for `settings_applied(path, embedding_provider.is_some())` — it
appears 4 times, verbatim identical each time) to:

```rust
                    settings_applied(path, embedding_provider.is_some(), role_override)
```

- [ ] **Step 8: Update `load_settings_config`'s own signature**

Change:

```rust
fn load_settings_config(toml_path: &Path) -> Result<aivyx_config::AivyxConfig, String> {
    let opts = aivyx_config::LoadOptions {
        toml_path: Some(toml_path.to_path_buf()),
        require_api_key: false,
        require_telegram_token: false,
        require_discord_token: false,
        require_slack_tokens: false,
        role_override: None,
    };
    aivyx_config::AivyxConfig::load_from_env_and_toml(&opts)
        .map_err(|e| format!("failed to load {}: {e}", toml_path.display()))
}
```

to:

```rust
fn load_settings_config(
    toml_path: &Path,
    role_override: Option<&str>,
) -> Result<aivyx_config::AivyxConfig, String> {
    let opts = aivyx_config::LoadOptions {
        toml_path: Some(toml_path.to_path_buf()),
        require_api_key: false,
        require_telegram_token: false,
        require_discord_token: false,
        require_slack_tokens: false,
        role_override: role_override.map(str::to_string),
    };
    aivyx_config::AivyxConfig::load_from_env_and_toml(&opts)
        .map_err(|e| format!("failed to load {}: {e}", toml_path.display()))
}
```

- [ ] **Step 9: Run the unit test to verify it passes**

```bash
cd /home/julian/Projects/Rust/aivyx
cargo test -p aivyx-channel --lib load_settings_config_resolves_a_non_default_role -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 10: Fix the 8 non-production `DaemonConfig` construction sites**

In `crates/aivyx-channel/src/daemon_server.rs`'s `run_daemon_compat`
(search for `run_daemon(DaemonConfig {`), add `role_override: None,`
anywhere in the struct literal (matching the style of its neighboring
`None` fields).

In `crates/aivyx-channel/tests/daemon_roundtrip_e2e.rs`, there are 7
`DaemonConfig { ... }` literals (search for `run_daemon(DaemonConfig {` —
confirm the count is still 7; add or remove steps here if it's drifted).
Add `role_override: None,` to each one, matching the style of its
neighboring fields.

- [ ] **Step 11: Wire the real production site in `aivyx.rs`**

In `crates/aivyx-cli/src/bin/aivyx.rs`, find the real `DaemonConfig { ... }`
construction (search for `let result = run_daemon(DaemonConfig {`). Add
`role_override: role_override.clone(),` to the struct literal — reusing
the exact same `role_override` local variable already used earlier in this
same function to build the primary `LoadOptions` (search for
`role_override: print_role.clone().or(role_override),` to confirm this
variable's real name and that it's still in scope at the `DaemonConfig`
construction site; if the variable has been renamed or shadowed between
those two points, use whatever the real current variable holding the same
value is called, and note the discrepancy in your report).

- [ ] **Step 12: Run the full build and test suites**

```bash
cd /home/julian/Projects/Rust/aivyx
cargo build -p aivyx-channel -p aivyx-cli
cargo test -p aivyx-channel --lib -- --test-threads=1
cargo test -p aivyx-channel --test daemon_roundtrip_e2e -- --test-threads=1
cargo test -p aivyx-cli --bin aivyx -- --test-threads=1
cargo clippy -p aivyx-channel --lib --tests -- -D warnings
cargo clippy -p aivyx-cli --bin aivyx -- -D warnings
```

Expected: clean builds; `aivyx-channel --lib` passes at baseline + 1 new
test; all 7 `daemon_roundtrip_e2e` tests pass unchanged; `aivyx-cli` 568/568
unchanged; clippy clean except the pre-existing, unrelated
`trigger.rs:223` finding.

- [ ] **Step 13: Commit**

```bash
cd /home/julian/Projects/Rust/aivyx
git add crates/aivyx-channel/src/daemon_server.rs \
        crates/aivyx-channel/tests/daemon_roundtrip_e2e.rs \
        crates/aivyx-cli/src/bin/aivyx.rs
git commit -m "Thread the daemon's real active role through both config re-reads

DaemonConfig.role_override carries the same value the daemon's own
primary config load already resolved its active role from, threaded
through the existing ConnectionContext/handle_query path
(config_toml_path's own pattern, one field over) to both places that
re-read aivyx.toml from disk: channel_trigger_authz's /team run
authorization build, and Chapter U's settings_applied handler. Closes
the deferred Piece C finding -- an operator running a non-default
--role no longer hits a spurious ConfigError::UnknownRole on either
re-read.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

# KV-Cache Store Path Sharing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an operator configure both `aivyx` and `aivyx-coder` to use the same on-disk KV-cache store directory, so a delegated `aivyx-coder --mcp-server` subprocess genuinely shares prefill work with `aivyx`'s own daemon when both point at the same `llama-server`.

**Architecture:** Each repo gets one new optional config field and one new "effective path" resolver function that is the single source of truth for "what kvcache path does this run actually use" — reused by both the real kvcache-construction call site and that repo's own sensitive-path security guard, so the two can never silently diverge. `aivyx-kvcache` itself needs no changes — its manifest is already WAL-mode sqlite specifically for cross-process safety.

**Tech Stack:** Rust, `serde`/`toml` config loading (each repo's own existing `aivyx-config` crate), no new dependencies.

## Global Constraints

- Each repo's new resolver function must be the *only* place that repo computes the effective kvcache path — both the kvcache-construction call site and the security-guard call site call it, never recompute the default independently.
- The security fix in each repo must track the *override*, not just the default — proven by a test where an override is set and the override path (not the old hardcoded default) ends up protected.
- No changes to `aivyx-kvcache` itself — its cross-process safety (WAL-mode manifest, double-delete-tolerant eviction) is already confirmed sufficient.
- `aivyx`'s own test/lint commands are bare `cargo test` / `cargo clippy --all-targets -- -D warnings` — **not** `--workspace` (that pulls in `aivyx-desktop`, which fails locally on an unrelated missing system `webkit2gtk` package).
- `aivyx-coder`'s own test/lint commands are `cargo test --workspace` / `cargo clippy --workspace --all-targets` (its own `CLAUDE.md`) — this repo has no equivalent excluded-crate problem.

---

### Task 1: `aivyx-coder` — configurable kvcache path + deny_paths fix

**Files:**
- Modify: `crates/aivyx-config/src/lib.rs` (in the `aivyx-coder` repo)
- Modify: `crates/aivyx/src/agent_builder.rs` (in the `aivyx-coder` repo)

**Interfaces:**
- Consumes: nothing from another task (independent).
- Produces: `BackendSettings::resolved_kvcache_store_path(&self) -> PathBuf` and `Settings::effective_deny_paths(&self) -> Vec<PathBuf>` — not consumed by any other task in this plan, but these are the two names Task 3's documentation refers to by behavior (not by direct code dependency).

- [ ] **Step 1: Write the failing tests for the new resolver method**

In `crates/aivyx-config/src/lib.rs`, find the `#[cfg(test)] mod tests` block (search for `fn default_deny_paths_includes_the_kvcache_directory`, since that existing test sits in the same module and will be edited in Step 5). Add these two new tests near it:

```rust
    #[test]
    fn resolved_kvcache_store_path_matches_the_historical_default_when_unset() {
        let settings = Settings::default();
        let path = settings.backend.resolved_kvcache_store_path();
        assert!(
            path.to_string_lossy().contains(".local/share/aivyx-coder/kvcache"),
            "default kvcache path must be unchanged when no override is configured, got {path:?}"
        );
    }

    #[test]
    fn resolved_kvcache_store_path_uses_the_configured_override() {
        let mut settings = Settings::default();
        settings.backend.kvcache_store_path = Some("~/shared-kvcache".to_string());
        let path = settings.backend.resolved_kvcache_store_path();
        assert!(
            path.ends_with("shared-kvcache"),
            "overridden kvcache path must be used verbatim (tilde-expanded), got {path:?}"
        );
        assert!(
            !path.to_string_lossy().contains("aivyx-coder/kvcache"),
            "overridden path must replace the default, not sit alongside it, got {path:?}"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test -p aivyx-config resolved_kvcache_store_path`
Expected: FAIL — `no field kvcache_store_path on type BackendSettings` / `no method named resolved_kvcache_store_path`.

- [ ] **Step 3: Add the `kvcache_store_path` field and the resolver method**

In `crates/aivyx-config/src/lib.rs`, find the `BackendSettings` struct (search for `pub struct BackendSettings`). Its last field is currently:

```rust
    pub kvcache_max_bytes: u64,
}
```

Add the new field right after it, inside the struct:

```rust
    pub kvcache_max_bytes: u64,
    /// Overrides where the kvcache store directory lives. `None`
    /// (default) preserves the historical per-app `ProjectDirs`-derived
    /// path. Set this to the *same* directory as `aivyx`'s own
    /// `kvcache_store_path` (and point both configs' backends at the
    /// same `llama-server`) to share prefill work across the two
    /// processes — see `docs/MCP_RECIPES.md`'s `aivyx-coder` recipe in
    /// the `aivyx` repo for the full pairing guidance. Supports a
    /// leading `~`, same convention as `deny_paths`.
    pub kvcache_store_path: Option<String>,
}
```

Find `BackendSettings`'s `Default` impl (search for `impl Default for BackendSettings`). Its last field assignment is currently:

```rust
            kvcache_max_bytes: 10 * 1024 * 1024 * 1024,
        }
    }
}
```

Add the new field's default:

```rust
            kvcache_max_bytes: 10 * 1024 * 1024 * 1024,
            kvcache_store_path: None,
        }
    }
}
```

Find `SandboxSettings`'s `impl` block (search for `impl SandboxSettings` — it contains `resolved_extra_read_paths`). Add a new `impl BackendSettings` block right after it (before `#[derive(Debug, Clone, Serialize, Deserialize)]\npub struct BackendSettings {`):

```rust
impl BackendSettings {
    /// The kvcache store directory this run actually uses: the
    /// configured override (tilde-expanded, same convention as
    /// `PermissionSettings::resolved_deny_paths`), or the historical
    /// `ProjectDirs`-derived default when unset. Single source of truth
    /// reused by both the real kvcache construction in `agent_builder.rs`
    /// and `Settings::effective_deny_paths` below -- the two can never
    /// silently diverge.
    pub fn resolved_kvcache_store_path(&self) -> PathBuf {
        match &self.kvcache_store_path {
            Some(raw) => resolve_tilde_paths(std::slice::from_ref(raw))
                .into_iter()
                .next()
                .unwrap_or_else(|| PathBuf::from(raw)),
            None => match directories::ProjectDirs::from("", "", "aivyx-coder") {
                Some(dirs) => dirs.data_local_dir().join("kvcache"),
                None => std::env::temp_dir().join("aivyx-coder").join("kvcache"),
            },
        }
    }
}

```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p aivyx-config resolved_kvcache_store_path`
Expected: `test result: ok. 2 passed`.

- [ ] **Step 5: Write the failing test for the deny_paths fix, then fix it**

Find the existing test `default_deny_paths_includes_the_kvcache_directory` (in the same test module):

```rust
    #[test]
    fn default_deny_paths_includes_the_kvcache_directory() {
        let settings = Settings::default();
        assert!(
            settings
                .permissions
                .resolved_deny_paths()
                .iter()
                .any(|p| p.to_string_lossy().contains(".local/share/aivyx-coder/kvcache")),
            "kvcache store directory must be in default deny_paths, same rationale as the \
             state directory"
        );
    }
```

Replace it with (renamed, and updated to call the new method):

```rust
    #[test]
    fn default_effective_deny_paths_includes_the_kvcache_directory() {
        let settings = Settings::default();
        assert!(
            settings
                .effective_deny_paths()
                .iter()
                .any(|p| p.to_string_lossy().contains(".local/share/aivyx-coder/kvcache")),
            "kvcache store directory must be in effective deny_paths, same rationale as the \
             state directory"
        );
    }

    #[test]
    fn effective_deny_paths_tracks_an_overridden_kvcache_path_not_the_old_default() {
        let mut settings = Settings::default();
        settings.backend.kvcache_store_path = Some("~/shared-kvcache".to_string());
        let paths = settings.effective_deny_paths();
        assert!(
            paths.iter().any(|p| p.ends_with("shared-kvcache")),
            "the overridden path must be protected"
        );
        assert!(
            !paths
                .iter()
                .any(|p| p.to_string_lossy().contains("aivyx-coder/kvcache")),
            "the stale default path must not remain protected once overridden away from it"
        );
    }
```

Run: `cargo test -p aivyx-config effective_deny_paths`
Expected: FAIL — `no method named effective_deny_paths on type Settings`, and the old test name (`default_deny_paths_includes_the_kvcache_directory`) no longer exists so isn't found by name either.

Now remove the static kvcache literal from `PermissionSettings::default()`. Find this block inside `impl Default for PermissionSettings`:

```rust
                // Same rationale as ~/.local/state/aivyx-coder above, for
                // the kvcache store: a restored `.slot` file IS the
                // model's context re-entering a future session invisibly
                // — without this, a generic write_file/edit_file could
                // plant or corrupt cache state a later session's kvcache
                // restore would silently trust.
                "~/.local/share/aivyx-coder/kvcache".to_string(),
```

Delete that entire block (the comment and the string literal line) from the `deny_paths: vec![...]` list.

Add the new `Settings::effective_deny_paths` method. Find `impl PermissionSettings` (containing `resolved_deny_paths`) and add a new `impl Settings` block after it:

```rust
impl Settings {
    /// The complete, resolved deny_paths list this run actually uses:
    /// `permissions.resolved_deny_paths()` plus the effective kvcache
    /// store path (default or overridden), which must always be
    /// protected regardless of whether kvcache is enabled for this
    /// particular run -- a previous run may have left slot files behind
    /// under a path this run's `backend.kind` no longer even selects.
    pub fn effective_deny_paths(&self) -> Vec<PathBuf> {
        let mut paths = self.permissions.resolved_deny_paths();
        paths.push(self.backend.resolved_kvcache_store_path());
        paths
    }
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p aivyx-config`
Expected: all tests in this crate pass, including the two new/renamed ones from Step 5 and the two from Step 1.

- [ ] **Step 7: Wire the resolver into the real call sites in `agent_builder.rs`**

In `crates/aivyx/src/agent_builder.rs`, find:

```rust
    let deny_paths = settings.permissions.resolved_deny_paths();
```

Replace with:

```rust
    let deny_paths = settings.effective_deny_paths();
```

Find the kvcache-construction block (search for `ProjectDirs::from("", "", "aivyx-coder")`):

```rust
                        let store_path = match directories::ProjectDirs::from("", "", "aivyx-coder")
                        {
                            Some(dirs) => dirs.data_local_dir().join("kvcache"),
                            None => std::env::temp_dir().join("aivyx-coder").join("kvcache"),
                        };
```

Replace with:

```rust
                        let store_path = settings.backend.resolved_kvcache_store_path();
```

- [ ] **Step 8: Run the full workspace test suite and clippy**

Run: `cargo test --workspace`
Expected: all tests pass (same or greater count than before this task started).

Run: `cargo clippy --workspace --all-targets`
Expected: clean, zero warnings.

- [ ] **Step 9: Commit**

```bash
git add crates/aivyx-config/src/lib.rs crates/aivyx/src/agent_builder.rs
git commit -m "feat: make the kvcache store path configurable, fix deny_paths to track it

Adds BackendSettings::kvcache_store_path (Option<String>, tilde-expanded,
None preserves the historical ProjectDirs-derived default) and
BackendSettings::resolved_kvcache_store_path() as the single source of
truth for the effective path, reused by both the real kvcache
construction in agent_builder.rs and the new Settings::effective_deny_paths().

Also fixes a latent gap: the kvcache directory was a static string in
PermissionSettings::default()'s deny_paths, blind to any override.
effective_deny_paths() now always protects whatever path is actually in
effect this run, default or overridden."
```

---

### Task 2: `aivyx` — configurable kvcache path + Ward fix

**Files:**
- Modify: `crates/aivyx-config/src/lib.rs` (in the `aivyx` repo)
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs` (in the `aivyx` repo)

**Interfaces:**
- Consumes: nothing from another task (independent).
- Produces: `effective_kvcache_store_path(config: &AivyxConfig) -> PathBuf` in `aivyx.rs` — not consumed by any other task in this plan.

- [ ] **Step 1: Add the raw TOML section**

In `crates/aivyx-config/src/lib.rs`, find `struct RawStorage` (search for `struct RawStorage`):

```rust
struct RawStorage {
    #[serde(default)]
    path: Option<PathBuf>,
}
```

Add a new raw struct right after it:

```rust

#[derive(Debug, Default, Deserialize)]
struct RawKvcache {
    #[serde(default)]
    store_path: Option<PathBuf>,
}
```

Find `struct RawToml` (search for `struct RawToml {`). It has a field:

```rust
    #[serde(default)]
    storage: RawStorage,
```

Add a new field right after it:

```rust
    #[serde(default)]
    storage: RawStorage,
    #[serde(default)]
    kvcache: RawKvcache,
```

- [ ] **Step 2: Add the `kvcache_store_path` field to `AivyxConfig`**

Find `pub struct AivyxConfig` (search for `pub struct AivyxConfig {`). Find the `openai_base_url` field:

```rust
    pub openai_base_url: Option<Sourced<String>>,
```

Add a new field right after the block it's in (anywhere in the struct is fine syntactically; place it near `openai_base_url` for readability):

```rust
    /// Overrides where the kvcache store directory lives. `None`
    /// (default) preserves the historical per-app `ProjectDirs`-derived
    /// path (`~/.local/share/aivyx/kvcache`). Set this to the *same*
    /// directory as `aivyx-coder`'s own `[backend] kvcache_store_path`
    /// (and point both configs' backends at the same `llama-server`) to
    /// share prefill work across the two processes — see
    /// `docs/MCP_RECIPES.md`'s `aivyx-coder` recipe for the full pairing
    /// guidance. `[kvcache] store_path` in TOML, `AIVYX_KVCACHE_STORE_PATH`
    /// env override.
    pub kvcache_store_path: Option<Sourced<PathBuf>>,
```

- [ ] **Step 3: Write the failing tests for TOML loading**

Find the test module for this file (search for `fn phase_122_banner_line_absent_for_anthropic_provider` in `aivyx.rs`, which uses the `load_phase_122_config` fixture already defined above it). Add these two tests near it:

```rust
    #[test]
    fn kvcache_store_path_defaults_to_none() {
        let cfg = load_phase_122_config("");
        assert!(cfg.kvcache_store_path.is_none());
    }

    #[test]
    fn kvcache_store_path_loads_from_toml() {
        let cfg = load_phase_122_config(
            "[kvcache]\n\
             store_path = \"/tmp/shared-kvcache\"\n",
        );
        let sourced = cfg.kvcache_store_path.expect("must be set from TOML");
        assert_eq!(sourced.value, PathBuf::from("/tmp/shared-kvcache"));
        assert_eq!(sourced.source, aivyx_config::FieldSource::Toml);
    }
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test kvcache_store_path_defaults_to_none kvcache_store_path_loads_from_toml`
Expected: FAIL to compile — `no field kvcache_store_path on type AivyxConfig`.

- [ ] **Step 5: Add the loading logic**

In `crates/aivyx-config/src/lib.rs`, find `const ENV_OPENAI_BASE_URL: &str = "AIVYX_OPENAI_BASE_URL";` and add a new constant after it:

```rust
const ENV_OPENAI_BASE_URL: &str = "AIVYX_OPENAI_BASE_URL";
const ENV_KVCACHE_STORE_PATH: &str = "AIVYX_KVCACHE_STORE_PATH";
```

Find the `openai_base_url` loading block inside `load_from_env_and_toml`:

```rust
        // --- openai_base_url ----------------------------------------
        let openai_base_url = match env_string(ENV_OPENAI_BASE_URL) {
            Some(v) => Some(Sourced::new(v, FieldSource::Env)),
            None => toml
                .openai
                .base_url
                .clone()
                .map(|v| Sourced::new(v, FieldSource::Toml)),
        };
```

Add a new block right after it:

```rust

        // --- kvcache_store_path ---------------------------------------
        let kvcache_store_path = match env_path(ENV_KVCACHE_STORE_PATH) {
            Some(p) => Some(Sourced::new(p, FieldSource::Env)),
            None => toml
                .kvcache
                .store_path
                .clone()
                .map(|p| Sourced::new(p, FieldSource::Toml)),
        };
```

Find where `AivyxConfig { ... }` is constructed from these locals (search for `storage_path,` near the end of `load_from_env_and_toml`, in a struct-literal field list):

```rust
            storage_path,
```

Add the new field right after it:

```rust
            storage_path,
            kvcache_store_path,
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test kvcache_store_path_defaults_to_none kvcache_store_path_loads_from_toml`
Expected: `test result: ok. 2 passed`.

- [ ] **Step 7: Add the resolver function and wire it into the real call sites**

In `crates/aivyx-cli/src/bin/aivyx.rs`, find the kvcache-construction block (search for `directories::ProjectDirs::from("", "", "aivyx")` inside the "kvcache (Task 5)" section):

```rust
                let store_path = directories::ProjectDirs::from("", "", "aivyx")
                    .map(|dirs| dirs.data_local_dir().join("kvcache"))
                    .unwrap_or_else(|| std::env::temp_dir().join("aivyx").join("kvcache"));
```

Replace with:

```rust
                let store_path = effective_kvcache_store_path(&config);
```

Add the new function above the function that contains this block (a top-level `fn`, not nested — place it near other small free functions in this file, e.g. right before the function that contains the block above):

```rust
/// The kvcache store directory this run actually uses: the configured
/// override, or the historical `ProjectDirs`-derived default when
/// unset. Single source of truth reused by both the real kvcache
/// construction above and the Ward `SensitivePolicy` extra-deny list
/// below -- the two can never silently diverge.
fn effective_kvcache_store_path(config: &aivyx_config::AivyxConfig) -> std::path::PathBuf {
    match &config.kvcache_store_path {
        Some(sourced) => sourced.value.clone(),
        None => match directories::ProjectDirs::from("", "", "aivyx") {
            Some(dirs) => dirs.data_local_dir().join("kvcache"),
            None => std::env::temp_dir().join("aivyx").join("kvcache"),
        },
    }
}
```

Find the real `SensitivePolicy::new` call site (search for `SensitivePolicy::new(\n                allow_sensitive_paths.clone()`):

```rust
    let sensitive_policy = std::sync::Arc::new(
        if guard_sensitive_paths.value {
            aivyx_core::sensitive_paths::SensitivePolicy::new(
                allow_sensitive_paths.clone(),
                Vec::new(),
            )
        } else {
            aivyx_core::sensitive_paths::SensitivePolicy::disabled()
        },
    );
```

Replace the `Vec::new()` argument:

```rust
    let sensitive_policy = std::sync::Arc::new(
        if guard_sensitive_paths.value {
            aivyx_core::sensitive_paths::SensitivePolicy::new(
                allow_sensitive_paths.clone(),
                vec![effective_kvcache_store_path(&config)],
            )
        } else {
            aivyx_core::sensitive_paths::SensitivePolicy::disabled()
        },
    );
```

- [ ] **Step 8: Write and run tests proving the resolver's default/override behavior and the Ward wiring**

Add these tests near the two from Step 3:

```rust
    #[test]
    fn effective_kvcache_store_path_matches_historical_default_when_unset() {
        let cfg = load_phase_122_config("");
        let path = effective_kvcache_store_path(&cfg);
        assert!(
            path.to_string_lossy().contains(".local/share/aivyx/kvcache"),
            "default kvcache path must be unchanged when no override is configured, got {path:?}"
        );
    }

    #[test]
    fn effective_kvcache_store_path_uses_the_configured_override() {
        let cfg = load_phase_122_config(
            "[kvcache]\n\
             store_path = \"/tmp/shared-kvcache\"\n",
        );
        let path = effective_kvcache_store_path(&cfg);
        assert_eq!(path, PathBuf::from("/tmp/shared-kvcache"));
    }

    #[test]
    fn ward_protects_the_effective_kvcache_path_default_and_overridden() {
        let default_cfg = load_phase_122_config("");
        let default_path = effective_kvcache_store_path(&default_cfg);
        let default_guard = aivyx_core::sensitive_paths::SensitivePolicy::new(
            Vec::new(),
            vec![default_path.clone()],
        );
        assert!(
            default_guard
                .classify(&default_path.join("slots").join("some-handle.bin"))
                .is_some(),
            "the default kvcache directory must be protected"
        );

        let overridden_cfg = load_phase_122_config(
            "[kvcache]\n\
             store_path = \"/tmp/shared-kvcache\"\n",
        );
        let overridden_path = effective_kvcache_store_path(&overridden_cfg);
        let overridden_guard = aivyx_core::sensitive_paths::SensitivePolicy::new(
            Vec::new(),
            vec![overridden_path.clone()],
        );
        assert!(
            overridden_guard
                .classify(&overridden_path.join("slots").join("some-handle.bin"))
                .is_some(),
            "the overridden kvcache directory must be protected"
        );
        assert!(
            overridden_guard.classify(&default_path).is_none(),
            "once overridden, the stale default path must no longer be the one protected"
        );
    }
```

Run: `cargo test kvcache_store_path effective_kvcache_store_path ward_protects_the_effective_kvcache_path`
Expected: all pass.

- [ ] **Step 9: Run the full default-members test suite and clippy**

Run: `cargo test`
Expected: 120+ test-result blocks, 0 failures (matches or exceeds this repo's known-good baseline).

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean, zero warnings.

- [ ] **Step 10: Commit**

```bash
git add crates/aivyx-config/src/lib.rs crates/aivyx-cli/src/bin/aivyx.rs
git commit -s -m "feat: make the kvcache store path configurable, extend Ward to cover it

Adds AivyxConfig::kvcache_store_path (Option<Sourced<PathBuf>>, [kvcache]
store_path in TOML, AIVYX_KVCACHE_STORE_PATH env override), and
effective_kvcache_store_path() as the single source of truth for the
effective path, reused by both the real kvcache construction and the
Ward SensitivePolicy's extra-deny list.

Also closes a real gap: aivyx's own kvcache directory had no Ward
protection at all (Ward classifies by directory name or extension,
neither of which matches kvcache .slot/manifest files) -- unlike
aivyx-coder, which already protects its own equivalent directory."
```

---

### Task 3: Documentation — the sharing recipe

**Files:**
- Modify: `docs/MCP_RECIPES.md` (in the `aivyx` repo)

**Interfaces:**
- Consumes: nothing (documentation only; describes the behavior Tasks 1 and 2 implement, but has no code dependency on either).
- Produces: nothing consumed by another task.

- [ ] **Step 1: Add the sharing subsection to the `aivyx-coder` recipe**

Find, in `docs/MCP_RECIPES.md`, the end of the `aivyx-coder` recipe entry:

```markdown
Verify it works: after configuring `aivyx-coder`'s
`max_access_level` and restarting the daemon (`aivyx daemon stop &&
aivyx`), run `aivyx mcp status` — `aivyx-coder` should show
connected with 2 tools (`code`, `code_reply`). See
`docs/NONAGON.md` §9 for a worked example wiring this into a
Nonagon specialist.

---
```

Insert a new subsection between the "Verify it works" paragraph and the closing `---`:

```markdown
Verify it works: after configuring `aivyx-coder`'s
`max_access_level` and restarting the daemon (`aivyx daemon stop &&
aivyx`), run `aivyx mcp status` — `aivyx-coder` should show
connected with 2 tools (`code`, `code_reply`). See
`docs/NONAGON.md` §9 for a worked example wiring this into a
Nonagon specialist.

**Sharing KV-cache prefill work with `aivyx-coder`:** if both `aivyx`
and this delegated `aivyx-coder` process point at the *same*
`llama-server` instance, they can also share the expensive prefill work
of a long, stable prompt prefix — `aivyx-kvcache`'s manifest is already
safe for two separate OS processes writing the same store directory
concurrently (WAL-mode sqlite index; eviction tolerates a file another
process already deleted). To opt in, set both configs' kvcache store
path to the *identical* absolute directory:

```toml
# aivyx's own config.toml
kvcache_store_path = "/home/me/.local/share/shared-kvcache"
```

```toml
# aivyx-coder's own config.toml
[backend]
kvcache_store_path = "/home/me/.local/share/shared-kvcache"
```

This only helps when both sides are *already* configured against the
same `llama-server` — pointing two processes at the same directory
while they talk to two different backend servers just means two
independent, non-interfering sets of cache entries coexisting in one
folder: harmless, but pointless.

---
```

- [ ] **Step 2: Confirm the doc renders as valid markdown**

Run: `sed -n '/## aivyx-coder/,/^---$/p' docs/MCP_RECIPES.md | head -60`
Expected: the new subsection appears once, in the right place, with both TOML code fences closed (no stray ` ``` `).

- [ ] **Step 3: Commit**

```bash
git add docs/MCP_RECIPES.md
git commit -s -m "docs: document kvcache store-path sharing between aivyx and aivyx-coder

Adds a subsection to the aivyx-coder MCP recipe explaining how to opt
both processes into sharing prefill work: set both configs'
kvcache_store_path to the same directory while both point at the same
llama-server. No new code -- both sides already support the override
(this phase) and aivyx-kvcache's manifest was already engineered for
cross-process safety."
```

---

## Self-Review

**1. Spec coverage:** Spec sections 1-3 (aivyx-coder's field, resolver, deny_paths fix) → Task 1. Sections 4-6 (aivyx's field, resolver, Ward fix) → Task 2. Section 7 (documentation) → Task 3. The spec's full testing list is covered: both repos' default/override resolver tests, both repos' security-fix-tracks-override tests. No gaps.

**2. Placeholder scan:** No TBD/TODO. Every step has complete, concrete code or an exact command with an expected result.

**3. Type consistency:** `BackendSettings::resolved_kvcache_store_path() -> PathBuf` (Task 1) and `effective_kvcache_store_path(config: &AivyxConfig) -> PathBuf` (Task 2) are each used consistently within their own task's steps (construction call site, security-guard call site, and tests) — no signature drift between where they're defined and where they're called. `Settings::effective_deny_paths() -> Vec<PathBuf>` likewise matches its one call site in Task 1 Step 7.

# Configurable KV-Cache Store Path Sharing (Aivyx ↔ Aivyx Coder)

**Status: approved, ready for implementation planning.**

## Context

`aivyx-kvcache`'s `LlamaServerSlotStore` persists prefill work to
`store_path/slots` on disk, via llama-server's native `/slots` save/
restore API, with an sqlite-WAL-backed manifest index. Its own code
comment (`aivyx-kvcache/src/manifest.rs`) states this WAL mode exists
*specifically* because `aivyx-coder` and `aivyx` are separate OS
processes and "cross-process safety is a real requirement here" — the
crate was deliberately engineered to be safe for two processes writing
the same store directory concurrently, confirmed by direct reading of
`manifest.rs` (WAL-mode sqlite for the index) and `llama_server.rs`
(`evict_to_budget` tolerates `NotFound` on a file another process
already deleted).

Despite that, genuine sharing is impossible today: both consumers derive
their store path from a hardcoded, app-name-scoped
`directories::ProjectDirs` call —

- `aivyx`: `ProjectDirs::from("", "", "aivyx")` → `~/.local/share/aivyx/kvcache`
  (`crates/aivyx-cli/src/bin/aivyx.rs`, the "kvcache (Task 5)" block)
- `aivyx-coder`: `ProjectDirs::from("", "", "aivyx-coder")` →
  `~/.local/share/aivyx-coder/kvcache`
  (`crates/aivyx/src/agent_builder.rs`)

— and neither exposes a config override. This work adds one to each,
so an operator running `aivyx` and a delegated `aivyx-coder --mcp-server`
subprocess against the *same* `llama-server` can also point both at the
*same* store directory and get real prefill sharing, with zero new
concurrency-safety code (the crate already handles it).

**A related, independently-real gap found during this scoping and folded
into the same work (user's explicit choice):** `aivyx-coder` already
protects its own kvcache directory from its own agent's `fs.read`/
`fs.write` tools (`PermissionSettings`'s `deny_paths`) — a restored
`.slot` file is context re-entering a session invisibly, the same class
of risk as `.ssh`/`.aws`. `aivyx`'s equivalent mechanism, Ward
(`aivyx-core/src/sensitive_paths.rs`), has no such protection: it
classifies by directory name (`.ssh`, `.aws`) or file extension
(`SENSITIVE_EXTENSIONS = ["pem", "key", "p12", "pfx", "redb"]`), and
kvcache `.slot`/manifest files match neither. `aivyx`'s own agent could
today `fs.read`/`fs.write` its own kvcache store with no guard at all.
This work closes that gap as part of making the path configurable (the
same code path that resolves "what path do we protect" already needs to
exist for the override to work correctly).

## Approach

### 1. `aivyx-coder`: the override field

Add `kvcache_store_path: Option<String>` to `BackendSettings`
(`crates/aivyx-config/src/lib.rs`, alongside the existing `kind`/
`kvcache_max_bytes` fields). `None` (default) preserves today's
behavior exactly. The struct already carries `#[serde(default)]`, so no
extra serde attributes are needed on the new field.

A new method, `BackendSettings::resolved_kvcache_store_path(&self) ->
PathBuf`, is the single source of truth for "what path does this run's
kvcache actually use": if `kvcache_store_path` is set, resolve it
through the same `resolve_tilde_paths` helper `resolved_deny_paths`/
`resolved_extra_read_paths` already use (so `~` and symlinks behave
identically to every other path-like config value in this crate); if
unset, fall back to today's exact `ProjectDirs`-derived default. Both
existing call sites — the real kvcache-construction block in
`agent_builder.rs` and the security fix below — call this one method,
so they can never silently diverge.

### 2. `aivyx-coder`: the security fix (deny_paths)

Today, `PermissionSettings::default()`'s `deny_paths` literal hardcodes
`"~/.local/share/aivyx-coder/kvcache"` — a static string, blind to any
override. Remove that literal. Add `Settings::effective_deny_paths(&self)
-> Vec<PathBuf>`, composing `self.permissions.resolved_deny_paths()`
with `self.backend.resolved_kvcache_store_path()` appended — always
protecting whatever path is *actually* in effect this run, default or
overridden, regardless of whether `kind == LlamaServer` this particular
run (a previous run may have left slot files behind under the old
default even if this run's backend kind changed). `agent_builder.rs`'s
existing call site (`settings.permissions.resolved_deny_paths()`)
becomes `settings.effective_deny_paths()` — the only call site, so
nothing else needs to change.

### 3. `aivyx-coder`: wiring the kvcache-construction block

`agent_builder.rs`'s existing inline `ProjectDirs::from("", "", "aivyx-coder")`
construction is replaced with a call to
`settings.backend.resolved_kvcache_store_path()` — same default value,
now override-aware.

### 4. `aivyx`: the override field

Add `kvcache_store_path: Option<Sourced<PathBuf>>` to `AivyxConfig`
(`crates/aivyx-config/src/lib.rs`), a top-level field (there is no
existing per-provider config section to nest it under — `aivyx` supports
multiple providers, and the LlamaCpp/Ollama/Jan family already shares
`openai_base_url` as "config-level sugar" rather than each having its
own section). Loaded via a new `ENV_KVCACHE_STORE_PATH =
"AIVYX_KVCACHE_STORE_PATH"` env var, checked before a new top-level TOML
key `kvcache_store_path`, mirroring `openai_base_url`'s existing
env-then-toml resolution pattern exactly (`Sourced::new(v,
FieldSource::Env)` / `FieldSource::Toml`).

A new function, `effective_kvcache_store_path(config: &AivyxConfig) ->
PathBuf`, is `aivyx`'s equivalent single source of truth: returns the
configured override's `.value` if set, else today's exact
`ProjectDirs::from("", "", "aivyx")`-derived default. Both call sites
below use it.

### 5. `aivyx`: wiring the kvcache-construction block

The existing inline `ProjectDirs` construction in `aivyx.rs`'s "kvcache
(Task 5)" block is replaced with a call to
`effective_kvcache_store_path(&config)`.

### 6. `aivyx`: the security fix (Ward)

At `aivyx.rs`'s real `SensitivePolicy::new(allow_sensitive_paths.clone(),
Vec::new())` call site, the second argument (`extra_deny`, currently
always empty) becomes `vec![effective_kvcache_store_path(&config)]` —
the same function from item 4/5, so the protected path can never
silently diverge from the path kvcache actually uses. This applies
**only when `guard_sensitive_paths` is enabled** (the existing `if
guard_sensitive_paths.value { ... } else { SensitivePolicy::disabled()
}` branch is unchanged) — consistent with every other Ward protection,
which is already conditional on that same operator setting.

### 7. Documentation

`docs/MCP_RECIPES.md`'s `aivyx-coder` recipe entry gains a new
subsection, after the existing "Verify it works" paragraph, explaining
the pairing concretely: set `aivyx`'s `kvcache_store_path` and
`aivyx-coder`'s `[backend] kvcache_store_path` to the identical absolute
directory, point both configs' backend at the same running
`llama-server` instance, and prefill work for a shared stable prompt
prefix is shared automatically — no other wiring needed, since
`aivyx-kvcache`'s manifest is already safe for exactly this two-process
scenario. Note explicitly that this only matters when both sides are
*already* configured against the same `llama-server` (sharing a
directory with two processes pointed at two different backend servers
would just mean two independent, non-interfering sets of cache entries
coexisting in one folder — harmless, but pointless).

## Testing

**`aivyx-coder`:**
- `resolved_kvcache_store_path` returns the exact `ProjectDirs` default
  when `kvcache_store_path` is `None` (regression guard, must match
  today's exact path string).
- `resolved_kvcache_store_path` returns the configured path
  (tilde-expanded) when set.
- `Settings::effective_deny_paths()` includes the default kvcache path
  when `kvcache_store_path` is unset (replaces the existing
  `default_deny_paths_includes_the_kvcache_directory` test, updated to
  call `effective_deny_paths()` instead of
  `permissions.resolved_deny_paths()`).
- `Settings::effective_deny_paths()` includes the *overridden* path,
  not the old default, when `kvcache_store_path` is set — proving the
  security fix actually tracks the override rather than the stale
  literal.

**`aivyx`:**
- `effective_kvcache_store_path` returns the exact `ProjectDirs` default
  when unset (regression guard).
- `effective_kvcache_store_path` returns the configured override when
  set, loaded from TOML.
- `effective_kvcache_store_path` prefers the env var over TOML when both
  are set (mirroring `openai_base_url`'s existing env-precedence test
  pattern).
- A `SensitivePolicy` built with `extra_deny =
  vec![effective_kvcache_store_path(&config)]` classifies a path inside
  the default kvcache directory as sensitive (a new test alongside
  `sensitive_paths.rs`'s existing classification tests).
- The same, with an overridden path — proving the guard tracks the
  override, not a stale default.

Both repos' full existing suites re-run to confirm no regression
(`cargo test` in `aivyx-coder`, plain `cargo test` — not `--workspace`,
per `aivyx`'s own default-members convention — in `aivyx`).

## Self-review

- **Placeholder scan:** none — every new function, field, and call site
  is named and located concretely against the real, currently-read
  source.
- **Internal consistency:** both repos' designs are structurally
  parallel (one override field, one "effective path" resolver function
  reused by both the kvcache-construction call site and the
  security-guard call site) even though the concrete types differ
  (`Option<String>` + tilde-expansion in `aivyx-coder`, matching that
  repo's existing convention; `Option<Sourced<PathBuf>>` + env/TOML
  precedence in `aivyx`, matching *that* repo's existing convention) —
  each follows its own repo's idiom rather than inventing a third,
  shared one.
- **Scope check:** two repos, but each side is small and mechanical
  (one new field, one new resolver function, two call-site edits, a few
  tests) — sized for two implementation tasks (one per repo) plus a
  docs task, not a decomposition into separate specs.
- **Ambiguity check:** the "fold in the Ward gap" scope decision was
  confirmed directly with the user rather than assumed. The TOML
  placement for `aivyx`'s new field (top-level, not nested under a
  provider section) is stated as a deliberate choice with its reasoning
  given, not left for the plan to improvise.

# Settings Coverage Expansion (POLISH_WAVES.md sub-project 7, plan 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Studio create/edit for `[memory] profile`, `[embedding]`, `[proactive]`, and `[[reflection_schedule]]` — the last unbuilt piece of sub-project 7 ("credentials in Studio").

**Architecture:** Reuses the config-write architecture plans 1-2 already shipped, unchanged: `aivyx-config/src/config_write.rs` gets 4 new section-writers (one array-of-table primitive for `[[reflection_schedule]]`, three singleton partial-update primitives for `[memory]`/`[embedding]`/`[proactive]`), `aivyx-ipc/src/protocol.rs` gets matching wire types, `aivyx-channel/src/daemon_server.rs` gets matching handlers, and `aivyx-web/src/main.rs` gets the Studio UI. Unlike plans 1-2, **no new UI-state struct types are needed** — `SettingsState` and `Dashboard` (both already threaded everywhere) just gain new fields, and the existing `SettingsState.notice`/`SchedulesUi.notice` banners are reused via id-prefix routing.

**Tech Stack:** Rust, `toml_edit` 0.22, Dioxus 0.6 (wasm32), the existing `QueryPayload`/`QueryResponsePayload` IPC protocol.

## Global Constraints

- Read functions for anything on this list use raw `toml_edit` parsing only (`load_document`), never `AivyxConfig::load_from_env_and_toml`/`load_settings_config`. This matters doubly for `[[reflection_schedule]]`: the resolving loader **silently drops every entry with `enabled = false`** from its `reflection_schedules: Vec` (`crates/aivyx-config/src/lib.rs:7256`, `if !raw.enabled { continue; }`) — the Studio list view must show disabled entries too, or an operator can never re-enable one through Studio.
- `[embedding] api_key` uses `RedactedSecret` on the read side; a `SetEmbeddingConfig` write's `api_key: Option<String>` field means `None` = leave the existing TOML value untouched — the established convention from plans 1-2.
- `[memory] profile`/`[embedding]`/`[proactive]` writes use the **leave-untouched-on-`None`** convention for every field (not `write_profile_section`'s older clear-on-`None` convention) — plan 2's hardened precedent, used uniformly here too even for `[memory] profile`'s single field.
- `[[reflection_schedule]]` writes use the array-of-table upsert-by-name + `KNOWN_KEYS` unknown-key-preservation pattern from plans 1-2, defending `role_override`/`skip_when_idle`/`min_audit_entries_to_fire` from being silently dropped on an edit — the exact failure class plan 1's `[mcp_server.sandbox]` incident was.
- `write_reflection_schedule_section`'s name-uniqueness check reads **both** `doc["reflection_schedule"]` and `doc["schedule"]` — the loader rejects a reflection-schedule name that collides with a regular `[[schedule]]` entry too (shared namespace).
- `write_proactive_section`'s `target` check is **deliberately stricter than today's loader** (`build_proactive_config` only checks non-empty, never that the name exists in `[[notify_target]]`) — this is new defense-in-depth, not a mirror of an existing loader check. Say so in code comments; don't imply a loader check exists that doesn't.
- Every new `ConfigWriteError` variant needs a matching arm in `map_config_write_error` (`crates/aivyx-channel/src/daemon_server.rs:6364`) — its match is exhaustive, a hard compile error otherwise.
- `ConfigWriteError` variants are struct-like (`{ reason: String }`), matching every existing variant — not a tuple variant.
- Validate against the *merged* post-write state for `[proactive]` (existing on-disk values combined with this call's `Some` fields), not the just-written fields in isolation — the posture plan 2's `write_email_section` converged on.
- None of the new wire-type struct fields are literally named `kind` — the `#[serde(tag = "kind")]` internal-tag collision plan 2's Task 3 hit. Confirmed not applicable here, but double-check before naming any new field.
- Full sweep before merge: `cargo clippy --workspace --exclude aivyx-desktop --all-targets -- -D warnings` and `cargo test --workspace --exclude aivyx-desktop` (this repo's `aivyx-desktop` needs system webkit2gtk libs not present in this environment — use `cargo test` with no `-p`/`--workspace` flag day-to-day, which only touches `default-members` and excludes it automatically), zero warnings/failures, plus a `dist/` rebuild in the final task.
- `dist/` rebuild recipe (from `crates/aivyx-web`): `dx bundle --release --platform web`, then `rm -rf dist && mkdir -p dist && cp -r target/dx/aivyx-web/release/web/public/. dist/`, then `find dist -name '*.br' -delete`, then verify via `git status --porcelain dist/assets/ | grep wasm` shows exactly one add + one delete (a clean rename). Requires the wasm32 toolchain on `PATH`: `~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin` + `~/.cargo/bin`.
- `aivyx-web`'s `#[cfg(test)]` blocks (none added by this plan, but if any task needs to run existing ones) run via plain `cargo test -p aivyx-web` — never `--target wasm32-unknown-unknown`.

---

## Task 1: `[[reflection_schedule]]` config-write primitive

**Files:**
- Modify: `crates/aivyx-config/src/config_write.rs`

**Interfaces:**
- Consumes: `load_document`, `write_toml_0600`, `ConfigWriteError` (existing, at the top of this file).
- Produces: `pub struct ReflectionScheduleEntryWrite { pub name: String, pub cron: String, pub lookback_window_secs: u64, pub enabled: bool }`, `pub fn write_reflection_schedule_section(path: &Path, entry: &ReflectionScheduleEntryWrite) -> Result<(), ConfigWriteError>`, `pub fn remove_reflection_schedule_section(path: &Path, name: &str) -> Result<(), ConfigWriteError>`, `pub fn read_reflection_schedule_entries(path: &Path) -> Result<Vec<ReflectionScheduleEntryWrite>, ConfigWriteError>` — Task 3 calls all three.

- [ ] **Step 1: Add the `InvalidReflectionSchedule` error variant**

In `crates/aivyx-config/src/config_write.rs`, add a new variant to `pub enum ConfigWriteError` (right after `InvalidEmailConfig { reason: String },`):

```rust
    /// A `[[reflection_schedule]]` entry is structurally invalid — mirrors
    /// the loader's own validation (`aivyx-config/src/lib.rs:7251`-7301:
    /// non-empty name/cron, lookback bounds, name uniqueness against both
    /// `[[reflection_schedule]]` and `[[schedule]]`) so a bad write is
    /// refused before it corrupts the next daemon load, same principle as
    /// `InvalidNotifyTarget`.
    InvalidReflectionSchedule { reason: String },
```

And add the matching `Display` arm in `impl std::fmt::Display for ConfigWriteError` (right after the `InvalidEmailConfig` arm):

```rust
            ConfigWriteError::InvalidReflectionSchedule { reason } => write!(f, "invalid reflection schedule entry: {reason}"),
```

- [ ] **Step 2: Add `ReflectionScheduleEntryWrite` and the array-of-table primitive**

Add this at the end of the file, right before the `#[cfg(test)] mod tests {` block:

```rust
/// The `[[reflection_schedule]]` array, as Studio's write form submits
/// it. `role_override`/`skip_when_idle`/`min_audit_entries_to_fire` stay
/// TOML-only — an upsert must preserve them via the `KNOWN_KEYS`
/// mechanism below, never silently drop them.
#[derive(Debug, Clone, PartialEq)]
pub struct ReflectionScheduleEntryWrite {
    pub name: String,
    pub cron: String,
    pub lookback_window_secs: u64,
    pub enabled: bool,
}

/// Add or replace (by `name`) one `[[reflection_schedule]]` entry,
/// preserving every other entry, section, and the operator's comments.
/// Validates the same rules the loader does
/// (`aivyx-config/src/lib.rs:7251`-7301): non-empty `name`/`cron`,
/// `lookback_window_secs` within `[MIN_REFLECTION_LOOKBACK_SECS,
/// MAX_REFLECTION_LOOKBACK_SECS]` (60s – 30 days), and name uniqueness
/// against `[[schedule]]` too — the loader rejects a reflection-schedule
/// name that collides with a regular schedule name (shared namespace).
/// Uniqueness *within* `[[reflection_schedule]]` itself needs no explicit
/// check: this function always upserts by name, so a matching existing
/// name is a replace, never a collision.
pub fn write_reflection_schedule_section(
    path: &Path,
    entry: &ReflectionScheduleEntryWrite,
) -> Result<(), ConfigWriteError> {
    if entry.name.trim().is_empty() {
        return Err(ConfigWriteError::InvalidReflectionSchedule {
            reason: "name must not be empty".to_string(),
        });
    }
    if entry.cron.trim().is_empty() {
        return Err(ConfigWriteError::InvalidReflectionSchedule {
            reason: format!("entry {:?}: cron must not be empty", entry.name),
        });
    }
    if entry.lookback_window_secs < crate::MIN_REFLECTION_LOOKBACK_SECS
        || entry.lookback_window_secs > crate::MAX_REFLECTION_LOOKBACK_SECS
    {
        return Err(ConfigWriteError::InvalidReflectionSchedule {
            reason: format!(
                "entry {:?}: lookback_window_secs = {} is outside the allowed range [{}, {}] (60s to 30 days)",
                entry.name,
                entry.lookback_window_secs,
                crate::MIN_REFLECTION_LOOKBACK_SECS,
                crate::MAX_REFLECTION_LOOKBACK_SECS,
            ),
        });
    }

    let mut doc = load_document(path)?;

    // Cross-array uniqueness against [[schedule]] — the loader's own
    // check, mirrored here (aivyx-config/src/lib.rs:7292-7300).
    if let Some(sched_arr) = doc.get("schedule").and_then(toml_edit::Item::as_array_of_tables) {
        if sched_arr
            .iter()
            .any(|t| t.get("name").and_then(|v| v.as_str()) == Some(entry.name.as_str()))
        {
            return Err(ConfigWriteError::InvalidReflectionSchedule {
                reason: format!(
                    "entry {:?}: collides with a [[schedule]] entry of the same name — names share a namespace",
                    entry.name
                ),
            });
        }
    }

    let arr = reflection_schedule_array_mut(&mut doc);

    let mut table = toml_edit::Table::new();
    table["name"] = value(entry.name.as_str());
    table["cron"] = value(entry.cron.as_str());
    table["lookback_window_secs"] = value(entry.lookback_window_secs as i64);
    table["enabled"] = value(entry.enabled);

    // Preserve any key this write schema doesn't know about —
    // role_override/skip_when_idle/min_audit_entries_to_fire — mirroring
    // write_mcp_server_section's/write_notify_target_section's own
    // defensive convention (plan 1's [mcp_server.sandbox] incident is
    // exactly the failure class this guards against).
    const KNOWN_KEYS: &[&str] = &["name", "cron", "lookback_window_secs", "enabled"];
    let idx = arr
        .iter()
        .position(|t| t.get("name").and_then(|v| v.as_str()) == Some(entry.name.as_str()));
    match idx {
        Some(i) => {
            if let Some(existing) = arr.get(i) {
                for (k, v) in existing.iter() {
                    if KNOWN_KEYS.contains(&k) {
                        continue;
                    }
                    table.insert(k, v.clone());
                }
            }
            *arr.get_mut(i).expect("index just found") = table;
        }
        None => arr.push(table),
    }

    write_toml_0600(path, &doc.to_string())
}

/// Remove one `[[reflection_schedule]]` entry by `name`. A no-op (not an
/// error) when no entry with that name exists.
pub fn remove_reflection_schedule_section(path: &Path, name: &str) -> Result<(), ConfigWriteError> {
    let mut doc = load_document(path)?;
    let arr = reflection_schedule_array_mut(&mut doc);
    let idx = arr.iter().position(|t| t.get("name").and_then(|v| v.as_str()) == Some(name));
    if let Some(i) = idx {
        arr.remove(i);
    }
    write_toml_0600(path, &doc.to_string())
}

/// Read every `[[reflection_schedule]]` entry as literally written on
/// disk — no resolution, and critically NOT the resolving loader's
/// `reflection_schedules: Vec<ReflectionScheduleConfig>`, which silently
/// drops any entry with `enabled = false` entirely (see this plan's
/// design spec, "Corrections" §3). The Studio list view must show
/// disabled entries too, so an operator can re-enable one.
pub fn read_reflection_schedule_entries(path: &Path) -> Result<Vec<ReflectionScheduleEntryWrite>, ConfigWriteError> {
    let doc = load_document(path)?;
    let Some(arr) = doc.get("reflection_schedule").and_then(toml_edit::Item::as_array_of_tables) else {
        return Ok(Vec::new());
    };
    Ok(arr.iter().map(raw_table_to_reflection_schedule).collect())
}

fn raw_table_to_reflection_schedule(table: &toml_edit::Table) -> ReflectionScheduleEntryWrite {
    let str_field = |key: &str| table.get(key).and_then(|v| v.as_str()).map(str::to_string);
    ReflectionScheduleEntryWrite {
        name: str_field("name").unwrap_or_default(),
        cron: str_field("cron").unwrap_or_default(),
        lookback_window_secs: table
            .get("lookback_window_secs")
            .and_then(|v| v.as_integer())
            .unwrap_or(86_400) as u64,
        enabled: table.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
    }
}

/// The `[[reflection_schedule]]` array, creating an empty one if the
/// section is absent from the document yet.
fn reflection_schedule_array_mut(doc: &mut DocumentMut) -> &mut toml_edit::ArrayOfTables {
    if doc
        .get("reflection_schedule")
        .and_then(toml_edit::Item::as_array_of_tables)
        .is_none()
    {
        doc["reflection_schedule"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    doc["reflection_schedule"]
        .as_array_of_tables_mut()
        .expect("just ensured present")
}
```

`MIN_REFLECTION_LOOKBACK_SECS`/`MAX_REFLECTION_LOOKBACK_SECS` are private `const`s declared at the crate root in `crates/aivyx-config/src/lib.rs:4200`-4201 (`60` and `30 * 86400`) — private items in a crate root module are visible to descendant modules, so `crate::MIN_REFLECTION_LOOKBACK_SECS` resolves without adding `pub`.

- [ ] **Step 3: Write the tests**

Add to the `#[cfg(test)] mod tests` block in the same file (near the existing `notify_target_write_*` tests — search for `fn notify_target_write_adds_a_new_entry` to find the right neighborhood):

```rust
    fn refl_entry(name: &str) -> ReflectionScheduleEntryWrite {
        ReflectionScheduleEntryWrite {
            name: name.to_string(),
            cron: "0 0 9 * * * *".to_string(),
            lookback_window_secs: 86_400,
            enabled: true,
        }
    }

    #[test]
    fn reflection_schedule_write_adds_a_new_entry() {
        let path = temp_toml("refl-add");
        std::fs::write(&path, "[access]\nlevel = \"sandbox\"\n").unwrap();
        write_reflection_schedule_section(&path, &refl_entry("nightly")).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("[access]"), "unrelated section survives");
        assert!(contents.contains("[[reflection_schedule]]"));
        assert!(contents.contains("name = \"nightly\""));
        assert!(contents.contains("cron = \"0 0 9 * * * *\""));
    }

    #[test]
    fn reflection_schedule_write_replaces_an_existing_entry_by_name() {
        let path = temp_toml("refl-replace");
        std::fs::write(&path, "").unwrap();
        write_reflection_schedule_section(&path, &refl_entry("nightly")).unwrap();
        let mut updated = refl_entry("nightly");
        updated.lookback_window_secs = 3600;
        write_reflection_schedule_section(&path, &updated).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.matches("name = \"nightly\"").count(), 1);
        assert!(contents.contains("lookback_window_secs = 3600"));
    }

    #[test]
    fn reflection_schedule_write_preserves_unknown_keys_on_replace() {
        let path = temp_toml("refl-preserve");
        std::fs::write(
            &path,
            "[[reflection_schedule]]\nname = \"nightly\"\ncron = \"0 0 9 * * * *\"\n\
             lookback_window_secs = 86400\nenabled = true\nrole_override = \"night-owl\"\n\
             skip_when_idle = true\nmin_audit_entries_to_fire = 3\n",
        )
        .unwrap();
        let mut updated = refl_entry("nightly");
        updated.lookback_window_secs = 7200;
        write_reflection_schedule_section(&path, &updated).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("role_override = \"night-owl\""), "role_override survives");
        assert!(contents.contains("skip_when_idle = true"), "skip_when_idle survives");
        assert!(contents.contains("min_audit_entries_to_fire = 3"), "min_audit_entries_to_fire survives");
        assert!(contents.contains("lookback_window_secs = 7200"), "the actual edit applied");
    }

    #[test]
    fn reflection_schedule_write_rejects_empty_name() {
        let path = temp_toml("refl-empty-name");
        std::fs::write(&path, "").unwrap();
        let mut entry = refl_entry("nightly");
        entry.name = "  ".to_string();
        let err = write_reflection_schedule_section(&path, &entry).unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidReflectionSchedule { .. }));
    }

    #[test]
    fn reflection_schedule_write_rejects_empty_cron() {
        let path = temp_toml("refl-empty-cron");
        std::fs::write(&path, "").unwrap();
        let mut entry = refl_entry("nightly");
        entry.cron = String::new();
        let err = write_reflection_schedule_section(&path, &entry).unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidReflectionSchedule { .. }));
    }

    #[test]
    fn reflection_schedule_write_rejects_lookback_out_of_range() {
        let path = temp_toml("refl-bad-lookback");
        std::fs::write(&path, "").unwrap();
        let mut entry = refl_entry("nightly");
        entry.lookback_window_secs = 30; // below the 60s floor
        let err = write_reflection_schedule_section(&path, &entry).unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidReflectionSchedule { .. }));
    }

    #[test]
    fn reflection_schedule_write_rejects_name_colliding_with_a_regular_schedule() {
        let path = temp_toml("refl-collide-schedule");
        std::fs::write(
            &path,
            "[[schedule]]\nname = \"nightly\"\ncron = \"0 0 9 * * * *\"\nprompt = \"check things\"\n",
        )
        .unwrap();
        let err = write_reflection_schedule_section(&path, &refl_entry("nightly")).unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidReflectionSchedule { .. }));
    }

    #[test]
    fn reflection_schedule_remove_drops_the_named_entry_only() {
        let path = temp_toml("refl-remove");
        std::fs::write(&path, "").unwrap();
        write_reflection_schedule_section(&path, &refl_entry("nightly")).unwrap();
        write_reflection_schedule_section(&path, &refl_entry("weekly-digest")).unwrap();
        remove_reflection_schedule_section(&path, "nightly").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(!contents.contains("nightly"));
        assert!(contents.contains("weekly-digest"));
    }

    #[test]
    fn read_reflection_schedule_entries_round_trips_and_includes_disabled() {
        let path = temp_toml("refl-round-trip");
        std::fs::write(&path, "").unwrap();
        write_reflection_schedule_section(&path, &refl_entry("nightly")).unwrap();
        let mut disabled = refl_entry("paused-one");
        disabled.enabled = false;
        write_reflection_schedule_section(&path, &disabled).unwrap();
        let entries = read_reflection_schedule_entries(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|e| e.name == "nightly" && e.enabled));
        assert!(
            entries.iter().any(|e| e.name == "paused-one" && !e.enabled),
            "a disabled entry must still be readable — the resolving loader drops these entirely"
        );
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p aivyx-config reflection_schedule -- --nocapture`
Expected: all 9 new tests pass (`ok`).

- [ ] **Step 5: Run clippy on the crate**

Run: `cargo clippy -p aivyx-config --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-config/src/config_write.rs
git commit -m "feat(config): add [[reflection_schedule]] config-write primitive

write_reflection_schedule_section/remove_.../read_... — the array-of-table
upsert-by-name pattern from plans 1-2, applied to reflection schedules.
Validates non-empty name/cron, lookback bounds, and name uniqueness
against BOTH [[reflection_schedule]] and [[schedule]] (shared namespace,
mirroring the loader). Preserves role_override/skip_when_idle/
min_audit_entries_to_fire via the KNOWN_KEYS mechanism. The raw read
surfaces enabled=false entries, which the resolving loader drops entirely.

POLISH_WAVES.md sub-project 7 plan 3, Task 1.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 2: `[memory] profile` / `[embedding]` / `[proactive]` config-write primitives

**Files:**
- Modify: `crates/aivyx-config/src/config_write.rs`

**Interfaces:**
- Consumes: `load_document`, `write_toml_0600`, `value` (from `toml_edit`), `ConfigWriteError` (existing).
- Produces: `pub fn write_memory_profile(path: &Path, profile: Option<&str>) -> Result<(), ConfigWriteError>`, `pub fn read_memory_profile(path: &Path) -> Result<Option<String>, ConfigWriteError>`; `pub struct EmbeddingEntryWrite { pub base_url: Option<String>, pub model: Option<String>, pub api_key: Option<String> }`, `pub fn write_embedding_section(path: &Path, entry: &EmbeddingEntryWrite) -> Result<(), ConfigWriteError>`, `pub fn read_embedding_section(path: &Path) -> Result<EmbeddingEntryWrite, ConfigWriteError>`; `pub struct ProactiveEntryWrite { pub enabled: Option<bool>, pub target: Option<String>, pub max_per_window: Option<u32>, pub window_secs: Option<u64> }`, `pub fn write_proactive_section(path: &Path, entry: &ProactiveEntryWrite) -> Result<(), ConfigWriteError>`, `pub fn read_proactive_section(path: &Path) -> Result<ProactiveEntryWrite, ConfigWriteError>` — Task 4 calls all of these.

- [ ] **Step 1: Add the 3 new error variants**

In `crates/aivyx-config/src/config_write.rs`, add to `pub enum ConfigWriteError` (right after the `InvalidReflectionSchedule` variant Task 1 added):

```rust
    /// `[memory] profile` is not one of the loader's recognized values.
    /// The loader itself (`MemoryProfile::from_arg`) is permissive —
    /// any unrecognized string silently falls back to `Off` rather than
    /// erroring — but the Studio picker only ever offers 3 concrete
    /// values, so a 4th string reaching this function means a
    /// non-Studio caller sent something wrong; refuse it rather than
    /// silently defaulting.
    InvalidMemoryProfile { reason: String },
    /// An `[embedding]` field is blank where a value was explicitly
    /// supplied (an explicit-but-blank `base_url`/`model` would still
    /// write an empty string, which the loader then treats differently
    /// from "absent" — refused here instead).
    InvalidEmbeddingConfig { reason: String },
    /// The `[proactive]` section, after this write is merged onto
    /// whatever's already on disk, would fail the loader's own
    /// `enabled = true` requirements (`build_proactive_config`,
    /// `aivyx-config/src/lib.rs:8689`): non-empty `target`,
    /// `max_per_window >= 1`, `window_secs >= 1`. Also enforces one
    /// check the loader itself does NOT make — that `target` names an
    /// existing `[[notify_target]]` entry — deliberately stricter than
    /// today's loader, not a mirror of an existing check (see this
    /// plan's design spec, "Corrections" §2).
    InvalidProactiveConfig { reason: String },
```

And the matching `Display` arms (right after the `InvalidReflectionSchedule` arm):

```rust
            ConfigWriteError::InvalidMemoryProfile { reason } => write!(f, "invalid memory profile: {reason}"),
            ConfigWriteError::InvalidEmbeddingConfig { reason } => write!(f, "invalid embedding config: {reason}"),
            ConfigWriteError::InvalidProactiveConfig { reason } => write!(f, "invalid proactive config: {reason}"),
```

- [ ] **Step 2: Add `write_memory_profile`/`read_memory_profile`**

Add at the end of the file, right before `#[cfg(test)] mod tests {` (after Task 1's additions):

```rust
/// `[memory]` is a shared table with fields this function does not touch
/// (`max_per_topic`, `ttl_secs`, the `[[memory.retention]]` array-of-
/// tables, `canonicalize_topics`) — only the `profile` key is
/// added/updated; every sibling key and the retention sub-array stay
/// byte-identical. `profile: None` leaves the on-disk value untouched
/// (leave-on-`None`, plan 2's convention).
pub fn write_memory_profile(path: &Path, profile: Option<&str>) -> Result<(), ConfigWriteError> {
    let Some(p) = profile else { return Ok(()) };
    let normalized = match p.trim().to_lowercase().as_str() {
        "off" | "lite" | "smart" => p.trim().to_lowercase(),
        other => {
            return Err(ConfigWriteError::InvalidMemoryProfile {
                reason: format!("{other:?} is not a valid profile — must be \"off\", \"lite\", or \"smart\""),
            })
        }
    };
    let mut doc = load_document(path)?;
    doc["memory"]["profile"] = value(normalized);
    write_toml_0600(path, &doc.to_string())
}

/// Raw read of `[memory] profile` — `None` when the section or key is
/// absent (the Studio picker's caller defaults that to `"off"`).
pub fn read_memory_profile(path: &Path) -> Result<Option<String>, ConfigWriteError> {
    let doc = load_document(path)?;
    Ok(doc
        .get("memory")
        .and_then(toml_edit::Item::as_table_like)
        .and_then(|t| t.get("profile"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string()))
}
```

- [ ] **Step 3: Add `EmbeddingEntryWrite`, `write_embedding_section`, `read_embedding_section`**

Add right after Step 2's additions:

```rust
/// The `[embedding]` section as Studio's write form submits it. Every
/// field `Option`, `None` meaning "leave this key untouched on disk" —
/// same partial-update convention every singleton section in this file
/// uses, applied uniformly here even though only `api_key` is a secret
/// (plan 2's hardened precedent, not `write_profile_section`'s older
/// clear-on-`None` convention). `dimensions`/`rag_top_k`/
/// `rag_min_similarity`/`recall_window_turns`/`recall_gate_min_chars`
/// and the Chapter Loom recall-fusion tuning fields stay TOML-only —
/// not represented here at all.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmbeddingEntryWrite {
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
}

/// Patch the `[embedding]` section, touching only the `Some` fields. An
/// explicit-but-blank `base_url`/`model` is refused (would write an
/// empty string, which the loader's own defaulting treats differently
/// from "key absent") rather than silently accepted.
pub fn write_embedding_section(path: &Path, entry: &EmbeddingEntryWrite) -> Result<(), ConfigWriteError> {
    if let Some(v) = &entry.base_url {
        if v.trim().is_empty() {
            return Err(ConfigWriteError::InvalidEmbeddingConfig {
                reason: "base_url, if set, must not be blank".to_string(),
            });
        }
    }
    if let Some(v) = &entry.model {
        if v.trim().is_empty() {
            return Err(ConfigWriteError::InvalidEmbeddingConfig {
                reason: "model, if set, must not be blank".to_string(),
            });
        }
    }

    let mut doc = load_document(path)?;
    if let Some(v) = &entry.base_url {
        doc["embedding"]["base_url"] = value(v.as_str());
    }
    if let Some(v) = &entry.model {
        doc["embedding"]["model"] = value(v.as_str());
    }
    if let Some(v) = &entry.api_key {
        doc["embedding"]["api_key"] = value(v.as_str());
    }
    write_toml_0600(path, &doc.to_string())
}

/// Read the `[embedding]` section as literally written on disk — no
/// resolution of anything. Absent section (or absent key) reads as
/// `None` for that field.
pub fn read_embedding_section(path: &Path) -> Result<EmbeddingEntryWrite, ConfigWriteError> {
    let doc = load_document(path)?;
    let Some(table) = doc.get("embedding").and_then(toml_edit::Item::as_table_like) else {
        return Ok(EmbeddingEntryWrite::default());
    };
    Ok(EmbeddingEntryWrite {
        base_url: table.get("base_url").and_then(|v| v.as_str()).map(str::to_string),
        model: table.get("model").and_then(|v| v.as_str()).map(str::to_string),
        api_key: table.get("api_key").and_then(|v| v.as_str()).map(str::to_string),
    })
}
```

- [ ] **Step 4: Add `ProactiveEntryWrite`, `write_proactive_section`, `read_proactive_section`**

Add right after Step 3's additions:

```rust
/// The `[proactive]` section as Studio's write form submits it. Same
/// leave-untouched-on-`None` convention as `EmbeddingEntryWrite`.
/// `signals` (the 3 `signal_*` toggles) stays TOML-only — not
/// represented here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProactiveEntryWrite {
    pub enabled: Option<bool>,
    pub target: Option<String>,
    pub max_per_window: Option<u32>,
    pub window_secs: Option<u64>,
}

/// Patch the `[proactive]` section, touching only the `Some` fields.
///
/// Validates the **merged** post-write state (existing on-disk values
/// combined with this call's `Some` fields — a save that only flips
/// `enabled` while `target` is already on disk from an earlier save
/// must keep succeeding) against `build_proactive_config`'s
/// (`aivyx-config/src/lib.rs:8689`) own `enabled = true` requirements:
/// non-empty `target`, `max_per_window >= 1`, `window_secs >= 1`. Also
/// checks that `target` names an existing `[[notify_target]]` entry —
/// the loader itself does NOT make this check (it only checks
/// non-empty), so this is deliberately stricter, not a mirror.
pub fn write_proactive_section(path: &Path, entry: &ProactiveEntryWrite) -> Result<(), ConfigWriteError> {
    let mut doc = load_document(path)?;

    let (merged_enabled, merged_target, merged_max_per_window, merged_window_secs) = {
        let existing = doc.get("proactive").and_then(toml_edit::Item::as_table_like);
        let existing_bool = |key: &str| existing.and_then(|t| t.get(key)).and_then(|v| v.as_bool());
        let existing_str =
            |key: &str| existing.and_then(|t| t.get(key)).and_then(|v| v.as_str()).map(str::to_string);
        let existing_int = |key: &str| existing.and_then(|t| t.get(key)).and_then(|v| v.as_integer());

        let merged_enabled = entry.enabled.or_else(|| existing_bool("enabled")).unwrap_or(false);
        let merged_target = entry.target.clone().or_else(|| existing_str("target"));
        let merged_max_per_window = entry
            .max_per_window
            .or_else(|| existing_int("max_per_window").map(|n| n as u32))
            .unwrap_or(crate::DEFAULT_PROACTIVE_MAX_PER_WINDOW);
        let merged_window_secs = entry
            .window_secs
            .or_else(|| existing_int("window_secs").map(|n| n as u64))
            .unwrap_or(crate::DEFAULT_PROACTIVE_WINDOW_SECS);

        (merged_enabled, merged_target, merged_max_per_window, merged_window_secs)
    };

    if merged_enabled {
        let target = merged_target.as_deref().unwrap_or("").trim().to_string();
        if target.is_empty() {
            return Err(ConfigWriteError::InvalidProactiveConfig {
                reason: "`target` is required when proactive is enabled (must name a [[notify_target]]) \
                         — this would fail to boot the daemon on the next start"
                    .to_string(),
            });
        }
        let known_target = doc
            .get("notify_target")
            .and_then(toml_edit::Item::as_array_of_tables)
            .is_some_and(|arr| {
                arr.iter().any(|t| t.get("name").and_then(|v| v.as_str()) == Some(target.as_str()))
            });
        if !known_target {
            return Err(ConfigWriteError::InvalidProactiveConfig {
                reason: format!("target {target:?} does not name a configured [[notify_target]] entry"),
            });
        }
        if merged_max_per_window == 0 {
            return Err(ConfigWriteError::InvalidProactiveConfig {
                reason: "`max_per_window` must be >= 1 when proactive is enabled".to_string(),
            });
        }
        if merged_window_secs == 0 {
            return Err(ConfigWriteError::InvalidProactiveConfig {
                reason: "`window_secs` must be >= 1 when proactive is enabled".to_string(),
            });
        }
    }

    if let Some(v) = entry.enabled {
        doc["proactive"]["enabled"] = value(v);
    }
    if let Some(v) = &entry.target {
        doc["proactive"]["target"] = value(v.as_str());
    }
    if let Some(v) = entry.max_per_window {
        doc["proactive"]["max_per_window"] = value(v as i64);
    }
    if let Some(v) = entry.window_secs {
        doc["proactive"]["window_secs"] = value(v as i64);
    }

    write_toml_0600(path, &doc.to_string())
}

/// Read the `[proactive]` section as literally written on disk — no
/// resolution of anything. Absent section (or absent key) reads as
/// `None` for that field.
pub fn read_proactive_section(path: &Path) -> Result<ProactiveEntryWrite, ConfigWriteError> {
    let doc = load_document(path)?;
    let Some(table) = doc.get("proactive").and_then(toml_edit::Item::as_table_like) else {
        return Ok(ProactiveEntryWrite::default());
    };
    Ok(ProactiveEntryWrite {
        enabled: table.get("enabled").and_then(|v| v.as_bool()),
        target: table.get("target").and_then(|v| v.as_str()).map(str::to_string),
        max_per_window: table.get("max_per_window").and_then(|v| v.as_integer()).map(|n| n as u32),
        window_secs: table.get("window_secs").and_then(|v| v.as_integer()).map(|n| n as u64),
    })
}
```

`DEFAULT_PROACTIVE_MAX_PER_WINDOW`/`DEFAULT_PROACTIVE_WINDOW_SECS` are `pub const`s already declared in `crates/aivyx-config/src/lib.rs:2408`-2410 (`3` and `86_400`) — `crate::DEFAULT_PROACTIVE_MAX_PER_WINDOW` resolves as-is.

- [ ] **Step 5: Write the tests**

Add to the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn memory_profile_write_sets_the_key_and_preserves_siblings() {
        let path = temp_toml("mem-profile-write");
        std::fs::write(
            &path,
            "[memory]\nmax_per_topic = 200\nttl_secs = 86400\n\n[[memory.retention]]\ntopic_glob = \"daily-*\"\nretention = \"forever\"\n",
        )
        .unwrap();
        write_memory_profile(&path, Some("smart")).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("profile = \"smart\""));
        assert!(contents.contains("max_per_topic = 200"), "sibling key survives");
        assert!(contents.contains("[[memory.retention]]"), "retention array survives");
    }

    #[test]
    fn memory_profile_write_none_is_a_no_op() {
        let path = temp_toml("mem-profile-noop");
        std::fs::write(&path, "[memory]\nprofile = \"lite\"\n").unwrap();
        write_memory_profile(&path, None).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("profile = \"lite\""));
    }

    #[test]
    fn memory_profile_write_rejects_unknown_value() {
        let path = temp_toml("mem-profile-bad");
        std::fs::write(&path, "").unwrap();
        let err = write_memory_profile(&path, Some("turbo")).unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidMemoryProfile { .. }));
    }

    #[test]
    fn read_memory_profile_round_trips() {
        let path = temp_toml("mem-profile-read");
        std::fs::write(&path, "").unwrap();
        assert_eq!(read_memory_profile(&path).unwrap(), None);
        write_memory_profile(&path, Some("smart")).unwrap();
        assert_eq!(read_memory_profile(&path).unwrap(), Some("smart".to_string()));
    }

    #[test]
    fn embedding_write_touches_only_provided_fields() {
        let path = temp_toml("embedding-partial");
        std::fs::write(&path, "[embedding]\nbase_url = \"https://old.example\"\nmodel = \"old-model\"\napi_key = \"sk-existing\"\n").unwrap();
        write_embedding_section(
            &path,
            &EmbeddingEntryWrite { base_url: None, model: Some("new-model".to_string()), api_key: None },
        )
        .unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("base_url = \"https://old.example\""), "untouched field survives");
        assert!(contents.contains("model = \"new-model\""), "the actual edit applied");
        assert!(contents.contains("api_key = \"sk-existing\""), "secret untouched by None");
    }

    #[test]
    fn embedding_write_rejects_blank_base_url() {
        let path = temp_toml("embedding-blank-url");
        std::fs::write(&path, "").unwrap();
        let err = write_embedding_section(
            &path,
            &EmbeddingEntryWrite { base_url: Some("  ".to_string()), model: None, api_key: None },
        )
        .unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidEmbeddingConfig { .. }));
    }

    #[test]
    fn read_embedding_section_round_trips_and_never_needed_for_the_secret_itself() {
        let path = temp_toml("embedding-read");
        std::fs::write(&path, "").unwrap();
        write_embedding_section(
            &path,
            &EmbeddingEntryWrite {
                base_url: Some("https://api.openai.com".to_string()),
                model: Some("text-embedding-3-small".to_string()),
                api_key: Some("sk-real-secret".to_string()),
            },
        )
        .unwrap();
        let read = read_embedding_section(&path).unwrap();
        assert_eq!(read.base_url.as_deref(), Some("https://api.openai.com"));
        assert_eq!(read.model.as_deref(), Some("text-embedding-3-small"));
        // read_embedding_section itself returns the raw value (Task 4's
        // daemon handler is what redacts it before it reaches the wire) —
        // this test only proves the round-trip is byte-correct.
        assert_eq!(read.api_key.as_deref(), Some("sk-real-secret"));
    }

    #[test]
    fn proactive_write_rejects_enabling_without_a_known_target() {
        let path = temp_toml("proactive-no-target");
        std::fs::write(&path, "").unwrap();
        let err = write_proactive_section(
            &path,
            &ProactiveEntryWrite {
                enabled: Some(true),
                target: Some("nonexistent".to_string()),
                max_per_window: None,
                window_secs: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidProactiveConfig { .. }));
    }

    #[test]
    fn proactive_write_allows_enabling_with_a_known_target() {
        let path = temp_toml("proactive-known-target");
        std::fs::write(
            &path,
            "[[notify_target]]\nname = \"ops\"\nkind = \"telegram\"\nchat_id = \"123\"\nenabled = true\ndefault = false\n",
        )
        .unwrap();
        write_proactive_section(
            &path,
            &ProactiveEntryWrite {
                enabled: Some(true),
                target: Some("ops".to_string()),
                max_per_window: None,
                window_secs: None,
            },
        )
        .unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("[proactive]"));
        assert!(contents.contains("target = \"ops\""));
    }

    #[test]
    fn proactive_write_merged_state_keeps_succeeding_on_a_later_partial_save() {
        let path = temp_toml("proactive-merged");
        std::fs::write(
            &path,
            "[[notify_target]]\nname = \"ops\"\nkind = \"telegram\"\nchat_id = \"123\"\nenabled = true\ndefault = false\n",
        )
        .unwrap();
        write_proactive_section(
            &path,
            &ProactiveEntryWrite {
                enabled: Some(true),
                target: Some("ops".to_string()),
                max_per_window: Some(5),
                window_secs: Some(3600),
            },
        )
        .unwrap();
        // A later save only touches max_per_window — enabled/target
        // must be read from disk (merged), not treated as newly absent.
        write_proactive_section(
            &path,
            &ProactiveEntryWrite { enabled: None, target: None, max_per_window: Some(9), window_secs: None },
        )
        .unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("max_per_window = 9"));
        assert!(contents.contains("target = \"ops\""), "untouched field survives the merge");
    }

    #[test]
    fn proactive_write_rejects_zero_max_per_window_when_enabled() {
        let path = temp_toml("proactive-zero-max");
        std::fs::write(
            &path,
            "[[notify_target]]\nname = \"ops\"\nkind = \"telegram\"\nchat_id = \"123\"\nenabled = true\ndefault = false\n",
        )
        .unwrap();
        let err = write_proactive_section(
            &path,
            &ProactiveEntryWrite {
                enabled: Some(true),
                target: Some("ops".to_string()),
                max_per_window: Some(0),
                window_secs: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidProactiveConfig { .. }));
    }

    #[test]
    fn proactive_write_allows_disabling_without_a_target() {
        let path = temp_toml("proactive-disable");
        std::fs::write(&path, "").unwrap();
        write_proactive_section(
            &path,
            &ProactiveEntryWrite { enabled: Some(false), target: None, max_per_window: None, window_secs: None },
        )
        .unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("enabled = false"));
    }
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p aivyx-config memory_profile -- --nocapture && cargo test -p aivyx-config embedding_write -- --nocapture && cargo test -p aivyx-config read_embedding -- --nocapture && cargo test -p aivyx-config proactive -- --nocapture`
Expected: all 12 new tests pass (`ok`).

- [ ] **Step 7: Run clippy on the crate**

Run: `cargo clippy -p aivyx-config --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-config/src/config_write.rs
git commit -m "feat(config): add [memory] profile / [embedding] / [proactive] write primitives

Three singleton partial-update primitives, plan 2's leave-on-None
convention applied uniformly (not just to secrets). write_proactive_section
validates the MERGED post-write state and adds a target-existence check
against [[notify_target]] the loader itself doesn't make — deliberately
stricter, not a mirror.

POLISH_WAVES.md sub-project 7 plan 3, Task 2.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 3: `[[reflection_schedule]]` wire types + daemon handlers

**Files:**
- Modify: `crates/aivyx-ipc/src/protocol.rs`
- Modify: `crates/aivyx-channel/src/daemon_server.rs`

**Interfaces:**
- Consumes: Task 1's `ReflectionScheduleEntryWrite`, `write_reflection_schedule_section`, `remove_reflection_schedule_section`, `read_reflection_schedule_entries`.
- Produces: `QueryPayload::{GetReflectionScheduleConfigs, SetReflectionSchedule, DeleteReflectionSchedule}`, `QueryResponsePayload::{GetReflectionScheduleConfigs, ReflectionScheduleConfigApplied}`, `pub struct ReflectionScheduleConfigView { pub name: String, pub cron: String, pub lookback_window_secs: u64, pub enabled: bool }` — Task 5 (Studio UI) sends/receives these.

- [ ] **Step 1: Add the wire view struct**

In `crates/aivyx-ipc/src/protocol.rs`, add right after `pub struct NotifyTargetConfigView { ... }` (search for that struct to find the spot):

```rust
/// The **editable configuration** of one `[[reflection_schedule]]`
/// entry. `role_override`/`skip_when_idle`/`min_audit_entries_to_fire`
/// stay TOML-only — not represented here; a write preserves them via
/// the `KNOWN_KEYS` mechanism in `write_reflection_schedule_section`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionScheduleConfigView {
    pub name: String,
    pub cron: String,
    pub lookback_window_secs: u64,
    pub enabled: bool,
}
```

- [ ] **Step 2: Add the `QueryPayload` variants**

In `crates/aivyx-ipc/src/protocol.rs`, insert right before the closing `}` of `pub enum QueryPayload` (the enum ends right after `SetSlackConfig { ... },` — search for that variant, add these right after it, before the enum's final `}`):

```rust
    /// POLISH_WAVES.md sub-project 7 plan 3 — the editable
    /// `[[reflection_schedule]]` list. Responds with
    /// [`QueryResponsePayload::GetReflectionScheduleConfigs`].
    GetReflectionScheduleConfigs,
    /// Add or replace (by `name`) one `[[reflection_schedule]]` entry.
    /// Takes effect on the next daemon start. Responds with
    /// [`QueryResponsePayload::ReflectionScheduleConfigApplied`] (or
    /// `QueryError`).
    SetReflectionSchedule {
        name: String,
        cron: String,
        lookback_window_secs: u64,
        enabled: bool,
    },
    /// Remove one `[[reflection_schedule]]` entry by name (a no-op if
    /// absent). Responds with
    /// [`QueryResponsePayload::ReflectionScheduleConfigApplied`].
    DeleteReflectionSchedule { name: String },
```

- [ ] **Step 3: Add the `QueryResponsePayload` variants**

In the same file, insert right before the closing `}` of `pub enum QueryResponsePayload` (the enum ends right after `SlackConfigApplied { ... },` — add these right after it):

```rust
    /// Response to [`QueryPayload::GetReflectionScheduleConfigs`].
    GetReflectionScheduleConfigs { schedules: Vec<ReflectionScheduleConfigView> },
    /// Response to [`QueryPayload::SetReflectionSchedule`] /
    /// [`QueryPayload::DeleteReflectionSchedule`]. Carries the fresh
    /// list and `restart_required` (config is load-time), mirroring
    /// `NotifyTargetsApplied`'s own shape.
    ReflectionScheduleConfigApplied { schedules: Vec<ReflectionScheduleConfigView>, restart_required: bool },
```

- [ ] **Step 4: Add the daemon handlers**

In `crates/aivyx-channel/src/daemon_server.rs`, inside `async fn handle_query`, find the `QueryPayload::DeleteNotifyTarget { name } => { ... }` arm (search for it) and add these 3 new arms right after it:

```rust
        QueryPayload::GetReflectionScheduleConfigs => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            match read_reflection_schedule_configs(path) {
                Ok(schedules) => QueryResponsePayload::GetReflectionScheduleConfigs { schedules },
                Err(e) => QueryResponsePayload::QueryError {
                    code: "config_reload_failed".into(),
                    message: format!("failed to read reflection schedule configs: {e}"),
                },
            }
        }
        QueryPayload::SetReflectionSchedule { name, cron, lookback_window_secs, enabled } => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            let entry = aivyx_config::config_write::ReflectionScheduleEntryWrite {
                name: name.clone(),
                cron,
                lookback_window_secs,
                enabled,
            };
            match aivyx_config::config_write::write_reflection_schedule_section(path, &entry) {
                Ok(()) => {
                    audit_config_change(audit_log, "reflection_schedule", &format!("set {name}"));
                    reflection_schedules_applied_after_write(path, "set")
                }
                Err(e) => map_config_write_error(e),
            }
        }
        QueryPayload::DeleteReflectionSchedule { name } => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            match aivyx_config::config_write::remove_reflection_schedule_section(path, &name) {
                Ok(()) => {
                    audit_config_change(audit_log, "reflection_schedule", &format!("delete {name}"));
                    reflection_schedules_applied_after_write(path, "delete")
                }
                Err(e) => map_config_write_error(e),
            }
        }
```

- [ ] **Step 5: Add the two helper functions**

In the same file, find `fn notify_targets_applied_after_write` (search for it) and add these two new functions right after it:

```rust
/// Re-read `[[reflection_schedule]]` from disk into the wire view type —
/// shared by `GetReflectionScheduleConfigs`/`SetReflectionSchedule`/
/// `DeleteReflectionSchedule`'s handlers, mirroring
/// `read_notify_target_configs`'s own convention exactly.
fn read_reflection_schedule_configs(
    path: &std::path::Path,
) -> Result<Vec<aivyx_ipc::protocol::ReflectionScheduleConfigView>, String> {
    let entries = aivyx_config::config_write::read_reflection_schedule_entries(path)
        .map_err(|e| e.to_string())?;
    Ok(entries
        .into_iter()
        .map(|e| aivyx_ipc::protocol::ReflectionScheduleConfigView {
            name: e.name,
            cron: e.cron,
            lookback_window_secs: e.lookback_window_secs,
            enabled: e.enabled,
        })
        .collect())
}

/// Mirrors `notify_targets_applied_after_write`'s own convention
/// exactly, including surfacing a re-read failure as a `QueryError`
/// rather than silently claiming zero entries.
fn reflection_schedules_applied_after_write(path: &std::path::Path, verb: &str) -> QueryResponsePayload {
    match read_reflection_schedule_configs(path) {
        Ok(schedules) => QueryResponsePayload::ReflectionScheduleConfigApplied {
            schedules,
            restart_required: true,
        },
        Err(e) => QueryResponsePayload::QueryError {
            code: "config_reload_failed".into(),
            message: format!("reflection schedule {verb} succeeded, but reloading the list failed: {e}"),
        },
    }
}
```

- [ ] **Step 6: Add the `map_config_write_error` arm**

In the same file, find `fn map_config_write_error` and add this line to the `match &e { ... }` block (right after the `E::InvalidEmailConfig { .. } => "invalid_email_config",` line):

```rust
        E::InvalidReflectionSchedule { .. } => "invalid_reflection_schedule",
```

This match is exhaustive — the crate will not compile until this arm is added, which is the point (a new `ConfigWriteError` variant with no matching arm is a hard error, not a silent gap).

- [ ] **Step 7: Write a protocol round-trip test**

In `crates/aivyx-ipc/src/protocol.rs`, in the `#[cfg(test)] mod tests` block (search for `let msg = QueryPayload::SetNotifyTarget {` to find the neighborhood), add:

```rust
    #[test]
    fn set_reflection_schedule_round_trips_over_json() {
        let msg = QueryPayload::SetReflectionSchedule {
            name: "nightly".to_string(),
            cron: "0 0 9 * * * *".to_string(),
            lookback_window_secs: 86_400,
            enabled: true,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: QueryPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn reflection_schedule_config_view_round_trips_over_json() {
        let view = ReflectionScheduleConfigView {
            name: "nightly".to_string(),
            cron: "0 0 9 * * * *".to_string(),
            lookback_window_secs: 86_400,
            enabled: true,
        };
        let json = serde_json::to_string(&view).unwrap();
        let back: ReflectionScheduleConfigView = serde_json::from_str(&json).unwrap();
        assert_eq!(view, back);
    }
```

(`QueryPayload` needs `PartialEq` for the first test's `assert_eq!` — check the existing `#[derive(...)]` on `pub enum QueryPayload`; if `PartialEq` is already there — plan 2's own `set_notify_target_round_trips_over_json`-style test already relies on it — no change needed.)

- [ ] **Step 8: Write a daemon-server integration test**

In `crates/aivyx-channel/src/daemon_server.rs`'s `#[cfg(test)] mod tests` block, add a test that exercises the real write→read→view chain. This test module has **no `tempfile` crate dependency** — every existing temp-file test here (search for `fn secret_leak_temp_toml`) uses a hand-rolled `std::env::temp_dir()`-based helper instead; reuse that same helper rather than introducing a new dependency:

```rust
    #[test]
    fn reflection_schedule_configs_reflect_a_real_write() {
        let path = secret_leak_temp_toml("reflection-schedule");
        std::fs::write(&path, "").unwrap();
        aivyx_config::config_write::write_reflection_schedule_section(
            &path,
            &aivyx_config::config_write::ReflectionScheduleEntryWrite {
                name: "nightly".to_string(),
                cron: "0 0 9 * * * *".to_string(),
                lookback_window_secs: 86_400,
                enabled: true,
            },
        )
        .unwrap();
        let configs = read_reflection_schedule_configs(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "nightly");
        assert_eq!(configs[0].cron, "0 0 9 * * * *");
    }
```

- [ ] **Step 9: Run the tests**

Run: `cargo test -p aivyx-ipc reflection_schedule -- --nocapture && cargo test -p aivyx-channel reflection_schedule -- --nocapture`
Expected: all 3 new tests pass (`ok`).

- [ ] **Step 10: Run clippy on both crates**

Run: `cargo clippy -p aivyx-ipc -p aivyx-channel --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 11: Commit**

```bash
git add crates/aivyx-ipc/src/protocol.rs crates/aivyx-channel/src/daemon_server.rs
git commit -m "feat(ipc,channel): wire [[reflection_schedule]] CRUD to the daemon

GetReflectionScheduleConfigs/SetReflectionSchedule/DeleteReflectionSchedule
+ ReflectionScheduleConfigView, mirroring the notify-target CRUD wire
shape from plan 2 exactly.

POLISH_WAVES.md sub-project 7 plan 3, Task 3.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 4: `[memory]` / `[embedding]` / `[proactive]` wire types + daemon handlers

**Files:**
- Modify: `crates/aivyx-ipc/src/protocol.rs`
- Modify: `crates/aivyx-channel/src/daemon_server.rs`

**Interfaces:**
- Consumes: Task 2's `write_memory_profile`, `read_memory_profile`, `EmbeddingEntryWrite`, `write_embedding_section`, `read_embedding_section`, `ProactiveEntryWrite`, `write_proactive_section`, `read_proactive_section`; the existing `redact()` helper in `daemon_server.rs`.
- Produces: `QueryPayload::{GetMemoryProfileConfig, SetMemoryProfile, GetEmbeddingConfig, SetEmbeddingConfig, GetProactiveConfig, SetProactiveConfig}`, `QueryResponsePayload::{GetMemoryProfileConfig, MemoryProfileConfigApplied, GetEmbeddingConfig, EmbeddingConfigApplied, GetProactiveConfig, ProactiveConfigApplied}`, `pub struct MemoryProfileConfigView { pub profile: String }`, `pub struct EmbeddingConfigView { pub base_url: Option<String>, pub model: Option<String>, pub api_key: RedactedSecret }`, `pub struct ProactiveConfigView { pub enabled: bool, pub target: Option<String>, pub max_per_window: u32, pub window_secs: u64 }` — Task 6 (Studio UI) sends/receives these.

- [ ] **Step 1: Add the 3 wire view structs**

In `crates/aivyx-ipc/src/protocol.rs`, add right after Task 3's `ReflectionScheduleConfigView`:

```rust
/// The **editable configuration** of `[memory] profile` — a 3-way
/// switch (`"off" | "lite" | "smart"`), not the 2-way lite/smart
/// picker an earlier design pass assumed (`MemoryProfile` has 3
/// variants — see `aivyx-config/src/lib.rs:2706`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryProfileConfigView {
    pub profile: String,
}

/// The **editable configuration** of the `[embedding]` section's
/// primary fields. `api_key` never carries the real value (see
/// [`RedactedSecret`]). `dimensions`/`rag_top_k`/`rag_min_similarity`/
/// `recall_window_turns`/`recall_gate_min_chars` and the Chapter Loom
/// recall-fusion tuning fields stay TOML-only — not represented here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingConfigView {
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key: RedactedSecret,
}

/// The **editable configuration** of the `[proactive]` section's
/// primary fields. `signals` (the 3 `signal_*` toggles) stays
/// TOML-only — not represented here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProactiveConfigView {
    pub enabled: bool,
    pub target: Option<String>,
    pub max_per_window: u32,
    pub window_secs: u64,
}
```

- [ ] **Step 2: Add the `QueryPayload` variants**

In the same file, insert right before the enum's closing `}`, after Task 3's `DeleteReflectionSchedule { name: String },`:

```rust
    /// Response: [`QueryResponsePayload::GetMemoryProfileConfig`].
    GetMemoryProfileConfig,
    /// `profile`: `"off" | "lite" | "smart"`. Always sent — the Studio
    /// picker has no blank state (unlike every other `SetX` in this
    /// sub-project, this field is a plain `String`, not `Option`).
    /// Responds with [`QueryResponsePayload::MemoryProfileConfigApplied`].
    SetMemoryProfile { profile: String },
    /// Response: [`QueryResponsePayload::GetEmbeddingConfig`].
    GetEmbeddingConfig,
    /// `api_key: None` means leave the existing key untouched. Responds
    /// with [`QueryResponsePayload::EmbeddingConfigApplied`].
    SetEmbeddingConfig {
        #[serde(default)]
        base_url: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        api_key: Option<String>,
    },
    /// Response: [`QueryResponsePayload::GetProactiveConfig`].
    GetProactiveConfig,
    /// Responds with [`QueryResponsePayload::ProactiveConfigApplied`].
    SetProactiveConfig {
        #[serde(default)]
        enabled: Option<bool>,
        #[serde(default)]
        target: Option<String>,
        #[serde(default)]
        max_per_window: Option<u32>,
        #[serde(default)]
        window_secs: Option<u64>,
    },
```

- [ ] **Step 3: Add the `QueryResponsePayload` variants**

In the same file, insert right before the enum's closing `}`, after Task 3's `ReflectionScheduleConfigApplied { ... },`:

```rust
    GetMemoryProfileConfig { config: MemoryProfileConfigView },
    MemoryProfileConfigApplied { config: MemoryProfileConfigView, restart_required: bool },
    GetEmbeddingConfig { config: EmbeddingConfigView },
    EmbeddingConfigApplied { config: EmbeddingConfigView, restart_required: bool },
    GetProactiveConfig { config: ProactiveConfigView },
    ProactiveConfigApplied { config: ProactiveConfigView, restart_required: bool },
```

- [ ] **Step 4: Add the daemon handlers**

In `crates/aivyx-channel/src/daemon_server.rs`'s `handle_query`, add these 6 arms right after Task 3's `QueryPayload::DeleteReflectionSchedule` arm:

```rust
        QueryPayload::GetMemoryProfileConfig => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            match aivyx_config::config_write::read_memory_profile(path) {
                Ok(profile) => QueryResponsePayload::GetMemoryProfileConfig {
                    config: aivyx_ipc::protocol::MemoryProfileConfigView {
                        profile: profile.unwrap_or_else(|| "off".to_string()),
                    },
                },
                Err(e) => map_config_write_error(e),
            }
        }
        QueryPayload::SetMemoryProfile { profile } => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            match aivyx_config::config_write::write_memory_profile(path, Some(&profile)) {
                Ok(()) => {
                    audit_config_change(audit_log, "memory", &format!("profile = {profile}"));
                    match aivyx_config::config_write::read_memory_profile(path) {
                        Ok(p) => QueryResponsePayload::MemoryProfileConfigApplied {
                            config: aivyx_ipc::protocol::MemoryProfileConfigView {
                                profile: p.unwrap_or_else(|| "off".to_string()),
                            },
                            restart_required: true,
                        },
                        Err(e) => QueryResponsePayload::QueryError {
                            code: "config_reload_failed".into(),
                            message: format!("memory profile saved, but reloading it failed: {e}"),
                        },
                    }
                }
                Err(e) => map_config_write_error(e),
            }
        }
        QueryPayload::GetEmbeddingConfig => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            match aivyx_config::config_write::read_embedding_section(path) {
                Ok(e) => QueryResponsePayload::GetEmbeddingConfig { config: embedding_config_view(&e) },
                Err(err) => map_config_write_error(err),
            }
        }
        QueryPayload::SetEmbeddingConfig { base_url, model, api_key } => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            let entry = aivyx_config::config_write::EmbeddingEntryWrite { base_url, model, api_key };
            match aivyx_config::config_write::write_embedding_section(path, &entry) {
                Ok(()) => {
                    audit_config_change(audit_log, "embedding", "updated");
                    match aivyx_config::config_write::read_embedding_section(path) {
                        Ok(e) => QueryResponsePayload::EmbeddingConfigApplied {
                            config: embedding_config_view(&e),
                            restart_required: true,
                        },
                        Err(err) => QueryResponsePayload::QueryError {
                            code: "config_reload_failed".into(),
                            message: format!("embedding config saved, but reloading it failed: {err}"),
                        },
                    }
                }
                Err(e) => map_config_write_error(e),
            }
        }
        QueryPayload::GetProactiveConfig => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            match aivyx_config::config_write::read_proactive_section(path) {
                Ok(p) => QueryResponsePayload::GetProactiveConfig { config: proactive_config_view(&p) },
                Err(err) => map_config_write_error(err),
            }
        }
        QueryPayload::SetProactiveConfig { enabled, target, max_per_window, window_secs } => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            let entry =
                aivyx_config::config_write::ProactiveEntryWrite { enabled, target, max_per_window, window_secs };
            match aivyx_config::config_write::write_proactive_section(path, &entry) {
                Ok(()) => {
                    audit_config_change(audit_log, "proactive", "updated");
                    match aivyx_config::config_write::read_proactive_section(path) {
                        Ok(p) => QueryResponsePayload::ProactiveConfigApplied {
                            config: proactive_config_view(&p),
                            restart_required: true,
                        },
                        Err(err) => QueryResponsePayload::QueryError {
                            code: "config_reload_failed".into(),
                            message: format!("proactive config saved, but reloading it failed: {err}"),
                        },
                    }
                }
                Err(e) => map_config_write_error(e),
            }
        }
```

- [ ] **Step 5: Add the two view-builder helper functions**

In the same file, right after `fn email_config_view` (search for it), add:

```rust
fn embedding_config_view(e: &aivyx_config::config_write::EmbeddingEntryWrite) -> aivyx_ipc::protocol::EmbeddingConfigView {
    aivyx_ipc::protocol::EmbeddingConfigView {
        base_url: e.base_url.clone(),
        model: e.model.clone(),
        api_key: redact(e.api_key.as_deref()),
    }
}

fn proactive_config_view(p: &aivyx_config::config_write::ProactiveEntryWrite) -> aivyx_ipc::protocol::ProactiveConfigView {
    aivyx_ipc::protocol::ProactiveConfigView {
        enabled: p.enabled.unwrap_or(false),
        target: p.target.clone(),
        max_per_window: p.max_per_window.unwrap_or(aivyx_config::DEFAULT_PROACTIVE_MAX_PER_WINDOW),
        window_secs: p.window_secs.unwrap_or(aivyx_config::DEFAULT_PROACTIVE_WINDOW_SECS),
    }
}
```

- [ ] **Step 6: Add the 3 `map_config_write_error` arms**

In `fn map_config_write_error`, add right after the `E::InvalidReflectionSchedule { .. } => "invalid_reflection_schedule",` line Task 3 added:

```rust
        E::InvalidMemoryProfile { .. } => "invalid_memory_profile",
        E::InvalidEmbeddingConfig { .. } => "invalid_embedding_config",
        E::InvalidProactiveConfig { .. } => "invalid_proactive_config",
```

- [ ] **Step 7: Write a secret-redaction test**

In `crates/aivyx-channel/src/daemon_server.rs`'s test module, add (mirroring `email_config_view_never_leaks_the_raw_password`'s shape):

```rust
    #[test]
    fn embedding_config_view_never_leaks_the_raw_api_key() {
        let entry = aivyx_config::config_write::EmbeddingEntryWrite {
            base_url: Some("https://api.openai.com".to_string()),
            model: Some("text-embedding-3-small".to_string()),
            api_key: Some("sk-super-secret-value".to_string()),
        };
        let view = embedding_config_view(&entry);
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("sk-super-secret-value"), "the real key must never reach the wire");
        assert!(view.api_key.configured);
        assert_eq!(view.api_key.source, "toml");
    }

    #[test]
    fn embedding_write_with_none_api_key_leaves_the_existing_secret_on_disk() {
        let path = secret_leak_temp_toml("embedding-rotate");
        std::fs::write(&path, "").unwrap();
        aivyx_config::config_write::write_embedding_section(
            &path,
            &aivyx_config::config_write::EmbeddingEntryWrite {
                base_url: Some("https://api.openai.com".to_string()),
                model: None,
                api_key: Some("sk-original-secret".to_string()),
            },
        )
        .unwrap();
        // A later save rotates only the model, api_key: None.
        aivyx_config::config_write::write_embedding_section(
            &path,
            &aivyx_config::config_write::EmbeddingEntryWrite {
                base_url: None,
                model: Some("text-embedding-3-large".to_string()),
                api_key: None,
            },
        )
        .unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(contents.contains("sk-original-secret"), "None must not clear the existing secret");
        assert!(contents.contains("text-embedding-3-large"));
    }

    #[test]
    fn proactive_config_view_defaults_match_the_loader_when_section_absent() {
        let entry = aivyx_config::config_write::ProactiveEntryWrite::default();
        let view = proactive_config_view(&entry);
        assert!(!view.enabled);
        assert_eq!(view.max_per_window, aivyx_config::DEFAULT_PROACTIVE_MAX_PER_WINDOW);
        assert_eq!(view.window_secs, aivyx_config::DEFAULT_PROACTIVE_WINDOW_SECS);
    }
```

- [ ] **Step 8: Run the tests**

Run: `cargo test -p aivyx-channel embedding_config -- --nocapture && cargo test -p aivyx-channel embedding_write -- --nocapture && cargo test -p aivyx-channel proactive_config -- --nocapture`
Expected: all 3 new tests pass (`ok`).

- [ ] **Step 9: Run clippy on both crates**

Run: `cargo clippy -p aivyx-ipc -p aivyx-channel --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 10: Commit**

```bash
git add crates/aivyx-ipc/src/protocol.rs crates/aivyx-channel/src/daemon_server.rs
git commit -m "feat(ipc,channel): wire [memory] profile / [embedding] / [proactive] to the daemon

GetMemoryProfileConfig/SetMemoryProfile, GetEmbeddingConfig/
SetEmbeddingConfig, GetProactiveConfig/SetProactiveConfig +
MemoryProfileConfigView/EmbeddingConfigView/ProactiveConfigView.
embedding_config_view redacts api_key via the existing RedactedSecret
convention; proactive_config_view fills the loader's own defaults when
the section is absent.

POLISH_WAVES.md sub-project 7 plan 3, Task 4.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 5: Studio — Reflection schedules UI (SchedulesPanel)

**Files:**
- Modify: `crates/aivyx-web/src/main.rs`

**Interfaces:**
- Consumes: Task 3's `QueryPayload::{GetReflectionScheduleConfigs, SetReflectionSchedule, DeleteReflectionSchedule}`, `QueryResponsePayload::{GetReflectionScheduleConfigs, ReflectionScheduleConfigApplied}`, `ReflectionScheduleConfigView`.
- Produces: nothing further tasks depend on — this is the UI leaf.

- [ ] **Step 1: Import the new wire type**

In `crates/aivyx-web/src/main.rs`, extend the `use aivyx_ipc::protocol::{ ... };` import block (near the top of the file) to add `ReflectionScheduleConfigView` alphabetically — e.g. right after `QueryResponsePayload,` add it on the `ScheduleView,` line's neighborhood:

```rust
    QueryResponsePayload, ReflectionScheduleConfigView, ScheduleView, SeedSkillWire, SessionSummary, SettingsSnapshot,
```

- [ ] **Step 2: Add the `reflection_schedules` field to `Dashboard`**

Find `struct Dashboard { ... }` (search for `struct Dashboard`) and add a new field right after `schedules: Vec<ScheduleView>,`:

```rust
    /// POLISH_WAVES.md sub-project 7 plan 3 — the editable
    /// `[[reflection_schedule]]` list, distinct from `schedules` above
    /// (regular `[[schedule]]` entries). Populated by
    /// `GetReflectionScheduleConfigs`/`ReflectionScheduleConfigApplied`.
    reflection_schedules: Vec<ReflectionScheduleConfigView>,
```

(`Dashboard` already derives `Default`, so `Vec::new()` is the automatic default — no other change needed for the struct itself.)

- [ ] **Step 3: Add the 3 query builders**

Find `fn schedules_refresh_query() -> FrontendMessage { ... }` (search for it) and add right after it:

```rust
fn reflection_schedule_configs_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-reflection-configs".to_string(),
        payload: QueryPayload::GetReflectionScheduleConfigs,
    }
}
fn set_reflection_schedule_query(entry: ReflectionScheduleConfigView) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-reflection-set".to_string(),
        payload: QueryPayload::SetReflectionSchedule {
            name: entry.name,
            cron: entry.cron,
            lookback_window_secs: entry.lookback_window_secs,
            enabled: entry.enabled,
        },
    }
}
fn delete_reflection_schedule_query(name: String) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-reflection-delete".to_string(),
        payload: QueryPayload::DeleteReflectionSchedule { name },
    }
}
```

- [ ] **Step 4: Add `ReflectionScheduleForm`**

Add this new component right after `fn NotifyTargetForm(...)`'s closing `}` (search for that function to find the spot):

```rust
/// POLISH_WAVES.md sub-project 7 plan 3 — add/edit form for one
/// `[[reflection_schedule]]` entry. Duplicates (rather than extracting
/// into a shared component) the freq/time/day cron builder
/// `SchedulesPanel`'s own regular-schedule create form already has —
/// that builder is inline in `SchedulesPanel`, not its own component,
/// and this is the only other cron-shaped form in the codebase.
#[component]
fn ReflectionScheduleForm(
    initial: Option<ReflectionScheduleConfigView>,
    on_cancel: EventHandler<()>,
    on_save: EventHandler<ReflectionScheduleConfigView>,
) -> Element {
    let seed = initial.clone().unwrap_or(ReflectionScheduleConfigView {
        name: String::new(),
        cron: String::new(),
        lookback_window_secs: 86_400,
        enabled: true,
    });
    let editing_existing = initial.is_some();
    let mut ui = use_context::<Signal<SchedulesUi>>();
    let mut name = use_signal(|| seed.name.clone());
    let mut freq = use_signal(|| "daily".to_string());
    let mut at_time = use_signal(|| "09:00".to_string());
    let mut weekday = use_signal(|| "Mon".to_string());
    let mut every_hours = use_signal(|| "6".to_string());
    let mut cron = use_signal(|| seed.cron.clone());
    let mut lookback_hours = use_signal(|| (seed.lookback_window_secs / 3600).max(1).to_string());
    let mut enabled = use_signal(|| seed.enabled);

    let built = use_memo(move || {
        let (h, m) = {
            let t = at_time();
            let mut it = t.splitn(2, ':');
            let h = it.next().unwrap_or("9").trim_start_matches('0');
            let m = it.next().unwrap_or("0").trim_start_matches('0');
            (
                if h.is_empty() { "0".to_string() } else { h.to_string() },
                if m.is_empty() { "0".to_string() } else { m.to_string() },
            )
        };
        match freq().as_str() {
            "daily" => format!("0 {m} {h} * * * *"),
            "weekly" => format!("0 {m} {h} * * {} *", weekday()),
            "hourly" => format!("0 0 */{} * * * *", every_hours()),
            _ => cron().trim().to_string(),
        }
    });

    rsx! {
        div { class: "glass-card",
            div { class: "field-row",
                label { "Name" }
                input { class: "input", value: "{name}", disabled: editing_existing, oninput: move |e| name.set(e.value()) }
            }
            label { class: "label-tech", "When" }
            select {
                class: "input",
                value: "{freq}",
                onchange: move |e| freq.set(e.value()),
                option { value: "daily", "Every day" }
                option { value: "weekly", "Once a week" }
                option { value: "hourly", "Every few hours" }
                option { value: "custom", "Custom (advanced)" }
            }
            if freq() == "daily" || freq() == "weekly" {
                div { style: "display:flex; gap:8px; align-items:center; margin:6px 0;",
                    if freq() == "weekly" {
                        select {
                            class: "input", style: "flex:1;", value: "{weekday}",
                            onchange: move |e| weekday.set(e.value()),
                            option { value: "Mon", "Monday" }
                            option { value: "Tue", "Tuesday" }
                            option { value: "Wed", "Wednesday" }
                            option { value: "Thu", "Thursday" }
                            option { value: "Fri", "Friday" }
                            option { value: "Sat", "Saturday" }
                            option { value: "Sun", "Sunday" }
                        }
                    }
                    span { class: "label-tech", "at" }
                    input { class: "input", style: "flex:1;", r#type: "time", value: "{at_time}", oninput: move |e| at_time.set(e.value()) }
                }
            }
            if freq() == "hourly" {
                div { style: "display:flex; gap:8px; align-items:center; margin:6px 0;",
                    span { class: "label-tech", "every" }
                    select {
                        class: "input", style: "flex:1;", value: "{every_hours}",
                        onchange: move |e| every_hours.set(e.value()),
                        option { value: "1", "1 hour" }
                        option { value: "2", "2 hours" }
                        option { value: "3", "3 hours" }
                        option { value: "4", "4 hours" }
                        option { value: "6", "6 hours" }
                        option { value: "12", "12 hours" }
                    }
                }
            }
            if freq() == "custom" {
                label { class: "label-tech", "Cron (sec min hour dom month dow year — local time)" }
                input { class: "input", placeholder: "0 0 9 * * * *", value: "{cron}", oninput: move |e| cron.set(e.value()) }
            }
            p { class: "label-tech", style: "opacity:0.7; margin:4px 0;", "cron: {built()}" }
            div { class: "field-row",
                label { "Lookback (hours)" }
                input { class: "input", value: "{lookback_hours}", oninput: move |e| lookback_hours.set(e.value()) }
            }
            div { class: "field-row",
                label { "Enabled" }
                input { r#type: "checkbox", checked: enabled(), onchange: move |e| enabled.set(e.checked()) }
            }
            div { style: "display:flex; gap:8px; margin-top:12px;",
                button {
                    class: "btn btn-primary btn-xs",
                    onclick: move |_| {
                        let n = name().trim().to_string();
                        let c = built();
                        if n.is_empty() || c.is_empty() {
                            ui.write().notice = Some((false, "name and cron are both required".into()));
                            return;
                        }
                        let hours = lookback_hours().trim().parse::<u64>().unwrap_or(24).max(1);
                        on_save.call(ReflectionScheduleConfigView {
                            name: n,
                            cron: c,
                            lookback_window_secs: hours * 3600,
                            enabled: enabled(),
                        });
                    },
                    "Save"
                }
                button { class: "btn btn-glass btn-xs", onclick: move |_| on_cancel.call(()), "Cancel" }
            }
        }
    }
}
```

- [ ] **Step 5: Extend `SchedulesPanel`**

In `fn SchedulesPanel() -> Element { ... }` (search for it):

5a. Add two new `use_signal`s right after `let mut start_enabled = use_signal(|| true);`:

```rust
    // POLISH_WAVES.md sub-project 7 plan 3 — the reflection-schedules
    // section's own add/edit state, separate from the regular-schedule
    // form above.
    let mut refl_editing = use_signal(|| None::<ReflectionScheduleConfigView>);
    let mut refl_adding = use_signal(|| false);
```

5b. Extend the top `use_future` to also load the reflection-schedule list — change:

```rust
    // Fresh list on open (the 5 s poll keeps it live afterwards).
    use_future(move || async move {
        ws.send(schedules_refresh_query());
    });
```

to:

```rust
    // Fresh list on open (the 5 s poll keeps it live afterwards).
    use_future(move || async move {
        ws.send(schedules_refresh_query());
        ws.send(reflection_schedule_configs_query());
    });
```

5c. Add the new "Reflection schedules" section as a second `section { class: "panel", ... }` inside `div { class: "dash-main", ... }`, right after the first one's closing `}` (the existing regular-schedule list section) and before `aside { class: "dash-rail", ...}` begins:

```rust
                section { class: "panel",
                    div { class: "panel-head",
                        h3 { "Reflection schedules" }
                        button {
                            class: "btn btn-primary btn-xs",
                            onclick: move |_| { refl_editing.set(None); refl_adding.set(true); },
                            "Add reflection schedule"
                        }
                    }
                    p { class: "label-tech",
                        "Distinct from regular schedules above — each entry fires a canonical \
                         reflection turn (outcome-summary input, persistent proposal store) on \
                         its own cron. role_override/skip_when_idle/min_audit_entries_to_fire \
                         stay TOML-only for now."
                    }
                    if refl_adding() || refl_editing().is_some() {
                        ReflectionScheduleForm {
                            key: "{refl_editing().map(|e| e.name.clone()).unwrap_or_else(|| \"new\".to_string())}",
                            initial: refl_editing(),
                            on_cancel: move |_| { refl_adding.set(false); refl_editing.set(None); },
                            on_save: move |entry: ReflectionScheduleConfigView| {
                                ws.send(set_reflection_schedule_query(entry));
                                refl_adding.set(false);
                                refl_editing.set(None);
                            },
                        }
                    } else if dashboard().reflection_schedules.is_empty() {
                        div { class: "glass-card empty", p { class: "label-tech", "No `[[reflection_schedule]]` entries configured yet." } }
                    } else {
                        div { class: "feed",
                            for r in dashboard().reflection_schedules.iter() {
                                {
                                    let r2 = r.clone();
                                    let rname = r.name.clone();
                                    rsx! {
                                        div { key: "{r.name}", class: "glass-card routine-row",
                                            div { class: "row1",
                                                span { class: "dot live" }
                                                span { class: "name", "{r.name}" }
                                                span { class: "label-tech", style: "opacity:0.7;", "{r.cron}" }
                                                span { class: if r.enabled { "chip sage" } else { "chip" }, if r.enabled { "enabled" } else { "disabled" } }
                                            }
                                            div { style: "display:flex; gap:8px; margin-top:8px;",
                                                button { class: "btn btn-glass btn-xs", onclick: move |_| { refl_adding.set(false); refl_editing.set(Some(r2.clone())); }, "Edit" }
                                                button { class: "btn btn-glass btn-xs", onclick: move |_| ws.send(delete_reflection_schedule_query(rname.clone())), "Delete" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
```

- [ ] **Step 6: Wire the `read_task` response handlers**

In `async fn read_task(...)`, find `DaemonEnvelope::QueryResponse { payload: QueryResponsePayload::Schedules { schedules }, .. } => { dashboard.write().schedules = schedules; }` (search for `QueryResponsePayload::Schedules {`) and add these two new arms right after it:

```rust
                // POLISH_WAVES.md sub-project 7 plan 3 — the editable
                // reflection-schedule list, mirroring the notify-target
                // list's own two handlers.
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetReflectionScheduleConfigs { schedules },
                    ..
                } => {
                    dashboard.write().reflection_schedules = schedules;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::ReflectionScheduleConfigApplied { schedules, .. },
                    ..
                } => {
                    dashboard.write().reflection_schedules = schedules;
                    schedules_ui.write().notice = Some((true, "Saved — restart the daemon to apply.".to_string()));
                }
```

- [ ] **Step 7: Add the error-routing arm**

Find the existing `if id.starts_with("mc-notify") => { notify_config_ui.write().notice = Some((false, message)); }` arm (search for `id.starts_with("mc-notify")`) and add a new arm right after it:

```rust
                // POLISH_WAVES.md sub-project 7 plan 3 — a reflection-
                // schedule config save/delete failure. Ids are prefixed
                // `mc-reflection` so it lands on the Schedules screen's
                // banner (shared with the regular-schedule mutations'
                // own `ScheduleMutated` notice).
                DaemonEnvelope::QueryResponse {
                    id,
                    payload: QueryResponsePayload::QueryError { message, .. },
                } if id.starts_with("mc-reflection") => {
                    schedules_ui.write().notice = Some((false, message));
                }
```

- [ ] **Step 8: Compile-check (wasm target)**

Run (needs the wasm32 toolchain on `PATH`; see Global Constraints):

```bash
PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH" \
  cargo build -p aivyx-web --target wasm32-unknown-unknown
```

Expected: builds clean, no errors.

- [ ] **Step 9: Run clippy (wasm target)**

Run:

```bash
PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH" \
  cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings
```

Expected: no warnings.

- [ ] **Step 10: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "feat(web): reflection-schedule CRUD in the Schedules screen

New 'Reflection schedules' section in SchedulesPanel, distinct from the
regular-schedule list above it — ReflectionScheduleForm duplicates (not
extracts) the same freq/time/day cron builder, keyed on the entry being
edited so switching 'editing X' -> 'adding new' never carries stale
form state. Dashboard gains reflection_schedules: Vec<...>; no new
UI-state struct needed.

POLISH_WAVES.md sub-project 7 plan 3, Task 5.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 6: Studio — Settings coverage UI (SettingsPanel)

**Files:**
- Modify: `crates/aivyx-web/src/main.rs`

**Interfaces:**
- Consumes: Task 4's `QueryPayload::{GetMemoryProfileConfig, SetMemoryProfile, GetEmbeddingConfig, SetEmbeddingConfig, GetProactiveConfig, SetProactiveConfig}`, `QueryResponsePayload::{GetMemoryProfileConfig, MemoryProfileConfigApplied, GetEmbeddingConfig, EmbeddingConfigApplied, GetProactiveConfig, ProactiveConfigApplied}`, `MemoryProfileConfigView`, `EmbeddingConfigView`, `ProactiveConfigView`; plan 2's `notify_target_configs_query()` and `NotifyTargetConfigView`/`NotificationsState.configs` (for the proactive-target picker).
- Produces: nothing further tasks depend on — this is the UI leaf and the plan's last code task.

- [ ] **Step 1: Import the new wire types**

Extend the same `use aivyx_ipc::protocol::{ ... };` block Task 5 touched, adding `EmbeddingConfigView`, `MemoryProfileConfigView`, `ProactiveConfigView` alphabetically (e.g. `EmbeddingConfigView` next to `EmailConfigView`, `MemoryProfileConfigView` next to `MemoryGraphNode`, `ProactiveConfigView` next to `PersonaProposalSummary`).

- [ ] **Step 2: Add the 3 fields to `SettingsState`**

Find `struct SettingsState { ... }` (search for `struct SettingsState`) and add 3 new fields right after `restart_required: bool,`:

```rust
    /// POLISH_WAVES.md sub-project 7 plan 3 — `[memory] profile`.
    memory_profile: Option<MemoryProfileConfigView>,
    /// `[embedding]`'s primary fields.
    embedding: Option<EmbeddingConfigView>,
    /// `[proactive]`'s primary fields.
    proactive: Option<ProactiveConfigView>,
```

(`SettingsState` already derives `Default`, so all 3 default to `None` — no other change needed for the struct itself.)

- [ ] **Step 3: Add the 6 query builders**

Find `fn get_settings_query() -> FrontendMessage { ... }` (search for it) and add right after it:

```rust
/// POLISH_WAVES.md sub-project 7 plan 3 — the Settings-coverage query
/// builders. All 3 sections share the "mc-settings" id prefix, which
/// the existing `id.starts_with("mc-settings")` `QueryError` routing
/// arm already catches (same tradeoff `mc-mcp`/`mc-notify` already
/// make for their own status/poll ids).
fn memory_profile_config_query() -> FrontendMessage {
    FrontendMessage::Query { id: "mc-settings-memory".to_string(), payload: QueryPayload::GetMemoryProfileConfig }
}
fn set_memory_profile_query(profile: String) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-settings-memory".to_string(),
        payload: QueryPayload::SetMemoryProfile { profile },
    }
}
fn embedding_config_query() -> FrontendMessage {
    FrontendMessage::Query { id: "mc-settings-embedding".to_string(), payload: QueryPayload::GetEmbeddingConfig }
}
fn set_embedding_config_query(base_url: Option<String>, model: Option<String>, api_key: Option<String>) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-settings-embedding".to_string(),
        payload: QueryPayload::SetEmbeddingConfig { base_url, model, api_key },
    }
}
fn proactive_config_query() -> FrontendMessage {
    FrontendMessage::Query { id: "mc-settings-proactive".to_string(), payload: QueryPayload::GetProactiveConfig }
}
fn set_proactive_config_query(
    enabled: Option<bool>,
    target: Option<String>,
    max_per_window: Option<u32>,
    window_secs: Option<u64>,
) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-settings-proactive".to_string(),
        payload: QueryPayload::SetProactiveConfig { enabled, target, max_per_window, window_secs },
    }
}
```

- [ ] **Step 4: Add `MemoryProfileCard`**

Add this new component right after `fn EmailAdapterCard(...)`'s closing `}` (search for that function to find the spot — keep the 3 new cards grouped together near it, before `fn ChannelAdaptersSection`):

```rust
/// POLISH_WAVES.md sub-project 7 plan 3 — `[memory] profile` picker.
/// Keyed on the seed data by its caller (like `EmailAdapterCard`), so a
/// changed on-disk value re-seeds the form instead of leaving stale
/// local state.
#[component]
fn MemoryProfileCard(config: MemoryProfileConfigView) -> Element {
    let ws = use_context::<Sender>();
    let mut profile = use_signal(|| config.profile.clone());

    rsx! {
        div { class: "glass-card settings-section",
            div { class: "panel-head", h3 { "Memory profile" } span { class: "chip", "{config.profile}" } }
            p { class: "label-tech",
                "Off: today's behavior. Lite: recall fusion over existing memory, no paid \
                 generation. Smart: adds the wiki/graph extraction sweeps. Takes effect on \
                 the next daemon restart."
            }
            div { class: "field-row",
                label { "Profile" }
                select {
                    class: "input",
                    value: "{profile}",
                    onchange: move |e| profile.set(e.value()),
                    option { value: "off", "off" }
                    option { value: "lite", "lite" }
                    option { value: "smart", "smart" }
                }
            }
            button {
                class: "btn btn-primary btn-xs",
                onclick: move |_| ws.send(set_memory_profile_query(profile())),
                "Save"
            }
        }
    }
}

/// POLISH_WAVES.md sub-project 7 plan 3 — `[embedding]`'s primary
/// fields. `api_key`'s masked "configured"/"not set" + blank-input UX
/// mirrors `EmailAdapterCard`'s password field exactly.
#[component]
fn EmbeddingConfigCard(config: EmbeddingConfigView) -> Element {
    let ws = use_context::<Sender>();
    let mut base_url = use_signal(|| config.base_url.clone().unwrap_or_default());
    let mut model = use_signal(|| config.model.clone().unwrap_or_default());
    let mut api_key = use_signal(String::new);

    rsx! {
        div { class: "glass-card settings-section",
            div { class: "panel-head", h3 { "Embedding" } }
            p { class: "label-tech",
                "Configures the OpenAI-compatible embedding backend that powers semantic memory \
                 search. Point base_url at a local server to keep memory content on this box. \
                 Takes effect on the next daemon restart."
            }
            div { class: "field-row",
                label { "Base URL" }
                input { class: "input", placeholder: "https://api.openai.com", value: "{base_url}", oninput: move |e| base_url.set(e.value()) }
            }
            div { class: "field-row",
                label { "Model" }
                input { class: "input", placeholder: "text-embedding-3-small", value: "{model}", oninput: move |e| model.set(e.value()) }
            }
            p { class: "label-tech",
                {if config.api_key.configured { "API key: configured" } else { "API key: not set" }}
            }
            input { class: "input", placeholder: "New API key (leave blank to keep current)",
                r#type: "password", value: "{api_key}",
                oninput: move |e| api_key.set(e.value()) }
            button {
                class: "btn btn-primary btn-xs",
                onclick: move |_| {
                    let b = base_url();
                    let m = model();
                    let k = api_key();
                    ws.send(set_embedding_config_query(
                        if b.trim().is_empty() { None } else { Some(b.trim().to_string()) },
                        if m.trim().is_empty() { None } else { Some(m.trim().to_string()) },
                        if k.trim().is_empty() { None } else { Some(k.trim().to_string()) },
                    ));
                    api_key.set(String::new());
                },
                "Save"
            }
        }
    }
}

/// POLISH_WAVES.md sub-project 7 plan 3 — `[proactive]`'s primary
/// fields. `targets` is the live `[[notify_target]]` list (plan 2),
/// rendered as a `<select>` so an invalid target is unreachable through
/// this form — the write path (`write_proactive_section`) still checks
/// independently, per this sub-project's defense-in-depth precedent.
#[component]
fn ProactiveConfigCard(config: ProactiveConfigView, targets: Vec<NotifyTargetConfigView>) -> Element {
    let ws = use_context::<Sender>();
    let mut settings = use_context::<Signal<SettingsState>>();
    let mut enabled = use_signal(|| config.enabled);
    let mut target = use_signal(|| config.target.clone().unwrap_or_default());
    let mut max_per_window = use_signal(|| config.max_per_window.to_string());
    let mut window_secs = use_signal(|| config.window_secs.to_string());

    rsx! {
        div { class: "glass-card settings-section",
            div { class: "panel-head", h3 { "Proactive surfacing" } span { class: "chip", if config.enabled { "on" } else { "off" } } }
            p { class: "label-tech",
                "The assistant reaching out unprompted (e.g. a due reminder). Off unless enabled \
                 and a target is picked. Hard-capped by max sends per window. Takes effect on \
                 the next daemon restart."
            }
            div { class: "field-row",
                label { "Enabled" }
                input { r#type: "checkbox", checked: enabled(), onchange: move |e| enabled.set(e.checked()) }
            }
            div { class: "field-row",
                label { "Target" }
                select {
                    class: "input",
                    value: "{target}",
                    onchange: move |e| target.set(e.value()),
                    option { value: "", "— choose a notify target —" }
                    for t in targets.iter() {
                        option { value: "{t.name}", "{t.name}" }
                    }
                }
            }
            div { class: "field-row",
                label { "Max sends per window" }
                input { class: "input", value: "{max_per_window}", oninput: move |e| max_per_window.set(e.value()) }
            }
            div { class: "field-row",
                label { "Window (seconds)" }
                input { class: "input", value: "{window_secs}", oninput: move |e| window_secs.set(e.value()) }
            }
            button {
                class: "btn btn-primary btn-xs",
                onclick: move |_| {
                    let t = target();
                    let mpw_raw = max_per_window().trim().to_string();
                    let secs_raw = window_secs().trim().to_string();
                    // Mirrors EmailAdapterCard's port-parsing guard
                    // (plan 2 final-review finding #4): a non-empty
                    // field that fails to parse must block Save, not be
                    // silently treated as "leave unchanged."
                    let mpw = if mpw_raw.is_empty() {
                        None
                    } else {
                        match mpw_raw.parse::<u32>() {
                            Ok(n) => Some(n),
                            Err(_) => {
                                settings.write().notice =
                                    Some((false, format!("{mpw_raw:?} is not a valid whole number — Save was not sent.")));
                                return;
                            }
                        }
                    };
                    let ws_secs = if secs_raw.is_empty() {
                        None
                    } else {
                        match secs_raw.parse::<u64>() {
                            Ok(n) => Some(n),
                            Err(_) => {
                                settings.write().notice = Some((
                                    false,
                                    format!("{secs_raw:?} is not a valid whole number of seconds — Save was not sent."),
                                ));
                                return;
                            }
                        }
                    };
                    ws.send(set_proactive_config_query(
                        Some(enabled()),
                        if t.trim().is_empty() { None } else { Some(t.trim().to_string()) },
                        mpw,
                        ws_secs,
                    ));
                },
                "Save"
            }
        }
    }
}
```

- [ ] **Step 5: Extend `SettingsPanel`**

In `fn SettingsPanel() -> Element { ... }` (search for it):

5a. Grab the notifications context (for the proactive-target picker) right after `let settings = use_context::<Signal<SettingsState>>();`:

```rust
    // POLISH_WAVES.md sub-project 7 plan 3 — the notify-target list
    // (plan 2) feeds the "Proactive surfacing" target picker below.
    let notifications = use_context::<Signal<NotificationsState>>();
```

5b. Extend the top `use_future` to also load the 4 new sections' data — change:

```rust
    // Load the current settings when the view opens.
    use_future(move || async move {
        ws.send(get_settings_query());
    });
```

to:

```rust
    // Load the current settings when the view opens.
    use_future(move || async move {
        ws.send(get_settings_query());
        ws.send(memory_profile_config_query());
        ws.send(embedding_config_query());
        ws.send(proactive_config_query());
        ws.send(notify_target_configs_query());
    });
```

5c. Add the 3 new cards right before the closing `}` of the outer `div { class: "settings", ... }` — i.e. right after the "Model" `glass-card settings-section` block's closing `}` and before the confirm-modal `if confirm_open() { ... }` block (search for `code { "aivyx init" } }` to find the exact spot, right after its enclosing `}`):

```rust
            if let Some(cfg) = settings().memory_profile.clone() {
                MemoryProfileCard { key: "{cfg:?}", config: cfg }
            }
            if let Some(cfg) = settings().embedding.clone() {
                EmbeddingConfigCard { key: "{cfg:?}", config: cfg }
            }
            if let Some(cfg) = settings().proactive.clone() {
                ProactiveConfigCard { key: "{cfg:?}", config: cfg, targets: notifications().configs.clone() }
            }
```

- [ ] **Step 6: Wire the `read_task` response handlers**

In `async fn read_task(...)`, find `DaemonEnvelope::QueryResponse { payload: QueryResponsePayload::SettingsApplied { settings: snap, restart_required }, .. } => { ... }` (search for `QueryResponsePayload::SettingsApplied`) and add these 6 new arms right after its closing `}`:

```rust
                // POLISH_WAVES.md sub-project 7 plan 3 — the 3
                // Settings-coverage sections. Get* (on screen mount)
                // seeds the panel silently; *ConfigApplied (after a
                // Save) also sets the restart-required notice, matching
                // the channel-adapter cards' own convention.
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetMemoryProfileConfig { config },
                    ..
                } => {
                    settings.write().memory_profile = Some(config);
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::MemoryProfileConfigApplied { config, .. },
                    ..
                } => {
                    let mut s = settings.write();
                    s.memory_profile = Some(config);
                    s.restart_required = true;
                    s.notice = Some((true, "Saved — restart the daemon to apply.".to_string()));
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetEmbeddingConfig { config },
                    ..
                } => {
                    settings.write().embedding = Some(config);
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::EmbeddingConfigApplied { config, .. },
                    ..
                } => {
                    let mut s = settings.write();
                    s.embedding = Some(config);
                    s.restart_required = true;
                    s.notice = Some((true, "Saved — restart the daemon to apply.".to_string()));
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetProactiveConfig { config },
                    ..
                } => {
                    settings.write().proactive = Some(config);
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::ProactiveConfigApplied { config, .. },
                    ..
                } => {
                    let mut s = settings.write();
                    s.proactive = Some(config);
                    s.restart_required = true;
                    s.notice = Some((true, "Saved — restart the daemon to apply.".to_string()));
                }
```

No new error-routing arm is needed: every query builder in Step 3 uses the `"mc-settings"` id prefix, which the existing `id.starts_with("mc-settings")` arm (search for it to confirm it's still there, unchanged) already routes to `settings.write().notice`.

- [ ] **Step 7: Compile-check (wasm target)**

Run:

```bash
PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH" \
  cargo build -p aivyx-web --target wasm32-unknown-unknown
```

Expected: builds clean, no errors.

- [ ] **Step 8: Run clippy (wasm target)**

Run:

```bash
PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH" \
  cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings
```

Expected: no warnings.

- [ ] **Step 9: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "feat(web): Settings coverage for [memory] profile / [embedding] / [proactive]

3 new SettingsPanel sections: a 3-way memory-profile picker,
base_url/model/api_key for embedding (masked secret UX matching
EmailAdapterCard), and enabled/target/max_per_window/window_secs for
proactive (target picker sourced from plan 2's notify-target list,
non-numeric input blocked client-side rather than silently dropped).
SettingsState gains 3 new Option fields; no new UI-state struct needed.

POLISH_WAVES.md sub-project 7 plan 3, Task 6.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 7: Full workspace sweep + `dist/` rebuild

**Files:**
- None created; verification + a `dist/` rebuild only.

**Interfaces:**
- Consumes: everything from Tasks 1-6.
- Produces: nothing — this is the plan's final task, gating the whole-branch review.

- [ ] **Step 1: Full clippy sweep**

Run: `cargo clippy --workspace --exclude aivyx-desktop --all-targets -- -D warnings`
Expected: zero warnings. If `aivyx-desktop` cannot be excluded this way in this checkout (verify the flag is accepted — `aivyx-desktop`'s own exclusion from `default-members` in `Cargo.toml` normally makes a plain `cargo clippy --workspace` attempt it anyway, same as `cargo test --workspace` did during this sub-project's earlier branch-finishing steps and failed on missing system `webkit2gtk` libs), fall back to `cargo clippy --all-targets -- -D warnings` (no `--workspace`, touches only `default-members`) plus an explicit `cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings` (already run per-task, but re-run once more here as the final gate).

- [ ] **Step 2: Full test sweep**

Run: `cargo test --workspace --exclude aivyx-desktop`
Expected: zero failures. Same fallback as Step 1 if the exclusion flag doesn't apply cleanly in this checkout: `cargo test` with no `-p`/`--workspace` (touches only `default-members`, which already excludes `aivyx-web` and `aivyx-desktop`).

- [ ] **Step 3: Rebuild `dist/`**

```bash
cd crates/aivyx-web
PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH" \
  dx bundle --release --platform web
rm -rf dist && mkdir -p dist
cp -r target/dx/aivyx-web/release/web/public/. dist/
find dist -name '*.br' -delete
cd ../..
```

- [ ] **Step 4: Verify the `dist/` diff is a clean rename**

Run: `git status --porcelain dist/assets/ 2>/dev/null || git -C crates/aivyx-web status --porcelain dist/assets/`
Expected: exactly one added `.wasm` file and one deleted `.wasm` file (a rename by content hash), plus the usual `.js`/`index.html` churn — no unexpected new/missing files.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "chore: workspace sweep + dist/ rebuild for sub-project 7 plan 3

cargo clippy/test clean across the workspace (aivyx-desktop excluded —
missing system webkit2gtk libs in this environment, pre-existing and
unrelated to this branch). dist/ rebuilt to serve the reflection-
schedule and Settings-coverage UI from this branch's HEAD.

POLISH_WAVES.md sub-project 7 plan 3, Task 7.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## After all tasks: whole-branch review

Dispatch the final whole-branch code review on the most capable available model (per subagent-driven-development's model-selection guidance and this sub-project's own established pattern — every prior final review in this sub-project found real, previously-invisible issues). Point it at:

- The design spec: `docs/superpowers/specs/2026-09-02-settings-coverage-expansion-design.md`.
- This plan file.
- A `scripts/review-package MERGE_BASE HEAD` diff package (`MERGE_BASE = git merge-base main HEAD`).

Specifically ask the reviewer to verify (beyond its own general sweep):

1. `write_proactive_section`'s target-existence check and the 3 numeric/enabled validations all evaluate the **merged** state, not just this call's own fields — replay plan 2's own two-round email-validation gap-hunt method (a merged-state bug is exactly the failure class that took 2 extra review rounds to fully close there).
2. `write_reflection_schedule_section`'s cross-array uniqueness check actually reads `[[schedule]]`, not just `[[reflection_schedule]]` — and that a disabled `[[reflection_schedule]]` entry really does survive `read_reflection_schedule_entries` (the resolving loader's silent-drop behavior is the one novel risk this plan introduces that plans 1-2 didn't have an analog for).
3. Every `ReflectionScheduleForm`/`MemoryProfileCard`/`EmbeddingConfigCard`/`ProactiveConfigCard` call site has a `key:` where it holds seed-dependent `use_signal` state, and that none of them can show stale data after a successful Save (the exact bug class plan 1 shipped once and plan 2 designed around from the start).
4. `dist/` in the final commit is actually fresh relative to `main.rs`'s HEAD (this sub-project's own recorded process gap — verified by checking the compiled `.wasm` for a string unique to this plan's new UI, e.g. `"Reflection schedules"` or `"Proactive surfacing"`, not just by trusting the rebuild step ran).

After the review (and any fix wave + re-review it triggers, following the same loop this sub-project has needed for every plan so far), invoke `superpowers:finishing-a-development-branch` for the feature branch, then update `docs/POLISH_WAVES.md` and `aivyx-ecosystem/ROADMAP.md` to record plan 3 (and therefore all of sub-project 7) as shipped, following the exact structure of the plan 1/plan 2 entries already there.

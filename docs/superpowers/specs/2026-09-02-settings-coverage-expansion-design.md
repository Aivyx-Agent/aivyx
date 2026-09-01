# Settings coverage expansion (POLISH_WAVES.md sub-project 7, plan 3) — design

**Status:** Approved, ready for planning.

## Motivation

The last unbuilt piece of sub-project 7 ("credentials in Studio"). The
original sub-project spec (`docs/superpowers/specs/2026-08-31-config-
write-surface-design.md`, its "## D. Settings coverage expansion"
section) scoped this at a high level and deferred the detail to plan
time. This document is that detail, re-grounded against the current
code (not the earlier spec's prose) after plans 1 and 2 shipped.

Four config surfaces, currently read-only or entirely absent from
Studio:

- **`[memory] profile`** — no Studio presence at all today.
  `SettingsSnapshot` (`crates/aivyx-ipc/src/protocol.rs:1498`) has no
  `profile` field, only a derived `embeddings_available: bool` — a
  genuine gap, not a UI-only miss.
- **`[embedding]`** — same: no Studio presence.
- **`[proactive]`** — same: no Studio presence.
- **`[[reflection_schedule]]`** — same: no Studio presence, not even
  read-only. The daemon consumes it (`daemon_server.rs`'s
  `reflection_schedules: Vec<ReflectionScheduleConfig>`, wired into
  `run_reflection_scheduler`), but nothing in `main.rs` lists, creates,
  edits, or deletes entries. An operator today can only manage these
  by hand-editing `aivyx.toml` and restarting the daemon.

**Out of scope**, unchanged from the sub-project spec: MCP tool-level
health (sub-project 8); anything already covered by plans 1-2 (MCP
CRUD, notify-target CRUD, the 4 channel-adapter sections).

## Corrections found while grounding this plan against current code

Three places where the original spec's shorthand doesn't quite match
what the code does, discovered by reading the real structs and
validation functions rather than trusting the earlier prose:

1. **`[memory] profile` is a 3-way switch, not 2-way.**
   `MemoryProfile` (`crates/aivyx-config/src/lib.rs:2706`) has three
   variants — `Off` (default, byte-identical to pre-Chapter-Synapse
   behavior), `Lite` (recall fusion over existing data, no paid
   generation), `Smart` (adds the wiki/graph extraction sweeps). The
   Studio picker exposes all three, not a lite/smart toggle.

2. **The loader does not cross-reference-validate `[proactive]
   target`.** `build_proactive_config`
   (`crates/aivyx-config/src/lib.rs:8689`) only checks `target` is
   non-empty when `enabled = true`; it never checks the name exists in
   `[[notify_target]]`. An unknown target loads fine and fails silently
   at dispatch time (`reflection_scheduler.rs:1525`'s
   `deps.notify.dispatch(...)`, whose `Err` branch is only an
   `eprintln!`). The Studio field will be a **picker** populated from
   the real `[[notify_target]]` list, which makes an invalid selection
   unreachable through the primary Studio path — but per this
   sub-project's established defense-in-depth precedent (plan 2 added
   loader-independent checks like the `[email]`-presence gate and the
   retry/rate-limit bounds), `write_proactive_section` itself will
   still reject a `target` that doesn't name an existing
   `[[notify_target]]` entry when `enabled = true`. This is
   deliberately **stricter** than today's loader, not a mirror of an
   existing check — call this out plainly in the task brief so an
   implementer doesn't go looking for a loader-side check to copy that
   doesn't exist.

3. **`[[reflection_schedule]]` name-uniqueness spans two arrays, and
   disabled entries vanish from the loaded list.** The loader
   (`crates/aivyx-config/src/lib.rs:7251`-7301) rejects a name that
   collides with either another `[[reflection_schedule]]` entry *or* a
   regular `[[schedule]]` entry (shared namespace, "keep the operator
   mental model single-namespace"). It also `continue`s past any entry
   with `enabled = false` — such an entry is skipped from validation
   and **entirely absent** from the loaded `reflection_schedules: Vec`,
   not merely marked inactive. Consequences for the write path:
   - `read_reflection_schedule_entries` (the Studio list-view read)
     must use raw `toml_edit` parsing, never
     `AivyxConfig::load_from_env_and_toml`/`load_settings_config` —
     already the standing rule for every write-adjacent read in this
     sub-project, but worth restating since this is the first place
     the resolving loader would silently drop rows rather than leak a
     secret (the failure mode plans 1-2 were guarding against).
   - `write_reflection_schedule_section`'s uniqueness check must read
     **both** `doc["reflection_schedule"]` and `doc["schedule"]` as
     arrays-of-tables and check the candidate name against both,
     mirroring the loader's two-array check.

## A. `[memory] profile`

`[memory]` is a shared table with fields this plan does not touch
(`max_per_topic`, `ttl_secs`, the `[[memory.retention]]` array-of-tables,
`canonicalize_topics`) — the write function touches only the `profile`
key, leaving every sibling key and the retention sub-array
byte-identical (`doc["memory"]["profile"] = value(...)` on an
already-table `[memory]` only ever adds/updates that one key;
`toml_edit` does not disturb siblings).

```rust
// crates/aivyx-config/src/config_write.rs

/// `[memory] profile` — `off` (default) / `lite` / `smart`. `None`
/// leaves the on-disk value untouched (plan 2's leave-on-`None`
/// convention — see the sub-project's Global Constraints).
pub fn write_memory_profile(
    path: &Path,
    profile: Option<&str>,
) -> Result<(), ConfigWriteError> {
    let Some(p) = profile else { return Ok(()) };
    let normalized = match p.trim().to_lowercase().as_str() {
        "off" | "lite" | "smart" => p.trim().to_lowercase(),
        other => {
            return Err(ConfigWriteError::InvalidMemoryProfile(format!(
                "`{other}` is not a valid profile — must be `off`, `lite`, or `smart`"
            )))
        }
    };
    let mut doc = load_document(path)?;
    doc["memory"]["profile"] = value(normalized);
    write_toml_0600(path, &doc.to_string())
}

/// Raw read of `[memory] profile`, for the Studio picker's seed value.
/// Raw `toml_edit` parse — this one field carries no secret, but stays
/// consistent with the write-adjacent-read rule so a future field added
/// to this function under the same name doesn't have to change the
/// convention.
pub fn read_memory_profile(path: &Path) -> Result<Option<String>, ConfigWriteError> {
    let doc = load_document(path)?;
    Ok(doc
        .get("memory")
        .and_then(|m| m.as_table_like())
        .and_then(|t| t.get("profile"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string()))
}
```

**Wire types** (`crates/aivyx-ipc/src/protocol.rs`):

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryProfileConfigView {
    /// `"off" | "lite" | "smart"`.
    pub profile: String,
}
```

`QueryPayload::GetMemoryProfileConfig`,
`QueryPayload::SetMemoryProfile { profile: String }` (always sent —
the picker has no blank state, so unlike the other three sections this
one is never `Option` on the wire; the write function's own `Option`
parameter exists only so the same primitive could serve a future
partial caller, not because Studio ever omits it).
`QueryResponsePayload::MemoryProfileConfig(MemoryProfileConfigView)`,
`QueryResponsePayload::MemoryProfileConfigApplied`.

## B. `[embedding]`

Primary fields only: `base_url`, `model`, `api_key` (secret). Every
other field on `EmbeddingConfig` (`dimensions`, `rag_top_k`,
`rag_min_similarity`, `recall_window_turns`, `recall_gate_min_chars`,
`ann_index`, `ann_rebuild_threshold`, `recall_token_budget`,
`recall_hybrid`, and the Chapter Loom recall-fusion tuning fields
`recall_lexical_weight`/`recall_graph_hops`/`recall_graph_decay`/
`recall_graph_weight`/`recall_wiki_weight`/`recall_graph_typed_weight`)
stays TOML-only, confirmed by reading `EmbeddingConfig`'s full field
list (`crates/aivyx-config/src/lib.rs:2156`) end to end — the earlier
spec's "stays TOML-only" list only named the first five; the Loom
tuning fields are additional, not a contradiction, just an earlier
incomplete enumeration.

Follows the **leave-untouched-on-`None`** convention uniformly across
all three fields (including the two non-secret ones), matching plan
2's channel-adapter sections rather than the older
`write_profile_section`'s clear-on-`None` convention — this section
mixes a secret with plain fields, and the leave-on-`None` convention is
the more recently hardened precedent for anything secret-adjacent (see
Global Constraints).

```rust
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmbeddingEntryWrite {
    pub base_url: Option<String>,
    pub model: Option<String>,
    /// `Some(new value)` writes it; `None` leaves the existing TOML
    /// value untouched (never clears a working key).
    pub api_key: Option<String>,
}

pub fn write_embedding_section(
    path: &Path,
    write: &EmbeddingEntryWrite,
) -> Result<(), ConfigWriteError> { /* ... */ }

pub fn read_embedding_section(
    path: &Path,
) -> Result<Option<EmbeddingEntryRead>, ConfigWriteError> { /* ... */ }
```

`EmbeddingEntryRead` mirrors the raw-read shape used for the plan-2
channel adapters: plain `base_url`/`model`, plus whether `api_key` is
currently set (never the value itself) — this is what the view-builder
turns into a `RedactedSecret`.

**Wire types:**

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingConfigView {
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key: RedactedSecret,
}
```

`QueryPayload::GetEmbeddingConfig`,
`QueryPayload::SetEmbeddingConfig { base_url: Option<String>, model: Option<String>, api_key: Option<String> }`,
`QueryResponsePayload::EmbeddingConfig(EmbeddingConfigView)`,
`QueryResponsePayload::EmbeddingConfigApplied`.

**Validation in `write_embedding_section`:** none beyond what the
loader itself enforces today for these three fields — `base_url` and
`model`, if present, must be non-empty after trim (the loader defaults
both when absent, `DEFAULT_EMBEDDING_BASE_URL`/`DEFAULT_EMBEDDING_MODEL`;
an explicit-but-blank value is the one case the write path should
reject client-side and server-side, mirroring how the email/telegram
cards already refuse blank-but-present required fields).

## C. `[proactive]`

Primary fields: `enabled`, `target` (picker, populated from
`GetNotifyTargetConfigs` — already shipped in plan 2, the direct
synergy this design was sequenced after), `max_per_window`,
`window_secs`. `signals` (the three `signal_ttl_expiry`/
`signal_recall_cluster`/`signal_due_reminder` toggles) stays TOML-only.

Same leave-untouched-on-`None` convention as `[embedding]`.

```rust
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProactiveEntryWrite {
    pub enabled: Option<bool>,
    pub target: Option<String>,
    pub max_per_window: Option<u32>,
    pub window_secs: Option<u64>,
}

pub fn write_proactive_section(
    path: &Path,
    write: &ProactiveEntryWrite,
) -> Result<(), ConfigWriteError> { /* ... */ }
```

**Validation in `write_proactive_section`** (evaluated against the
*merged* post-write state — the same "merged, not per-field" posture
plan 2's email fix wave established, since a `target` written now can
combine with an `enabled = true` already on disk from an earlier
write):

- When merged `enabled = true`:
  - `target` must be non-empty after trim.
  - `target` must name an existing `[[notify_target]]` entry — read via
    `read_notify_target_entries` (already shipped, plan 2). This is the
    write-path-only check described above (§ Corrections, item 2); no
    loader-side equivalent exists to mirror.
  - `max_per_window` (merged, default `DEFAULT_PROACTIVE_MAX_PER_WINDOW`
    if never set) must be `>= 1`, mirroring the loader.
  - `window_secs` (merged, default `DEFAULT_PROACTIVE_WINDOW_SECS` if
    never set) must be `>= 1`, mirroring the loader.
- New `ConfigWriteError::InvalidProactiveConfig(String)`.

**Wire types:**

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProactiveConfigView {
    pub enabled: bool,
    pub target: Option<String>,
    pub max_per_window: u32,
    pub window_secs: u64,
}
```

`QueryPayload::GetProactiveConfig`,
`QueryPayload::SetProactiveConfig { enabled: Option<bool>, target: Option<String>, max_per_window: Option<u32>, window_secs: Option<u64> }`,
`QueryResponsePayload::ProactiveConfig(ProactiveConfigView)`,
`QueryResponsePayload::ProactiveConfigApplied`.

## D. `[[reflection_schedule]]`

CRUD via the array-of-table primitive from plans 1-2 (upsert-by-`name`,
`KNOWN_KEYS` unknown-key preservation on replace — `role_override`/
`skip_when_idle`/`min_audit_entries_to_fire` are the fields this plan
doesn't expose but must not silently destroy on an edit, exactly the
class of bug plan 1's `[mcp_server.sandbox]` incident was). Primary
fields: `name`, `cron`, `lookback_window_secs`, `enabled`.
`role_override`/`skip_when_idle`/`min_audit_entries_to_fire` stay
TOML-only.

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct ReflectionScheduleEntryWrite {
    pub name: String,
    pub cron: String,
    pub lookback_window_secs: u64,
    pub enabled: bool,
}

pub fn write_reflection_schedule_section(
    path: &Path,
    write: &ReflectionScheduleEntryWrite,
) -> Result<(), ConfigWriteError> { /* upsert by name */ }

pub fn remove_reflection_schedule_section(
    path: &Path,
    name: &str,
) -> Result<(), ConfigWriteError> { /* ... */ }

/// Raw read — ALL entries including `enabled = false` ones (the
/// resolving loader drops disabled entries entirely; the write UI's
/// list view must not).
pub fn read_reflection_schedule_entries(
    path: &Path,
) -> Result<Vec<ReflectionScheduleEntryWrite>, ConfigWriteError> { /* ... */ }
```

**Validation in `write_reflection_schedule_section`**, mirroring the
loader (`crates/aivyx-config/src/lib.rs:7251`-7301) field for field:

- `name` non-empty after trim.
- `cron` non-empty after trim. (Not syntax-validated against the cron
  parser — the loader itself doesn't syntax-check `cron` at this layer
  either, matching `SchedulesPanel`'s existing regular-schedule form,
  which also only checks non-empty and leaves cron syntax errors to
  surface at scheduler-registration time.)
- `lookback_window_secs` in `[MIN_REFLECTION_LOOKBACK_SECS,
  MAX_REFLECTION_LOOKBACK_SECS]` (60s – 30 days).
- Name uniqueness against **both** `[[reflection_schedule]]` (excluding
  the entry being replaced, same same-entry-name exclusion pattern as
  plan 2's at-most-one-default check) and `[[schedule]]` — read both
  arrays via raw `toml_edit`.
- New `ConfigWriteError::InvalidReflectionSchedule(String)`.

Note: `role_override` is not exposed for writing, so the write path
never needs to validate it — an existing entry's `role_override` is
preserved untouched by the `KNOWN_KEYS` mechanism regardless of what
this plan's form does.

**Wire types:**

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReflectionScheduleConfigView {
    pub name: String,
    pub cron: String,
    pub lookback_window_secs: u64,
    pub enabled: bool,
}
```

`QueryPayload::GetReflectionScheduleConfigs`,
`QueryPayload::SetReflectionSchedule(ReflectionScheduleConfigView)`,
`QueryPayload::DeleteReflectionSchedule { name: String }`,
`QueryResponsePayload::ReflectionScheduleConfigs(Vec<ReflectionScheduleConfigView>)`,
`QueryResponsePayload::ReflectionScheduleConfigApplied`.

Watch for the same `#[serde(tag = "kind")]` collision plan 2's Task 3
hit: none of these struct-variant fields may be literally named `kind`.
None of the above are, but flag it in the task brief anyway since it's
bitten this exact pattern once already.

## UI

- **`SettingsPanel`** (`crates/aivyx-web/src/main.rs`) gains three new
  `glass-card settings-section` blocks, following the existing
  access/budget/autonomy sections' layout: a "Memory profile" block (3
  radio/select options), an "Embedding" block (base_url/model/api_key
  fields, masked "configured"/"not set" secret UX matching plan 2's
  channel-adapter cards), a "Proactive surfacing" block (enabled
  toggle, target `<select>` populated from the notify-target list,
  max_per_window/window_secs number fields).
- **`SchedulesPanel`** gains a new "Reflection schedules" section below
  the existing regular-schedule list and create form, with its own
  create/edit/delete form (name, cron — reusing the same
  frequency/time/day cron-builder UX the regular-schedule form already
  has, since operators shouldn't need two different cron mental models
  in the same panel — lookback_window_secs, enabled).
- Settings' new-section notices reuse the existing `SettingsState.notice:
  Option<(bool, String)>` (`crates/aivyx-web/src/main.rs:341`) — there is
  no separate `SettingsUi` struct; `SettingsState` already carries the
  snapshot + last write outcome + `restart_required` in one place, and
  the three new sections follow that same struct rather than adding a
  parallel one. Reflection-schedule notices reuse the existing
  `SchedulesUi.notice: Option<(bool, String)>`
  (`crates/aivyx-web/src/main.rs:553`) — already shared with the
  regular-schedule mutations, confirmed present, not assumed.
- Every new/edited form gets a `key:` on its call site if it holds
  `use_signal` state that must reset between "editing X" and "adding
  new" — the exact bug plan 1 shipped once (`McpServerForm`) and plan 2
  designed around from the start (`NotifyTargetForm`, `EmailAdapterCard`).

## Global Constraints

- Read functions for anything on this list use raw `toml_edit` parsing
  only (`load_document`), never `AivyxConfig::load_from_env_and_toml`/
  `load_settings_config` — binding across all of sub-project 7, and the
  specific failure mode item D's disabled-reflection-schedule quirk
  makes newly relevant (§ Corrections, item 3).
- Secret fields (`[embedding] api_key`) use `RedactedSecret` on the
  read side; a `SetX` write's secret field is `Option<String>` where
  `None` means leave the existing TOML value untouched — the
  established convention from plans 1-2.
- `[embedding]`/`[proactive]`/`[memory] profile` writes use the
  leave-untouched-on-`None` convention for **every** field, not just
  secrets — plan 2's convention, not `write_profile_section`'s older
  clear-on-`None` convention.
- `[[reflection_schedule]]` writes use the array-of-table
  upsert-by-name + `KNOWN_KEYS` unknown-key-preservation pattern from
  plans 1-2, defending `role_override`/`skip_when_idle`/
  `min_audit_entries_to_fire` from being silently dropped on edit.
- Every new `ConfigWriteError` variant needs a matching arm in
  `map_config_write_error` (`daemon_server.rs`) — its match is
  exhaustive; a hard compile error otherwise, not an optional nicety.
- Validate against the *merged* post-write state, not the
  just-written fields in isolation — the posture plan 2's two fix
  waves converged on after finding gaps in field-at-a-time validation
  twice.
- After any fix wave touching this validation surface, dispatch a
  genuine independent re-review before considering the fix wave done —
  every fix wave in this sub-project so far (plan 1's sandbox
  preservation, plan 2's email validation) has needed at least one more
  round.

## Testing

- Round-trip tests per new section: write → re-read (raw) → assert the
  written fields match and untouched sibling keys/comments survive
  byte-identical. For `[[reflection_schedule]]`, both the
  upsert-new-entry and replace-existing-entry paths, plus a disabled
  entry surviving a round-trip through `read_reflection_schedule_
  entries` (the case the resolving loader would silently drop).
- `[proactive] target` referencing a nonexistent notify-target name
  surfaces a real `InvalidProactiveConfig` error, both when `target` is
  written together with `enabled = true` in the same call and when
  `enabled = true` is already on disk and only `target` is written.
- `[[reflection_schedule]]` name colliding with an existing
  `[[schedule]]` entry (and vice versa is out of scope — this plan
  doesn't touch `[[schedule]]` writes) surfaces a real
  `InvalidReflectionSchedule` error.
- A secret round-trip test for `[embedding] api_key`: a `GetEmbeddingConfig`
  response never contains a real key substring for a config seeded with
  one; a `SetEmbeddingConfig` write with `api_key: None` leaves the
  existing TOML value unchanged (read the file after write, assert the
  old key string is still present verbatim) — the same test shape
  plans 1-2 used for every secret field.
- Full sweep before merge: `cargo clippy --workspace --exclude
  aivyx-desktop --all-targets -- -D warnings` and `cargo test
  --workspace --exclude aivyx-desktop`, zero warnings/failures, plus a
  `dist/` rebuild in the final task (established convention, and the
  one place this sub-project has already been bitten by staleness
  once).

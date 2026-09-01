# Notify-target + channel-adapter CRUD Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship plan 2 of POLISH_WAVES.md sub-project 7 (item C): `[[notify_target]]` CRUD, the shared `[email]` SMTP block, and the `[telegram]`/`[discord]`/`[slack]` channel-adapter bot-token sections — all editable from Studio — per `docs/superpowers/specs/2026-08-31-config-write-surface-design.md`.

**Architecture:** Reuse plan 1's now-proven, now-fixed patterns exactly: `[[notify_target]]` gets the same array-of-table upsert-by-name primitive as `[[mcp_server]]` (`write_mcp_server_section`/`read_mcp_server_entries`, `crates/aivyx-config/src/config_write.rs`), including its hard-won lessons (raw-TOML reads that never resolve `${VAR}`/interpolation, unknown-key preservation on upsert). `[email]`/`[telegram]`/`[discord]`/`[slack]` are singleton sections, so they use a simpler **partial-update** pattern: every write field is `Option<T>` where `None` means "leave this key untouched on disk" — this is where the plan's real secrets live (SMTP password, bot tokens), so every secret field's read side uses `RedactedSecret` (shipped in plan 1, unused until now) and never sends a real value. Studio's `NotificationsPanel` (Chapter Herald, currently read-only) gets a notify-target CRUD form plus a new "Channel adapters" section for the four singleton forms.

**Tech Stack:** `toml_edit` 0.22 (same crate, same idioms as plan 1), the existing `aivyx-ipc`/`aivyx-channel` wire-protocol machinery, Dioxus 0.6 for the Studio forms.

## Global Constraints

- Zero clippy warnings: `cargo clippy --workspace --exclude aivyx-desktop --all-targets -- -D warnings` must stay clean after every task.
- `cargo test --workspace --exclude aivyx-desktop` must stay green.
- `aivyx-web`'s own `#[cfg(test)]` tests run via plain `cargo test -p aivyx-web` — **no** `--target wasm32-unknown-unknown` flag (verified in plan 1: that flag produces an unexecutable `.wasm` test binary on this host). Only `cargo build`/`cargo clippy` for `aivyx-web`/`aivyx-ipc` need `--target wasm32-unknown-unknown` (use `~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin` + `~/.cargo/bin` on `PATH` if the system `cargo` lacks the target).
- **Secret-field convention (binding, learned the hard way in plan 1):** a read-side response for `[email].password`, `[telegram].token`, `[discord].token`, `[slack].bot_token`/`.app_token` must NEVER send the real value — use `RedactedSecret { configured: bool, source: String }` (already shipped, `crates/aivyx-ipc/src/protocol.rs`). Every read in this plan uses **raw TOML parsing only** (`load_document`, never `AivyxConfig::load_from_env_and_toml`/`load_settings_config`) — the interpolating loader must never be on the path a `GetX` response is built from, for exactly the reason plan 1's Critical finding #1 discovered (a resolved secret gets echoed back and baked into plaintext on the next save). Since these reads never resolve anything (not `${VAR}`, not env-var fallbacks), every `RedactedSecret.source` in this plan is the literal string `"toml"` — there is no other source this read path can ever observe.
- **Partial-update convention (singleton sections only):** every field on `EmailEntryWrite`/`TelegramEntryWrite`/`DiscordEntryWrite`/`SlackEntryWrite` is `Option<T>`; a write function only touches (writes) the TOML keys whose value is `Some`, leaving every `None` field's on-disk value exactly as it was. This applies uniformly to every field, not just secrets — the Studio form always resends the current value for a field it isn't changing.
- `[[notify_target]]` upsert must preserve every key not in its own known-field list on an existing entry, exactly like `write_mcp_server_section` already does for `[[mcp_server]]` — defensive against future schema growth, not because a known unexposed field exists on this section today (confirm this during Task 1 by reading `RawNotifyTarget`'s full field list; if it turns out to carry no extra fields beyond what this plan exposes, the preservation loop is still correct to include, it just never has anything to preserve today).
- Every config write in this plan requires a daemon restart to take effect (matches plan 1's stated posture for every `config_write.rs` consumer) — every write response carries `restart_required: true` and the Studio UI shows the existing `.restart-banner`/notice treatment.
- After the final task, rebuild and commit `dist/` per this repo's established convention: `rm -rf dist && cp -r <dx-bundle-output> dist` (never a merging copy), strip `.br` files, verify via `git status --porcelain dist/assets/ | grep wasm` showing exactly one add + one delete.

---

## Task 1: `[[notify_target]]` array-of-table primitive

**Files:**
- Modify: `crates/aivyx-config/src/config_write.rs`

**Interfaces:**
- Produces: `pub struct NotifyTargetEntryWrite { pub name: String, pub kind: String, pub chat_id: Option<String>, pub url: Option<String>, pub to: Option<String>, pub enabled: bool, pub is_default: bool, pub retry_count: u32, pub retry_backoff_ms_start: u64, pub rate_limit_max: Option<u32>, pub rate_limit_window_secs: Option<u64> }`; `pub fn write_notify_target_section(path: &Path, target: &NotifyTargetEntryWrite) -> Result<(), ConfigWriteError>` (upsert-by-`name`); `pub fn remove_notify_target_section(path: &Path, name: &str) -> Result<(), ConfigWriteError>`; `pub fn read_notify_target_entries(path: &Path) -> Result<Vec<NotifyTargetEntryWrite>, ConfigWriteError>`; a new `ConfigWriteError::InvalidNotifyTarget { reason: String }` variant.

- [ ] **Step 1: Write the failing tests**

Add to `crates/aivyx-config/src/config_write.rs`'s existing `#[cfg(test)] mod tests` block:

```rust
    fn telegram_target(name: &str) -> NotifyTargetEntryWrite {
        NotifyTargetEntryWrite {
            name: name.to_string(),
            kind: "telegram".to_string(),
            chat_id: Some("123456".to_string()),
            url: None,
            to: None,
            enabled: true,
            is_default: false,
            retry_count: 0,
            retry_backoff_ms_start: 500,
            rate_limit_max: None,
            rate_limit_window_secs: None,
        }
    }

    #[test]
    fn notify_target_write_adds_a_new_entry() {
        let path = temp_toml("notify-add");
        std::fs::write(&path, "[access]\nlevel = \"sandbox\"\n").unwrap();
        write_notify_target_section(&path, &telegram_target("ops")).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("[access]"), "unrelated section survives");
        assert!(contents.contains("[[notify_target]]"));
        assert!(contents.contains("name = \"ops\""));
        assert!(contents.contains("chat_id = \"123456\""));
    }

    #[test]
    fn notify_target_write_replaces_an_existing_entry_by_name() {
        let path = temp_toml("notify-replace");
        std::fs::write(&path, "").unwrap();
        write_notify_target_section(&path, &telegram_target("ops")).unwrap();
        let mut updated = telegram_target("ops");
        updated.chat_id = Some("999".to_string());
        write_notify_target_section(&path, &updated).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.matches("name = \"ops\"").count(), 1);
        assert!(contents.contains("chat_id = \"999\""));
        assert!(!contents.contains("chat_id = \"123456\""));
    }

    #[test]
    fn notify_target_write_rejects_telegram_without_chat_id() {
        let path = temp_toml("notify-tg-no-chat");
        std::fs::write(&path, "").unwrap();
        let mut entry = telegram_target("ops");
        entry.chat_id = None;
        let err = write_notify_target_section(&path, &entry).unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidNotifyTarget { .. }));
    }

    #[test]
    fn notify_target_write_rejects_webhook_with_bad_url_scheme() {
        let path = temp_toml("notify-webhook-bad-url");
        std::fs::write(&path, "").unwrap();
        let entry = NotifyTargetEntryWrite {
            name: "alerts".to_string(),
            kind: "webhook".to_string(),
            chat_id: None,
            url: Some("ftp://example.com".to_string()),
            to: None,
            enabled: true,
            is_default: false,
            retry_count: 0,
            retry_backoff_ms_start: 500,
            rate_limit_max: None,
            rate_limit_window_secs: None,
        };
        let err = write_notify_target_section(&path, &entry).unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidNotifyTarget { .. }));
    }

    #[test]
    fn notify_target_write_rejects_email_with_no_at_sign() {
        let path = temp_toml("notify-email-bad-to");
        std::fs::write(&path, "").unwrap();
        let entry = NotifyTargetEntryWrite {
            name: "digest".to_string(),
            kind: "email".to_string(),
            chat_id: None,
            url: None,
            to: Some("not-an-email".to_string()),
            enabled: true,
            is_default: false,
            retry_count: 0,
            retry_backoff_ms_start: 500,
            rate_limit_max: None,
            rate_limit_window_secs: None,
        };
        let err = write_notify_target_section(&path, &entry).unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidNotifyTarget { .. }));
    }

    #[test]
    fn notify_target_write_rejects_a_second_default() {
        let path = temp_toml("notify-two-defaults");
        std::fs::write(&path, "").unwrap();
        let mut first = telegram_target("ops");
        first.is_default = true;
        write_notify_target_section(&path, &first).unwrap();
        let mut second = telegram_target("backup");
        second.is_default = true;
        let err = write_notify_target_section(&path, &second).unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidNotifyTarget { .. }));
    }

    #[test]
    fn notify_target_write_allows_replacing_the_existing_default() {
        let path = temp_toml("notify-replace-default");
        std::fs::write(&path, "").unwrap();
        let mut first = telegram_target("ops");
        first.is_default = true;
        write_notify_target_section(&path, &first).unwrap();
        // Re-writing the SAME entry (still marked default) must not be
        // rejected as "a second default" — it's the same target.
        write_notify_target_section(&path, &first).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.matches("default = true").count(), 1);
    }

    #[test]
    fn notify_target_remove_drops_the_named_entry_only() {
        let path = temp_toml("notify-remove");
        std::fs::write(&path, "").unwrap();
        write_notify_target_section(&path, &telegram_target("ops")).unwrap();
        write_notify_target_section(&path, &telegram_target("backup")).unwrap();
        remove_notify_target_section(&path, "ops").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(!contents.contains("name = \"ops\""));
        assert!(contents.contains("name = \"backup\""));
    }

    #[test]
    fn read_notify_target_entries_round_trips() {
        let path = temp_toml("notify-read");
        std::fs::write(&path, "").unwrap();
        write_notify_target_section(&path, &telegram_target("ops")).unwrap();
        let entries = read_notify_target_entries(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "ops");
        assert_eq!(entries[0].chat_id.as_deref(), Some("123456"));
    }
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p aivyx-config config_write::tests::notify_target
cargo test -p aivyx-config config_write::tests::read_notify_target_entries_round_trips
```
Expected: FAIL — `write_notify_target_section`/`remove_notify_target_section`/`read_notify_target_entries`/`NotifyTargetEntryWrite`/`ConfigWriteError::InvalidNotifyTarget` not defined.

- [ ] **Step 3: Add the error variant**

In `ConfigWriteError`'s enum (alongside `InvalidMcpServer`):

```rust
    /// A `[[notify_target]]` entry is structurally invalid — mirrors the
    /// loader's own per-kind validation (`aivyx-config/src/lib.rs`'s
    /// notify_targets parsing) so a bad write is refused before it
    /// corrupts the next daemon load, same principle as `InvalidMcpServer`.
    InvalidNotifyTarget { reason: String },
```

And its `Display` arm, right after `InvalidMcpServer`'s:

```rust
            ConfigWriteError::InvalidNotifyTarget { reason } => write!(f, "invalid notify target entry: {reason}"),
```

- [ ] **Step 4: Add the primitive**

Add to `crates/aivyx-config/src/config_write.rs`, near `write_mcp_server_section`/`remove_mcp_server_section`/`read_mcp_server_entries` (same file, same neighborhood):

```rust
/// One `[[notify_target]]` entry as Studio's write form submits it. A
/// plain (non-wire) struct — `aivyx-ipc` has its own serde-derived mirror;
/// `aivyx-channel`'s daemon handler converts between them. `chat_id`/
/// `url`/`to` are mutually exclusive per `kind` (mirrors
/// `NotifyTargetKind`'s own shape) but all three are plain `Option<String>`
/// here since only one is ever populated for a given `kind`.
pub struct NotifyTargetEntryWrite {
    pub name: String,
    /// `"telegram"`, `"webhook"`, `"email"`, or `"web-ui"`.
    pub kind: String,
    pub chat_id: Option<String>,
    pub url: Option<String>,
    pub to: Option<String>,
    pub enabled: bool,
    pub is_default: bool,
    pub retry_count: u32,
    pub retry_backoff_ms_start: u64,
    pub rate_limit_max: Option<u32>,
    pub rate_limit_window_secs: Option<u64>,
}

/// Add or replace (by `name`) one `[[notify_target]]` entry, preserving
/// every other entry, section, and the operator's comments. Validates the
/// same per-kind requirements the loader does (`aivyx-config/src/lib.rs`'s
/// notify_targets parsing: telegram needs a non-empty `chat_id`, webhook
/// needs an `http(s)://` `url`, email needs an `@`-containing `to`) plus
/// the at-most-one-default rule, refused here rather than failing the
/// next daemon load.
pub fn write_notify_target_section(
    path: &Path,
    target: &NotifyTargetEntryWrite,
) -> Result<(), ConfigWriteError> {
    if target.name.trim().is_empty() {
        return Err(ConfigWriteError::InvalidNotifyTarget {
            reason: "name must not be empty".to_string(),
        });
    }
    match target.kind.as_str() {
        "telegram" => {
            if target.chat_id.as_deref().is_none_or(str::is_empty) {
                return Err(ConfigWriteError::InvalidNotifyTarget {
                    reason: format!(
                        "target {:?}: kind \"telegram\" requires a non-empty `chat_id`",
                        target.name
                    ),
                });
            }
        }
        "webhook" => {
            let url = target.url.as_deref().unwrap_or("");
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(ConfigWriteError::InvalidNotifyTarget {
                    reason: format!(
                        "target {:?}: kind \"webhook\" requires a `url` starting with http:// or https://",
                        target.name
                    ),
                });
            }
        }
        "email" => {
            if !target.to.as_deref().unwrap_or("").contains('@') {
                return Err(ConfigWriteError::InvalidNotifyTarget {
                    reason: format!(
                        "target {:?}: kind \"email\" requires a `to` address containing `@`",
                        target.name
                    ),
                });
            }
        }
        "web-ui" => {}
        other => {
            return Err(ConfigWriteError::InvalidNotifyTarget {
                reason: format!(
                    "target {:?}: unknown kind {:?} (expected \"telegram\", \"webhook\", \"email\", or \"web-ui\")",
                    target.name, other
                ),
            });
        }
    }

    let mut doc = load_document(path)?;
    let arr = notify_target_array_mut(&mut doc);

    if target.is_default {
        let other_default = arr
            .iter()
            .any(|t| {
                t.get("name").and_then(|v| v.as_str()) != Some(target.name.as_str())
                    && t.get("default").and_then(|v| v.as_bool()).unwrap_or(false)
            });
        if other_default {
            return Err(ConfigWriteError::InvalidNotifyTarget {
                reason: format!(
                    "target {:?}: another notify_target is already the default — at most one is allowed",
                    target.name
                ),
            });
        }
    }

    let mut table = toml_edit::Table::new();
    table["name"] = value(target.name.as_str());
    table["kind"] = value(target.kind.as_str());
    table["enabled"] = value(target.enabled);
    table["default"] = value(target.is_default);
    if let Some(chat_id) = &target.chat_id {
        table["chat_id"] = value(chat_id.as_str());
    }
    if let Some(url) = &target.url {
        table["url"] = value(url.as_str());
    }
    if let Some(to) = &target.to {
        table["to"] = value(to.as_str());
    }
    if target.retry_count != 0 {
        table["retry_count"] = value(target.retry_count as i64);
    }
    if target.retry_backoff_ms_start != 500 {
        table["retry_backoff_ms_start"] = value(target.retry_backoff_ms_start as i64);
    }
    if let Some(max) = target.rate_limit_max {
        table["rate_limit_max"] = value(max as i64);
    }
    if let Some(secs) = target.rate_limit_window_secs {
        table["rate_limit_window_secs"] = value(secs as i64);
    }

    // Preserve any key this write schema doesn't know about, exactly
    // mirroring `write_mcp_server_section`'s own defensive convention —
    // no known unexposed field exists on `[[notify_target]]` today, but
    // an upsert must never silently destroy one a future schema adds.
    const KNOWN_KEYS: &[&str] = &[
        "name", "kind", "enabled", "default", "chat_id", "url", "to",
        "retry_count", "retry_backoff_ms_start", "rate_limit_max", "rate_limit_window_secs",
    ];
    let idx = arr.iter().position(|t| t.get("name").and_then(|v| v.as_str()) == Some(target.name.as_str()));
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

/// Remove one `[[notify_target]]` entry by `name`. A no-op (not an error)
/// when no entry with that name exists.
pub fn remove_notify_target_section(path: &Path, name: &str) -> Result<(), ConfigWriteError> {
    let mut doc = load_document(path)?;
    let arr = notify_target_array_mut(&mut doc);
    if let Some(i) = arr.iter().position(|t| t.get("name").and_then(|v| v.as_str()) == Some(name)) {
        arr.remove(i);
    }
    write_toml_0600(path, &doc.to_string())
}

/// Read every `[[notify_target]]` entry as literally written on disk — no
/// resolution of anything, matching [`read_mcp_server_entries`]'s own
/// raw-TOML convention (this section carries no secrets of its own today,
/// but reading it the same way as every other section here keeps one
/// convention, not two, for future maintainers to reason about).
pub fn read_notify_target_entries(path: &Path) -> Result<Vec<NotifyTargetEntryWrite>, ConfigWriteError> {
    let doc = load_document(path)?;
    let Some(arr) = doc.get("notify_target").and_then(toml_edit::Item::as_array_of_tables) else {
        return Ok(Vec::new());
    };
    Ok(arr.iter().map(raw_table_to_notify_target).collect())
}

fn raw_table_to_notify_target(table: &toml_edit::Table) -> NotifyTargetEntryWrite {
    let str_field = |key: &str| table.get(key).and_then(|v| v.as_str()).map(str::to_string);
    NotifyTargetEntryWrite {
        name: str_field("name").unwrap_or_default(),
        kind: str_field("kind").unwrap_or_default(),
        chat_id: str_field("chat_id"),
        url: str_field("url"),
        to: str_field("to"),
        enabled: table.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
        is_default: table.get("default").and_then(|v| v.as_bool()).unwrap_or(false),
        retry_count: table.get("retry_count").and_then(|v| v.as_integer()).unwrap_or(0) as u32,
        retry_backoff_ms_start: table
            .get("retry_backoff_ms_start")
            .and_then(|v| v.as_integer())
            .unwrap_or(500) as u64,
        rate_limit_max: table.get("rate_limit_max").and_then(|v| v.as_integer()).map(|n| n as u32),
        rate_limit_window_secs: table
            .get("rate_limit_window_secs")
            .and_then(|v| v.as_integer())
            .map(|n| n as u64),
    }
}

/// The `[[notify_target]]` array, creating an empty one if the section is
/// absent from the document yet.
fn notify_target_array_mut(doc: &mut DocumentMut) -> &mut toml_edit::ArrayOfTables {
    if doc.get("notify_target").and_then(toml_edit::Item::as_array_of_tables).is_none() {
        doc["notify_target"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    doc["notify_target"]
        .as_array_of_tables_mut()
        .expect("just ensured present")
}
```

`Option::is_none_or` is stable since Rust 1.82 — check this workspace's `rust-version` in the root `Cargo.toml` if the build fails on this method; if the toolchain floor is older, replace `target.chat_id.as_deref().is_none_or(str::is_empty)` with `target.chat_id.as_deref().map(str::is_empty).unwrap_or(true)`.

- [ ] **Step 5: Run to verify it passes**

```bash
cargo test -p aivyx-config config_write::tests::notify_target
cargo test -p aivyx-config config_write::tests::read_notify_target_entries_round_trips
```
Expected: PASS (9 passed).

- [ ] **Step 6: Full-crate check**

```bash
cargo test -p aivyx-config
cargo clippy -p aivyx-config --all-targets -- -D warnings
```
Expected: all green, zero warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-config/src/config_write.rs
git commit -m "feat(config): array-of-table config-write primitive for [[notify_target]]

write_notify_target_section/remove_notify_target_section/
read_notify_target_entries mirror the now-proven [[mcp_server]]
pattern exactly, including raw-TOML reads (never resolving anything)
and unknown-key preservation on upsert. Adds per-kind validation
(telegram/webhook/email) and the at-most-one-default rule, both
mirroring the loader's own checks."
```

---

## Task 2: `[email]`/`[telegram]`/`[discord]`/`[slack]` singleton primitives

**Files:**
- Modify: `crates/aivyx-config/src/config_write.rs`

**Interfaces:**
- Produces: `pub struct EmailEntryWrite { pub host: Option<String>, pub port: Option<u16>, pub tls_mode: Option<String>, pub username: Option<String>, pub password: Option<String>, pub from: Option<String> }`, `pub fn write_email_section`/`pub fn read_email_section`; `pub struct TelegramEntryWrite { pub token: Option<String>, pub chat_id: Option<i64>, pub team_run_channel: Option<bool>, pub team_trigger_rate_limit: Option<u32>, pub team_command_allowed_senders: Option<Vec<i64>> }`, `write_telegram_section`/`read_telegram_section`; `pub struct DiscordEntryWrite { pub token: Option<String>, pub application_id: Option<u64>, pub team_run_channel: Option<bool>, pub team_trigger_rate_limit: Option<u32>, pub team_command_allowed_senders: Option<Vec<u64>> }`, `write_discord_section`/`read_discord_section`; `pub struct SlackEntryWrite { pub bot_token: Option<String>, pub app_token: Option<String>, pub team_id: Option<String>, pub team_run_channel: Option<bool>, pub team_trigger_rate_limit: Option<u32>, pub team_command_allowed_senders: Option<Vec<String>> }`, `write_slack_section`/`read_slack_section`.

Every read function returns the struct with every field populated from disk (`None` only when the key is genuinely absent) — the **secret fields are NOT redacted at this layer**; `aivyx-config` has no dependency on `aivyx-ipc` and doesn't know about `RedactedSecret`. Redaction happens in `aivyx-channel`'s daemon handler (Task 6), which is the layer that builds the wire response.

- [ ] **Step 1: Write the failing tests**

Add to `crates/aivyx-config/src/config_write.rs`'s test module:

```rust
    #[test]
    fn email_write_then_read_round_trips_all_fields() {
        let path = temp_toml("email-round-trip");
        std::fs::write(&path, "").unwrap();
        write_email_section(&path, &EmailEntryWrite {
            host: Some("smtp.example.com".to_string()),
            port: Some(587),
            tls_mode: Some("starttls".to_string()),
            username: Some("bot@example.com".to_string()),
            password: Some("hunter2".to_string()),
            from: Some("bot@example.com".to_string()),
        }).unwrap();
        let read = read_email_section(&path).unwrap();
        assert_eq!(read.host.as_deref(), Some("smtp.example.com"));
        assert_eq!(read.port, Some(587));
        assert_eq!(read.password.as_deref(), Some("hunter2"));
    }

    #[test]
    fn email_write_with_none_password_leaves_existing_password_untouched() {
        let path = temp_toml("email-keep-password");
        std::fs::write(&path, "").unwrap();
        write_email_section(&path, &EmailEntryWrite {
            host: Some("smtp.example.com".to_string()),
            port: None,
            tls_mode: None,
            username: None,
            password: Some("original-secret".to_string()),
            from: None,
        }).unwrap();
        // Second write: change only the host, password is None ("don't touch").
        write_email_section(&path, &EmailEntryWrite {
            host: Some("smtp2.example.com".to_string()),
            port: None,
            tls_mode: None,
            username: None,
            password: None,
            from: None,
        }).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("original-secret"), "password survives when not touched");
        assert!(contents.contains("smtp2.example.com"));
    }

    #[test]
    fn telegram_write_then_read_round_trips() {
        let path = temp_toml("telegram-round-trip");
        std::fs::write(&path, "").unwrap();
        write_telegram_section(&path, &TelegramEntryWrite {
            token: Some("123:ABC".to_string()),
            chat_id: Some(42),
            team_run_channel: Some(true),
            team_trigger_rate_limit: Some(5),
            team_command_allowed_senders: Some(vec![111, 222]),
        }).unwrap();
        let read = read_telegram_section(&path).unwrap();
        assert_eq!(read.token.as_deref(), Some("123:ABC"));
        assert_eq!(read.chat_id, Some(42));
        assert_eq!(read.team_command_allowed_senders, Some(vec![111, 222]));
    }

    #[test]
    fn telegram_write_with_none_token_leaves_existing_token_untouched() {
        let path = temp_toml("telegram-keep-token");
        std::fs::write(&path, "").unwrap();
        write_telegram_section(&path, &TelegramEntryWrite {
            token: Some("original-token".to_string()),
            chat_id: None,
            team_run_channel: None,
            team_trigger_rate_limit: None,
            team_command_allowed_senders: None,
        }).unwrap();
        write_telegram_section(&path, &TelegramEntryWrite {
            token: None,
            chat_id: Some(99),
            team_run_channel: None,
            team_trigger_rate_limit: None,
            team_command_allowed_senders: None,
        }).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("original-token"));
        assert!(contents.contains("chat_id = 99"));
    }

    #[test]
    fn discord_write_then_read_round_trips() {
        let path = temp_toml("discord-round-trip");
        std::fs::write(&path, "").unwrap();
        write_discord_section(&path, &DiscordEntryWrite {
            token: Some("discord-token".to_string()),
            application_id: Some(555),
            team_run_channel: Some(false),
            team_trigger_rate_limit: None,
            team_command_allowed_senders: Some(vec![1, 2, 3]),
        }).unwrap();
        let read = read_discord_section(&path).unwrap();
        assert_eq!(read.token.as_deref(), Some("discord-token"));
        assert_eq!(read.application_id, Some(555));
    }

    #[test]
    fn slack_write_then_read_round_trips_both_tokens() {
        let path = temp_toml("slack-round-trip");
        std::fs::write(&path, "").unwrap();
        write_slack_section(&path, &SlackEntryWrite {
            bot_token: Some("xoxb-1".to_string()),
            app_token: Some("xapp-1".to_string()),
            team_id: Some("T123".to_string()),
            team_run_channel: None,
            team_trigger_rate_limit: None,
            team_command_allowed_senders: Some(vec!["U1".to_string()]),
        }).unwrap();
        let read = read_slack_section(&path).unwrap();
        assert_eq!(read.bot_token.as_deref(), Some("xoxb-1"));
        assert_eq!(read.app_token.as_deref(), Some("xapp-1"));
    }

    #[test]
    fn slack_write_with_none_app_token_leaves_it_untouched_while_updating_bot_token() {
        let path = temp_toml("slack-partial-update");
        std::fs::write(&path, "").unwrap();
        write_slack_section(&path, &SlackEntryWrite {
            bot_token: Some("xoxb-old".to_string()),
            app_token: Some("xapp-keep-me".to_string()),
            team_id: None,
            team_run_channel: None,
            team_trigger_rate_limit: None,
            team_command_allowed_senders: None,
        }).unwrap();
        write_slack_section(&path, &SlackEntryWrite {
            bot_token: Some("xoxb-new".to_string()),
            app_token: None,
            team_id: None,
            team_run_channel: None,
            team_trigger_rate_limit: None,
            team_command_allowed_senders: None,
        }).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("xoxb-new"));
        assert!(!contents.contains("xoxb-old"));
        assert!(contents.contains("xapp-keep-me"), "untouched field survives");
    }

    #[test]
    fn email_read_of_missing_section_returns_all_none() {
        let path = temp_toml("email-missing");
        std::fs::write(&path, "[access]\nlevel = \"sandbox\"\n").unwrap();
        let read = read_email_section(&path).unwrap();
        assert_eq!(read.host, None);
        assert_eq!(read.password, None);
    }
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p aivyx-config config_write::tests::email
cargo test -p aivyx-config config_write::tests::telegram
cargo test -p aivyx-config config_write::tests::discord
cargo test -p aivyx-config config_write::tests::slack
```
Expected: FAIL — none of the 8 new functions/structs exist yet.

- [ ] **Step 3: Add the four singleton primitives**

Add to `crates/aivyx-config/src/config_write.rs`, after the notify-target primitive from Task 1:

```rust
/// The `[email]` section as Studio's write form submits it — every field
/// `Option`, `None` meaning "leave this key untouched on disk" (the
/// partial-update convention every singleton section in this file uses).
pub struct EmailEntryWrite {
    pub host: Option<String>,
    pub port: Option<u16>,
    /// `"starttls"`, `"implicit"`, or `"none"` — matches the loader's own
    /// accepted strings.
    pub tls_mode: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: Option<String>,
}

/// Patch the `[email]` section, touching only the `Some` fields.
pub fn write_email_section(path: &Path, entry: &EmailEntryWrite) -> Result<(), ConfigWriteError> {
    let mut doc = load_document(path)?;
    if let Some(v) = &entry.host {
        doc["email"]["host"] = value(v.as_str());
    }
    if let Some(v) = entry.port {
        doc["email"]["port"] = value(v as i64);
    }
    if let Some(v) = &entry.tls_mode {
        doc["email"]["tls_mode"] = value(v.as_str());
    }
    if let Some(v) = &entry.username {
        doc["email"]["username"] = value(v.as_str());
    }
    if let Some(v) = &entry.password {
        doc["email"]["password"] = value(v.as_str());
    }
    if let Some(v) = &entry.from {
        doc["email"]["from"] = value(v.as_str());
    }
    write_toml_0600(path, &doc.to_string())
}

/// Read the `[email]` section as literally written on disk — no
/// resolution of anything. Absent section (or absent key) reads as `None`.
pub fn read_email_section(path: &Path) -> Result<EmailEntryWrite, ConfigWriteError> {
    let doc = load_document(path)?;
    let table = doc.get("email").and_then(toml_edit::Item::as_table);
    let str_field = |key: &str| table.and_then(|t| t.get(key)).and_then(|v| v.as_str()).map(str::to_string);
    Ok(EmailEntryWrite {
        host: str_field("host"),
        port: table.and_then(|t| t.get("port")).and_then(|v| v.as_integer()).map(|n| n as u16),
        tls_mode: str_field("tls_mode"),
        username: str_field("username"),
        password: str_field("password"),
        from: str_field("from"),
    })
}

/// The `[telegram]` section as Studio's write form submits it. Partial
/// update, same convention as [`EmailEntryWrite`].
pub struct TelegramEntryWrite {
    pub token: Option<String>,
    pub chat_id: Option<i64>,
    pub team_run_channel: Option<bool>,
    pub team_trigger_rate_limit: Option<u32>,
    pub team_command_allowed_senders: Option<Vec<i64>>,
}

pub fn write_telegram_section(path: &Path, entry: &TelegramEntryWrite) -> Result<(), ConfigWriteError> {
    let mut doc = load_document(path)?;
    if let Some(v) = &entry.token {
        doc["telegram"]["token"] = value(v.as_str());
    }
    if let Some(v) = entry.chat_id {
        doc["telegram"]["chat_id"] = value(v);
    }
    if let Some(v) = entry.team_run_channel {
        doc["telegram"]["team_run_channel"] = value(v);
    }
    if let Some(v) = entry.team_trigger_rate_limit {
        doc["telegram"]["team_trigger_rate_limit"] = value(v as i64);
    }
    if let Some(ids) = &entry.team_command_allowed_senders {
        let mut arr = toml_edit::Array::new();
        for id in ids {
            arr.push(*id);
        }
        doc["telegram"]["team_command_allowed_senders"] = toml_edit::Item::Value(arr.into());
    }
    write_toml_0600(path, &doc.to_string())
}

pub fn read_telegram_section(path: &Path) -> Result<TelegramEntryWrite, ConfigWriteError> {
    let doc = load_document(path)?;
    let table = doc.get("telegram").and_then(toml_edit::Item::as_table);
    Ok(TelegramEntryWrite {
        token: table.and_then(|t| t.get("token")).and_then(|v| v.as_str()).map(str::to_string),
        chat_id: table.and_then(|t| t.get("chat_id")).and_then(|v| v.as_integer()),
        team_run_channel: table.and_then(|t| t.get("team_run_channel")).and_then(|v| v.as_bool()),
        team_trigger_rate_limit: table
            .and_then(|t| t.get("team_trigger_rate_limit"))
            .and_then(|v| v.as_integer())
            .map(|n| n as u32),
        team_command_allowed_senders: table
            .and_then(|t| t.get("team_command_allowed_senders"))
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_integer()).collect()),
    })
}

/// The `[discord]` section as Studio's write form submits it. Partial
/// update, same convention as [`EmailEntryWrite`].
pub struct DiscordEntryWrite {
    pub token: Option<String>,
    pub application_id: Option<u64>,
    pub team_run_channel: Option<bool>,
    pub team_trigger_rate_limit: Option<u32>,
    pub team_command_allowed_senders: Option<Vec<u64>>,
}

pub fn write_discord_section(path: &Path, entry: &DiscordEntryWrite) -> Result<(), ConfigWriteError> {
    let mut doc = load_document(path)?;
    if let Some(v) = &entry.token {
        doc["discord"]["token"] = value(v.as_str());
    }
    if let Some(v) = entry.application_id {
        doc["discord"]["application_id"] = value(v as i64);
    }
    if let Some(v) = entry.team_run_channel {
        doc["discord"]["team_run_channel"] = value(v);
    }
    if let Some(v) = entry.team_trigger_rate_limit {
        doc["discord"]["team_trigger_rate_limit"] = value(v as i64);
    }
    if let Some(ids) = &entry.team_command_allowed_senders {
        let mut arr = toml_edit::Array::new();
        for id in ids {
            arr.push(*id as i64);
        }
        doc["discord"]["team_command_allowed_senders"] = toml_edit::Item::Value(arr.into());
    }
    write_toml_0600(path, &doc.to_string())
}

pub fn read_discord_section(path: &Path) -> Result<DiscordEntryWrite, ConfigWriteError> {
    let doc = load_document(path)?;
    let table = doc.get("discord").and_then(toml_edit::Item::as_table);
    Ok(DiscordEntryWrite {
        token: table.and_then(|t| t.get("token")).and_then(|v| v.as_str()).map(str::to_string),
        application_id: table
            .and_then(|t| t.get("application_id"))
            .and_then(|v| v.as_integer())
            .map(|n| n as u64),
        team_run_channel: table.and_then(|t| t.get("team_run_channel")).and_then(|v| v.as_bool()),
        team_trigger_rate_limit: table
            .and_then(|t| t.get("team_trigger_rate_limit"))
            .and_then(|v| v.as_integer())
            .map(|n| n as u32),
        team_command_allowed_senders: table
            .and_then(|t| t.get("team_command_allowed_senders"))
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_integer()).map(|n| n as u64).collect()),
    })
}

/// The `[slack]` section as Studio's write form submits it. Partial
/// update, same convention as [`EmailEntryWrite`] — two independent
/// secrets (`bot_token`/`app_token`), each individually optional so one
/// can be rotated without touching the other.
pub struct SlackEntryWrite {
    pub bot_token: Option<String>,
    pub app_token: Option<String>,
    pub team_id: Option<String>,
    pub team_run_channel: Option<bool>,
    pub team_trigger_rate_limit: Option<u32>,
    pub team_command_allowed_senders: Option<Vec<String>>,
}

pub fn write_slack_section(path: &Path, entry: &SlackEntryWrite) -> Result<(), ConfigWriteError> {
    let mut doc = load_document(path)?;
    if let Some(v) = &entry.bot_token {
        doc["slack"]["bot_token"] = value(v.as_str());
    }
    if let Some(v) = &entry.app_token {
        doc["slack"]["app_token"] = value(v.as_str());
    }
    if let Some(v) = &entry.team_id {
        doc["slack"]["team_id"] = value(v.as_str());
    }
    if let Some(v) = entry.team_run_channel {
        doc["slack"]["team_run_channel"] = value(v);
    }
    if let Some(v) = entry.team_trigger_rate_limit {
        doc["slack"]["team_trigger_rate_limit"] = value(v as i64);
    }
    if let Some(ids) = &entry.team_command_allowed_senders {
        let mut arr = toml_edit::Array::new();
        for id in ids {
            arr.push(id.as_str());
        }
        doc["slack"]["team_command_allowed_senders"] = toml_edit::Item::Value(arr.into());
    }
    write_toml_0600(path, &doc.to_string())
}

pub fn read_slack_section(path: &Path) -> Result<SlackEntryWrite, ConfigWriteError> {
    let doc = load_document(path)?;
    let table = doc.get("slack").and_then(toml_edit::Item::as_table);
    Ok(SlackEntryWrite {
        bot_token: table.and_then(|t| t.get("bot_token")).and_then(|v| v.as_str()).map(str::to_string),
        app_token: table.and_then(|t| t.get("app_token")).and_then(|v| v.as_str()).map(str::to_string),
        team_id: table.and_then(|t| t.get("team_id")).and_then(|v| v.as_str()).map(str::to_string),
        team_run_channel: table.and_then(|t| t.get("team_run_channel")).and_then(|v| v.as_bool()),
        team_trigger_rate_limit: table
            .and_then(|t| t.get("team_trigger_rate_limit"))
            .and_then(|v| v.as_integer())
            .map(|n| n as u32),
        team_command_allowed_senders: table
            .and_then(|t| t.get("team_command_allowed_senders"))
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).map(str::to_string).collect()),
    })
}
```

- [ ] **Step 4: Run to verify it passes**

```bash
cargo test -p aivyx-config config_write::tests::email
cargo test -p aivyx-config config_write::tests::telegram
cargo test -p aivyx-config config_write::tests::discord
cargo test -p aivyx-config config_write::tests::slack
```
Expected: PASS (9 passed).

- [ ] **Step 5: Full-crate check**

```bash
cargo test -p aivyx-config
cargo clippy -p aivyx-config --all-targets -- -D warnings
```
Expected: all green, zero warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-config/src/config_write.rs
git commit -m "feat(config): partial-update write/read for [email]/[telegram]/[discord]/[slack]

Every field Option<T>; None means leave the on-disk value untouched.
Read functions parse raw TOML only (no interpolation) — the layer
above (aivyx-channel) is responsible for redacting secret fields
before they cross the wire, since aivyx-config has no dependency on
aivyx-ipc's RedactedSecret type."
```

---

## Task 3: `[[notify_target]]` wire types + daemon handlers

**Files:**
- Modify: `crates/aivyx-ipc/src/protocol.rs`
- Modify: `crates/aivyx-channel/src/daemon_server.rs`

**Interfaces:**
- Consumes: `aivyx_config::config_write::{write_notify_target_section, remove_notify_target_section, read_notify_target_entries, NotifyTargetEntryWrite}` (Task 1).
- Produces: `pub struct NotifyTargetConfigView { pub name: String, pub kind: String, pub chat_id: Option<String>, pub url: Option<String>, pub to: Option<String>, pub enabled: bool, pub is_default: bool, pub retry_count: u32, pub retry_backoff_ms_start: u64, pub rate_limit_max: Option<u32>, pub rate_limit_window_secs: Option<u64> }` (aivyx-ipc; field-for-field identical to `NotifyTargetEntryWrite` — no secrets on this type, so no `RedactedSecret` involved); `QueryPayload::{GetNotifyTargetConfigs, SetNotifyTarget{...}, DeleteNotifyTarget{name}}`; `QueryResponsePayload::{GetNotifyTargetConfigs{targets}, NotifyTargetsApplied{targets, restart_required}}`.

- [ ] **Step 1: Add the wire types**

In `crates/aivyx-ipc/src/protocol.rs`, near the existing `NotifyTargetView` (the read-only status type, `main.rs`'s `NotificationsPanel` "Targets" rail) — add a distinct, separate config-editing type:

```rust
/// The **editable configuration** of one `[[notify_target]]` entry —
/// distinct from `NotifyTargetView` (the read-only status type shown in
/// the Notifications screen's "Targets" rail). No secret fields — the
/// actual credentials for an email-kind target live in the separate
/// `[email]` section (see `EmailConfigView`), and telegram/webhook/email
/// targets here only carry routing data (`chat_id`/`url`/`to`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotifyTargetConfigView {
    pub name: String,
    pub kind: String,
    pub chat_id: Option<String>,
    pub url: Option<String>,
    pub to: Option<String>,
    pub enabled: bool,
    pub is_default: bool,
    pub retry_count: u32,
    pub retry_backoff_ms_start: u64,
    pub rate_limit_max: Option<u32>,
    pub rate_limit_window_secs: Option<u64>,
}
```

In `QueryPayload` (near `GetNotifyTargets`):

```rust
    /// POLISH_WAVES.md sub-project 7 plan 2 — the editable notify-target
    /// list (distinct from `GetNotifyTargets`'s read-only status view).
    /// Responds with [`QueryResponsePayload::GetNotifyTargetConfigs`].
    GetNotifyTargetConfigs,
    /// Add or replace (by `name`) one `[[notify_target]]` entry. Takes
    /// effect on the next daemon start. Responds with
    /// [`QueryResponsePayload::NotifyTargetsApplied`] (or `QueryError`).
    SetNotifyTarget {
        name: String,
        kind: String,
        #[serde(default)]
        chat_id: Option<String>,
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        to: Option<String>,
        enabled: bool,
        is_default: bool,
        #[serde(default)]
        retry_count: u32,
        #[serde(default)]
        retry_backoff_ms_start: u64,
        #[serde(default)]
        rate_limit_max: Option<u32>,
        #[serde(default)]
        rate_limit_window_secs: Option<u64>,
    },
    /// Remove one `[[notify_target]]` entry by name (a no-op if absent).
    /// Responds with [`QueryResponsePayload::NotifyTargetsApplied`].
    DeleteNotifyTarget { name: String },
```

In `QueryResponsePayload` (near `GetNotifyTargets`'s response variant):

```rust
    /// Response to [`QueryPayload::GetNotifyTargetConfigs`].
    GetNotifyTargetConfigs { targets: Vec<NotifyTargetConfigView> },
    /// Response to [`QueryPayload::SetNotifyTarget`] / [`QueryPayload::
    /// DeleteNotifyTarget`]. Carries the fresh list and `restart_required`
    /// (always `true` — notify targets are boot-constructed).
    NotifyTargetsApplied {
        targets: Vec<NotifyTargetConfigView>,
        restart_required: bool,
    },
```

- [ ] **Step 2: Round-trip test**

Add alongside `set_mcp_server_round_trips` (plan 1's own test, same test module):

```rust
    #[test]
    fn set_notify_target_round_trips() {
        let msg = QueryPayload::SetNotifyTarget {
            name: "ops".to_string(),
            kind: "telegram".to_string(),
            chat_id: Some("123456".to_string()),
            url: None,
            to: None,
            enabled: true,
            is_default: false,
            retry_count: 0,
            retry_backoff_ms_start: 500,
            rate_limit_max: None,
            rate_limit_window_secs: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: QueryPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn notify_target_config_view_round_trips() {
        let view = NotifyTargetConfigView {
            name: "ops".to_string(),
            kind: "telegram".to_string(),
            chat_id: Some("123456".to_string()),
            url: None,
            to: None,
            enabled: true,
            is_default: false,
            retry_count: 0,
            retry_backoff_ms_start: 500,
            rate_limit_max: None,
            rate_limit_window_secs: None,
        };
        let json = serde_json::to_string(&view).unwrap();
        let back: NotifyTargetConfigView = serde_json::from_str(&json).unwrap();
        assert_eq!(back, view);
    }
```

- [ ] **Step 3: Verify wire types**

```bash
cargo test -p aivyx-ipc set_notify_target_round_trips notify_target_config_view_round_trips
cargo clippy -p aivyx-ipc --all-targets -- -D warnings
```
Expected: PASS, zero warnings.

- [ ] **Step 4: Add the daemon handlers**

Find `QueryPayload::GetMcpServerConfigs`'s handler in `crates/aivyx-channel/src/daemon_server.rs` and add a new arm nearby:

```rust
        QueryPayload::GetNotifyTargetConfigs => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            match read_notify_target_configs(path) {
                Ok(targets) => QueryResponsePayload::GetNotifyTargetConfigs { targets },
                Err(e) => QueryResponsePayload::QueryError {
                    code: "config_reload_failed".into(),
                    message: format!("failed to read notify target configs: {e}"),
                },
            }
        }
```

Find `QueryPayload::SetMcpServer`/`DeleteMcpServer`'s handlers and add two matching arms:

```rust
        QueryPayload::SetNotifyTarget {
            name,
            kind,
            chat_id,
            url,
            to,
            enabled,
            is_default,
            retry_count,
            retry_backoff_ms_start,
            rate_limit_max,
            rate_limit_window_secs,
        } => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            let entry = aivyx_config::config_write::NotifyTargetEntryWrite {
                name: name.clone(),
                kind,
                chat_id,
                url,
                to,
                enabled,
                is_default,
                retry_count,
                retry_backoff_ms_start,
                rate_limit_max,
                rate_limit_window_secs,
            };
            match aivyx_config::config_write::write_notify_target_section(path, &entry) {
                Ok(()) => {
                    audit_config_change(audit_log, "notify_target", &format!("set {name}"));
                    notify_targets_applied_after_write(path, "set")
                }
                Err(e) => map_config_write_error(e),
            }
        }
        QueryPayload::DeleteNotifyTarget { name } => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            match aivyx_config::config_write::remove_notify_target_section(path, &name) {
                Ok(()) => {
                    audit_config_change(audit_log, "notify_target", &format!("delete {name}"));
                    notify_targets_applied_after_write(path, "delete")
                }
                Err(e) => map_config_write_error(e),
            }
        }
```

Find `map_config_write_error` (which already gained `InvalidMcpServer` in plan 1) and add the new arm this task's `ConfigWriteError::InvalidNotifyTarget` variant requires (the match is exhaustive — this is a compile-time requirement, not optional):

```rust
        E::InvalidNotifyTarget { .. } => "invalid_notify_target",
```

Find `read_mcp_server_configs`/`mcp_servers_applied_after_write` (plan 1's own helpers) and add the matching pair for notify targets right after them:

```rust
fn read_notify_target_configs(
    path: &std::path::Path,
) -> Result<Vec<aivyx_ipc::protocol::NotifyTargetConfigView>, String> {
    let entries = aivyx_config::config_write::read_notify_target_entries(path)
        .map_err(|e| e.to_string())?;
    Ok(entries
        .into_iter()
        .map(|t| aivyx_ipc::protocol::NotifyTargetConfigView {
            name: t.name,
            kind: t.kind,
            chat_id: t.chat_id,
            url: t.url,
            to: t.to,
            enabled: t.enabled,
            is_default: t.is_default,
            retry_count: t.retry_count,
            retry_backoff_ms_start: t.retry_backoff_ms_start,
            rate_limit_max: t.rate_limit_max,
            rate_limit_window_secs: t.rate_limit_window_secs,
        })
        .collect())
}

/// Mirrors `mcp_servers_applied_after_write`'s own convention exactly,
/// including surfacing a re-read failure as a `QueryError` rather than
/// silently claiming zero targets (plan 1's final-review finding #5).
fn notify_targets_applied_after_write(path: &std::path::Path, verb: &str) -> QueryResponsePayload {
    match read_notify_target_configs(path) {
        Ok(targets) => QueryResponsePayload::NotifyTargetsApplied {
            targets,
            restart_required: true,
        },
        Err(e) => QueryResponsePayload::QueryError {
            code: "config_reload_failed".into(),
            message: format!("notify target {verb} succeeded, but reloading the list failed: {e}"),
        },
    }
}
```

- [ ] **Step 5: Verify**

```bash
cargo build -p aivyx-channel
cargo clippy -p aivyx-channel --all-targets -- -D warnings
cargo test -p aivyx-config -p aivyx-ipc -p aivyx-channel
```
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-ipc/src/protocol.rs crates/aivyx-channel/src/daemon_server.rs
git commit -m "feat(ipc,channel): notify-target config wire types + daemon handlers

Mirrors plan 1's GetMcpServerConfigs/SetMcpServer/DeleteMcpServer
pattern exactly, including the fresh-re-read-with-QueryError-on-
failure convention (mcp_servers_applied_after_write's own lesson)."
```

---

## Task 4: `[email]`/`[telegram]`/`[discord]`/`[slack]` wire types

**Files:**
- Modify: `crates/aivyx-ipc/src/protocol.rs`

**Interfaces:**
- Consumes: `RedactedSecret` (shipped in plan 1).
- Produces: `pub struct EmailConfigView { pub host: Option<String>, pub port: Option<u16>, pub tls_mode: Option<String>, pub username: Option<String>, pub password: RedactedSecret, pub from: Option<String> }`; `pub struct TelegramConfigView { pub token: RedactedSecret, pub chat_id: Option<i64>, pub team_run_channel: bool, pub team_trigger_rate_limit: Option<u32>, pub team_command_allowed_senders: Vec<i64> }`; `pub struct DiscordConfigView { pub token: RedactedSecret, pub application_id: Option<u64>, pub team_run_channel: bool, pub team_trigger_rate_limit: Option<u32>, pub team_command_allowed_senders: Vec<u64> }`; `pub struct SlackConfigView { pub bot_token: RedactedSecret, pub app_token: RedactedSecret, pub team_id: Option<String>, pub team_run_channel: bool, pub team_trigger_rate_limit: Option<u32>, pub team_command_allowed_senders: Vec<String> }`; `QueryPayload::{GetEmailConfig, SetEmailConfig{...}, GetTelegramConfig, SetTelegramConfig{...}, GetDiscordConfig, SetDiscordConfig{...}, GetSlackConfig, SetSlackConfig{...}}`; matching `QueryResponsePayload` variants (`GetXConfig{config}`, `XConfigApplied{config, restart_required}`) for each of the 4.

Note the View types use non-`Option` `bool`/`Vec` for `team_run_channel`/`team_command_allowed_senders` (defaulted from the read side — `false`/empty when absent), while the corresponding `Set*` write payloads use `Option<T>` for every field (the partial-update convention) — this is intentional: what the operator SEES is always a concrete value (never null/undefined in the form), but what they SEND back on save only carries `Some` for fields they actually changed.

- [ ] **Step 1: Add the wire types**

In `crates/aivyx-ipc/src/protocol.rs`, near `RedactedSecret`:

```rust
/// The **editable configuration** of the shared `[email]` SMTP block.
/// `password` never carries the real value (see [`RedactedSecret`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmailConfigView {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub tls_mode: Option<String>,
    pub username: Option<String>,
    pub password: RedactedSecret,
    pub from: Option<String>,
}

/// The **editable configuration** of the `[telegram]` channel adapter.
/// `token` never carries the real value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelegramConfigView {
    pub token: RedactedSecret,
    pub chat_id: Option<i64>,
    pub team_run_channel: bool,
    pub team_trigger_rate_limit: Option<u32>,
    pub team_command_allowed_senders: Vec<i64>,
}

/// The **editable configuration** of the `[discord]` channel adapter.
/// `token` never carries the real value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscordConfigView {
    pub token: RedactedSecret,
    pub application_id: Option<u64>,
    pub team_run_channel: bool,
    pub team_trigger_rate_limit: Option<u32>,
    pub team_command_allowed_senders: Vec<u64>,
}

/// The **editable configuration** of the `[slack]` channel adapter. Two
/// independent secrets, neither ever carrying its real value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlackConfigView {
    pub bot_token: RedactedSecret,
    pub app_token: RedactedSecret,
    pub team_id: Option<String>,
    pub team_run_channel: bool,
    pub team_trigger_rate_limit: Option<u32>,
    pub team_command_allowed_senders: Vec<String>,
}
```

In `QueryPayload`, add all 8 variants (one Get + one Set per section):

```rust
    /// Response: [`QueryResponsePayload::GetEmailConfig`].
    GetEmailConfig,
    /// `password: None` means leave the existing SMTP password untouched.
    /// Responds with [`QueryResponsePayload::EmailConfigApplied`].
    SetEmailConfig {
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        port: Option<u16>,
        #[serde(default)]
        tls_mode: Option<String>,
        #[serde(default)]
        username: Option<String>,
        #[serde(default)]
        password: Option<String>,
        #[serde(default)]
        from: Option<String>,
    },
    /// Response: [`QueryResponsePayload::GetTelegramConfig`].
    GetTelegramConfig,
    /// `token: None` means leave the existing bot token untouched.
    /// Responds with [`QueryResponsePayload::TelegramConfigApplied`].
    SetTelegramConfig {
        #[serde(default)]
        token: Option<String>,
        #[serde(default)]
        chat_id: Option<i64>,
        #[serde(default)]
        team_run_channel: Option<bool>,
        #[serde(default)]
        team_trigger_rate_limit: Option<u32>,
        #[serde(default)]
        team_command_allowed_senders: Option<Vec<i64>>,
    },
    /// Response: [`QueryResponsePayload::GetDiscordConfig`].
    GetDiscordConfig,
    /// `token: None` means leave the existing bot token untouched.
    /// Responds with [`QueryResponsePayload::DiscordConfigApplied`].
    SetDiscordConfig {
        #[serde(default)]
        token: Option<String>,
        #[serde(default)]
        application_id: Option<u64>,
        #[serde(default)]
        team_run_channel: Option<bool>,
        #[serde(default)]
        team_trigger_rate_limit: Option<u32>,
        #[serde(default)]
        team_command_allowed_senders: Option<Vec<u64>>,
    },
    /// Response: [`QueryResponsePayload::GetSlackConfig`].
    GetSlackConfig,
    /// `bot_token`/`app_token`: `None` means leave that token untouched
    /// (each rotatable independently). Responds with
    /// [`QueryResponsePayload::SlackConfigApplied`].
    SetSlackConfig {
        #[serde(default)]
        bot_token: Option<String>,
        #[serde(default)]
        app_token: Option<String>,
        #[serde(default)]
        team_id: Option<String>,
        #[serde(default)]
        team_run_channel: Option<bool>,
        #[serde(default)]
        team_trigger_rate_limit: Option<u32>,
        #[serde(default)]
        team_command_allowed_senders: Option<Vec<String>>,
    },
```

In `QueryResponsePayload`, add all 8 matching variants:

```rust
    GetEmailConfig { config: EmailConfigView },
    EmailConfigApplied { config: EmailConfigView, restart_required: bool },
    GetTelegramConfig { config: TelegramConfigView },
    TelegramConfigApplied { config: TelegramConfigView, restart_required: bool },
    GetDiscordConfig { config: DiscordConfigView },
    DiscordConfigApplied { config: DiscordConfigView, restart_required: bool },
    GetSlackConfig { config: SlackConfigView },
    SlackConfigApplied { config: SlackConfigView, restart_required: bool },
```

- [ ] **Step 2: Round-trip tests**

```rust
    #[test]
    fn email_config_view_round_trips_with_redacted_password() {
        let view = EmailConfigView {
            host: Some("smtp.example.com".to_string()),
            port: Some(587),
            tls_mode: Some("starttls".to_string()),
            username: Some("bot@example.com".to_string()),
            password: RedactedSecret { configured: true, source: "toml".to_string() },
            from: Some("bot@example.com".to_string()),
        };
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("hunter2"), "no real secret value in the type at all, sanity check on the test itself");
        let back: EmailConfigView = serde_json::from_str(&json).unwrap();
        assert_eq!(back, view);
    }

    #[test]
    fn set_slack_config_round_trips() {
        let msg = QueryPayload::SetSlackConfig {
            bot_token: Some("xoxb-1".to_string()),
            app_token: None,
            team_id: None,
            team_run_channel: None,
            team_trigger_rate_limit: None,
            team_command_allowed_senders: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: QueryPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }
```

- [ ] **Step 3: Verify**

```bash
cargo test -p aivyx-ipc email_config_view_round_trips_with_redacted_password set_slack_config_round_trips
cargo test -p aivyx-ipc
cargo clippy -p aivyx-ipc --all-targets -- -D warnings
```
Expected: all green.

- [ ] **Step 4: Commit**

```bash
git add crates/aivyx-ipc/src/protocol.rs
git commit -m "feat(ipc): wire types for [email]/[telegram]/[discord]/[slack] config

RedactedSecret gets its first real consumers: EmailConfigView.password,
Telegram/DiscordConfigView.token, SlackConfigView.{bot_token,app_token}.
Set* payloads use Option<T> per field for the partial-update
convention; Get responses always carry concrete (non-Option)
non-secret fields."
```

---

## Task 5: `[email]`/`[telegram]`/`[discord]`/`[slack]` daemon handlers

**Files:**
- Modify: `crates/aivyx-channel/src/daemon_server.rs`

**Interfaces:**
- Consumes: Task 2's `aivyx_config::config_write` functions; Task 4's wire types.
- Produces: nothing further tasks in this plan consume.

This is the layer where redaction actually happens — converting a raw `Option<String>` secret from `aivyx-config` into a `RedactedSecret` the wire never carries the real value on.

- [ ] **Step 1: Add a shared redaction helper**

Near `map_config_write_error`/`audit_config_change` in `crates/aivyx-channel/src/daemon_server.rs`:

```rust
/// Build a `RedactedSecret` from a raw value read straight off disk (never
/// through the interpolating loader — see `read_mcp_server_entries`'s own
/// doc comment for why that distinction matters). `source` is always
/// `"toml"`: this read path never resolves an env-var fallback, so `"toml"`
/// is the only value it could ever truthfully report.
fn redact(raw: Option<&str>) -> aivyx_ipc::protocol::RedactedSecret {
    aivyx_ipc::protocol::RedactedSecret {
        configured: raw.is_some_and(|s| !s.is_empty()),
        source: "toml".to_string(),
    }
}
```

- [ ] **Step 2: Add the 8 handlers**

Add near the notify-target handlers from Task 3:

```rust
        QueryPayload::GetEmailConfig => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            match aivyx_config::config_write::read_email_section(path) {
                Ok(e) => QueryResponsePayload::GetEmailConfig { config: email_config_view(&e) },
                Err(err) => map_config_write_error(err),
            }
        }
        QueryPayload::SetEmailConfig { host, port, tls_mode, username, password, from } => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            let entry = aivyx_config::config_write::EmailEntryWrite { host, port, tls_mode, username, password, from };
            match aivyx_config::config_write::write_email_section(path, &entry) {
                Ok(()) => {
                    audit_config_change(audit_log, "email", "updated");
                    match aivyx_config::config_write::read_email_section(path) {
                        Ok(e) => QueryResponsePayload::EmailConfigApplied {
                            config: email_config_view(&e),
                            restart_required: true,
                        },
                        Err(err) => QueryResponsePayload::QueryError {
                            code: "config_reload_failed".into(),
                            message: format!("email config saved, but reloading it failed: {err}"),
                        },
                    }
                }
                Err(e) => map_config_write_error(e),
            }
        }
        QueryPayload::GetTelegramConfig => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            match aivyx_config::config_write::read_telegram_section(path) {
                Ok(t) => QueryResponsePayload::GetTelegramConfig { config: telegram_config_view(&t) },
                Err(err) => map_config_write_error(err),
            }
        }
        QueryPayload::SetTelegramConfig { token, chat_id, team_run_channel, team_trigger_rate_limit, team_command_allowed_senders } => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            let entry = aivyx_config::config_write::TelegramEntryWrite {
                token, chat_id, team_run_channel, team_trigger_rate_limit, team_command_allowed_senders,
            };
            match aivyx_config::config_write::write_telegram_section(path, &entry) {
                Ok(()) => {
                    audit_config_change(audit_log, "telegram", "updated");
                    match aivyx_config::config_write::read_telegram_section(path) {
                        Ok(t) => QueryResponsePayload::TelegramConfigApplied {
                            config: telegram_config_view(&t),
                            restart_required: true,
                        },
                        Err(err) => QueryResponsePayload::QueryError {
                            code: "config_reload_failed".into(),
                            message: format!("telegram config saved, but reloading it failed: {err}"),
                        },
                    }
                }
                Err(e) => map_config_write_error(e),
            }
        }
        QueryPayload::GetDiscordConfig => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            match aivyx_config::config_write::read_discord_section(path) {
                Ok(d) => QueryResponsePayload::GetDiscordConfig { config: discord_config_view(&d) },
                Err(err) => map_config_write_error(err),
            }
        }
        QueryPayload::SetDiscordConfig { token, application_id, team_run_channel, team_trigger_rate_limit, team_command_allowed_senders } => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            let entry = aivyx_config::config_write::DiscordEntryWrite {
                token, application_id, team_run_channel, team_trigger_rate_limit, team_command_allowed_senders,
            };
            match aivyx_config::config_write::write_discord_section(path, &entry) {
                Ok(()) => {
                    audit_config_change(audit_log, "discord", "updated");
                    match aivyx_config::config_write::read_discord_section(path) {
                        Ok(d) => QueryResponsePayload::DiscordConfigApplied {
                            config: discord_config_view(&d),
                            restart_required: true,
                        },
                        Err(err) => QueryResponsePayload::QueryError {
                            code: "config_reload_failed".into(),
                            message: format!("discord config saved, but reloading it failed: {err}"),
                        },
                    }
                }
                Err(e) => map_config_write_error(e),
            }
        }
        QueryPayload::GetSlackConfig => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            match aivyx_config::config_write::read_slack_section(path) {
                Ok(s) => QueryResponsePayload::GetSlackConfig { config: slack_config_view(&s) },
                Err(err) => map_config_write_error(err),
            }
        }
        QueryPayload::SetSlackConfig { bot_token, app_token, team_id, team_run_channel, team_trigger_rate_limit, team_command_allowed_senders } => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            let entry = aivyx_config::config_write::SlackEntryWrite {
                bot_token, app_token, team_id, team_run_channel, team_trigger_rate_limit, team_command_allowed_senders,
            };
            match aivyx_config::config_write::write_slack_section(path, &entry) {
                Ok(()) => {
                    audit_config_change(audit_log, "slack", "updated");
                    match aivyx_config::config_write::read_slack_section(path) {
                        Ok(s) => QueryResponsePayload::SlackConfigApplied {
                            config: slack_config_view(&s),
                            restart_required: true,
                        },
                        Err(err) => QueryResponsePayload::QueryError {
                            code: "config_reload_failed".into(),
                            message: format!("slack config saved, but reloading it failed: {err}"),
                        },
                    }
                }
                Err(e) => map_config_write_error(e),
            }
        }
```

- [ ] **Step 3: Add the 4 view-builder helpers**

Near `redact` (Step 1):

```rust
fn email_config_view(e: &aivyx_config::config_write::EmailEntryWrite) -> aivyx_ipc::protocol::EmailConfigView {
    aivyx_ipc::protocol::EmailConfigView {
        host: e.host.clone(),
        port: e.port,
        tls_mode: e.tls_mode.clone(),
        username: e.username.clone(),
        password: redact(e.password.as_deref()),
        from: e.from.clone(),
    }
}

fn telegram_config_view(t: &aivyx_config::config_write::TelegramEntryWrite) -> aivyx_ipc::protocol::TelegramConfigView {
    aivyx_ipc::protocol::TelegramConfigView {
        token: redact(t.token.as_deref()),
        chat_id: t.chat_id,
        team_run_channel: t.team_run_channel.unwrap_or(false),
        team_trigger_rate_limit: t.team_trigger_rate_limit,
        team_command_allowed_senders: t.team_command_allowed_senders.clone().unwrap_or_default(),
    }
}

fn discord_config_view(d: &aivyx_config::config_write::DiscordEntryWrite) -> aivyx_ipc::protocol::DiscordConfigView {
    aivyx_ipc::protocol::DiscordConfigView {
        token: redact(d.token.as_deref()),
        application_id: d.application_id,
        team_run_channel: d.team_run_channel.unwrap_or(false),
        team_trigger_rate_limit: d.team_trigger_rate_limit,
        team_command_allowed_senders: d.team_command_allowed_senders.clone().unwrap_or_default(),
    }
}

fn slack_config_view(s: &aivyx_config::config_write::SlackEntryWrite) -> aivyx_ipc::protocol::SlackConfigView {
    aivyx_ipc::protocol::SlackConfigView {
        bot_token: redact(s.bot_token.as_deref()),
        app_token: redact(s.app_token.as_deref()),
        team_id: s.team_id.clone(),
        team_run_channel: s.team_run_channel.unwrap_or(false),
        team_trigger_rate_limit: s.team_trigger_rate_limit,
        team_command_allowed_senders: s.team_command_allowed_senders.clone().unwrap_or_default(),
    }
}
```

Note: `TelegramEntryWrite`/`DiscordEntryWrite`'s `read_*_section` functions (Task 2) always return `Some(false)`-shaped data as `None` when the key is absent (matching every other field's "absent = None" reading convention) — so `.unwrap_or(false)` here is exactly right: an absent `team_run_channel` key reads as `false`, matching the loader's own `#[serde(default)]` (bool defaults to `false`).

- [ ] **Step 4: Route `QueryError`s to a shared UI notice**

In `crates/aivyx-web/src/main.rs`'s `read_task`, find the existing `id.starts_with("mc-mcp")` routing arm (added in plan 1) and add sibling ids for this plan's new queries — but note these queries don't have per-screen ids assigned yet (that happens in Task 6's Studio work). Skip this step here; Task 6 owns wiring the ids and the routing arm together, since the id strings and the UI state field they route to are decided there.

- [ ] **Step 5: Verify**

```bash
cargo build -p aivyx-channel
cargo clippy -p aivyx-channel --all-targets -- -D warnings
cargo test -p aivyx-config -p aivyx-ipc -p aivyx-channel
```
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-channel/src/daemon_server.rs
git commit -m "feat(channel): daemon handlers for email/telegram/discord/slack config

redact() is the one place a raw on-disk secret ever gets converted to
RedactedSecret before crossing the wire. Each Set handler re-reads
the fresh state after a successful write, surfacing a reload failure
as QueryError rather than silently claiming a blank config."
```

---

## Task 6: Studio — notify-target CRUD form

**Files:**
- Modify: `crates/aivyx-web/src/main.rs`

**Interfaces:**
- Consumes: `QueryPayload::{GetNotifyTargetConfigs,SetNotifyTarget,DeleteNotifyTarget}`/`QueryResponsePayload::{GetNotifyTargetConfigs,NotifyTargetsApplied}` (Task 3).
- Produces: `struct NotifyConfigUi { notice: Option<(bool, String)> }` (mirrors `McpConfigUi`'s exact shape from plan 1), threaded through `App`/`ws_task`/`read_task` the same way — find `mcp_config_ui`'s own 4 threading points (plan 1's `App`'s `use_signal`+`use_context_provider`, `ws_task`'s signature + its `spawn(read_task(...))` call, `read_task`'s signature) and copy that exact pattern (not `server_info`'s — plan 1's own final review found `server_info` has no `use_context_provider` and isn't the right template for a signal read from a component other than `App`; `mcp_config_ui` is read from `McpPanel`, a separate component, exactly like `notify_config_ui` will be read from `NotificationsPanel`).

- [ ] **Step 1: Add `NotifyConfigUi` + thread it**

```rust
/// POLISH_WAVES.md sub-project 7 plan 2 — notify-target/channel-adapter
/// config-write UI state, mirroring `McpConfigUi`'s own minimal shape.
#[derive(Clone, Default, PartialEq)]
struct NotifyConfigUi {
    notice: Option<(bool, String)>,
}
```

Thread `notify_config_ui: Signal<NotifyConfigUi>` through `App`/`ws_task`/`read_task` following `mcp_config_ui`'s exact 4 edit points.

- [ ] **Step 2: Add query builders + response handling**

```rust
fn notify_target_configs_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-notify-configs".to_string(),
        payload: QueryPayload::GetNotifyTargetConfigs,
    }
}

fn set_notify_target_query(target: NotifyTargetConfigView) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-notify-set".to_string(),
        payload: QueryPayload::SetNotifyTarget {
            name: target.name,
            kind: target.kind,
            chat_id: target.chat_id,
            url: target.url,
            to: target.to,
            enabled: target.enabled,
            is_default: target.is_default,
            retry_count: target.retry_count,
            retry_backoff_ms_start: target.retry_backoff_ms_start,
            rate_limit_max: target.rate_limit_max,
            rate_limit_window_secs: target.rate_limit_window_secs,
        },
    }
}

fn delete_notify_target_query(name: String) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-notify-delete".to_string(),
        payload: QueryPayload::DeleteNotifyTarget { name },
    }
}
```

In `read_task`'s `match env { ... }` (before the final `_ => {}`):

```rust
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetNotifyTargetConfigs { targets },
                    ..
                } => {
                    notifications.write().configs = targets;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::NotifyTargetsApplied { targets, .. },
                    ..
                } => {
                    notifications.write().configs = targets;
                    notify_config_ui.write().notice = Some((true, "Saved — restart the daemon to apply.".to_string()));
                }
```

(`read_task`'s existing `NotificationsState` signal parameter is named `notifications` — confirmed directly in the file, matching how `GetNotifyTargets`'s existing handler already writes into it.)

Add `configs: Vec<NotifyTargetConfigView>` to `NotificationsState` (find its struct definition, add the field the same way plan 1 added `configs: Vec<McpServerConfigView>` to `McpState`).

Add the `QueryError` routing arm (deferred from Task 5):

```rust
                DaemonEnvelope::QueryResponse {
                    id,
                    payload: QueryResponsePayload::QueryError { message, .. },
                } if id.starts_with("mc-notify") => {
                    notify_config_ui.write().notice = Some((false, message));
                }
```

Place this arm alongside the existing `id.starts_with("mc-mcp")` arm from plan 1 (same match, same style).

- [ ] **Step 3: Extend `NotificationsPanel` with notify-target CRUD**

Add to `NotificationsPanel`'s existing "Targets" rail section (`crates/aivyx-web/src/main.rs`, the `aside { class: "dash-rail", section { class: "panel", div { class: "panel-head", h3 { "Targets" } } ... } }` block) — keep the existing read-only Targets list exactly as-is for the live-status view, and add a new, separate "Configure targets" section below it in `dash-main` (alongside "Notification history"), following the exact same list+form+Add/Edit/Delete structure `McpPanel`'s "Configured servers" section already established in plan 1 (`McpServerCard`-style list rows with Edit/Delete buttons, a form component shown when adding/editing, using a `key:` on the form derived from the editing target's name — plan 1's final review found and fixed a real bug from a missing `key:` here, so include it from the start this time):

```rust
#[component]
fn NotifyTargetForm(
    initial: Option<NotifyTargetConfigView>,
    on_cancel: EventHandler<()>,
    on_save: EventHandler<NotifyTargetConfigView>,
) -> Element {
    let seed = initial.clone().unwrap_or(NotifyTargetConfigView {
        name: String::new(),
        kind: "telegram".to_string(),
        chat_id: None,
        url: None,
        to: None,
        enabled: true,
        is_default: false,
        retry_count: 0,
        retry_backoff_ms_start: 500,
        rate_limit_max: None,
        rate_limit_window_secs: None,
    });
    let editing_existing = initial.is_some();
    let mut name = use_signal(|| seed.name.clone());
    let mut kind = use_signal(|| seed.kind.clone());
    let mut chat_id = use_signal(|| seed.chat_id.clone().unwrap_or_default());
    let mut url = use_signal(|| seed.url.clone().unwrap_or_default());
    let mut to = use_signal(|| seed.to.clone().unwrap_or_default());
    let mut enabled = use_signal(|| seed.enabled);
    let mut is_default = use_signal(|| seed.is_default);

    rsx! {
        div { class: "glass-card",
            div { class: "field-row",
                label { "Name" }
                input { class: "input", value: "{name}", disabled: editing_existing, oninput: move |e| name.set(e.value()) }
            }
            div { class: "field-row",
                label { "Kind" }
                select { class: "input", value: "{kind}", onchange: move |e| kind.set(e.value()),
                    option { value: "telegram", "telegram" }
                    option { value: "webhook", "webhook" }
                    option { value: "email", "email" }
                    option { value: "web-ui", "web-ui" }
                }
            }
            if kind() == "telegram" {
                div { class: "field-row",
                    label { "Chat ID" }
                    input { class: "input", value: "{chat_id}", oninput: move |e| chat_id.set(e.value()) }
                }
            } else if kind() == "webhook" {
                div { class: "field-row",
                    label { "URL" }
                    input { class: "input", value: "{url}", oninput: move |e| url.set(e.value()) }
                }
            } else if kind() == "email" {
                div { class: "field-row",
                    label { "To" }
                    input { class: "input", value: "{to}", oninput: move |e| to.set(e.value()) }
                }
            }
            div { class: "field-row",
                label { "Enabled" }
                input { r#type: "checkbox", checked: enabled(), onchange: move |e| enabled.set(e.checked()) }
            }
            div { class: "field-row",
                label { "Default target" }
                input { r#type: "checkbox", checked: is_default(), onchange: move |e| is_default.set(e.checked()) }
            }
            div { style: "display:flex; gap:8px; margin-top:12px;",
                button {
                    class: "btn btn-primary btn-xs",
                    onclick: move |_| {
                        let k = kind();
                        let entry = NotifyTargetConfigView {
                            name: name().trim().to_string(),
                            kind: k.clone(),
                            chat_id: if k == "telegram" && !chat_id().trim().is_empty() { Some(chat_id().trim().to_string()) } else { None },
                            url: if k == "webhook" && !url().trim().is_empty() { Some(url().trim().to_string()) } else { None },
                            to: if k == "email" && !to().trim().is_empty() { Some(to().trim().to_string()) } else { None },
                            enabled: enabled(),
                            is_default: is_default(),
                            retry_count: seed.retry_count,
                            retry_backoff_ms_start: seed.retry_backoff_ms_start,
                            rate_limit_max: seed.rate_limit_max,
                            rate_limit_window_secs: seed.rate_limit_window_secs,
                        };
                        on_save.call(entry);
                    },
                    "Save"
                }
                button { class: "btn btn-glass btn-xs", onclick: move |_| on_cancel.call(()), "Cancel" }
            }
        }
    }
}
```

Wire it into `NotificationsPanel` with the same `editing`/`adding` signal pattern and `key:` fix `McpPanel` uses (`key: "{editing().map(|e| e.name.clone()).unwrap_or_else(|| \"new\".to_string())}"` on the form call site), calling `ws.send(set_notify_target_query(entry))` on save and `ws.send(delete_notify_target_query(name))` on delete, and `ws.send(notify_target_configs_query())` in a `use_future` on panel open (alongside the existing target-status query this panel already sends).

- [ ] **Step 4: Verify**

```bash
cargo build -p aivyx-web --target wasm32-unknown-unknown
cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings
```
Expected: both green. No new unit tests for this task (forms/rendering).

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "feat(web): notify-target add/edit/remove form in Studio

NotificationsPanel gains a 'Configure targets' section below the
existing, untouched read-only Targets rail. Form uses a key: derived
from the editing target's name from the start, closing plan 1's
McpServerForm remount bug before it could recur here."
```

---

## Task 7: Studio — channel-adapter forms (email/telegram/discord/slack)

**Files:**
- Modify: `crates/aivyx-web/src/main.rs`

**Interfaces:**
- Consumes: the 8 `QueryPayload`/`QueryResponsePayload` variants from Task 4, the daemon handlers from Task 5.
- Produces: nothing further tasks consume.

- [ ] **Step 1: Query builders**

```rust
fn get_email_config_query() -> FrontendMessage {
    FrontendMessage::Query { id: "mc-notify-channels".to_string(), payload: QueryPayload::GetEmailConfig }
}
fn set_email_config_query(host: Option<String>, port: Option<u16>, tls_mode: Option<String>, username: Option<String>, password: Option<String>, from: Option<String>) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-notify-channels".to_string(),
        payload: QueryPayload::SetEmailConfig { host, port, tls_mode, username, password, from },
    }
}
fn get_telegram_config_query() -> FrontendMessage {
    FrontendMessage::Query { id: "mc-notify-channels".to_string(), payload: QueryPayload::GetTelegramConfig }
}
fn set_telegram_config_query(token: Option<String>, chat_id: Option<i64>) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-notify-channels".to_string(),
        payload: QueryPayload::SetTelegramConfig { token, chat_id, team_run_channel: None, team_trigger_rate_limit: None, team_command_allowed_senders: None },
    }
}
fn get_discord_config_query() -> FrontendMessage {
    FrontendMessage::Query { id: "mc-notify-channels".to_string(), payload: QueryPayload::GetDiscordConfig }
}
fn set_discord_config_query(token: Option<String>, application_id: Option<u64>) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-notify-channels".to_string(),
        payload: QueryPayload::SetDiscordConfig { token, application_id, team_run_channel: None, team_trigger_rate_limit: None, team_command_allowed_senders: None },
    }
}
fn get_slack_config_query() -> FrontendMessage {
    FrontendMessage::Query { id: "mc-notify-channels".to_string(), payload: QueryPayload::GetSlackConfig }
}
fn set_slack_config_query(bot_token: Option<String>, app_token: Option<String>, team_id: Option<String>) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-notify-channels".to_string(),
        payload: QueryPayload::SetSlackConfig { bot_token, app_token, team_id, team_run_channel: None, team_trigger_rate_limit: None, team_command_allowed_senders: None },
    }
}
```

(These query builders only expose the fields the Task 7 forms edit — `team_run_channel`/`team_trigger_rate_limit`/`team_command_allowed_senders` stay TOML-only for now, matching the design spec's scope for this pass; a future pass can extend these forms and add the remaining parameters without changing the wire types, which already carry them.)

All 8 queries share the id `"mc-notify-channels"`, which the `id.starts_with("mc-notify")` `QueryError` routing arm Task 6 already added catches by prefix — a rejected save on any of the 4 channel forms lands on the same `notify_config_ui` notice banner Task 6's notify-target form uses. No separate error-routing arm is needed in this task.

- [ ] **Step 2: State + response handling**

Add 4 new fields to `NotificationsState` (the same struct Task 6 added `configs` to): `email: Option<EmailConfigView>`, `telegram: Option<TelegramConfigView>`, `discord: Option<DiscordConfigView>`, `slack: Option<SlackConfigView>`.

In `read_task`, handle all 8 responses (before the final `_ => {}`):

```rust
                DaemonEnvelope::QueryResponse { payload: QueryResponsePayload::GetEmailConfig { config }, .. }
                | DaemonEnvelope::QueryResponse { payload: QueryResponsePayload::EmailConfigApplied { config, .. }, .. } => {
                    notifications.write().email = Some(config);
                }
                DaemonEnvelope::QueryResponse { payload: QueryResponsePayload::GetTelegramConfig { config }, .. }
                | DaemonEnvelope::QueryResponse { payload: QueryResponsePayload::TelegramConfigApplied { config, .. }, .. } => {
                    notifications.write().telegram = Some(config);
                }
                DaemonEnvelope::QueryResponse { payload: QueryResponsePayload::GetDiscordConfig { config }, .. }
                | DaemonEnvelope::QueryResponse { payload: QueryResponsePayload::DiscordConfigApplied { config, .. }, .. } => {
                    notifications.write().discord = Some(config);
                }
                DaemonEnvelope::QueryResponse { payload: QueryResponsePayload::GetSlackConfig { config }, .. }
                | DaemonEnvelope::QueryResponse { payload: QueryResponsePayload::SlackConfigApplied { config, .. }, .. } => {
                    notifications.write().slack = Some(config);
                }
```

(If Rust's or-pattern syntax across two differently-shaped `DaemonEnvelope::QueryResponse` variants doesn't compile as written above — the `payload` field's enum variant differs between the two arms of each `|`, which needs identical binding names/types on both sides, and both DO bind `config: XConfigView` here so this should work, but confirm by compiling — if it doesn't, split each into two separate match arms with identical bodies instead.)

- [ ] **Step 3: Add a "Channel adapters" section to `NotificationsPanel`**

Add below the "Configure targets" section from Task 6, one card per channel with a masked-secret field pattern: show `"configured"`/`"not set"` as a label (never an editable pre-filled value) plus a text input labeled "New token (leave blank to keep current)":

```rust
#[component]
fn ChannelAdaptersSection(state: NotificationsState) -> Element {
    let ws = use_context::<Sender>();
    let mut email_password = use_signal(String::new);
    let mut telegram_token = use_signal(String::new);
    let mut discord_token = use_signal(String::new);
    let mut slack_bot_token = use_signal(String::new);
    let mut slack_app_token = use_signal(String::new);

    rsx! {
        section { class: "panel",
            div { class: "panel-head", h3 { "Channel adapters" } }
            if let Some(email) = &state.email {
                div { class: "glass-card",
                    h4 { "Email (SMTP)" }
                    p { class: "label-tech",
                        {if email.password.configured { "Password: configured" } else { "Password: not set" }}
                    }
                    input { class: "input", placeholder: "New password (leave blank to keep current)",
                        r#type: "password", value: "{email_password}",
                        oninput: move |e| email_password.set(e.value()) }
                    button {
                        class: "btn btn-primary btn-xs",
                        onclick: move |_| {
                            let pw = email_password();
                            ws.send(set_email_config_query(
                                None, None, None, None,
                                if pw.trim().is_empty() { None } else { Some(pw.trim().to_string()) },
                                None,
                            ));
                            email_password.set(String::new());
                        },
                        "Save"
                    }
                }
            }
            if let Some(tg) = &state.telegram {
                div { class: "glass-card",
                    h4 { "Telegram" }
                    p { class: "label-tech",
                        {if tg.token.configured { "Token: configured" } else { "Token: not set" }}
                    }
                    input { class: "input", placeholder: "New bot token (leave blank to keep current)",
                        r#type: "password", value: "{telegram_token}",
                        oninput: move |e| telegram_token.set(e.value()) }
                    button {
                        class: "btn btn-primary btn-xs",
                        onclick: move |_| {
                            let t = telegram_token();
                            ws.send(set_telegram_config_query(
                                if t.trim().is_empty() { None } else { Some(t.trim().to_string()) },
                                None,
                            ));
                            telegram_token.set(String::new());
                        },
                        "Save"
                    }
                }
            }
            if let Some(d) = &state.discord {
                div { class: "glass-card",
                    h4 { "Discord" }
                    p { class: "label-tech",
                        {if d.token.configured { "Token: configured" } else { "Token: not set" }}
                    }
                    input { class: "input", placeholder: "New bot token (leave blank to keep current)",
                        r#type: "password", value: "{discord_token}",
                        oninput: move |e| discord_token.set(e.value()) }
                    button {
                        class: "btn btn-primary btn-xs",
                        onclick: move |_| {
                            let t = discord_token();
                            ws.send(set_discord_config_query(
                                if t.trim().is_empty() { None } else { Some(t.trim().to_string()) },
                                None,
                            ));
                            discord_token.set(String::new());
                        },
                        "Save"
                    }
                }
            }
            if let Some(s) = &state.slack {
                div { class: "glass-card",
                    h4 { "Slack" }
                    p { class: "label-tech",
                        {if s.bot_token.configured { "Bot token: configured" } else { "Bot token: not set" }}
                    }
                    input { class: "input", placeholder: "New bot token (leave blank to keep current)",
                        r#type: "password", value: "{slack_bot_token}",
                        oninput: move |e| slack_bot_token.set(e.value()) }
                    p { class: "label-tech",
                        {if s.app_token.configured { "App token: configured" } else { "App token: not set" }}
                    }
                    input { class: "input", placeholder: "New app token (leave blank to keep current)",
                        r#type: "password", value: "{slack_app_token}",
                        oninput: move |e| slack_app_token.set(e.value()) }
                    button {
                        class: "btn btn-primary btn-xs",
                        onclick: move |_| {
                            let bt = slack_bot_token();
                            let at = slack_app_token();
                            ws.send(set_slack_config_query(
                                if bt.trim().is_empty() { None } else { Some(bt.trim().to_string()) },
                                if at.trim().is_empty() { None } else { Some(at.trim().to_string()) },
                                None,
                            ));
                            slack_bot_token.set(String::new());
                            slack_app_token.set(String::new());
                        },
                        "Save"
                    }
                }
            }
        }
    }
}
```

Call `ChannelAdaptersSection { state: n() }` from `NotificationsPanel`'s render — confirmed directly in the file: `NotificationsPanel` itself (`main.rs:2083-2085`) binds its context as `let n = use_context::<Signal<NotificationsState>>(); let state = n();`, a DIFFERENT local name from `read_task`'s own `notifications` parameter used in Step 2 above (two different scopes, two different existing names — use whichever is correct for the scope you're editing, not the same name in both). Send the 4 `get_*_config_query()` calls in the panel's existing `use_future` (alongside the target-status and target-configs queries).

- [ ] **Step 4: Verify**

```bash
cargo build -p aivyx-web --target wasm32-unknown-unknown
cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings
```
Expected: both green.

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "feat(web): channel adapter (email/telegram/discord/slack) forms in Studio

Each secret field shows only 'configured'/'not set', never a real
value; a blank input on Save means 'don't change this token' —
the same never-round-trip convention plan 1 established for the
config-write architecture, now exercised by its first real secrets."
```

---

## Task 8: Full workspace sweep + `dist/` rebuild

**Files:** none (verification + build artifact only).

- [ ] **Step 1: Full workspace sweep**

```bash
export PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH"
cargo clippy --workspace --exclude aivyx-desktop --all-targets -- -D warnings
cargo test --workspace --exclude aivyx-desktop
```
Expected: zero warnings, zero failures.

- [ ] **Step 2: Rebuild and replace `dist/`**

```bash
cd crates/aivyx-web && dx bundle --release --platform web && cd ../..
rm -rf crates/aivyx-web/dist && mkdir -p crates/aivyx-web/dist
cp -r target/dx/aivyx-web/release/web/public/. crates/aivyx-web/dist/
find crates/aivyx-web/dist -name '*.br' -delete
git status --porcelain crates/aivyx-web/dist/assets/ | grep wasm
```
Expected: exactly one `A ` line and one `D ` line (a clean rename).

- [ ] **Step 3: Commit**

```bash
git add crates/aivyx-web/dist/
git commit -m "chore(web): rebuild dist/ bundle for notify-target + channel-adapter CRUD (sub-project 7, plan 2)"
```

This is the final task — once this sweep is clean, proceed to the final whole-branch review.

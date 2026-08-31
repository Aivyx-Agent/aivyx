# Config-write architecture + MCP CRUD Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the shared array-of-table config-write primitive (item A) and the first real consumer, MCP full CRUD with a test-connection probe (item B), per `docs/superpowers/specs/2026-08-31-config-write-surface-design.md`.

**Architecture:** Extend `aivyx-config/src/config_write.rs` (Chapter U's proven `load_document → patch in place → write_toml_0600` recipe) with array-of-table helpers for `[[mcp_server]]`. Add typed `QueryPayload::{GetMcpServerConfigs,SetMcpServer,DeleteMcpServer,TestMcpServerConnection}` wire messages — matching the existing `SetAccessLevel`/`SetBudget`/`SetAutonomyLevel`/`SetTeamRoster` convention (a `QueryPayload::SetX` write responds with a dedicated `QueryResponsePayload::XApplied { fresh_state, restart_required }`), not the separate `CreateSchedule`-style top-level-message convention the spec's own prose loosely gestured at. Studio's `McpPanel` gets a new "Configured servers" section (add/edit/remove + test-connection) alongside its existing, untouched, read-only live-status cards.

**Tech Stack:** `toml_edit` 0.22 (`ArrayOfTables`), the existing `aivyx-ipc`/`aivyx-channel` wire-protocol machinery, `aivyx-mcp`'s real `McpServerBridge`/transport connect calls for the probe, Dioxus 0.6 for the Studio form.

## Global Constraints

- Zero clippy warnings: `cargo clippy --workspace --exclude aivyx-desktop --all-targets -- -D warnings` must stay clean after every task.
- `cargo test --workspace --exclude aivyx-desktop` must stay green.
- `aivyx-web`'s own `#[cfg(test)]` tests run via plain `cargo test -p aivyx-web` — **no** `--target wasm32-unknown-unknown` flag (verified in a prior sub-project on this exact host: that flag produces an unexecutable `.wasm` test binary). Only `cargo build`/`cargo clippy` for `aivyx-web`/`aivyx-ipc` need `--target wasm32-unknown-unknown` (use `~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin` + `~/.cargo/bin` on `PATH` if the system `cargo` lacks the target).
- Secret-field convention (not exercised by MCP's own fields — see Task 6's note — but binding on the *shape* every later plan in this sub-project must match): a `GetX` response never sends a real secret value; a `SetX` write's secret field is `Option<String>` where `None` means "leave the existing TOML value untouched."
- Every config write in this plan requires a daemon restart to take effect (MCP servers are constructed once at boot) — every write response carries `restart_required: true` and the Studio UI shows the existing `.restart-banner` treatment.
- `[[mcp_server]].sandbox`/`.bundled` are **not** exposed in this plan — internal/advanced, stay TOML-only.
- After the final task, rebuild and commit `dist/` per this repo's established convention: `rm -rf dist && cp -r <dx-bundle-output> dist` (never a merging copy), strip `.br` files, verify via `git status --porcelain dist/assets/ | grep wasm` showing exactly one add + one delete.

---

## Task 1: `ConfigWriteError::InvalidMcpServer` + the array-of-table write primitive

**Files:**
- Modify: `crates/aivyx-config/src/config_write.rs`

**Interfaces:**
- Produces: `ConfigWriteError::InvalidMcpServer { reason: String }` (a new enum variant); `pub fn write_mcp_server_section(path: &Path, server: &McpServerEntryWrite) -> Result<(), ConfigWriteError>` (upsert-by-`name`); `pub fn remove_mcp_server_section(path: &Path, name: &str) -> Result<(), ConfigWriteError>`; `pub struct McpServerEntryWrite { pub name: String, pub transport: String, pub command: Option<String>, pub args: Vec<String>, pub env: Vec<(String, String)>, pub headers: Vec<(String, String)>, pub url: Option<String>, pub enabled: bool }` (a plain, non-wire struct local to `aivyx-config` — `aivyx-ipc`'s wire type in Task 3 is a separate, serde-derived type; `aivyx-channel`'s daemon handler in Task 4 converts between them).

- [ ] **Step 1: Write the failing tests**

Add to `crates/aivyx-config/src/config_write.rs`'s existing `#[cfg(test)] mod tests` block (after the existing `access_writes_level_confirm_and_drops_stale_root`-style tests — same file, same module, just append):

```rust
    fn stdio_entry(name: &str) -> McpServerEntryWrite {
        McpServerEntryWrite {
            name: name.to_string(),
            transport: "stdio".to_string(),
            command: Some("npx".to_string()),
            args: vec!["-y".to_string(), "some-server".to_string()],
            env: vec![("TOKEN".to_string(), "${GITHUB_TOKEN}".to_string())],
            headers: Vec::new(),
            url: None,
            enabled: true,
        }
    }

    #[test]
    fn mcp_server_write_adds_a_new_entry() {
        let path = temp_toml("mcp-add");
        std::fs::write(&path, "[access]\nlevel = \"sandbox\"\n").unwrap();
        write_mcp_server_section(&path, &stdio_entry("github")).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("[access]"), "unrelated section survives");
        assert!(contents.contains("[[mcp_server]]"));
        assert!(contents.contains("name = \"github\""));
        assert!(contents.contains("command = \"npx\""));
    }

    #[test]
    fn mcp_server_write_replaces_an_existing_entry_by_name() {
        let path = temp_toml("mcp-replace");
        std::fs::write(&path, "").unwrap();
        write_mcp_server_section(&path, &stdio_entry("github")).unwrap();
        let mut updated = stdio_entry("github");
        updated.command = Some("uvx".to_string());
        write_mcp_server_section(&path, &updated).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        // Exactly one entry named "github" — not two.
        assert_eq!(contents.matches("name = \"github\"").count(), 1);
        assert!(contents.contains("command = \"uvx\""));
        assert!(!contents.contains("command = \"npx\""));
    }

    #[test]
    fn mcp_server_write_preserves_a_different_existing_entry() {
        let path = temp_toml("mcp-preserve");
        std::fs::write(&path, "").unwrap();
        write_mcp_server_section(&path, &stdio_entry("github")).unwrap();
        write_mcp_server_section(&path, &stdio_entry("filesystem")).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("name = \"github\""));
        assert!(contents.contains("name = \"filesystem\""));
    }

    #[test]
    fn mcp_server_write_rejects_stdio_without_command() {
        let path = temp_toml("mcp-stdio-no-cmd");
        std::fs::write(&path, "").unwrap();
        let mut entry = stdio_entry("github");
        entry.command = None;
        let err = write_mcp_server_section(&path, &entry).unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidMcpServer { .. }));
    }

    #[test]
    fn mcp_server_write_rejects_sse_without_url() {
        let path = temp_toml("mcp-sse-no-url");
        std::fs::write(&path, "").unwrap();
        let entry = McpServerEntryWrite {
            name: "remote".to_string(),
            transport: "sse".to_string(),
            command: None,
            args: Vec::new(),
            env: Vec::new(),
            headers: Vec::new(),
            url: None,
            enabled: true,
        };
        let err = write_mcp_server_section(&path, &entry).unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidMcpServer { .. }));
    }

    #[test]
    fn mcp_server_remove_drops_the_named_entry_only() {
        let path = temp_toml("mcp-remove");
        std::fs::write(&path, "").unwrap();
        write_mcp_server_section(&path, &stdio_entry("github")).unwrap();
        write_mcp_server_section(&path, &stdio_entry("filesystem")).unwrap();
        remove_mcp_server_section(&path, "github").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(!contents.contains("name = \"github\""));
        assert!(contents.contains("name = \"filesystem\""));
    }

    #[test]
    fn mcp_server_remove_of_unknown_name_is_a_harmless_no_op() {
        let path = temp_toml("mcp-remove-unknown");
        std::fs::write(&path, "").unwrap();
        write_mcp_server_section(&path, &stdio_entry("github")).unwrap();
        remove_mcp_server_section(&path, "does-not-exist").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("name = \"github\""));
    }
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p aivyx-config config_write::tests::mcp_server
```
Expected: FAIL — `write_mcp_server_section`/`remove_mcp_server_section`/`McpServerEntryWrite`/`ConfigWriteError::InvalidMcpServer` not defined.

- [ ] **Step 3: Add the error variant**

In `crates/aivyx-config/src/config_write.rs`, find the `ConfigWriteError` enum (its `RootRequired`/`RootNotAllowed`/`InvalidBudget`/`Parse`/`Io` variants) and add:

```rust
    /// An `[[mcp_server]]` entry is structurally invalid — mirrors the
    /// loader's own transport-field validation (`aivyx-config/src/lib.rs`'s
    /// mcp_servers parsing) so a bad write is refused before it corrupts
    /// the next daemon load, same principle as `InvalidBudget`.
    InvalidMcpServer { reason: String },
```

And its `Display` arm, right after `InvalidBudget`'s:

```rust
            ConfigWriteError::InvalidMcpServer { reason } => write!(f, "invalid MCP server entry: {reason}"),
```

- [ ] **Step 4: Add the array-of-table helpers**

Add to `crates/aivyx-config/src/config_write.rs`, after `write_voice_section`/its own private helpers (before the `#[cfg(test)]` module):

```rust
/// One `[[mcp_server]]` entry as Studio's write form submits it. A plain
/// (non-wire) struct — `aivyx-ipc` has its own serde-derived mirror type
/// for the wire; `aivyx-channel`'s daemon handler converts between them,
/// matching how `write_budget_section` takes `&aivyx_cost::BudgetConfig`
/// rather than a wire type directly.
pub struct McpServerEntryWrite {
    pub name: String,
    /// `"stdio"`, `"sse"`, or `"http"` — matches the loader's own accepted
    /// strings (`aivyx-config/src/lib.rs`'s mcp_servers parsing; that parser
    /// also accepts `"streamable-http"` as an alias for `"http"`, but writes
    /// always normalize to `"http"`).
    pub transport: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub url: Option<String>,
    pub enabled: bool,
}

/// Add or replace (by `name`) one `[[mcp_server]]` entry, preserving every
/// other entry, section, and the operator's comments. Validates the
/// transport-specific required field the same way the loader does
/// (`command` for stdio, `url` for sse/http) — refused here rather than
/// failing the next daemon load.
pub fn write_mcp_server_section(
    path: &Path,
    server: &McpServerEntryWrite,
) -> Result<(), ConfigWriteError> {
    if server.name.trim().is_empty() {
        return Err(ConfigWriteError::InvalidMcpServer {
            reason: "name must not be empty".to_string(),
        });
    }
    let transport_key = match server.transport.as_str() {
        "stdio" => "stdio",
        "sse" => "sse",
        "http" | "streamable-http" => "http",
        other => {
            return Err(ConfigWriteError::InvalidMcpServer {
                reason: format!(
                    "server {:?}: unknown transport {:?} (expected \"stdio\", \"sse\", or \"http\")",
                    server.name, other
                ),
            });
        }
    };
    if transport_key == "stdio" && server.command.is_none() {
        return Err(ConfigWriteError::InvalidMcpServer {
            reason: format!("server {:?}: stdio transport requires `command`", server.name),
        });
    }
    if transport_key != "stdio" && server.url.is_none() {
        return Err(ConfigWriteError::InvalidMcpServer {
            reason: format!("server {:?}: {transport_key} transport requires `url`", server.name),
        });
    }

    let mut doc = load_document(path)?;
    let arr = mcp_server_array_mut(&mut doc);

    let mut table = toml_edit::Table::new();
    table["name"] = value(server.name.as_str());
    table["transport"] = value(transport_key);
    table["enabled"] = value(server.enabled);
    if let Some(cmd) = &server.command {
        table["command"] = value(cmd.as_str());
    }
    if !server.args.is_empty() {
        let mut arr_val = toml_edit::Array::new();
        for a in &server.args {
            arr_val.push(a.as_str());
        }
        table["args"] = toml_edit::Item::Value(arr_val.into());
    }
    if !server.env.is_empty() {
        let mut env_table = toml_edit::InlineTable::new();
        for (k, v) in &server.env {
            env_table.insert(k, v.as_str().into());
        }
        table["env"] = toml_edit::Item::Value(env_table.into());
    }
    if !server.headers.is_empty() {
        let mut headers_table = toml_edit::InlineTable::new();
        for (k, v) in &server.headers {
            headers_table.insert(k, v.as_str().into());
        }
        table["headers"] = toml_edit::Item::Value(headers_table.into());
    }
    if let Some(url) = &server.url {
        table["url"] = value(url.as_str());
    }

    match arr.iter().position(|t| t.get("name").and_then(|v| v.as_str()) == Some(server.name.as_str())) {
        Some(i) => *arr.get_mut(i).expect("index just found") = table,
        None => arr.push(table),
    }

    write_toml_0600(path, &doc.to_string())
}

/// Remove one `[[mcp_server]]` entry by `name`. A no-op (not an error) when
/// no entry with that name exists — matches DELETE-idempotent semantics
/// used elsewhere in this codebase's IPC handlers.
pub fn remove_mcp_server_section(path: &Path, name: &str) -> Result<(), ConfigWriteError> {
    let mut doc = load_document(path)?;
    let arr = mcp_server_array_mut(&mut doc);
    if let Some(i) = arr.iter().position(|t| t.get("name").and_then(|v| v.as_str()) == Some(name)) {
        arr.remove(i);
    }
    write_toml_0600(path, &doc.to_string())
}

/// The `[[mcp_server]]` array, creating an empty one if the section is
/// absent from the document yet.
fn mcp_server_array_mut(doc: &mut DocumentMut) -> &mut toml_edit::ArrayOfTables {
    if doc.get("mcp_server").and_then(toml_edit::Item::as_array_of_tables).is_none() {
        doc["mcp_server"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    doc["mcp_server"]
        .as_array_of_tables_mut()
        .expect("just ensured present")
}
```

- [ ] **Step 5: Run to verify it passes**

```bash
cargo test -p aivyx-config config_write::tests::mcp_server
```
Expected: PASS (7 passed).

- [ ] **Step 6: Full-crate check**

```bash
cargo test -p aivyx-config
cargo clippy -p aivyx-config --all-targets -- -D warnings
```
Expected: all green, zero warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-config/src/config_write.rs
git commit -m "feat(config): array-of-table config-write primitive, [[mcp_server]] first

write_mcp_server_section/remove_mcp_server_section extend Chapter U's
load-patch-write recipe to array-of-tables (upsert-by-name, preserving
every other entry/section/comment). Validates the same transport-field
requirements the loader does. ConfigWriteError gains InvalidMcpServer."
```

---

## Task 2: `RedactedSecret` wire type (foundation for later plans in this sub-project)

**Files:**
- Modify: `crates/aivyx-ipc/src/protocol.rs`

**Interfaces:**
- Produces: `pub struct RedactedSecret { pub configured: bool, pub source: String }` — used by NO task in this plan (MCP's own fields aren't secrets — see Task 3's note), but is the shared type the Notify-target/channel-CRUD plan and the Settings-coverage plan both need for their secret fields, so it's built once here rather than duplicated per plan.

- [ ] **Step 1: Write the failing test**

Find `DaemonEnvelope`'s or a nearby small wire-type's own round-trip test in `crates/aivyx-ipc/src/protocol.rs` (e.g. search for `fn server_info_round_trips`) and add alongside it:

```rust
    #[test]
    fn redacted_secret_round_trips_without_the_real_value() {
        let s = RedactedSecret { configured: true, source: "toml".to_string() };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"configured\":true"));
        assert!(json.contains("\"source\":\"toml\""));
        let back: RedactedSecret = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p aivyx-ipc redacted_secret_round_trips
```
Expected: FAIL — `RedactedSecret` not defined.

- [ ] **Step 3: Add the type**

Add near the top of `crates/aivyx-ipc/src/protocol.rs`, alongside its other small shared wire structs (search for where `DocFile` or `AuditEntrySummary` is defined, and add nearby):

```rust
/// A secret field's wire representation on the READ side, for every
/// config-write consumer in POLISH_WAVES.md sub-project 7 (MCP env/header
/// values are NOT secrets by this codebase's convention — see
/// `McpServerConfigView`'s own doc comment — so this type's first real use
/// is the Notify-target/channel-adapter and Settings-coverage plans, not
/// this one). Never carries the real value: `configured` says whether a
/// value exists at all, `source` says where it came from (`"toml"`,
/// `"env"`, ...) — mirrors `aivyx_config::SourcedSecret`'s own `Debug`
/// redaction and its `FieldSource` provenance, on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RedactedSecret {
    pub configured: bool,
    pub source: String,
}
```

- [ ] **Step 4: Run to verify it passes**

```bash
cargo test -p aivyx-ipc redacted_secret_round_trips
```
Expected: PASS (1 passed).

- [ ] **Step 5: Full-crate check + commit**

```bash
cargo test -p aivyx-ipc
cargo clippy -p aivyx-ipc --all-targets -- -D warnings
git add crates/aivyx-ipc/src/protocol.rs
git commit -m "feat(ipc): RedactedSecret wire type for config-write secret fields

Shared foundation for every secret-carrying config-write consumer in
POLISH_WAVES.md sub-project 7 — built once here since this plan (MCP
CRUD) doesn't itself need it (env/header values are plain ${VAR}
placeholders by convention, not raw secrets)."
```

---

## Task 3: MCP config wire types + queries

**Files:**
- Modify: `crates/aivyx-ipc/src/protocol.rs`

**Interfaces:**
- Consumes: nothing from Tasks 1-2 directly (this task only adds wire types; Task 4 is what calls Task 1's `aivyx-config` functions).
- Produces: `pub struct McpServerConfigView { pub name: String, pub transport: String, pub command: Option<String>, pub args: Vec<String>, pub env: Vec<(String, String)>, pub headers: Vec<(String, String)>, pub url: Option<String>, pub enabled: bool }`; `QueryPayload::GetMcpServerConfigs`; `QueryPayload::SetMcpServer { name: String, transport: String, command: Option<String>, args: Vec<String>, env: Vec<(String, String)>, headers: Vec<(String, String)>, url: Option<String>, enabled: bool }`; `QueryPayload::DeleteMcpServer { name: String }`; `QueryResponsePayload::GetMcpServerConfigs { servers: Vec<McpServerConfigView> }`; `QueryResponsePayload::McpServersApplied { servers: Vec<McpServerConfigView>, restart_required: bool }`.

**Note on why MCP's own env/header values aren't treated as secrets:** the config schema's own doc comment on `McpServerConfig::env`/`::headers` (`crates/aivyx-config/src/lib.rs`) says `${VAR}` values are resolved from the daemon's own environment at load time "so secrets stay out of `aivyx.toml`" — the intended operator practice is to write a `${GITHUB_TOKEN}`-style placeholder, not a literal secret. `McpServerConfigView` sends these fields as plain strings; Task 6's form is a plain text field, not the redacted-secret UI.

- [ ] **Step 1: Add the config view type**

In `crates/aivyx-ipc/src/protocol.rs`, add near `McpServerStatusView`'s own definition:

```rust
/// The **editable configuration** of one `[[mcp_server]]` entry — distinct
/// from `McpServerStatusView` (the connection's live runtime status).
/// `env`/`headers` are NOT secrets on this wire type — see
/// `crates/aivyx-config/src/lib.rs`'s own doc comment on `McpServerConfig`:
/// the intended operator practice is a `${VAR}` placeholder, resolved from
/// the daemon's own environment at load time, not a literal secret value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServerConfigView {
    pub name: String,
    /// `"stdio"`, `"sse"`, or `"http"`.
    pub transport: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub url: Option<String>,
    pub enabled: bool,
}
```

- [ ] **Step 2: Add the query/write/response variants**

In `QueryPayload` (near `GetMcpStatus`), add:

```rust
    /// POLISH_WAVES.md sub-project 7, item B — the editable MCP server
    /// list (distinct from `GetMcpStatus`'s live connection status).
    /// Responds with [`QueryResponsePayload::GetMcpServerConfigs`].
    GetMcpServerConfigs,
    /// Add or replace (by `name`) one `[[mcp_server]]` entry. Takes effect
    /// on the next daemon start (MCP servers are boot-constructed).
    /// Responds with [`QueryResponsePayload::McpServersApplied`] (or
    /// `QueryError` on a structural validation failure).
    SetMcpServer {
        name: String,
        transport: String,
        #[serde(default)]
        command: Option<String>,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: Vec<(String, String)>,
        #[serde(default)]
        headers: Vec<(String, String)>,
        #[serde(default)]
        url: Option<String>,
        enabled: bool,
    },
    /// Remove one `[[mcp_server]]` entry by name (a no-op, not an error, if
    /// no entry with that name exists). Takes effect on the next daemon
    /// start. Responds with [`QueryResponsePayload::McpServersApplied`].
    DeleteMcpServer { name: String },
```

In `QueryResponsePayload` (near `GetMcpStatus`'s own response variant), add:

```rust
    /// Response to [`QueryPayload::GetMcpServerConfigs`].
    GetMcpServerConfigs { servers: Vec<McpServerConfigView> },
    /// Response to [`QueryPayload::SetMcpServer`] / [`QueryPayload::
    /// DeleteMcpServer`]. Carries the **fresh** list (re-read from disk)
    /// and `restart_required` (always `true` — MCP servers are
    /// boot-constructed).
    McpServersApplied {
        servers: Vec<McpServerConfigView>,
        restart_required: bool,
    },
```

- [ ] **Step 3: Round-trip test**

Add alongside `redacted_secret_round_trips_without_the_real_value` (Task 2):

```rust
    #[test]
    fn set_mcp_server_round_trips() {
        let msg = QueryPayload::SetMcpServer {
            name: "github".to_string(),
            transport: "stdio".to_string(),
            command: Some("npx".to_string()),
            args: vec!["-y".to_string()],
            env: vec![("TOKEN".to_string(), "${GITHUB_TOKEN}".to_string())],
            headers: Vec::new(),
            url: None,
            enabled: true,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: QueryPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn mcp_server_config_view_round_trips() {
        let view = McpServerConfigView {
            name: "github".to_string(),
            transport: "stdio".to_string(),
            command: Some("npx".to_string()),
            args: vec!["-y".to_string()],
            env: Vec::new(),
            headers: Vec::new(),
            url: None,
            enabled: true,
        };
        let json = serde_json::to_string(&view).unwrap();
        let back: McpServerConfigView = serde_json::from_str(&json).unwrap();
        assert_eq!(back, view);
    }
```

- [ ] **Step 4: Verify + full-crate check**

```bash
cargo test -p aivyx-ipc set_mcp_server_round_trips mcp_server_config_view_round_trips
cargo test -p aivyx-ipc
cargo clippy -p aivyx-ipc --all-targets -- -D warnings
```
Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-ipc/src/protocol.rs
git commit -m "feat(ipc): MCP server config wire types (GetMcpServerConfigs/SetMcpServer/DeleteMcpServer)

McpServerConfigView is the editable-config counterpart to the
existing McpServerStatusView (live connection status). env/headers
are plain strings, not RedactedSecret — MCP secrets live behind
\${VAR} placeholders by this codebase's own existing convention."
```

---

## Task 4: Daemon-side handlers

**Files:**
- Modify: `crates/aivyx-channel/src/daemon_server.rs`

**Interfaces:**
- Consumes: `aivyx_config::config_write::{write_mcp_server_section, remove_mcp_server_section, McpServerEntryWrite}` (Task 1); `aivyx_ipc::protocol::{QueryPayload::{GetMcpServerConfigs,SetMcpServer,DeleteMcpServer}, QueryResponsePayload::{GetMcpServerConfigs,McpServersApplied}, McpServerConfigView}` (Task 3); the existing `config_toml_path: Option<&Path>`, `audit_log: Option<&PersistentAuditLog>`, `role_override: Option<&str>`, `audit_config_change`, `map_config_write_error`, `no_config_file_error`, `load_settings_config` helpers/values already in scope in this file's query-handling function (confirmed present — `SetAccessLevel`'s own handler, ~line 5375, uses the identical `config_toml_path`/`audit_config_change`/`map_config_write_error` pattern; `settings_applied`, called from the same function, is the confirmed real caller of both `role_override` and `load_settings_config`).
- Produces: nothing further tasks in this plan consume (Task 5 adds its own, separate `TestMcpServerConnection` handler alongside these).

- [ ] **Step 1: Read the current config to add `GetMcpServerConfigs`**

Find `QueryPayload::GetMcpStatus`'s handler (`crates/aivyx-channel/src/daemon_server.rs:4402`) and add a new arm right after it:

```rust
        QueryPayload::GetMcpServerConfigs => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            QueryResponsePayload::GetMcpServerConfigs {
                servers: read_mcp_server_configs(path, role_override),
            }
        }
```

- [ ] **Step 2: Add `SetMcpServer`/`DeleteMcpServer`**

Find `QueryPayload::SetAccessLevel`'s handler (~line 5375) and add two new arms right before or after it (matching that handler's own style exactly):

```rust
        QueryPayload::SetMcpServer {
            name,
            transport,
            command,
            args,
            env,
            headers,
            url,
            enabled,
        } => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            let entry = aivyx_config::config_write::McpServerEntryWrite {
                name: name.clone(),
                transport,
                command,
                args,
                env,
                headers,
                url,
                enabled,
            };
            match aivyx_config::config_write::write_mcp_server_section(path, &entry) {
                Ok(()) => {
                    audit_config_change(audit_log, "mcp_server", &format!("set {name}"));
                    QueryResponsePayload::McpServersApplied {
                        servers: read_mcp_server_configs(path, role_override),
                        restart_required: true,
                    }
                }
                Err(e) => map_config_write_error(e),
            }
        }
        QueryPayload::DeleteMcpServer { name } => {
            let path = match config_toml_path {
                Some(p) => p,
                None => return no_config_file_error(),
            };
            match aivyx_config::config_write::remove_mcp_server_section(path, &name) {
                Ok(()) => {
                    audit_config_change(audit_log, "mcp_server", &format!("delete {name}"));
                    QueryResponsePayload::McpServersApplied {
                        servers: read_mcp_server_configs(path, role_override),
                        restart_required: true,
                    }
                }
                Err(e) => map_config_write_error(e),
            }
        }
```

- [ ] **Step 3: Add `map_config_write_error`'s new arm + the shared re-read helper**

`map_config_write_error` (~line 6022) matches `ConfigWriteError` exhaustively — Task 1 added a new variant, so this **must** gain a matching arm or the crate won't compile:

```rust
        E::InvalidMcpServer { .. } => "invalid_mcp_server",
```

(add this line inside the existing `match &e { ... }` block in `map_config_write_error`, alongside `E::InvalidBudget { .. } => "invalid_budget",`).

Add a new private helper near `map_config_write_error`/`audit_config_change` (bottom of the file's helper section). Reuse `load_settings_config` — the real function `settings_applied` itself already calls to get a full, fresh `AivyxConfig` (confirmed by reading it: `load_settings_config(toml_path, role_override) -> Result<aivyx_config::AivyxConfig, String>`, which wraps `AivyxConfig::load_from_env_and_toml`) — rather than inventing a new loader entry point:

```rust
/// Re-read `[[mcp_server]]` from disk into the wire view type — shared by
/// `GetMcpServerConfigs`/`SetMcpServer`/`DeleteMcpServer`'s handlers so the
/// response always reflects authoritative on-disk state, same principle as
/// `settings_applied`'s own fresh re-read (and reusing the exact same
/// `load_settings_config` helper it calls).
fn read_mcp_server_configs(
    path: &std::path::Path,
    role_override: Option<&str>,
) -> Vec<aivyx_ipc::protocol::McpServerConfigView> {
    let Ok(cfg) = load_settings_config(path, role_override) else {
        return Vec::new();
    };
    cfg.mcp_servers
        .iter()
        .map(|s| aivyx_ipc::protocol::McpServerConfigView {
            name: s.name.clone(),
            transport: match s.transport {
                aivyx_config::McpTransportKind::Stdio => "stdio",
                aivyx_config::McpTransportKind::Sse => "sse",
                aivyx_config::McpTransportKind::Http => "http",
            }
            .to_string(),
            command: s.command.clone(),
            args: s.args.clone(),
            env: s.env.clone(),
            headers: s.headers.clone(),
            url: s.url.clone(),
            enabled: s.enabled,
        })
        .collect()
}
```

- [ ] **Step 4: Verify**

```bash
cargo build -p aivyx-channel
cargo clippy -p aivyx-channel --all-targets -- -D warnings
cargo test -p aivyx-config -p aivyx-ipc -p aivyx-channel
```
Expected: all green — this step will surface if `AivyxConfig::load_from_path` needs correcting per Step 3's own note; fix and re-verify before moving on.

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-channel/src/daemon_server.rs
git commit -m "feat(channel): daemon handlers for GetMcpServerConfigs/SetMcpServer/DeleteMcpServer

Every write re-reads the fresh on-disk state before responding,
matching settings_applied's own convention. map_config_write_error
gains the InvalidMcpServer arm Task 1's new error variant requires."
```

---

## Task 5: MCP test-connection probe

**Files:**
- Modify: `crates/aivyx-ipc/src/protocol.rs`
- Modify: `crates/aivyx-channel/src/daemon_server.rs`

**Interfaces:**
- Consumes: `aivyx_mcp::{McpServerBridge, SseTransport, StreamableHttpTransport}` (real connection APIs — `McpServerBridge::start_with_sandbox(command: &str, args: &[&str], env: &[(String,String)], sandbox: Option<&SandboxConfig>, stderr_log: Option<&StderrLog>, server_name: impl Into<String>) -> Result<Self, String>`; `SseTransport::connect(url: &str, headers: &[(String,String)]) -> Result<SseTransport, String>` (confirm exact signature against `crates/aivyx-mcp/src/sse.rs` before using — the plan's own Step 2 code assumes this shape); `McpServerBridge::from_transport(transport: Arc<dyn McpTransport>, server_name: impl Into<String>) -> Result<Self, String>`; `.list_tools(&self) -> Result<Vec<McpToolDef>, String>`; `.shutdown(&self) -> Result<(), String>`).
- Produces: `QueryPayload::TestMcpServerConnection { transport, command, args, env, headers, url }`; `QueryResponsePayload::McpServerTestResult { ok: bool, tool_count: usize, error: Option<String> }`. No later task in this plan consumes these.

- [ ] **Step 1: Add the wire types**

In `crates/aivyx-ipc/src/protocol.rs`, add to `QueryPayload`:

```rust
    /// POLISH_WAVES.md sub-project 7, item B — attempt a real connection
    /// (the same logic the daemon uses at boot) against in-progress form
    /// values, before the operator saves. Never joins the live server
    /// list — a one-shot probe, torn down after. Responds with
    /// [`QueryResponsePayload::McpServerTestResult`].
    TestMcpServerConnection {
        transport: String,
        #[serde(default)]
        command: Option<String>,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: Vec<(String, String)>,
        #[serde(default)]
        headers: Vec<(String, String)>,
        #[serde(default)]
        url: Option<String>,
    },
```

And to `QueryResponsePayload`:

```rust
    /// Response to [`QueryPayload::TestMcpServerConnection`]. `error` is
    /// `None` iff `ok` — the raw connection error string otherwise (not a
    /// `QueryError`, since a failed test-connection is an expected,
    /// non-exceptional outcome the form should just display inline).
    McpServerTestResult {
        ok: bool,
        tool_count: usize,
        #[serde(default)]
        error: Option<String>,
    },
```

- [ ] **Step 2: Round-trip test**

```rust
    #[test]
    fn mcp_server_test_result_round_trips() {
        let msg = QueryResponsePayload::McpServerTestResult {
            ok: true,
            tool_count: 4,
            error: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: QueryResponsePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }
```

- [ ] **Step 3: Verify wire types compile + pass**

```bash
cargo test -p aivyx-ipc mcp_server_test_result_round_trips
cargo clippy -p aivyx-ipc --all-targets -- -D warnings
```
Expected: PASS, zero warnings.

- [ ] **Step 4: Commit the wire types**

```bash
git add crates/aivyx-ipc/src/protocol.rs
git commit -m "feat(ipc): TestMcpServerConnection/McpServerTestResult wire types"
```

- [ ] **Step 5: Add the daemon-side probe handler**

Before writing this handler, read `crates/aivyx-mcp/src/sse.rs` and `crates/aivyx-mcp/src/streamable_http.rs` to confirm `SseTransport::connect`/`StreamableHttpTransport::connect`'s exact signatures (the aivyx-cli `aivyx.rs` startup loop at ~line 7532 in this repo's history calls `aivyx_mcp::SseTransport::connect(url, &mcp_cfg.headers).await` and `aivyx_mcp::StreamableHttpTransport::connect(url, &mcp_cfg.headers).await` — mirror those call sites exactly, they are the real, working precedent).

Add a new arm to the same `match query_payload { ... }` this file's query handler lives in (near the `SetMcpServer`/`DeleteMcpServer` arms from Task 4):

```rust
        QueryPayload::TestMcpServerConnection {
            transport,
            command,
            args,
            env,
            headers,
            url,
        } => {
            let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
            let bridge_result = match transport.as_str() {
                "stdio" => {
                    let Some(cmd) = command.as_deref() else {
                        return QueryResponsePayload::McpServerTestResult {
                            ok: false,
                            tool_count: 0,
                            error: Some("stdio transport requires `command`".to_string()),
                        };
                    };
                    aivyx_mcp::McpServerBridge::start_with_sandbox(
                        cmd, &args_ref, &env, None, None, "test-connection",
                    )
                    .await
                }
                "sse" | "http" | "streamable-http" => {
                    let Some(u) = url.as_deref() else {
                        return QueryResponsePayload::McpServerTestResult {
                            ok: false,
                            tool_count: 0,
                            error: Some("sse/http transport requires `url`".to_string()),
                        };
                    };
                    let transport_result = if transport == "sse" {
                        aivyx_mcp::SseTransport::connect(u, &headers)
                            .await
                            .map(|t| std::sync::Arc::new(t) as std::sync::Arc<dyn aivyx_mcp::McpTransport>)
                    } else {
                        aivyx_mcp::StreamableHttpTransport::connect(u, &headers)
                            .await
                            .map(|t| std::sync::Arc::new(t) as std::sync::Arc<dyn aivyx_mcp::McpTransport>)
                    };
                    match transport_result {
                        Ok(t) => aivyx_mcp::McpServerBridge::from_transport(t, "test-connection").await,
                        Err(e) => Err(e),
                    }
                }
                other => {
                    return QueryResponsePayload::McpServerTestResult {
                        ok: false,
                        tool_count: 0,
                        error: Some(format!("unknown transport {other:?}")),
                    };
                }
            };
            match bridge_result {
                Ok(bridge) => {
                    let tool_count = bridge.list_tools().await.map(|t| t.len()).unwrap_or(0);
                    let _ = bridge.shutdown().await;
                    QueryResponsePayload::McpServerTestResult {
                        ok: true,
                        tool_count,
                        error: None,
                    }
                }
                Err(e) => QueryResponsePayload::McpServerTestResult {
                    ok: false,
                    tool_count: 0,
                    error: Some(e),
                },
            }
        }
```

`aivyx_mcp::McpTransport`/`SseTransport`/`StreamableHttpTransport`/`McpServerBridge` are all `pub use`-exported from `aivyx-mcp`'s crate root (confirmed: `crates/aivyx-mcp/src/lib.rs`), and `SseTransport::connect(endpoint: &str, headers: &[(String,String)])`/`StreamableHttpTransport::connect(endpoint: &str, headers: &[(String,String)])` match this code's usage exactly. `aivyx-channel` does **not** yet depend on `aivyx-mcp` (confirmed: absent from `crates/aivyx-channel/Cargo.toml`) — add it in that Cargo.toml's `[dependencies]`, matching `aivyx-cli`'s own existing line:

```toml
aivyx-mcp = { path = "../aivyx-mcp" }
```

- [ ] **Step 6: Verify**

```bash
cargo build -p aivyx-channel
cargo clippy -p aivyx-channel --all-targets -- -D warnings
```
Expected: both green. No new unit test for this handler — it makes a real network/process connection, matching how this codebase already treats other live-connection code (untested at this layer; Studio's manual test in Task 7 is the real verification).

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-channel/src/daemon_server.rs crates/aivyx-channel/Cargo.toml
git commit -m "feat(channel): MCP test-connection probe reuses the real boot-time connection logic

Same McpServerBridge::start_with_sandbox / SseTransport::connect /
StreamableHttpTransport::connect calls aivyx.rs's own startup loop
uses, run once against in-progress form values and torn down after."
```

---

## Task 6: Studio — MCP config list + add/edit/remove form

**Files:**
- Modify: `crates/aivyx-web/src/main.rs`

**Interfaces:**
- Consumes: `aivyx_ipc::protocol::{QueryPayload::{GetMcpServerConfigs,SetMcpServer,DeleteMcpServer}, QueryResponsePayload::{GetMcpServerConfigs,McpServersApplied}, McpServerConfigView}` (Tasks 3-4).
- Produces: a new `McpConfigUi` struct (`notice: Option<(bool, String)>` — mirrors `SkillsUi`/`MemoryUi`/`ServerInfoUi`'s established minimal shape) threaded through `App`/`ws_task`/`read_task` exactly like those (plain in `App`'s `use_coroutine` call and `ws_task`'s own signature, `mut` in `read_task`'s signature — this file's established, deliberate convention).

- [ ] **Step 1: Add `McpConfigUi` + thread it through `App`/`ws_task`/`read_task`**

Add near `MemoryUi` (`crates/aivyx-web/src/main.rs:551`):

```rust
/// POLISH_WAVES.md sub-project 7, item B — MCP config-write UI state
/// (save/delete outcome feedback), mirroring `MemoryUi`'s own minimal shape.
#[derive(Clone, Default, PartialEq)]
struct McpConfigUi {
    notice: Option<(bool, String)>,
}
```

Thread `mcp_config_ui: Signal<McpConfigUi>` through `App` (declare + `use_context_provider`), `ws_task`'s signature + its `spawn(read_task(...))` call, and `read_task`'s signature (as `mut`) — following exactly the same 4 edit points Task 3 of the UI-modernization plan used for `server_info` (see `crates/aivyx-web/src/main.rs`'s `App`/`ws_task`/`read_task` — `server_info`'s own threading, already shipped, is the concrete template to copy).

- [ ] **Step 2: Add query builders + response handling**

Add near `mcp_query()` (`crates/aivyx-web/src/main.rs:3693`):

```rust
fn mcp_server_configs_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-mcp-configs".to_string(),
        payload: QueryPayload::GetMcpServerConfigs,
    }
}

fn set_mcp_server_query(entry: McpServerConfigView) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-mcp-set".to_string(),
        payload: QueryPayload::SetMcpServer {
            name: entry.name,
            transport: entry.transport,
            command: entry.command,
            args: entry.args,
            env: entry.env,
            headers: entry.headers,
            url: entry.url,
            enabled: entry.enabled,
        },
    }
}

fn delete_mcp_server_query(name: String) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-mcp-delete".to_string(),
        payload: QueryPayload::DeleteMcpServer { name },
    }
}
```

In `read_task`'s `match env { ... }`, add (before the final `_ => {}`):

```rust
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetMcpServerConfigs { servers },
                    ..
                } => {
                    mcp.write().configs = servers;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::McpServersApplied { servers, .. },
                    ..
                } => {
                    mcp.write().configs = servers;
                    mcp_config_ui.write().notice = Some((true, "Saved — restart the daemon to apply.".to_string()));
                }
```

Add `configs: Vec<McpServerConfigView>` to `McpState` (`crates/aivyx-web/src/main.rs:305`):

```rust
struct McpState {
    servers: Vec<McpServerStatusView>,
    captured_unix: u64,
    loaded: bool,
    configs: Vec<McpServerConfigView>,
}
```

(`McpState` derives `#[derive(Clone, Default, PartialEq)]` — confirmed, no hand-written `impl Default` — so adding the `configs` field needs no further change beyond the struct definition itself; `Vec::new()` is automatic.)

- [ ] **Step 3: Extend `McpPanel` with a "Configured servers" section**

Replace `McpPanel`'s body (`crates/aivyx-web/src/main.rs:3701-3743`) — keep everything about the existing live-status section unchanged, add a new section below it:

```rust
#[component]
fn McpPanel() -> Element {
    let ws = use_context::<Sender>();
    let mcp = use_context::<Signal<McpState>>();
    let mut mcp_config_ui = use_context::<Signal<McpConfigUi>>();
    let mut editing = use_signal(|| None::<McpServerConfigView>);
    let mut adding = use_signal(|| false);

    use_future(move || async move {
        ws.send(mcp_query());
        ws.send(mcp_server_configs_query());
    });

    let m = mcp();
    let connected = m.servers.iter().filter(|s| s.connected).count();
    rsx! {
        div { class: "mcp",
            div { class: "panel-head",
                h3 { "MCP Servers" }
                if m.loaded && !m.servers.is_empty() {
                    span { class: "label-tech", "{connected}/{m.servers.len()} connected" }
                }
                button { class: "btn-ghost", onclick: move |_| { ws.send(mcp_query()); ws.send(mcp_server_configs_query()); }, "Refresh" }
            }
            if !m.loaded {
                SkeletonCards { cards: 3 }
            } else if m.servers.is_empty() {
                div { class: "glass-card empty",
                    p { class: "label-tech",
                        "No MCP servers reported at the last daemon start. Add one below, then restart the daemon."
                    }
                }
            } else {
                div { class: "mcp-grid",
                    for sv in m.servers.iter() {
                        { rsx! { McpServerCard { key: "{sv.name}", view: sv.clone() } } }
                    }
                }
            }

            div { class: "panel-head", style: "margin-top:22px;",
                h3 { "Configured servers" }
                button { class: "btn btn-primary btn-xs", onclick: move |_| { editing.set(None); adding.set(true); }, "Add server" }
            }
            if let Some((ok, text)) = mcp_config_ui().notice {
                div { class: if ok { "notice ok" } else { "notice err" }, "{text}" }
            }
            if adding() || editing().is_some() {
                McpServerForm {
                    initial: editing(),
                    on_cancel: move |_| { adding.set(false); editing.set(None); },
                    on_save: move |entry: McpServerConfigView| {
                        ws.send(set_mcp_server_query(entry));
                        adding.set(false);
                        editing.set(None);
                    },
                }
            } else if m.configs.is_empty() {
                div { class: "glass-card empty", p { class: "label-tech", "No `[[mcp_server]]` entries configured yet." } }
            } else {
                div { class: "mcp-grid",
                    for cfg in m.configs.iter() {
                        {
                            let cfg2 = cfg.clone();
                            let cfg3 = cfg.clone();
                            let name = cfg.name.clone();
                            rsx! {
                                div { key: "{cfg.name}", class: "glass-card mcp-card",
                                    div { class: "mcp-card-head",
                                        span { class: "mcp-name", "{cfg.name}" }
                                        span { class: "label-tech", "{cfg.transport}" }
                                        span { class: if cfg.enabled { "chip sage" } else { "chip" }, if cfg.enabled { "enabled" } else { "disabled" } }
                                    }
                                    div { style: "display:flex; gap:8px; margin-top:8px;",
                                        button { class: "btn btn-glass btn-xs", onclick: move |_| { adding.set(false); editing.set(Some(cfg2.clone())); }, "Edit" }
                                        button { class: "btn btn-glass btn-xs", onclick: move |_| ws.send(delete_mcp_server_query(name.clone())), "Delete" }
                                    }
                                    { let _ = &cfg3; rsx! {} }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
```

(The `{ let _ = &cfg3; rsx! {} }` line is a placeholder no-op — remove it; `cfg3` was scaffolding from drafting and isn't needed since `cfg2`/`name` already cover Edit/Delete's captures. Clean this up while implementing, don't ship a stray no-op line — self-review should catch this before commit.)

- [ ] **Step 4: `McpServerForm` component**

Add a new component after `McpServerCard`:

```rust
#[component]
fn McpServerForm(
    initial: Option<McpServerConfigView>,
    on_cancel: EventHandler<()>,
    on_save: EventHandler<McpServerConfigView>,
) -> Element {
    let seed = initial.clone().unwrap_or(McpServerConfigView {
        name: String::new(),
        transport: "stdio".to_string(),
        command: None,
        args: Vec::new(),
        env: Vec::new(),
        headers: Vec::new(),
        url: None,
        enabled: true,
    });
    let editing_existing = initial.is_some();
    let mut name = use_signal(|| seed.name.clone());
    let mut transport = use_signal(|| seed.transport.clone());
    let mut command = use_signal(|| seed.command.clone().unwrap_or_default());
    let mut args_raw = use_signal(|| seed.args.join(" "));
    let mut url = use_signal(|| seed.url.clone().unwrap_or_default());
    let mut env_raw = use_signal(|| {
        seed.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n")
    });
    let mut headers_raw = use_signal(|| {
        seed.headers.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n")
    });
    let mut enabled = use_signal(|| seed.enabled);
    let is_stdio = transport() == "stdio";

    let parse_pairs = |raw: &str| -> Vec<(String, String)> {
        raw.lines()
            .filter_map(|line| line.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .filter(|(k, _)| !k.is_empty())
            .collect()
    };

    rsx! {
        div { class: "glass-card",
            div { class: "field-row",
                label { "Name" }
                input { class: "input", value: "{name}", disabled: editing_existing, oninput: move |e| name.set(e.value()) }
            }
            div { class: "field-row",
                label { "Transport" }
                select { class: "input", value: "{transport}", onchange: move |e| transport.set(e.value()),
                    option { value: "stdio", "stdio" }
                    option { value: "sse", "sse" }
                    option { value: "http", "http" }
                }
            }
            if is_stdio {
                div { class: "field-row",
                    label { "Command" }
                    input { class: "input", value: "{command}", oninput: move |e| command.set(e.value()) }
                }
                div { class: "field-row",
                    label { "Args (space-separated)" }
                    input { class: "input", value: "{args_raw}", oninput: move |e| args_raw.set(e.value()) }
                }
                div { class: "field-row",
                    label { "Env (one KEY=value per line)" }
                    textarea { class: "doc-edit", value: "{env_raw}", oninput: move |e| env_raw.set(e.value()) }
                }
            } else {
                div { class: "field-row",
                    label { "URL" }
                    input { class: "input", value: "{url}", oninput: move |e| url.set(e.value()) }
                }
                div { class: "field-row",
                    label { "Headers (one Name=value per line)" }
                    textarea { class: "doc-edit", value: "{headers_raw}", oninput: move |e| headers_raw.set(e.value()) }
                }
            }
            div { class: "field-row",
                label { "Enabled" }
                input { r#type: "checkbox", checked: enabled(), onchange: move |e| enabled.set(e.checked()) }
            }
            div { style: "display:flex; gap:8px; margin-top:12px;",
                button {
                    class: "btn btn-primary btn-xs",
                    onclick: move |_| {
                        let entry = McpServerConfigView {
                            name: name().trim().to_string(),
                            transport: transport(),
                            command: if is_stdio && !command().trim().is_empty() { Some(command().trim().to_string()) } else { None },
                            args: if is_stdio { args_raw().split_whitespace().map(str::to_string).collect() } else { Vec::new() },
                            env: if is_stdio { parse_pairs(&env_raw()) } else { Vec::new() },
                            headers: if is_stdio { Vec::new() } else { parse_pairs(&headers_raw()) },
                            url: if is_stdio { None } else if url().trim().is_empty() { None } else { Some(url().trim().to_string()) },
                            enabled: enabled(),
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

- [ ] **Step 5: Verify**

```bash
cargo build -p aivyx-web --target wasm32-unknown-unknown
cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings
```
Expected: both green. No new unit tests for this task — it's forms/rendering, no new pure-function surface; manual verification happens once `dist/` is rebuilt (Task 8).

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "feat(web): MCP server add/edit/remove form in Studio

McpPanel gains a 'Configured servers' section below the existing,
untouched live-status cards: list + McpServerForm (add/edit), Delete.
env/header fields are plain textareas (not the redacted-secret UI —
these carry \${VAR} placeholders by convention, not raw secrets)."
```

---

## Task 7: Studio — test-connection button

**Files:**
- Modify: `crates/aivyx-web/src/main.rs`

**Interfaces:**
- Consumes: `QueryPayload::TestMcpServerConnection`/`QueryResponsePayload::McpServerTestResult` (Task 5).
- Produces: nothing further tasks consume.

- [ ] **Step 1: Add a `test_result: Option<(bool, String)>` field to `McpConfigUi`**

Change Task 6's `McpConfigUi` to:

```rust
#[derive(Clone, Default, PartialEq)]
struct McpConfigUi {
    notice: Option<(bool, String)>,
    test_result: Option<(bool, String)>,
}
```

- [ ] **Step 2: Handle the response in `read_task`**

Add to `read_task`'s `match env { ... }` (before the final `_ => {}`):

```rust
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::McpServerTestResult { ok, tool_count, error },
                    ..
                } => {
                    let text = if ok {
                        format!("Connected — {tool_count} tool(s) found.")
                    } else {
                        error.unwrap_or_else(|| "connection failed".to_string())
                    };
                    mcp_config_ui.write().test_result = Some((ok, text));
                }
```

- [ ] **Step 3: Add a "Test connection" button to `McpServerForm`**

In `McpServerForm` (Task 6), add a `Test connection` button next to Save/Cancel, and read `mcp_config_ui` for the result:

```rust
    let mut mcp_config_ui = use_context::<Signal<McpConfigUi>>();
```

(add this line at the top of `McpServerForm`, alongside its other `use_signal` calls)

```rust
                button {
                    class: "btn btn-glass btn-xs",
                    onclick: move |_| {
                        let ws = ws.clone();
                        let msg = FrontendMessage::Query {
                            id: "mc-mcp-test".to_string(),
                            payload: QueryPayload::TestMcpServerConnection {
                                transport: transport(),
                                command: if is_stdio && !command().trim().is_empty() { Some(command().trim().to_string()) } else { None },
                                args: if is_stdio { args_raw().split_whitespace().map(str::to_string).collect() } else { Vec::new() },
                                env: if is_stdio { parse_pairs(&env_raw()) } else { Vec::new() },
                                headers: if is_stdio { Vec::new() } else { parse_pairs(&headers_raw()) },
                                url: if is_stdio { None } else if url().trim().is_empty() { None } else { Some(url().trim().to_string()) },
                            },
                        };
                        ws.send(msg);
                    },
                    "Test connection"
                }
```

(Insert this button between Save and Cancel; `ws` must be brought into scope in `McpServerForm` via `let ws = use_context::<Sender>();` at the top, alongside the new `mcp_config_ui` line — add both. Note the closure captures `ws` by clone since `Sender`/the coroutine handle is `Clone`, matching this file's existing convention elsewhere for handlers that both read signals and send.)

Render the result below the buttons:

```rust
            if let Some((ok, text)) = mcp_config_ui().test_result {
                div { class: if ok { "notice ok" } else { "notice err" }, "{text}" }
            }
```

- [ ] **Step 4: Verify**

```bash
cargo build -p aivyx-web --target wasm32-unknown-unknown
cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings
```
Expected: both green.

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "feat(web): test-connection button in the MCP server form

Sends TestMcpServerConnection with the form's current (unsaved)
values; renders McpServerTestResult inline."
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
Expected: exactly one `A ` line and one `D ` line (a clean rename, not an accumulation of stale binaries).

- [ ] **Step 3: Commit**

```bash
git add crates/aivyx-web/dist/
git commit -m "chore(web): rebuild dist/ bundle for MCP CRUD (sub-project 7, plan 1)"
```

This is the final task — once this sweep is clean, proceed to the final whole-branch review.

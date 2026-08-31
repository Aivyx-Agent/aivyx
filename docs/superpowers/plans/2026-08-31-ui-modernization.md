# UI Modernization pass (POLISH_WAVES.md sub-project 6) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship all 4 sub-project 6 findings — Command Center depth restyle, a version-mismatch reload hint, Memory/Wiki graph-view readability, and safe markdown (incl. mermaid) rendering in Documents — per `docs/superpowers/specs/2026-08-31-ui-modernization-design.md`.

**Architecture:** Task 1 is a pure CSS change. Tasks 2-3 add a new bridge-only WebSocket message (`DaemonEnvelope::ServerInfo`) so Studio can detect a daemon restart. Tasks 4-5 add presentation-only fixes to the one shared force-directed layout function both knowledge-graph screens already call. Tasks 6-7 add a second, injection-safe markdown renderer alongside the existing trusted-content one, then wire in a lazy-loaded vendored mermaid.js for diagram fences.

**Tech Stack:** Dioxus 0.6 (Rust→WASM), `pulldown-cmark` 0.12, `web-sys`/`wasm-bindgen`, `tokio-tungstenite` (the daemon-side WS bridge), `uuid` (already a workspace dependency).

## Two corrections to the approved spec, found during planning

Both preserve the spec's intent; neither changes scope. Recorded here so a
reviewer comparing this plan to the spec doesn't read them as drift.

1. **Item B's mechanism.** The spec proposed comparing `aivyx-web`'s own
   `CARGO_PKG_VERSION`. That version is `version.workspace = true` and
   changes rarely — a rebuilt `dist/` almost never bumps it, so the
   comparison would hardly ever fire. Tracing the actual message path
   (`aivyx-channel/src/web_ui.rs`'s `handle_websocket`, the process that
   bridges the browser's WebSocket to the daemon's Unix socket) found a
   better signal already half-built: every WS connection already reads a
   `DaemonReady` handshake from the daemon and swallows it without
   forwarding to the browser. Task 2/3 below instead mint a random
   `boot_id` once per daemon process and have Studio compare it across
   reconnects — "the daemon this tab now talks to isn't the one it
   started against" is a more direct proxy for "a redeployed bundle
   restarted the daemon" than a rarely-bumped crate version, and needs no
   new build-time plumbing.
2. **Item C's "or selected" clause.** The spec said `MemoryGraph` "already
   threads `on_select`/has a notion of the active topic" — checking the
   component's actual signature (`main.rs`) found it only takes
   `on_select: EventHandler<String>`, an outbound click callback, not an
   inbound "currently selected" value. Task 4 below implements hover-only
   label disclosure without a selection concept, rather than adding a new
   prop neither component has today.

## Global Constraints

- Zero clippy warnings workspace-wide: `cargo clippy --workspace --exclude
  aivyx-desktop --all-targets -- -D warnings` must stay clean after every
  task.
- `cargo test --workspace --exclude aivyx-desktop` must stay green.
- `cargo build -p aivyx-web --target wasm32-unknown-unknown` (the
  `just check-web` guard) must stay green after every `aivyx-web`/
  `aivyx-ipc` change — this workspace's wasm32 toolchain is
  `~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin` +
  `~/.cargo/bin` on `PATH`.
- No CDN calls, no new runtime network dependency for any UI feature —
  vendor JS assets under `crates/aivyx-web/assets/`, served from the
  daemon's own embedded bundle (local-first stance).
- New wire fields/messages follow the existing backward-compatible
  convention (`#[serde(default)]` on struct fields added to an existing
  type; a brand-new enum variant needs no such attribute since old clients
  simply won't send/expect it).
- After Tasks 6/7 (the only tasks touching `aivyx-web`'s runtime UI in a
  user-visible way) the `dist/` bundle must be rebuilt and committed per
  this repo's established convention: `rm -rf dist && cp -r
  <dx-bundle-output> dist` (never a merging copy — this has previously
  left orphaned stale wasm binaries), strip `.br` files, then verify via
  `git status --porcelain dist/assets/ | grep wasm` showing exactly one
  add + one delete (a rename). Doing this once after Task 7 (not after
  every intermediate task) is fine since no task before Task 7 needs to be
  independently deployed.

---

## Task 1: Command Center depth restyle

**Files:**
- Modify: `crates/aivyx-web/assets/stitch.css:293-302` (`.card`/
  `.glass-card`), `:375` (`.panel`), `:329` (`.stat-card`)

**Interfaces:** CSS-only; no Rust signatures change. `CommandPanel`/
`CommandSkeleton` (`crates/aivyx-web/src/main.rs:1371`/`1472`) already use
`.glass-card`/`.panel`/`.stat-card` throughout, so this task needs no
changes to `main.rs` at all — every consumer of these classes across the
whole app picks up the new depth automatically.

- [ ] **Step 1: Add card/section shadows**

In `crates/aivyx-web/assets/stitch.css`, change:

```css
.glass-card {
  background: rgba(53,52,59,0.4); backdrop-filter: blur(12px);
  border: 1px solid var(--color-border-warm);
  border-radius: 0.5rem; padding: 16px;
}
```

to:

```css
.glass-card {
  background: rgba(53,52,59,0.4); backdrop-filter: blur(12px);
  border: 1px solid var(--color-border-warm);
  border-radius: 0.5rem; padding: 16px;
  /* POLISH_WAVES.md sub-project 6, item A — the brand guide's own Shadow
     System table assigns shadow-md to "Cards"; this token existed but was
     never applied to the card class itself, only to overlay chrome
     (command palette, mobile nav drawer). */
  box-shadow: var(--shadow-md);
}
```

And change:

```css
.panel { display: flex; flex-direction: column; gap: 12px; }
```

to:

```css
.panel {
  display: flex; flex-direction: column; gap: 12px;
  /* shadow-ambient is the brand guide's "Page sections" tier — the
     softer, wider falloff that sits under the tighter shadow-md on the
     cards inside each panel, matching "inner cards sit atop sections,
     which sit on the global canvas" (brand-guidelines.md §5). */
  box-shadow: var(--shadow-ambient);
}
```

- [ ] **Step 2: Hover-glow on stat cards**

In the same file, change:

```css
.stat-card { display: flex; flex-direction: column; gap: 6px; }
```

to:

```css
.stat-card {
  display: flex; flex-direction: column; gap: 6px;
  transition: box-shadow 0.15s var(--ease-smooth);
}
.stat-card:hover { box-shadow: var(--shadow-glow); }
```

(Mirrors the existing `.icon-btn:hover { box-shadow: var(--shadow-glow); }`
pattern at `stitch.css:255` — reusing the same token/approach rather than
inventing a new one.)

- [ ] **Step 3: Entrance motion on the dashboard**

`crates/aivyx-web/src/main.rs`'s `main { class: "view fade-in", ... }`
wrapper (~line 974) already applies `.fade-in` to every screen's outer
container on switch — confirm this by reading that line before editing
anything: if `.fade-in` is already there app-wide, Command Center already
gets an entrance animation for free and this step is a no-op (do not add
a second, redundant `.fade-in` to `CommandPanel`'s own `.stat-row`, which
would just restart the animation on an unrelated re-render and could look
like flicker). Record which case it was in the task's completion note.

- [ ] **Step 4: Verify**

```bash
cargo build -p aivyx-web --target wasm32-unknown-unknown
```
Expected: builds clean (CSS is not type-checked by cargo, so this is a
compile sanity check that nothing else broke, not a style verification).

Then rebuild `dist/` per the Global Constraints convention and visually
confirm in a browser that Command Center's stat cards and panels now read
with visible depth (a soft shadow under each card, a wider soft falloff
under each panel), and that stat cards glow on hover.

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-web/assets/stitch.css
git commit -m "style(web): apply shadow-md/shadow-ambient tokens to cards and panels

Both tokens were already defined (matching aivyx-brand's own Shadow
System table: shadow-md for Cards, shadow-ambient for Page sections)
but never referenced outside overlay chrome. Applying them to
.glass-card/.panel fixes the 'flat and boxy' Command Center finding
app-wide, since every screen already uses these classes."
```

---

## Task 2: Version-mismatch — bridge-side `ServerInfo` message

**Files:**
- Modify: `crates/aivyx-ipc/src/protocol.rs` (`DaemonEnvelope` enum,
  ~line 2652)
- Modify: `crates/aivyx-channel/src/web_ui.rs` (`run_web_ui_server`
  ~line 163, `handle_connection` ~line 380, `handle_websocket` ~line 775)
- Test: `crates/aivyx-channel/src/web_ui.rs`'s existing `mod tests`
  (~line 1046)

**Interfaces:**
- Produces: `aivyx_ipc::protocol::DaemonEnvelope::ServerInfo { boot_id:
  String }` (a new enum variant — Task 3 matches on it). `fn
  server_info_json(boot_id: &str) -> serde_json::Value` (a free function
  in `web_ui.rs`, the browser-facing JSON shape `handle_websocket` sends).

This message is bridge-only: `web_ui.rs` hand-constructs its JSON exactly
like the existing `session_started_json` a few lines above it, rather than
routing it through the daemon's own `DaemonMessage`/Unix-socket protocol —
the boot id is a concept the bridge process owns, not the daemon core, so
there is no `DaemonMessage` counterpart to add.

- [ ] **Step 1: Add the wire variant**

In `crates/aivyx-ipc/src/protocol.rs`, find the `DaemonEnvelope` enum
(`#[serde(tag = "type")]`, starting ~line 2652) and add a new variant
right after its `SessionStarted { session_id: String }` arm:

```rust
    /// POLISH_WAVES.md sub-project 6, item B. Sent once per WebSocket
    /// connection by the web-UI bridge (`aivyx-channel/src/web_ui.rs`)
    /// immediately after `SessionStarted` — never by the daemon core
    /// itself, so there is no `DaemonMessage` counterpart. `boot_id` is a
    /// random id minted once when the bridge's `run_web_ui_server` starts,
    /// stable for that process's lifetime and different after any
    /// restart. Studio compares it across reconnects to detect "the
    /// daemon I'm now talking to isn't the one I started against" — a
    /// direct proxy for "a redeployed dist bundle restarted the daemon" —
    /// and hints that a reload will pick up the newer bundle.
    ServerInfo {
        boot_id: String,
    },
```

- [ ] **Step 2: Verify it compiles and round-trips**

Add a test near `DaemonEnvelope`'s other round-trip tests in
`crates/aivyx-ipc/src/protocol.rs` (search the file for an existing test
using `DaemonEnvelope::SessionStarted` to find that test module, then add
alongside it):

```rust
    #[test]
    fn server_info_round_trips() {
        let msg = DaemonEnvelope::ServerInfo {
            boot_id: "test-boot-id".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"ServerInfo\""));
        let back: DaemonEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }
```

Run: `cargo test -p aivyx-ipc server_info_round_trips`
Expected: PASS (1 passed).

- [ ] **Step 3: Mint a per-process boot id**

In `crates/aivyx-channel/src/web_ui.rs`'s `run_web_ui_server` (~line 163),
find where `allowed_origins`/`auth_token`/`comfyui_base_url` are wrapped in
`Arc::new` (~lines 175-177):

```rust
    let allowed_origins = Arc::new(allowed_origins);
    let auth_token = Arc::new(auth_token);
    let comfyui_base_url = Arc::new(comfyui_base_url);
```

Add a fourth line right after:

```rust
    // POLISH_WAVES.md sub-project 6, item B — one random id per daemon
    // process, stable for its whole lifetime. See `DaemonEnvelope::
    // ServerInfo`'s doc comment (aivyx-ipc) for what this detects.
    let boot_id = Arc::new(uuid::Uuid::new_v4().to_string());
```

Then in the accept loop (~lines 232-236), where the per-connection clones
are made:

```rust
        let conn_socket_path = Arc::clone(&socket_path);
        let conn_broadcaster = web_ui_broadcaster.clone();
        let conn_allowed_origins = Arc::clone(&allowed_origins);
        let conn_auth_token = Arc::clone(&auth_token);
        let conn_comfyui_base_url = Arc::clone(&comfyui_base_url);
```

add:

```rust
        let conn_boot_id = Arc::clone(&boot_id);
```

And in the `handle_connection(...)` call right below it (~lines 239-248),
add `&conn_boot_id` as a new argument (position doesn't matter for
correctness; place it right after `&conn_socket_path` to keep
connection-identity-ish parameters grouped):

```rust
            if let Err(e) = handle_connection(
                stream,
                remote,
                &conn_socket_path,
                &conn_boot_id,
                port,
                &conn_allowed_origins,
                conn_auth_token.as_deref(),
                conn_comfyui_base_url.as_deref(),
                conn_broadcaster,
            )
            .await
```

- [ ] **Step 4: Thread it through `handle_connection`**

`handle_connection`'s signature (~line 380) currently is:

```rust
async fn handle_connection(
    stream: tokio::net::TcpStream,
    remote_addr: std::net::SocketAddr,
    socket_path: &Path,
    port: u16,
    allowed_origins: &[String],
    auth_token: Option<&str>,
    comfyui_base_url: Option<&str>,
    web_ui_broadcaster: Option<Arc<WebUiBroadcaster>>,
) -> Result<(), DaemonError> {
```

Add `boot_id: &str` right after `socket_path: &Path`:

```rust
async fn handle_connection(
    stream: tokio::net::TcpStream,
    remote_addr: std::net::SocketAddr,
    socket_path: &Path,
    boot_id: &str,
    port: u16,
    allowed_origins: &[String],
    auth_token: Option<&str>,
    comfyui_base_url: Option<&str>,
    web_ui_broadcaster: Option<Arc<WebUiBroadcaster>>,
) -> Result<(), DaemonError> {
```

Then find its own call to `handle_websocket` (~line 440):

```rust
        handle_websocket(ws_stream, socket_path, web_ui_broadcaster).await
```

change to:

```rust
        handle_websocket(ws_stream, socket_path, boot_id, web_ui_broadcaster).await
```

- [ ] **Step 5: Send `ServerInfo` from `handle_websocket`**

`handle_websocket`'s signature (~line 775) currently is:

```rust
async fn handle_websocket(
    ws_stream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    socket_path: &Path,
    web_ui_broadcaster: Option<Arc<WebUiBroadcaster>>,
) -> Result<(), DaemonError> {
```

Add `boot_id: &str` the same way:

```rust
async fn handle_websocket(
    ws_stream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    socket_path: &Path,
    boot_id: &str,
    web_ui_broadcaster: Option<Arc<WebUiBroadcaster>>,
) -> Result<(), DaemonError> {
```

Find the existing `SessionStarted` send (~lines 862-874):

```rust
    // Send SessionStarted to the browser.
    let session_started_json = serde_json::json!({
        "type": "SessionStarted",
        "session_id": session_id,
    });
    {
        let mut sink = ws_sink.lock().await;
        let _ = sink
            .send(tokio_tungstenite::tungstenite::Message::Text(
                session_started_json.to_string().into(),
            ))
            .await;
    }
```

Add right after it:

```rust
    // Send ServerInfo to the browser (POLISH_WAVES.md sub-project 6, item
    // B) — one shot per connection, same pattern as SessionStarted above.
    let server_info_json = server_info_json(boot_id);
    {
        let mut sink = ws_sink.lock().await;
        let _ = sink
            .send(tokio_tungstenite::tungstenite::Message::Text(
                server_info_json.to_string().into(),
            ))
            .await;
    }
```

Then add the small pure helper it calls — put it near this repo's other
small pure helpers in this file (e.g. right after `should_log_rejected_
token`, ~line 361), so it's testable without spinning up a real
WebSocket:

```rust
/// The JSON shape sent to the browser for `POLISH_WAVES.md` sub-project 6,
/// item B. Split out from `handle_websocket`'s send call so the shape is
/// unit-testable without a real connection — mirrors `should_log_rejected_
/// token`'s own split from its side-effecting caller.
fn server_info_json(boot_id: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "ServerInfo",
        "boot_id": boot_id,
    })
}
```

- [ ] **Step 6: Test the JSON shape**

In `web_ui.rs`'s existing `mod tests` (~line 1046), add:

```rust
    #[test]
    fn server_info_json_shape() {
        let v = server_info_json("abc-123");
        assert_eq!(v["type"], "ServerInfo");
        assert_eq!(v["boot_id"], "abc-123");
    }
```

Run: `cargo test -p aivyx-channel server_info_json_shape`
Expected: PASS (1 passed).

- [ ] **Step 7: Full-crate check**

```bash
cargo test -p aivyx-ipc -p aivyx-channel
cargo clippy -p aivyx-ipc -p aivyx-channel --all-targets -- -D warnings
```
Expected: all green, zero warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-ipc/src/protocol.rs crates/aivyx-channel/src/web_ui.rs
git commit -m "feat(channel,ipc): send a per-boot ServerInfo message to Studio

New DaemonEnvelope::ServerInfo{boot_id} variant, bridge-only (no
DaemonMessage counterpart — the daemon core doesn't need to know
about it). web_ui.rs mints one random boot_id per daemon process and
sends it right after SessionStarted on every WS connection, so Studio
can detect a daemon restart across a reconnect (Task 3 consumes this)."
```

---

## Task 3: Version-mismatch — Studio client banner

**Files:**
- Modify: `crates/aivyx-web/src/main.rs` (`App` ~line 640, `ws_task`
  ~line 7519, `read_task` ~line 7659, the top-level render ~line 966)
- Modify: `crates/aivyx-web/assets/stitch.css` (new `.notice.info` tone)

**Interfaces:**
- Consumes: `aivyx_ipc::protocol::DaemonEnvelope::ServerInfo { boot_id:
  String }` (Task 2).
- Produces: `struct ServerInfoUi { boot_id: Option<String>, update_
  available: bool }` and `fn apply_server_info(current: &ServerInfoUi,
  boot_id: String) -> ServerInfoUi` (a pure function, unit-tested
  directly) — no other task depends on these.

- [ ] **Step 1: Add the `ServerInfoUi` struct + pure transition function**

Add near `MemoryUi` (`crates/aivyx-web/src/main.rs:551`), mirroring its
exact shape/doc-comment convention:

```rust
/// POLISH_WAVES.md sub-project 6, item B — tracks the most recent
/// `DaemonEnvelope::ServerInfo.boot_id` Studio has seen, and whether a
/// *different* one has arrived since (meaning the daemon this tab now
/// talks to isn't the one it started against).
#[derive(Clone, Default, PartialEq)]
struct ServerInfoUi {
    boot_id: Option<String>,
    update_available: bool,
}

/// The state transition `ServerInfoUi` takes on receiving a `boot_id`:
/// the first one seen this session is just recorded (no banner); any
/// later one that *differs* latches `update_available` on. It stays on
/// once set — a flapping reconnect landing back on the same new boot_id
/// doesn't clear it, and a manual dismiss (not modeled here; see the
/// render step) is the only way off, so a real update can't be hidden by
/// a lucky match.
fn apply_server_info(current: &ServerInfoUi, boot_id: String) -> ServerInfoUi {
    match &current.boot_id {
        None => ServerInfoUi { boot_id: Some(boot_id), update_available: false },
        Some(seen) if *seen == boot_id => current.clone(),
        Some(_) => ServerInfoUi { boot_id: Some(boot_id), update_available: true },
    }
}
```

- [ ] **Step 2: Unit-test the pure function**

Add a new test module right after the struct/function (this file has no
single shared test module — small topic-scoped `mod ..._tests` blocks are
the established pattern, e.g. `mission_control_tests` at line 6335):

```rust
#[cfg(test)]
mod server_info_tests {
    use super::*;

    #[test]
    fn first_boot_id_is_recorded_without_a_banner() {
        let next = apply_server_info(&ServerInfoUi::default(), "a".to_string());
        assert_eq!(next.boot_id, Some("a".to_string()));
        assert!(!next.update_available);
    }

    #[test]
    fn same_boot_id_again_does_not_trigger_the_banner() {
        let seen = ServerInfoUi { boot_id: Some("a".to_string()), update_available: false };
        let next = apply_server_info(&seen, "a".to_string());
        assert!(!next.update_available);
    }

    #[test]
    fn a_different_boot_id_triggers_the_banner() {
        let seen = ServerInfoUi { boot_id: Some("a".to_string()), update_available: false };
        let next = apply_server_info(&seen, "b".to_string());
        assert_eq!(next.boot_id, Some("b".to_string()));
        assert!(next.update_available);
    }

    #[test]
    fn banner_stays_on_across_a_further_reconnect_to_the_same_new_id() {
        let updated = ServerInfoUi { boot_id: Some("b".to_string()), update_available: true };
        let next = apply_server_info(&updated, "b".to_string());
        assert!(next.update_available);
    }
}
```

Run: `cargo test -p aivyx-web server_info_tests`

Note: despite `aivyx-web` being a wasm32-only *build target*
(`--target wasm32-unknown-unknown` is required for `cargo build`/
`cargo clippy` — see Global Constraints), its `#[cfg(test)]` blocks
compile and run natively with a plain `cargo test -p aivyx-web`, no
`--target` flag — confirmed against this file's existing
`mission_control_tests` module, which already runs this way (23 passing
tests). Do NOT add `--target wasm32-unknown-unknown` to a `cargo test`
invocation for this crate: it compiles the test binary as a `.wasm` file
this host can't execute (`Exec format error`) — verified by actually
running it during planning, not assumed.

- [ ] **Step 3: Wire the signal into `App`, `ws_task`, `read_task`**

In `App()` (`crates/aivyx-web/src/main.rs:640`), add right after
`let memory_ui = use_signal(MemoryUi::default);` (~line 721):

```rust
    let server_info = use_signal(ServerInfoUi::default);
```

Find the `use_coroutine` call constructing `ws_task` (~line 753-760) and
add `server_info` as the last argument:

```rust
    let ws: Sender = use_coroutine(move |rx| {
        ws_task(
            rx, missions, running_overlay, dashboard, memory, memory_ui, wiki, lattice, settings, agents,
            teams, documents, voice, skills, skills_ui, mcp, tools, gallery, schedules_ui,
            notifications, audit_page, sessions_page, connected, session, transcript, streaming,
            gate, mission_ui, server_info,
        )
    });
```

`server_info` does NOT need `use_context_provider` — `App`'s own render
reads it directly for the banner (Step 5), the same way `connected` is
read directly rather than via context.

In `ws_task`'s signature (`crates/aivyx-web/src/main.rs:7519`), add a
plain (non-`mut`) parameter at the end, matching `memory_ui`'s own
non-`mut` treatment in this function:

```rust
    mission_ui: Signal<MissionControlUi>,
    server_info: Signal<ServerInfoUi>,
) {
```

and add it to the `spawn(read_task(...))` call inside `ws_task` (~lines
7578-7583):

```rust
        spawn(read_task(
            read, missions, running_overlay, dashboard, memory, memory_ui, wiki, lattice, settings, agents,
            teams, documents, voice, skills, skills_ui, mcp, tools, gallery, schedules_ui,
            notifications, audit_page, sessions_page, connected, session, transcript, streaming,
            gate, mission_ui, server_info,
        ));
```

In `read_task`'s signature (~line 7659), add a `mut` parameter at the end,
matching `mut memory_ui`'s own treatment in this function (this
non-uniform `mut`-in-`read_task`-only convention is intentional and
already established — see the file's comment on `memory_ui` for why):

```rust
    mut mission_ui: Signal<MissionControlUi>,
    mut server_info: Signal<ServerInfoUi>,
) {
```

- [ ] **Step 4: Handle the envelope in `read_task`**

In `read_task`'s `match env { ... }` block, add a new arm before the
final `_ => {}` catch-all (~line 8324):

```rust
                DaemonEnvelope::ServerInfo { boot_id } => {
                    let current = server_info();
                    server_info.set(apply_server_info(&current, boot_id));
                }
```

- [ ] **Step 5: Render the banner**

In `App`'s render (~line 966), right after the existing "Connection to the
agent lost" banner block:

```rust
                if !connected() {
                    div {
                        style: "position:sticky;top:0;z-index:1000;background:var(--danger, #b91c1c);color:#fff;text-align:center;padding:6px 12px;font-size:13px;letter-spacing:0.02em;",
                        role: "alert",
                        "Connection to the agent lost — reconnecting…"
                    }
                }
```

add:

```rust
                if server_info().update_available {
                    div { class: "notice info reload-hint", role: "status",
                        "A new version of Aivyx Studio is available. "
                        button {
                            class: "btn btn-primary btn-xs",
                            onclick: move |_| {
                                if let Some(w) = web_sys::window() {
                                    let _ = w.location().reload();
                                }
                            },
                            "Reload"
                        }
                        button {
                            class: "btn btn-glass btn-xs",
                            onclick: move |_| {
                                let mut cur = server_info();
                                cur.update_available = false;
                                server_info.set(cur);
                            },
                            "Dismiss"
                        }
                    }
                }
```

- [ ] **Step 6: Add the `.notice.info` tone + sticky banner styling**

In `crates/aivyx-web/assets/stitch.css`, find:

```css
.notice { padding: 10px 14px; border-radius: 0.5rem; font-size: 13px; }
.notice.ok  { background: rgba(77, 139, 106, 0.12); color: var(--color-sage); }
.notice.err { background: rgba(196, 85, 62, 0.15); color: var(--color-error); }
```

change to:

```css
.notice { padding: 10px 14px; border-radius: 0.5rem; font-size: 13px; }
.notice.ok   { background: rgba(77, 139, 106, 0.12); color: var(--color-sage); }
.notice.err  { background: rgba(196, 85, 62, 0.15); color: var(--color-error); }
.notice.info { background: rgba(255, 183, 125, 0.12); color: var(--color-primary); }
.reload-hint {
  position: sticky; top: 0; z-index: 999;
  display: flex; align-items: center; gap: 10px; justify-content: center;
  border-radius: 0;
}
```

- [ ] **Step 7: Verify**

```bash
cargo build -p aivyx-web --target wasm32-unknown-unknown
cargo test -p aivyx-web server_info_tests
cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings
```
Expected: all green.

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-web/src/main.rs crates/aivyx-web/assets/stitch.css
git commit -m "feat(web): show a reload hint when the daemon's boot id changes

ServerInfoUi tracks the boot_id from DaemonEnvelope::ServerInfo
(Task 2) across reconnects; a different id latches a dismissible
'new version available' banner. Pure apply_server_info() is
unit-tested directly."
```

---

## Task 4: Knowledge-graph label collision + hover-only labels

**Files:**
- Modify: `crates/aivyx-web/src/main.rs` (`compute_layout`'s neighborhood
  ~line 4082-4148, `MemoryGraph` ~line 4154, `LatticeGraph` ~line 3980)
- Modify: `crates/aivyx-web/assets/stitch.css` (`.mem-edge` opacity)

**Interfaces:**
- Produces: `fn label_sides(nodes: &[MemoryGraphNode], pos: &[(f64,
  f64)]) -> Vec<f64>` (shared by both `MemoryGraph` and `LatticeGraph` —
  `LatticeGraph` already builds a `Vec<MemoryGraphNode>` locally to reuse
  `compute_layout`, so it reuses this the same way, no signature changes
  needed on its side).
- Consumes: `node_radius` (`main.rs:4146`, unchanged).

- [ ] **Step 1: Write the failing tests**

Add a new test module right after `node_radius` (~line 4148), before
`MemoryGraph`'s definition:

```rust
#[cfg(test)]
mod graph_label_tests {
    use super::*;

    fn node(topic: &str, entry_count: u32) -> MemoryGraphNode {
        MemoryGraphNode { topic: topic.to_string(), entry_count }
    }

    #[test]
    fn far_apart_labels_both_go_below() {
        let nodes = vec![node("alpha", 1), node("beta", 1)];
        let pos = vec![(0.0, 0.0), (500.0, 400.0)];
        let sides = label_sides(&nodes, &pos);
        assert_eq!(sides, vec![1.0, 1.0]);
    }

    #[test]
    fn close_labels_alternate_to_avoid_collision() {
        let nodes = vec![node("alpha", 1), node("beta", 1)];
        // Same y, close x — "below" placement for both would overlap.
        let pos = vec![(100.0, 100.0), (108.0, 100.0)];
        let sides = label_sides(&nodes, &pos);
        assert_eq!(sides[0], 1.0);
        assert_eq!(sides[1], -1.0);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p aivyx-web graph_label_tests
```
Expected: FAIL — `label_sides`/`labels_collide` not defined.

- [ ] **Step 3: Implement the collision helpers**

Add right after `node_radius` (~line 4148), before the test module added
above:

```rust
/// Approximate on-screen width of a label in SVG viewBox units. No real
/// text-measurement API exists outside the DOM, so this is a fixed
/// per-character estimate tuned to `.mem-node text`'s font-size — a
/// heuristic for a collision *check*, not a pixel-perfect layout.
const LABEL_CHAR_WIDTH: f64 = 6.0;
const LABEL_HEIGHT: f64 = 12.0;

fn label_width(label: &str) -> f64 {
    label.chars().count() as f64 * LABEL_CHAR_WIDTH
}

/// Two labels "collide" when their approximate bounding boxes — centered
/// on `(ax, ay)`/`(bx, by)`, `label_width` wide, `LABEL_HEIGHT` tall —
/// overlap.
fn labels_collide(ax: f64, ay: f64, a_label: &str, bx: f64, by: f64, b_label: &str) -> bool {
    let (aw, bw) = (label_width(a_label), label_width(b_label));
    let dx = (ax - bx).abs();
    let dy = (ay - by).abs();
    dx < (aw + bw) / 2.0 && dy < LABEL_HEIGHT
}

/// One label placement side per node, in `nodes`/`pos` order: `1.0` places
/// the label below the node (today's only behavior), `-1.0` places it
/// above. A node's label goes above only when placing it below would
/// collide with an EARLIER node's label at that node's own decided side —
/// a single greedy left-to-right pass, not a full layout solve, but
/// enough to break the dense-cluster case that made every label overlap.
/// Shared by `MemoryGraph` and `LatticeGraph` (the latter already builds
/// a `Vec<MemoryGraphNode>` locally to reuse `compute_layout`, and reuses
/// this the same way).
fn label_sides(nodes: &[MemoryGraphNode], pos: &[(f64, f64)]) -> Vec<f64> {
    let mut sides: Vec<f64> = Vec::with_capacity(nodes.len());
    for i in 0..nodes.len() {
        let (xi, yi) = pos[i];
        let ri = node_radius(nodes[i].entry_count);
        let below = yi + ri + 11.0;
        let collides = (0..i).any(|j| {
            let (xj, yj) = pos[j];
            let rj = node_radius(nodes[j].entry_count);
            let yj_label = yj + sides[j] * (rj + 11.0);
            labels_collide(xi, below, &nodes[i].topic, xj, yj_label, &nodes[j].topic)
        });
        sides.push(if collides { -1.0 } else { 1.0 });
    }
    sides
}
```

- [ ] **Step 4: Run to verify it passes**

```bash
cargo test -p aivyx-web graph_label_tests
```
Expected: PASS (2 passed).

- [ ] **Step 5: Use `label_sides` + hover-only disclosure in `MemoryGraph`**

`MemoryGraph`'s current node-rendering loop (`main.rs:4192-4205`):

```rust
                // Nodes.
                for (i, node) in nodes.iter().enumerate() {
                    {
                        let (cx, cy) = pos[i];
                        let r = node_radius(node.entry_count);
                        let topic = node.topic.clone();
                        rsx! {
                            g { class: "mem-node",
                                onclick: move |_| on_select.call(topic.clone()),
                                circle { cx: "{cx}", cy: "{cy}", r: "{r}" }
                                text { x: "{cx}", y: "{cy + r + 11.0}", text_anchor: "middle", "{node.topic}" }
                            }
                        }
                    }
                }
```

Replace the whole `MemoryGraph` function body with (new/changed lines
marked by the surrounding comments — the edge-rendering block above the
node loop is unchanged and omitted here for brevity, do not delete it):

```rust
#[component]
fn MemoryGraph(
    nodes: Vec<MemoryGraphNode>,
    edges: Vec<PairScore>,
    on_select: EventHandler<String>,
) -> Element {
    let pos = compute_layout(&nodes, &edges);
    let sides = label_sides(&nodes, &pos);
    // POLISH_WAVES.md sub-project 6, item C — past this many nodes,
    // always-on labels overlap into an unreadable smear; show a label
    // only for the hovered node instead.
    const LABEL_ALWAYS_ON_MAX: usize = 25;
    let always_on = nodes.len() <= LABEL_ALWAYS_ON_MAX;
    let mut hovered = use_signal(|| None::<String>);
    let idx: std::collections::HashMap<&str, usize> =
        nodes.iter().enumerate().map(|(i, nd)| (nd.topic.as_str(), i)).collect();
    let max_score = edges.iter().map(|e| e.score).fold(0.1_f32, f32::max);

    rsx! {
        div { class: "glass-card mem-graph-card",
            if edges.is_empty() {
                p { class: "label-tech sub", "No co-occurrence links yet — topics appear as a cloud until the agent recalls them together." }
            }
            svg {
                class: "mem-graph",
                view_box: "0 0 {GRAPH_W} {GRAPH_H}",
                // Edges first (under the nodes).
                for e in edges.iter() {
                    if let (Some(&i), Some(&j)) = (idx.get(e.a.as_str()), idx.get(e.b.as_str())) {
                        {
                            let (x1, y1) = pos[i];
                            let (x2, y2) = pos[j];
                            let frac = (e.score / max_score).clamp(0.1, 1.0) as f64;
                            let w = 0.6 + frac * 3.4;
                            let op = 0.08 + frac * 0.4;
                            rsx! {
                                line {
                                    x1: "{x1}", y1: "{y1}", x2: "{x2}", y2: "{y2}",
                                    class: "mem-edge",
                                    stroke_width: "{w}", opacity: "{op}",
                                }
                            }
                        }
                    }
                }
                // Nodes.
                for (i, node) in nodes.iter().enumerate() {
                    {
                        let (cx, cy) = pos[i];
                        let r = node_radius(node.entry_count);
                        let side = sides[i];
                        let ly = cy + side * (r + 11.0);
                        let topic = node.topic.clone();
                        let topic_hover = node.topic.clone();
                        let topic_leave = node.topic.clone();
                        let show_label = always_on || hovered() == Some(node.topic.clone());
                        rsx! {
                            g { class: "mem-node",
                                onclick: move |_| on_select.call(topic.clone()),
                                onmouseenter: move |_| hovered.set(Some(topic_hover.clone())),
                                onmouseleave: move |_| {
                                    if hovered() == Some(topic_leave.clone()) {
                                        hovered.set(None);
                                    }
                                },
                                circle { cx: "{cx}", cy: "{cy}", r: "{r}" }
                                if show_label {
                                    text { x: "{cx}", y: "{ly}", text_anchor: "middle", "{node.topic}" }
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

(The edge-opacity floor was also lowered from `0.12 + frac * 0.5` to
`0.08 + frac * 0.4` per the spec's "fade low-weight edges further" — a
small constant change alongside code you're already touching in this
function.)

- [ ] **Step 6: Same treatment in `LatticeGraph`**

`LatticeGraph`'s current body (`main.rs:3980-4052`) computes `pos` via
`compute_layout(&nodes, &layout_edges)` and then renders directed edges
(with a `predicate` label at each edge midpoint) followed by entity nodes
with an always-on label. Apply the same `label_sides` + hover-only pattern
to the node labels, and gate the edge-predicate labels on the same
`always_on` flag (the denser the graph, the more those overlap too):

Replace the whole `LatticeGraph` function body with:

```rust
#[component]
fn LatticeGraph(entities: Vec<GraphEntity>, edges: Vec<GraphTriple>) -> Element {
    // Map to the layout types (the FR layout cares only about
    // connectivity, not direction).
    let nodes: Vec<MemoryGraphNode> = entities
        .iter()
        .map(|e| MemoryGraphNode { topic: e.name.clone(), entry_count: e.degree })
        .collect();
    let layout_edges: Vec<PairScore> = edges
        .iter()
        .map(|t| PairScore {
            a: t.subject.clone(),
            b: t.object.clone(),
            score: t.mentions.max(1) as f32,
            samples: t.mentions,
        })
        .collect();
    let pos = compute_layout(&nodes, &layout_edges);
    let sides = label_sides(&nodes, &pos);
    // POLISH_WAVES.md sub-project 6, item C — same threshold/rationale as
    // MemoryGraph; also gates the edge-predicate labels below, which are
    // an even denser source of overlap than the node labels alone.
    const LABEL_ALWAYS_ON_MAX: usize = 25;
    let always_on = nodes.len() <= LABEL_ALWAYS_ON_MAX;
    let mut hovered = use_signal(|| None::<String>);
    let idx: std::collections::HashMap<&str, usize> =
        nodes.iter().enumerate().map(|(i, nd)| (nd.topic.as_str(), i)).collect();

    rsx! {
        div { class: "glass-card mem-graph-card",
            svg {
                class: "mem-graph",
                view_box: "0 0 {GRAPH_W} {GRAPH_H}",
                defs {
                    marker {
                        id: "lattice-arrow", view_box: "0 0 10 10",
                        ref_x: "9", ref_y: "5", marker_width: "7", marker_height: "7",
                        orient: "auto-start-reverse",
                        path { d: "M 0 0 L 10 5 L 0 10 z", class: "lattice-arrowhead" }
                    }
                }
                // Directed edges (under the nodes), shortened to the target
                // node's rim so the arrowhead is visible.
                for t in edges.iter() {
                    if let (Some(&i), Some(&j)) = (idx.get(t.subject.as_str()), idx.get(t.object.as_str())) {
                        {
                            let (x1, y1) = pos[i];
                            let (x2c, y2c) = pos[j];
                            let r = node_radius(entities[j].degree) + 4.0;
                            let dx = x2c - x1; let dy = y2c - y1;
                            let d = (dx * dx + dy * dy).sqrt().max(0.01);
                            let (x2, y2) = (x2c - dx / d * r, y2c - dy / d * r);
                            let (mx, my) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
                            let label = t.predicate.clone();
                            rsx! {
                                line {
                                    x1: "{x1}", y1: "{y1}", x2: "{x2}", y2: "{y2}",
                                    class: "lattice-edge", marker_end: "url(#lattice-arrow)",
                                }
                                if always_on {
                                    text { x: "{mx}", y: "{my}", class: "lattice-edge-label", text_anchor: "middle", "{label}" }
                                }
                            }
                        }
                    }
                }
                // Entity nodes.
                for (i, ent) in entities.iter().enumerate() {
                    {
                        let (cx, cy) = pos[i];
                        let r = node_radius(ent.degree);
                        let side = sides[i];
                        let ly = cy + side * (r + 11.0);
                        let name_hover = ent.name.clone();
                        let name_leave = ent.name.clone();
                        let show_label = always_on || hovered() == Some(ent.name.clone());
                        rsx! {
                            g { class: "mem-node",
                                onmouseenter: move |_| hovered.set(Some(name_hover.clone())),
                                onmouseleave: move |_| {
                                    if hovered() == Some(name_leave.clone()) {
                                        hovered.set(None);
                                    }
                                },
                                circle { cx: "{cx}", cy: "{cy}", r: "{r}" }
                                if show_label {
                                    text { x: "{cx}", y: "{ly}", text_anchor: "middle", "{ent.name}" }
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

- [ ] **Step 7: Verify**

```bash
cargo test -p aivyx-web graph_label_tests
cargo build -p aivyx-web --target wasm32-unknown-unknown
cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings
```
Expected: all green.

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "fix(web): avoid label collisions in the Memory/Wiki knowledge graphs

label_sides() alternates a node's label above/below when the default
(below) placement would overlap an earlier node's — shared by
MemoryGraph and LatticeGraph via the MemoryGraphNode type both
already normalize to. Past 25 nodes, labels (and LatticeGraph's edge
predicates) switch from always-on to hover-only. Edge opacity floor
lowered so the dominant structure reads more clearly at a glance."
```

---

## Task 5: Knowledge-graph pan/zoom

**Files:**
- Modify: `crates/aivyx-web/src/main.rs` (`MemoryGraph`, `LatticeGraph` —
  same two components Task 4 touched)
- Modify: `crates/aivyx-web/assets/stitch.css` (`.mem-graph` cursor)

**Interfaces:** No new shared functions — `zoom`/`pan` are per-component
local signals in each of `MemoryGraph`/`LatticeGraph` (each screen's graph
is panned/zoomed independently; there is no shared "current viewport"
concept to thread between them).

Depends on Task 4 (this task edits the same two function bodies Task 4
just rewrote) — do this task after Task 4's commit lands, not in
parallel.

- [ ] **Step 1: Add zoom/pan state + viewBox computation to `MemoryGraph`**

Add these lines right after `let mut hovered = use_signal(|| None::<String>);`
in `MemoryGraph` (from Task 4's rewritten version):

```rust
    // POLISH_WAVES.md sub-project 6, item C — pan/zoom so a dense graph
    // can be explored instead of always fit-to-canvas. `zoom` scales the
    // visible viewBox (>1.0 = zoomed in / a smaller visible area); `pan`
    // is the viewBox's top-left corner in the same SVG-unit space.
    let mut zoom = use_signal(|| 1.0_f64);
    let mut pan = use_signal(|| (0.0_f64, 0.0_f64));
    let mut dragging = use_signal(|| None::<(f64, f64)>);
    let (vb_x, vb_y) = pan();
    let vb_w = GRAPH_W / zoom();
    let vb_h = GRAPH_H / zoom();
```

Change the `svg`'s `view_box` attribute from the fixed
`"0 0 {GRAPH_W} {GRAPH_H}"` to the computed viewport, and add the
wheel/drag handlers:

```rust
            svg {
                class: "mem-graph",
                view_box: "{vb_x} {vb_y} {vb_w} {vb_h}",
                onwheel: move |e| {
                    e.prevent_default();
                    let dy = e.delta().strip_units().y;
                    let factor = if dy > 0.0 { 0.9 } else { 1.1 };
                    let z = (zoom() * factor).clamp(0.4, 3.0);
                    zoom.set(z);
                },
                onmousedown: move |e| {
                    let p = e.client_coordinates();
                    dragging.set(Some((p.x, p.y)));
                },
                onmousemove: move |e| {
                    if let Some((sx, sy)) = dragging() {
                        let p = e.client_coordinates();
                        let (dx, dy) = (p.x - sx, p.y - sy);
                        // Drag right/down should move the *view* left/up
                        // (the content should follow the cursor), and the
                        // delta is in screen pixels while pan is in
                        // viewBox units — scale by the current zoom so a
                        // drag feels the same speed at any zoom level.
                        let (px, py) = pan();
                        pan.set((px - dx / zoom(), py - dy / zoom()));
                        dragging.set(Some((p.x, p.y)));
                    }
                },
                onmouseup: move |_| dragging.set(None),
                // A simpler fallback for "the drag ended off-element"
                // than a window-level listener: releasing outside the
                // SVG just stops the pan, it doesn't need to resume.
                onmouseleave: move |_| dragging.set(None),
```

(This replaces only the `svg { class: "mem-graph", view_box: ..., ` opening
— everything inside the `svg { ... }` block from Task 4 is unchanged.)

- [ ] **Step 2: Same treatment in `LatticeGraph`**

Apply the identical block (state signals + `view_box`/event handlers) to
`LatticeGraph`'s `svg` opening, using the exact same code as Step 1 (the
zoom/pan logic is graph-content-agnostic — it operates purely on the
viewBox, not on nodes/edges).

- [ ] **Step 3: Cursor affordance**

In `crates/aivyx-web/assets/stitch.css`, change:

```css
.mem-graph { width: 100%; height: auto; display: block; }
```

to:

```css
.mem-graph { width: 100%; height: auto; display: block; cursor: grab; }
.mem-graph:active { cursor: grabbing; }
```

- [ ] **Step 4: Verify**

```bash
cargo build -p aivyx-web --target wasm32-unknown-unknown
cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings
```
Expected: both green. No new unit tests — this step is pure DOM/pointer
interaction with no pure-function surface beyond the zoom clamp, which is
inline and trivial; verify manually in a browser after `dist/` is rebuilt
at the end of Task 7 (scroll-to-zoom changes the visible area, click-drag
pans, released outside the graph stops the pan).

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-web/src/main.rs crates/aivyx-web/assets/stitch.css
git commit -m "feat(web): pan/zoom on the Memory/Wiki knowledge graphs

Wheel scales the SVG viewBox (clamped 0.4x-3x); click-drag pans it.
Per-component local state (each graph's viewport is independent).
Releasing the drag outside the SVG (onmouseleave) stops the pan
rather than tracking a global pointer-up, a deliberate simplification
over a window-level listener for a graph-exploration nicety."
```

---

## Task 6: Documents — safe markdown rendering + Preview/Source toggle

**Files:**
- Modify: `crates/aivyx-web/src/guide.rs` (new `render_untrusted_
  markdown` function + its own test module)
- Modify: `crates/aivyx-web/src/main.rs` (`FileViewer` ~line 7336)
- Modify: `crates/aivyx-web/assets/stitch.css` (reuse `.guide-content`
  typography for the preview; no new rules needed beyond a small
  container tweak)

**Interfaces:**
- Produces: `pub fn render_untrusted_markdown(markdown: &str) -> String`
  and `fn is_markdown_path(path: &str) -> bool` (Task 7 detects a mermaid
  fence in the SAME rendered output this task produces, but adds its own
  detection logic rather than depending on a new function from this task
  — see Task 7's own Step 1).
- Consumes: nothing new from earlier tasks — independent of Tasks 1-5.

**Correction from the design spec's own read of `FileViewer`:** the spec
described replacing "the raw `pre{doc-text}` block." Re-reading
`FileViewer`'s actual branching (`main.rs:7336-7391`) during planning
found that branch only renders for a *truncated* file or one with no
content — a normal, small, non-binary file (the common case, including
every ordinary `.md` file) renders through the **editable `textarea`**
branch instead, since `editable = file.content.is_some() &&
!file.binary` is `true` there. This task adds a Preview/Source toggle
(defaulting to Preview for non-truncated `.md` files) rather than
replacing the `pre` block, which is what actually achieves the spec's
intent ("render `.md` files as markdown instead of raw monospace") given
what the code really does today.

- [ ] **Step 1: Write the failing tests**

`guide.rs` has no test module yet. Add one at the end of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_raw_html_instead_of_passing_it_through() {
        let out = render_untrusted_markdown("hello <script>alert(1)</script> world");
        assert!(!out.contains("<script>"));
        assert!(out.contains("&lt;script&gt;"));
    }

    #[test]
    fn inline_html_is_also_escaped() {
        let out = render_untrusted_markdown("click <a href=\"x\" onclick=\"evil()\">here</a>");
        assert!(!out.contains("onclick="));
    }

    #[test]
    fn mermaid_fence_becomes_a_mermaid_pre_block() {
        let out = render_untrusted_markdown("```mermaid\ngraph TD; A-->B;\n```");
        assert!(out.contains("<pre class=\"mermaid\">"));
        assert!(out.contains("graph TD; A-->B;"));
    }

    #[test]
    fn non_mermaid_fence_renders_as_an_ordinary_code_block() {
        let out = render_untrusted_markdown("```rust\nfn f() {}\n```");
        assert!(out.contains("<pre><code"));
        assert!(!out.contains("class=\"mermaid\""));
    }

    #[test]
    fn ordinary_markdown_renders_headings_and_tables() {
        let out = render_untrusted_markdown("# Title\n\n| a | b |\n|---|---|\n| 1 | 2 |\n");
        assert!(out.contains("<h1>Title</h1>"));
        assert!(out.contains("<table>"));
    }

    #[test]
    fn is_markdown_path_matches_md_and_markdown_extensions() {
        assert!(is_markdown_path("notes.md"));
        assert!(is_markdown_path("README.markdown"));
        assert!(!is_markdown_path("notes.txt"));
        assert!(!is_markdown_path("script.js"));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p aivyx-web guide::tests
```
Expected: FAIL — `render_untrusted_markdown`/`is_markdown_path` not
defined.

- [ ] **Step 3: Implement `render_untrusted_markdown`**

Change guide.rs's import line from:

```rust
use pulldown_cmark::{html, Options, Parser};
```

to:

```rust
use pulldown_cmark::{html, CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd};
```

Add, right after the existing `render` function:

```rust
/// Render markdown that may contain content the operator didn't author
/// themselves — an agent's `fs.write`, or arbitrary content under the
/// Documents screen's "Files" filesystem root. Unlike [`render`], this
/// never passes raw HTML through: `Event::Html`/`Event::InlineHtml` are
/// re-emitted as escaped visible text instead of executable markup,
/// closing the injection path `render`'s own doc comment says it depends
/// on trusted input to avoid. A fenced ` ```mermaid ` code block is
/// special-cased to `<pre class="mermaid">` — the markup mermaid.js's
/// browser build expects to find and typeset in place (see
/// `crates/aivyx-web/src/main.rs`'s `FileViewer`/mermaid loader).
pub fn render_untrusted_markdown(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(markdown, options);

    let mut events: Vec<Event> = Vec::new();
    let mut in_mermaid = false;
    let mut mermaid_src = String::new();

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(ref info)))
                if info.as_ref() == "mermaid" =>
            {
                in_mermaid = true;
                mermaid_src.clear();
            }
            Event::End(TagEnd::CodeBlock) if in_mermaid => {
                in_mermaid = false;
                let escaped = escape_html(&mermaid_src);
                events.push(Event::Html(CowStr::from(format!(
                    "<pre class=\"mermaid\">{escaped}</pre>"
                ))));
            }
            Event::Text(text) if in_mermaid => {
                mermaid_src.push_str(&text);
            }
            Event::Html(raw) | Event::InlineHtml(raw) => {
                events.push(Event::Text(CowStr::from(escape_html(&raw))));
            }
            other => events.push(other),
        }
    }

    let mut out = String::new();
    html::push_html(&mut out, events.into_iter());
    out
}

/// Minimal HTML-escape for text that must render as literal characters,
/// not markup — used both for a raw `<script>`-style block turned into
/// visible text and for a mermaid fence's source before it's wrapped in
/// `<pre>`.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// True for a path whose extension is `.md` or `.markdown` (case-sensitive
/// — matches how agents/operators actually name files in this workspace;
/// broaden to case-insensitive later if that proves too strict).
pub fn is_markdown_path(path: &str) -> bool {
    path.ends_with(".md") || path.ends_with(".markdown")
}
```

- [ ] **Step 4: Run to verify it passes**

```bash
cargo test -p aivyx-web guide::tests
```
Expected: PASS (6 passed).

- [ ] **Step 5: Wire a Preview/Source toggle into `FileViewer`**

Replace `FileViewer`'s current body (`main.rs:7336-7391`):

```rust
#[component]
fn FileViewer(file: DocFile, root: String) -> Element {
    let ws = use_context::<Sender>();
    let mut documents = use_context::<Signal<DocumentsState>>();
    // Seeded once per file (the panel keys this component by path, so it
    // remounts — and re-seeds — when a different file is opened).
    let mut edited = use_signal(|| file.content.clone().unwrap_or_default());
    let editable = file.content.is_some() && !file.binary;

    rsx! {
        div { class: "glass-card doc-viewer",
            div { class: "panel-head",
                h4 { "{file.path}" }
                span { class: "chip", {fmt_size(file.size_bytes)} }
                if editable {
                    {
                        let (r, p) = (root.clone(), file.path.clone());
                        rsx! {
                            button { class: "btn btn-primary btn-xs",
                                onclick: move |_| {
                                    ws.send(write_file_query(&r, &p, edited(), true));
                                    // Vitrine §8 — the bridge handles frames in
                                    // order, so this re-read returns the
                                    // post-write content and refreshes the open
                                    // file in place (no screen reload needed).
                                    ws.send(read_file_query(&r, &p));
                                },
                                "Save"
                            }
                        }
                    }
                }
                button { class: "btn btn-glass btn-xs", onclick: move |_| documents.write().file = None, "Close" }
            }
            if file.truncated {
                div { class: "notice err", "Showing the first 256 KB of a larger file — editing is disabled to avoid truncating it." }
            }
            if editable && !file.truncated {
                textarea { class: "doc-edit", spellcheck: "false",
                    value: "{edited}", oninput: move |e| edited.set(e.value()) }
            } else {
                match &file.content {
                    Some(text) => rsx! { pre { class: "doc-text", "{text}" } },
                    None => rsx! {
                        p { class: "label-tech sub",
                            {if file.binary {
                                format!("Binary file — {} not shown.", fmt_size(file.size_bytes))
                            } else {
                                "File too large to display.".to_string()
                            }}
                        }
                    },
                }
            }
        }
    }
}
```

with:

```rust
#[component]
fn FileViewer(file: DocFile, root: String) -> Element {
    let ws = use_context::<Sender>();
    let mut documents = use_context::<Signal<DocumentsState>>();
    // Seeded once per file (the panel keys this component by path, so it
    // remounts — and re-seeds — when a different file is opened).
    let mut edited = use_signal(|| file.content.clone().unwrap_or_default());
    let editable = file.content.is_some() && !file.binary;
    // POLISH_WAVES.md sub-project 6, item D — a non-truncated .md file
    // with content defaults to a rendered Preview instead of the plain
    // editable textarea every other file type gets; any file can still be
    // flipped to Source (which is exactly today's textarea/pre behavior,
    // unchanged) to see or edit the raw text.
    let is_md = guide::is_markdown_path(&file.path) && file.content.is_some() && !file.truncated;
    let mut preview = use_signal(move || is_md);

    rsx! {
        div { class: "glass-card doc-viewer",
            div { class: "panel-head",
                h4 { "{file.path}" }
                span { class: "chip", {fmt_size(file.size_bytes)} }
                if is_md {
                    button {
                        class: "btn btn-glass btn-xs",
                        onclick: move |_| preview.set(!preview()),
                        {if preview() { "Source" } else { "Preview" }}
                    }
                }
                if editable && !(is_md && preview()) {
                    {
                        let (r, p) = (root.clone(), file.path.clone());
                        rsx! {
                            button { class: "btn btn-primary btn-xs",
                                onclick: move |_| {
                                    ws.send(write_file_query(&r, &p, edited(), true));
                                    // Vitrine §8 — the bridge handles frames in
                                    // order, so this re-read returns the
                                    // post-write content and refreshes the open
                                    // file in place (no screen reload needed).
                                    ws.send(read_file_query(&r, &p));
                                },
                                "Save"
                            }
                        }
                    }
                }
                button { class: "btn btn-glass btn-xs", onclick: move |_| documents.write().file = None, "Close" }
            }
            if file.truncated {
                div { class: "notice err", "Showing the first 256 KB of a larger file — editing is disabled to avoid truncating it." }
            }
            if is_md && preview() {
                div {
                    class: "guide-content doc-preview",
                    dangerous_inner_html: guide::render_untrusted_markdown(file.content.as_deref().unwrap_or_default()),
                }
            } else if editable && !file.truncated {
                textarea { class: "doc-edit", spellcheck: "false",
                    value: "{edited}", oninput: move |e| edited.set(e.value()) }
            } else {
                match &file.content {
                    Some(text) => rsx! { pre { class: "doc-text", "{text}" } },
                    None => rsx! {
                        p { class: "label-tech sub",
                            {if file.binary {
                                format!("Binary file — {} not shown.", fmt_size(file.size_bytes))
                            } else {
                                "File too large to display.".to_string()
                            }}
                        }
                    },
                }
            }
        }
    }
}
```

(`file.content.as_deref().unwrap_or_default()` is safe here: `is_md` is
only `true` when `file.content.is_some()`, so this always has real text to
render when reached.)

- [ ] **Step 6: A small container tweak for the preview**

`.guide-content` (`stitch.css:829`) is `max-width: 820px`, matched to the
Guide screen's own two-column layout; `.doc-viewer` (`stitch.css:694`) has
no such cap, so reusing `.guide-content` verbatim would visually narrow
the Documents preview against the rest of that screen for no reason. In
`crates/aivyx-web/assets/stitch.css`, add right after the `.guide-content`
rule block (after its last related rule, `.guide-content hr` ~line 877):

```css
/* Documents preview reuses .guide-content's typography (POLISH_WAVES.md
   sub-project 6, item D) but not its Guide-specific max-width. */
.doc-preview { max-width: none; padding: 12px 14px; }
```

- [ ] **Step 7: Verify**

```bash
cargo test -p aivyx-web guide::tests
cargo build -p aivyx-web --target wasm32-unknown-unknown
cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings
```
Expected: all green.

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-web/src/guide.rs crates/aivyx-web/src/main.rs crates/aivyx-web/assets/stitch.css
git commit -m "feat(web): render markdown safely in the Documents screen

render_untrusted_markdown (guide.rs) walks pulldown-cmark's Event
stream directly instead of using html::push_html blind: raw HTML is
escaped to visible text rather than passed through, unlike guide::
render (which stays as-is — its own doc comment already scopes it to
trusted content only). A .md file with content defaults to a
Preview/Source toggle in FileViewer; Source is unchanged existing
behavior. Fenced \`\`\`mermaid blocks render as <pre class=\"mermaid\">
(Task 7 wires up actual diagram rendering)."
```

---

## Task 7: Mermaid diagram rendering

**Files:**
- Create: `crates/aivyx-web/assets/vendor/mermaid.min.js` (vendored,
  not authored — see Step 1)
- Modify: `crates/aivyx-web/src/main.rs` (`FileViewer`, a new small JS
  interop shim)

**Interfaces:** No new shared functions for later tasks — this is the
final task in the plan.

Depends on Task 6 (this task activates the `<pre class="mermaid">` markup
Task 6 already emits; it adds nothing if run before Task 6).

- [ ] **Step 1: Vendor mermaid.min.js**

Download a released mermaid.js browser build (the single-file UMD/global
`mermaid.min.js`, the same artifact `<script src="https://cdn.../
mermaid.min.js">` would normally point at) and place it at
`crates/aivyx-web/assets/vendor/mermaid.min.js`. Record the exact version
downloaded in a one-line comment at the top of a new
`crates/aivyx-web/assets/vendor/README.md`:

```markdown
# Vendored JS

- `mermaid.min.js` — mermaid.js browser build, version X.Y.Z (fill in the
  actual version downloaded). Vendored (not a Cargo/npm dependency)
  because this crate targets wasm32 with no JS package manager in the
  build; served from the daemon's own embedded bundle, no CDN call, per
  this project's local-first stance. To update: download the new
  release's `mermaid.min.js` browser build and replace this file, then
  update this version note.
```

- [ ] **Step 2: Declare it as a lazy asset**

Near the other `asset!()` declarations (`main.rs:57-78`), add:

```rust
// POLISH_WAVES.md sub-project 6, item D — vendored, not referenced from
// the base app shell (see FileViewer's mermaid loader below): loading it
// eagerly on every Studio boot would cost every operator a few hundred
// KB of transfer for a screen most sessions never open. `with_minify
// (false)` because the file is already minified upstream — running it
// through the bundler's own minifier again is redundant risk for zero
// benefit.
const MERMAID_JS: Asset = asset!(
    "/assets/vendor/mermaid.min.js",
    JsAssetOptions::new().with_minify(false)
);
```

Add `JsAssetOptions` to this file's `dioxus` import (find the existing
`use dioxus::prelude::*;`-style import near the top of `main.rs` and
confirm `JsAssetOptions` is exported from the same prelude — if not
already in scope, add `use dioxus::prelude::JsAssetOptions;` next to it).

- [ ] **Step 3: A tiny JS interop shim to invoke `mermaid.run()`**

Add near `FileViewer` (this crate has no existing "call into a vendored
global library" precedent — this is new, keep it to exactly what's
needed):

```rust
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
export function mermaid_run() {
    if (window.mermaid) {
        window.mermaid.run();
    }
}
")]
extern "C" {
    fn mermaid_run();
}

/// Load the vendored mermaid.js exactly once per page session (checking
/// `window.mermaid` first so a second `.md` file with a mermaid fence
/// doesn't re-inject the `<script>` tag), then call `mermaid.run()` once
/// it's loaded — or immediately if it was already loaded by an earlier
/// call. `dangerous_inner_html`-injected `<script>` tags never execute
/// per the HTML spec, so this creates a real, appended `<script>` element
/// via `web_sys` instead.
fn ensure_mermaid_loaded_then_run() {
    let Some(window) = web_sys::window() else { return };
    let Some(document) = window.document() else { return };
    let already_loaded = js_sys::Reflect::has(&window, &"mermaid".into()).unwrap_or(false);
    if already_loaded {
        mermaid_run();
        return;
    }
    let Ok(script) = document.create_element("script") else { return };
    script.set_attribute("src", &MERMAID_JS.to_string()).ok();
    let onload = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
        mermaid_run();
    });
    if let Some(el) = script.dyn_ref::<web_sys::HtmlScriptElement>() {
        el.set_onload(Some(onload.as_ref().unchecked_ref()));
    }
    onload.forget();
    if let Some(head) = document.head() {
        let _ = head.append_child(&script);
    }
}
```

This needs `use wasm_bindgen::JsCast;` in scope for `dyn_ref`/
`unchecked_ref` — confirm it's already imported at the top of `main.rs`
(Task 6/earlier code in this file already uses `JsCast` for the Guide
screen's click delegation, per `main.rs`'s own doc comment on
`GuidePanel`) before adding a second import.

- [ ] **Step 4: Trigger it from `FileViewer` after a mermaid-bearing
  render**

In `FileViewer` (from Task 6's version), change the Preview branch:

```rust
            if is_md && preview() {
                div {
                    class: "guide-content doc-preview",
                    dangerous_inner_html: guide::render_untrusted_markdown(file.content.as_deref().unwrap_or_default()),
                }
            } else if editable && !file.truncated {
```

to:

```rust
            if is_md && preview() {
                {
                    let content = file.content.as_deref().unwrap_or_default();
                    let html = guide::render_untrusted_markdown(content);
                    let has_mermaid = html.contains("class=\"mermaid\"");
                    if has_mermaid {
                        // Runs after this render commits the new DOM nodes
                        // mermaid needs to find — `use_effect` fires after
                        // the render, `dangerous_inner_html` included.
                        use_effect(move || ensure_mermaid_loaded_then_run());
                    }
                    rsx! {
                        div {
                            class: "guide-content doc-preview",
                            dangerous_inner_html: html,
                        }
                    }
                }
            } else if editable && !file.truncated {
```

- [ ] **Step 5: Verify**

```bash
cargo build -p aivyx-web --target wasm32-unknown-unknown
cargo clippy -p aivyx-web --target wasm32-unknown-unknown --all-targets -- -D warnings
```
Expected: both green. There's no unit-testable surface for the
DOM/script-injection path itself (same category as Task 5's pointer
interaction) — verify manually after the `dist/` rebuild below: open a
`.md` file (or create one via Documents) containing:

    ```mermaid
    graph TD; A-->B;
    ```

and confirm it renders as a diagram, not a code block, and that opening a
*second* mermaid-bearing file doesn't re-fetch the script (check the
Network tab: one request for `mermaid.min.js` for the whole session).

- [ ] **Step 6: Rebuild and commit `dist/`**

Per the Global Constraints: rebuild the wasm bundle
(`~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin` +
`~/.cargo/bin` on `PATH`, then `dx bundle --release --platform web` from
`crates/aivyx-web`), replace `crates/aivyx-web/dist/` wholesale (`rm -rf
dist && cp -r <bundle output> dist` — never a merging copy), strip any
`.br` files, and verify:

```bash
git status --porcelain crates/aivyx-web/dist/assets/ | grep wasm
```
Expected: exactly one line starting `A ` (added) and one starting `D `
(deleted) — a rename, not an accumulation of stale binaries.

```bash
git add crates/aivyx-web/assets/vendor/mermaid.min.js crates/aivyx-web/assets/vendor/README.md \
        crates/aivyx-web/src/main.rs crates/aivyx-web/dist/
git commit -m "feat(web): render mermaid diagrams in Documents markdown preview

Vendored mermaid.min.js, lazy-loaded via a real <script> element the
first time a rendered .md preview contains a mermaid fence (dangerous_
inner_html-injected <script> tags never execute per spec, so this
can't just be part of the rendered HTML). A small wasm-bindgen shim
calls mermaid.run() once the script loads, or immediately on a later
file if mermaid is already on window. Rebuilt dist/."
```

- [ ] **Step 7: Full workspace sweep**

```bash
cargo clippy --workspace --exclude aivyx-desktop --all-targets -- -D warnings
cargo test --workspace --exclude aivyx-desktop
cargo build -p aivyx-web --target wasm32-unknown-unknown
```
Expected: zero warnings, zero failures, clean wasm build. This is the
final task — once this sweep is clean, proceed to the final whole-branch
review.
